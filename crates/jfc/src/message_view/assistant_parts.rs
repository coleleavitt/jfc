use super::core::RenderItem;
use super::*;

pub(super) fn push_reasoning_lines<'a>(
    items: &mut Vec<RenderItem<'a>>,
    text: &'a str,
    expanded: bool,
    t: &Theme,
) {
    if expanded {
        // Header: just `∴ Thinking` with a quiet collapse hint. The old
        // `[Ctrl+O to collapse | key=N]` leaked an internal render index
        // (`key=N`) into the chat — debug noise the user never needed.
        items.push(RenderItem::TextLine(Line::from(vec![
            Span::styled(
                format!("{} Thinking", crate::glyphs::REASONING_HEADER),
                Style::default()
                    .fg(t.text_muted)
                    .add_modifier(Modifier::ITALIC),
            ),
            Span::styled("  ctrl+o to collapse", Style::default().fg(t.text_muted)),
        ])));
        // Reasoning ribbon: each thinking line gets a `┃` prefix in
        // `t.reasoning_fg` so the block visually nests inside the
        // assistant message. The ribbon's own color is the same as
        // the reasoning text, so the indent reads as a soft "this is
        // a thought" guide rather than a competing structural
        // element. Mirrors how Discord / Slack indent quoted blocks.
        for l in text.lines() {
            items.push(RenderItem::TextLine(Line::from(vec![
                Span::styled(
                    format!("{} ", crate::glyphs::REASONING_RIBBON),
                    Style::default().fg(t.reasoning_fg),
                ),
                Span::styled(l.to_string(), t.reasoning()),
            ])));
        }
    } else {
        // The collapsed preview is a single-line teaser. Without flattening
        // newlines / collapsing whitespace runs, multi-line thinking like
        //     "The user wants me to:\n1. Show the diff\n2. Stage..."
        // renders as "The user wants me to:1. Show the diff2. Stage..." —
        // newlines vanish in single-line layout, leaving the digits jammed
        // against the trailing punctuation. Replace ANY whitespace run
        // (including newlines, tabs, multi-space) with a single space so
        // the preview reads naturally.
        const PREVIEW_MAX_CHARS: usize = 60;
        let mut flattened = String::with_capacity(PREVIEW_MAX_CHARS);
        let mut char_count: usize = 0;
        let mut last_was_space = true; // suppress leading whitespace
        let mut truncated = false;
        for ch in text.chars() {
            if char_count >= PREVIEW_MAX_CHARS {
                truncated = true;
                break;
            }
            if ch.is_whitespace() {
                if !last_was_space {
                    flattened.push(' ');
                    char_count += 1;
                    last_was_space = true;
                }
            } else {
                flattened.push(ch);
                char_count += 1;
                last_was_space = false;
            }
        }
        if flattened.ends_with(' ') {
            flattened.pop();
        }
        let ellipsis = if truncated { "…" } else { "" };
        // v126 cli.js never repeats "(ctrl+o to expand)" on every collapsed
        // thinking summary — it's reserved for collapsed long *output* and
        // the diagnostic line. Repeating it on every Thinking row clutters
        // the chat (see screenshot — it appears 5+ times in a single scroll).
        // The summary itself signals collapsibility; the keybind is
        // discoverable through the palette.
        items.push(RenderItem::TextLine(Line::from(vec![
            Span::styled(
                format!("{} Thinking", crate::glyphs::REASONING_HEADER),
                Style::default()
                    .fg(t.text_muted)
                    .add_modifier(Modifier::ITALIC),
            ),
            Span::styled(
                format!(" — {flattened}{ellipsis}"),
                Style::default().fg(t.text_muted),
            ),
        ])));
    }
}

/// Render a `MessagePart::Advisor` payload. Visually distinct from the main
/// agent's reply: italic body in `text_secondary`, with a bolded "ADVISOR:"
/// prefix and a left-side ribbon (`▎`) in the accent color so the user can
/// pick out the advisor's contribution at a glance even when scrolling fast.
///
/// Inline-only — see the module-level note in `advisor.rs` re: side-pane
/// rendering as a follow-up. The hook for a split-pane would be: wrap each
/// `RenderItem::TextLine` produced here in a new `RenderItem::AdvisorPane`
/// variant, then have the layout code carve out a right-side rect and direct
/// those items there. That's out of scope for the inline implementation.
pub(super) fn push_advisor_lines<'a>(items: &mut Vec<RenderItem<'a>>, text: &'a str, t: &Theme) {
    // Header row: bold, accent-colored "ADVISOR:" so it pops against the
    // muted body. Without the bold, the prefix blended into the body and
    // the user couldn't tell where the main reply ended and the advisor
    // started.
    items.push(RenderItem::TextLine(Line::from(vec![
        Span::styled("▎ ", Style::default().fg(t.accent)),
        Span::styled(
            "Advisor:",
            Style::default().fg(t.accent).add_modifier(Modifier::BOLD),
        ),
    ])));
    // Body rows: italic in text_secondary, ribboned with `▎` for the same
    // visual nesting effect as Reasoning. Empty body still gets a single
    // placeholder line so the height calculation in `compute_total_lines`
    // (which adds 1 for empty bodies) lines up with what we render.
    if text.is_empty() {
        items.push(RenderItem::TextLine(Line::from(vec![
            Span::styled("▎ ", Style::default().fg(t.accent)),
            Span::styled(
                "(no advice returned)",
                Style::default()
                    .fg(t.text_muted)
                    .add_modifier(Modifier::ITALIC),
            ),
        ])));
        return;
    }
    for l in text.lines() {
        items.push(RenderItem::TextLine(Line::from(vec![
            Span::styled("▎ ", Style::default().fg(t.accent)),
            Span::styled(
                l.to_string(),
                Style::default()
                    .fg(t.text_secondary)
                    .add_modifier(Modifier::ITALIC),
            ),
        ])));
    }
}

pub(super) fn push_task_status_lines<'a>(
    items: &mut Vec<RenderItem<'a>>,
    ts: &'a TaskStatusPart,
    t: &Theme,
    inner_w: usize,
) {
    let (icon, style) = match ts.status {
        TaskLifecycle::Pending => ("◌", Style::default().fg(t.text_muted)),
        TaskLifecycle::Running => ("◎", Style::default().fg(t.text_primary)),
        TaskLifecycle::Idle => ("⏸", Style::default().fg(t.text_muted)),
        TaskLifecycle::Completed => ("●", Style::default().fg(t.success)),
        TaskLifecycle::Failed => ("✗", Style::default().fg(t.error)),
        TaskLifecycle::Cancelled => ("○", Style::default().fg(t.text_muted)),
    };
    let elapsed = ts
        .elapsed_ms
        .map(|ms| format!(" [{:.1}s]", ms as f64 / 1000.0))
        .unwrap_or_default();

    // TaskCompleted stuffs the entire subagent response (often
    // thousands of chars of markdown — headings, tables, fenced code)
    // into `summary`. Packing that into a single styled Span garbles
    // the output and trashes scroll math, since one logical Line gets
    // word-wrapped as plain text and `RenderItem::TextLine::height`
    // has to walk a multi-KB string per frame. Multi-line summaries
    // get split off into their own markdown-rendered TextLines,
    // matching what the dedicated task view does.
    let summary = ts.summary.as_deref().unwrap_or("");
    let summary_is_block = summary.contains('\n');

    // A single-line summary can still be multi-KB — the whole completion
    // text with its newlines stripped. Showing it as the header word-wraps
    // into a giant run-on blob that duplicates the agent's final message
    // (already rendered just above in the transcript). Cap it to the first
    // line at panel width; the full text lives in the transcript.
    let header_label: String = if summary.is_empty() || summary_is_block {
        ts.description.clone()
    } else {
        let first = summary.lines().next().unwrap_or(summary);
        truncate_str(first, inner_w.saturating_sub(12).max(24))
    };
    let mut spans = vec![
        Span::styled(format!("{icon} task "), style),
        Span::styled(header_label, Style::default().fg(t.text_secondary)),
        Span::styled(elapsed, Style::default().fg(t.text_muted)),
    ];
    if let Some(model) = ts.model.as_deref() {
        let badge = pretty_model_badge(model);
        spans.push(Span::styled(
            format!(" · {badge}"),
            Style::default().fg(t.text_muted),
        ));
    }
    items.push(RenderItem::TextLine(Line::from(spans)));

    if summary_is_block {
        const MAX_LINES: usize = 120;
        let width = inner_w.max(1);
        let mut lines = markdown::to_lines(summary, t, width);
        if lines.len() > MAX_LINES {
            let total = lines.len();
            lines.truncate(MAX_LINES);
            lines.push(Line::from(Span::styled(
                format!(
                    "… {} more lines · open task view to see full result",
                    total.saturating_sub(MAX_LINES)
                ),
                Style::default()
                    .fg(t.text_muted)
                    .add_modifier(Modifier::ITALIC),
            )));
        }
        for line in lines {
            items.push(RenderItem::TextLine(line));
        }
    }

    if let Some(err) = &ts.error {
        items.push(RenderItem::TextLine(Line::from(vec![
            Span::styled("  error: ", Style::default().fg(t.error)),
            Span::styled(err.clone(), Style::default().fg(t.text_secondary)),
        ])));
    }
}

/// Compact a provider-qualified model id into the short tail a human reads at
/// a glance. `bedrock-claude-4-6-haiku` → `haiku`, `claude-opus-4-7` → `opus`,
/// `claude-haiku-4-5-20251001` → `haiku`. Leaves unrecognized ids untouched
/// (truncated only to fit the inline badge), so an "Explore uses haiku while
/// main runs on opus" distinction lands as `haiku` vs `opus`.
pub fn pretty_model_badge(raw: &str) -> String {
    let lower = raw.to_ascii_lowercase();
    for variant in ["haiku", "sonnet", "opus"] {
        if lower.contains(variant) {
            return variant.to_owned();
        }
    }
    truncate_str(raw, 24)
}

pub(super) fn truncate_str(s: &str, max: usize) -> String {
    if max == 0 {
        return String::new();
    }
    let chars: Vec<char> = s.chars().collect();
    if chars.len() <= max {
        s.to_owned()
    } else {
        let trunc: String = chars[..max.saturating_sub(1)].iter().collect();
        format!("{}…", trunc)
    }
}

pub(super) fn sanitize_terminal_text(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\u{1b}' {
            match chars.peek().copied() {
                Some('[') => {
                    chars.next();
                    for next in chars.by_ref() {
                        if ('@'..='~').contains(&next) {
                            break;
                        }
                    }
                }
                Some(']') => {
                    chars.next();
                    let mut saw_esc = false;
                    for next in chars.by_ref() {
                        if saw_esc && next == '\\' {
                            break;
                        }
                        if next == '\u{7}' {
                            break;
                        }
                        saw_esc = next == '\u{1b}';
                    }
                }
                Some(_) => {
                    chars.next();
                }
                None => {}
            }
            continue;
        }
        match ch {
            '\n' => out.push('\n'),
            '\t' => out.push_str("    "),
            ch if ch.is_control() => {}
            ch => out.push(ch),
        }
    }
    out
}

/// Hit-test a list of `(tool_id, screen_rect)` regions against a terminal
/// cell coordinate. Returns the first tool id whose rect contains the
/// click, or `None` if the click landed outside every region.
///
/// "First match wins" is intentional: tool blocks shouldn't overlap in
/// practice, but the tie-break is well-defined and stable.
/// Half-open semantics (`>= x && < x+w`) match ratatui's `Rect::contains`.
pub fn find_tool_at(regions: &[(String, Rect)], col: u16, row: u16) -> Option<&str> {
    let pos = ratatui::layout::Position { x: col, y: row };
    regions
        .iter()
        .find(|(_, rect)| rect.contains(pos))
        .map(|(id, _)| id.as_str())
}

#[cfg(test)]
mod reasoning_preview_tests {
    use super::*;

    fn collapsed_preview(text: &str) -> String {
        let mut items: Vec<RenderItem<'_>> = Vec::new();
        let theme = crate::theme::Theme::dark();
        push_reasoning_lines(&mut items, text, false, &theme);
        // The single line we pushed has two spans; the second contains the
        // preview. Concatenate the visible text so tests can assert on it.
        match items.into_iter().next() {
            Some(RenderItem::TextLine(line)) => line
                .spans
                .into_iter()
                .map(|s| s.content.into_owned())
                .collect::<String>(),
            _ => String::new(),
        }
    }

    #[test]
    fn flattens_newlines_in_multiline_thinking_normal() {
        let s =
            collapsed_preview("The user wants me to:\n1. Show the git diff\n2. Stage the changes");
        assert!(
            s.contains("The user wants me to: 1. Show"),
            "newlines should be replaced with spaces; got: {s:?}"
        );
        assert!(!s.contains(":1."), "digits jammed into prior text: {s:?}");
    }

    #[test]
    fn collapses_whitespace_runs_normal() {
        let s = collapsed_preview("aaa     bbb\t\tccc");
        assert!(s.contains("aaa bbb ccc"), "got: {s:?}");
    }

    #[test]
    fn handles_leading_whitespace_robust() {
        // A reasoning that starts with newlines/spaces shouldn't render with
        // a leading run of blanks before the first word.
        let s = collapsed_preview("\n\n   Thinking through the problem now");
        // The visible preview begins after " — "; ensure the next char is
        // a letter, not space.
        let dash = s.find(" — ").expect("preview separator missing");
        let after = &s[dash + " — ".len()..];
        assert!(
            after.starts_with("Thinking"),
            "leading whitespace not trimmed; got: {after:?}"
        );
    }

    #[test]
    fn no_per_line_expand_hint_normal() {
        // v126 doesn't put `(ctrl+o to expand)` on every collapsed thinking
        // — repeating it 5+ times in one scroll clutters the chat. The
        // summary itself signals collapsibility; the binding is in the
        // palette. Pin this so a future "helpful" change doesn't add it back.
        let s = collapsed_preview("a quick thinking note");
        assert!(!s.to_lowercase().contains("ctrl+o"), "got: {s:?}");
        assert!(!s.to_lowercase().contains("expand"), "got: {s:?}");
    }

    #[test]
    fn empty_reasoning_does_not_panic_robust() {
        // No content -> empty preview, no ellipsis. Just shouldn't panic.
        let s = collapsed_preview("");
        assert!(s.contains("∴ Thinking"));
    }

    #[test]
    fn unicode_grapheme_count_correct_robust() {
        // 60-char cap must be by char count, not byte count, so emoji /
        // CJK don't truncate mid-codepoint. Input of 80 CJK chars (each
        // 3 bytes) -> 80 chars total, capped to 60, ellipsis present.
        let input = "日".repeat(80);
        let s = collapsed_preview(&input);
        assert!(s.contains('…'), "expected truncation indicator; got: {s:?}");
    }

    #[test]
    fn no_ellipsis_when_under_cap_robust() {
        // Whitespace collapse can shrink the visible preview below the
        // input's char count, but that's not truncation — no ellipsis.
        let s = collapsed_preview("a   b   c");
        assert!(!s.contains('…'), "false truncation marker; got: {s:?}");
    }
}
