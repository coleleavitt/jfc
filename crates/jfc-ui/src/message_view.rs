use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Paragraph, Widget, Wrap},
};

use crate::app::App;
use crate::markdown;
use crate::theme::Theme;
use crate::types::*;

pub struct MessageView<'a> {
    pub app: &'a App,
}

pub fn message_view_total_lines(app: &App, inner_w: usize) -> usize {
    let items = build_render_items(app, inner_w);
    items.iter().map(|i| i.height(inner_w)).sum()
}

impl Widget for MessageView<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let t = self.app.theme;
        let width = area.width;
        let inner_w = width as usize;

        let items = build_render_items(self.app, inner_w);

        let total_h: usize = items.iter().map(|i| i.height(inner_w)).sum();
        let max_scroll = total_h.saturating_sub(area.height as usize);
        let scroll = self.app.scroll_offset.min(max_scroll);

        let mut lines_skipped: usize = 0;
        let mut y = area.y;
        let bottom = area.y + area.height;

        for item in &items {
            if y >= bottom {
                break;
            }
            let h = item.height(inner_w);
            if lines_skipped + h <= scroll {
                lines_skipped += h;
                continue;
            }
            let item_scroll_skip = scroll.saturating_sub(lines_skipped);
            let visible_h = h.saturating_sub(item_scroll_skip);
            let render_h = (visible_h as u16).min(bottom - y);
            if render_h == 0 {
                lines_skipped += h;
                continue;
            }

            let item_area = Rect {
                x: area.x,
                y,
                width,
                height: render_h,
            };
            // Record screen rect for any tool block we're about to paint so
            // the mouse handler can hit-test left clicks against the
            // currently-visible tools. Tools partially clipped by scroll
            // still get a hit region for the visible portion — clicking
            // any visible row of a tool toggles its `expanded` state.
            if let RenderItem::ToolBlock(tool) = item {
                self.app
                    .tool_hit_regions
                    .borrow_mut()
                    .push((tool.id.clone(), item_area));
            }
            item.render_with_skip(item_area, buf, t, item_scroll_skip);
            y += render_h;
            lines_skipped += h;
        }
    }
}

enum RenderItem<'a> {
    TextLine(Line<'a>),
    ToolBlock(&'a ToolCall),
    Blank,
}

impl<'a> RenderItem<'a> {
    fn height(&self, width: usize) -> usize {
        match self {
            RenderItem::Blank => 1,
            RenderItem::TextLine(line) => {
                let w = line.width();
                if w == 0 || width == 0 {
                    1
                } else {
                    w.div_ceil(width).max(1)
                }
            }
            RenderItem::ToolBlock(tool) => tool_block_height(tool, width),
        }
    }

    fn render_with_skip(&self, area: Rect, buf: &mut Buffer, t: Theme, skip: usize) {
        match self {
            RenderItem::Blank => {}
            RenderItem::TextLine(line) => {
                Paragraph::new(line.clone())
                    .wrap(Wrap { trim: false })
                    .scroll((skip as u16, 0))
                    .style(Style::default().bg(t.bg))
                    .render(area, buf);
            }
            RenderItem::ToolBlock(tool) => {
                render_tool_block(tool, area, t, buf, skip);
            }
        }
    }
}

fn build_render_items<'a>(app: &'a App, inner_w: usize) -> Vec<RenderItem<'a>> {
    let t = app.theme;
    let mut items: Vec<RenderItem<'a>> = Vec::new();

    for (idx, msg) in app.messages.iter().enumerate() {
        // The streaming-placeholder assistant message gets mutated in place
        // by the StreamChunk handler — text/reasoning chunks append to its
        // parts as they arrive. We render it inline like any other message
        // so the user sees content arriving in the chat timeline (rather
        // than a duplicate "assistant" header pinned to the bottom). When
        // the placeholder still has no content (parts are all empty Text /
        // empty Reasoning), skip it so we don't show a label with nothing
        // under it — the dedicated spinner row above the input is the
        // visual cue that work is in flight.
        let is_streaming_placeholder = app.streaming_assistant_idx == Some(idx) && app.is_streaming;
        if is_streaming_placeholder {
            let has_content = msg.parts.iter().any(|p| match p {
                MessagePart::Text(s) => !s.is_empty(),
                MessagePart::Reasoning(s) => !s.is_empty(),
                _ => true,
            });
            if !has_content {
                continue;
            }
        }

        let label_line = match msg.role {
            Role::User => Line::from(Span::styled("you", t.user_label())),
            Role::Assistant => Line::from(Span::styled("assistant", t.asst_label())),
        };
        items.push(RenderItem::TextLine(label_line));

        let reasoning_expanded = app.reasoning_expanded.get(&idx).copied().unwrap_or(false);

        for part in &msg.parts {
            match part {
                MessagePart::Text(text) => {
                    for line in markdown::to_lines(text, &t, inner_w) {
                        items.push(RenderItem::TextLine(line));
                    }
                }
                MessagePart::Reasoning(text) => {
                    push_reasoning_lines(&mut items, text, reasoning_expanded, idx, &t);
                }
                MessagePart::Tool(tool) => {
                    items.push(RenderItem::ToolBlock(tool));
                }
                MessagePart::TaskStatus(ts) => {
                    push_task_status_lines(&mut items, ts, &t);
                }
                MessagePart::CompactBoundary { pre_tokens } => {
                    items.push(RenderItem::TextLine(Line::from(vec![
                        Span::styled("─── ", Style::default().fg(t.border)),
                        Span::styled(
                            format!("compacted ({pre_tokens} tokens summarized)"),
                            t.muted(),
                        ),
                        Span::styled(" ───", Style::default().fg(t.border)),
                    ])));
                }
            }
        }

        // v126 cli.js:341376 — `Cooked for Nm Ns` post-turn footer with a
        // randomized past-tense verb. Only attached to completed assistant
        // turns (skip user messages, skip the in-flight placeholder which
        // already has its own spinner row). `msg.elapsed` carries the
        // duration string written at StreamDone time.
        if msg.role == Role::Assistant && !is_streaming_placeholder {
            if let Some(elapsed) = &msg.elapsed {
                items.push(RenderItem::TextLine(Line::from(Span::styled(
                    format!("  {elapsed}"),
                    Style::default()
                        .fg(t.text_muted)
                        .add_modifier(Modifier::DIM),
                ))));
            }
        }

        items.push(RenderItem::Blank);
    }

    // Pre-spinner-row architecture used to emit a duplicate "assistant"
    // header + streaming text + spinner here, on top of also pushing those
    // chunks into the placeholder message's parts via StreamChunk. With
    // the dedicated `spinner_row()` widget above the input bar (see
    // `render::spinner_row`), this block is dead weight — it produced the
    // doubled `assistant / ∴ Thinking [streaming…]` the user reported.
    // The placeholder now renders inline like any other message; when it
    // has no content yet the loop above skips it so only the spinner row
    // signals activity.

    items
}

fn tool_block_height(tool: &ToolCall, inner_w: usize) -> usize {
    if tool.is_collapsed {
        return 1;
    }
    // v126-style flat layout: 1 title row + (optional) bash-continuation
    // rows + content rows. Continuation rows show lines 2+ of a multi-
    // line bash command — the title only fits the first line — so the
    // user sees the heredoc body, not just `cat > file <<'EOF'`.
    let cont = bash_continuation_lines(tool).len();
    1 + cont + tool_content_height(&tool.output, inner_w.saturating_sub(2))
}

pub fn tool_block_height_pub(tool: &ToolCall, inner_w: usize) -> usize {
    tool_block_height(tool, inner_w)
}

fn tool_content_height(output: &ToolOutput, content_w: usize) -> usize {
    match output {
        ToolOutput::Empty => 0,

        ToolOutput::Text(s) => wrapped_line_count(s, content_w).min(80),

        ToolOutput::LargeText(lt) => {
            if lt.line_count > LargeText::COLLAPSE_LINES
                || lt.content.len() > LargeText::COLLAPSE_BYTES
            {
                1
            } else {
                wrapped_line_count(&lt.content, content_w).min(80)
            }
        }

        ToolOutput::Command { stdout, stderr, .. } => {
            1 + if stdout.is_empty() {
                0
            } else {
                wrapped_line_count(stdout, content_w).min(80)
            } + if stderr.is_empty() {
                0
            } else {
                wrapped_line_count(stderr, content_w).min(80)
            }
        }

        ToolOutput::Diff(diff) => {
            // 1 row for the `□ Added N lines` summary + sum of hunk
            // heights (header + clamped lines + optional `… N more`).
            let summary_row = if diff.additions > 0 || diff.deletions > 0 {
                1
            } else {
                0
            };
            summary_row
                + diff
                    .hunks
                    .iter()
                    .map(|h| 1 + h.lines.len().min(50) + if h.lines.len() > 50 { 1 } else { 0 })
                    .sum::<usize>()
        }

        ToolOutput::FileContent { content, .. } => wrapped_line_count(content, content_w).min(80),

        ToolOutput::FileList(files) => files.len().min(20) + if files.len() > 20 { 1 } else { 0 },
    }
}

fn wrapped_line_count(text: &str, width: usize) -> usize {
    if width == 0 {
        return text.lines().count().max(1);
    }
    text.lines()
        .map(|line| {
            let chars = line.chars().count();
            if chars == 0 { 1 } else { chars.div_ceil(width) }
        })
        .sum::<usize>()
        .max(if text.is_empty() { 0 } else { 1 })
}

fn render_tool_block(tool: &ToolCall, area: Rect, t: Theme, buf: &mut Buffer, skip: usize) {
    if area.height == 0 {
        return;
    }

    if tool.is_collapsed {
        if skip == 0 {
            let header = build_collapsed_header(tool, &t, area.width as usize);
            Paragraph::new(header)
                .style(Style::default().bg(t.bg))
                .render(Rect { height: 1, ..area }, buf);
        }
        return;
    }

    // Animate the icon for Running tools by deriving the frame from
    // wall-clock time (every TICK_MS ticks → next frame). This sidesteps
    // having to thread `app.spinner_frame` through `MessageView` while
    // still pulsing in lockstep with the top-of-input spinner. v126
    // cli.js:323158 does the same — frame cycles indexed by elapsed ms.
    let frame_idx = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| (d.as_millis() / 80) as usize)
        .unwrap_or(0);
    let (status_icon, status_style) = tool_status_icon_animated(tool, &t, frame_idx);
    let title_spans = build_title_spans(
        tool,
        &t,
        status_icon,
        status_style,
        area.width.saturating_sub(2) as usize,
    );

    let full_h = tool_block_height(tool, area.width as usize) as u16;
    if skip >= full_h as usize {
        return;
    }

    // Row 0: title — only draw it if scrolling hasn't pushed it out of view.
    if skip == 0 && area.height > 0 {
        let title_area = Rect { height: 1, ..area };
        Paragraph::new(Line::from(title_spans))
            .style(Style::default().bg(t.bg))
            .render(title_area, buf);
    }

    // Rows 1..N: indented content, 2-char left gutter to mirror v126's
    // visual hierarchy (the title's leading status icon "owns" the column,
    // so the body sits two cells in). When `skip == 0` we already consumed
    // row 0 for the title; otherwise skip-1 rows of content have scrolled
    // off and we keep going from there.
    let title_consumed: u16 = if skip == 0 { 1 } else { 0 };
    let content_skip = skip.saturating_sub(1);
    let content_y = area.y + title_consumed;
    let content_h = area.height.saturating_sub(title_consumed);
    if content_h == 0 {
        return;
    }
    let content_area = Rect {
        x: area.x + 2,
        y: content_y,
        width: area.width.saturating_sub(2),
        height: content_h,
    };
    if content_area.width > 0 {
        render_tool_content_with_skip(tool, content_area, t, buf, content_skip);
    }
}

fn build_collapsed_header<'a>(tool: &'a ToolCall, t: &Theme, width: usize) -> Line<'a> {
    let (status_icon, status_style) = tool_status_icon(tool, t);
    let mut spans = vec![
        Span::styled("▶ ", Style::default().fg(t.text_muted)),
        Span::styled(status_icon.to_owned(), status_style),
        Span::raw(" "),
    ];
    spans.extend(build_header_inner_spans(tool, t, width.saturating_sub(6)));
    Line::from(spans)
}

/// Cap the visible tool title at a sensible length even on wide terminals.
/// A 200-column terminal showing a sprawling `bash uname -a && cat … |
/// head -5 && echo --- && lscpu | head -10 && echo --- && free -h`
/// across a full row reads as one giant ribbon of grey instead of as a
/// labeled invocation. v126 keeps tool titles brief; the full command is
/// visible in the expanded body. Tunable via `JFC_TOOL_TITLE_WIDTH` for
/// users who want the full command on a wide screen.
fn tool_title_width_cap() -> usize {
    std::env::var("JFC_TOOL_TITLE_WIDTH")
        .ok()
        .and_then(|s| s.parse::<usize>().ok())
        .filter(|n| *n >= 20)
        .unwrap_or(100)
}

fn build_title_spans<'a>(
    tool: &'a ToolCall,
    t: &Theme,
    status_icon: &'static str,
    status_style: Style,
    width: usize,
) -> Vec<Span<'a>> {
    let mut spans = vec![
        Span::styled("▼ ", Style::default().fg(t.text_muted)),
        Span::styled(status_icon.to_owned(), status_style),
        Span::raw(" "),
    ];
    let effective = width.min(tool_title_width_cap()).saturating_sub(4);
    spans.extend(build_header_inner_spans(tool, t, effective));
    spans
}

fn build_header_inner_spans<'a>(tool: &'a ToolCall, t: &Theme, max_w: usize) -> Vec<Span<'a>> {
    let kind_label = tool.kind.label();
    let summary = tool.input.summary();

    match &tool.input {
        ToolInput::Bash { command, .. } => {
            // `Bash(<command>)` — function-call shape matches v126 and
            // reads consistently next to `Update(...)` / `Read(...)`.
            // First-line-of-command since multi-line bash bodies are
            // common and we cap at title width anyway.
            let first_line = command.lines().next().unwrap_or(command);
            let cmd = truncate_str(first_line, max_w.saturating_sub(8));
            vec![
                Span::styled("Bash", Style::default().fg(t.accent)),
                Span::styled("(", Style::default().fg(t.text_muted)),
                Span::styled(cmd, Style::default().fg(t.text_primary)),
                Span::styled(")", Style::default().fg(t.text_muted)),
            ]
        }
        ToolInput::Edit { file_path, .. } => {
            // v126 style (cli.js diff renderer): `Update(<path>)` rather
            // than `edit <path>` — the title reads as a function call,
            // matching `Bash(<command>)` and `Read(<path>)` shapes so the
            // tool list scans uniformly.
            let path = truncate_str(file_path, max_w.saturating_sub(8));
            vec![
                Span::styled("Update", Style::default().fg(t.accent)),
                Span::styled("(", Style::default().fg(t.text_muted)),
                Span::styled(path, Style::default().fg(t.text_primary)),
                Span::styled(")", Style::default().fg(t.text_muted)),
            ]
        }
        ToolInput::Write { file_path, .. } => {
            let path = truncate_str(file_path, max_w.saturating_sub(8));
            vec![
                Span::styled("Write", Style::default().fg(t.accent)),
                Span::styled("(", Style::default().fg(t.text_muted)),
                Span::styled(path, Style::default().fg(t.text_primary)),
                Span::styled(")", Style::default().fg(t.text_muted)),
            ]
        }
        ToolInput::Read { file_path, .. } => {
            let path = truncate_str(file_path, max_w.saturating_sub(7));
            vec![
                Span::styled("Read", Style::default().fg(t.accent)),
                Span::styled("(", Style::default().fg(t.text_muted)),
                Span::styled(path, Style::default().fg(t.text_secondary)),
                Span::styled(")", Style::default().fg(t.text_muted)),
            ]
        }
        _ => {
            let s = truncate_str(&summary, max_w.saturating_sub(kind_label.len() + 1));
            vec![
                Span::styled(format!("{kind_label} "), Style::default().fg(t.text_muted)),
                Span::styled(s, Style::default().fg(t.text_secondary)),
            ]
        }
    }
}

/// Icon + style for a tool's status. Static for the resolved states
/// (Complete/Failed) and the queued state (Pending). The Running state
/// returns a frame-aware icon so the caller — typically the main
/// renderer with `app.spinner_frame` in hand — can animate it. v126
/// cli.js:323158 pulses tool-use mode at 1Hz via `Math.sin`. We do the
/// equivalent with the same 6-frame spinner cycle the top-of-input
/// spinner uses, so a Running bash tool reads as alive instead of
/// frozen.
fn tool_status_icon(tool: &ToolCall, t: &Theme) -> (&'static str, Style) {
    match tool.status {
        ToolStatus::Pending => ("○", Style::default().fg(t.warning)),
        ToolStatus::Running => ("◌", Style::default().fg(t.accent)),
        ToolStatus::Complete => ("●", Style::default().fg(t.success)),
        ToolStatus::Failed => ("✗", Style::default().fg(t.error)),
    }
}

/// Solid-blink variant for Running tools — single fixed icon (`●`,
/// same shape as Complete) toggling between accent and muted color at
/// ~500ms cadence so the bullet visibly *throbs* without rotating
/// glyphs that are easy to miss. Mirrors v126's `Math.sin` opacity
/// pulse (cli.js:323158): we can't fade alpha in a TUI, but
/// alternating between two intensities at a steady beat reads as the
/// same heartbeat.
///
/// Why a solid blink, not a Braille spin: rotating glyphs (`⠋ ⠙ ⠹`)
/// jitter the eye and are hard to glance at when scanning a list of
/// tools — the user can't tell at a glance which is running. A
/// fixed-icon-with-pulsing-color reads as "still running" instantly
/// while the *shape* matches the resolved Complete state, so the
/// transition Running→Complete is just the color settling.
pub fn tool_status_icon_animated(
    tool: &ToolCall,
    t: &Theme,
    frame: usize,
) -> (&'static str, Style) {
    match tool.status {
        ToolStatus::Running => {
            // 6 ticks at 80ms each = ~480ms per half-cycle, ≈1Hz blink.
            // Even = bright accent (BOLD); odd = muted (no modifier).
            // Intentionally a sharp transition — matches the user's
            // mental model of "blink" rather than a smooth fade.
            let bright = (frame / 6) % 2 == 0;
            let style = if bright {
                Style::default()
                    .fg(t.accent)
                    .add_modifier(ratatui::style::Modifier::BOLD)
            } else {
                Style::default().fg(t.text_muted)
            };
            ("●", style)
        }
        _ => tool_status_icon(tool, t),
    }
}

fn border_color_for_status(tool: &ToolCall, t: &Theme) -> Color {
    match tool.status {
        ToolStatus::Pending => t.warning,
        ToolStatus::Running => t.accent,
        ToolStatus::Complete => t.border,
        ToolStatus::Failed => t.error,
    }
}

fn render_tool_content_clipped(tool: &ToolCall, area: Rect, t: Theme, buf: &mut Buffer) {
    render_tool_content_with_skip(tool, area, t, buf, 0);
}

/// Lines 2+ of a multi-line Bash command (the heredoc body, the `&&`
/// chain wrapped, etc.) — the title only shows line 1 due to the
/// title-width cap. Without rendering the rest, a `cat > file << 'EOF'\n
/// <... source ...>\nEOF` invocation would only ever show the `cat >`
/// line, hiding what was actually written. Mirrors v126's behavior of
/// showing the full command body as part of the tool block.
fn bash_continuation_lines(tool: &ToolCall) -> Vec<String> {
    if let ToolInput::Bash { command, .. } = &tool.input {
        let lines: Vec<&str> = command.lines().collect();
        if lines.len() > 1 {
            return lines.iter().skip(1).map(|s| (*s).to_owned()).collect();
        }
    }
    Vec::new()
}

fn render_tool_content_with_skip(
    tool: &ToolCall,
    area: Rect,
    t: Theme,
    buf: &mut Buffer,
    skip: usize,
) {
    if area.height == 0 {
        return;
    }
    // For multi-line Bash commands, show the rest of the command body
    // before the output. Each continuation line is prefixed with `┆ ` in
    // muted color so it visually nests under the title and reads as
    // continuation of the same invocation.
    let bash_cont = bash_continuation_lines(tool);
    let mut local_skip = skip;
    let mut content_y = area.y;
    let mut remaining_h = area.height;
    if !bash_cont.is_empty() {
        for line in &bash_cont {
            if remaining_h == 0 {
                break;
            }
            if local_skip > 0 {
                local_skip -= 1;
                continue;
            }
            let row = Rect {
                x: area.x,
                y: content_y,
                width: area.width,
                height: 1,
            };
            // Truncate to row width so a 200-col heredoc line doesn't
            // spill into the input border below.
            let max_w = (area.width as usize).saturating_sub(2);
            let truncated: String = if line.chars().count() > max_w && max_w > 1 {
                let mut s: String = line.chars().take(max_w.saturating_sub(1)).collect();
                s.push('…');
                s
            } else {
                line.clone()
            };
            Paragraph::new(Line::from(vec![
                Span::styled("┆ ", Style::default().fg(t.text_muted)),
                Span::styled(truncated, Style::default().fg(t.text_secondary)),
            ]))
            .style(Style::default().bg(t.bg))
            .render(row, buf);
            content_y += 1;
            remaining_h -= 1;
        }
    }
    if remaining_h == 0 {
        return;
    }
    let area = Rect {
        x: area.x,
        y: content_y,
        width: area.width,
        height: remaining_h,
    };
    let skip = local_skip;
    match &tool.output {
        ToolOutput::Empty => {}
        ToolOutput::Text(s) => {
            let lang = infer_lang_from_tool(tool);
            if let Some(lang) = lang.as_deref() {
                render_highlighted_with_line_numbers(lang, s, area, t, buf, skip);
            } else {
                render_text_block_skip(s, area, t.text_secondary, t, buf, skip);
            }
        }
        ToolOutput::LargeText(lt) => {
            if lt.line_count > LargeText::COLLAPSE_LINES
                || lt.content.len() > LargeText::COLLAPSE_BYTES
            {
                if skip == 0 {
                    Paragraph::new(Line::from(Span::styled(
                        format!("[{} · press o to expand]", lt.size_label()),
                        Style::default().fg(t.text_muted),
                    )))
                    .style(Style::default().bg(t.bg))
                    .render(area, buf);
                }
            } else {
                render_text_block_skip(&lt.content, area, t.text_secondary, t, buf, skip);
            }
        }
        ToolOutput::Command {
            stdout,
            stderr,
            exit_code,
        } => render_command_output_skip(stdout, stderr, *exit_code, area, t, buf, skip),
        ToolOutput::Diff(diff) => render_diff_skip(diff, area, t, buf, skip),
        ToolOutput::FileContent {
            content, language, ..
        } => {
            let hl_lang = if language.is_empty() {
                "rs"
            } else {
                language.as_str()
            };
            render_highlighted_block_skip(hl_lang, content, area, t, buf, skip);
        }
        ToolOutput::FileList(files) => render_file_list_skip(files, area, t, buf, skip),
    }
}

fn render_text_block_skip(
    text: &str,
    area: Rect,
    text_style: Color,
    t: Theme,
    buf: &mut Buffer,
    skip: usize,
) {
    let max_lines = 80usize;
    let width = area.width as usize;
    let mut lines: Vec<Line<'static>> = Vec::new();
    let mut count = 0usize;

    'outer: for raw in text.lines() {
        let wrapped = markdown::hard_wrap_str(raw, width.max(1));
        for chunk in wrapped {
            if count >= max_lines {
                lines.push(Line::from(Span::styled(
                    format!("… truncated ({} lines total)", text.lines().count()),
                    Style::default().fg(t.text_muted),
                )));
                break 'outer;
            }
            lines.push(Line::from(Span::styled(
                sanitize_terminal_text(&chunk),
                Style::default().fg(text_style),
            )));
            count += 1;
        }
    }

    Paragraph::new(lines)
        .style(Style::default().bg(t.bg))
        .scroll((skip as u16, 0))
        .render(area, buf);
}

fn render_highlighted_with_line_numbers(
    lang: &str,
    text: &str,
    area: Rect,
    t: Theme,
    buf: &mut Buffer,
    skip: usize,
) {
    let (line_numbers, code) = split_line_numbers(text);
    let code_ref = code.as_deref().unwrap_or(text);

    let gutter_width = line_numbers
        .as_ref()
        .map(|nums| nums.iter().map(|n| n.len()).max().unwrap_or(0))
        .unwrap_or(0);

    let gutter_cols = if gutter_width > 0 {
        gutter_width + 3
    } else {
        2
    };
    let code_w = (area.width as usize).saturating_sub(gutter_cols).max(10);

    let highlighted = markdown::highlight_code_raw(lang, code_ref, code_w, &t);

    let gutter_style = Style::default().fg(t.text_muted);
    let separator_style = Style::default().fg(t.border);

    let lines: Vec<Line<'static>> = highlighted
        .into_iter()
        .enumerate()
        .map(|(i, mut hl_line)| {
            let mut spans = if let Some(nums) = &line_numbers {
                let num = nums.get(i).map(|s| s.as_str()).unwrap_or("");
                vec![
                    Span::styled(
                        format!("{:>width$}", num, width = gutter_width),
                        gutter_style,
                    ),
                    Span::styled(" │ ", separator_style),
                ]
            } else {
                vec![Span::styled("│ ", separator_style)]
            };
            spans.extend(hl_line.spans.drain(..));
            Line::from(spans)
        })
        .collect();

    Paragraph::new(lines)
        .style(Style::default().bg(t.bg))
        .scroll((skip as u16, 0))
        .render(area, buf);
}

fn split_line_numbers(text: &str) -> (Option<Vec<String>>, Option<String>) {
    let lines: Vec<&str> = text.lines().collect();
    if lines.is_empty() {
        return (None, None);
    }
    let mut numbers = Vec::with_capacity(lines.len());
    let mut code_lines = Vec::with_capacity(lines.len());

    for line in &lines {
        if line.is_empty() {
            numbers.push(String::new());
            code_lines.push("");
            continue;
        }
        match line.find(": ") {
            Some(pos) if line[..pos].bytes().all(|b| b.is_ascii_digit()) => {
                numbers.push(line[..pos].to_string());
                code_lines.push(&line[pos + 2..]);
            }
            _ => return (None, None),
        }
    }
    (Some(numbers), Some(code_lines.join("\n")))
}

fn infer_lang_from_tool(tool: &ToolCall) -> Option<String> {
    let path = match &tool.input {
        ToolInput::Read { file_path, .. } => file_path.as_str(),
        ToolInput::Edit { file_path, .. } => file_path.as_str(),
        ToolInput::Write { file_path, .. } => file_path.as_str(),
        _ => return None,
    };
    let p = std::path::Path::new(path);
    p.extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_string())
        .or_else(|| {
            p.file_name()
                .and_then(|f| f.to_str())
                .map(|f| f.to_string())
        })
}

fn render_highlighted_block_skip(
    lang: &str,
    code: &str,
    area: Rect,
    t: Theme,
    buf: &mut Buffer,
    skip: usize,
) {
    let inner_w = area.width.saturating_sub(2) as usize;
    let lines = markdown::highlight_code(lang, code, inner_w, &t);
    Paragraph::new(lines)
        .style(Style::default().bg(t.bg))
        .scroll((skip as u16, 0))
        .render(area, buf);
}

fn render_command_output_skip(
    stdout: &str,
    stderr: &str,
    exit_code: Option<i32>,
    area: Rect,
    t: Theme,
    buf: &mut Buffer,
    skip: usize,
) {
    let mut lines: Vec<Line<'static>> = Vec::new();
    let w = area.width as usize;

    let (code_str, code_style) = match exit_code {
        Some(0) => ("exit 0".to_owned(), Style::default().fg(t.success)),
        Some(n) => (format!("exit {n}"), Style::default().fg(t.error)),
        None => ("running…".to_owned(), Style::default().fg(t.text_muted)),
    };
    lines.push(Line::from(Span::styled(code_str, code_style)));

    let max_lines = 80usize;
    let mut count = 0usize;

    for raw in stdout.lines() {
        if count >= max_lines {
            break;
        }
        for chunk in markdown::hard_wrap_str(raw, w.max(1)) {
            lines.push(Line::from(Span::styled(
                sanitize_terminal_text(&chunk),
                Style::default().fg(t.text_secondary),
            )));
            count += 1;
            if count >= max_lines {
                break;
            }
        }
    }

    for raw in stderr.lines() {
        if count >= max_lines {
            break;
        }
        for chunk in markdown::hard_wrap_str(raw, w.max(1)) {
            lines.push(Line::from(Span::styled(
                sanitize_terminal_text(&chunk),
                Style::default().fg(t.error),
            )));
            count += 1;
            if count >= max_lines {
                break;
            }
        }
    }

    Paragraph::new(lines)
        .style(Style::default().bg(t.bg))
        .scroll((skip as u16, 0))
        .render(area, buf);
}

/// Best-effort language detection for a diff view. Returns a token suitable
/// for `markdown::highlight_code_raw` (typically the file extension, falling
/// back to the filename for ext-less files like `Makefile`/`Dockerfile`).
/// Returns `None` for empty paths or paths with no recognizable token.
///
/// The returned string is *not* guaranteed to map to a real syntect syntax —
/// `highlight_code_raw` will fall back to plain text for unknowns. Keeping
/// this lossy on purpose: matching the syntect set up front would couple this
/// helper to syntect's loaded syntaxes, but the highlighter already does that
/// resolution downstream and degrades gracefully.
pub fn diff_lang(diff: &DiffView) -> Option<String> {
    let p = std::path::Path::new(&diff.file_path);
    if let Some(ext) = p.extension().and_then(|e| e.to_str()) {
        if !ext.is_empty() {
            return Some(ext.to_string());
        }
    }
    // No extension — fall back to the filename (lowercased) so things like
    // `Makefile` / `Dockerfile` / `Rakefile` get a chance to resolve via
    // syntect's by-name / by-token lookup.
    p.file_name()
        .and_then(|f| f.to_str())
        .map(|f| f.to_lowercase())
        .filter(|s| !s.is_empty())
}

fn render_diff_skip(diff: &DiffView, area: Rect, t: Theme, buf: &mut Buffer, skip: usize) {
    let bottom = area.y + area.height;
    let mut virtual_row: usize = 0;
    let lang = diff_lang(diff);

    // Sub-status row: `□ Added N lines, removed M` matching v126's
    // `□ Added 3 lines` summary line under the Update title (cli.js
    // diff renderer). Skipped when both counts are zero (e.g. a
    // metadata-only edit).
    if diff.additions > 0 || diff.deletions > 0 {
        let mut parts: Vec<String> = Vec::new();
        if diff.additions > 0 {
            parts.push(format!(
                "Added {} {}",
                diff.additions,
                if diff.additions == 1 { "line" } else { "lines" }
            ));
        }
        if diff.deletions > 0 {
            parts.push(format!(
                "removed {} {}",
                diff.deletions,
                if diff.deletions == 1 { "line" } else { "lines" }
            ));
        }
        let summary = format!("□ {}", parts.join(", "));
        if virtual_row >= skip {
            let screen_y = area.y + (virtual_row - skip) as u16;
            if screen_y < bottom {
                let row = Rect {
                    x: area.x,
                    y: screen_y,
                    width: area.width,
                    height: 1,
                };
                Paragraph::new(Line::from(Span::styled(
                    summary,
                    Style::default().fg(t.text_muted),
                )))
                .style(Style::default().bg(t.bg))
                .render(row, buf);
            }
        }
        virtual_row += 1;
    }

    for hunk in &diff.hunks {
        if area.y + (virtual_row.saturating_sub(skip)) as u16 >= bottom {
            break;
        }

        if virtual_row >= skip {
            let screen_y = area.y + (virtual_row - skip) as u16;
            if screen_y < bottom {
                let row = Rect {
                    x: area.x,
                    y: screen_y,
                    width: area.width,
                    height: 1,
                };
                buf.set_style(row, Style::default().bg(t.bg));
                Paragraph::new(Line::from(Span::styled(
                    sanitize_terminal_text(&hunk.header),
                    Style::default().fg(t.text_muted),
                )))
                .style(Style::default().bg(t.bg))
                .render(row, buf);
            }
        }
        virtual_row += 1;

        let max_dl = hunk.lines.len().min(50);

        // Per-hunk syntax highlighting. Build a single string containing all
        // line bodies (sigils stripped) joined by `\n`, then run syntect over
        // it once so multi-line constructs (block comments, raw strings,
        // here-docs) tokenize correctly across +/-/context boundaries. We
        // pass `wrap_w = 0` to disable hard-wrapping, guaranteeing a 1:1 map
        // from input lines to output lines that we can index into by row.
        // Mirrors codex's diff_render approach (codex-rs/tui/src/diff_render
        // .rs around the `hunk_syntax_lines` block).
        let highlighted: Option<Vec<Line<'static>>> = lang.as_deref().and_then(|l| {
            let visible = &hunk.lines[..max_dl];
            let hunk_text: String = visible
                .iter()
                .map(|dl| sanitize_terminal_text(&dl.content))
                .collect::<Vec<_>>()
                .join("\n");
            let lines = markdown::highlight_code_raw(l, &hunk_text, 0, &t);
            // Defensive: if line counts don't agree (shouldn't happen with
            // wrap_w=0, but syntect can occasionally produce extra rows on
            // pathological inputs), bail and let the unhighlighted branch
            // render. Better plain than misaligned.
            (lines.len() == visible.len()).then_some(lines)
        });

        for (idx, dl) in hunk.lines.iter().take(max_dl).enumerate() {
            if virtual_row >= skip {
                let screen_y = area.y + (virtual_row - skip) as u16;
                if screen_y < bottom {
                    let row = Rect {
                        x: area.x,
                        y: screen_y,
                        width: area.width,
                        height: 1,
                    };
                    let (bg_color, fg_color, sigil) = match dl.kind {
                        DiffLineKind::Added => {
                            (Color::Rgb(0, 40, 20), Color::Rgb(0, 220, 120), "+")
                        }
                        DiffLineKind::Removed => {
                            (Color::Rgb(50, 0, 0), Color::Rgb(255, 100, 100), "-")
                        }
                        DiffLineKind::Context => (t.bg, t.text_secondary, " "),
                    };
                    // Line-number column matches v126's diff style — show
                    // the `new_line` for added/context (the post-edit
                    // location) and `old_line` for removed (the source
                    // location). Pad to 5 cells so the gutter aligns
                    // across hunks. Dim color so content remains the
                    // visual center.
                    let lineno = match dl.kind {
                        DiffLineKind::Removed => dl.old_line,
                        _ => dl.new_line,
                    };
                    let lineno_str = match lineno {
                        Some(n) => format!("{n:>5} "),
                        None => "      ".into(),
                    };
                    buf.set_style(row, Style::default().bg(bg_color));

                    // Build the row spans: gutter (line number) + sigil with
                    // diff-tinted bg, followed by the content. When we have
                    // syntect output, overlay the syntax-colored spans on
                    // top of the diff bg tint; otherwise fall back to a
                    // single solid-fg span over the bg.
                    let mut spans: Vec<Span<'static>> = vec![
                        Span::styled(lineno_str, Style::default().fg(t.text_muted).bg(bg_color)),
                        Span::styled(
                            format!("{sigil} "),
                            Style::default().fg(fg_color).bg(bg_color),
                        ),
                    ];

                    match highlighted.as_ref().and_then(|h| h.get(idx)) {
                        Some(hl) => {
                            // Span composition: keep syntect's foreground
                            // color, force the diff bg tint over it, and
                            // for Removed lines layer a DIM modifier so
                            // deletions read as fading out (additions stay
                            // bright). Context lines get neither tint nor
                            // dim — pure syntax colors over `t.bg`.
                            let extra_mod = matches!(dl.kind, DiffLineKind::Removed)
                                .then_some(Modifier::DIM);
                            for sp in &hl.spans {
                                let mut style = sp.style;
                                style.bg = Some(bg_color);
                                if let Some(m) = extra_mod {
                                    style = style.add_modifier(m);
                                }
                                spans.push(Span::styled(sp.content.clone().into_owned(), style));
                            }
                        }
                        None => {
                            spans.push(Span::styled(
                                sanitize_terminal_text(&dl.content),
                                Style::default().fg(fg_color).bg(bg_color),
                            ));
                        }
                    }

                    Paragraph::new(Line::from(spans))
                        .style(Style::default().bg(bg_color))
                        .render(row, buf);
                }
            }
            virtual_row += 1;
        }

        if hunk.lines.len() > 50 {
            if virtual_row >= skip {
                let screen_y = area.y + (virtual_row - skip) as u16;
                if screen_y < bottom {
                    let row = Rect {
                        x: area.x,
                        y: screen_y,
                        width: area.width,
                        height: 1,
                    };
                    Paragraph::new(Line::from(Span::styled(
                        format!("… {} more lines", hunk.lines.len() - 50),
                        Style::default().fg(t.text_muted),
                    )))
                    .style(Style::default().bg(t.bg))
                    .render(row, buf);
                }
            }
            virtual_row += 1;
        }
    }
}

fn render_file_list_skip(files: &[String], area: Rect, t: Theme, buf: &mut Buffer, skip: usize) {
    let mut lines: Vec<Line<'static>> = Vec::new();
    for f in files.iter().take(20) {
        lines.push(Line::from(Span::styled(
            sanitize_terminal_text(f),
            Style::default().fg(t.text_secondary),
        )));
    }
    if files.len() > 20 {
        lines.push(Line::from(Span::styled(
            format!("… {} more", files.len() - 20),
            Style::default().fg(t.text_muted),
        )));
    }
    Paragraph::new(lines)
        .style(Style::default().bg(t.bg))
        .scroll((skip as u16, 0))
        .render(area, buf);
}

fn render_assistant_text_lines<'a>(
    text: &'a str,
    t: &'a Theme,
    width: usize,
    convention: crate::provider::StreamConvention,
) -> Vec<Line<'static>> {
    use crate::inline_tools::{self, Segment as InlineSeg};
    use crate::provider::StreamConvention as SC;

    let needs_inline = matches!(convention, SC::InlineXmlTags)
        || (matches!(convention, SC::AnthropicNative | SC::OpenAiNative)
            && inline_tools::contains_inline_tools(text));

    if !needs_inline {
        return markdown::to_lines(text, t, width);
    }

    let mut lines = Vec::new();
    for seg in inline_tools::parse(text) {
        match seg {
            InlineSeg::Text(s) => {
                if !s.trim().is_empty() {
                    lines.extend(markdown::to_lines(&s, t, width));
                }
            }
            InlineSeg::ToolCall { raw_body, parsed } => {
                let header = match parsed {
                    Some(p) => format!("▸ {} · {}", p.name, truncate_str(&p.summary, 80)),
                    None => format!("▸ tool_call · {}", truncate_str(&raw_body, 80)),
                };
                lines.push(Line::from(vec![
                    Span::styled(String::from("┌─ "), Style::default().fg(t.border)),
                    Span::styled(header, Style::default().fg(t.accent)),
                ]));
            }
            InlineSeg::ToolResult(body) => {
                let total = body.lines().count();
                let mut emitted = 0usize;
                for ln in body.lines().take(6) {
                    let clean = sanitize_terminal_text(ln);
                    let truncated = truncate_str(&clean, width.saturating_sub(4).max(20));
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
    lines
}

fn streaming_task_footer_lines(app: &App, t: &Theme) -> Vec<Line<'static>> {
    use crate::tasks::{DeletedFilter, TaskStatus};

    let tasks = app.task_store.list(DeletedFilter::Exclude);
    if tasks.is_empty() {
        return Vec::new();
    }

    let counts = app.task_store.counts();

    let completed_ids: std::collections::HashSet<String> = tasks
        .iter()
        .filter(|tk| tk.status == TaskStatus::Completed)
        .map(|tk| tk.id.as_str().to_owned())
        .collect();

    let fade_dur = std::time::Duration::from_secs(30);
    let now = std::time::Instant::now();
    let recently_completed: Vec<&crate::tasks::Task> = tasks
        .iter()
        .filter(|tk| {
            tk.status == TaskStatus::Completed
                && app
                    .task_completion_times
                    .get(&tk.id)
                    .map_or(false, |&t| now.duration_since(t) < fade_dur)
        })
        .collect();

    let open_tasks: Vec<&crate::tasks::Task> = tasks
        .iter()
        .filter(|tk| matches!(tk.status, TaskStatus::Pending | TaskStatus::InProgress))
        .collect();

    let mut lines: Vec<Line<'static>> = Vec::new();
    let max_visible = 5usize;
    let mut visible = 0usize;

    for tk in open_tasks.iter().chain(recently_completed.iter()) {
        if visible >= max_visible {
            break;
        }
        visible += 1;

        let is_recently_completed = tk.status == TaskStatus::Completed;

        let (icon, icon_style) = match tk.status {
            TaskStatus::Pending => ("□ ", Style::default().fg(t.text_muted)),
            TaskStatus::InProgress => ("▣ ", Style::default().fg(t.accent)),
            TaskStatus::Completed => (
                "✓ ",
                Style::default().fg(t.success).add_modifier(Modifier::DIM),
            ),
            _ => ("✗ ", Style::default().fg(t.error)),
        };

        let subj_style = if is_recently_completed {
            Style::default()
                .fg(t.text_muted)
                .add_modifier(Modifier::CROSSED_OUT | Modifier::DIM)
        } else if tk.status == TaskStatus::InProgress {
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

        if !tk.blocked_by.is_empty() {
            let open_blockers: Vec<&str> = tk
                .blocked_by
                .iter()
                .filter(|id| !completed_ids.contains(id.as_str()))
                .map(|id| id.as_str())
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

    lines
}

fn push_reasoning_lines<'a>(
    items: &mut Vec<RenderItem<'a>>,
    text: &'a str,
    expanded: bool,
    key: usize,
    t: &Theme,
) {
    if expanded {
        items.push(RenderItem::TextLine(Line::from(vec![
            Span::styled(
                "∴ Thinking",
                Style::default()
                    .fg(t.text_muted)
                    .add_modifier(Modifier::ITALIC),
            ),
            Span::styled(
                format!(" [Ctrl+O to collapse | key={}]", key),
                Style::default().fg(t.text_muted),
            ),
        ])));
        for l in text.lines() {
            items.push(RenderItem::TextLine(Line::from(vec![
                Span::styled("  ", Style::default()),
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
                "∴ Thinking",
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

#[cfg(test)]
mod reasoning_preview_tests {
    use super::*;

    fn collapsed_preview(text: &str) -> String {
        let mut items: Vec<RenderItem<'_>> = Vec::new();
        let theme = crate::theme::Theme::dark();
        push_reasoning_lines(&mut items, text, false, 0, &theme);
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
        // No content → empty preview, no ellipsis. Just shouldn't panic.
        let s = collapsed_preview("");
        assert!(s.contains("∴ Thinking"));
    }

    #[test]
    fn unicode_grapheme_count_correct_robust() {
        // 60-char cap must be by char count, not byte count, so emoji /
        // CJK don't truncate mid-codepoint. Input of 80 CJK chars (each
        // 3 bytes) → 80 chars total, capped to 60, ellipsis present.
        let input: String = std::iter::repeat('日').take(80).collect();
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

fn push_task_status_lines<'a>(items: &mut Vec<RenderItem<'a>>, ts: &'a TaskStatusPart, t: &Theme) {
    let (icon, style) = match ts.status {
        TaskLifecycle::Pending => ("◌", Style::default().fg(t.text_muted)),
        TaskLifecycle::Running => ("◎", Style::default().fg(t.text_primary)),
        TaskLifecycle::Completed => ("●", Style::default().fg(t.success)),
        TaskLifecycle::Failed => ("✗", Style::default().fg(t.error)),
        TaskLifecycle::Cancelled => ("○", Style::default().fg(t.text_muted)),
    };
    let label = ts.summary.as_deref().unwrap_or(ts.description.as_str());
    let elapsed = ts
        .elapsed_ms
        .map(|ms| format!(" [{:.1}s]", ms as f64 / 1000.0))
        .unwrap_or_default();
    items.push(RenderItem::TextLine(Line::from(vec![
        Span::styled(format!("{icon} task "), style),
        Span::styled(label.to_owned(), Style::default().fg(t.text_secondary)),
        Span::styled(elapsed, Style::default().fg(t.text_muted)),
    ])));
    if let Some(err) = &ts.error {
        items.push(RenderItem::TextLine(Line::from(vec![
            Span::styled("  error: ", Style::default().fg(t.error)),
            Span::styled(err.clone(), Style::default().fg(t.text_secondary)),
        ])));
    }
}

fn truncate_str(s: &str, max: usize) -> String {
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
mod diff_lang_tests {
    use super::*;

    fn diff_with_path(path: &str) -> DiffView {
        DiffView {
            file_path: path.to_string(),
            hunks: Vec::new(),
            additions: 0,
            deletions: 0,
        }
    }

    #[test]
    fn diff_lang_detects_rust_normal() {
        let lang = diff_lang(&diff_with_path("src/main.rs"));
        assert_eq!(lang.as_deref(), Some("rs"));
    }

    #[test]
    fn diff_lang_detects_python_normal() {
        let lang = diff_lang(&diff_with_path("main.py"));
        assert_eq!(lang.as_deref(), Some("py"));
    }

    #[test]
    fn diff_lang_unknown_returns_none_robust() {
        let lang = diff_lang(&diff_with_path(""));
        assert_eq!(lang, None);
    }

    #[test]
    fn diff_lang_handles_no_extension_robust() {
        let lang = diff_lang(&diff_with_path("Makefile"));
        assert_eq!(lang.as_deref(), Some("makefile"));
    }
}

#[cfg(test)]
mod hit_test_tests {
    use super::*;

    fn r(x: u16, y: u16, w: u16, h: u16) -> Rect {
        Rect { x, y, width: w, height: h }
    }

    #[test]
    fn find_tool_at_inside_rect_normal() {
        let regions = vec![("tool-1".to_string(), r(0, 0, 10, 3))];
        assert_eq!(find_tool_at(&regions, 5, 1), Some("tool-1"));
    }

    #[test]
    fn find_tool_at_outside_all_rects_normal() {
        let regions = vec![
            ("tool-1".to_string(), r(0, 0, 10, 3)),
            ("tool-2".to_string(), r(0, 5, 10, 3)),
        ];
        assert_eq!(find_tool_at(&regions, 5, 4), None);
        assert_eq!(find_tool_at(&regions, 20, 1), None);
    }

    #[test]
    fn find_tool_at_picks_first_match_robust() {
        let regions = vec![
            ("first".to_string(), r(0, 0, 10, 5)),
            ("second".to_string(), r(2, 1, 5, 2)),
        ];
        assert_eq!(find_tool_at(&regions, 3, 2), Some("first"));
    }

    #[test]
    fn find_tool_at_empty_regions_robust() {
        let regions: Vec<(String, Rect)> = Vec::new();
        assert_eq!(find_tool_at(&regions, 0, 0), None);
        assert_eq!(find_tool_at(&regions, 99, 99), None);
    }

    #[test]
    fn find_tool_at_boundary_inclusive_normal() {
        let regions = vec![("tool".to_string(), r(2, 3, 4, 2))];
        assert_eq!(find_tool_at(&regions, 2, 3), Some("tool"));
        assert_eq!(find_tool_at(&regions, 5, 4), Some("tool"));
        assert_eq!(find_tool_at(&regions, 6, 3), None);
        assert_eq!(find_tool_at(&regions, 2, 5), None);
    }
}
