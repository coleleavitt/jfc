use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Paragraph, Widget, Wrap},
};

use crate::app::App;
use crate::markdown;
use crate::theme::Theme;
use crate::types::*;

pub struct MessageView<'a> {
    pub app: &'a App,
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
        if app.streaming_assistant_idx == Some(idx) && app.is_streaming {
            continue;
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

        items.push(RenderItem::Blank);
    }

    if app.is_streaming || !app.streaming_text.is_empty() || !app.streaming_reasoning.is_empty() {
        items.push(RenderItem::TextLine(Line::from(Span::styled(
            "assistant",
            t.asst_label(),
        ))));

        if !app.streaming_reasoning.is_empty() {
            items.push(RenderItem::TextLine(Line::from(vec![
                Span::styled(
                    "∴ Thinking",
                    Style::default()
                        .fg(t.text_muted)
                        .add_modifier(Modifier::ITALIC),
                ),
                Span::styled(" [streaming…]", Style::default().fg(t.text_muted)),
            ])));
        }

        let convention = app.provider.stream_convention();
        for line in render_assistant_text_lines(&app.streaming_text, &t, inner_w, convention) {
            items.push(RenderItem::TextLine(line));
        }

        if app.is_streaming {
            items.push(RenderItem::TextLine(Line::from(Span::styled(
                format!(" {} ", crate::app::SPINNER[app.spinner_frame]),
                Style::default().fg(t.text_muted),
            ))));
            for line in streaming_task_footer_lines(app, &t) {
                items.push(RenderItem::TextLine(line));
            }
        }

        items.push(RenderItem::Blank);
    }

    items
}

fn tool_block_height(tool: &ToolCall, inner_w: usize) -> usize {
    if tool.is_collapsed {
        return 1;
    }
    2 + tool_content_height(&tool.output, inner_w.saturating_sub(2))
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

        ToolOutput::Diff(diff) => diff
            .hunks
            .iter()
            .map(|h| 1 + h.lines.len().min(50) + if h.lines.len() > 50 { 1 } else { 0 })
            .sum(),

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

    let (status_icon, status_style) = tool_status_icon(tool, &t);
    let border_color = border_color_for_status(tool, &t);
    let title_line = Line::from(build_title_spans(
        tool,
        &t,
        status_icon,
        status_style,
        area.width as usize,
    ));

    let full_h = tool_block_height(tool, area.width as usize) as u16;
    if skip >= full_h as usize {
        return;
    }

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(border_color))
        .title(title_line)
        .style(Style::default().bg(t.bg));

    if skip == 0 {
        let inner = block.inner(area);
        block.render(area, buf);
        if inner.height > 0 {
            render_tool_content_clipped(tool, inner, t, buf);
        }
        return;
    }

    let content_skip = skip.saturating_sub(1);
    let bottom_border_screen_y = (full_h as usize).saturating_sub(1).saturating_sub(skip) as u16;

    let content_h = if bottom_border_screen_y < area.height {
        bottom_border_screen_y
    } else {
        area.height
    };

    let content_area = Rect {
        x: area.x + 1,
        y: area.y,
        width: area.width.saturating_sub(2),
        height: content_h,
    };

    if content_area.width > 0 && content_area.height > 0 {
        render_tool_content_with_skip(tool, content_area, t, buf, content_skip);
    }

    for row_offset in 0..content_h {
        let ry = area.y + row_offset;
        if ry >= area.y + area.height {
            break;
        }
        if area.x < buf.area.right() {
            buf[(area.x, ry)]
                .set_char('│')
                .set_style(Style::default().fg(border_color));
        }
        let rx = area.x + area.width.saturating_sub(1);
        if rx < buf.area.right() {
            buf[(rx, ry)]
                .set_char('│')
                .set_style(Style::default().fg(border_color));
        }
    }

    if bottom_border_screen_y < area.height {
        let by = area.y + bottom_border_screen_y;
        for col in area.x..area.x + area.width {
            if col < buf.area.right() {
                let ch = if col == area.x {
                    '╰'
                } else if col == area.x + area.width - 1 {
                    '╯'
                } else {
                    '─'
                };
                buf[(col, by)]
                    .set_char(ch)
                    .set_style(Style::default().fg(border_color));
            }
        }
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
    // Reserve space for "▼ X " prefix (4 chars) plus border chars
    spans.extend(build_header_inner_spans(tool, t, width.saturating_sub(8)));
    spans
}

fn build_header_inner_spans<'a>(tool: &'a ToolCall, t: &Theme, max_w: usize) -> Vec<Span<'a>> {
    let kind_label = tool.kind.label();
    let summary = tool.input.summary();

    match &tool.input {
        ToolInput::Bash { command, .. } => {
            let cmd = truncate_str(command, max_w.saturating_sub(5));
            vec![
                Span::styled("bash ", Style::default().fg(t.text_muted)),
                Span::styled(cmd, Style::default().fg(t.accent)),
            ]
        }
        ToolInput::Edit { file_path, .. } => {
            let path = truncate_str(file_path, max_w.saturating_sub(5));
            vec![
                Span::styled("edit ", Style::default().fg(t.text_muted)),
                Span::styled(path, Style::default().fg(t.text_primary)),
            ]
        }
        ToolInput::Write { file_path, .. } => {
            let path = truncate_str(file_path, max_w.saturating_sub(6));
            vec![
                Span::styled("write ", Style::default().fg(t.text_muted)),
                Span::styled(path, Style::default().fg(t.text_primary)),
            ]
        }
        ToolInput::Read { file_path, .. } => {
            let path = truncate_str(file_path, max_w.saturating_sub(5));
            vec![
                Span::styled("read ", Style::default().fg(t.text_muted)),
                Span::styled(path, Style::default().fg(t.text_secondary)),
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

fn tool_status_icon(tool: &ToolCall, t: &Theme) -> (&'static str, Style) {
    match tool.status {
        ToolStatus::Pending => ("○", Style::default().fg(t.warning)),
        ToolStatus::Running => ("◌", Style::default().fg(t.accent)),
        ToolStatus::Complete => ("●", Style::default().fg(t.success)),
        ToolStatus::Failed => ("✗", Style::default().fg(t.error)),
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
    match &tool.output {
        ToolOutput::Empty => {}
        ToolOutput::Text(s) => render_text_block_skip(s, area, t.text_secondary, t, buf, skip),
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
        ToolOutput::FileContent { content, .. } => render_text_block_skip(
            content,
            area,
            t.code_block().fg.unwrap_or(t.text_secondary),
            t,
            buf,
            skip,
        ),
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

fn render_diff_skip(diff: &DiffView, area: Rect, t: Theme, buf: &mut Buffer, skip: usize) {
    let bottom = area.y + area.height;
    let mut virtual_row: usize = 0;

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
        for dl in hunk.lines.iter().take(max_dl) {
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
                    buf.set_style(row, Style::default().bg(bg_color));
                    Paragraph::new(Line::from(Span::styled(
                        format!("{} {}", sigil, sanitize_terminal_text(&dl.content)),
                        Style::default().fg(fg_color),
                    )))
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
        let preview: String = text.chars().take(60).collect();
        let ellipsis = if text.chars().count() > 60 { "…" } else { "" };
        items.push(RenderItem::TextLine(Line::from(vec![
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
        ])));
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
