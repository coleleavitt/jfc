//! Render-layer guard against leaked tool-call markup.
//!
//! When a model emits a tool call as *visible text* instead of a real
//! structured call — e.g. `<Bash command="set -o pipefail\n…" />` — the
//! provider's stream interceptor (which only recognizes `<tool_call>` /
//! `<tool_use>` shapes, see `jfc-providers/.../openwebui`) lets it through and
//! the markdown renderer prints the raw XML verbatim. This is a known failure
//! mode: when a prompt is misclassified as non-action the tool catalog is
//! stripped (`stream::request`), so a model that still wants to act writes the
//! call as prose.
//!
//! This guard is defense-in-depth at the render layer: it recognizes a
//! tool-call-shaped element whose tag is a *known tool name* and replaces it
//! with a muted notice, so the garbage never reaches the transcript regardless
//! of which upstream heuristic misfired. It is deliberately high-precision —
//! the tag must name a real tool, and markup inside fenced code blocks (where
//! someone might legitimately *document* the syntax) is left untouched.

use std::borrow::Cow;
use std::collections::HashSet;
use std::sync::LazyLock;

/// Known tool names, captured once from the live tool catalog. A leaked call
/// must name one of these to be suppressed — bare `<div>` / `<Foo>` prose is
/// left alone. Names are case-sensitive (`Bash`, `Read`, …) to match how the
/// catalog advertises them and avoid colliding with lowercase HTML tags.
static TOOL_NAMES: LazyLock<HashSet<String>> = LazyLock::new(|| {
    jfc_engine::tools::all_tool_defs()
        .into_iter()
        .map(|d| d.name)
        .collect()
});

/// Suppress any leaked tool-call markup in `text`, returning a sanitized copy.
///
/// Borrows (zero-copy) when the text is clean — the common case — and only
/// allocates when at least one leaked element is found and rewritten.
pub(super) fn sanitize_leaked_tool_calls(text: &str) -> Cow<'_, str> {
    sanitize_with(text, &TOOL_NAMES)
}

/// Inner worker, parameterized over the tool-name set so tests can supply a
/// fixed catalog instead of depending on the global one.
fn sanitize_with<'a>(text: &'a str, tool_names: &HashSet<String>) -> Cow<'a, str> {
    // Fast path: a tool call always begins with `<` — skip the full scan when
    // the text can't possibly contain one.
    if !text.contains('<') {
        return Cow::Borrowed(text);
    }

    let bytes = text.as_bytes();
    let mut out = String::new();
    let mut last_flushed = 0usize;
    let mut i = 0usize;
    let mut in_fence = false;
    let mut at_line_start = true;
    let mut found = false;

    while i < bytes.len() {
        let b = bytes[i];

        // Track fenced code blocks (``` / ~~~). A fence toggles only when the
        // marker is the first non-whitespace on its line; markup inside a fence
        // is documentation, not a leaked call, so we never rewrite it.
        if at_line_start && is_fence_marker(text, i) {
            in_fence = !in_fence;
            // Advance to end of this line so the fence run isn't re-scanned.
            while i < bytes.len() && bytes[i] != b'\n' {
                i += 1;
            }
            at_line_start = true;
            continue;
        }

        if b == b'\n' {
            at_line_start = true;
            i += 1;
            continue;
        }
        at_line_start = at_line_start && (b == b' ' || b == b'\t');

        if in_fence || b != b'<' {
            i += 1;
            continue;
        }

        // Candidate `<` outside a fence — does a known tool name follow?
        match leaked_element_span(text, i, tool_names) {
            Some((end, name)) => {
                out.push_str(&text[last_flushed..i]);
                out.push_str(&suppression_notice(name));
                last_flushed = end;
                i = end;
                found = true;
                at_line_start = false;
            }
            None => i += 1,
        }
    }

    if !found {
        return Cow::Borrowed(text);
    }
    out.push_str(&text[last_flushed..]);
    Cow::Owned(out)
}

/// The muted in-flow marker left where a leaked element used to be. Keeps the
/// user informed that something was hidden rather than silently dropping it.
fn suppression_notice(name: &str) -> String {
    format!("⚠ [suppressed leaked tool-call markup: {name}]")
}

/// True when position `i` begins a ``` or ~~~ fence marker (≥3 of the same
/// char). Caller guarantees `i` is at the line's first non-whitespace column.
fn is_fence_marker(text: &str, i: usize) -> bool {
    let bytes = text.as_bytes();
    let c = bytes[i];
    if c != b'`' && c != b'~' {
        return false;
    }
    let run = bytes[i..].iter().take_while(|&&x| x == c).count();
    run >= 3
}

/// If `text[lt..]` starts a tool-call-shaped element naming a known tool,
/// return `(end_byte, tool_name)` covering the whole element. `lt` must index a
/// `<`. Handles self-closing (`<Bash .../>`), paired (`<Bash …>…</Bash>`), and
/// unterminated (streaming / truncated) forms — the last consumes to end of
/// text so a half-formed leak never flickers into view.
fn leaked_element_span<'a>(
    text: &'a str,
    lt: usize,
    tool_names: &'a HashSet<String>,
) -> Option<(usize, &'a str)> {
    let bytes = text.as_bytes();
    debug_assert_eq!(bytes[lt], b'<');

    // Read the tag name: ASCII alphanumerics immediately after `<`.
    let name_start = lt + 1;
    let mut p = name_start;
    while p < bytes.len() && bytes[p].is_ascii_alphanumeric() {
        p += 1;
    }
    if p == name_start {
        return None;
    }
    let name = &text[name_start..p];

    // A tool call's tag must be followed by a boundary — whitespace, `>`, or
    // `/` — so `<Bashful>` does not match the `Bash` tool.
    let boundary = bytes.get(p).copied();
    let is_boundary = matches!(boundary, Some(b' ' | b'\t' | b'\n' | b'\r' | b'>' | b'/'));
    if !is_boundary {
        return None;
    }
    // Match against the real catalog by exact name.
    let matched = tool_names.get(name)?;

    // Scan the opening tag to its `>`, respecting quoted attribute values so a
    // `command="echo a > b"` doesn't end the tag early.
    let mut q = p;
    let mut quote: Option<u8> = None;
    let mut self_closing = false;
    let mut tag_end: Option<usize> = None;
    while q < bytes.len() {
        let c = bytes[q];
        if let Some(qc) = quote {
            // Inside a quoted attribute value: only the matching quote closes
            // it; every other byte (including `>`) is literal content.
            if c == qc {
                quote = None;
            }
        } else if c == b'"' || c == b'\'' {
            quote = Some(c);
        } else if c == b'>' {
            self_closing = q > p && bytes[q - 1] == b'/';
            tag_end = Some(q + 1);
            break;
        }
        // Any other byte outside quotes is ordinary tag text — advance.
        q += 1;
    }

    let Some(open_end) = tag_end else {
        // Unterminated opening tag (streaming partial or truncation): the rest
        // of the buffer is the forming leak.
        return Some((text.len(), matched.as_str()));
    };
    if self_closing {
        return Some((open_end, matched.as_str()));
    }

    // Paired form: consume through `</Name>` if present; otherwise the opening
    // tag alone is still leaked markup, so suppress just that.
    let close = format!("</{name}>");
    match text[open_end..].find(&close) {
        Some(rel) => Some((open_end + rel + close.len(), matched.as_str())),
        None => Some((open_end, matched.as_str())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn catalog() -> HashSet<String> {
        ["Bash", "Read", "Edit", "Grep", "Write"]
            .into_iter()
            .map(String::from)
            .collect()
    }

    // Normal: the exact screenshot shape — a self-closing `<Bash command=.../>`
    // with an escaped newline — is replaced by the muted notice.
    #[test]
    fn suppresses_self_closing_bash_call_normal() {
        let names = catalog();
        let input = r#"Sure: <Bash command="set -o pipefail\nuname -a" /> done"#;
        let out = sanitize_with(input, &names);
        assert!(matches!(out, Cow::Owned(_)));
        assert!(!out.contains("<Bash"), "raw markup leaked: {out}");
        assert!(
            out.contains("suppressed leaked tool-call markup: Bash"),
            "{out}"
        );
        assert!(out.starts_with("Sure: ") && out.ends_with(" done"), "{out}");
    }

    // Normal: a paired `<Read>…</Read>` element is consumed whole.
    #[test]
    fn suppresses_paired_element_normal() {
        let names = catalog();
        let input = "before <Read path=\"/x\">\nbody\n</Read> after";
        let out = sanitize_with(input, &names);
        assert!(!out.contains("<Read"), "{out}");
        assert!(!out.contains("</Read>"), "{out}");
        assert!(out.contains("before ") && out.contains(" after"), "{out}");
    }

    // Robust: an attribute value containing `>` must not end the tag early.
    #[test]
    fn quoted_gt_does_not_end_tag_robust() {
        let names = catalog();
        let input = r#"<Bash command="echo a > b" /> tail"#;
        let out = sanitize_with(input, &names);
        assert_eq!(out, "⚠ [suppressed leaked tool-call markup: Bash] tail");
    }

    // Robust: a streaming partial (`<Bash` with no closing `>`) suppresses the
    // forming leak through end-of-text instead of flickering raw XML.
    #[test]
    fn unterminated_opening_tag_suppressed_robust() {
        let names = catalog();
        let input = r#"working <Bash command="uname -a"#;
        let out = sanitize_with(input, &names);
        assert!(!out.contains("<Bash"), "{out}");
        assert!(out.starts_with("working "), "{out}");
    }

    // Negative: prose that merely *mentions* tool syntax inside a fenced code
    // block is documentation, not a leak — it must survive untouched.
    #[test]
    fn fenced_code_block_is_left_untouched_negative() {
        let names = catalog();
        let input = "Example:\n```\n<Bash command=\"ls\" />\n```\nend";
        let out = sanitize_with(input, &names);
        assert!(
            matches!(out, Cow::Borrowed(_)),
            "fenced markup rewritten: {out}"
        );
        assert!(out.contains("<Bash command=\"ls\" />"), "{out}");
    }

    // Negative: an unknown / non-tool tag is ordinary markup and is preserved.
    #[test]
    fn unknown_tag_is_preserved_negative() {
        let names = catalog();
        let input = "math: a <b and c> d, plus <Bashful x=\"1\"/>";
        let out = sanitize_with(input, &names);
        assert!(matches!(out, Cow::Borrowed(_)), "{out}");
    }

    // Negative: text with no `<` at all takes the zero-copy fast path.
    #[test]
    fn clean_text_borrows_negative() {
        let names = catalog();
        let out = sanitize_with("just a normal answer about my device", &names);
        assert!(matches!(out, Cow::Borrowed(_)));
    }

    // Robust: multiple leaked calls in one message are all suppressed.
    #[test]
    fn multiple_leaks_all_suppressed_robust() {
        let names = catalog();
        let input = r#"<Read path="/a" /> mid <Grep pattern="x" />"#;
        let out = sanitize_with(input, &names);
        assert_eq!(
            out.matches("suppressed leaked tool-call markup").count(),
            2,
            "{out}"
        );
        assert!(out.contains(" mid "), "{out}");
    }

    // Sanity: the live catalog actually contains the tools we guard against.
    #[test]
    fn live_catalog_contains_bash_normal() {
        assert!(
            TOOL_NAMES.contains("Bash"),
            "expected Bash in live tool catalog"
        );
    }
}
