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
    // Fast path: compute total height using cached line counts without
    // materializing the full RenderItem vec. This avoids .to_vec() cloning
    // of cached Line vecs just to count them.
    let mut total = 0usize;
    let width = inner_w as u16;

    for (idx, msg) in app.messages.iter().enumerate() {
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

        // Role label
        total += 1;

        let reasoning_expanded = app.reasoning_expanded.get(&idx).copied().unwrap_or(false);

        for part in &msg.parts {
            match part {
                MessagePart::Text(text) => {
                    let mut cache = app.render_cache.borrow_mut();
                    let t = app.theme;
                    if is_streaming_placeholder {
                        // Streaming: render via fast path (no syntect), store in
                        // dedicated slot so LRU is untouched.
                        let rendered = markdown::to_lines_streaming(text, &t, width as usize);
                        cache.set_streaming(idx, width, rendered);
                        total += cache.streaming_wrapped_line_count(idx, width);
                    } else {
                        // Populate the cache (compute lines + wrapped count once),
                        // then read the precomputed wrapped count. Pre-cache this
                        // loop instantiated `Paragraph::new(line.clone()).wrap(...)`
                        // for EVERY line on EVERY frame — see render_cache.rs
                        // `compute_wrapped_line_count` for the moved-once version.
                        cache.get_or_insert_with(text, width, |t_text, w| {
                            markdown::to_lines(t_text, &t, w as usize)
                        });
                        total += cache.wrapped_line_count(text, width).unwrap_or(0);
                    }
                }
                MessagePart::Reasoning(text) => {
                    if reasoning_expanded {
                        total += 1 + text.lines().count();
                    } else {
                        total += 1;
                    }
                }
                MessagePart::Tool(tool) => {
                    // Apply the same grouping the renderer uses so
                    // total-line math matches what we'll actually
                    // draw. We approximate by checking each tool: if
                    // it's the START of a groupable run of >=3 same-
                    // kind tools AND the group is not expanded, the
                    // run contributes 1 row total. Otherwise this
                    // tool contributes its block height.
                    if is_groupable(&tool.kind) {
                        // Find the parts vector index for this tool
                        // by pointer identity (cheap — same Vec).
                        let pos = msg
                            .parts
                            .iter()
                            .position(|p| match p {
                                MessagePart::Tool(t) => std::ptr::eq(t, tool),
                                _ => false,
                            })
                            .unwrap_or(0);
                        // If the previous part was a same-kind groupable,
                        // this tool's height was already counted as part
                        // of the run header — skip it.
                        let prev_is_same = pos > 0
                            && matches!(
                                &msg.parts[pos - 1],
                                MessagePart::Tool(prev)
                                    if std::mem::discriminant(&prev.kind)
                                        == std::mem::discriminant(&tool.kind)
                                        && is_groupable(&prev.kind)
                            );
                        if prev_is_same {
                            continue;
                        }
                        // We're at the start of a run. Count length.
                        let mut run_len = 1usize;
                        let mut cursor = pos + 1;
                        while cursor < msg.parts.len() {
                            if let MessagePart::Tool(t2) = &msg.parts[cursor] {
                                if std::mem::discriminant(&t2.kind)
                                    == std::mem::discriminant(&tool.kind)
                                {
                                    run_len += 1;
                                    cursor += 1;
                                    continue;
                                }
                            }
                            break;
                        }
                        let group_key = format!("{}:{}", idx, tool.id);
                        let expanded = app.tool_group_expanded.contains(&group_key);
                        if run_len >= 3 && !expanded {
                            // Single-row group header replaces the
                            // entire run.
                            total += 1;
                        } else {
                            // Sum the run's actual heights.
                            for run_part in &msg.parts[pos..pos + run_len] {
                                if let MessagePart::Tool(t2) = run_part {
                                    total += tool_block_height(t2, inner_w);
                                }
                            }
                        }
                    } else {
                        total += tool_block_height(tool, inner_w);
                    }
                }
                MessagePart::TaskStatus(ts) => {
                    total += 1;
                    if ts.error.is_some() {
                        total += 1;
                    }
                }
                MessagePart::CompactBoundary { .. } => {
                    total += 1;
                }
                MessagePart::Advisor(text) => {
                    // 1 row for the "ADVISOR:" header + 1 wrapped row per
                    // visible line. We approximate wrapping at `width`
                    // (matches the Text branch's heuristic before
                    // RenderCache enrichment lands).
                    total += 1;
                    let w = (width as usize).max(1);
                    for line in text.lines() {
                        // Each line wraps to ceil(len / width) rows.
                        total += line.len().div_ceil(w).max(1);
                    }
                    if text.is_empty() {
                        total += 1;
                    }
                }
            }
        }

        // Elapsed footer
        if msg.role == Role::Assistant && !is_streaming_placeholder {
            if msg.elapsed.is_some() {
                total += 1;
            }
        }

        // Blank separator
        total += 1;
    }

    total
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

        // Active message scope. Used only to know when to drop a
        // pulsing typing cursor `▋` after the most-recent streamed
        // text row (see MessageEnd handling below). The earlier
        // full-height gutter + bg tint painting have been removed —
        // color + bold on the role label is the entire visual
        // differentiation, no per-row decoration.
        struct Scope {
            is_streaming_placeholder: bool,
        }
        let mut scope: Option<Scope> = None;
        // Track the last row + content end-column drawn inside a
        // streaming-placeholder scope so we can drop a pulsing typing
        // cursor `▋` there at MessageEnd. Without this, the
        // streaming text just stops dead at the right edge of the
        // last char — the user has no inline cue that more text is
        // coming. The cursor pulses on the same 1.2s clock as the
        // gutter so the two signals reinforce each other.
        let mut last_streaming_cursor: Option<(u16, u16, u16)> = None;

        for item in &items {
            if y >= bottom {
                break;
            }

            // Scope markers update the active draw context but emit
            // no rows. Process them inline so the actual draw stays
            // simple.
            match item {
                RenderItem::MessageStart {
                    role: _,
                    is_streaming_placeholder,
                } => {
                    scope = Some(Scope {
                        is_streaming_placeholder: *is_streaming_placeholder,
                    });
                    last_streaming_cursor = None;
                    continue;
                }
                RenderItem::MessageEnd => {
                    // If we were rendering a streaming placeholder
                    // and tracked a "last drawn row", drop a pulsing
                    // typing cursor there now. Reduced-motion skips
                    // the cursor entirely — the gutter already gives
                    // a static "this message is in flight" signal.
                    if let Some((cx, cy, _w)) = last_streaming_cursor.take() {
                        if !crate::spinner::reduced_motion()
                            && cx < buf.area().right()
                            && cy < buf.area().bottom()
                        {
                            let phase = (std::time::SystemTime::now()
                                .duration_since(std::time::UNIX_EPOCH)
                                .map(|d| d.as_millis())
                                .unwrap_or(0)
                                % 1200) as f32
                                / 1200.0;
                            let intensity = if phase < 0.5 {
                                phase * 2.0
                            } else {
                                (1.0 - phase) * 2.0
                            };
                            let cursor_color =
                                crate::render::pulse_color_pub(t.text_muted, t.accent, intensity);
                            let cell = &mut buf[(cx, cy)];
                            cell.set_symbol("▋");
                            cell.set_style(Style::default().fg(cursor_color));
                        }
                    }
                    scope = None;
                    continue;
                }
                _ => {}
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

            // No left-side inset: items render at full width. The
            // earlier 2-column gutter strip is gone — color + bold
            // on the role label carries the message-block identity.
            let item_area = Rect {
                x: area.x,
                y,
                width,
                height: render_h,
            };
            // Hit-region: each clickable item registers its area so
            // mouse handler can hit-test. Both individual tool blocks
            // and collapsed groups participate. Group keys are
            // prefixed with `group:` so the click handler can tell
            // them apart from raw tool ids when toggling state.
            match item {
                RenderItem::ToolBlock(_, tool) => {
                    self.app
                        .tool_hit_regions
                        .borrow_mut()
                        .push((tool.id.clone(), item_area));
                }
                RenderItem::ToolGroup { key, .. } => {
                    self.app
                        .tool_hit_regions
                        .borrow_mut()
                        .push((format!("group:{key}"), item_area));
                }
                _ => {}
            }
            item.render_with_skip(item_area, buf, t, item_scroll_skip);

            // For streaming-placeholder scopes, remember the bottom
            // row's last-content column so MessageEnd can drop a
            // typing cursor right after the most-recent char. We scan
            // the bottom row of the just-rendered area for the
            // rightmost non-space cell, then bump x by 1 so the
            // cursor sits in the cell immediately after the text.
            if let Some(s) = &scope {
                if s.is_streaming_placeholder {
                    let last_y = y + render_h.saturating_sub(1);
                    if last_y < buf.area().bottom() {
                        let row_left = item_area.x;
                        let row_right = (item_area.x + item_area.width).min(buf.area().right());
                        let mut last_content_x: Option<u16> = None;
                        let mut x_pos = row_left;
                        while x_pos < row_right {
                            let cell = &buf[(x_pos, last_y)];
                            if cell.symbol() != " " && !cell.symbol().is_empty() {
                                last_content_x = Some(x_pos);
                            }
                            x_pos += 1;
                        }
                        if let Some(lx) = last_content_x {
                            let cursor_x = (lx + 1).min(row_right.saturating_sub(1));
                            last_streaming_cursor = Some((cursor_x, last_y, render_h));
                        }
                    }
                }
            }

            // No per-row decoration — the role-label color + bold
            // is the entire message-block identity. (Gutter +
            // bg-tint painting used to live here; both removed per
            // the user's "those blue lines look dumb" feedback.)

            y += render_h;
            lines_skipped += h;
        }
    }
}

enum RenderItem<'a> {
    TextLine(Line<'a>),
    /// Carries `&App` so the renderer can read `app.diagnostics`
    /// when rendering a Read result — without piping the whole App
    /// through the render-stack as a separate parameter at every
    /// helper. Only the tool-block path needs it; other items don't.
    ToolBlock(&'a App, &'a ToolCall),
    /// Collapsed group of consecutive same-kind tool calls (Read,
    /// Glob, Grep, Search). Renders as a single one-line teaser
    /// "▶ N reads · click to expand"; click on the row or `o` flips
    /// `app.tool_group_expanded` and the next render emits each
    /// tool individually.
    ToolGroup {
        key: String,
        kind_label: String,
        count: usize,
        kind_color: ratatui::style::Color,
    },
    Blank,
    /// Zero-height scope markers: bracket all the items belonging to
    /// a single chat message so the renderer can paint a full-height
    /// gutter glyph and (for assistant messages) a subtle bg tint
    /// down the entire range. Without these markers the renderer
    /// has no idea where one message ends and the next begins; it
    /// just sees a flat stream of TextLine / ToolBlock / Blank.
    MessageStart {
        role: Role,
        is_streaming_placeholder: bool,
    },
    MessageEnd,
}

impl<'a> RenderItem<'a> {
    fn height(&self, width: usize) -> usize {
        match self {
            RenderItem::Blank => 1,
            RenderItem::TextLine(line) => {
                if line.width() == 0 || width == 0 {
                    1
                } else {
                    // Use ratatui's actual word-wrap count, same as
                    // `message_view_total_lines` does. `div_ceil(width)`
                    // assumed character-wrap and could be off by 1+
                    // rows for a line whose word boundaries don't land
                    // at the column edge.
                    use ratatui::widgets::{Paragraph, Wrap};
                    let p = Paragraph::new(line.clone()).wrap(Wrap { trim: false });
                    p.line_count(width as u16).max(1)
                }
            }
            RenderItem::ToolBlock(_, tool) => tool_block_height(tool, width),
            RenderItem::ToolGroup { .. } => 1,
            // Scope markers occupy no rows — they only affect the
            // surrounding draw context (gutter color, bg tint).
            RenderItem::MessageStart { .. } | RenderItem::MessageEnd => 0,
        }
    }

    fn render_with_skip(&self, area: Rect, buf: &mut Buffer, t: Theme, skip: usize) {
        match self {
            RenderItem::MessageStart { .. } | RenderItem::MessageEnd => {}
            RenderItem::Blank => {}
            RenderItem::TextLine(line) => {
                Paragraph::new(line.clone())
                    .wrap(Wrap { trim: false })
                    .scroll((skip as u16, 0))
                    .style(Style::default().bg(t.bg))
                    .render(area, buf);
            }
            RenderItem::ToolBlock(app, tool) => {
                render_tool_block(app, tool, area, t, buf, skip);
            }
            RenderItem::ToolGroup {
                kind_label,
                count,
                kind_color,
                ..
            } => {
                if skip > 0 || area.height == 0 {
                    return;
                }
                // No leading gutter glyph — the `▶ N reads ·` text
                // already reads as a teaser. The `▶` triangle marks
                // it as expandable; kind color goes on the count
                // text. Same simplification as the other tool paths.
                let plural = if *count == 1 {
                    kind_label.clone()
                } else {
                    format!("{}s", kind_label.to_lowercase())
                };
                let row = Rect {
                    x: area.x,
                    y: area.y,
                    width: area.width,
                    height: 1,
                };
                Paragraph::new(Line::from(vec![
                    Span::styled(
                        format!("{count} {plural}"),
                        Style::default()
                            .fg(*kind_color)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(
                        " · click or press o to expand".to_string(),
                        Style::default()
                            .fg(t.text_muted)
                            .add_modifier(Modifier::ITALIC),
                    ),
                ]))
                .style(Style::default().bg(t.bg))
                .render(row, buf);
            }
        }
    }
}

/// Build a `HashMap<line_number, Severity>` for the file path
/// referenced by `input` (Read/Edit/Write), pulling from
/// `app.diagnostics`. Returns an empty map when the input isn't a
/// file-tool or no diagnostics match. The lookup uses the basename
/// when the diagnostic stores a relative path that doesn't match
/// the absolute one from the tool input — robust against either
/// representation showing up.
fn diagnostics_for_path(
    app: &App,
    input: &ToolInput,
) -> std::collections::HashMap<usize, crate::diagnostics::Severity> {
    use std::collections::HashMap;
    let mut out: HashMap<usize, crate::diagnostics::Severity> = HashMap::new();
    let path = match input {
        ToolInput::Read { file_path, .. }
        | ToolInput::Edit { file_path, .. }
        | ToolInput::Write { file_path, .. } => file_path.as_str(),
        _ => return out,
    };
    let path_basename = std::path::Path::new(path)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or(path);
    for d in &app.diagnostics {
        let d_basename = std::path::Path::new(&d.file)
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or(d.file.as_str());
        let same_path = d.file == path
            || d_basename == path_basename
            || path.ends_with(d.file.as_str())
            || d.file.ends_with(path);
        if !same_path {
            continue;
        }
        let entry = out.entry(d.line as usize).or_insert(d.severity);
        // Higher severity wins when several diagnostics target the
        // same line.
        if severity_rank(d.severity) > severity_rank(*entry) {
            *entry = d.severity;
        }
    }
    out
}

fn severity_rank(s: crate::diagnostics::Severity) -> u8 {
    use crate::diagnostics::Severity;
    match s {
        Severity::Error => 4,
        Severity::Warning => 3,
        Severity::Info => 2,
        Severity::Hint => 1,
    }
}

/// Tool kinds that get auto-grouped when the model fires several in a
/// row. These are the "search-pattern" kinds — running 5 Reads or 5
/// Greps individually drowns out the next user prompt; collapsing
/// them keeps the transcript scannable while preserving the option
/// to drill in. Edit/Write/Bash never group because each one's
/// behavior matters per-call.
fn is_groupable(kind: &ToolKind) -> bool {
    matches!(
        kind,
        ToolKind::Read | ToolKind::Glob | ToolKind::Grep | ToolKind::Search
    )
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

        // Role label gets a colored gutter glyph (`▎`) prefix so the
        // start of each message is anchored visually instead of being
        // an unframed text fragment. While the assistant is streaming,
        // the gutter pulses accent ↔ border on the same 1.2s clock as
        // the main spinner so the in-flight message reads as alive
        // even if no chars have arrived yet.
        // MessageStart/MessageEnd still bracket the scope so the
        // render loop can drop the typing-cursor `▋` after the last
        // text row of a streaming placeholder. The gutter painting
        // and bg tint that used to ride along with this scope have
        // been removed — the role label's color + bold is the only
        // visual differentiation now (matches the sidebar's bare
        // colored-bold-headers convention the user wanted).
        items.push(RenderItem::MessageStart {
            role: msg.role,
            is_streaming_placeholder,
        });
        let label_line = match msg.role {
            Role::User => Line::from(Span::styled("you", t.user_label())),
            Role::Assistant => {
                let mut spans = Vec::new();
                if is_streaming_placeholder && !crate::spinner::reduced_motion() {
                    let phase = (std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map(|d| d.as_millis())
                        .unwrap_or(0)
                        % 1200) as f32
                        / 1200.0;
                    let intensity = if phase < 0.5 {
                        phase * 2.0
                    } else {
                        (1.0 - phase) * 2.0
                    };
                    let dot_color =
                        crate::render::pulse_color_pub(t.text_muted, t.accent, intensity);
                    spans.push(Span::styled("● ", Style::default().fg(dot_color)));
                }
                spans.push(Span::styled("assistant", t.asst_label()));
                Line::from(spans)
            }
        };
        items.push(RenderItem::TextLine(label_line));

        let reasoning_expanded = app.reasoning_expanded.get(&idx).copied().unwrap_or(false);

        // Walk parts with peek-ahead so consecutive groupable tools
        // (Read/Glob/Grep) collapse into a single ToolGroup row when
        // the user hasn't expanded the group. Min run length is 3 —
        // 1–2 tools render fine individually and grouping them just
        // adds an extra click.
        const MIN_GROUP_LEN: usize = 3;
        let mut p = 0usize;
        while p < msg.parts.len() {
            let part = &msg.parts[p];
            match part {
                MessagePart::Tool(first_tool) if is_groupable(&first_tool.kind) => {
                    // Probe forward for consecutive same-kind tools.
                    let mut run_end = p + 1;
                    while run_end < msg.parts.len() {
                        if let MessagePart::Tool(t2) = &msg.parts[run_end] {
                            if std::mem::discriminant(&t2.kind)
                                == std::mem::discriminant(&first_tool.kind)
                            {
                                run_end += 1;
                                continue;
                            }
                        }
                        break;
                    }
                    let run_len = run_end - p;
                    let group_key = format!("{}:{}", idx, first_tool.id);
                    let expanded = app.tool_group_expanded.contains(&group_key);
                    if run_len >= MIN_GROUP_LEN && !expanded {
                        items.push(RenderItem::ToolGroup {
                            key: group_key,
                            kind_label: first_tool.kind.label().to_owned(),
                            count: run_len,
                            kind_color: tool_kind_color(&first_tool.kind, &t),
                        });
                        p = run_end;
                        continue;
                    }
                    // Either the run was too short to bother grouping
                    // or the user has expanded it — emit each tool
                    // individually.
                    for tool_part in &msg.parts[p..run_end] {
                        if let MessagePart::Tool(tool) = tool_part {
                            items.push(RenderItem::ToolBlock(app, tool));
                        }
                    }
                    p = run_end;
                    continue;
                }
                _ => {}
            }
            match part {
                MessagePart::Text(text) => {
                    let lines = if is_streaming_placeholder {
                        // Streaming fast path: recompute every frame without
                        // syntect. Cost is ~5µs/KB (pulldown-cmark only) vs
                        // ~200µs/KB with syntect highlighting. The streaming
                        // slot in RenderCache stores the result for line-count
                        // queries within the same frame.
                        let theme = t;
                        let rendered = markdown::to_lines_streaming(text, &theme, inner_w);
                        let mut cache = app.render_cache.borrow_mut();
                        cache.set_streaming(idx, inner_w as u16, rendered.clone());
                        rendered
                    } else {
                        let mut cache = app.render_cache.borrow_mut();
                        let width = inner_w as u16;
                        let theme = t;
                        cache
                            .get_or_insert_with(text, width, |t_text, w| {
                                markdown::to_lines(t_text, &theme, w as usize)
                            })
                            .to_vec()
                    };
                    for line in lines {
                        items.push(RenderItem::TextLine(line));
                    }
                }
                MessagePart::Reasoning(text) => {
                    push_reasoning_lines(&mut items, text, reasoning_expanded, idx, &t);
                }
                MessagePart::Tool(tool) => {
                    items.push(RenderItem::ToolBlock(app, tool));
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
                MessagePart::Advisor(text) => {
                    push_advisor_lines(&mut items, text, &t);
                }
            }
            p += 1;
        }

        // v126 cli.js:341376 — `Cooked for Nm Ns` post-turn footer with a
        // randomized past-tense verb. Only attached to completed assistant
        // turns (skip user messages, skip the in-flight placeholder which
        // already has its own spinner row). `msg.elapsed` carries the
        // duration string written at StreamDone time.
        if msg.role == Role::Assistant && !is_streaming_placeholder {
            if let Some(elapsed) = &msg.elapsed {
                // Dim italic, no leading glyph. The earlier `▎`
                // prefix bracketed the message visually with the
                // role-header gutter; with the gutter gone, the
                // bracket is gone too. The elapsed line just sits
                // muted under the body, which reads cleaner.
                items.push(RenderItem::TextLine(Line::from(Span::styled(
                    elapsed.clone(),
                    Style::default()
                        .fg(t.text_muted)
                        .add_modifier(Modifier::DIM),
                ))));
            }
        }
        // Close the message scope BEFORE the Blank separator so the
        // gutter doesn't bleed into the empty row between messages.
        // Reads as: each message is a contained band, separated by a
        // narrow gap.
        items.push(RenderItem::MessageEnd);
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
    let content_w = inner_w.saturating_sub(2);
    // When the renderer will route ToolOutput::Text through the
    // highlighted-with-line-numbers path, the effective wrap width
    // is narrower (gutter eats columns). Approximate the gutter
    // width so wrapped_line_count uses the same width the renderer
    // will. Without this, long lines in Read output undercount their
    // wrapped height and get clipped offscreen.
    let effective_content_w = if matches!(&tool.output, ToolOutput::Text(s) if !s.is_empty())
        && infer_lang_from_tool(tool).is_some()
    {
        // Estimate gutter: max line-number width + separator.
        // split_line_numbers would be exact but expensive; approximate
        // from line count of the text. 4-digit line numbers + ` │ ` = 7,
        // 5-digit = 8. Use 8 as conservative default.
        content_w.saturating_sub(8)
    } else {
        content_w
    };
    1 + cont + tool_content_height_with(&tool.output, effective_content_w, tool.expanded)
}

pub fn tool_block_height_pub(tool: &ToolCall, inner_w: usize) -> usize {
    tool_block_height(tool, inner_w)
}

fn tool_content_height(output: &ToolOutput, content_w: usize) -> usize {
    tool_content_height_with(output, content_w, false)
}

/// Compute the rendered row count for a tool's body. `expanded` lifts
/// the per-section preview caps so the user sees the full content
/// after toggling expand (Ctrl+O / `o` / left-click on the tool
/// block). The "+ N more" footer line is included in the count when
/// the cap clips content; the renderer matches the same logic.
fn tool_content_height_with(output: &ToolOutput, content_w: usize, expanded: bool) -> usize {
    let cap = if expanded { 500 } else { 80 };
    let footer_if = |total: usize| if total > cap { 1 } else { 0 };
    match output {
        ToolOutput::Empty => 0,

        ToolOutput::Text(s) => {
            let total = wrapped_line_count(s, content_w);
            total.min(cap) + footer_if(total)
        }

        ToolOutput::LargeText(lt) => {
            if !expanded
                && (lt.line_count > LargeText::COLLAPSE_LINES
                    || lt.content.len() > LargeText::COLLAPSE_BYTES)
            {
                1
            } else {
                let total = wrapped_line_count(&lt.content, content_w);
                total.min(cap) + footer_if(total)
            }
        }

        ToolOutput::Command { stdout, stderr, .. } => {
            let stdout_total = if stdout.is_empty() {
                0
            } else {
                wrapped_line_count(stdout, content_w)
            };
            let stderr_total = if stderr.is_empty() {
                0
            } else {
                wrapped_line_count(stderr, content_w)
            };
            // +1 for exit code row. When both stdout and stderr are
            // non-empty the renderer also emits a `↳ stderr` divider
            // row between them — account for it so the height doesn't
            // undercount and clip the last stderr line.
            let stderr_divider = if !stdout.is_empty() && !stderr.is_empty() {
                1
            } else {
                0
            };
            1 + stdout_total.min(cap)
                + footer_if(stdout_total)
                + stderr_divider
                + stderr_total.min(cap)
                + footer_if(stderr_total)
        }

        ToolOutput::Diff(diff) => {
            let summary_row = if diff.additions > 0 || diff.deletions > 0 {
                1
            } else {
                0
            };
            let hunk_cap = if expanded { 500 } else { 50 };
            summary_row
                + diff
                    .hunks
                    .iter()
                    .map(|h| {
                        1 + h.lines.len().min(hunk_cap)
                            + if h.lines.len() > hunk_cap { 1 } else { 0 }
                    })
                    .sum::<usize>()
        }

        ToolOutput::FileContent { content, .. } => {
            // The renderer wraps at area.width - 2 (gutter `│ ` prefix),
            // so subtract 2 here to match the effective wrap width.
            let effective_w = content_w.saturating_sub(2);
            let total = wrapped_line_count(content, effective_w);
            total.min(cap) + footer_if(total)
        }

        ToolOutput::FileList(files) => {
            let cap = if expanded { 500 } else { 20 };
            files.len().min(cap) + if files.len() > cap { 1 } else { 0 }
        }
    }
}

fn wrapped_line_count(text: &str, width: usize) -> usize {
    use unicode_width::UnicodeWidthChar;
    if width == 0 {
        return text.lines().count().max(1);
    }
    // Use display-cell width (matching `markdown::hard_wrap_str` at
    // render time) rather than raw char count. Without this, lines
    // containing CJK / emoji / box-drawing chars undercount their
    // wrapped row height (each takes 2 cells, counted as 1) and the
    // tool block's allocated area is smaller than what the renderer
    // emits — corrupting the buffer with overlapping text. Bug
    // observed when streaming `git diff --stat` output containing
    // wide path glyphs.
    text.lines()
        .map(|line| {
            let cells: usize = line
                .chars()
                .map(|c| UnicodeWidthChar::width(c).unwrap_or(0))
                .sum();
            if cells == 0 { 1 } else { cells.div_ceil(width) }
        })
        .sum::<usize>()
        .max(if text.is_empty() { 0 } else { 1 })
}

fn render_tool_block(
    app: &App,
    tool: &ToolCall,
    area: Rect,
    t: Theme,
    buf: &mut Buffer,
    skip: usize,
) {
    if area.height == 0 {
        return;
    }

    if tool.is_collapsed {
        if skip == 0 {
            // Collapsed-tool header: no gutter glyph (matching the
            // expanded path). The header itself includes the status
            // icon and kind-colored title which carry the same info.
            let header = build_collapsed_header(tool, &t, area.width as usize);
            Paragraph::new(header)
                .style(Style::default().bg(t.bg))
                .render(
                    Rect {
                        x: area.x,
                        y: area.y,
                        width: area.width,
                        height: 1,
                    },
                    buf,
                );
        }
        return;
    }

    let frame_idx = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| (d.as_millis() / 80) as usize)
        .unwrap_or(0);
    let (status_icon, status_style) = tool_status_icon_animated(tool, &t, frame_idx);

    let full_h = tool_block_height(tool, area.width as usize) as u16;
    if skip >= full_h as usize {
        return;
    }

    // No more full-height left gutter bar. The tool's identity is
    // already shown three different ways — title text (`Bash(...)`,
    // `Read(...)`), the status icon (`●`/`○`/`✓`/`✘`), and the
    // kind-colored title — so painting a fourth signal as a column
    // down the left edge was redundant decoration. Same problem
    // the sidebar gutters had. v126's actual tool rendering uses
    // just title-line + indent; mirroring that here.

    // Sparkle on tool complete: when this tool just finished
    // successfully, flash a `✦` next to the title for 600ms with a
    // fade. Reduced-motion skips it. Now sits at column 0 (where
    // the gutter used to be) since there's no bar to compete with.
    if skip == 0
        && matches!(tool.status, crate::types::ToolStatus::Complete)
        && !crate::spinner::reduced_motion()
    {
        if let Some((id, when)) = &app.recent_tool_completion {
            if id == &tool.id {
                let age = when.elapsed();
                if age < std::time::Duration::from_millis(600) {
                    let intensity = 1.0 - (age.as_millis() as f32 / 600.0);
                    if area.x < buf.area().right() {
                        let cell = &mut buf[(area.x, area.y)];
                        cell.set_symbol("✦");
                        let blended = crate::render::pulse_color_pub(t.bg, t.accent, intensity);
                        cell.set_style(Style::default().fg(blended));
                    }
                }
            }
        }
    }

    let title_spans = build_title_spans(
        tool,
        &t,
        status_icon,
        status_style,
        area.width.saturating_sub(2) as usize,
    );

    // Title now sits at column 0 (no gutter to dodge). The status
    // icon at the start of `title_spans` is the visual anchor.
    if skip == 0 && area.height > 0 {
        let title_area = Rect {
            x: area.x,
            y: area.y,
            width: area.width,
            height: 1,
        };
        Paragraph::new(Line::from(title_spans))
            .style(Style::default().bg(t.bg))
            .render(title_area, buf);
    }

    let title_consumed: u16 = if skip == 0 { 1 } else { 0 };
    let content_skip = skip.saturating_sub(1);
    let content_y = area.y + title_consumed;
    let content_h = area.height.saturating_sub(title_consumed);
    if content_h == 0 {
        return;
    }
    // Body indents 2 columns from the title's left edge so it
    // visually nests under the tool's status icon. With the gutter
    // gone, the indent is a pure visual cue: title at column 0, body
    // starts at column 2. Mirrors how `gh pr view`, `git log`, and
    // most CLI tools nest output under their headers.
    let content_area = Rect {
        x: area.x + 2,
        y: content_y,
        width: area.width.saturating_sub(2),
        height: content_h,
    };
    if content_area.width > 0 {
        render_tool_content_with_skip(app, tool, content_area, t, buf, content_skip);
    }
}

fn build_collapsed_header<'a>(tool: &'a ToolCall, t: &Theme, width: usize) -> Line<'a> {
    // Match the expanded-header path: derive the spinner frame from
    // wall-clock time so a Pending or Running tool keeps animating
    // even when it's collapsed (the more common case while a batch
    // is in flight). Without this the bullet froze at `○`/`◌` and the
    // user couldn't tell "queued and alive" apart from "stuck".
    let frame_idx = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| (d.as_millis() / 80) as usize)
        .unwrap_or(0);
    let (status_icon, status_style) = tool_status_icon_animated(tool, t, frame_idx);
    // Collapsed-tool header: status icon + title. The chevron `▶`
    // that used to mark "expandable" was redundant — a collapsed
    // tool is already visibly missing its body. The status icon is
    // the only visual anchor that carries unique info, so it gets
    // the front spot.
    let mut spans = vec![
        Span::styled(status_icon.to_owned(), status_style),
        Span::raw(" "),
    ];
    spans.extend(build_header_inner_spans(tool, t, width.saturating_sub(4)));
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
    // Expanded-tool title: status icon + title. The `▼` chevron that
    // used to mark "expanded" was redundant — the body's presence
    // underneath already shows it's expanded. Cleaner without it.
    let mut spans = vec![
        Span::styled(status_icon.to_owned(), status_style),
        Span::raw(" "),
    ];
    if tool.pinned {
        spans.push(Span::styled(
            "📌 ",
            Style::default().fg(t.warning).add_modifier(Modifier::BOLD),
        ));
    }
    // Reserve a few columns at the right for the optional elapsed
    // badge. `format_elapsed_badge` returns `Some("[2.3s]")` only for
    // completed/failed tools that have a measured duration, otherwise
    // None.
    let badge = format_elapsed_badge(tool);
    let badge_w = badge.as_ref().map(|s| s.chars().count() + 1).unwrap_or(0);
    let effective = width
        .min(tool_title_width_cap())
        .saturating_sub(4 + badge_w);
    spans.extend(build_header_inner_spans(tool, t, effective));
    if let Some(b) = badge {
        spans.push(Span::raw(" "));
        spans.push(Span::styled(
            b,
            Style::default()
                .fg(t.text_muted)
                .add_modifier(Modifier::DIM),
        ));
    }
    spans
}

/// Render the elapsed duration as a compact badge for the title row.
/// Only shown after a tool finishes — pending/running tools show
/// the spinner and don't need a badge yet. Skips sub-100ms results
/// (their badge is too noisy and adds nothing — most reads, glob,
/// memory ops finish in <100ms).
fn format_elapsed_badge(tool: &ToolCall) -> Option<String> {
    if !matches!(tool.status, ToolStatus::Complete | ToolStatus::Failed) {
        return None;
    }
    let ms = tool.elapsed_ms?;
    if ms < 100 {
        return None;
    }
    if ms < 10_000 {
        Some(format!("[{:.1}s]", ms as f64 / 1000.0))
    } else if ms < 60_000 {
        Some(format!("[{}s]", ms / 1000))
    } else {
        let mins = ms / 60_000;
        let secs = (ms % 60_000) / 1000;
        Some(format!("[{mins}m {secs}s]"))
    }
}

fn build_header_inner_spans<'a>(tool: &'a ToolCall, t: &Theme, max_w: usize) -> Vec<Span<'a>> {
    let kind_label = tool.kind.label();
    let summary = tool.input.summary();
    let kind_style = Style::default()
        .fg(tool_kind_color(&tool.kind, t))
        .add_modifier(Modifier::BOLD);

    match &tool.input {
        ToolInput::Bash { command, .. } => {
            let first_line = command.lines().next().unwrap_or(command);
            let cmd = truncate_str(first_line, max_w.saturating_sub(8));
            vec![
                Span::styled("Bash", kind_style),
                Span::styled("(", Style::default().fg(t.text_muted)),
                Span::styled(cmd, Style::default().fg(t.text_primary)),
                Span::styled(")", Style::default().fg(t.text_muted)),
            ]
        }
        ToolInput::Edit { file_path, .. } => {
            let path = truncate_str(file_path, max_w.saturating_sub(8));
            vec![
                Span::styled("Update", kind_style),
                Span::styled("(", Style::default().fg(t.text_muted)),
                Span::styled(path, Style::default().fg(t.text_primary)),
                Span::styled(")", Style::default().fg(t.text_muted)),
            ]
        }
        ToolInput::Write { file_path, .. } => {
            let path = truncate_str(file_path, max_w.saturating_sub(8));
            vec![
                Span::styled("Write", kind_style),
                Span::styled("(", Style::default().fg(t.text_muted)),
                Span::styled(path, Style::default().fg(t.text_primary)),
                Span::styled(")", Style::default().fg(t.text_muted)),
            ]
        }
        ToolInput::Read { file_path, .. } => {
            let path = truncate_str(file_path, max_w.saturating_sub(7));
            vec![
                Span::styled("Read", kind_style),
                Span::styled("(", Style::default().fg(t.text_muted)),
                Span::styled(path, Style::default().fg(t.text_secondary)),
                Span::styled(")", Style::default().fg(t.text_muted)),
            ]
        }
        _ => {
            let s = truncate_str(&summary, max_w.saturating_sub(kind_label.len() + 1));
            vec![
                Span::styled(format!("{kind_label} "), kind_style),
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

/// Distinct accent color per tool kind. The gutter bar and tool name
/// span both pick this color (mixed with status state for Running /
/// Failed) so the user can spot at a glance "this is a Bash" vs
/// "this is a Read" without reading the label. Mirrors Claude Code's
/// per-tool color identity.
///
/// Picks are tuned for the dark theme to stay distinguishable from
/// each other AND from status colors: success (green) and error (red)
/// are reserved for status indicators, so Read/Write/etc. use blues,
/// purples, and ambers that don't collide.
pub fn tool_kind_color(kind: &ToolKind, t: &Theme) -> ratatui::style::Color {
    use ratatui::style::Color;
    match kind {
        ToolKind::Read => Color::Rgb(120, 180, 255), // soft blue
        ToolKind::Write => Color::Rgb(255, 200, 130), // amber
        ToolKind::Edit | ToolKind::ApplyPatch => Color::Rgb(160, 230, 170), // mint
        ToolKind::Bash => Color::Rgb(180, 180, 200), // neutral grey
        ToolKind::Glob | ToolKind::Grep | ToolKind::Search => Color::Rgb(200, 160, 255), // lavender
        ToolKind::Task => Color::Rgb(255, 170, 220), // rose
        ToolKind::TaskCreate | ToolKind::TaskUpdate | ToolKind::TaskList | ToolKind::TaskDone => {
            Color::Rgb(140, 220, 220)
        } // teal
        ToolKind::MemoryCreate | ToolKind::MemoryDelete => Color::Rgb(220, 220, 140), // olive
        ToolKind::TeamCreate
        | ToolKind::TeamDelete
        | ToolKind::SendMessage
        | ToolKind::TeamMemberMode => Color::Rgb(255, 150, 130), // coral
        ToolKind::Skill => Color::Rgb(180, 220, 255), // ice
        ToolKind::GraphQuery | ToolKind::SymbolEdit => Color::Rgb(130, 200, 180), // sage
        ToolKind::PostBounty | ToolKind::RunBounty | ToolKind::MarketStatus => {
            Color::Rgb(255, 215, 100)
        } // gold
        ToolKind::ExitPlanMode => Color::Rgb(170, 200, 255),
        ToolKind::MultiEdit => Color::Rgb(160, 230, 170),
        ToolKind::AskUserQuestion => Color::Rgb(255, 200, 240),
        ToolKind::WebFetch | ToolKind::WebSearch => Color::Rgb(120, 200, 220),
        ToolKind::Mcp(_) => Color::Rgb(190, 170, 240),
        ToolKind::CronCreate
        | ToolKind::CronList
        | ToolKind::CronDelete
        | ToolKind::ScheduleWakeup
        | ToolKind::Monitor => Color::Rgb(180, 200, 255),
        ToolKind::Lsp => Color::Rgb(140, 200, 240),
        ToolKind::PushNotification | ToolKind::RemoteTrigger => Color::Rgb(255, 180, 110),
        ToolKind::EnterPlanMode | ToolKind::EnterWorktree | ToolKind::ExitWorktree => {
            Color::Rgb(180, 220, 180)
        }
        ToolKind::NotebookRead | ToolKind::NotebookEdit => Color::Rgb(255, 170, 100),
        ToolKind::Generic(_) => t.text_secondary,
    }
}

/// 4-frame star-burst rotation used for Running tools — same shape family
/// as v126's tool-use indicator (Claude Code shows alternating `* ✱ +`
/// glyphs as the bullet). Each frame is one codepoint so column width
/// stays stable regardless of which frame is showing.
const RUNNING_FRAMES: &[&str] = &["✶", "✷", "✸", "✹"];

/// 2-frame pulse for Pending: open ring → dotted ring. Same column
/// width, just enough motion that "queued behind another tool" reads
/// as queued rather than frozen.
const PENDING_FRAMES: &[&str] = &["○", "◌"];

/// Per-frame animated icon. Running tools rotate through the star-burst
/// frames at ~120ms each (one frame per ~1.5 ticks), so the bullet
/// visibly steps through the cycle instead of just two-tone blinking the
/// same shape — that was indistinguishable from a static `●` on most
/// terminal themes. Pending tools alternate between `○` and `◌` at a
/// slower cadence so a queued tool reads differently from an idle one.
///
/// Why glyph rotation over color-only blink: terminals with low foreground
/// contrast (light themes, Solarized variants) wash out the bold/muted
/// color toggle to the point of invisibility. A shape change is robust
/// across themes — the user always sees motion. Mirrors v126's tool-use
/// spinner (cli.js:323158) which rotates a glyph on every frame.
pub fn tool_status_icon_animated(
    tool: &ToolCall,
    t: &Theme,
    frame: usize,
) -> (&'static str, Style) {
    match tool.status {
        ToolStatus::Running => {
            // Two-layer animation:
            //  - Glyph rotates slowly (every 4 ticks ≈ 320ms per frame,
            //    full cycle ≈ 1.28s). Rotation tells the eye "this is
            //    moving" without strobing.
            //  - Color pulses at a different cadence (every 9 ticks ≈
            //    720ms BOLD ⇄ DIM) so the two effects don't sync into
            //    a single distracting beat.
            // Picked the prime-ish 4 vs 9 spacing so the two
            // periodicities take ~25 ticks (2s) to align — beyond
            // perceptual gestalt.
            let glyph = RUNNING_FRAMES[(frame / 4) % RUNNING_FRAMES.len()];
            let bright = (frame / 9) % 2 == 0;
            let style = if bright {
                Style::default()
                    .fg(t.accent)
                    .add_modifier(ratatui::style::Modifier::BOLD)
            } else {
                Style::default().fg(t.text_muted)
            };
            (glyph, style)
        }
        ToolStatus::Pending => {
            let glyph = PENDING_FRAMES[(frame / 6) % PENDING_FRAMES.len()];
            (glyph, Style::default().fg(t.warning))
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

#[allow(dead_code)]
fn render_tool_content_clipped(app: &App, tool: &ToolCall, area: Rect, t: Theme, buf: &mut Buffer) {
    render_tool_content_with_skip(app, tool, area, t, buf, 0);
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
    app: &App,
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
                // Build a per-line severity map for the file this
                // tool is reading so the gutter can decorate
                // offending rows. Only Read produces line-numbered
                // output; for Edit/Write the path is the file but
                // the content shown is the *new* state, not the
                // current diagnostic-bearing state — skipping the
                // map there avoids stale glyphs.
                let diag_lines = if matches!(tool.kind, ToolKind::Read) {
                    diagnostics_for_path(app, &tool.input)
                } else {
                    std::collections::HashMap::new()
                };
                render_highlighted_with_line_numbers(
                    lang,
                    s,
                    area,
                    t,
                    buf,
                    skip,
                    tool.expanded,
                    &diag_lines,
                );
            } else if matches!(tool.kind, ToolKind::Task) {
                render_markdown_block_skip(s, area, t, buf, skip);
            } else {
                render_text_block_skip(s, area, t.text_secondary, t, buf, skip, tool.expanded);
            }
        }
        ToolOutput::LargeText(lt) => {
            // Three-state: huge + not expanded → 1-row teaser; huge +
            // expanded → render through the same text path as moderate
            // outputs; moderate → always render.
            let huge = lt.line_count > LargeText::COLLAPSE_LINES
                || lt.content.len() > LargeText::COLLAPSE_BYTES;
            if huge && !tool.expanded {
                if skip == 0 {
                    Paragraph::new(Line::from(Span::styled(
                        format!("[{} · click or press o to expand]", lt.size_label()),
                        Style::default()
                            .fg(t.text_muted)
                            .add_modifier(Modifier::ITALIC),
                    )))
                    .style(Style::default().bg(t.bg))
                    .render(area, buf);
                }
            } else {
                render_text_block_skip(
                    &lt.content,
                    area,
                    t.text_secondary,
                    t,
                    buf,
                    skip,
                    tool.expanded,
                );
            }
        }
        ToolOutput::Command {
            stdout,
            stderr,
            exit_code,
        } => {
            // When a Bash command is a plain `cat <file.ext>` / `head`
            // / `tail`, route stdout through syntect highlighting so
            // the user sees real Rust/Python/etc. coloring instead
            // of monochrome. The fallback path (no recognised file)
            // keeps the existing ANSI-aware rendering for arbitrary
            // commands. Stderr always renders red; exit-code line
            // stays as before.
            // Bash output routing:
            //   - Structured tools (grep/rg/find/ls/git diff/git log)
            //     get dedicated parsers + colored renderers.
            //   - cat/head/tail get markdown-or-syntax rendering as
            //     before (markdown wins when content sniffs md-y).
            //   - Everything else falls through to plain command
            //     output (which already does ANSI passthrough).
            let cmd_str = match &tool.input {
                ToolInput::Bash { command, .. } => command.as_str(),
                _ => "",
            };
            let cmd_kind = classify_bash_cmd(cmd_str);
            let success = !stdout.is_empty() && exit_code.unwrap_or(-1) == 0;
            // grep returns 1 for "no matches" — treat as success
            // visually so the renderer fires even when the result
            // is just the header.
            let grep_success = matches!(cmd_kind, BashCmdKind::Grep)
                && !stdout.is_empty()
                && exit_code.unwrap_or(-1) <= 1;
            // git diff returns 1 when there are diffs (with --exit-code).
            let gitdiff_success = matches!(cmd_kind, BashCmdKind::GitDiff)
                && !stdout.is_empty()
                && exit_code.unwrap_or(-1) <= 1;

            match cmd_kind {
                BashCmdKind::Grep if grep_success => render_grep_output_skip(
                    stdout,
                    stderr,
                    *exit_code,
                    area,
                    t,
                    buf,
                    skip,
                    tool.expanded,
                    cmd_str,
                ),
                BashCmdKind::PathList if success => render_path_list_output_skip(
                    stdout,
                    stderr,
                    *exit_code,
                    area,
                    t,
                    buf,
                    skip,
                    tool.expanded,
                ),
                BashCmdKind::GitDiff if gitdiff_success => render_git_diff_output_skip(
                    stdout,
                    stderr,
                    *exit_code,
                    area,
                    t,
                    buf,
                    skip,
                    tool.expanded,
                ),
                BashCmdKind::GitLog if success => render_git_log_output_skip(
                    stdout,
                    stderr,
                    *exit_code,
                    area,
                    t,
                    buf,
                    skip,
                    tool.expanded,
                ),
                BashCmdKind::HexDump if success => render_hex_dump_output_skip(
                    stdout,
                    stderr,
                    *exit_code,
                    area,
                    t,
                    buf,
                    skip,
                    tool.expanded,
                ),
                BashCmdKind::TabularList if success => render_tabular_list_output_skip(
                    stdout,
                    stderr,
                    *exit_code,
                    area,
                    t,
                    buf,
                    skip,
                    tool.expanded,
                ),
                // Cargo / make / npm builds: the structured output
                // (`Compiling X`, `error[E…]`, `warning:`, `Finished`,
                // `running N tests`, `test foo … ok/FAILED`) deserves
                // semantic coloring, not the flat ANSI passthrough
                // command_output gives. Treat exit_code 0 / 101 (test
                // failure is still informative) as a render trigger.
                BashCmdKind::CompilerOutput
                    if !stdout.is_empty()
                        && exit_code.map(|c| c == 0 || c == 101 || c == 1).unwrap_or(true) =>
                {
                    render_compiler_output_skip(
                        stdout,
                        stderr,
                        *exit_code,
                        area,
                        t,
                        buf,
                        skip,
                        tool.expanded,
                    )
                }
                _ => {
                    // cat/head/tail path — fall through to the
                    // existing markdown-or-syntect routing.
                    let lang_hint = infer_lang_from_tool(tool);
                    let lang_lc = lang_hint.as_deref().map(|l| l.to_ascii_lowercase());
                    let is_markdown_lang = lang_lc
                        .as_deref()
                        .map(|l| matches!(l, "md" | "markdown" | "mdx" | "mkd" | "mdown"))
                        .unwrap_or(false);
                    let content_is_md = !is_markdown_lang && looks_like_markdown(stdout);
                    if success && (is_markdown_lang || content_is_md) {
                        render_cat_markdown_output_skip(
                            stdout, stderr, *exit_code, area, t, buf, skip,
                        );
                    } else if let Some(lang) = lang_hint.as_deref().filter(|_| success) {
                        render_cat_output_skip(
                            lang,
                            stdout,
                            stderr,
                            *exit_code,
                            area,
                            t,
                            buf,
                            skip,
                            tool.expanded,
                        );
                    } else {
                        render_command_output_skip(
                            stdout,
                            stderr,
                            *exit_code,
                            area,
                            t,
                            buf,
                            skip,
                            tool.expanded,
                        );
                    }
                }
            }
        }
        ToolOutput::Diff(diff) => render_diff_skip(diff, area, t, buf, skip, tool.expanded),
        ToolOutput::FileContent {
            content, language, ..
        } => {
            let hl_lang = if language.is_empty() {
                "rs"
            } else {
                language.as_str()
            };
            render_highlighted_block_skip(hl_lang, content, area, t, buf, skip, tool.expanded);
        }
        ToolOutput::FileList(files) => render_file_list_skip(files, area, t, buf, skip),
    }
}

/// Render `text` through the full markdown pipeline (`markdown::to_lines`)
/// instead of the plain width-wrapper. Use for Task subagent output and
/// other tool results that are known to be assistant-authored markdown.
/// Caps at `MAX_LINES` so a runaway agent can't drown the transcript.
fn render_markdown_block_skip(text: &str, area: Rect, t: Theme, buf: &mut Buffer, skip: usize) {
    const MAX_LINES: usize = 200;
    let width = area.width as usize;
    let mut lines = markdown::to_lines(text, &t, width.max(1));
    if lines.len() > MAX_LINES {
        let total = lines.len();
        lines.truncate(MAX_LINES);
        lines.push(Line::from(Span::styled(
            format!("… truncated ({total} lines total)"),
            Style::default().fg(t.text_muted),
        )));
    }
    // Wrap{trim:false} word-wraps long lines instead of clipping them
    // at the right edge. Without this a Task-tool result whose JSON
    // contains a long string value (e.g. "message": "Spawned successfully…")
    // got cut to "message": "Spawned su" with no continuation. Markdown
    // is the right rendering for the JSON pretty-print body — we just
    // need it to wrap rather than chop.
    Paragraph::new(lines)
        .style(Style::default().bg(t.bg))
        .wrap(ratatui::widgets::Wrap { trim: false })
        .scroll((skip as u16, 0))
        .render(area, buf);
}

/// Wrap a styled `Line` to `width` columns, preserving span styles
/// across wrap points. Used by the command-output renderer so a long
/// red `error[E0382]: ...` line still wraps cleanly while keeping its
/// red color on every continuation row. Returns one or more `Line`s.
fn wrap_styled_line(line: &Line<'static>, width: usize) -> Vec<Line<'static>> {
    if width == 0 {
        return vec![line.clone()];
    }
    let total_chars: usize = line.spans.iter().map(|s| s.content.chars().count()).sum();
    if total_chars <= width {
        return vec![line.clone()];
    }
    let mut out: Vec<Line<'static>> = Vec::new();
    let mut current: Vec<Span<'static>> = Vec::new();
    let mut current_w: usize = 0;
    for span in &line.spans {
        let mut buf = String::new();
        for ch in span.content.chars() {
            if current_w >= width {
                if !buf.is_empty() {
                    current.push(Span::styled(std::mem::take(&mut buf), span.style));
                }
                out.push(Line::from(std::mem::take(&mut current)));
                current_w = 0;
            }
            buf.push(ch);
            current_w += 1;
        }
        if !buf.is_empty() {
            current.push(Span::styled(buf, span.style));
        }
    }
    if !current.is_empty() {
        out.push(Line::from(current));
    }
    if out.is_empty() {
        out.push(line.clone());
    }
    out
}

fn render_text_block_skip(
    text: &str,
    area: Rect,
    text_style: Color,
    t: Theme,
    buf: &mut Buffer,
    skip: usize,
    expanded: bool,
) {
    // Expanded blocks lift the cap from 80 to 500 so the user can
    // see the full Read/Bash output without leaving the transcript.
    // Click on the tool block (or `o` / Ctrl+O) toggles `expanded`.
    let max_lines = if expanded { 500usize } else { 80usize };
    let width = area.width as usize;
    let mut lines: Vec<Line<'static>> = Vec::new();
    let mut count = 0usize;

    'outer: for raw in text.lines() {
        let wrapped = markdown::hard_wrap_str(raw, width.max(1));
        for chunk in wrapped {
            if count >= max_lines {
                let total = text.lines().count();
                lines.push(Line::from(Span::styled(
                    format!(
                        "… {} more lines · click or press o to expand",
                        total.saturating_sub(count)
                    ),
                    Style::default()
                        .fg(t.text_muted)
                        .add_modifier(Modifier::ITALIC),
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
    expanded: bool,
    diag_lines: &std::collections::HashMap<usize, crate::diagnostics::Severity>,
) {
    let (line_numbers, code) = split_line_numbers(text);
    let code_ref = code.as_deref().unwrap_or(text);

    let gutter_width = line_numbers
        .as_ref()
        .map(|nums| nums.iter().map(|n| n.len()).max().unwrap_or(0))
        .unwrap_or(0);

    // When we have any diagnostics for this file, reserve one column
    // for the severity glyph between the line number and separator
    // (` 12 ✘ │ `). When no diagnostics, the gutter stays at the
    // existing width so unaffected reads don't shift.
    let has_diags = !diag_lines.is_empty();
    let glyph_w: usize = if has_diags { 2 } else { 0 };
    let gutter_cols = if gutter_width > 0 {
        gutter_width + 3 + glyph_w
    } else {
        2
    };
    let code_w = (area.width as usize).saturating_sub(gutter_cols).max(10);

    // Cap matches the body in tool_content_height_with: 80 collapsed,
    // 500 expanded. Footer line tells the user how to see the rest.
    let max_lines = if expanded { 500usize } else { 80usize };
    let highlighted = markdown::highlight_code_raw(lang, code_ref, code_w, &t);
    let total = highlighted.len();
    let truncated = total > max_lines;
    let take_n = total.min(max_lines);

    let gutter_style = Style::default().fg(t.text_muted);
    let separator_style = Style::default().fg(t.border);

    let mut lines: Vec<Line<'static>> = highlighted
        .into_iter()
        .take(take_n)
        .enumerate()
        .map(|(i, mut hl_line)| {
            let mut spans = if let Some(nums) = &line_numbers {
                let num_str = nums.get(i).map(|s| s.as_str()).unwrap_or("");
                let mut spans_init = vec![Span::styled(
                    format!("{:>width$}", num_str, width = gutter_width),
                    gutter_style,
                )];
                // Severity glyph column: shows ✘/⚠/ℹ on lines that
                // have a diagnostic, blank otherwise. Color matches
                // severity. The lookup uses the parsed line number,
                // not the row index `i`, because Read tools may
                // start at a non-zero offset.
                if has_diags {
                    let lineno: usize = num_str.parse().unwrap_or(0);
                    let (glyph, color) = match diag_lines.get(&lineno) {
                        Some(crate::diagnostics::Severity::Error) => ("✘", t.error),
                        Some(crate::diagnostics::Severity::Warning) => ("⚠", t.warning),
                        Some(crate::diagnostics::Severity::Info) => ("ℹ", t.accent),
                        Some(crate::diagnostics::Severity::Hint) => ("★", t.text_secondary),
                        None => (" ", t.text_muted),
                    };
                    spans_init.push(Span::styled(
                        format!(" {glyph}"),
                        Style::default().fg(color).add_modifier(Modifier::BOLD),
                    ));
                }
                spans_init.push(Span::styled(" │ ", separator_style));
                spans_init
            } else {
                vec![Span::styled("│ ", separator_style)]
            };
            spans.extend(hl_line.spans.drain(..));
            Line::from(spans)
        })
        .collect();

    if truncated {
        lines.push(Line::from(Span::styled(
            format!(
                "… {} more lines · click or press o to expand",
                total - take_n
            ),
            Style::default()
                .fg(t.text_muted)
                .add_modifier(Modifier::ITALIC),
        )));
    }

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
    let path: &str = match &tool.input {
        ToolInput::Read { file_path, .. } => file_path.as_str(),
        ToolInput::Edit { file_path, .. } => file_path.as_str(),
        ToolInput::Write { file_path, .. } => file_path.as_str(),
        // Bash: when the user runs `cat path/file.ext`, `head -N file`,
        // or `tail file`, the stdout *is* the file content. Sniff
        // the command for one of those shapes and pull out the path
        // so the output gets the same language treatment as a Read.
        // Mirrors v126's bash → file-content highlighting heuristic.
        ToolInput::Bash { command, .. } => {
            return infer_lang_from_bash(command);
        }
        _ => return None,
    };
    lang_from_path(path)
}

fn lang_from_path(path: &str) -> Option<String> {
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

/// Quote-aware tokenizer. Splits `cmd` on whitespace except inside
/// matched single- or double-quoted segments, which are emitted as
/// a single token. `awk '{print $1}' file` → `["awk", "'{print $1}'",
/// "file"]`. Backslashes escape the next char outside quotes. We
/// keep the quote characters in the returned token so callers can
/// still detect "this token was quoted" by its leading char.
fn quote_aware_tokens(cmd: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut cur = String::new();
    let mut chars = cmd.chars().peekable();
    let mut quote: Option<char> = None;
    while let Some(c) = chars.next() {
        match (quote, c) {
            (None, ws) if ws.is_whitespace() => {
                if !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                }
            }
            (None, '\'') | (None, '"') => {
                cur.push(c);
                quote = Some(c);
            }
            (Some(q), c2) if c2 == q => {
                cur.push(c2);
                quote = None;
            }
            (None, '\\') => {
                cur.push('\\');
                if let Some(n) = chars.next() {
                    cur.push(n);
                }
            }
            _ => cur.push(c),
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

/// Replace the contents of every single- and double-quoted segment
/// in `cmd` with spaces, preserving the surrounding quotes and the
/// original length. Used to make the dangerous-meta-character checks
/// (`$`, `;`, etc.) quote-aware: `sed -n '1,$p' file` is a perfectly
/// safe sed call but the `$` lives inside `'…'` so we shouldn't
/// reject it. Without this, the canonical sed/awk idiom defeats the
/// language-inference path and the file falls back to plain rendering.
fn redact_quoted(cmd: &str) -> String {
    let mut out = String::with_capacity(cmd.len());
    let mut chars = cmd.chars().peekable();
    let mut quote: Option<char> = None;
    while let Some(c) = chars.next() {
        match (quote, c) {
            (None, '\'') | (None, '"') => {
                out.push(c);
                quote = Some(c);
            }
            (Some(q), c2) if c2 == q => {
                out.push(c2);
                quote = None;
            }
            (Some(_), _) => out.push(' '),
            (None, '\\') => {
                // Skip the next char so an escaped quote doesn't
                // start a fake quoted segment.
                out.push('\\');
                if let Some(n) = chars.next() {
                    out.push(n);
                }
            }
            (None, _) => out.push(c),
        }
    }
    out
}

/// Recognise `cat <file>` / `head <file>` / `tail <file>` commands
/// (with or without flags) and return the inferred language. Skips
/// when the command does anything fancier (pipes, redirects, multi-
/// file cats) — those need their own treatment, and over-applying
/// syntax highlighting to e.g. piped output breaks readability.
fn infer_lang_from_bash(command: &str) -> Option<String> {
    // Pipeline + chain aware. `cmd1 || cmd2` takes cmd1; `cmd | less`
    // takes cmd; `cd X && cat README.md` takes the LAST segment
    // (the cat). Same logic as `classify_bash_cmd` so the two
    // dispatch paths agree.
    let primary_alt = command
        .split("||")
        .next()
        .unwrap_or(command)
        .split('|')
        .next()
        .unwrap_or(command);
    let primary = primary_alt
        .split("&&")
        .filter(|s| !s.trim().is_empty())
        .last()
        .unwrap_or(primary_alt);
    let trimmed = primary.trim();

    // Reject command-substitution / backticks / lone `&` / `;` —
    // those still indicate the cat is wrapped in something funky
    // and the file-path sniff would lie. `&&` was already split
    // out so any `&` here is the lone-background form. Check
    // *outside* quoted strings so `sed -n '1,$p' file.md` (the
    // canonical "print all lines" idiom) doesn't get rejected for
    // its quoted `$`.
    let probe = redact_quoted(trimmed);
    if probe.contains('$')
        || probe.contains('`')
        || probe.contains('&')
        || probe.contains(';')
    {
        return None;
    }
    // Strip stderr-redirect tokens like `2>/dev/null` or `2>&1`
    // so the file-path sniff works on the cat side. We tokenize
    // *quote-aware* so awk's `'{print $1}'` (which contains a
    // whitespace) stays a single token instead of fragmenting and
    // confusing the file-path sniff.
    let toks: Vec<String> = quote_aware_tokens(trimmed)
        .into_iter()
        .filter(|t| !t.starts_with("2>") && !t.starts_with('>'))
        .collect();
    let mut it = toks.iter().map(|s| s.as_str());
    let verb = it.next()?;
    if !matches!(verb, "cat" | "head" | "tail" | "bat" | "less" | "more"
        | "sed" | "awk" | "perl" | "jq" | "yq" | "python" | "python3" | "node") {
        return None;
    }

    // jq/yq always output JSON/YAML
    if matches!(verb, "jq") {
        return Some("json".to_string());
    }
    if matches!(verb, "yq") {
        return Some("yaml".to_string());
    }
    // python/node inline scripts — highlight as that language
    if matches!(verb, "python" | "python3") {
        return Some("python".to_string());
    }
    if matches!(verb, "node") {
        return Some("javascript".to_string());
    }
    // Pick the file-path argument. For most verbs the first
    // non-flag/non-numeric token is the file. For sed/awk/perl the
    // FIRST positional is the script (`'1,$p'`, `'{print}'`, ...);
    // the file is the next positional. Detect a script positional
    // by its leading quote character (the tokenizer kept quotes
    // because we split on whitespace, not via a real shell parser).
    let script_verb = matches!(verb, "sed" | "awk" | "perl");
    let mut seen_positional = false;
    let mut file: Option<&str> = None;
    for arg in it {
        if arg.starts_with('-') {
            continue;
        }
        if arg.parse::<i64>().is_ok() {
            continue;
        }
        // For sed/awk/perl: skip the first positional iff it looks
        // like a script (starts with a quote). A bare path with no
        // surrounding quotes still wins, so `awk file.txt` works
        // (degenerate but harmless).
        if script_verb
            && !seen_positional
            && (arg.starts_with('\'') || arg.starts_with('"'))
        {
            seen_positional = true;
            continue;
        }
        file = Some(arg);
        break;
    }
    let path = file?;
    lang_from_path(path)
}

/// Heuristic: does this text look like markdown content? Used when
/// the file path didn't tell us (e.g. `.sisyphus`, `README` with no
/// extension, hidden dotfile that happens to be MD). Counts the
/// most distinctive markers in the first 2KB so a long file's
/// detection is cheap.
fn looks_like_markdown(text: &str) -> bool {
    let prefix: &str = if text.len() > 2048 {
        &text[..2048]
    } else {
        text
    };
    let mut score = 0;
    // Header lines are the strongest signal — `# ` / `## ` at start
    // of any line is rare in non-markdown text.
    for line in prefix.lines().take(60) {
        let l = line.trim_start();
        if l.starts_with("# ") || l.starts_with("## ") || l.starts_with("### ") {
            score += 2;
        }
        if l.starts_with("- ") || l.starts_with("* ") {
            score += 1;
        }
        if l.starts_with("```") {
            score += 2;
        }
        if l.contains("**") {
            score += 1;
        }
        if l.contains("|") && l.contains("---") {
            // Table separator row.
            score += 2;
        }
    }
    score >= 4
}

fn render_highlighted_block_skip(
    lang: &str,
    code: &str,
    area: Rect,
    t: Theme,
    buf: &mut Buffer,
    skip: usize,
    expanded: bool,
) {
    let inner_w = area.width.saturating_sub(2) as usize;
    let max_lines = if expanded { 500usize } else { 80usize };
    let mut lines = markdown::highlight_code(lang, code, inner_w, &t);
    let total = lines.len();
    if total > max_lines {
        lines.truncate(max_lines);
        lines.push(Line::from(Span::styled(
            format!(
                "… {} more lines · click or press o to expand",
                total - max_lines
            ),
            Style::default()
                .fg(t.text_muted)
                .add_modifier(Modifier::ITALIC),
        )));
    }
    Paragraph::new(lines)
        .style(Style::default().bg(t.bg))
        .scroll((skip as u16, 0))
        .render(area, buf);
}

/// What kind of bash command produced this output, derived purely
/// from the command string. Drives renderer dispatch — each kind
/// has its own visual treatment.
#[derive(Debug, Clone)]
enum BashCmdKind {
    /// `grep` / `rg` / `ack` results: `path:line:match` per line.
    Grep,
    /// `find` / `ls` / `tree` / `fd` etc. — flat path list.
    PathList,
    /// `git diff` / `git show` / raw `diff -u` — unified diff with
    /// `+`/`-`/`@@` lines that should be colored.
    GitDiff,
    /// `git log` — commit metadata + body.
    GitLog,
    /// `jq` — output is always JSON.
    Json,
    /// `cargo test` / `cargo check` / `make` — compiler/test output.
    CompilerOutput,
    /// `curl` — HTTP response (may be JSON/HTML/XML).
    HttpResponse,
    /// `xxd` / `hexyl` / `od` — hex dump (offset · bytes · ASCII).
    HexDump,
    /// `docker ps` / `docker images` / `kubectl get` — fixed-width
    /// table with a header row and aligned columns.
    TabularList,
    /// Plain command (default).
    Other,
}

/// Classify the *primary* command (first segment of `||` / `|`)
/// for output-rendering dispatch. Independent of the
/// `infer_lang_from_bash` path which is for cat-and-friends file
/// content; this one routes structured tools (grep, find, git).
fn classify_bash_cmd(command: &str) -> BashCmdKind {
    // Pipeline / chain decomposition. We walk in this order:
    //   1. split on `||` (cat-with-fallback pattern),
    //   2. split on `|` (pipe to less etc.),
    //   3. split on `&&` (cd-and-then pattern: `cd X && grep …`).
    // For (3) we take the LAST segment because the chain semantically
    // ends with the meaningful command — `cd ~/dir && cat README.md`
    // is "the cat is what produces output", not the cd.
    let primary_alt = command
        .split("||")
        .next()
        .unwrap_or(command)
        .split('|')
        .next()
        .unwrap_or(command);
    let primary = primary_alt
        .split("&&")
        .filter(|s| !s.trim().is_empty())
        .last()
        .unwrap_or(primary_alt);
    let trimmed = primary.trim();
    // Reject only the *truly* fancy patterns now: command
    // substitution, backticks, sequential `;`, background `&` not
    // covered by `&&` (single-`&` daemonization). The earlier
    // version blanket-rejected `&` which broke `cd X && cmd` for
    // every structured tool.
    // Quote-aware meta-character check: `sed -n '1,$p' file` is a
    // benign call and shouldn't be rejected for its quoted `$`.
    let probe = redact_quoted(trimmed);
    if probe.contains('$') || probe.contains('`') || probe.contains(';') {
        return BashCmdKind::Other;
    }
    // Reject lone `&` (background) — but `&&` was already split
    // out above, so any `&` left here is the lone form.
    if probe.contains('&') {
        return BashCmdKind::Other;
    }
    let toks: Vec<&str> = trimmed
        .split_whitespace()
        .filter(|t| !t.starts_with("2>") && !t.starts_with(">"))
        .collect();
    let Some(verb) = toks.first() else {
        return BashCmdKind::Other;
    };
    // git subcommand routing — `git diff`, `git show`, `git log`
    // each get their own renderer.
    if *verb == "git" {
        if let Some(sub) = toks.get(1) {
            match *sub {
                "diff" | "show" => return BashCmdKind::GitDiff,
                "log" => return BashCmdKind::GitLog,
                _ => return BashCmdKind::Other,
            }
        }
        return BashCmdKind::Other;
    }
    match *verb {
        "grep" | "rg" | "ack" | "ag" => BashCmdKind::Grep,
        "find" | "ls" | "tree" | "fd" | "exa" | "eza" => BashCmdKind::PathList,
        "jq" | "yq" => BashCmdKind::Json,
        // Raw POSIX `diff` (with -u/--unified) emits the same +/-/@@
        // shape `git diff` does — share the renderer so coloring
        // works for ad-hoc `diff -u a b` invocations too.
        "diff" => BashCmdKind::GitDiff,
        "cargo" => {
            if let Some(sub) = toks.get(1) {
                match *sub {
                    "test" | "check" | "build" | "clippy" => BashCmdKind::CompilerOutput,
                    _ => BashCmdKind::Other,
                }
            } else {
                BashCmdKind::Other
            }
        }
        "make" | "cmake" | "gcc" | "g++" | "rustc" | "tsc" | "npm" | "yarn" | "pnpm" => {
            BashCmdKind::CompilerOutput
        }
        "curl" | "wget" | "httpie" | "http" => BashCmdKind::HttpResponse,
        "xxd" | "hexyl" | "od" => BashCmdKind::HexDump,
        // Container / k8s tools — `docker ps`, `docker images`,
        // `kubectl get …`, `podman ps` — output is always a header
        // row + fixed-width columns.
        "docker" | "podman" => match toks.get(1).copied() {
            Some("ps") | Some("images") | Some("image") | Some("container")
            | Some("network") | Some("volume") => BashCmdKind::TabularList,
            _ => BashCmdKind::Other,
        },
        "kubectl" | "k9s" | "oc" => match toks.get(1).copied() {
            Some("get") | Some("describe") | Some("top") => BashCmdKind::TabularList,
            _ => BashCmdKind::Other,
        },
        _ => BashCmdKind::Other,
    }
}

/// Style a file path differently by its extension/family. Covers
/// the common languages that `grep -rn`, `find`, and `ls` results
/// surface. Falls back to muted gray for anything unknown — paths
/// still read clearly but don't pull attention.
fn path_color(path: &str, t: Theme) -> Color {
    let ext = std::path::Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    match ext.as_str() {
        // Code
        "rs" | "go" | "py" | "js" | "ts" | "tsx" | "jsx" | "rb" | "java" | "c" | "cpp" | "h"
        | "hpp" | "swift" | "kt" | "lua" | "zig" | "ml" | "hs" | "ex" | "exs" => t.accent,
        // Config / data
        "toml" | "yaml" | "yml" | "json" | "ini" | "cfg" | "conf" | "env" | "lock" => {
            t.text_secondary
        }
        // Docs
        "md" | "mdx" | "rst" | "txt" | "adoc" => t.text_primary,
        // Web
        "html" | "css" | "scss" | "sass" | "less" | "vue" | "svelte" => t.success,
        // Shell
        "sh" | "bash" | "zsh" | "fish" => t.warning,
        _ => t.text_muted,
    }
}

/// Parsed grep/rg result line. `Match` covers both real match
/// rows (`:` separator) and context rows (`-` separator from
/// grep `-A`/`-B`/`-C`); `HeadingPath` is the rg `--heading`
/// bare-path-on-its-own-line form.
enum GrepLine<'a> {
    Match {
        path: &'a str,
        lineno: Option<&'a str>,
        col: Option<&'a str>,
        body: &'a str,
        is_context: bool,
    },
    HeadingPath(&'a str),
}

/// Parse a single grep / rg result line into its components.
/// Tries the structured forms in order: column-form
/// (`path:line:col:body`), match (`path:line:body`), file-only
/// (`path:body`), single-file `<line>:<body>` (no path prefix),
/// context with `-` separators, then bare-path heading.
fn parse_grep_line<'a>(raw: &'a str) -> Option<GrepLine<'a>> {
    // Try `:` separator first (most common).
    if let Some(parsed) = parse_grep_with_sep(raw, ':', false) {
        return Some(parsed);
    }
    // Then `-` for context lines.
    if let Some(parsed) = parse_grep_with_sep(raw, '-', true) {
        return Some(parsed);
    }
    // No path prefix: `grep -n pat single-file` emits `<lineno>:<body>`.
    // Also rg `--no-filename`. Detect by leading digits + `:`.
    if let Some(parsed) = parse_grep_no_path(raw, ':', false) {
        return Some(parsed);
    }
    // No-path context (grep `-A`/`-B`/`-C` against single file):
    // `<lineno>-<body>`.
    if let Some(parsed) = parse_grep_no_path(raw, '-', true) {
        return Some(parsed);
    }
    // Fall back to bare-path detection: a line that *looks like* a
    // file path (has slash or extension) and contains no `:` or
    // `-` markers is probably a heading.
    let trimmed = raw.trim();
    if !trimmed.is_empty()
        && (trimmed.contains('/') || std::path::Path::new(trimmed).extension().is_some())
        && !trimmed.contains(':')
    {
        return Some(GrepLine::HeadingPath(trimmed));
    }
    None
}

/// Parse the path-less `<lineno><sep><body>` form. Used by single-
/// file grep invocations where the filename isn't repeated on each
/// line. Returns `Match` with `path = ""` so the renderer skips
/// the path span entirely.
fn parse_grep_no_path<'a>(raw: &'a str, sep: char, is_context: bool) -> Option<GrepLine<'a>> {
    let bytes = raw.as_bytes();
    if bytes.is_empty() || !bytes[0].is_ascii_digit() {
        return None;
    }
    let mut j = 0;
    while j < bytes.len() && bytes[j].is_ascii_digit() {
        j += 1;
    }
    // After the digit run, expect the separator. Reject if the
    // digit run is the whole line (just a number, no body).
    if j >= bytes.len() || bytes[j] != sep as u8 {
        return None;
    }
    let lineno = &raw[..j];
    let body = &raw[j + 1..];
    // Reasonable line numbers are 1..=10M. Anything wildly larger
    // is probably a different format (a hex offset, a hash) we
    // shouldn't false-match.
    if lineno.parse::<u32>().is_err() {
        return None;
    }
    Some(GrepLine::Match {
        path: "",
        lineno: Some(lineno),
        col: None,
        body,
        is_context,
    })
}

/// Look for `path<sep>lineno<sep>[col<sep>]body` in `raw`.
/// Returns None if the structure doesn't match — caller falls
/// through to the next separator or the heading-path fallback.
fn parse_grep_with_sep<'a>(raw: &'a str, sep: char, is_context: bool) -> Option<GrepLine<'a>> {
    // Walk the string finding `<sep><digits><sep>` — that
    // anchors the "this is a (path, lineno) prefix" claim. Without
    // the digit-bracketed pattern, a path like
    // `src/foo:bar.rs:10:hi` would mis-parse.
    let bytes = raw.as_bytes();
    let sep_b = sep as u8;
    let mut i = 0;
    let mut path_end: Option<usize> = None;
    while i < bytes.len() {
        if bytes[i] == sep_b {
            // Tentative path ends at i. After i+1, we want digits
            // then another sep.
            let after = i + 1;
            let mut j = after;
            while j < bytes.len() && bytes[j].is_ascii_digit() {
                j += 1;
            }
            if j > after && j < bytes.len() && bytes[j] == sep_b {
                path_end = Some(i);
                break;
            }
        }
        i += 1;
    }
    let p_end = path_end?;
    let path = &raw[..p_end];
    if path.is_empty() {
        return None;
    }
    let after_path = p_end + 1;
    let mut lineno_end = after_path;
    while lineno_end < bytes.len() && bytes[lineno_end].is_ascii_digit() {
        lineno_end += 1;
    }
    if lineno_end == after_path || lineno_end >= bytes.len() || bytes[lineno_end] != sep_b {
        return None;
    }
    let lineno = &raw[after_path..lineno_end];
    let after_lineno = lineno_end + 1;
    // Optional column: another `<digits><sep>` block.
    let mut col: Option<&str> = None;
    let body_start;
    let mut col_end = after_lineno;
    while col_end < bytes.len() && bytes[col_end].is_ascii_digit() {
        col_end += 1;
    }
    if col_end > after_lineno && col_end < bytes.len() && bytes[col_end] == sep_b {
        col = Some(&raw[after_lineno..col_end]);
        body_start = col_end + 1;
    } else {
        body_start = after_lineno;
    }
    let body = &raw[body_start..];
    Some(GrepLine::Match {
        path,
        lineno: Some(lineno),
        col,
        body,
        is_context,
    })
}

/// Walk the original command and return the first positional that
/// looks like a file/directory the user grep'd against. Used by
/// `render_grep_output_skip` to surface a heading line when grep
/// emitted path-less `<lineno>:<body>` rows (single-file mode), so
/// the user can see *which* file is being searched.
///
/// Heuristic: skip the verb (`grep`/`rg`/`ack`/`ag`), skip flags
/// (`-X`, `--long`), skip the value of flag pairs that take an
/// argument (`-e PAT`, `-f FILE`, `--type rust`), skip what looks
/// like the regex pattern (the first un-quoted positional). The
/// next positional is the target file/path. Quote-aware so a
/// pattern like `"foo("` doesn't get mistaken for a path. Returns
/// the path with surrounding quotes stripped.
fn grep_target_file(cmd: &str) -> Option<String> {
    let toks = quote_aware_tokens(cmd);
    let mut it = toks.into_iter();
    let verb = it.next()?;
    if !matches!(
        verb.as_str(),
        "grep" | "rg" | "ack" | "ag" | "ripgrep"
    ) {
        return None;
    }
    // Flags whose value lives in the *next* token. Skip both.
    const VALUE_FLAGS: &[&str] = &[
        "-e", "-f", "-A", "-B", "-C", "-m", "--max-count", "--type", "-t",
        "--type-not", "-T", "--color", "--colour", "-g", "--glob", "--iglob",
        "--include", "--exclude", "--exclude-dir", "--threads", "-j",
    ];
    // `-e PAT` and `-f FILE` (regex source file) supply the pattern
    // via a flag value rather than a positional. When we see one of
    // those we absorb the value AND mark seen_pattern so the next
    // positional is treated as the target file.
    const PATTERN_FLAGS: &[&str] = &["-e", "--regexp", "-f", "--file"];
    let mut seen_pattern = false;
    while let Some(tok) = it.next() {
        if tok.starts_with("--") {
            let key = tok.split('=').next().unwrap_or(&tok);
            if PATTERN_FLAGS.iter().any(|f| *f == key) {
                if !tok.contains('=') {
                    let _ = it.next();
                }
                seen_pattern = true;
                continue;
            }
            if !tok.contains('=') && VALUE_FLAGS.iter().any(|f| *f == tok.as_str()) {
                let _ = it.next();
            }
            continue;
        }
        if tok.starts_with('-') && tok.len() > 1 && !tok.chars().all(|c| c == '-') {
            if PATTERN_FLAGS.iter().any(|f| *f == tok.as_str()) {
                let _ = it.next();
                seen_pattern = true;
                continue;
            }
            if VALUE_FLAGS.iter().any(|f| *f == tok.as_str()) {
                let _ = it.next();
            }
            continue;
        }
        if !seen_pattern {
            seen_pattern = true;
            continue;
        }
        // First positional after the pattern → target.
        let unquoted = tok
            .strip_prefix('\'')
            .and_then(|s| s.strip_suffix('\''))
            .or_else(|| tok.strip_prefix('"').and_then(|s| s.strip_suffix('"')))
            .map(|s| s.to_string())
            .unwrap_or(tok);
        return Some(unquoted);
    }
    None
}

/// Render `grep -rn` / `rg` / `ack` output. Handles all the
/// formats those tools emit (verified against ripgrep's
/// `crates/printer/src/standard.rs` and GNU grep's `print_sep`):
///
/// - `path:line:col:match`   (rg with `--column`)
/// - `path:line:match`       (default rg / `grep -n`)
/// - `path:match`            (no line numbers, e.g. `grep -h`)
/// - `path-line-context`     (grep `-A`/`-B`/`-C`, context uses `-`)
/// - `--`                    (group separator between matches)
/// - bare path on its own line (rg `--heading` mode)
///
/// Path gets its language-tinted color, line number warning-yellow
/// (matches grep's default), `:` separators muted, match body in
/// surface text color. Context lines (`-` separator) dim their
/// body to differentiate from matches.
#[allow(clippy::too_many_arguments)]
fn render_grep_output_skip(
    stdout: &str,
    stderr: &str,
    exit_code: Option<i32>,
    area: Rect,
    t: Theme,
    buf: &mut Buffer,
    skip: usize,
    expanded: bool,
    cmd: &str,
) {
    let max_lines = if expanded { 500usize } else { 80usize };
    let mut lines: Vec<Line<'static>> = Vec::new();
    if let Some(code) = exit_code {
        // grep returns 1 for "no matches found" — that's not a
        // failure visually, just an empty result. Only color the
        // exit code red for truly weird codes (>1).
        if code > 1 {
            lines.push(Line::from(Span::styled(
                format!("[exit {code}]"),
                Style::default().fg(t.error),
            )));
        }
    }
    // Single-file grep (`grep -n PAT file.rs`) emits `<lineno>:body`
    // with no path prefix on each line. Without a heading the user
    // can't tell which file they searched — surface the file path
    // we extracted from the command so each match has context.
    let first_data = stdout.lines().find(|l| !l.is_empty());
    let pathless = first_data
        .map(|l| matches!(parse_grep_line(l), Some(GrepLine::Match { path: "", .. })))
        .unwrap_or(false);
    if pathless
        && let Some(target) = grep_target_file(cmd)
    {
        lines.push(Line::from(Span::styled(
            target,
            Style::default()
                .fg(t.accent)
                .add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
        )));
    }
    let mut total = 0usize;
    for raw in stdout.lines() {
        total += 1;
        if lines.len() >= max_lines {
            continue;
        }
        // Group separator from grep `-A`/`-B`/`-C`: literal `--`.
        if raw == "--" {
            lines.push(Line::from(Span::styled(
                "──".to_string(),
                Style::default().fg(t.text_muted),
            )));
            continue;
        }

        // Try to peel off `path<sep1><lineno><sep2>[<col><sep3>]<body>`
        // where `sep1` and `sep2` are both `:` for matches, both `-`
        // for context lines (per GNU grep's `print_sep` and rg's
        // `write_prelude`). Mixing `:` and `-` doesn't happen — each
        // line is either fully match or fully context.
        let parsed = parse_grep_line(raw);
        match parsed {
            Some(GrepLine::Match {
                path,
                lineno,
                col,
                body,
                is_context,
            }) => {
                let sep_color = if is_context {
                    t.text_muted
                } else {
                    t.text_muted
                };
                let body_color = if is_context {
                    t.text_muted
                } else {
                    t.text_secondary
                };
                let lineno_color = if is_context { t.text_muted } else { t.warning };
                let sep_str = if is_context { "-" } else { ":" };
                let mut spans: Vec<Span<'static>> = Vec::new();
                // Skip the path span when grep was invoked against a
                // single file and didn't repeat the filename on each
                // line — `parse_grep_line` returns `path = ""` for
                // that form.
                if !path.is_empty() {
                    spans.push(Span::styled(
                        path.to_owned(),
                        Style::default().fg(path_color(path, t)),
                    ));
                }
                if let Some(n) = lineno {
                    if !path.is_empty() {
                        spans.push(Span::styled(
                            sep_str.to_owned(),
                            Style::default().fg(sep_color),
                        ));
                    }
                    spans.push(Span::styled(
                        n.to_owned(),
                        Style::default().fg(lineno_color),
                    ));
                }
                if let Some(c) = col {
                    spans.push(Span::styled(
                        sep_str.to_owned(),
                        Style::default().fg(sep_color),
                    ));
                    spans.push(Span::styled(
                        c.to_owned(),
                        Style::default().fg(t.text_muted),
                    ));
                }
                spans.push(Span::styled(
                    sep_str.to_owned(),
                    Style::default().fg(sep_color),
                ));
                spans.push(Span::styled(
                    body.to_owned(),
                    Style::default().fg(body_color),
                ));
                lines.push(Line::from(spans));
            }
            Some(GrepLine::HeadingPath(path)) => {
                // `--heading` mode: bare path on its own line.
                lines.push(Line::from(Span::styled(
                    path.to_owned(),
                    Style::default()
                        .fg(path_color(path, t))
                        .add_modifier(Modifier::BOLD),
                )));
            }
            None => {
                lines.push(Line::from(Span::styled(
                    raw.to_owned(),
                    Style::default().fg(t.text_secondary),
                )));
            }
        }
    }
    if total > max_lines {
        lines.push(Line::from(Span::styled(
            format!(
                "… {} more lines · click or press o to expand",
                total - max_lines
            ),
            Style::default()
                .fg(t.text_muted)
                .add_modifier(Modifier::ITALIC),
        )));
    }
    if !stderr.is_empty() {
        lines.push(Line::from(""));
        for sl in stderr.lines() {
            lines.push(Line::from(Span::styled(
                sl.to_owned(),
                Style::default().fg(t.error),
            )));
        }
    }
    Paragraph::new(lines)
        .style(Style::default().bg(t.bg))
        .scroll((skip as u16, 0))
        .render(area, buf);
}

/// Render `find` / `ls` / `tree` / `fd` output as a list of paths
/// colored by file extension. Multi-column `ls` output (no flags)
/// is split on whitespace and each entry gets its own colored
/// span; `ls -l` lines get split by column with file mode in muted,
/// size right-aligned, name colored.
#[allow(clippy::too_many_arguments)]
fn render_path_list_output_skip(
    stdout: &str,
    stderr: &str,
    exit_code: Option<i32>,
    area: Rect,
    t: Theme,
    buf: &mut Buffer,
    skip: usize,
    expanded: bool,
) {
    let max_lines = if expanded { 500usize } else { 80usize };
    let mut lines: Vec<Line<'static>> = Vec::new();
    if let Some(code) = exit_code {
        if code != 0 {
            lines.push(Line::from(Span::styled(
                format!("[exit {code}]"),
                Style::default().fg(t.error),
            )));
        }
    }
    let mut total = 0usize;
    for raw in stdout.lines() {
        total += 1;
        if lines.len() >= max_lines {
            continue;
        }
        // `ls -l` long format: `<perms> <links> <user> <group> <size> <date> <name>`
        // — first char is a file-type indicator (`-`, `d`, `l`, etc.).
        let is_ls_long = raw
            .chars()
            .next()
            .map(|c| matches!(c, '-' | 'd' | 'l' | 'c' | 'b' | 'p' | 's'))
            .unwrap_or(false)
            && raw.split_whitespace().count() >= 7;
        if is_ls_long {
            let cols: Vec<&str> = raw.splitn(9, char::is_whitespace).collect();
            // Re-split smarter: we want file mode, ..., name (which
            // may contain spaces in `ls -lQ` etc.).
            let parts: Vec<&str> = raw.split_whitespace().collect();
            if parts.len() >= 8 {
                let perms = parts[0];
                // Find the size column (5th non-empty token after links)
                let name_start = parts[..parts.len() - 1]
                    .iter()
                    .map(|s| s.len())
                    .sum::<usize>()
                    + parts.len()
                    - 2; // approximation
                let name = parts.last().copied().unwrap_or("");
                let _ = name_start;
                let _ = cols;
                let mut spans: Vec<Span<'static>> = Vec::new();
                spans.push(Span::styled(
                    perms.to_owned(),
                    Style::default().fg(t.text_muted),
                ));
                spans.push(Span::raw(" "));
                // Middle columns rendered muted as one block.
                let middle = parts[1..parts.len() - 1].join(" ");
                spans.push(Span::styled(middle, Style::default().fg(t.text_muted)));
                spans.push(Span::raw(" "));
                spans.push(Span::styled(
                    name.to_owned(),
                    Style::default().fg(path_color(name, t)),
                ));
                lines.push(Line::from(spans));
                continue;
            }
        }
        // Simple path-per-line: tint by extension.
        let trimmed = raw.trim_end();
        if trimmed.is_empty() {
            lines.push(Line::from(""));
        } else {
            lines.push(Line::from(Span::styled(
                raw.to_owned(),
                Style::default().fg(path_color(trimmed, t)),
            )));
        }
    }
    if total > max_lines {
        lines.push(Line::from(Span::styled(
            format!(
                "… {} more lines · click or press o to expand",
                total - max_lines
            ),
            Style::default()
                .fg(t.text_muted)
                .add_modifier(Modifier::ITALIC),
        )));
    }
    if !stderr.is_empty() {
        lines.push(Line::from(""));
        for sl in stderr.lines() {
            lines.push(Line::from(Span::styled(
                sl.to_owned(),
                Style::default().fg(t.error),
            )));
        }
    }
    Paragraph::new(lines)
        .style(Style::default().bg(t.bg))
        .scroll((skip as u16, 0))
        .render(area, buf);
}

/// Render `git diff` / `git show` output as colored unified diff.
/// Each line gets a per-prefix color: `+` green, `-` red, `@@`
/// cyan, file headers bold, index/`diff --git` lines muted.
#[allow(clippy::too_many_arguments)]
fn render_git_diff_output_skip(
    stdout: &str,
    stderr: &str,
    exit_code: Option<i32>,
    area: Rect,
    t: Theme,
    buf: &mut Buffer,
    skip: usize,
    expanded: bool,
) {
    let max_lines = if expanded { 1000usize } else { 200usize };
    let mut lines: Vec<Line<'static>> = Vec::new();
    if let Some(code) = exit_code {
        // git diff exits 1 when there are differences (with --exit-code).
        // 0 = no diffs, 1 = diffs found, >1 = real error.
        if code > 1 {
            lines.push(Line::from(Span::styled(
                format!("[exit {code}]"),
                Style::default().fg(t.error),
            )));
        }
    }
    let mut total = 0usize;
    for raw in stdout.lines() {
        total += 1;
        if lines.len() >= max_lines {
            continue;
        }
        let style = if raw.starts_with("diff --git ") || raw.starts_with("index ") {
            Style::default().fg(t.text_muted)
        } else if raw.starts_with("--- ") || raw.starts_with("+++ ") {
            Style::default()
                .fg(t.text_primary)
                .add_modifier(Modifier::BOLD)
        } else if raw.starts_with("@@") {
            Style::default().fg(t.accent)
        } else if raw.starts_with('+') {
            Style::default().fg(t.success)
        } else if raw.starts_with('-') {
            Style::default().fg(t.error)
        } else {
            Style::default().fg(t.text_secondary)
        };
        lines.push(Line::from(Span::styled(raw.to_owned(), style)));
    }
    if total > max_lines {
        lines.push(Line::from(Span::styled(
            format!(
                "… {} more lines · click or press o to expand",
                total - max_lines
            ),
            Style::default()
                .fg(t.text_muted)
                .add_modifier(Modifier::ITALIC),
        )));
    }
    if !stderr.is_empty() {
        lines.push(Line::from(""));
        for sl in stderr.lines() {
            lines.push(Line::from(Span::styled(
                sl.to_owned(),
                Style::default().fg(t.error),
            )));
        }
    }
    Paragraph::new(lines)
        .style(Style::default().bg(t.bg))
        .scroll((skip as u16, 0))
        .render(area, buf);
}

/// Render `git log` output. Detects two formats:
///   - `--oneline`: `SHA message` — SHA in accent, rest plain
///   - default: `commit SHA\nAuthor: ...\nDate: ...\n\n    body\n`
///     — `commit` line in accent, Author/Date muted, body italic.
#[allow(clippy::too_many_arguments)]
fn render_git_log_output_skip(
    stdout: &str,
    stderr: &str,
    exit_code: Option<i32>,
    area: Rect,
    t: Theme,
    buf: &mut Buffer,
    skip: usize,
    expanded: bool,
) {
    let max_lines = if expanded { 500usize } else { 100usize };
    let mut lines: Vec<Line<'static>> = Vec::new();
    if let Some(code) = exit_code {
        if code != 0 {
            lines.push(Line::from(Span::styled(
                format!("[exit {code}]"),
                Style::default().fg(t.error),
            )));
        }
    }
    let mut total = 0usize;
    for raw in stdout.lines() {
        total += 1;
        if lines.len() >= max_lines {
            continue;
        }
        // Default format heuristic: lines starting with `commit `
        // followed by a hex SHA; `Author:` / `Date:` headers; body
        // indented with 4 spaces; everything else default.
        if let Some(rest) = raw.strip_prefix("commit ") {
            // Split SHA from any trailing decorations like
            // `(HEAD -> main, origin/main)`.
            let (sha, decoration) = rest
                .split_once(' ')
                .map(|(s, d)| (s, Some(d)))
                .unwrap_or((rest, None));
            let mut spans = vec![
                Span::styled("commit ", Style::default().fg(t.text_muted)),
                Span::styled(
                    sha.to_owned(),
                    Style::default().fg(t.accent).add_modifier(Modifier::BOLD),
                ),
            ];
            if let Some(d) = decoration {
                spans.push(Span::raw(" "));
                spans.push(Span::styled(d.to_owned(), Style::default().fg(t.warning)));
            }
            lines.push(Line::from(spans));
        } else if raw.starts_with("Author:") || raw.starts_with("Date:") {
            lines.push(Line::from(Span::styled(
                raw.to_owned(),
                Style::default().fg(t.text_muted),
            )));
        } else if raw.starts_with("    ") {
            // 4-space-indented body line.
            lines.push(Line::from(Span::styled(
                raw.to_owned(),
                Style::default().fg(t.text_secondary),
            )));
        } else {
            // `--oneline` format: <SHA> <msg>. Sniff a short hex
            // SHA at the start.
            if let Some(space) = raw.find(' ') {
                let (head, tail) = raw.split_at(space);
                let head_clean = head.trim_end_matches(|c: char| !c.is_ascii_alphanumeric());
                if !head_clean.is_empty()
                    && head_clean.len() >= 6
                    && head_clean.len() <= 40
                    && head_clean.chars().all(|c| c.is_ascii_hexdigit())
                {
                    lines.push(Line::from(vec![
                        Span::styled(
                            head.to_owned(),
                            Style::default().fg(t.accent).add_modifier(Modifier::BOLD),
                        ),
                        Span::styled(tail.to_owned(), Style::default().fg(t.text_secondary)),
                    ]));
                    continue;
                }
            }
            lines.push(Line::from(Span::styled(
                raw.to_owned(),
                Style::default().fg(t.text_secondary),
            )));
        }
    }
    if total > max_lines {
        lines.push(Line::from(Span::styled(
            format!(
                "… {} more lines · click or press o to expand",
                total - max_lines
            ),
            Style::default()
                .fg(t.text_muted)
                .add_modifier(Modifier::ITALIC),
        )));
    }
    if !stderr.is_empty() {
        lines.push(Line::from(""));
        for sl in stderr.lines() {
            lines.push(Line::from(Span::styled(
                sl.to_owned(),
                Style::default().fg(t.error),
            )));
        }
    }
    Paragraph::new(lines)
        .style(Style::default().bg(t.bg))
        .scroll((skip as u16, 0))
        .render(area, buf);
}

/// Render `cat <markdown-file>` output as actual rendered markdown
/// (formatted headers, tables, code fences) instead of syntax-
/// highlighted source. The user expects `cat README.md` to show
/// the document the way the model's prose is shown — not the raw
/// `# Header` characters with syntax coloring. Mirrors v126's
/// markdown rendering for tool output.
#[allow(clippy::too_many_arguments)]
fn render_cat_markdown_output_skip(
    stdout: &str,
    stderr: &str,
    exit_code: Option<i32>,
    area: Rect,
    t: Theme,
    buf: &mut Buffer,
    skip: usize,
) {
    const MAX_LINES: usize = 500;
    let inner_w = area.width.saturating_sub(2) as usize;
    let mut lines: Vec<Line> = Vec::new();

    // Optional exit-code badge for the rare case where a `cat`
    // succeeds but emits a non-zero exit (shouldn't happen, but
    // mirrors the behavior of the syntect path for parity).
    if let Some(code) = exit_code {
        if code != 0 {
            lines.push(Line::from(Span::styled(
                format!("[exit {code}]"),
                Style::default().fg(t.warning),
            )));
        }
    }

    // The actual markdown render — same pipeline the assistant's
    // text uses. `to_lines` handles headers, tables, code fences,
    // bullets, etc.
    let body = markdown::to_lines(stdout, &t, inner_w.max(1));
    lines.extend(body);

    if lines.len() > MAX_LINES {
        let total = lines.len();
        lines.truncate(MAX_LINES);
        lines.push(Line::from(Span::styled(
            format!(
                "… {} more lines · click or press o to expand",
                total - MAX_LINES
            ),
            Style::default()
                .fg(t.text_muted)
                .add_modifier(Modifier::ITALIC),
        )));
    }

    if !stderr.is_empty() {
        lines.push(Line::from(""));
        for sl in stderr.lines() {
            lines.push(Line::from(Span::styled(
                sl.to_owned(),
                Style::default().fg(t.error),
            )));
        }
    }

    Paragraph::new(lines)
        .style(Style::default().bg(t.bg))
        .wrap(ratatui::widgets::Wrap { trim: false })
        .scroll((skip as u16, 0))
        .render(area, buf);
}

/// Render `xxd` / `hexyl` / `od` hex-dump output. Each input line
/// has the canonical shape `OFFSET: BYTES  ASCII` (xxd) or hexyl's
/// boxed table form. We split on the first colon (offset/bytes) and
/// the doubled-space separator before the ASCII column, color each
/// region distinctly, and pass everything else through unstyled so
/// hexyl's box-drawing characters survive intact.
#[allow(clippy::too_many_arguments)]
fn render_hex_dump_output_skip(
    stdout: &str,
    stderr: &str,
    exit_code: Option<i32>,
    area: Rect,
    t: Theme,
    buf: &mut Buffer,
    skip: usize,
    expanded: bool,
) {
    let max_lines = if expanded { 1000usize } else { 200usize };
    let mut lines: Vec<Line<'static>> = Vec::new();
    if let Some(code) = exit_code
        && code != 0
    {
        lines.push(Line::from(Span::styled(
            format!("[exit {code}]"),
            Style::default().fg(t.error),
        )));
    }
    let mut total = 0usize;
    for raw in stdout.lines() {
        total += 1;
        if lines.len() >= max_lines {
            continue;
        }
        // xxd canonical form: `00000000: 4865 6c6c 6f0a                           Hello.`
        // hexyl decorates with │ │ box separators — let those
        // pass through styled neutrally.
        if let Some((offset, rest)) = raw.split_once(':') {
            // Heuristic for the hex/ASCII split: xxd uses two
            // consecutive spaces, hexyl uses ` │ ` separators.
            let (bytes, ascii) = if let Some(idx) = rest.find("  ") {
                let (a, b) = rest.split_at(idx);
                (a, b.trim_start())
            } else if let Some(idx) = rest.find(" │ ") {
                let (a, b) = rest.split_at(idx);
                (a, &b[3..])
            } else {
                (rest, "")
            };
            // Sanity check: real offsets are mostly hex digits.
            // A non-hex prefix means we're looking at unrelated
            // output (stderr-style line) — fall back to plain.
            let looks_offset = !offset.is_empty()
                && offset.trim_start().chars().all(|c| c.is_ascii_hexdigit());
            if looks_offset {
                let mut spans = vec![
                    Span::styled(
                        format!("{offset}:"),
                        Style::default().fg(t.text_muted),
                    ),
                    Span::styled(
                        bytes.to_owned(),
                        Style::default().fg(t.accent),
                    ),
                ];
                if !ascii.is_empty() {
                    spans.push(Span::raw("  "));
                    spans.push(Span::styled(
                        ascii.to_owned(),
                        Style::default().fg(t.text_secondary),
                    ));
                }
                lines.push(Line::from(spans));
                continue;
            }
        }
        // hexyl header / footer / unknown line — keep raw.
        lines.push(Line::from(Span::styled(
            raw.to_owned(),
            Style::default().fg(t.text_muted),
        )));
    }
    if total > max_lines {
        lines.push(Line::from(Span::styled(
            format!(
                "… {} more lines · click or press o to expand",
                total - max_lines
            ),
            Style::default()
                .fg(t.text_muted)
                .add_modifier(Modifier::ITALIC),
        )));
    }
    if !stderr.is_empty() {
        lines.push(Line::from(""));
        for sl in stderr.lines() {
            lines.push(Line::from(Span::styled(
                sl.to_owned(),
                Style::default().fg(t.error),
            )));
        }
    }
    Paragraph::new(lines)
        .style(Style::default().bg(t.bg))
        .scroll((skip as u16, 0))
        .render(area, buf);
}

/// Render `docker ps` / `docker images` / `kubectl get …` and
/// similar fixed-width tables. The first non-empty stdout line is
/// the column header (uppercase column names) — bold it and use the
/// accent color so it pops; body rows alternate between primary and
/// muted text so wide tables remain scannable. Container/pod state
/// columns get an extra tint when we recognise the value (`Running`,
/// `Up …`, `Exited`, `Error`, `CrashLoopBackOff`).
#[allow(clippy::too_many_arguments)]
fn render_tabular_list_output_skip(
    stdout: &str,
    stderr: &str,
    exit_code: Option<i32>,
    area: Rect,
    t: Theme,
    buf: &mut Buffer,
    skip: usize,
    expanded: bool,
) {
    let max_lines = if expanded { 500usize } else { 100usize };
    let mut lines: Vec<Line<'static>> = Vec::new();
    if let Some(code) = exit_code
        && code != 0
    {
        lines.push(Line::from(Span::styled(
            format!("[exit {code}]"),
            Style::default().fg(t.error),
        )));
    }
    let mut header_drawn = false;
    let mut total = 0usize;
    for raw in stdout.lines() {
        total += 1;
        if lines.len() >= max_lines {
            continue;
        }
        if !header_drawn && !raw.trim().is_empty() {
            lines.push(Line::from(Span::styled(
                raw.to_owned(),
                Style::default()
                    .fg(t.accent)
                    .add_modifier(Modifier::BOLD),
            )));
            header_drawn = true;
            continue;
        }
        // Tint a status word if we can spot one. We don't try to
        // parse columns — just look at the line for known tokens.
        let style = if raw.contains("CrashLoopBackOff")
            || raw.contains("Error")
            || raw.contains("Exited")
        {
            Style::default().fg(t.error)
        } else if raw.contains("Running") || raw.starts_with("Up ") || raw.contains(" Up ") {
            Style::default().fg(t.success)
        } else if raw.contains("Pending") || raw.contains("ContainerCreating") {
            Style::default().fg(t.warning)
        } else {
            Style::default().fg(t.text_primary)
        };
        lines.push(Line::from(Span::styled(raw.to_owned(), style)));
    }
    if total > max_lines {
        lines.push(Line::from(Span::styled(
            format!(
                "… {} more lines · click or press o to expand",
                total - max_lines
            ),
            Style::default()
                .fg(t.text_muted)
                .add_modifier(Modifier::ITALIC),
        )));
    }
    if !stderr.is_empty() {
        lines.push(Line::from(""));
        for sl in stderr.lines() {
            lines.push(Line::from(Span::styled(
                sl.to_owned(),
                Style::default().fg(t.error),
            )));
        }
    }
    Paragraph::new(lines)
        .style(Style::default().bg(t.bg))
        .scroll((skip as u16, 0))
        .render(area, buf);
}

/// Render `cargo build` / `cargo test` / `cargo check` / `make` /
/// `npm run build` output. Routes recognised line shapes to colored
/// styles so the user can scan a long compile log at a glance:
///
///   * `Compiling foo v1.2.3` → muted (info, lots of these scroll by)
///   * `Finished … in N.NNs` / `Finished` → success green, bold
///   * `Building [...]` progress bars → accent
///   * `error[E0123]:` / `error: …` → error red, bold prefix
///   * `warning:` → warning yellow, bold prefix
///   * `note:` / `help:` → accent muted
///   * `--> path:line:col` location markers → accent
///   * `running N tests` / `test result: ok. N passed` → success
///   * `test foo::bar ... ok` → success; `... FAILED` → error
///   * `failures:` block headers → error
///   * Everything else → text_secondary
#[allow(clippy::too_many_arguments)]
fn render_compiler_output_skip(
    stdout: &str,
    stderr: &str,
    exit_code: Option<i32>,
    area: Rect,
    t: Theme,
    buf: &mut Buffer,
    skip: usize,
    expanded: bool,
) {
    let max_lines = if expanded { 1500usize } else { 300usize };
    let mut lines: Vec<Line<'static>> = Vec::new();
    if let Some(code) = exit_code
        && code != 0
    {
        let badge_color = if code == 101 || code == 1 {
            t.error
        } else {
            t.warning
        };
        lines.push(Line::from(Span::styled(
            format!("[exit {code}]"),
            Style::default().fg(badge_color).add_modifier(Modifier::BOLD),
        )));
    }
    let mut total = 0usize;
    // `cargo` writes status to stderr (Compiling/Finished/warning),
    // diagnostics to stderr too, and final binary output to stdout.
    // Walk both streams in order — stderr first (the build log),
    // then stdout (test output, run output).
    for raw in stderr.lines().chain(stdout.lines()) {
        total += 1;
        if lines.len() >= max_lines {
            continue;
        }
        let trimmed = raw.trim_start();
        let leading_ws_len = raw.len() - trimmed.len();
        let leading = if leading_ws_len > 0 {
            &raw[..leading_ws_len]
        } else {
            ""
        };

        // Build progress: `Compiling foo v1.2.3` / `Building […]`
        // / `Downloading foo v1`. Use muted color so the dozens of
        // these don't dominate the log visually.
        if let Some(pkg) = trimmed.strip_prefix("Compiling ") {
            let mut spans = vec![
                Span::raw(leading.to_owned()),
                Span::styled(
                    "Compiling ".to_string(),
                    Style::default().fg(t.accent).add_modifier(Modifier::BOLD),
                ),
                Span::styled(pkg.to_owned(), Style::default().fg(t.text_secondary)),
            ];
            // Trim line so spans length matches trimmed length
            let _ = &mut spans;
            lines.push(Line::from(spans));
            continue;
        }
        for prefix in &[
            "Checking ", "Building ", "Downloading ", "Updating ", "Verifying ",
            "Installing ", "Removing ", "Fresh ", "Documenting ",
        ] {
            if let Some(rest) = trimmed.strip_prefix(prefix) {
                lines.push(Line::from(vec![
                    Span::raw(leading.to_owned()),
                    Span::styled(
                        (*prefix).to_string(),
                        Style::default().fg(t.text_muted),
                    ),
                    Span::styled(rest.to_owned(), Style::default().fg(t.text_muted)),
                ]));
                continue;
            }
        }

        // `Finished` (build success) / `Compiled` etc. — bold green.
        if trimmed.starts_with("Finished ")
            || trimmed.starts_with("Compiled ")
            || trimmed.starts_with("Built ")
        {
            lines.push(Line::from(Span::styled(
                raw.to_owned(),
                Style::default().fg(t.success).add_modifier(Modifier::BOLD),
            )));
            continue;
        }

        // Errors: `error[E0123]: …` and `error: …`. Color the
        // prefix red+bold and let the rest of the line read in
        // primary text so the message is legible.
        if let Some(rest) = trimmed.strip_prefix("error") {
            // Match `error[…]:` or `error:` — anything else is text.
            let after = rest.trim_start_matches(|c: char| c == '[' || c == ']' || c.is_alphanumeric());
            if rest.is_empty() || rest.starts_with(':') || rest.starts_with('[') || after.starts_with(':') {
                lines.push(Line::from(vec![
                    Span::raw(leading.to_owned()),
                    Span::styled(
                        format!("error{}", rest.split(':').next().unwrap_or("")),
                        Style::default().fg(t.error).add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(
                        rest.split_once(':')
                            .map(|(_, after)| format!(":{after}"))
                            .unwrap_or_default(),
                        Style::default().fg(t.text_primary),
                    ),
                ]));
                continue;
            }
        }
        if let Some(rest) = trimmed.strip_prefix("warning") {
            let after = rest.trim_start_matches(|c: char| c == '[' || c == ']' || c.is_alphanumeric());
            if rest.is_empty() || rest.starts_with(':') || rest.starts_with('[') || after.starts_with(':') {
                lines.push(Line::from(vec![
                    Span::raw(leading.to_owned()),
                    Span::styled(
                        format!("warning{}", rest.split(':').next().unwrap_or("")),
                        Style::default().fg(t.warning).add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(
                        rest.split_once(':')
                            .map(|(_, after)| format!(":{after}"))
                            .unwrap_or_default(),
                        Style::default().fg(t.text_primary),
                    ),
                ]));
                continue;
            }
        }
        // Diagnostic detail: `note:`, `help:` — softer color.
        if trimmed.starts_with("note:") || trimmed.starts_with("help:") {
            lines.push(Line::from(Span::styled(
                raw.to_owned(),
                Style::default().fg(t.accent),
            )));
            continue;
        }
        // Location pointer: `   --> src/foo.rs:12:5`. Pick out the
        // arrow and color the path/lineno region.
        if let Some(idx) = raw.find("--> ") {
            let (before, after) = raw.split_at(idx + 4);
            lines.push(Line::from(vec![
                Span::styled(
                    before.to_owned(),
                    Style::default().fg(t.text_muted),
                ),
                Span::styled(
                    after.to_owned(),
                    Style::default().fg(t.accent),
                ),
            ]));
            continue;
        }

        // `cargo test` results.
        if trimmed.starts_with("running ")
            && trimmed.ends_with(" tests")
            && !trimmed.contains("0 tests")
        {
            lines.push(Line::from(Span::styled(
                raw.to_owned(),
                Style::default().fg(t.accent).add_modifier(Modifier::BOLD),
            )));
            continue;
        }
        if trimmed.starts_with("test ") {
            // `test foo::bar ... ok` / `... FAILED` / `... ignored`
            let style = if trimmed.contains(" ... ok") || trimmed.contains(" ... bench:") {
                Style::default().fg(t.success)
            } else if trimmed.contains(" ... FAILED") || trimmed.contains(" ... fail") {
                Style::default().fg(t.error).add_modifier(Modifier::BOLD)
            } else if trimmed.contains(" ... ignored") {
                Style::default().fg(t.text_muted)
            } else {
                Style::default().fg(t.text_secondary)
            };
            lines.push(Line::from(Span::styled(raw.to_owned(), style)));
            continue;
        }
        if let Some(rest) = trimmed.strip_prefix("test result:") {
            // `test result: ok. N passed; M failed; …`
            let body_color = if rest.contains(" FAILED") || rest.contains("failed; 0") {
                if rest.contains("0 failed") {
                    t.success
                } else {
                    t.error
                }
            } else if rest.contains(" ok") {
                t.success
            } else {
                t.warning
            };
            lines.push(Line::from(vec![
                Span::raw(leading.to_owned()),
                Span::styled(
                    "test result:".to_owned(),
                    Style::default().fg(t.accent).add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    rest.to_owned(),
                    Style::default().fg(body_color).add_modifier(Modifier::BOLD),
                ),
            ]));
            continue;
        }
        if trimmed.starts_with("failures:") {
            lines.push(Line::from(Span::styled(
                raw.to_owned(),
                Style::default().fg(t.error).add_modifier(Modifier::BOLD),
            )));
            continue;
        }

        // Carat / pipe gutters from the rust diagnostic format —
        // they hint at code so let them inherit accent.
        if trimmed.starts_with('|') || trimmed.starts_with('=') || trimmed.starts_with("^") {
            lines.push(Line::from(Span::styled(
                raw.to_owned(),
                Style::default().fg(t.accent),
            )));
            continue;
        }

        lines.push(Line::from(Span::styled(
            raw.to_owned(),
            Style::default().fg(t.text_secondary),
        )));
    }
    if total > max_lines {
        lines.push(Line::from(Span::styled(
            format!(
                "… {} more lines · click or press o to expand",
                total - max_lines
            ),
            Style::default()
                .fg(t.text_muted)
                .add_modifier(Modifier::ITALIC),
        )));
    }
    Paragraph::new(lines)
        .style(Style::default().bg(t.bg))
        .scroll((skip as u16, 0))
        .render(area, buf);
}

/// Render Bash output where stdout is the contents of a single
/// file (cat / head / tail). Top row is the exit-code badge, then
/// stdout flows through syntect highlighting (no line numbers — the
/// `cat` user opted out of those), then any stderr in red.
#[allow(clippy::too_many_arguments)]
fn render_cat_output_skip(
    lang: &str,
    stdout: &str,
    stderr: &str,
    exit_code: Option<i32>,
    area: Rect,
    t: Theme,
    buf: &mut Buffer,
    skip: usize,
    expanded: bool,
) {
    let mut lines: Vec<Line<'static>> = Vec::new();
    let (code_str, code_style) = match exit_code {
        Some(0) => ("exit 0".to_owned(), Style::default().fg(t.success)),
        Some(n) => (format!("exit {n}"), Style::default().fg(t.error)),
        None => ("running…".to_owned(), Style::default().fg(t.text_muted)),
    };
    lines.push(Line::from(Span::styled(code_str, code_style)));

    let max_lines = if expanded { 500usize } else { 80usize };
    let inner_w = area.width as usize;
    let mut highlighted = markdown::highlight_code_raw(lang, stdout, inner_w, &t);
    let total = highlighted.len();
    let truncated = total > max_lines;
    if truncated {
        highlighted.truncate(max_lines);
    }
    lines.extend(highlighted);
    if truncated {
        lines.push(Line::from(Span::styled(
            format!(
                "… {} more lines · click or press o to expand",
                total - max_lines
            ),
            Style::default()
                .fg(t.text_muted)
                .add_modifier(Modifier::ITALIC),
        )));
    }

    if !stderr.is_empty() {
        lines.push(Line::from(Span::styled(
            "↳ stderr",
            Style::default()
                .fg(t.error)
                .add_modifier(Modifier::ITALIC | Modifier::BOLD),
        )));
        for line in stderr.lines().take(40) {
            lines.push(Line::from(Span::styled(
                line.to_owned(),
                Style::default().fg(t.error),
            )));
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
    expanded: bool,
) {
    use ansi_to_tui::IntoText;

    let mut lines: Vec<Line<'static>> = Vec::new();
    let w = area.width as usize;

    let (code_str, code_style) = match exit_code {
        Some(0) => ("exit 0".to_owned(), Style::default().fg(t.success)),
        Some(n) => (format!("exit {n}"), Style::default().fg(t.error)),
        None => ("running…".to_owned(), Style::default().fg(t.text_muted)),
    };
    lines.push(Line::from(Span::styled(code_str, code_style)));

    let max_lines = if expanded { 500usize } else { 80usize };
    let mut count = 0usize;

    // Route through `ansi-to-tui` so SGR codes from cargo / git diff /
    // test runners survive as ratatui `Style`s. Falls back to the
    // sanitize-and-stripe path when the parser rejects (rare — only
    // truly malformed escape sequences). Each parsed Line is then
    // wrapped to fit the column width while preserving its spans.
    let push_styled =
        |raw: &str, fallback_style: Style, lines: &mut Vec<Line<'static>>, count: &mut usize| {
            if *count >= max_lines {
                return;
            }
            let parsed = raw.into_text().ok();
            let source_lines: Vec<Line<'static>> = match parsed {
                Some(text) => text.lines.into_iter().collect(),
                None => raw
                    .lines()
                    .map(|l| Line::from(Span::styled(sanitize_terminal_text(l), fallback_style)))
                    .collect(),
            };
            for line in source_lines {
                if *count >= max_lines {
                    return;
                }
                for wrapped in wrap_styled_line(&line, w.max(1)) {
                    lines.push(wrapped);
                    *count += 1;
                    if *count >= max_lines {
                        return;
                    }
                }
            }
        };

    push_styled(
        stdout,
        Style::default().fg(t.text_secondary),
        &mut lines,
        &mut count,
    );
    // Make the stdout→stderr boundary visible. Without this the user
    // sees red lines mixed in with grey and can't tell where the
    // failure stream begins. The divider only appears when both
    // streams have content.
    if !stdout.is_empty() && !stderr.is_empty() && count < max_lines {
        lines.push(Line::from(Span::styled(
            "↳ stderr",
            Style::default()
                .fg(t.error)
                .add_modifier(Modifier::ITALIC | Modifier::BOLD),
        )));
        count += 1;
    }
    push_styled(stderr, Style::default().fg(t.error), &mut lines, &mut count);

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

fn render_diff_skip(
    diff: &DiffView,
    area: Rect,
    t: Theme,
    buf: &mut Buffer,
    skip: usize,
    expanded: bool,
) {
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

        let hunk_cap = if expanded { 500 } else { 50 };
        let max_dl = hunk.lines.len().min(hunk_cap);

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
                            let extra_mod =
                                matches!(dl.kind, DiffLineKind::Removed).then_some(Modifier::DIM);
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

        if hunk.lines.len() > hunk_cap {
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
                        format!("… {} more lines", hunk.lines.len() - hunk_cap),
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
        // Reasoning ribbon: each thinking line gets a `┃` prefix in
        // `t.reasoning_fg` so the block visually nests inside the
        // assistant message. The ribbon's own color is the same as
        // the reasoning text, so the indent reads as a soft "this is
        // a thought" guide rather than a competing structural
        // element. Mirrors how Discord / Slack indent quoted blocks.
        for l in text.lines() {
            items.push(RenderItem::TextLine(Line::from(vec![
                Span::styled("┃ ", Style::default().fg(t.reasoning_fg)),
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
fn push_advisor_lines<'a>(items: &mut Vec<RenderItem<'a>>, text: &'a str, t: &Theme) {
    // Header row: bold, accent-colored "ADVISOR:" so it pops against the
    // muted body. Without the bold, the prefix blended into the body and
    // the user couldn't tell where the main reply ended and the advisor
    // started.
    items.push(RenderItem::TextLine(Line::from(vec![
        Span::styled("▎ ", Style::default().fg(t.accent)),
        Span::styled(
            "ADVISOR:",
            Style::default()
                .fg(t.accent)
                .add_modifier(Modifier::BOLD),
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

fn push_task_status_lines<'a>(items: &mut Vec<RenderItem<'a>>, ts: &'a TaskStatusPart, t: &Theme) {
    let (icon, style) = match ts.status {
        TaskLifecycle::Pending => ("◌", Style::default().fg(t.text_muted)),
        TaskLifecycle::Running => ("◎", Style::default().fg(t.text_primary)),
        TaskLifecycle::Idle => ("⏸", Style::default().fg(t.text_muted)),
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
        Rect {
            x,
            y,
            width: w,
            height: h,
        }
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

#[cfg(test)]
mod bash_output_tests {
    use super::*;

    // Normal: cat <file.md> classifies as Other (cat falls through
    // to the markdown / lang sniff path, not the structured tool
    // dispatch).
    #[test]
    fn classify_cat_is_other_normal() {
        assert!(matches!(
            classify_bash_cmd("cat README.md"),
            BashCmdKind::Other
        ));
    }

    // Normal: grep_target_file pulls the file argument out so the
    // renderer can show a heading. Pattern is *not* the target.
    #[test]
    fn grep_target_file_extracts_path_normal() {
        assert_eq!(
            grep_target_file("grep -n \"sws_headers(\" ~/foo/auth.rs"),
            Some("~/foo/auth.rs".into())
        );
        assert_eq!(
            grep_target_file("rg \"open(\" --type rust src/"),
            Some("src/".into())
        );
        assert_eq!(
            grep_target_file("grep -e PAT -B 2 -A 2 file.rs"),
            Some("file.rs".into())
        );
        // Quoted target gets unquoted.
        assert_eq!(
            grep_target_file("grep PAT 'file with spaces.rs'"),
            Some("file with spaces.rs".into())
        );
    }

    // Robust: grep_target_file is None when there's no positional
    // file (recursive grep over cwd, or pattern-only invocation).
    #[test]
    fn grep_target_file_none_when_no_target_robust() {
        // `rg PAT` with no target = search cwd recursively → None
        assert_eq!(grep_target_file("rg \"foo\""), None);
        assert_eq!(grep_target_file("grep PAT"), None);
        // Wrong verb returns None.
        assert_eq!(grep_target_file("cat file.rs"), None);
    }

    // Normal: the user's actual reported case — `grep -n "pattern("
    // file` — must classify as Grep so render_grep_output_skip
    // fires. The trailing `(` inside the double-quoted pattern was
    // suspected of confusing the classifier; this test pins the
    // expected behaviour so a future redact_quoted regression gets
    // caught.
    #[test]
    fn classify_grep_with_paren_inside_quotes_normal() {
        for cmd in &[
            "grep -n \"sws_headers(\" ~/foo/auth.rs",
            "grep -rn \"foo(\" src/",
            "rg \"open(\" --type rust",
            "grep \"async fn (\" file.rs",
        ] {
            assert!(
                matches!(classify_bash_cmd(cmd), BashCmdKind::Grep),
                "{cmd} should classify as Grep"
            );
        }
    }

    // Normal: `sed -n '1,$p' file.md` — the canonical "print all
    // lines" idiom — must NOT be rejected for the `$` inside its
    // quoted script. Before redact_quoted() this fell through to
    // plain rendering and lost markdown formatting.
    #[test]
    fn infer_lang_handles_sed_with_dollar_in_quotes_normal() {
        assert_eq!(
            infer_lang_from_bash("sed -n '1,$p' README.md").as_deref(),
            Some("md")
        );
        assert_eq!(
            infer_lang_from_bash("awk '{print $1}' main.rs").as_deref(),
            Some("rs")
        );
    }

    // Robust: an *unquoted* `$` (real command substitution) must
    // still be rejected — we only ignore `$` that lives inside a
    // matched quote.
    #[test]
    fn infer_lang_rejects_unquoted_dollar_robust() {
        assert!(infer_lang_from_bash("cat $(which README.md)").is_none());
        assert!(infer_lang_from_bash("cat $FILE").is_none());
    }

    // Normal: redact_quoted preserves length and quote chars but
    // blanks out the contents.
    #[test]
    fn redact_quoted_blanks_inside_quotes_normal() {
        assert_eq!(redact_quoted("sed -n '1,$p' file"), "sed -n '    ' file");
        assert_eq!(redact_quoted("echo \"$x\""), "echo \"  \"");
        assert_eq!(redact_quoted("plain text no quotes"), "plain text no quotes");
    }

    // Normal: hex-dump tools route to HexDump.
    #[test]
    fn classify_hex_dump_tools_normal() {
        for cmd in &["xxd file.bin", "hexyl file.bin", "od -c file.bin"] {
            assert!(
                matches!(classify_bash_cmd(cmd), BashCmdKind::HexDump),
                "{cmd}"
            );
        }
    }

    // Normal: docker / podman list-style subcommands route to TabularList.
    #[test]
    fn classify_docker_tabular_normal() {
        for cmd in &[
            "docker ps",
            "docker ps -a",
            "docker images",
            "podman ps",
            "docker container ls",
            "docker network ls",
            "docker volume ls",
        ] {
            assert!(
                matches!(classify_bash_cmd(cmd), BashCmdKind::TabularList),
                "{cmd}"
            );
        }
    }

    // Robust: unknown docker subcommand falls through to Other (so
    // we don't try to color e.g. `docker run` interactive output).
    #[test]
    fn classify_docker_unknown_subcmd_other_robust() {
        assert!(matches!(classify_bash_cmd("docker run x"), BashCmdKind::Other));
        assert!(matches!(classify_bash_cmd("docker"), BashCmdKind::Other));
    }

    // Normal: kubectl get / describe / top all route to TabularList.
    #[test]
    fn classify_kubectl_tabular_normal() {
        for cmd in &[
            "kubectl get pods",
            "kubectl get nodes -o wide",
            "kubectl describe pod x",
            "kubectl top pod",
            "oc get routes",
        ] {
            assert!(
                matches!(classify_bash_cmd(cmd), BashCmdKind::TabularList),
                "{cmd}"
            );
        }
    }

    // Normal: raw `diff -u a b` shares the GitDiff renderer so its
    // +/-/@@ lines get colored too — fixes the gap where someone
    // runs `diff -u` outside a git tree.
    #[test]
    fn classify_raw_diff_routes_to_gitdiff_normal() {
        assert!(matches!(
            classify_bash_cmd("diff -u a.txt b.txt"),
            BashCmdKind::GitDiff
        ));
    }

    // Normal: grep / rg / ack / ag all dispatch to the Grep renderer.
    #[test]
    fn classify_grep_family_normal() {
        for cmd in &[
            "grep -rn x src/",
            "rg \"TODO\" --type rust",
            "ack pat",
            "ag pat",
        ] {
            assert!(matches!(classify_bash_cmd(cmd), BashCmdKind::Grep), "{cmd}");
        }
    }

    // Normal: find / ls / tree / fd all dispatch to PathList.
    #[test]
    fn classify_path_list_family_normal() {
        for cmd in &["find . -name '*.rs'", "ls -la", "tree", "fd rust"] {
            assert!(
                matches!(classify_bash_cmd(cmd), BashCmdKind::PathList),
                "{cmd}"
            );
        }
    }

    // Normal: git diff / git show / git log dispatch correctly.
    #[test]
    fn classify_git_subcommands_normal() {
        assert!(matches!(
            classify_bash_cmd("git diff HEAD"),
            BashCmdKind::GitDiff
        ));
        assert!(matches!(
            classify_bash_cmd("git show abc123"),
            BashCmdKind::GitDiff
        ));
        assert!(matches!(
            classify_bash_cmd("git log --oneline -20"),
            BashCmdKind::GitLog
        ));
        assert!(matches!(
            classify_bash_cmd("git status"),
            BashCmdKind::Other
        ));
    }

    // Robust: pipeline-aware classification — first segment of `||` or `|`
    // wins. The cat-with-fallback pattern is common.
    #[test]
    fn classify_pipeline_takes_first_segment_robust() {
        assert!(matches!(
            classify_bash_cmd("rg foo 2>/dev/null || rg bar"),
            BashCmdKind::Grep
        ));
        assert!(matches!(
            classify_bash_cmd("git diff | less"),
            BashCmdKind::GitDiff
        ));
    }

    // Robust: `2>/dev/null` and `>file` redirects don't break the verb sniff.
    #[test]
    fn classify_strips_redirects_robust() {
        assert!(matches!(
            classify_bash_cmd("grep -rn pat src/ 2>/dev/null"),
            BashCmdKind::Grep
        ));
        assert!(matches!(
            classify_bash_cmd("find . -name '*.rs' >list.txt"),
            BashCmdKind::PathList
        ));
    }

    // Robust: command substitution / backticks / & / ; reject (those
    // change semantics in ways the simple sniff can't reason about).
    #[test]
    fn classify_rejects_complex_shell_robust() {
        assert!(matches!(
            classify_bash_cmd("echo $(grep x y)"),
            BashCmdKind::Other
        ));
        assert!(matches!(
            classify_bash_cmd("grep x y; echo done"),
            BashCmdKind::Other
        ));
        assert!(matches!(
            classify_bash_cmd("grep x y &"),
            BashCmdKind::Other
        ));
    }

    // Normal: parse a standard `path:line:body` grep result.
    #[test]
    fn parse_grep_path_line_body_normal() {
        let line = "src/main.rs:42:fn main() {";
        match parse_grep_line(line) {
            Some(GrepLine::Match {
                path,
                lineno,
                col,
                body,
                is_context,
            }) => {
                assert_eq!(path, "src/main.rs");
                assert_eq!(lineno, Some("42"));
                assert_eq!(col, None);
                assert_eq!(body, "fn main() {");
                assert!(!is_context);
            }
            other => panic!("expected match, got {other:?}", other = other.is_some()),
        }
    }

    // Normal: rg with --column emits `path:line:col:body`.
    #[test]
    fn parse_grep_with_column_normal() {
        let line = "src/foo.rs:15:5:    let x = 1;";
        match parse_grep_line(line) {
            Some(GrepLine::Match {
                path,
                lineno,
                col,
                body,
                is_context,
            }) => {
                assert_eq!(path, "src/foo.rs");
                assert_eq!(lineno, Some("15"));
                assert_eq!(col, Some("5"));
                assert_eq!(body, "    let x = 1;");
                assert!(!is_context);
            }
            other => panic!("expected match, got {other:?}", other = other.is_some()),
        }
    }

    // Normal: grep -B/-C context lines use `-` separators.
    #[test]
    fn parse_grep_context_lines_use_dash_normal() {
        let line = "src/foo.rs-41-/// docstring";
        match parse_grep_line(line) {
            Some(GrepLine::Match {
                path,
                lineno,
                body,
                is_context,
                ..
            }) => {
                assert_eq!(path, "src/foo.rs");
                assert_eq!(lineno, Some("41"));
                assert_eq!(body, "/// docstring");
                assert!(is_context);
            }
            other => panic!(
                "expected context match, got {other:?}",
                other = other.is_some()
            ),
        }
    }

    // Robust: a path containing `:` (Windows-style) shouldn't false-match.
    // The parser anchors on `:digits:` so a colon in the path doesn't break it.
    #[test]
    fn parse_grep_handles_path_with_colon_robust() {
        let line = "C:/code/main.rs:99:hello";
        match parse_grep_line(line) {
            Some(GrepLine::Match {
                path, lineno, body, ..
            }) => {
                assert_eq!(path, "C:/code/main.rs");
                assert_eq!(lineno, Some("99"));
                assert_eq!(body, "hello");
            }
            _ => panic!("expected match"),
        }
    }

    // Robust: rg --heading mode emits a bare path on its own line
    // (no separators). Recognized as HeadingPath.
    #[test]
    fn parse_grep_heading_path_robust() {
        let line = "src/utils/foo.rs";
        match parse_grep_line(line) {
            Some(GrepLine::HeadingPath(p)) => assert_eq!(p, "src/utils/foo.rs"),
            _ => panic!("expected heading path"),
        }
    }

    // Normal: markdown content sniff fires on a doc with headers + table.
    #[test]
    fn looks_like_markdown_detects_real_md_normal() {
        let content = "# Title\n\nSome text\n\n## Section\n\n| a | b |\n|---|---|\n| 1 | 2 |\n";
        assert!(looks_like_markdown(content));
    }

    // Robust: plain code shouldn't sniff as markdown even if it has `#` chars.
    #[test]
    fn looks_like_markdown_rejects_python_robust() {
        let content = "# This is a Python comment\nprint('hello')\nx = 1\ny = 2\n";
        assert!(!looks_like_markdown(content));
    }
}

#[cfg(test)]
mod bash_chain_tests {
    use super::*;

    // Normal: `cd X && grep ...` should classify as Grep — the LAST
    // segment of an `&&` chain is the meaningful command, not the cd.
    #[test]
    fn classify_cd_and_then_grep_normal() {
        assert!(matches!(
            classify_bash_cmd("cd ~/src && grep -rn TODO"),
            BashCmdKind::Grep
        ));
    }

    // Normal: `cd X && cat README.md 2>/dev/null || cat docs/README.md`
    // — the whole chain compiles down to `cat <markdown>` so the lang
    // sniff should pick up `.md`.
    #[test]
    fn infer_lang_through_cd_and_chain_normal() {
        let lang =
            infer_lang_from_bash("cd ~/proj && cat README.md 2>/dev/null || cat docs/README.md");
        assert_eq!(lang.as_deref(), Some("md"));
    }

    // Robust: `cd X && cat foo &` (background) still rejected.
    #[test]
    fn classify_rejects_lone_background_robust() {
        assert!(matches!(
            classify_bash_cmd("cd ~/src && cat foo &"),
            BashCmdKind::Other
        ));
    }

    // Normal: `grep -n pat single-file.txt` emits `<lineno>:<body>`
    // with no path prefix. Parser handles it.
    #[test]
    fn parse_grep_no_path_single_file_normal() {
        let line = "187214:    var _X = \"ScheduleWakeup\";";
        match parse_grep_line(line) {
            Some(GrepLine::Match {
                path, lineno, body, ..
            }) => {
                assert_eq!(path, "");
                assert_eq!(lineno, Some("187214"));
                assert_eq!(body, "    var _X = \"ScheduleWakeup\";");
            }
            _ => panic!("expected match"),
        }
    }

    // Robust: a line starting with a number that isn't grep-style
    // (no `:` after digits) shouldn't false-match.
    #[test]
    fn parse_grep_no_path_rejects_bare_numbers_robust() {
        let line = "1234567 records processed";
        // `1234567 ` is digits + space, no `:` or `-` after digits,
        // so the no-path parser returns None and the line falls
        // through to plain text.
        assert!(parse_grep_line(line).is_none());
    }

    // Robust: hex/long IDs that look like digits but aren't reasonable
    // line numbers are rejected. E.g. a SHA prefix.
    #[test]
    fn parse_grep_no_path_rejects_huge_lineno_robust() {
        // 99999999999 (11 digits) — won't fit in u32, parser rejects.
        let line = "99999999999:body";
        assert!(parse_grep_line(line).is_none());
    }
}

#[cfg(test)]
mod path_color_tests {
    use super::*;
    use crate::theme::Theme;

    fn t() -> Theme {
        Theme::dark()
    }

    // Normal: code extensions get the accent color so paths in grep
    // results stand out as code files.
    #[test]
    fn path_color_code_extensions_normal() {
        let theme = t();
        for path in &["main.rs", "src/foo.go", "scripts/run.py", "app.ts"] {
            assert_eq!(
                path_color(path, theme),
                theme.accent,
                "{path} should be accent"
            );
        }
    }

    // Normal: config / data files get text_secondary so they
    // visually demote below code.
    #[test]
    fn path_color_config_extensions_normal() {
        let theme = t();
        assert_eq!(path_color("Cargo.toml", theme), theme.text_secondary);
        assert_eq!(path_color("package.json", theme), theme.text_secondary);
        assert_eq!(path_color("config.yaml", theme), theme.text_secondary);
        assert_eq!(path_color(".env", theme), theme.text_muted); // no ext
    }

    // Normal: docs (md, txt, rst) get text_primary (white) so they
    // stand out as readable content.
    #[test]
    fn path_color_doc_extensions_normal() {
        let theme = t();
        assert_eq!(path_color("README.md", theme), theme.text_primary);
        assert_eq!(path_color("notes.txt", theme), theme.text_primary);
    }

    // Robust: unknown extension falls back to text_muted (least
    // attention-grabbing).
    #[test]
    fn path_color_unknown_falls_back_robust() {
        let theme = t();
        assert_eq!(path_color("file.xyz", theme), theme.text_muted);
        assert_eq!(path_color("noext", theme), theme.text_muted);
        assert_eq!(path_color("", theme), theme.text_muted);
    }

    // Robust: extension matching is case-insensitive — a path like
    // `MAIN.RS` (some Windows tools emit uppercase) still resolves
    // to the Rust accent color.
    #[test]
    fn path_color_case_insensitive_robust() {
        let theme = t();
        assert_eq!(path_color("Main.RS", theme), theme.accent);
        assert_eq!(path_color("CONFIG.TOML", theme), theme.text_secondary);
    }
}

// =====================================================================

#[cfg(test)]
mod helper_tests {
    use super::*;

    fn dummy_tool(input: ToolInput, output: ToolOutput, kind: ToolKind) -> ToolCall {
        ToolCall {
            id: "t-1".to_string(),
            kind,
            status: ToolStatus::Complete,
            input,
            output,
            is_collapsed: false,
            expanded: false,
            elapsed_ms: None,
            started_at: None,
            pinned: false,
        }
    }

    // --- infer_lang_from_tool ----------------------------------------

    #[test]
    fn infer_lang_from_read_uses_path_extension_normal() {
        let t = dummy_tool(
            ToolInput::Read {
                file_path: "src/main.rs".into(),
                offset: None,
                limit: None,
            },
            ToolOutput::Empty,
            ToolKind::Read,
        );
        assert_eq!(infer_lang_from_tool(&t).as_deref(), Some("rs"));
    }

    #[test]
    fn infer_lang_from_edit_uses_path_extension_normal() {
        let t = dummy_tool(
            ToolInput::Edit {
                file_path: "src/lib.py".into(),
                old_string: "".into(),
                new_string: "".into(),
                replacement: ReplacementMode::FirstOnly,
            },
            ToolOutput::Empty,
            ToolKind::Edit,
        );
        assert_eq!(infer_lang_from_tool(&t).as_deref(), Some("py"));
    }

    #[test]
    fn infer_lang_from_write_uses_path_extension_normal() {
        let t = dummy_tool(
            ToolInput::Write {
                file_path: "config.toml".into(),
                content: "".into(),
            },
            ToolOutput::Empty,
            ToolKind::Write,
        );
        assert_eq!(infer_lang_from_tool(&t).as_deref(), Some("toml"));
    }

    #[test]
    fn infer_lang_from_bash_input_delegates_robust() {
        // Bash-tool path delegates to infer_lang_from_bash, which sniffs
        // `cat path/file.ext`.
        let t = dummy_tool(
            ToolInput::Bash {
                command: "cat README.md".into(),
                timeout: None,
                workdir: None,
            },
            ToolOutput::Empty,
            ToolKind::Bash,
        );
        assert_eq!(infer_lang_from_tool(&t).as_deref(), Some("md"));
    }

    #[test]
    fn infer_lang_from_unknown_kind_returns_none_robust() {
        let t = dummy_tool(
            ToolInput::TeamDelete,
            ToolOutput::Empty,
            ToolKind::TeamDelete,
        );
        assert_eq!(infer_lang_from_tool(&t), None);
    }

    // --- lang_from_path ----------------------------------------------

    #[test]
    fn lang_from_path_extension_wins_normal() {
        assert_eq!(lang_from_path("src/main.rs").as_deref(), Some("rs"));
        assert_eq!(lang_from_path("foo.JS").as_deref(), Some("JS"));
    }

    #[test]
    fn lang_from_path_no_extension_falls_back_to_filename_robust() {
        // No extension → use the filename (e.g. `Makefile` → `makefile`
        // when downstream lowercases it, but lang_from_path returns the
        // raw filename).
        assert_eq!(lang_from_path("Makefile").as_deref(), Some("Makefile"));
    }

    #[test]
    fn lang_from_path_empty_returns_none_robust() {
        assert_eq!(lang_from_path(""), None);
    }

    // --- infer_lang_from_bash ----------------------------------------

    #[test]
    fn infer_lang_from_bash_cat_normal() {
        assert_eq!(
            infer_lang_from_bash("cat src/main.rs").as_deref(),
            Some("rs")
        );
    }

    #[test]
    fn infer_lang_from_bash_head_with_flags_normal() {
        // Skips `-50` (numeric arg) and picks `file.py`.
        assert_eq!(
            infer_lang_from_bash("head -50 file.py").as_deref(),
            Some("py")
        );
    }

    #[test]
    fn infer_lang_from_bash_pipeline_takes_first_robust() {
        // `cat foo.rs | less` → primary segment is `cat foo.rs`.
        assert_eq!(
            infer_lang_from_bash("cat foo.rs | less").as_deref(),
            Some("rs")
        );
    }

    #[test]
    fn infer_lang_from_bash_command_substitution_rejected_robust() {
        // `$(...)` patterns disqualify — not safe to sniff.
        assert_eq!(infer_lang_from_bash("cat $(echo foo.rs)"), None);
    }

    #[test]
    fn infer_lang_from_bash_non_cat_verb_rejected_robust() {
        // Only `cat`/`head`/`tail`/`bat`/`less`/`more` qualify.
        assert_eq!(infer_lang_from_bash("echo hello.rs"), None);
    }

    // --- path_color --------------------------------------------------

    #[test]
    fn path_color_code_extension_uses_accent_normal() {
        let t = Theme::dark();
        assert_eq!(path_color("src/main.rs", t), t.accent);
        assert_eq!(path_color("app.py", t), t.accent);
        assert_eq!(path_color("foo.go", t), t.accent);
    }

    #[test]
    fn path_color_config_uses_text_secondary_normal() {
        let t = Theme::dark();
        assert_eq!(path_color("Cargo.toml", t), t.text_secondary);
        assert_eq!(path_color("settings.json", t), t.text_secondary);
    }

    #[test]
    fn path_color_docs_use_text_primary_normal() {
        let t = Theme::dark();
        assert_eq!(path_color("README.md", t), t.text_primary);
    }

    #[test]
    fn path_color_shell_uses_warning_robust() {
        let t = Theme::dark();
        assert_eq!(path_color("install.sh", t), t.warning);
    }

    #[test]
    fn path_color_unknown_falls_back_to_muted_robust() {
        let t = Theme::dark();
        assert_eq!(path_color("data.bin", t), t.text_muted);
        // No extension at all also goes to muted.
        assert_eq!(path_color("Makefile", t), t.text_muted);
    }

    #[test]
    fn path_color_uppercase_extension_normalized_robust() {
        // ASCII-lowercased, so .RS / .Rs all hit the code branch.
        let t = Theme::dark();
        assert_eq!(path_color("FOO.RS", t), t.accent);
    }

    // --- looks_like_markdown -----------------------------------------

    #[test]
    fn looks_like_markdown_combines_signals_normal() {
        // Headers + table + bold marker → score >= 4.
        let s = "# Title\n\nSome **bold** text\n\n## Section\n";
        // 2 headers (each +2) + bold (+1) = 5 → markdown.
        assert!(looks_like_markdown(s));
    }

    #[test]
    fn looks_like_markdown_pure_code_not_md_robust() {
        // Python code with `#` comments doesn't trigger header signals.
        let s = "# this is a comment\nprint('x')\nx = 1\ny = 2\n";
        assert!(!looks_like_markdown(s));
    }

    #[test]
    fn looks_like_markdown_first_2kb_only_robust() {
        // Strong markdown signal in the prefix → triggers; rest can be huge.
        let prefix = "# h1\n## h2\n### h3\n```rust\nlet x = 1;\n```\n";
        let mut s = String::from(prefix);
        s.push_str(&"x".repeat(10_000));
        assert!(looks_like_markdown(&s));
    }

    #[test]
    fn looks_like_markdown_empty_returns_false_robust() {
        assert!(!looks_like_markdown(""));
    }

    // --- wrapped_line_count ------------------------------------------

    #[test]
    fn wrapped_line_count_short_one_line_normal() {
        assert_eq!(wrapped_line_count("hello", 80), 1);
    }

    #[test]
    fn wrapped_line_count_multi_line_normal() {
        assert_eq!(wrapped_line_count("a\nb\nc", 80), 3);
    }

    #[test]
    fn wrapped_line_count_wraps_normal() {
        // 12 chars at width 5 = ceil(12/5) = 3.
        assert_eq!(wrapped_line_count("abcdefghijkl", 5), 3);
    }

    #[test]
    fn wrapped_line_count_zero_width_robust() {
        // Zero width → fall back to text.lines().count().max(1).
        assert_eq!(wrapped_line_count("a\nb", 0), 2);
        assert_eq!(wrapped_line_count("", 0), 1);
    }

    #[test]
    fn wrapped_line_count_empty_text_returns_zero_robust() {
        // Empty text contributes 0 (the fold's `.max(...)` only kicks
        // in if text is non-empty).
        assert_eq!(wrapped_line_count("", 80), 0);
    }

    #[test]
    fn wrapped_line_count_blank_line_counts_as_one_robust() {
        // A truly blank logical line still rendered as one row.
        assert_eq!(wrapped_line_count("\n", 80), 1);
    }

    // --- tool_content_height_with ------------------------------------

    #[test]
    fn tool_content_height_empty_zero_normal() {
        assert_eq!(tool_content_height_with(&ToolOutput::Empty, 80, false), 0);
    }

    #[test]
    fn tool_content_height_text_simple_normal() {
        // A 3-line text: height = 3.
        let out = ToolOutput::Text("a\nb\nc".to_string());
        assert_eq!(tool_content_height_with(&out, 80, false), 3);
    }

    #[test]
    fn tool_content_height_text_truncates_with_footer_robust() {
        // > 80 lines → cap at 80 + 1 footer row.
        let body: String = (0..150).map(|n| format!("line{n}\n")).collect();
        let out = ToolOutput::Text(body);
        let h = tool_content_height_with(&out, 80, false);
        assert_eq!(h, 81, "expect 80 cap + 1 footer");
    }

    #[test]
    fn tool_content_height_text_expanded_lifts_cap_robust() {
        // expanded=true → cap rises to 500.
        let body: String = (0..150).map(|n| format!("line{n}\n")).collect();
        let out = ToolOutput::Text(body);
        let h = tool_content_height_with(&out, 80, true);
        assert_eq!(h, 150, "no truncation under expanded cap");
    }

    #[test]
    fn tool_content_height_command_includes_exit_row_normal() {
        let out = ToolOutput::Command {
            stdout: "ok\n".to_string(),
            stderr: String::new(),
            exit_code: Some(0),
        };
        // 1 (exit) + 1 (stdout) = 2.
        assert_eq!(tool_content_height_with(&out, 80, false), 2);
    }

    #[test]
    fn tool_content_height_command_with_stderr_divider_robust() {
        // Both streams present → +1 divider row between them.
        let out = ToolOutput::Command {
            stdout: "out".to_string(),
            stderr: "err".to_string(),
            exit_code: Some(1),
        };
        // exit (1) + stdout (1) + divider (1) + stderr (1) = 4.
        assert_eq!(tool_content_height_with(&out, 80, false), 4);
    }

    #[test]
    fn tool_content_height_filelist_caps_normal() {
        let files: Vec<String> = (0..5).map(|n| format!("f{n}")).collect();
        let out = ToolOutput::FileList(files);
        assert_eq!(tool_content_height_with(&out, 80, false), 5);
    }

    #[test]
    fn tool_content_height_filelist_truncates_with_footer_robust() {
        // 25 files, cap=20 → 20 rows + 1 footer.
        let files: Vec<String> = (0..25).map(|n| format!("f{n}")).collect();
        let out = ToolOutput::FileList(files);
        assert_eq!(tool_content_height_with(&out, 80, false), 21);
    }

    #[test]
    fn tool_content_height_largetext_huge_collapses_to_one_robust() {
        // Force `huge` by making line_count exceed COLLAPSE_LINES.
        let lt = LargeText {
            content: "x".to_string(),
            line_count: LargeText::COLLAPSE_LINES + 10,
            byte_count: 1,
        };
        let out = ToolOutput::LargeText(lt);
        assert_eq!(tool_content_height_with(&out, 80, false), 1);
    }

    // --- tool_block_height -------------------------------------------

    #[test]
    fn tool_block_height_collapsed_is_one_normal() {
        let mut t = dummy_tool(
            ToolInput::Bash {
                command: "ls".into(),
                timeout: None,
                workdir: None,
            },
            ToolOutput::Text("foo\nbar\nbaz".into()),
            ToolKind::Bash,
        );
        t.is_collapsed = true;
        assert_eq!(tool_block_height(&t, 80), 1);
        // Public wrapper should match.
        assert_eq!(tool_block_height_pub(&t, 80), 1);
    }

    #[test]
    fn tool_block_height_includes_title_normal() {
        // Empty output + 1-line bash → 1 (title) + 0 (cont) + 0 (body) = 1.
        let t = dummy_tool(
            ToolInput::Bash {
                command: "ls".into(),
                timeout: None,
                workdir: None,
            },
            ToolOutput::Empty,
            ToolKind::Bash,
        );
        assert_eq!(tool_block_height(&t, 80), 1);
    }

    #[test]
    fn tool_block_height_counts_continuation_lines_robust() {
        // Multi-line bash → 1 (title) + N continuation rows.
        let t = dummy_tool(
            ToolInput::Bash {
                command: "cat <<EOF\nfoo\nbar\nEOF".into(),
                timeout: None,
                workdir: None,
            },
            ToolOutput::Empty,
            ToolKind::Bash,
        );
        // 1 title + 3 cont rows + 0 body = 4.
        assert_eq!(tool_block_height(&t, 80), 4);
    }

    // --- bash_continuation_lines -------------------------------------

    #[test]
    fn bash_continuation_lines_empty_for_single_line_normal() {
        let t = dummy_tool(
            ToolInput::Bash {
                command: "echo hi".into(),
                timeout: None,
                workdir: None,
            },
            ToolOutput::Empty,
            ToolKind::Bash,
        );
        assert!(bash_continuation_lines(&t).is_empty());
    }

    #[test]
    fn bash_continuation_lines_drops_first_line_normal() {
        let t = dummy_tool(
            ToolInput::Bash {
                command: "first\nsecond\nthird".into(),
                timeout: None,
                workdir: None,
            },
            ToolOutput::Empty,
            ToolKind::Bash,
        );
        assert_eq!(
            bash_continuation_lines(&t),
            vec!["second".to_string(), "third".to_string()]
        );
    }

    #[test]
    fn bash_continuation_lines_non_bash_returns_empty_robust() {
        let t = dummy_tool(
            ToolInput::Read {
                file_path: "foo.rs".into(),
                offset: None,
                limit: None,
            },
            ToolOutput::Empty,
            ToolKind::Read,
        );
        assert!(bash_continuation_lines(&t).is_empty());
    }

    // --- wrap_styled_line --------------------------------------------

    #[test]
    fn wrap_styled_line_short_returns_unchanged_normal() {
        let line = Line::from(vec![Span::raw("hello")]);
        let wrapped = wrap_styled_line(&line, 80);
        assert_eq!(wrapped.len(), 1);
    }

    #[test]
    fn wrap_styled_line_breaks_long_normal() {
        // 12 chars at width 5 → 3 lines.
        let line = Line::from(vec![Span::raw("abcdefghijkl")]);
        let wrapped = wrap_styled_line(&line, 5);
        assert_eq!(wrapped.len(), 3);
        let combined: String = wrapped
            .iter()
            .flat_map(|l| l.spans.iter().map(|s| s.content.as_ref()))
            .collect();
        assert_eq!(combined, "abcdefghijkl");
    }

    #[test]
    fn wrap_styled_line_zero_width_returns_unchanged_robust() {
        // 0 width → return clone unchanged, don't infinite-loop.
        let line = Line::from(vec![Span::raw("anything")]);
        let wrapped = wrap_styled_line(&line, 0);
        assert_eq!(wrapped.len(), 1);
    }

    #[test]
    fn wrap_styled_line_preserves_styles_across_wraps_robust() {
        // Two styled spans across a wrap boundary keep their styles.
        let red = Style::default().fg(Color::Red);
        let blue = Style::default().fg(Color::Blue);
        let line = Line::from(vec![
            Span::styled("redred", red),
            Span::styled("blueblue", blue),
        ]);
        let wrapped = wrap_styled_line(&line, 4);
        // 14 chars at width 4 = 4 lines.
        assert_eq!(wrapped.len(), 4);
    }

    // --- sanitize_terminal_text --------------------------------------

    #[test]
    fn sanitize_keeps_visible_text_normal() {
        assert_eq!(sanitize_terminal_text("hello world"), "hello world");
    }

    #[test]
    fn sanitize_strips_csi_escape_normal() {
        // \x1b[31m red CSI sequence — should be removed entirely.
        let input = "\u{1b}[31mred\u{1b}[0m text";
        assert_eq!(sanitize_terminal_text(input), "red text");
    }

    #[test]
    fn sanitize_expands_tab_to_four_spaces_normal() {
        assert_eq!(sanitize_terminal_text("a\tb"), "a    b");
    }

    #[test]
    fn sanitize_keeps_newline_robust() {
        assert_eq!(sanitize_terminal_text("a\nb"), "a\nb");
    }

    #[test]
    fn sanitize_strips_osc_terminated_by_bel_robust() {
        // OSC `\x1b]...\x07` sequence should be stripped.
        let input = "\u{1b}]0;title\u{7}body";
        assert_eq!(sanitize_terminal_text(input), "body");
    }

    #[test]
    fn sanitize_drops_control_chars_robust() {
        // Backspace (0x08) and other control chars vanish.
        assert_eq!(sanitize_terminal_text("a\u{8}b"), "ab");
    }

    // --- tool_kind_color ---------------------------------------------

    #[test]
    fn tool_kind_color_distinct_per_family_normal() {
        let t = Theme::dark();
        // Read = blue, Write = amber, Edit = mint — all distinct.
        let read_c = tool_kind_color(&ToolKind::Read, &t);
        let write_c = tool_kind_color(&ToolKind::Write, &t);
        let edit_c = tool_kind_color(&ToolKind::Edit, &t);
        assert_ne!(read_c, write_c);
        assert_ne!(write_c, edit_c);
        assert_ne!(read_c, edit_c);
    }

    #[test]
    fn tool_kind_color_grep_glob_search_share_lavender_normal() {
        // Grep family shares the search/lavender color.
        let t = Theme::dark();
        assert_eq!(
            tool_kind_color(&ToolKind::Grep, &t),
            tool_kind_color(&ToolKind::Glob, &t)
        );
        assert_eq!(
            tool_kind_color(&ToolKind::Grep, &t),
            tool_kind_color(&ToolKind::Search, &t)
        );
    }

    #[test]
    fn tool_kind_color_generic_uses_secondary_robust() {
        // Generic kinds fall back to text_secondary.
        let t = Theme::dark();
        assert_eq!(
            tool_kind_color(&ToolKind::Generic("custom".into()), &t),
            t.text_secondary
        );
    }

    // --- is_groupable ------------------------------------------------

    #[test]
    fn is_groupable_search_kinds_normal() {
        assert!(is_groupable(&ToolKind::Read));
        assert!(is_groupable(&ToolKind::Glob));
        assert!(is_groupable(&ToolKind::Grep));
        assert!(is_groupable(&ToolKind::Search));
    }

    #[test]
    fn is_groupable_destructive_kinds_robust() {
        // Edit/Write/Bash never group — each call's behavior matters.
        assert!(!is_groupable(&ToolKind::Edit));
        assert!(!is_groupable(&ToolKind::Write));
        assert!(!is_groupable(&ToolKind::Bash));
        assert!(!is_groupable(&ToolKind::Generic("foo".into())));
    }

    // --- tool_status_icon_animated -----------------------------------

    #[test]
    fn tool_status_icon_animated_running_rotates_glyph_normal() {
        // Running + frame=0 → first frame; frame=4 → second; frame=8 → third.
        let t = Theme::dark();
        let tool = dummy_tool(
            ToolInput::Bash {
                command: "x".into(),
                timeout: None,
                workdir: None,
            },
            ToolOutput::Empty,
            ToolKind::Bash,
        );
        let mut tool = tool;
        tool.status = ToolStatus::Running;
        let (g0, _) = tool_status_icon_animated(&tool, &t, 0);
        let (g4, _) = tool_status_icon_animated(&tool, &t, 4);
        let (g8, _) = tool_status_icon_animated(&tool, &t, 8);
        let (g12, _) = tool_status_icon_animated(&tool, &t, 12);
        assert_eq!(g0, "✶");
        assert_eq!(g4, "✷");
        assert_eq!(g8, "✸");
        assert_eq!(g12, "✹");
    }

    #[test]
    fn tool_status_icon_animated_pending_alternates_normal() {
        let t = Theme::dark();
        let tool = ToolCall {
            id: "p".into(),
            kind: ToolKind::Bash,
            status: ToolStatus::Pending,
            input: ToolInput::Bash {
                command: "x".into(),
                timeout: None,
                workdir: None,
            },
            output: ToolOutput::Empty,
            is_collapsed: false,
            expanded: false,
            elapsed_ms: None,
            started_at: None,
            pinned: false,
        };
        let (g0, _) = tool_status_icon_animated(&tool, &t, 0);
        let (g6, _) = tool_status_icon_animated(&tool, &t, 6);
        // PENDING_FRAMES is &["○", "◌"] at frame/6 cadence.
        assert_eq!(g0, "○");
        assert_eq!(g6, "◌");
    }

    #[test]
    fn tool_status_icon_animated_complete_static_robust() {
        // Complete state always returns the static icon regardless of frame.
        let t = Theme::dark();
        let tool = ToolCall {
            id: "c".into(),
            kind: ToolKind::Bash,
            status: ToolStatus::Complete,
            input: ToolInput::Bash {
                command: "x".into(),
                timeout: None,
                workdir: None,
            },
            output: ToolOutput::Empty,
            is_collapsed: false,
            expanded: false,
            elapsed_ms: None,
            started_at: None,
            pinned: false,
        };
        let (g0, _) = tool_status_icon_animated(&tool, &t, 0);
        let (g100, _) = tool_status_icon_animated(&tool, &t, 100);
        assert_eq!(g0, "●");
        assert_eq!(g100, "●");
    }

    #[test]
    fn tool_status_icon_animated_failed_static_robust() {
        let t = Theme::dark();
        let tool = ToolCall {
            id: "f".into(),
            kind: ToolKind::Bash,
            status: ToolStatus::Failed,
            input: ToolInput::Bash {
                command: "x".into(),
                timeout: None,
                workdir: None,
            },
            output: ToolOutput::Empty,
            is_collapsed: false,
            expanded: false,
            elapsed_ms: None,
            started_at: None,
            pinned: false,
        };
        let (g, _) = tool_status_icon_animated(&tool, &t, 42);
        assert_eq!(g, "✗");
    }

    // --- format_elapsed_badge ----------------------------------------

    #[test]
    fn format_elapsed_badge_below_threshold_returns_none_normal() {
        // Sub-100ms results don't get a badge (too noisy).
        let mut t = dummy_tool(
            ToolInput::Bash {
                command: "x".into(),
                timeout: None,
                workdir: None,
            },
            ToolOutput::Empty,
            ToolKind::Bash,
        );
        t.elapsed_ms = Some(50);
        assert_eq!(format_elapsed_badge(&t), None);
    }

    #[test]
    fn format_elapsed_badge_seconds_decimal_normal() {
        let mut t = dummy_tool(
            ToolInput::Bash {
                command: "x".into(),
                timeout: None,
                workdir: None,
            },
            ToolOutput::Empty,
            ToolKind::Bash,
        );
        t.elapsed_ms = Some(2300);
        assert_eq!(format_elapsed_badge(&t).as_deref(), Some("[2.3s]"));
    }

    #[test]
    fn format_elapsed_badge_tens_of_seconds_no_decimal_normal() {
        let mut t = dummy_tool(
            ToolInput::Bash {
                command: "x".into(),
                timeout: None,
                workdir: None,
            },
            ToolOutput::Empty,
            ToolKind::Bash,
        );
        t.elapsed_ms = Some(15_000);
        assert_eq!(format_elapsed_badge(&t).as_deref(), Some("[15s]"));
    }

    #[test]
    fn format_elapsed_badge_minutes_format_robust() {
        let mut t = dummy_tool(
            ToolInput::Bash {
                command: "x".into(),
                timeout: None,
                workdir: None,
            },
            ToolOutput::Empty,
            ToolKind::Bash,
        );
        t.elapsed_ms = Some(125_000);
        assert_eq!(format_elapsed_badge(&t).as_deref(), Some("[2m 5s]"));
    }

    #[test]
    fn format_elapsed_badge_running_returns_none_robust() {
        let mut t = dummy_tool(
            ToolInput::Bash {
                command: "x".into(),
                timeout: None,
                workdir: None,
            },
            ToolOutput::Empty,
            ToolKind::Bash,
        );
        t.status = ToolStatus::Running;
        t.elapsed_ms = Some(2300);
        assert_eq!(format_elapsed_badge(&t), None);
    }

    #[test]
    fn format_elapsed_badge_no_elapsed_returns_none_robust() {
        let t = dummy_tool(
            ToolInput::Bash {
                command: "x".into(),
                timeout: None,
                workdir: None,
            },
            ToolOutput::Empty,
            ToolKind::Bash,
        );
        // Even Complete: if elapsed_ms is None, no badge.
        assert_eq!(format_elapsed_badge(&t), None);
    }

    // --- tool_title_width_cap ----------------------------------------

    #[test]
    fn tool_title_width_cap_default_is_100_normal() {
        // Without any env override, default is 100.
        unsafe {
            std::env::remove_var("JFC_TOOL_TITLE_WIDTH");
        }
        assert_eq!(tool_title_width_cap(), 100);
    }

    #[test]
    fn tool_title_width_cap_rejects_too_small_robust() {
        // Values < 20 are rejected by `.filter(|n| *n >= 20)` → fallback to 100.
        unsafe {
            std::env::set_var("JFC_TOOL_TITLE_WIDTH", "5");
        }
        assert_eq!(tool_title_width_cap(), 100);
        unsafe {
            std::env::remove_var("JFC_TOOL_TITLE_WIDTH");
        }
    }

    // --- build_collapsed_header / build_title_spans / build_header_inner_spans

    #[test]
    fn build_header_inner_spans_bash_format_normal() {
        let t = Theme::dark();
        let tool = dummy_tool(
            ToolInput::Bash {
                command: "echo hi".into(),
                timeout: None,
                workdir: None,
            },
            ToolOutput::Empty,
            ToolKind::Bash,
        );
        let spans = build_header_inner_spans(&tool, &t, 80);
        // 4 spans: "Bash" + "(" + cmd + ")".
        assert_eq!(spans.len(), 4);
        assert_eq!(spans[0].content, "Bash");
        assert_eq!(spans[1].content, "(");
        assert_eq!(spans[3].content, ")");
    }

    #[test]
    fn build_header_inner_spans_read_format_normal() {
        let t = Theme::dark();
        let tool = dummy_tool(
            ToolInput::Read {
                file_path: "src/main.rs".into(),
                offset: None,
                limit: None,
            },
            ToolOutput::Empty,
            ToolKind::Read,
        );
        let spans = build_header_inner_spans(&tool, &t, 80);
        assert_eq!(spans.len(), 4);
        assert_eq!(spans[0].content, "Read");
        assert!(spans[2].content.contains("src/main.rs"));
    }

    #[test]
    fn build_header_inner_spans_write_format_normal() {
        let t = Theme::dark();
        let tool = dummy_tool(
            ToolInput::Write {
                file_path: "out.txt".into(),
                content: "".into(),
            },
            ToolOutput::Empty,
            ToolKind::Write,
        );
        let spans = build_header_inner_spans(&tool, &t, 80);
        assert_eq!(spans[0].content, "Write");
    }

    #[test]
    fn build_header_inner_spans_edit_format_normal() {
        let t = Theme::dark();
        let tool = dummy_tool(
            ToolInput::Edit {
                file_path: "src/lib.rs".into(),
                old_string: "".into(),
                new_string: "".into(),
                replacement: ReplacementMode::FirstOnly,
            },
            ToolOutput::Empty,
            ToolKind::Edit,
        );
        let spans = build_header_inner_spans(&tool, &t, 80);
        assert_eq!(spans[0].content, "Update");
    }

    #[test]
    fn build_header_inner_spans_long_path_truncates_robust() {
        // A very long path gets truncated with ellipsis.
        let t = Theme::dark();
        let long_path = "a/".repeat(100) + "main.rs";
        let tool = dummy_tool(
            ToolInput::Read {
                file_path: long_path,
                offset: None,
                limit: None,
            },
            ToolOutput::Empty,
            ToolKind::Read,
        );
        let spans = build_header_inner_spans(&tool, &t, 30);
        let path_span = &spans[2].content;
        assert!(
            path_span.chars().count() <= 30,
            "got len {}: {path_span:?}",
            path_span.chars().count()
        );
    }

    #[test]
    fn build_collapsed_header_includes_status_icon_normal() {
        let t = Theme::dark();
        let tool = dummy_tool(
            ToolInput::Bash {
                command: "echo".into(),
                timeout: None,
                workdir: None,
            },
            ToolOutput::Empty,
            ToolKind::Bash,
        );
        let line = build_collapsed_header(&tool, &t, 80);
        // First span is the status icon, second is " ".
        assert_eq!(line.spans[1].content, " ");
        assert!(!line.spans.is_empty());
    }

    #[test]
    fn build_title_spans_includes_pin_glyph_when_pinned_robust() {
        let t = Theme::dark();
        let mut tool = dummy_tool(
            ToolInput::Bash {
                command: "x".into(),
                timeout: None,
                workdir: None,
            },
            ToolOutput::Empty,
            ToolKind::Bash,
        );
        tool.pinned = true;
        let spans = build_title_spans(&tool, &t, "●", Style::default(), 80);
        let combined: String = spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(combined.contains("📌"), "expected pin glyph: {combined:?}");
    }

    #[test]
    fn build_title_spans_appends_elapsed_badge_robust() {
        let t = Theme::dark();
        let mut tool = dummy_tool(
            ToolInput::Bash {
                command: "x".into(),
                timeout: None,
                workdir: None,
            },
            ToolOutput::Empty,
            ToolKind::Bash,
        );
        tool.elapsed_ms = Some(2500);
        let spans = build_title_spans(&tool, &t, "●", Style::default(), 80);
        let combined: String = spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(
            combined.contains("[2.5s]"),
            "expected elapsed badge: {combined:?}"
        );
    }

    // --- border_color_for_status -------------------------------------

    #[test]
    fn border_color_for_status_each_state_normal() {
        let t = Theme::dark();
        for (status, expected) in [
            (ToolStatus::Pending, t.warning),
            (ToolStatus::Running, t.accent),
            (ToolStatus::Complete, t.border),
            (ToolStatus::Failed, t.error),
        ] {
            let mut tool = dummy_tool(
                ToolInput::Bash {
                    command: "x".into(),
                    timeout: None,
                    workdir: None,
                },
                ToolOutput::Empty,
                ToolKind::Bash,
            );
            tool.status = status;
            assert_eq!(border_color_for_status(&tool, &t), expected);
        }
    }

    // --- severity_rank -----------------------------------------------

    #[test]
    fn severity_rank_orders_correctly_normal() {
        use crate::diagnostics::Severity;
        assert!(severity_rank(Severity::Error) > severity_rank(Severity::Warning));
        assert!(severity_rank(Severity::Warning) > severity_rank(Severity::Info));
        assert!(severity_rank(Severity::Info) > severity_rank(Severity::Hint));
    }

    #[test]
    fn severity_rank_distinct_values_robust() {
        use crate::diagnostics::Severity;
        let mut v = vec![
            severity_rank(Severity::Error),
            severity_rank(Severity::Warning),
            severity_rank(Severity::Info),
            severity_rank(Severity::Hint),
        ];
        v.sort();
        v.dedup();
        assert_eq!(v.len(), 4, "all 4 ranks must be distinct");
    }

    // --- truncate_str (private inside message_view) ------------------

    #[test]
    fn truncate_str_short_passes_through_normal() {
        assert_eq!(truncate_str("hi", 10), "hi");
    }

    #[test]
    fn truncate_str_long_truncates_with_ellipsis_normal() {
        let s = truncate_str("hello world", 5);
        assert!(s.ends_with('…'));
        assert_eq!(s.chars().count(), 5);
    }

    #[test]
    fn truncate_str_zero_returns_empty_robust() {
        assert_eq!(truncate_str("hi", 0), "");
    }

    // --- parse_grep_with_sep / parse_grep_no_path direct ------------

    #[test]
    fn parse_grep_with_sep_match_form_normal() {
        let r = parse_grep_with_sep("src/foo.rs:5:body", ':', false);
        match r {
            Some(GrepLine::Match {
                path,
                lineno,
                body,
                is_context,
                ..
            }) => {
                assert_eq!(path, "src/foo.rs");
                assert_eq!(lineno, Some("5"));
                assert_eq!(body, "body");
                assert!(!is_context);
            }
            _ => panic!("expected match"),
        }
    }

    #[test]
    fn parse_grep_with_sep_no_match_returns_none_robust() {
        // No `<sep><digits><sep>` anchor → None.
        assert!(parse_grep_with_sep("just plain text", ':', false).is_none());
    }

    #[test]
    fn parse_grep_no_path_match_form_normal() {
        let r = parse_grep_no_path("42:body line", ':', false);
        match r {
            Some(GrepLine::Match {
                path,
                lineno,
                body,
                is_context,
                ..
            }) => {
                assert_eq!(path, "");
                assert_eq!(lineno, Some("42"));
                assert_eq!(body, "body line");
                assert!(!is_context);
            }
            _ => panic!("expected match"),
        }
    }

    #[test]
    fn parse_grep_no_path_non_digit_start_robust() {
        assert!(parse_grep_no_path("foo:body", ':', false).is_none());
    }

    #[test]
    fn parse_grep_no_path_empty_robust() {
        assert!(parse_grep_no_path("", ':', false).is_none());
    }

    // --- message_view_total_lines ------------------------------------

    #[test]
    fn message_view_total_lines_empty_app_normal() {
        // Build a fake App via the test helpers — empty messages → 0 lines.
        use crate::provider::{EventStream, ModelInfo, Provider, ProviderMessage, StreamOptions};
        use std::sync::Arc;

        struct Stub;
        #[async_trait::async_trait]
        impl Provider for Stub {
            fn name(&self) -> &str {
                "test"
            }
            fn available_models(&self) -> Vec<ModelInfo> {
                Vec::new()
            }
            async fn stream(
                &self,
                _: Vec<ProviderMessage>,
                _: &StreamOptions,
            ) -> anyhow::Result<EventStream> {
                Ok(Box::pin(futures::stream::empty()))
            }
        }

        let app = App::new(Arc::new(Stub), "test-model");
        // No messages → 0 lines.
        assert_eq!(message_view_total_lines(&app, 80), 0);
    }
}
