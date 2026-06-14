use ratatui::style::Style;
use ratatui_textarea::{CursorMove, TextArea};

use crate::app::App;

pub(super) fn reset_input(app: &mut App) {
    let before = textarea_char_len(app);
    app.textarea = TextArea::default();
    app.textarea.set_cursor_line_style(Style::default());
    app.textarea.set_placeholder_text("send a message…");
    tracing::debug!(
        target: "jfc::input::recall",
        cleared_chars = before,
        "reset_input: textarea cleared"
    );
}

/// Total character count across all textarea lines (joined by `\n`). Cheap
/// scalar used by the input-flow tracing to spot append-instead-of-replace
/// bugs (the prompt-doubling regression): if a recall/submit logs
/// `before > 0` where it expected an empty buffer, the textarea wasn't
/// cleared first.
pub(super) fn textarea_char_len(app: &App) -> usize {
    app.textarea
        .lines()
        .iter()
        .map(|l| l.chars().count())
        .sum::<usize>()
        // account for the `\n` joiners so the number matches submit's text.len()
        + app.textarea.lines().len().saturating_sub(1)
}

fn input_line_char_len(app: &App, line: usize) -> usize {
    app.textarea
        .lines()
        .get(line)
        .map(|line| line.chars().count())
        .unwrap_or_default()
}

fn cursor_index(value: usize) -> u16 {
    value.min(u16::MAX as usize) as u16
}

pub(super) fn move_input_cursor_visual_up(app: &mut App) {
    let width = app.input_wrap_width.max(1);
    let cursor = app.textarea.cursor();
    let (line, col) = (cursor.0, cursor.1);

    if col >= width {
        app.textarea.move_cursor(CursorMove::Jump(
            cursor_index(line),
            cursor_index(col - width),
        ));
        return;
    }

    if line == 0 {
        app.textarea.move_cursor(CursorMove::Head);
        return;
    }

    let prev_len = input_line_char_len(app, line - 1);
    let prev_visual_start = (prev_len / width) * width;
    let target_col = prev_len.min(prev_visual_start + col);
    app.textarea.move_cursor(CursorMove::Jump(
        cursor_index(line - 1),
        cursor_index(target_col),
    ));
}

pub(super) fn move_input_cursor_visual_down(app: &mut App) {
    let width = app.input_wrap_width.max(1);
    let cursor = app.textarea.cursor();
    let (line, col) = (cursor.0, cursor.1);
    let line_len = input_line_char_len(app, line);

    if col + width <= line_len {
        app.textarea.move_cursor(CursorMove::Jump(
            cursor_index(line),
            cursor_index(col + width),
        ));
        return;
    }

    let next_line = line + 1;
    if next_line >= app.textarea.lines().len() {
        app.textarea.move_cursor(CursorMove::End);
        return;
    }

    let target_col = input_line_char_len(app, next_line).min(col % width);
    app.textarea.move_cursor(CursorMove::Jump(
        cursor_index(next_line),
        cursor_index(target_col),
    ));
}

pub(super) fn input_has_text(app: &App) -> bool {
    app.textarea.lines().iter().any(|line| !line.is_empty())
}

pub(super) fn step_reasoning_effort(app: &mut App, raise: bool) {
    let current = app.engine.effort_state.current.unwrap_or_default();
    let next = if raise {
        current.next()
    } else {
        current.previous()
    };
    let message = match next {
        Some(level) => app.engine.effort_state.set(level),
        None if raise => format!("Reasoning effort is already at max ({current})"),
        None => format!("Reasoning effort is already at min ({current})"),
    };
    jfc_engine::toast::push_with_cap(
        &mut app.engine.toasts,
        jfc_engine::toast::Toast::new(jfc_engine::toast::ToastKind::Info, message),
    );
}
