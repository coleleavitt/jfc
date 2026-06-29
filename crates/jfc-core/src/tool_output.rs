use super::DiffView;

#[derive(Clone, Debug)]
pub struct LargeText {
    pub content: String,
    pub line_count: usize,
    pub byte_count: usize,
}

impl LargeText {
    pub const COLLAPSE_LINES: usize = 500;
    pub const COLLAPSE_BYTES: usize = 30_720;

    pub fn new(content: String) -> Self {
        let line_count = content.lines().count();
        let byte_count = content.len();
        Self {
            content,
            line_count,
            byte_count,
        }
    }

    pub fn should_collapse(text: &str) -> bool {
        text.len() > Self::COLLAPSE_BYTES || text.lines().count() > Self::COLLAPSE_LINES
    }

    pub fn size_label(&self) -> String {
        let kb = self.byte_count as f64 / 1024.0;
        format!("{} lines · {:.1} KB", self.line_count, kb)
    }
}

#[derive(Clone, Debug)]
pub enum ToolOutput {
    Text(String),
    LargeText(LargeText),
    Diff(DiffView),
    FileContent {
        path: String,
        content: String,
        language: String,
    },
    Command {
        stdout: String,
        stderr: String,
        exit_code: Option<i32>,
    },
    FileList(Vec<String>),
    /// Anthropic server-side tool result (e.g. `web_search_tool_result`).
    /// The runtime never produces these locally — they arrive on a
    /// `StreamEvent::ServerToolResult` event and get attached to the
    /// originating `server_tool_use` ToolCall so that:
    ///
    ///   * the renderer can show the actual results instead of a stub
    ///     "🔍 Executed server-side by Anthropic" placeholder;
    ///   * `build_provider_messages_with_tool_results` re-emits the
    ///     block byte-faithfully on resend (cli.js v142:441375) instead
    ///     of fabricating a synthetic user `tool_result` (which would
    ///     break the server-side sampling loop's resumption logic per
    ///     cli.js v142:7057).
    ///
    /// `content` is the raw JSON value Anthropic returned (array of
    /// `{title,url}` for web_search, `{error_code,...}` on failure,
    /// etc.) so future server-tool result shapes round-trip without
    /// code changes.
    ServerToolResult {
        tool_kind: crate::ServerToolResultKind,
        content: serde_json::Value,
    },
    Empty,
}

/// Public wrapper around `format_server_tool_result_text` so the
/// renderer and tool-blocks module (which live outside `types::tool`)
/// can use the same formatting rules without duplicating the cli.js
/// consumer logic.
pub fn format_server_tool_result_text_public(
    tool_kind: &crate::ServerToolResultKind,
    content: &serde_json::Value,
) -> String {
    format_server_tool_result_text(tool_kind, content)
}

/// Render a server-side tool result (e.g. `web_search_tool_result`) as
/// human-readable text. Mirrors the v142 cli.js consumer at line
/// 394261 (`Bt_`): an array of `{title, url}` objects renders as a
/// bulleted list; an error wrapper (`{error_code: ...}`) renders as a
/// short error line.
///
/// The original JSON `content` is preserved on the
/// `ToolOutput::ServerToolResult` variant for byte-faithful resend;
/// this function is only for display + log.
fn format_server_tool_result_text(
    tool_kind: &crate::ServerToolResultKind,
    content: &serde_json::Value,
) -> String {
    use crate::ServerToolResultKind;
    if matches!(tool_kind, ServerToolResultKind::Advisor)
        && let Some(obj) = content.as_object()
    {
        let ty = obj.get("type").and_then(|v| v.as_str()).unwrap_or_default();
        if ty == "advisor_tool_result_error" {
            let code = obj
                .get("error_code")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown");
            return format!("Advisor unavailable ({code})");
        }
        if ty == "advisor_result"
            && let Some(text) = obj.get("text").and_then(|v| v.as_str())
        {
            return text.to_owned();
        }
        if ty == "advisor_redacted_result" {
            return "Advisor reviewed the conversation and returned redacted feedback.".to_owned();
        }
    }
    // Error variant first — Anthropic wraps failures in
    // `{ "error_code": "..." }` rather than an array.
    if let Some(obj) = content.as_object()
        && let Some(code) = obj.get("error_code").and_then(|v| v.as_str())
    {
        return format!("[{wire} error] {code}", wire = tool_kind.wire_type());
    }
    match tool_kind {
        ServerToolResultKind::WebSearch => {
            let Some(items) = content.as_array() else {
                return format!(
                    "[web_search_tool_result] (non-array content: {})",
                    content.to_string().chars().take(200).collect::<String>()
                );
            };
            if items.is_empty() {
                return "[web_search_tool_result] no results".to_owned();
            }
            let mut out = format!("Web search returned {} result(s):\n", items.len());
            for item in items {
                let title = item
                    .get("title")
                    .and_then(|v| v.as_str())
                    .unwrap_or("(no title)");
                let url = item
                    .get("url")
                    .and_then(|v| v.as_str())
                    .unwrap_or("(no url)");
                out.push_str(&format!("  • {title}\n    {url}\n"));
            }
            out
        }
        ServerToolResultKind::CodeExecution
        | ServerToolResultKind::WebFetch
        | ServerToolResultKind::Advisor
        | ServerToolResultKind::Other(_) => format!(
            "[{wire}]\n{content}",
            wire = tool_kind.wire_type(),
            content = serde_json::to_string_pretty(content).unwrap_or_else(|_| content.to_string())
        ),
    }
}

impl ToolOutput {
    /// Mirror of the wire-format truncation cap in `stream.rs`
    /// (`MAX_TOOL_RESULT_CHARS`). The API only ever sees a tool result
    /// shortened to this many bytes, so the local token estimate must cap
    /// here too — otherwise a 500KB Read output makes `compact_level` think
    /// the context is full when the API only received 30KB of it. That
    /// mismatch is what made compaction trigger on every tool batch with a
    /// large file in it.
    /// Matches Claude Code v2.1.131's per-tool default cap (`yIK = 5e4` in
    /// the deob bundle). Was 30KB; 50KB lets a Read on a typical source
    /// file land entirely in the head slice without triggering the
    /// truncation marker, while still keeping the per-result wire size
    /// bounded so a single tool call can't blow a 1M-token request.
    pub const APPROX_LEN_CAP: usize = 50_000;

    pub fn approx_text_len(&self) -> usize {
        let raw = match self {
            Self::Text(s) => s.len(),
            Self::LargeText(lt) => lt.byte_count,
            Self::Diff(d) => d
                .hunks
                .iter()
                .flat_map(|h| &h.lines)
                .map(|l| l.content.len())
                .sum(),
            Self::FileContent { content, .. } => content.len(),
            Self::Command { stdout, stderr, .. } => stdout.len() + stderr.len(),
            Self::FileList(files) => files.iter().map(|f| f.len()).sum(),
            Self::ServerToolResult { content, .. } => {
                serde_json::to_string(content).map(|s| s.len()).unwrap_or(0)
            }
            Self::Empty => 0,
        };
        raw.min(Self::APPROX_LEN_CAP)
    }

    pub fn text_only(&self) -> String {
        match self {
            Self::Text(s) => s.clone(),
            Self::LargeText(lt) => format!("[large: {}]", lt.size_label()),
            Self::Diff(d) => format!("{} (+{}/-{})", d.file_path, d.additions, d.deletions),
            Self::FileContent { path, .. } => format!("[file: {}]", path),
            Self::Command {
                stdout,
                stderr,
                exit_code,
            } => {
                let code = exit_code
                    .map(|c| c.to_string())
                    .unwrap_or_else(|| "?".into());
                format!(
                    "exit={} stdout={}B stderr={}B",
                    code,
                    stdout.len(),
                    stderr.len()
                )
            }
            Self::FileList(files) => format!("{} files", files.len()),
            Self::ServerToolResult { tool_kind, content } => {
                format_server_tool_result_text(tool_kind, content)
            }
            Self::Empty => String::new(),
        }
    }

    pub fn to_display_string(&self) -> String {
        match self {
            Self::Text(s) => s.clone(),
            Self::LargeText(lt) => lt.content.clone(),
            Self::Diff(d) => format!("{} (+{}/-{})", d.file_path, d.additions, d.deletions),
            Self::FileContent { path, content, .. } => {
                format!("{} ({} chars)", path, content.len())
            }
            Self::Command {
                stdout, exit_code, ..
            } => {
                let code = exit_code
                    .map(|c| c.to_string())
                    .unwrap_or_else(|| "?".into());
                let preview = if stdout.len() > 100 {
                    format!("{}...", &stdout[..stdout.floor_char_boundary(100)])
                } else {
                    stdout.clone()
                };
                format!("exit={}: {}", code, preview)
            }
            Self::FileList(files) => format!("{} files", files.len()),
            Self::ServerToolResult { tool_kind, content } => {
                format_server_tool_result_text(tool_kind, content)
            }
            Self::Empty => "[empty]".into(),
        }
    }

    pub fn to_api_text(&self) -> String {
        match self {
            Self::LargeText(lt) => lt.content.clone(),
            other => other.to_display_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diff::parse_unified_diff;

    // ─── LargeText ────────────────────────────────────────────────────────

    #[test]
    fn large_text_collapses_above_threshold() {
        let short = "line\n".repeat(10);
        assert!(!LargeText::should_collapse(&short));

        let tall = "line\n".repeat(LargeText::COLLAPSE_LINES + 1);
        assert!(LargeText::should_collapse(&tall));

        let fat = "x".repeat(LargeText::COLLAPSE_BYTES + 1);
        assert!(LargeText::should_collapse(&fat));
    }

    #[test]
    fn large_text_size_label_formats_correctly() {
        let lt = LargeText::new("hello\nworld\n".into());
        assert_eq!(lt.line_count, 2);
        assert!(lt.size_label().contains("lines"));
        assert!(lt.size_label().contains("KB"));
    }

    #[test]
    fn large_text_new_counts_lines_and_bytes_normal() {
        let lt = LargeText::new("a\nb\nc\n".into());
        assert_eq!(lt.line_count, 3);
        assert_eq!(lt.byte_count, 6);
    }

    #[test]
    fn large_text_should_not_collapse_below_thresholds_normal() {
        let s = "x".repeat(LargeText::COLLAPSE_BYTES);
        // Exactly at byte limit shouldn't collapse — the check is `>` not `>=`.
        assert!(!LargeText::should_collapse(&s));
    }

    #[test]
    fn large_text_size_label_includes_kilobytes_normal() {
        let lt = LargeText::new("x".repeat(2048));
        let label = lt.size_label();
        assert!(label.contains("KB"), "{label}");
        assert!(label.contains("lines"), "{label}");
    }

    // ─── ToolOutput ───────────────────────────────────────────────────────

    #[test]
    fn tool_output_large_text_api_text_returns_full_content() {
        let lt = LargeText::new("abc\ndef\n".into());
        let out = ToolOutput::LargeText(lt);
        assert_eq!(out.to_api_text(), "abc\ndef\n");
    }

    #[test]
    fn tool_output_approx_text_len_caps_at_30k_robust() {
        // Even a megabyte of text reports cap value — important for token
        // estimation against the truncated wire result.
        let huge = "x".repeat(2_000_000);
        let out = ToolOutput::Text(huge);
        assert_eq!(out.approx_text_len(), ToolOutput::APPROX_LEN_CAP);
    }

    #[test]
    fn tool_output_approx_text_len_command_combines_streams_normal() {
        let out = ToolOutput::Command {
            stdout: "abc".into(),
            stderr: "de".into(),
            exit_code: Some(0),
        };
        assert_eq!(out.approx_text_len(), 5);
    }

    #[test]
    fn tool_output_approx_text_len_empty_is_zero_normal() {
        assert_eq!(ToolOutput::Empty.approx_text_len(), 0);
    }

    #[test]
    fn tool_output_approx_text_len_filelist_sums_path_lens_normal() {
        let out = ToolOutput::FileList(vec!["abc".into(), "de".into()]);
        assert_eq!(out.approx_text_len(), 5);
    }

    #[test]
    fn tool_output_approx_text_len_diff_sums_line_content_normal() {
        let view = parse_unified_diff("x.rs", "@@ -1,1 +1,1 @@\n-abc\n+abcd\n");
        let out = ToolOutput::Diff(view);
        // "abc" (3) + "abcd" (4) = 7
        assert_eq!(out.approx_text_len(), 7);
    }

    #[test]
    fn tool_output_text_only_diff_includes_counts_normal() {
        let view = parse_unified_diff("x.rs", "@@ -1,1 +1,1 @@\n-old\n+new\n");
        let s = ToolOutput::Diff(view).text_only();
        assert!(s.contains("x.rs"), "{s}");
        assert!(s.contains("+1"), "{s}");
        assert!(s.contains("-1"), "{s}");
    }

    #[test]
    fn tool_output_text_only_command_renders_exit_code_normal() {
        let s = ToolOutput::Command {
            stdout: "ok".into(),
            stderr: String::new(),
            exit_code: Some(2),
        }
        .text_only();
        assert!(s.contains("exit=2"), "{s}");
        assert!(s.contains("stdout=2B"), "{s}");
    }

    #[test]
    fn tool_output_text_only_command_renders_question_mark_when_no_code_robust() {
        // exit_code: None (kill via signal, etc.) renders "?".
        let s = ToolOutput::Command {
            stdout: String::new(),
            stderr: String::new(),
            exit_code: None,
        }
        .text_only();
        assert!(s.contains("exit=?"), "{s}");
    }

    #[test]
    fn tool_output_text_only_filecontent_includes_path_normal() {
        let s = ToolOutput::FileContent {
            path: "src/main.rs".into(),
            content: "fn main() {}".into(),
            language: "rust".into(),
        }
        .text_only();
        assert!(s.contains("src/main.rs"), "{s}");
    }

    #[test]
    fn tool_output_text_only_filelist_count_normal() {
        let s = ToolOutput::FileList(vec!["a".into(), "b".into(), "c".into()]).text_only();
        assert_eq!(s, "3 files");
    }

    #[test]
    fn tool_output_to_display_string_command_truncates_at_100_chars_robust() {
        let huge = "x".repeat(200);
        let s = ToolOutput::Command {
            stdout: huge,
            stderr: String::new(),
            exit_code: Some(0),
        }
        .to_display_string();
        assert!(s.contains("..."), "expected ellipsis on truncation: {s}");
    }

    #[test]
    fn tool_output_to_display_string_empty_renders_marker_normal() {
        assert_eq!(ToolOutput::Empty.to_display_string(), "[empty]");
    }

    #[test]
    fn tool_output_to_api_text_falls_back_to_display_robust() {
        // Non-LargeText variants delegate to to_display_string.
        let t = ToolOutput::Text("hello".into());
        assert_eq!(t.to_api_text(), "hello");
    }
}
