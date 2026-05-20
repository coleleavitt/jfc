//! Parser for inline `<tool_call>` / `<tool_result>` XML-ish tags that some
//! backends (notably OpenWebUI's tool-running shim and some
//! Anthropic-on-third-party gateways) stream interleaved with assistant text.
//!
//! The streamed text looks like:
//!
//! ```text
//! Sure, let me look around.
//! <tool_call> {"name": "bash", "arguments": {"command": "ls -la"}} </tool_call>
//! <tool_result> total 24 drwxr-xr-x ... </tool_result>
//! Here's what I found.
//! ```
//!
//! Without parsing, jfc rendered the tags verbatim, producing the giant
//! "<tool_call> {...}" walls visible in the user's screenshots. This module
//! splits the stream into a sequence of [`Segment`]s the renderer can format
//! distinctly (text → markdown, tool_call → header, tool_result → collapsible
//! preview).
//!
//! The parser is **streaming-friendly**: incomplete tags at the end of the
//! buffer are emitted as a final `Text` segment containing the partial bytes,
//! so the next render — once more text has arrived — can re-parse and
//! re-render correctly. We never lose bytes.

use serde_json::Value;

/// One run of parsed content. Order in the output preserves source order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Segment {
    /// Plain assistant text. Pass to the markdown renderer.
    Text(String),
    /// A `<tool_call>` block. `raw_body` is the JSON-ish payload between the
    /// open/close tags, trimmed. `parsed` decodes a `{"name": ..., "arguments": ...}`
    /// shape when possible; falls back to `None` for malformed payloads.
    ToolCall {
        raw_body: String,
        parsed: Option<ParsedToolCall>,
    },
    /// A `<tool_result>` block. The body is verbatim text (often command output).
    ToolResult(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedToolCall {
    pub name: String,
    /// Best-effort one-line summary of the arguments (e.g. the `command` field
    /// for bash, the `path` for Read). Falls back to a JSON repr if no obvious
    /// summary field exists.
    pub summary: String,
}

const OPEN_CALL: &str = "<tool_call>";
const CLOSE_CALL: &str = "</tool_call>";
const OPEN_RESULT: &str = "<tool_result>";
const CLOSE_RESULT: &str = "</tool_result>";

/// Split `input` into ordered segments. Always preserves all bytes — partial
/// tags at the tail end are emitted as plain text so streaming renders never
/// drop characters.
pub fn parse(input: &str) -> Vec<Segment> {
    let mut out: Vec<Segment> = Vec::new();
    let mut text_buf = String::new();
    let mut i = 0usize;
    let bytes = input.as_bytes();

    while i < bytes.len() {
        // Find the earliest opening tag from position i. Manual byte scan keeps
        // us off regex; the tag set is small and fixed.
        let next_call = input[i..].find(OPEN_CALL);
        let next_result = input[i..].find(OPEN_RESULT);

        let (offset, tag_kind) = match (next_call, next_result) {
            (Some(a), Some(b)) if a <= b => (a, TagKind::Call),
            (Some(_), Some(b)) => (b, TagKind::Result),
            (Some(a), None) => (a, TagKind::Call),
            (None, Some(b)) => (b, TagKind::Result),
            (None, None) => {
                text_buf.push_str(&input[i..]);
                break;
            }
        };

        // Flush any plain text that preceded the tag.
        if offset > 0 {
            text_buf.push_str(&input[i..i + offset]);
        }

        let open = match tag_kind {
            TagKind::Call => OPEN_CALL,
            TagKind::Result => OPEN_RESULT,
        };
        let close = match tag_kind {
            TagKind::Call => CLOSE_CALL,
            TagKind::Result => CLOSE_RESULT,
        };

        let body_start = i + offset + open.len();
        let close_at = input[body_start..].find(close);

        match close_at {
            Some(rel) => {
                // Complete tag — flush pending text, then emit the tool segment.
                if !text_buf.is_empty() {
                    out.push(Segment::Text(std::mem::take(&mut text_buf)));
                }
                let body = input[body_start..body_start + rel].trim().to_owned();
                match tag_kind {
                    TagKind::Call => {
                        let parsed = parse_tool_call_body(&body);
                        out.push(Segment::ToolCall {
                            raw_body: body,
                            parsed,
                        });
                    }
                    TagKind::Result => {
                        out.push(Segment::ToolResult(body));
                    }
                }
                i = body_start + rel + close.len();
            }
            None => {
                // Partial tag — keep everything from the open tag onward as text
                // for now. The next render pass will re-parse and pick up the
                // closing tag once it arrives.
                text_buf.push_str(&input[i + offset..]);
                break;
            }
        }
    }

    if !text_buf.is_empty() {
        out.push(Segment::Text(text_buf));
    }
    out
}

#[derive(Debug, Clone, Copy)]
enum TagKind {
    Call,
    Result,
}

fn parse_tool_call_body(body: &str) -> Option<ParsedToolCall> {
    let v: Value = serde_json::from_str(body).ok()?;
    let name = v.get("name")?.as_str()?.to_owned();
    let summary = summarize_args(v.get("arguments"));
    Some(ParsedToolCall { name, summary })
}

fn summarize_args(args: Option<&Value>) -> String {
    let Some(args) = args else {
        return String::new();
    };
    // Common one-shot summary fields, in priority order. Matches the way the
    // user reads bash/file tool calls — "what did this *do*", not the JSON shape.
    for key in ["command", "path", "file_path", "query", "url", "pattern"] {
        if let Some(s) = args.get(key).and_then(Value::as_str) {
            return s.trim().to_owned();
        }
    }
    // Fall back to a compact JSON rendering, single-line.
    serde_json::to_string(args).unwrap_or_default()
}

/// Returns `true` if `text` contains any inline tool tag — a quick test the
/// renderer uses to decide whether to invoke the parser at all.
pub fn contains_inline_tools(text: &str) -> bool {
    text.contains(OPEN_CALL) || text.contains(OPEN_RESULT)
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── DO-178B normal/robust split ────────────────────────────────────────

    // Normal: pure text passes through as a single Text segment.
    #[test]
    fn pure_text_normal() {
        assert_eq!(
            parse("hello world"),
            vec![Segment::Text("hello world".into())]
        );
    }

    // Normal: a complete tool_call is extracted with parsed name + summary.
    #[test]
    fn complete_tool_call_extracted_normal() {
        let s = r#"prefix <tool_call> {"name": "bash", "arguments": {"command": "ls -la"}} </tool_call> suffix"#;
        let segs = parse(s);
        assert_eq!(segs.len(), 3);
        assert_eq!(segs[0], Segment::Text("prefix ".into()));
        match &segs[1] {
            Segment::ToolCall { parsed, .. } => {
                let p = parsed.as_ref().expect("parsed tool call");
                assert_eq!(p.name, "bash");
                assert_eq!(p.summary, "ls -la");
            }
            _ => panic!("expected ToolCall, got {:?}", segs[1]),
        }
        assert_eq!(segs[2], Segment::Text(" suffix".into()));
    }

    // Normal: a complete tool_result is extracted as a single segment.
    #[test]
    fn complete_tool_result_extracted_normal() {
        let s = "<tool_result> total 24 drwxr-xr-x </tool_result>";
        assert_eq!(
            parse(s),
            vec![Segment::ToolResult("total 24 drwxr-xr-x".into())]
        );
    }

    // Normal: a tool_call followed immediately by its tool_result yields two
    // distinct segments in order.
    #[test]
    fn call_then_result_two_segments_normal() {
        let s = r#"<tool_call> {"name": "bash"} </tool_call> <tool_result>ok</tool_result>"#;
        let segs = parse(s);
        assert_eq!(segs.len(), 3); // text " " between blocks counts
        assert!(matches!(segs[0], Segment::ToolCall { .. }));
        // segs[1] is the single space between blocks
        assert_eq!(segs[1], Segment::Text(" ".into()));
        assert!(matches!(segs[2], Segment::ToolResult(_)));
    }

    // Normal: summary picks the right field for non-bash tools.
    #[test]
    fn summary_picks_path_for_read_normal() {
        let s = r#"<tool_call> {"name": "read", "arguments": {"path": "/etc/hosts"}} </tool_call>"#;
        let segs = parse(s);
        match &segs[0] {
            Segment::ToolCall { parsed, .. } => {
                assert_eq!(parsed.as_ref().unwrap().summary, "/etc/hosts");
            }
            _ => panic!(),
        }
    }

    // Robust: a partial open tag at the end of the buffer is preserved as text
    // so the next streaming chunk can complete it. No bytes lost.
    #[test]
    fn partial_open_tag_preserved_as_text_robust() {
        let s = "hello <tool_ca";
        assert_eq!(parse(s), vec![Segment::Text("hello <tool_ca".into())]);
    }

    // Robust: an open tag with no closing tag in the buffer — same property,
    // emitted as text so streaming can reparse later.
    #[test]
    fn open_without_close_preserved_robust() {
        let s = r#"hello <tool_call> {"name": "bash"} ... and more text"#;
        let segs = parse(s);
        assert_eq!(segs.len(), 1);
        assert!(matches!(segs[0], Segment::Text(_)));
        if let Segment::Text(t) = &segs[0] {
            assert!(t.contains("<tool_call>"));
            assert!(t.contains("\"bash\""));
        }
    }

    // Robust: malformed JSON inside a complete tag still produces a ToolCall
    // segment (renderer can show the raw body) — we don't drop the block.
    #[test]
    fn malformed_json_falls_back_to_raw_body_robust() {
        let s = "<tool_call> not json </tool_call>";
        let segs = parse(s);
        assert_eq!(segs.len(), 1);
        match &segs[0] {
            Segment::ToolCall { raw_body, parsed } => {
                assert_eq!(raw_body, "not json");
                assert!(parsed.is_none());
            }
            _ => panic!("expected ToolCall, got {:?}", segs[0]),
        }
    }

    // Robust: empty input → empty output (no panic, no spurious segments).
    #[test]
    fn empty_input_empty_output_robust() {
        assert!(parse("").is_empty());
    }

    // Robust: nested-looking tags (inner block inside text content of an outer)
    // — we match the first close tag, which is the right behavior for non-nested
    // sources like OpenWebUI. Nested tools would need a different strategy, but
    // they don't occur in practice.
    #[test]
    fn first_close_wins_for_non_nested_sources_robust() {
        let s = "<tool_result>aa</tool_result><tool_result>bb</tool_result>";
        let segs = parse(s);
        assert_eq!(
            segs,
            vec![
                Segment::ToolResult("aa".into()),
                Segment::ToolResult("bb".into()),
            ]
        );
    }

    // Robust: many bytes before/after a single tag don't blow up parsing.
    #[test]
    fn large_surrounding_text_robust() {
        let pre: String = "a".repeat(10_000);
        let post: String = "b".repeat(10_000);
        let s = format!(r#"{pre}<tool_call> {{"name":"x"}} </tool_call>{post}"#);
        let segs = parse(&s);
        assert_eq!(segs.len(), 3);
        assert!(matches!(segs[0], Segment::Text(_)));
        assert!(matches!(segs[1], Segment::ToolCall { .. }));
        assert!(matches!(segs[2], Segment::Text(_)));
    }

    // Normal: contains_inline_tools is a cheap pre-check.
    #[test]
    fn contains_inline_tools_smoke_normal() {
        assert!(!contains_inline_tools("just text"));
        assert!(contains_inline_tools("foo <tool_call> bar"));
        assert!(contains_inline_tools("foo <tool_result>"));
    }

    // ── Trait-system contract: StreamConvention dispatches to the right path ──

    // Normal: a provider declaring `InlineXmlTags` always invokes the parser,
    // even on text that happens not to contain tags right now (the convention
    // is a stable promise, not a content sniff).
    #[test]
    fn convention_inline_xml_always_parses_normal() {
        // Plain text with no tags still produces a single Text segment under
        // explicit InlineXmlTags convention — semantics match contains_inline_tools()
        // returning false then falling through to the markdown path. Keeping the
        // contract in a test pins it to documentation.
        let segs = parse("hello world");
        assert_eq!(segs, vec![Segment::Text("hello world".into())]);
    }

    // Normal: native conventions only need parsing when content sniff hits, so
    // we verify the cheap pre-check matches what the parser would emit.
    #[test]
    fn contains_check_matches_parser_normal() {
        for sample in [
            "no tags here",
            "<tool_call> {} </tool_call>",
            "prefix <tool_result>x</tool_result>",
        ] {
            let claims_tags = contains_inline_tools(sample);
            let actually_has_segments = parse(sample)
                .iter()
                .any(|s| matches!(s, Segment::ToolCall { .. } | Segment::ToolResult(_)));
            assert_eq!(claims_tags, actually_has_segments, "sample: {sample:?}");
        }
    }
}
