use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Position, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::{
        Block, Borders, Cell, Clear, LineGauge, List, ListItem, Paragraph, Row, Table, Wrap,
    },
};

#[allow(unused_imports)]
use ratatui::style::Stylize as _;

use crate::app::{App, ApprovalChoice, SPINNER};
use crate::inline_tools::{self, Segment as InlineSeg};
use crate::input::{filtered_models, palette_items};
use crate::markdown;
use crate::theme::Theme;
use crate::types::*;

pub fn frame(f: &mut Frame, app: &mut App) {
    let t = app.theme;

    f.render_widget(Block::default().style(Style::default().bg(t.bg)), f.area());

    let input_lines = input_visual_line_count(app, f.area().width.saturating_sub(4) as usize);
    let input_height = (input_lines + 2).min(8) as u16;

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(3),
            Constraint::Length(input_height),
            Constraint::Length(2),
        ])
        .split(f.area());

    // When the sessions sidebar is toggled on, split the messages row
    // horizontally: 28-col sidebar on the left, chat on the right. Hidden by
    // default so the chat stays full-width on narrow terminals.
    if app.show_sidebar {
        let split = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Length(28), Constraint::Min(20)])
            .split(chunks[0]);
        sidebar(f, app, split[0]);
        messages(f, app, split[1]);
    } else {
        messages(f, app, chunks[0]);
    }
    input(f, app, chunks[1]);
    status(f, app, chunks[2]);

    if app.show_palette {
        palette(f, app);
    }

    if app.show_model_picker {
        model_picker(f, app);
    }

    if app.show_task_panel {
        task_panel(f, app);
    }

    if app.pending_approval.is_some() {
        approval(f, app);
    }
}

/// Sessions sidebar — toggled with Ctrl+B. Renders the saved-session ids from
/// `~/.config/jfc/sessions/` (cached on `App::session_ids` so render() does no
/// disk I/O). Selecting a row with Enter loads its messages into `App::messages`.
fn sidebar(f: &mut Frame, app: &mut App, area: Rect) {
    let t = app.theme;
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(t.border))
        .title(Span::styled(
            " sessions ",
            Style::default().fg(t.accent).add_modifier(Modifier::BOLD),
        ))
        .title_bottom(
            Line::from(Span::styled(
                " ↑↓ · Enter ",
                Style::default().fg(t.text_muted),
            ))
            .right_aligned(),
        )
        .style(Style::default().bg(t.surface));

    let items: Vec<ListItem> = if app.session_ids.is_empty() {
        vec![ListItem::new(Line::from(Span::styled(
            "  (no saved sessions)",
            Style::default()
                .fg(t.text_muted)
                .add_modifier(Modifier::ITALIC),
        )))]
    } else {
        app.session_ids
            .iter()
            .map(|id| {
                let is_active = app.current_session_id.as_deref() == Some(id.as_str());
                let prefix = if is_active { "● " } else { "  " };
                let row = format!("{prefix}{}", session_display(id));
                ListItem::new(Line::from(Span::styled(
                    row,
                    if is_active {
                        Style::default().fg(t.accent).add_modifier(Modifier::BOLD)
                    } else {
                        Style::default().fg(t.text_primary)
                    },
                )))
            })
            .collect()
    };

    let list = List::new(items)
        .block(block)
        .highlight_style(
            Style::default()
                .bg(t.surface_raised)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("▶ ");
    f.render_stateful_widget(list, area, &mut app.session_list_state);
}

/// Convert a session id like `ses_20260503_212945` into a friendly
/// `2026-05-03 21:29` for the sidebar list.
fn session_display(id: &str) -> String {
    let cleaned = id.strip_prefix("ses_").unwrap_or(id);
    let mut parts = cleaned.splitn(2, '_');
    let date = parts.next().unwrap_or("");
    let time = parts.next().unwrap_or("");
    if date.len() == 8 && time.len() >= 4 {
        format!(
            "{}-{}-{} {}:{}",
            &date[..4],
            &date[4..6],
            &date[6..8],
            &time[..2],
            &time[2..4]
        )
    } else {
        id.to_owned()
    }
}

fn messages(f: &mut Frame, app: &mut App, area: Rect) {
    let t = app.theme;
    let inner_width = area.width.saturating_sub(4) as usize;
    let mut lines: Vec<Line> = Vec::new();

    if app.messages.is_empty() && app.streaming_text.is_empty() {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "What can I help you with?",
            Style::default().fg(t.text_muted),
        )));
    } else {
        for (idx, msg) in app.messages.iter().enumerate() {
            if app.streaming_assistant_idx == Some(idx) && app.is_streaming {
                continue;
            }
            let expanded = app.reasoning_expanded.get(&idx).copied().unwrap_or(false);
            message_lines(
                &mut lines,
                msg,
                &t,
                inner_width,
                expanded,
                idx,
                app.provider.stream_convention(),
            );
            lines.push(Line::from(""));
        }

        if app.is_streaming || !app.streaming_text.is_empty() || !app.streaming_reasoning.is_empty()
        {
            lines.push(Line::from(Span::styled("assistant", t.asst_label())));

            if !app.streaming_reasoning.is_empty() {
                lines.push(Line::from(vec![
                    Span::styled(
                        "∴ Thinking",
                        Style::default()
                            .fg(t.text_muted)
                            .add_modifier(Modifier::ITALIC),
                    ),
                    Span::styled(" [streaming…]", Style::default().fg(t.text_muted)),
                ]));
            }

            render_assistant_text(
                &mut lines,
                &app.streaming_text,
                &t,
                inner_width,
                app.provider.stream_convention(),
            );
            if app.is_streaming {
                lines.push(Line::from(Span::styled(
                    format!(" {} ", SPINNER[app.spinner_frame]),
                    Style::default().fg(t.text_muted),
                )));
                // v126 SpinnerWithTasks: surface the open task list below the
                // streaming indicator so the user can see what's in flight.
                render_task_footer(&mut lines, app);
            }
            lines.push(Line::from(""));
        }
    }

    let content_width = area.width.saturating_sub(2) as usize;
    let wrapped_total: usize = lines
        .iter()
        .map(|line| {
            let w = line.width();
            if w == 0 || content_width == 0 {
                1
            } else {
                (w + content_width - 1) / content_width
            }
        })
        .sum();

    app.total_lines = wrapped_total;

    let visible = area.height.saturating_sub(2) as usize;
    app.viewport_height = visible;

    if app.follow_bottom {
        app.scroll_offset = wrapped_total.saturating_sub(visible);
    } else if app.scroll_offset + visible > wrapped_total {
        app.scroll_offset = wrapped_total.saturating_sub(visible);
    }

    let at_bottom = app.is_at_bottom();
    let title_right = if !at_bottom {
        let remaining = wrapped_total.saturating_sub(app.scroll_offset + visible);
        format!(" ↓ {remaining} more ")
    } else {
        String::new()
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(t.border))
        .title(Span::styled(
            " jfc ",
            Style::default().fg(t.accent).add_modifier(Modifier::BOLD),
        ))
        .title(
            ratatui::widgets::block::Title::from(Span::styled(
                title_right,
                Style::default().fg(t.text_muted),
            ))
            .alignment(ratatui::layout::Alignment::Right),
        )
        .style(Style::default().bg(t.bg));

    let para = Paragraph::new(Text::from(lines))
        .block(block)
        .scroll((app.scroll_offset as u16, 0))
        .wrap(Wrap { trim: false });

    f.render_widget(para, area);
}

/// v126 task footer (`SpinnerWithTasks`). Shows up to 5 open tasks indented
/// under the streaming spinner so the user can see what's queued. Status
/// icons match v126's iP_ table:
///   pending     → □  (muted)
///   in_progress → ▣  (accent, bold subject)
///   completed   → ✓  (success, hidden in default footer view)
/// Overflow summary `… +N pending, M completed` matches the v126 component.
fn render_task_footer(lines: &mut Vec<Line<'static>>, app: &App) {
    let tasks = app.task_store.list(false);
    if tasks.is_empty() {
        return;
    }
    let t = app.theme;
    let counts = app.task_store.counts();

    // Collect completed task IDs so we can filter blocked_by to only open blockers.
    let completed_ids: std::collections::HashSet<&str> = tasks
        .iter()
        .filter(|tk| tk.status == crate::tasks::TaskStatus::Completed)
        .map(|tk| tk.id.as_str())
        .collect();

    // Recently-completed tasks still within the 30 s fade-out window.
    let fade_dur = std::time::Duration::from_secs(30);
    let now = std::time::Instant::now();
    let recently_completed: Vec<&crate::tasks::Task> = tasks
        .iter()
        .filter(|tk| {
            tk.status == crate::tasks::TaskStatus::Completed
                && app
                    .task_completion_times
                    .get(&tk.id)
                    .map_or(false, |&t| now.duration_since(t) < fade_dur)
        })
        .collect();

    // Open (pending / in-progress) tasks first, then recently-completed.
    let open_tasks: Vec<&crate::tasks::Task> = tasks
        .iter()
        .filter(|task| {
            matches!(
                task.status,
                crate::tasks::TaskStatus::Pending | crate::tasks::TaskStatus::InProgress
            )
        })
        .collect();

    let mut visible = 0usize;
    let max_visible = 5usize;

    for tk in open_tasks.iter().chain(recently_completed.iter()) {
        if visible >= max_visible {
            break;
        }
        visible += 1;

        let is_recently_completed = tk.status == crate::tasks::TaskStatus::Completed;

        let (icon, icon_style) = match tk.status {
            crate::tasks::TaskStatus::Pending => ("□ ", Style::default().fg(t.text_muted)),
            crate::tasks::TaskStatus::InProgress => ("▣ ", Style::default().fg(t.accent)),
            crate::tasks::TaskStatus::Completed => (
                "✓ ",
                Style::default().fg(t.success).add_modifier(Modifier::DIM),
            ),
            _ => ("✗ ", Style::default().fg(t.error)),
        };

        let subj_style = if is_recently_completed {
            Style::default()
                .fg(t.text_muted)
                .add_modifier(Modifier::CROSSED_OUT | Modifier::DIM)
        } else if tk.status == crate::tasks::TaskStatus::InProgress {
            Style::default()
                .fg(t.text_primary)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(t.text_secondary)
        };

        let mut spans = vec![
            Span::styled("    ", Style::default()),
            Span::styled(icon, icon_style),
            Span::styled(tk.subject.clone(), subj_style),
        ];

        if let Some(owner) = &tk.owner {
            spans.push(Span::styled(
                format!(" (@{owner})"),
                Style::default()
                    .fg(t.text_muted)
                    .add_modifier(Modifier::ITALIC),
            ));
        }

        // Blocked-by indicator: only show blockers that are still open.
        if !tk.blocked_by.is_empty() {
            let open_blockers: Vec<&str> = tk
                .blocked_by
                .iter()
                .filter(|id| !completed_ids.contains(id.as_str()))
                .map(String::as_str)
                .collect();
            if !open_blockers.is_empty() {
                spans.push(Span::styled(
                    format!(" ▸ blocked by {}", open_blockers.join(", ")),
                    Style::default().fg(t.text_muted),
                ));
            }
        }

        lines.push(Line::from(spans));
    }

    let total_open = counts.pending + counts.in_progress;
    if total_open > visible || counts.completed > 0 {
        let overflow_open = total_open.saturating_sub(visible);
        let mut parts: Vec<String> = Vec::new();
        if overflow_open > 0 {
            parts.push(format!("+{overflow_open} pending"));
        }
        if counts.completed > 0 {
            parts.push(format!("{} completed", counts.completed));
        }
        if !parts.is_empty() {
            lines.push(Line::from(Span::styled(
                format!("    … {}", parts.join(", ")),
                Style::default()
                    .fg(t.text_muted)
                    .add_modifier(Modifier::ITALIC),
            )));
        }
    }
}

/// Render assistant text using the active provider's stream convention. The
/// `AnthropicNative` / `OpenAiNative` paths render verbatim through markdown;
/// the `InlineXmlTags` path splits the text into [`InlineSeg`]s first so
/// `<tool_call>` / `<tool_result>` blocks become compact tool widgets instead
/// of raw XML walls.
///
/// All `Span`s here use `String` (via `format!`) rather than borrowed `&str`
/// because the caller's `Vec<Line<'static>>` is invariant over its lifetime
/// parameter, so any borrowed `&str` from a local would fail to satisfy `'static`.
fn render_assistant_text(
    lines: &mut Vec<Line<'static>>,
    text: &str,
    t: &Theme,
    width: usize,
    convention: crate::provider::StreamConvention,
) {
    use crate::provider::StreamConvention as SC;
    let needs_inline_parse = matches!(convention, SC::InlineXmlTags)
        || (matches!(convention, SC::AnthropicNative | SC::OpenAiNative)
            && inline_tools::contains_inline_tools(text));
    if !needs_inline_parse {
        lines.extend(markdown::to_lines(text, t, width));
        return;
    }
    for seg in inline_tools::parse(text) {
        match seg {
            InlineSeg::Text(s) => {
                if !s.trim().is_empty() {
                    lines.extend(markdown::to_lines(&s, t, width));
                }
            }
            InlineSeg::ToolCall { raw_body, parsed } => {
                let header = match parsed {
                    Some(p) => format!("▸ {} · {}", p.name, truncate_inline(&p.summary, 80)),
                    None => format!("▸ tool_call · {}", truncate_inline(&raw_body, 80)),
                };
                lines.push(Line::from(vec![
                    Span::styled(String::from("┌─ "), Style::default().fg(t.border)),
                    Span::styled(header, Style::default().fg(t.accent)),
                ]));
            }
            InlineSeg::ToolResult(body) => {
                let preview_line_count = 6;
                let total = body.lines().count();
                let mut emitted = 0usize;
                for ln in body.lines().take(preview_line_count) {
                    let clean = sanitize_terminal_text(ln);
                    let truncated = truncate_inline(&clean, width.saturating_sub(4).max(20));
                    lines.push(Line::from(vec![
                        Span::styled(String::from("│ "), Style::default().fg(t.border)),
                        Span::styled(truncated, Style::default().fg(t.text_secondary)),
                    ]));
                    emitted += 1;
                }
                if total > emitted {
                    lines.push(Line::from(vec![
                        Span::styled(String::from("│ "), Style::default().fg(t.border)),
                        Span::styled(
                            format!("… {} more lines", total - emitted),
                            Style::default()
                                .fg(t.text_muted)
                                .add_modifier(Modifier::ITALIC),
                        ),
                    ]));
                }
                lines.push(Line::from(Span::styled(
                    String::from("└─"),
                    Style::default().fg(t.border),
                )));
            }
        }
    }
}

/// Single-line truncate helper local to this module — keeps tool headers tidy.
fn truncate_inline(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_owned();
    }
    let mut out: String = s.chars().take(max.saturating_sub(1)).collect();
    out.push('…');
    out
}

fn message_lines(
    lines: &mut Vec<Line<'static>>,
    msg: &ChatMessage,
    t: &Theme,
    width: usize,
    reasoning_expanded: bool,
    reasoning_key: usize,
    convention: crate::provider::StreamConvention,
) {
    match msg.role {
        Role::User => {
            lines.push(Line::from(Span::styled("you", t.user_label())));
            for part in &msg.parts {
                if let MessagePart::Text(text) = part {
                    lines.extend(markdown::to_lines(text, t, width));
                }
            }
        }
        Role::Assistant => {
            lines.push(Line::from(Span::styled("assistant", t.asst_label())));
            for part in &msg.parts {
                match part {
                    MessagePart::Text(text) => {
                        render_assistant_text(lines, text, t, width, convention);
                    }
                    MessagePart::Reasoning(text) => {
                        if reasoning_expanded {
                            lines.push(Line::from(vec![
                                Span::styled(
                                    "∴ Thinking",
                                    Style::default()
                                        .fg(t.text_muted)
                                        .add_modifier(Modifier::ITALIC),
                                ),
                                Span::styled(
                                    format!(" [Ctrl+O to collapse | key={}]", reasoning_key),
                                    Style::default().fg(t.text_muted),
                                ),
                            ]));
                            for l in text.lines() {
                                lines.push(Line::from(vec![
                                    Span::styled("  ", Style::default()),
                                    Span::styled(l.to_string(), t.reasoning()),
                                ]));
                            }
                        } else {
                            let preview: String = text.chars().take(60).collect();
                            let ellipsis = if text.chars().count() > 60 { "…" } else { "" };
                            lines.push(Line::from(vec![
                                Span::styled(
                                    "∴ Thinking",
                                    Style::default()
                                        .fg(t.text_muted)
                                        .add_modifier(Modifier::ITALIC),
                                ),
                                Span::styled(
                                    format!(" — {preview}{ellipsis}  [Ctrl+O to expand]"),
                                    Style::default().fg(t.text_muted),
                                ),
                            ]));
                        }
                    }
                    MessagePart::Tool(tool) => {
                        lines.extend(tool_lines(tool, t, width));
                    }
                    MessagePart::CompactBoundary { pre_tokens } => {
                        lines.push(Line::from(vec![
                            Span::styled("─── ", Style::default().fg(t.border)),
                            Span::styled(
                                format!("compacted ({pre_tokens} tokens summarized)"),
                                t.muted(),
                            ),
                            Span::styled(" ───", Style::default().fg(t.border)),
                        ]));
                    }
                }
            }
        }
    }
}

fn sanitize_terminal_text(input: &str) -> String {
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

#[cfg(test)]
mod tests {
    use super::sanitize_terminal_text;

    #[test]
    fn sanitize_terminal_text_removes_ansi_and_control_sequences() {
        let input = "ok\u{1b}[31mred\u{1b}[0m\rbad\u{1b}]0;title\u{7}\tend";
        assert_eq!(sanitize_terminal_text(input), "okredbad    end");
    }
}

fn push_prefixed_wrapped(
    out: &mut Vec<Line<'static>>,
    text: &str,
    max_lines: usize,
    width: usize,
    style: Style,
    border: Color,
) {
    let content_width = width.saturating_sub(2).max(1);
    for raw_line in text.lines().take(max_lines) {
        let clean = sanitize_terminal_text(raw_line);
        for chunk in markdown::hard_wrap_str(&clean, content_width) {
            out.push(Line::from(vec![
                Span::styled("│ ", Style::default().fg(border)),
                Span::styled(chunk, style),
            ]));
        }
    }
}

fn push_diff_wrapped(
    out: &mut Vec<Line<'static>>,
    prefix: &str,
    text: &str,
    width: usize,
    style: Style,
    border: Color,
) {
    let clean = sanitize_terminal_text(text);
    let content_width = width.saturating_sub(4).max(1);
    for chunk in markdown::hard_wrap_str(&clean, content_width) {
        out.push(Line::from(vec![
            Span::styled("│ ", Style::default().fg(border)),
            Span::styled(format!("{prefix} {chunk}"), style),
        ]));
    }
}

fn tool_lines(tool: &ToolCall, t: &Theme, width: usize) -> Vec<Line<'static>> {
    let status_style = match tool.status {
        ToolStatus::Pending => Style::default().fg(t.warning),
        ToolStatus::Running => Style::default().fg(t.accent),
        ToolStatus::Complete => Style::default().fg(t.success),
        ToolStatus::Failed => Style::default().fg(t.error),
    };
    let arrow = if tool.is_collapsed { "▶" } else { "▼" };
    let header = format!("{} {} {}", arrow, tool.kind.label(), tool.input.summary());

    let mut out = vec![Line::from(vec![
        Span::styled("┌─ ", Style::default().fg(t.border)),
        Span::styled(header, status_style),
    ])];

    if !tool.is_collapsed {
        match &tool.output {
            ToolOutput::Text(s) => {
                push_prefixed_wrapped(
                    &mut out,
                    s,
                    20,
                    width,
                    Style::default().fg(t.text_secondary),
                    t.border,
                );
            }
            ToolOutput::Command {
                stdout,
                stderr,
                exit_code,
            } => {
                let code_style = match exit_code {
                    Some(0) => Style::default().fg(t.success),
                    Some(_) => Style::default().fg(t.error),
                    None => Style::default().fg(t.text_muted),
                };
                let code_str = exit_code
                    .map(|c| format!("exit {c}"))
                    .unwrap_or_else(|| "running".into());
                out.push(Line::from(vec![
                    Span::styled("│ ", Style::default().fg(t.border)),
                    Span::styled(code_str, code_style),
                ]));
                push_prefixed_wrapped(
                    &mut out,
                    stdout,
                    15,
                    width,
                    Style::default().fg(t.text_secondary),
                    t.border,
                );
                push_prefixed_wrapped(
                    &mut out,
                    stderr,
                    5,
                    width,
                    Style::default().fg(t.error),
                    t.border,
                );
            }
            ToolOutput::Diff(diff) => {
                for hunk in &diff.hunks {
                    push_prefixed_wrapped(
                        &mut out,
                        &hunk.header,
                        1,
                        width,
                        Style::default().fg(t.text_muted),
                        t.border,
                    );
                    for dl in hunk.lines.iter().take(30) {
                        let (prefix, style) = match dl.kind {
                            DiffLineKind::Added => ("+", Style::default().fg(t.success)),
                            DiffLineKind::Removed => ("-", Style::default().fg(t.error)),
                            DiffLineKind::Context => (" ", Style::default().fg(t.text_secondary)),
                        };
                        push_diff_wrapped(&mut out, prefix, &dl.content, width, style, t.border);
                    }
                }
            }
            ToolOutput::FileContent { content, .. } => {
                push_prefixed_wrapped(&mut out, content, 20, width, t.code_block(), t.border);
            }
            ToolOutput::FileList(files) => {
                for f in files.iter().take(15) {
                    out.push(Line::from(vec![
                        Span::styled("│ ", Style::default().fg(t.border)),
                        Span::styled(
                            sanitize_terminal_text(f),
                            Style::default().fg(t.text_secondary),
                        ),
                    ]));
                }
            }
            ToolOutput::Empty => {}
        }
        out.push(Line::from(Span::styled(
            "└─",
            Style::default().fg(t.border),
        )));
    }

    out
}

fn input(f: &mut Frame, app: &mut App, area: Rect) {
    let t = app.theme;
    let border_style = if app.is_streaming {
        Style::default().fg(t.warning)
    } else {
        Style::default().fg(t.border)
    };
    let title = if app.is_streaming {
        format!(" {} streaming… ", SPINNER[app.spinner_frame])
    } else {
        " message ".to_string()
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(border_style)
        .title(Span::styled(title, Style::default().fg(t.text_muted)))
        .style(Style::default().bg(t.surface));
    let inner = block.inner(area);
    f.render_widget(block, area);

    let content_width = inner.width.max(1) as usize;
    let (lines, cursor_row, cursor_col) = input_soft_wrapped_lines(app, content_width);
    let visible_rows = inner.height.max(1) as usize;
    let start = cursor_row.saturating_add(1).saturating_sub(visible_rows);
    let visible = lines
        .iter()
        .skip(start)
        .take(visible_rows)
        .map(|line| {
            Line::from(Span::styled(
                line.clone(),
                Style::default().fg(t.text_primary),
            ))
        })
        .collect::<Vec<_>>();

    f.render_widget(
        Paragraph::new(visible).style(Style::default().bg(t.surface)),
        inner,
    );

    if area.height > 2 && area.width > 2 {
        f.set_cursor_position(Position::new(
            inner
                .x
                .saturating_add(cursor_col as u16)
                .min(inner.right().saturating_sub(1)),
            inner
                .y
                .saturating_add(cursor_row.saturating_sub(start) as u16)
                .min(inner.bottom().saturating_sub(1)),
        ));
    }
}

fn input_visual_line_count(app: &App, content_width: usize) -> usize {
    input_soft_wrapped_lines(app, content_width).0.len().max(1)
}

fn input_soft_wrapped_lines(app: &App, content_width: usize) -> (Vec<String>, usize, usize) {
    let width = content_width.max(1);
    let logical_lines = app.textarea.lines();
    let (cursor_line, cursor_col) = app.textarea.cursor();
    let mut out = Vec::new();
    let mut visual_cursor_row = 0usize;
    let mut visual_cursor_col = 0usize;

    if logical_lines.iter().all(|line| line.is_empty()) {
        out.push("Type a message… (Enter to send, Shift+Enter for newline)".to_string());
        return (out, 0, 0);
    }

    for (line_idx, line) in logical_lines.iter().enumerate() {
        let wrapped = markdown::hard_wrap_str(line, width);
        if line_idx == cursor_line {
            visual_cursor_row = out.len() + cursor_col / width;
            visual_cursor_col = cursor_col % width;
        }
        out.extend(wrapped);
    }

    (out, visual_cursor_row, visual_cursor_col)
}

fn status(f: &mut Frame, app: &App, area: Rect) {
    let t = app.theme;

    // Two-row status: row 0 = info line (model, profile, cwd, hints),
    // row 1 = context-window LineGauge with color-coded usage.
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Length(1)])
        .split(area);

    let cwd_display = {
        let home = std::env::var("HOME").unwrap_or_default();
        app.cwd
            .strip_prefix(&home)
            .map(|rest| format!("~{rest}"))
            .unwrap_or_else(|| app.cwd.clone())
    };

    let msg_count = app.messages.iter().filter(|m| m.role == Role::User).count();

    // OAuth profile badge: subscription_type ("max"/"pro"/…) and seat_tier when set.
    // Only shown if the profile fetch succeeded — otherwise the bar stays terse.
    let profile_badge = match (&app.subscription_type, &app.seat_tier) {
        (Some(sub), Some(tier)) => format!("  {}·{}", sub, tier),
        (Some(sub), None) => format!("  {}", sub),
        (None, Some(tier)) => format!("  {}", tier),
        (None, None) => String::new(),
    };

    let auto_badge = if app.auto_mode.enabled {
        "  ⚡ auto".to_string()
    } else {
        String::new()
    };

    // v126 input queueing: show how many prompts the user has queued behind
    // the active stream. They render in the transcript with a `⏳` prefix and
    // get drained when the turn ends.
    let queue_badge = if !app.queued_prompts.is_empty() {
        format!("  ⏳ {} queued", app.queued_prompts.len())
    } else {
        String::new()
    };

    let left = format!(
        " {}{}{}{}  {}  {} msgs ",
        app.model, profile_badge, auto_badge, queue_badge, cwd_display, msg_count
    );
    let right =
        " Ctrl+C: quit  Ctrl+B: sidebar  Ctrl+P: palette  Ctrl+M: models  Ctrl+O: thinking ";

    let total_width = area.width as usize;
    let right_start = total_width.saturating_sub(right.len());
    // Use char count, not byte length — `left` contains multi-byte chars
    // (⚡, ⏳) that would panic on byte-indexed slicing.
    let left_chars: usize = left.chars().count();
    let left_truncated = if left_chars > right_start.saturating_sub(1) {
        let truncated: String = left.chars().take(right_start.saturating_sub(2)).collect();
        format!("{truncated}…")
    } else {
        left
    };

    let padding = " ".repeat(right_start.saturating_sub(left_truncated.chars().count()));

    let line = Line::from(vec![
        Span::styled(left_truncated, Style::default().fg(t.text_secondary)),
        Span::styled(padding, Style::default().fg(t.text_muted)),
        Span::styled(right, Style::default().fg(t.text_muted)),
    ]);

    f.render_widget(
        Paragraph::new(line).style(Style::default().bg(t.surface)),
        rows[0],
    );

    // Context-window gauge: live ratio of `tool_ctx.approx_tokens` to
    // `max_context_tokens`. Color thresholds match the user's mental model of
    // "safe / watch / about to compact": green <60%, yellow 60–85%, red >85%.
    let used = app.tool_ctx.approx_tokens;
    let max = app.max_context_tokens.max(1);
    let ratio = (used as f64 / max as f64).clamp(0.0, 1.0);
    let pct = (ratio * 100.0).round() as u32;
    let bar_color = if pct < 60 {
        t.success
    } else if pct < 85 {
        t.warning
    } else {
        t.error
    };
    let label = format!(" ctx {}k / {}k · {}% ", used / 1000, max / 1000, pct);
    let gauge = LineGauge::default()
        .filled_style(Style::default().fg(bar_color))
        .unfilled_style(Style::default().fg(t.border))
        .label(Span::styled(label, Style::default().fg(t.text_secondary)))
        .ratio(ratio);
    f.render_widget(gauge, rows[1]);
}

fn palette(f: &mut Frame, app: &App) {
    let t = app.theme;
    let area = f.area();
    let width = 50u16.min(area.width.saturating_sub(4));
    let height = 10u16.min(area.height.saturating_sub(4));
    let x = area.width.saturating_sub(width) / 2;
    let y = area.height.saturating_sub(height) / 2;
    let palette_area = Rect::new(x, y, width, height);

    f.render_widget(Clear, palette_area);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(t.accent))
        .title(Span::styled(
            " Command Palette ",
            Style::default().fg(t.accent),
        ))
        .style(Style::default().bg(t.surface));

    let inner = block.inner(palette_area);
    f.render_widget(block, palette_area);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Min(1)])
        .split(inner);

    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("> ", Style::default().fg(t.accent)),
            Span::styled(
                app.palette_input.clone(),
                Style::default().fg(t.text_primary),
            ),
        ])),
        chunks[0],
    );

    let items: Vec<ListItem> = palette_items(app)
        .iter()
        .enumerate()
        .map(|(i, label)| {
            let style = if i == app.palette_selected {
                Style::default()
                    .fg(t.accent)
                    .add_modifier(Modifier::BOLD)
                    .bg(t.surface_raised)
            } else {
                Style::default().fg(t.text_primary)
            };
            ListItem::new(Line::from(Span::styled(*label, style)))
        })
        .collect();

    f.render_widget(
        List::new(items).style(Style::default().bg(t.surface)),
        chunks[1],
    );
}

/// Color-code each provider so the user can scan the picker by source at a glance.
/// Hardcoded for the providers jfc currently supports — extend when adding a new one.
fn provider_color(provider: &str) -> Color {
    match provider {
        "anthropic" | "anthropic-oauth" => Color::Rgb(204, 120, 50), // Anthropic orange
        "openwebui" => Color::Rgb(100, 180, 200),                    // teal
        _ => Color::Gray,
    }
}

/// Friendly name for the provider badge column. Kept short so it doesn't crowd ids.
fn provider_label(provider: &str) -> &'static str {
    match provider {
        "anthropic" => "API",
        "anthropic-oauth" => "OAuth",
        "openwebui" => "OpenWebUI",
        _ => "?",
    }
}

fn model_picker(f: &mut Frame, app: &mut App) {
    let t = app.theme;
    let area = f.area();
    // Fluid sizing: take up to 90% of the screen, capped at 130 cols / 28 rows.
    // The previous fixed 60×16 truncated long OpenWebUI names like "Anthropic -
    // Claude Haiku 4.5 ($$)" mid-cell.
    let width = (area.width * 9 / 10).min(130).max(60);
    let height = (area.height * 8 / 10).min(28).max(12);
    let x = area.width.saturating_sub(width) / 2;
    let y = area.height.saturating_sub(height) / 2;
    let picker_area = Rect::new(x, y, width, height);

    f.render_widget(Clear, picker_area);

    let total = app.model_picker_models.len();
    let visible = filtered_models(app);
    let title = if app.model_picker_filter.is_empty() {
        format!(" Select Model · {} models ", total)
    } else {
        format!(
            " Select Model · {}/{} matching '{}' ",
            visible.len(),
            total,
            app.model_picker_filter
        )
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(t.accent))
        .title(Span::styled(
            title,
            Style::default().fg(t.accent).add_modifier(Modifier::BOLD),
        ))
        .title_bottom(
            Line::from(vec![
                Span::styled(" ↑↓", Style::default().fg(t.text_muted)),
                Span::styled(" navigate ", Style::default().fg(t.text_secondary)),
                Span::styled("· ", Style::default().fg(t.text_muted)),
                Span::styled("Enter", Style::default().fg(t.text_muted)),
                Span::styled(" select ", Style::default().fg(t.text_secondary)),
                Span::styled("· ", Style::default().fg(t.text_muted)),
                Span::styled("Esc", Style::default().fg(t.text_muted)),
                Span::styled(" cancel ", Style::default().fg(t.text_secondary)),
                Span::styled("· ", Style::default().fg(t.text_muted)),
                Span::styled("type", Style::default().fg(t.text_muted)),
                Span::styled(" filter ", Style::default().fg(t.text_secondary)),
            ])
            .right_aligned(),
        )
        .style(Style::default().bg(t.surface));

    let inner = block.inner(picker_area);
    f.render_widget(block, picker_area);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(2), Constraint::Min(1)])
        .split(inner);

    // Filter input row.
    let filter_line = if app.model_picker_filter.is_empty() {
        Line::from(vec![
            Span::styled("  ⌕ ", Style::default().fg(t.accent)),
            Span::styled(
                "type to filter…",
                Style::default()
                    .fg(t.text_muted)
                    .add_modifier(Modifier::ITALIC),
            ),
        ])
    } else {
        Line::from(vec![
            Span::styled("  ⌕ ", Style::default().fg(t.accent)),
            Span::styled(
                app.model_picker_filter.clone(),
                Style::default()
                    .fg(t.text_primary)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled("▏", Style::default().fg(t.accent)),
        ])
    };
    f.render_widget(Paragraph::new(filter_line), chunks[0]);

    // Build the table.
    let header_style = Style::default()
        .fg(t.text_muted)
        .add_modifier(Modifier::BOLD);
    let header = Row::new(vec![
        Cell::from("  "),
        Cell::from("Model").style(header_style),
        Cell::from("ID").style(header_style),
        Cell::from("Source").style(header_style),
    ])
    .height(1)
    .bottom_margin(0);

    let rows: Vec<Row> = visible
        .iter()
        .map(|m| {
            let is_current = m.id == app.model;
            let marker = if is_current { " ● " } else { "   " };
            let name_style = if is_current {
                Style::default().fg(t.accent).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(t.text_primary)
            };
            let badge_style = Style::default()
                .fg(provider_color(&m.provider))
                .add_modifier(Modifier::BOLD);
            Row::new(vec![
                Cell::from(Span::styled(marker, Style::default().fg(t.accent))),
                Cell::from(Span::styled(m.display_name.clone(), name_style)),
                Cell::from(Span::styled(
                    m.id.clone(),
                    Style::default().fg(t.text_muted),
                )),
                Cell::from(Span::styled(
                    provider_label(&m.provider).to_string(),
                    badge_style,
                )),
            ])
        })
        .collect();

    // Column constraints: marker fixed, name+id share most of the width,
    // source badge fixed. ratatui's Table widget auto-truncates per cell.
    let widths = [
        Constraint::Length(3),
        Constraint::Percentage(45),
        Constraint::Percentage(45),
        Constraint::Length(10),
    ];

    let table = Table::new(rows, widths)
        .header(header)
        .column_spacing(1)
        .row_highlight_style(
            Style::default()
                .bg(t.surface_raised)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("▶ ")
        .style(Style::default().bg(t.surface));

    f.render_stateful_widget(table, chunks[1], &mut app.model_picker_state);
}

fn task_panel(f: &mut Frame, app: &mut App) {
    let t = app.theme;
    let area = f.area();

    let w = (area.width as f32 * 0.80).round() as u16;
    let h = (area.height as f32 * 0.70).round() as u16;
    let x = area.x + (area.width.saturating_sub(w)) / 2;
    let y = area.y + (area.height.saturating_sub(h)) / 2;
    let popup = Rect::new(x, y, w, h);

    f.render_widget(Clear, popup);

    let all_tasks = app.task_store.list(false);
    let counts = app.task_store.counts();

    let completed_ids: std::collections::HashSet<&str> = all_tasks
        .iter()
        .filter(|tk| tk.status == crate::tasks::TaskStatus::Completed)
        .map(|tk| tk.id.as_str())
        .collect();

    let title = format!(
        " Tasks · {} total ({} done, {} in progress, {} pending) ",
        counts.pending + counts.in_progress + counts.completed,
        counts.completed,
        counts.in_progress,
        counts.pending,
    );

    let header = Row::new(vec![
        Cell::from("ID").style(
            Style::default()
                .fg(t.text_muted)
                .add_modifier(Modifier::BOLD),
        ),
        Cell::from("Status").style(
            Style::default()
                .fg(t.text_muted)
                .add_modifier(Modifier::BOLD),
        ),
        Cell::from("Subject").style(
            Style::default()
                .fg(t.text_muted)
                .add_modifier(Modifier::BOLD),
        ),
        Cell::from("Owner").style(
            Style::default()
                .fg(t.text_muted)
                .add_modifier(Modifier::BOLD),
        ),
        Cell::from("Blocked By").style(
            Style::default()
                .fg(t.text_muted)
                .add_modifier(Modifier::BOLD),
        ),
    ]);

    let rows: Vec<Row> = all_tasks
        .iter()
        .map(|tk| {
            let (icon, status_style) = match tk.status {
                crate::tasks::TaskStatus::Pending => {
                    ("□ pending", Style::default().fg(t.text_muted))
                }
                crate::tasks::TaskStatus::InProgress => (
                    "▣ in_progress",
                    Style::default().fg(t.accent).add_modifier(Modifier::BOLD),
                ),
                crate::tasks::TaskStatus::Completed => (
                    "✓ completed",
                    Style::default()
                        .fg(t.success)
                        .add_modifier(Modifier::CROSSED_OUT),
                ),
                _ => ("✗ deleted", Style::default().fg(t.error)),
            };

            let subj_style = if tk.status == crate::tasks::TaskStatus::Completed {
                Style::default()
                    .fg(t.text_muted)
                    .add_modifier(Modifier::CROSSED_OUT)
            } else {
                Style::default().fg(t.text_primary)
            };

            let open_blockers: Vec<&str> = tk
                .blocked_by
                .iter()
                .filter(|id| !completed_ids.contains(id.as_str()))
                .map(String::as_str)
                .collect();

            Row::new(vec![
                Cell::from(tk.id.clone()).style(Style::default().fg(t.text_muted)),
                Cell::from(icon).style(status_style),
                Cell::from(tk.subject.clone()).style(subj_style),
                Cell::from(tk.owner.clone().unwrap_or_default())
                    .style(Style::default().fg(t.text_secondary)),
                Cell::from(open_blockers.join(", ")).style(Style::default().fg(t.text_muted)),
            ])
        })
        .collect();

    // Clamp selection to valid range.
    if !all_tasks.is_empty() {
        let max = all_tasks.len().saturating_sub(1);
        if app.task_panel_selected > max {
            app.task_panel_selected = max;
        }
        app.task_panel_state.select(Some(app.task_panel_selected));
    }

    let table = Table::new(
        rows,
        [
            Constraint::Length(10),
            Constraint::Length(15),
            Constraint::Min(20),
            Constraint::Length(14),
            Constraint::Length(18),
        ],
    )
    .header(header)
    .block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(t.border))
            .title(Span::styled(
                title,
                Style::default().fg(t.accent).add_modifier(Modifier::BOLD),
            ))
            .title_bottom(Span::styled(
                " ↑↓ navigate · Esc close ",
                Style::default()
                    .fg(t.text_muted)
                    .add_modifier(Modifier::ITALIC),
            ))
            .style(Style::default().bg(t.surface)),
    )
    .row_highlight_style(
        Style::default()
            .bg(t.surface_raised)
            .add_modifier(Modifier::BOLD),
    )
    .highlight_symbol("▶ ")
    .style(Style::default().bg(t.surface));

    f.render_stateful_widget(table, popup, &mut app.task_panel_state);
}

fn approval(f: &mut Frame, app: &App) {
    let Some(ref pending) = app.pending_approval else {
        return;
    };
    let t = app.theme;
    let area = f.area();

    // Mutating-tool kinds get the wider modal with a diff preview pane below
    // the choices. Read-only tools (Bash, Glob/Grep with side effects deferred
    // to the bash kind) keep the compact original layout.
    let preview = build_diff_preview(&pending.tool);
    let has_preview = preview.is_some();

    let (width, height) = if has_preview {
        (
            (area.width * 8 / 10).min(110).max(70),
            (area.height * 7 / 10).min(28).max(14),
        )
    } else {
        (
            60u16.min(area.width.saturating_sub(4)),
            10u16.min(area.height.saturating_sub(4)),
        )
    };
    let x = area.width.saturating_sub(width) / 2;
    let y = area.height.saturating_sub(height) / 2;
    let dialog_area = Rect::new(x, y, width, height);

    f.render_widget(Clear, dialog_area);

    let tool_summary = format!(
        "{} {}",
        pending.tool.kind.label(),
        pending.tool.input.summary()
    );

    // Count the queue depth so the user knows there's more behind the current
    // approval. Without this, multi-tool turns silently waited on each modal.
    let queue_len = app.approval_queue.len();
    let title = if queue_len > 0 {
        format!(" Allow tool use? · 1 of {} ", queue_len + 1)
    } else {
        " Allow tool use? ".to_string()
    };
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(t.warning))
        .title(Span::styled(
            title,
            Style::default().fg(t.warning).add_modifier(Modifier::BOLD),
        ))
        .style(Style::default().bg(t.surface));

    let inner = block.inner(dialog_area);
    f.render_widget(block, dialog_area);

    if has_preview {
        // Three rows: summary header (2), choice list (5–6), diff preview (rest).
        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(2),
                Constraint::Length(ApprovalChoice::ALL.len() as u16),
                Constraint::Min(3),
            ])
            .split(inner);

        let truncated: String = tool_summary
            .chars()
            .take((rows[0].width as usize).saturating_sub(2))
            .collect();
        f.render_widget(
            Paragraph::new(vec![
                Line::from(Span::styled(truncated, Style::default().fg(t.text_primary))),
                Line::from(""),
            ]),
            rows[0],
        );

        render_choice_list(f, app, pending, rows[1]);

        let preview_lines = preview.unwrap();
        let preview_block = Block::default()
            .borders(Borders::TOP)
            .border_style(Style::default().fg(t.border))
            .title(Span::styled(" preview ", Style::default().fg(t.text_muted)));
        let inner_preview = preview_block.inner(rows[2]);
        f.render_widget(preview_block, rows[2]);
        f.render_widget(
            Paragraph::new(preview_lines).style(Style::default().bg(t.surface)),
            inner_preview,
        );
    } else {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(2), Constraint::Min(1)])
            .split(inner);
        let truncated: String = tool_summary
            .chars()
            .take((width as usize).saturating_sub(4))
            .collect();
        f.render_widget(
            Paragraph::new(vec![
                Line::from(Span::styled(truncated, Style::default().fg(t.text_primary))),
                Line::from(""),
            ]),
            chunks[0],
        );
        render_choice_list(f, app, pending, chunks[1]);
    }
}

/// Produce a diff/content preview for the pending tool, when applicable. Returns
/// `None` for tools whose effects can't be summarized as a diff (Bash, Read).
fn build_diff_preview(tool: &ToolCall) -> Option<Vec<Line<'static>>> {
    let theme = Theme::dark();
    let t = &theme;
    match &tool.input {
        ToolInput::Edit {
            file_path,
            old_string,
            new_string,
            ..
        } => {
            let mut lines: Vec<Line<'static>> = Vec::new();
            lines.push(Line::from(Span::styled(
                format!(" {file_path}"),
                Style::default()
                    .fg(t.text_secondary)
                    .add_modifier(Modifier::BOLD),
            )));
            lines.push(Line::from(""));
            for (kind, txt) in [("- ", old_string), ("+ ", new_string)] {
                let color = if kind == "- " { t.error } else { t.success };
                for ln in txt.lines().take(20) {
                    lines.push(Line::from(vec![
                        Span::styled(kind.to_owned(), Style::default().fg(color)),
                        Span::styled(ln.to_owned(), Style::default().fg(color)),
                    ]));
                }
            }
            Some(lines)
        }
        ToolInput::Write { file_path, content } => {
            let mut lines: Vec<Line<'static>> = Vec::new();
            lines.push(Line::from(Span::styled(
                format!(" {file_path}  ({} bytes)", content.len()),
                Style::default()
                    .fg(t.text_secondary)
                    .add_modifier(Modifier::BOLD),
            )));
            lines.push(Line::from(""));
            for ln in content.lines().take(30) {
                lines.push(Line::from(Span::styled(
                    ln.to_owned(),
                    Style::default().fg(t.text_primary),
                )));
            }
            let total = content.lines().count();
            if total > 30 {
                lines.push(Line::from(Span::styled(
                    format!("… {} more lines", total - 30),
                    Style::default()
                        .fg(t.text_muted)
                        .add_modifier(Modifier::ITALIC),
                )));
            }
            Some(lines)
        }
        ToolInput::ApplyPatch { patch } => {
            let mut lines: Vec<Line<'static>> = Vec::new();
            for ln in patch.lines().take(40) {
                let color = match ln.chars().next() {
                    Some('+') if !ln.starts_with("+++") => t.success,
                    Some('-') if !ln.starts_with("---") => t.error,
                    Some('@') => t.accent,
                    _ => t.text_secondary,
                };
                lines.push(Line::from(Span::styled(
                    ln.to_owned(),
                    Style::default().fg(color),
                )));
            }
            let total = patch.lines().count();
            if total > 40 {
                lines.push(Line::from(Span::styled(
                    format!("… {} more diff lines", total - 40),
                    Style::default()
                        .fg(t.text_muted)
                        .add_modifier(Modifier::ITALIC),
                )));
            }
            Some(lines)
        }
        ToolInput::Bash { command, .. } => {
            // Bash gets a single-line "preview" so the user sees the exact
            // command that would run. Useful when the summary truncates.
            Some(vec![
                Line::from(Span::styled(
                    String::from("$ "),
                    Style::default().fg(t.accent),
                )),
                Line::from(Span::styled(
                    command.clone(),
                    Style::default().fg(t.text_primary),
                )),
            ])
        }
        _ => None,
    }
}

fn render_choice_list(
    f: &mut Frame,
    _app: &App,
    pending: &crate::app::PendingApproval,
    area: Rect,
) {
    let t = Theme::dark();
    let items: Vec<ListItem> = ApprovalChoice::ALL
        .iter()
        .enumerate()
        .map(|(i, choice)| {
            let style = if i == pending.selected {
                Style::default()
                    .fg(t.warning)
                    .add_modifier(Modifier::BOLD)
                    .bg(t.surface_raised)
            } else {
                Style::default().fg(t.text_primary)
            };
            ListItem::new(Line::from(Span::styled(choice.label(), style)))
        })
        .collect();
    f.render_widget(List::new(items).style(Style::default().bg(t.surface)), area);
}
