use super::visual::*;
use super::*;
use crate::markdown;
pub(super) fn input(f: &mut Frame, app: &mut App, area: Rect) {
    let t = app.theme;
    let ui_tokens = t.claude_ui_tokens();
    // Up to 4 cells reserved for the prompt + an animation tail
    // (currently only used by `:comet` mode).
    //
    // Prompt glyph: a static `›` chevron by default — honest, zero
    // animation, reads instantly as "type here". Power users can still
    // opt into a different glyph or an animated preset via JFC_PROMPT_CHAR:
    //   :comet / :moon / :dice / :notes / :hourglass / :atom — presets
    //   <any single char> — that char as a static glyph
    // Edit mode overrides any choice with `✎` (pencil).
    let in_edit_mode = app.editing_message_idx.is_some();
    let raw_setting = std::env::var("JFC_PROMPT_CHAR").unwrap_or_else(|_| "›".to_string());
    let mode = parse_prompt_mode(&raw_setting);
    let now_ms = app.launched_at.elapsed().as_millis();
    let streaming_for_anim = app.engine.is_streaming && !crate::spinner::reduced_motion();
    let prompt_char: String = if in_edit_mode {
        "✎".to_string()
    } else if let PromptMode::Static(s) = &mode {
        s.clone()
    } else {
        prompt_mode_frame(&mode, streaming_for_anim, now_ms).to_string()
    };

    let (prompt_color, border_color) = if in_edit_mode {
        (t.warning, t.warning)
    } else if app.engine.is_streaming {
        (t.accent, ui_tokens.prompt_border_shimmer)
    } else {
        (t.accent, ui_tokens.prompt_border)
    };

    // Edit-mode / vim-mode badge in the title (top border) so the user
    // can't miss the editing state. Title is otherwise empty.
    let title_line = if let Some(idx) = app.editing_message_idx {
        Some(Line::from(Span::styled(
            format!(" editing #{idx} · Esc to cancel "),
            Style::default().fg(t.warning).add_modifier(Modifier::BOLD),
        )))
    } else if let Some(vim) = app.vim.as_ref() {
        // Mode color tracks vim convention: Normal=accent, Insert=success,
        // Visual=warning. A steady tag, no animation.
        let mode = vim.mode;
        let color = match mode {
            crate::input::vim::VimMode::Insert => t.success,
            crate::input::vim::VimMode::Visual => t.warning,
            _ => t.accent,
        };
        Some(Line::from(Span::styled(
            format!(" {} ", mode.tag()),
            Style::default().fg(color).add_modifier(Modifier::BOLD),
        )))
    } else {
        None
    };

    let has_title = title_line.is_some();
    let mut block = Block::default()
        .borders(if has_title {
            Borders::LEFT
        } else {
            Borders::NONE
        })
        .border_style(Style::default().fg(border_color))
        .padding(Padding::horizontal(1))
        .style(Style::default().bg(t.bg));
    if let Some(title_line) = title_line {
        block = block.title(title_line);
    }
    let inner = block.inner(area);
    f.render_widget(block, area);
    let vertical_pad = if has_title || inner.height < 3 { 0 } else { 1 };
    let content_inner = Rect {
        x: inner.x,
        y: inner.y.saturating_add(vertical_pad),
        width: inner.width,
        height: inner.height.saturating_sub(vertical_pad.saturating_mul(2)),
    };

    // Prompt strip: glyph display width + trailing space. Custom prompt
    // glyphs can be double-width, so fixed 2-cell math lets the textarea
    // overlap the prompt.
    let prompt_width =
        unicode_width::UnicodeWidthStr::width(prompt_char.as_str()).min(u16::MAX as usize) as u16;
    let prompt_cells: u16 = prompt_width.saturating_add(1);
    let textarea_x = content_inner.x + prompt_cells.min(content_inner.width);
    let textarea_w = content_inner.width.saturating_sub(prompt_cells);

    if content_inner.height > 0 && content_inner.y < f.buffer_mut().area().bottom() {
        let buf = f.buffer_mut();
        // Glyph cell.
        let glyph_x = content_inner.x;
        if glyph_x < buf.area().right() {
            let cell = &mut buf[(glyph_x, content_inner.y)];
            cell.set_symbol(&prompt_char);
            let invert = matches!(
                std::env::var("JFC_PROMPT_INVERT").as_deref(),
                Ok("1") | Ok("true")
            );
            let style = if invert {
                Style::default()
                    .fg(t.bg)
                    .bg(prompt_color)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default()
                    .fg(prompt_color)
                    .bg(t.bg)
                    .add_modifier(Modifier::BOLD)
            };
            cell.set_style(style);
        }
        // Trailing space so the glyph isn't glued to text.
        let space_x = content_inner.x.saturating_add(prompt_width);
        if space_x < buf.area().right() {
            let cell = &mut buf[(space_x, content_inner.y)];
            cell.set_symbol(" ");
            cell.set_style(Style::default().bg(t.bg));
        }
    }

    // Textarea inner rect (everything to the right of the prompt
    // strip).
    let inner = Rect {
        x: textarea_x,
        y: content_inner.y,
        width: textarea_w,
        height: content_inner.height,
    };
    *app.input_rect.borrow_mut() = Some(inner);

    let content_width = inner.width.max(1) as usize;
    app.input_wrap_width = content_width;
    let (lines, cursor_row, cursor_col) = input_soft_wrapped_lines(app, content_width);
    let visible_rows = inner.height.max(1) as usize;
    let start = cursor_row.saturating_add(1).saturating_sub(visible_rows);
    // Slash-command and @mention tokens get one accent color (bold) so
    // the user can see they'll route somewhere distinct — a slash command,
    // a file mention — rather than be sent as plain text. A flat color,
    // not the old wallclock-driven rainbow that animated for no reason.
    let visible = lines
        .iter()
        .skip(start)
        .take(visible_rows)
        .map(|line| Line::from(input_line_to_spans(line, t)))
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
            .style(Style::default().bg(t.bg)),
        inner,
    );

    let cursor_x = inner
        .x
        .saturating_add(cursor_col as u16)
        .min(inner.right().saturating_sub(1));
    let cursor_y = inner
        .y
        .saturating_add(cursor_row.saturating_sub(start) as u16)
        .min(inner.bottom().saturating_sub(1));

    // While recording, the cursor *becomes* a live RMS bar whose hue rotates
    // while the mic is open — the literal port of CC 2.1.177's animated cursor
    // (`RM8`/`pd9`). We paint the glyph into the cursor cell at the insertion
    // point and deliberately skip `set_cursor_position`, so ratatui hides the
    // hardware cursor and the animated glyph is what the user sees. Reduced
    // motion falls back to the normal cursor (matching CC, which disables the
    // custom cursor under reduced motion).
    let animate_voice_cursor = app.voice_state == jfc_voice::VoiceState::Recording
        && !crate::spinner::reduced_motion()
        && area.height > 1
        && area.width > 2;

    if animate_voice_cursor {
        let elapsed = app
            .voice_record_started
            .map(|t| t.elapsed().as_millis())
            .unwrap_or(0);
        let (glyph, color) = crate::render::voice_cursor::recording_glyph(
            &app.voice_audio_levels,
            elapsed,
            crate::render::voice_cursor::terminal_truecolor(),
        );
        let buf = f.buffer_mut();
        if cursor_x < buf.area().right() && cursor_y < buf.area().bottom() {
            let cell = &mut buf[(cursor_x, cursor_y)];
            cell.set_char(glyph);
            cell.set_style(
                cell.style()
                    .fg(color)
                    .add_modifier(ratatui::style::Modifier::BOLD),
            );
        }
        // No set_cursor_position — the animated glyph is the cursor.
    } else {
        // Ghost cursor pulse: tint the cursor cell's background between
        // surface_raised and accent so the typing surface feels "ready" even
        // when nothing's moving. Only visible when not streaming (the spinner
        // takes over the visual focus during streaming) and not in edit mode
        // (the orange edit border is already a strong signal). Reduced-motion
        // skips the pulse.
        if !app.engine.is_streaming
            && !in_edit_mode
            && !crate::spinner::reduced_motion()
            && area.height > 1
            && area.width > 2
        {
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

        if area.height > 1 && area.width > 2 {
            f.set_cursor_position(Position::new(cursor_x, cursor_y));
        }
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
        if app.engine.queued_prompts.is_empty() {
            out.push(String::new());
        } else {
            out.push("Press up to edit queued messages".to_owned());
        }
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
