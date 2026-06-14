//! Collapse-aware body renderer for the subagent task view.
//!
//! `task_view_body_lines` renders `BackgroundTask.messages` (raw strings) to
//! ratatui `Line`s using the markdown pipeline, with auto-collapse for long
//! entries (>80 lines or >5 KB). This is the legacy string-log path used by
//! daemon-launched agents whose events arrive as TaskProgress strings.
//!
//! The structured `MessageView` path (rich tool blocks, reasoning collapse,
//! etc.) is used when `bt.chat_messages` is non-empty.

use ratatui::{
    style::Style,
    text::{Line, Span},
};

use crate::markdown;
use crate::theme::Theme;

/// Per-entry collapse threshold for the subagent task view. A single
/// `BackgroundTask.messages[i]` longer than this (line count) renders as a
/// 5-line preview + a muted "press o to expand" footer until the user toggles
/// it via `viewing_task_expanded`. Smaller than `LargeText::COLLAPSE_LINES`
/// because subagent entries are *individual* turn outputs, not whole tool
/// results — 80 lines is already a wall in a narrow drilled-in pane.
pub const TASK_VIEW_COLLAPSE_LINES: usize = 80;

/// Per-entry byte threshold for the subagent task view. Mirrors the line
/// threshold's reasoning at 5 KB — typical 200-line file dumps blow past this
/// long before they hit `LargeText`'s 30 KB ceiling.
pub const TASK_VIEW_COLLAPSE_BYTES: usize = 5 * 1024;

/// Number of leading lines preserved when an entry collapses. Mirrors v126's
/// `Read` tool preview length so the user gets enough context to decide
/// whether to expand.
const TASK_VIEW_COLLAPSE_PREVIEW_LINES: usize = 5;

/// Render `BackgroundTask.messages` to ratatui `Line`s the same way the main
/// chat handles assistant text: each raw string flows through
/// `markdown::to_lines`, which calls `strip_inline_tool_xml` internally so
/// `<tool_call>…</tool_call>` and `<tool_result>…</tool_result>` markers
/// don't bleed into the screen as literal angle brackets, and code fences
/// pick up syntect highlighting.
///
/// Long entries (>80 lines or >5 KB raw) collapse to a 5-line preview + a
/// muted `… N more lines · press o to expand` row unless their index is in
/// `expanded`. Pure function so tests can assert behavior without standing
/// up a `Frame`/`Buffer`.
///
/// TODO Phase B: when `BackgroundTask.messages` migrates to
/// `Vec<ChatMessage>`, this helper collapses into the same `MessageView`
/// pipeline the main chat uses, picking up tool blocks, reasoning collapse,
/// and diff rendering for free.
pub fn task_view_body_lines(
    messages: &[String],
    expanded: &std::collections::HashSet<usize>,
    theme: &Theme,
    inner_width: usize,
    task_done: bool,
) -> Vec<Line<'static>> {
    let mut out: Vec<Line<'static>> = Vec::new();
    for (idx, raw) in messages.iter().enumerate() {
        let line_count = raw.lines().count();
        // For finished tasks, never auto-collapse — the whole point
        // of opening the task view is to see the result. Only running
        // tasks (whose output is still streaming) get the threshold.
        let collapsible = !task_done
            && (line_count > TASK_VIEW_COLLAPSE_LINES || raw.len() > TASK_VIEW_COLLAPSE_BYTES);
        let is_expanded = expanded.contains(&idx);

        if collapsible && !is_expanded {
            // Truncate the raw string to the first N lines *before* feeding
            // it to the markdown renderer — letting `to_lines` produce 80
            // wrapped lines and then slicing produces visually-broken
            // output (e.g. half a code fence). Slicing the source keeps
            // markdown structure intact.
            let preview: String = raw
                .lines()
                .take(TASK_VIEW_COLLAPSE_PREVIEW_LINES)
                .collect::<Vec<_>>()
                .join("\n");
            let mut preview_lines = markdown::to_lines(&preview, theme, inner_width);
            out.append(&mut preview_lines);
            let hidden = line_count.saturating_sub(TASK_VIEW_COLLAPSE_PREVIEW_LINES);
            out.push(Line::from(Span::styled(
                format!("… {hidden} more lines · press o to expand"),
                Style::default().fg(theme.text_muted),
            )));
        } else {
            let mut lines = markdown::to_lines(raw, theme, inner_width);
            out.append(&mut lines);
        }
    }
    out
}
