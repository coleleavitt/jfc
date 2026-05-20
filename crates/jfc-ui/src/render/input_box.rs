use super::visual::*;
use super::*;
use crate::markdown;
pub(super) fn input(f: &mut Frame, app: &mut App, area: Rect) {
    let t = app.theme;
    // Boxed input with rounded border. The prompt char sits INLINE
    // at the start of the typing surface — like a shell prompt.
    // Up to 4 cells reserved for the prompt + an animation tail
    // (currently only used by `:comet` mode).
    //
    // Prompt mode selector via `JFC_PROMPT_CHAR`:
    //   :comet     — comet `☄` with streak tail (default)
    //   :moon      — moon phases ○◐●◑ cycle while streaming
    //   :dice      — dice faces ⚀⚁⚂⚃⚄⚅ shuffle while streaming
    //   :notes     — music notes ♩♪♫♬ cycle while streaming
    //   :hourglass — `⌛` ↔ `⌚` flip every 800ms
    //   :atom      — atom `⚛` (just color pulse, no shape change)
    //   <any single char> — that char as a static glyph (color pulse)
    // Edit mode overrides any choice with `✎` (pencil).
    let in_edit_mode = app.editing_message_idx.is_some();
    let raw_setting = std::env::var("JFC_PROMPT_CHAR").unwrap_or_else(|_| ":comet".to_string());
    let mode = parse_prompt_mode(&raw_setting);
    let now_ms = app.launched_at.elapsed().as_millis();
    let streaming_for_anim = app.is_streaming && !crate::spinner::reduced_motion();
    let prompt_char: String = if in_edit_mode {
        "✎".to_string()
    } else if let PromptMode::Static(s) = &mode {
        s.clone()
    } else {
        prompt_mode_frame(&mode, streaming_for_anim, now_ms).to_string()
    };

    let (prompt_color, border_color) = if in_edit_mode {
        (t.warning, t.warning)
    } else if app.is_streaming {
        (t.accent, t.text_muted)
    } else {
        (t.accent, t.border)
    };

    // Edit-mode badge in the title (top border) so the user can't
    // miss the editing state. Title is otherwise empty.
    let title_line = if let Some(idx) = app.editing_message_idx {
        Line::from(Span::styled(
            format!(" editing #{idx} · Esc to cancel "),
            Style::default().fg(t.warning).add_modifier(Modifier::BOLD),
        ))
    } else {
        Line::from("")
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(ratatui::widgets::BorderType::Rounded)
        .border_style(Style::default().fg(border_color))
        .padding(Padding::horizontal(1))
        .title(title_line)
        .style(Style::default().bg(t.surface));
    let inner = block.inner(area);
    f.render_widget(block, area);

    // Prompt strip: 2 cells reserved (glyph + trailing space).
    let prompt_cells: u16 = 2;
    let textarea_x = inner.x + prompt_cells.min(inner.width);
    let textarea_w = inner.width.saturating_sub(prompt_cells);

    // Paint the prompt glyph on the first row of inner.
    if inner.height > 0 && inner.y < f.buffer_mut().area().bottom() {
        let buf = f.buffer_mut();
        // Glyph cell.
        let glyph_x = inner.x;
        if glyph_x < buf.area().right() {
            let cell = &mut buf[(glyph_x, inner.y)];
            cell.set_symbol(&prompt_char);
            let invert = matches!(
                std::env::var("JFC_PROMPT_INVERT").as_deref(),
                Ok("1") | Ok("true")
            );
            let style = if invert {
                Style::default()
                    .fg(t.surface)
                    .bg(prompt_color)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default()
                    .fg(prompt_color)
                    .bg(t.surface)
                    .add_modifier(Modifier::BOLD)
            };
            cell.set_style(style);
        }
        // Trailing space so the glyph isn't glued to text.
        let space_x = inner.x + 1;
        if space_x < buf.area().right() {
            let cell = &mut buf[(space_x, inner.y)];
            cell.set_symbol(" ");
            cell.set_style(Style::default().bg(t.surface));
        }
    }

    // Textarea inner rect (everything to the right of the prompt
    // strip).
    let inner = Rect {
        x: textarea_x,
        y: inner.y,
        width: textarea_w,
        height: inner.height,
    };

    let content_width = inner.width.max(1) as usize;
    app.input_wrap_width = content_width;
    let (lines, cursor_row, cursor_col) = input_soft_wrapped_lines(app, content_width);
    let visible_rows = inner.height.max(1) as usize;
    let start = cursor_row.saturating_add(1).saturating_sub(visible_rows);
    // Rainbow gradient for slash-command and @mention prefixes — gives
    // those tokens a visible "specialness" so the user sees that
    // they'll route somewhere distinct (a slash command, a file
    // mention) rather than be sent as plain text. Phase rotates with
    // wallclock so the gradient gently flows through the chars on
    // each redraw. Reduced-motion holds the phase at 0 so the colors
    // stay still but the gradient is still applied — readable, just
    // not animated.
    let rainbow_phase = if crate::spinner::reduced_motion() {
        0.0_f32
    } else {
        (app.launched_at.elapsed().as_millis() as f32 / 25.0) % 360.0
    };
    let visible = lines
        .iter()
        .skip(start)
        .take(visible_rows)
        .map(|line| Line::from(input_line_to_spans(line, t, rainbow_phase)))
        .collect::<Vec<_>>();

    // `.wrap(Wrap{trim:false})` — without it, ratatui falls back to
    // `LineTruncator` and any visible line longer than `inner.width`
    // cells gets clipped at the right edge. `input_soft_wrapped_lines`
    // pre-wraps to fit, but pre-wrapping is char-count based; for
    // multi-cell unicode (CJK / emoji / fullwidth punctuation) the
    // pre-wrapped line was N chars but 2N cells wide → second half
    // disappeared.
    f.render_widget(
        Paragraph::new(visible)
            .wrap(ratatui::widgets::Wrap { trim: false })
            .style(Style::default().bg(t.surface)),
        inner,
    );

    // Ghost cursor pulse: tint the cursor cell's background between
    // surface_raised and accent on a 1.2s clock so the typing surface
    // feels "ready" even when nothing's moving. Only visible when
    // not streaming (the spinner takes over the visual focus during
    // streaming) and not in edit mode (the orange edit border is
    // already a strong signal). Reduced-motion skips the pulse.
    if !app.is_streaming
        && !in_edit_mode
        && !crate::spinner::reduced_motion()
        && area.height > 2
        && area.width > 2
    {
        let cursor_x = inner
            .x
            .saturating_add(cursor_col as u16)
            .min(inner.right().saturating_sub(1));
        let cursor_y = inner
            .y
            .saturating_add(cursor_row.saturating_sub(start) as u16)
            .min(inner.bottom().saturating_sub(1));
        let buf = f.buffer_mut();
        if cursor_x < buf.area().right() && cursor_y < buf.area().bottom() {
            // Static accent bg on the cursor cell. Previously this was a
            // pulsing animation (sin wave on elapsed time) which caused
            // ratatui to see a buffer diff every frame → 30fps terminal
            // writes even during idle. A static tint eliminates the diff
            // while still visually marking the cursor position.
            let bg = pulse_color(t.surface_raised, t.accent, 0.18);
            let cell = &mut buf[(cursor_x, cursor_y)];
            cell.set_style(cell.style().bg(bg));
        }
    }

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

pub(super) fn input_visual_line_count(app: &App, content_width: usize) -> usize {
    input_soft_wrapped_lines(app, content_width).0.len().max(1)
}

pub(super) fn input_soft_wrapped_lines(
    app: &App,
    content_width: usize,
) -> (Vec<String>, usize, usize) {
    use unicode_width::UnicodeWidthChar;

    let width = content_width.max(1);
    let logical_lines = app.textarea.lines();
    let cursor = app.textarea.cursor();
    let (cursor_line, cursor_col) = (cursor.0, cursor.1);
    let mut out = Vec::new();
    let mut visual_cursor_row = 0usize;
    let mut visual_cursor_col = 0usize;

    if logical_lines.iter().all(|line| line.is_empty()) {
        out.push("send a message…".to_string());
        return (out, 0, 0);
    }

    for (line_idx, line) in logical_lines.iter().enumerate() {
        if line_idx == cursor_line {
            // The textarea reports `cursor_col` as a CHAR INDEX, but
            // `hard_wrap_str` now wraps by CELL WIDTH. Convert by
            // walking the line up to the cursor's character index,
            // accumulating cell widths, and tracking which wrap row
            // the running total falls into.
            //
            // Earlier this used `cursor_col / width` and
            // `cursor_col % width` — correct only when 1 char = 1
            // cell. For CJK / emoji / fullwidth chars (each 2 cells)
            // the cursor displayed in the wrong visual position,
            // sometimes offscreen, and pre-wrapped lines didn't
            // line up with the rendered cell columns.
            let mut col_width = 0usize;
            let mut wrap_row = 0usize;
            for (i, ch) in line.chars().enumerate() {
                if i >= cursor_col {
                    break;
                }
                let cw = UnicodeWidthChar::width(ch).unwrap_or(1);
                if cw > 0 && col_width + cw > width {
                    wrap_row += 1;
                    col_width = 0;
                }
                col_width += cw;
            }
            visual_cursor_row = out.len() + wrap_row;
            visual_cursor_col = col_width;
        }
        let wrapped = markdown::hard_wrap_str(line, width);
        out.extend(wrapped);
    }

    (out, visual_cursor_row, visual_cursor_col)
}
