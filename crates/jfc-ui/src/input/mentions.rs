use ratatui::style::Style;
use ratatui_textarea::{CursorMove, TextArea};

use crate::app::App;

/// Replace the active `@<query>` token in the textarea with the picked
/// path + trailing space. Reconstructs the textarea from the resulting
/// string so cursor positioning is correct (the `ratatui_textarea` API
/// doesn't expose a "replace range" operation).
pub(super) fn apply_mention_pick(app: &mut App, pick: &str) {
    let buffer = app.textarea.lines().join("\n");
    let anchor = app.mention.anchor_byte;
    let q_len = app.mention.query.chars().count();
    let (new_buf, _new_cursor) = crate::mentions::apply_acceptance(&buffer, anchor, q_len, pick);
    app.textarea = TextArea::from(new_buf.lines().map(str::to_string).collect::<Vec<_>>());
    app.textarea.set_cursor_line_style(Style::default());
    app.textarea.set_placeholder_text("send a message…");
    app.textarea.move_cursor(CursorMove::End);
}

/// Decide whether the popup should activate (newly-typed `@` after
/// whitespace) or update its query (already-active, more chars typed
/// or backspace shrunk the buffer).
pub(super) fn update_mention_state_after_input(app: &mut App) {
    let cursor = app.textarea.cursor();
    let (line_idx, col) = (cursor.0, cursor.1);
    let line = match app.textarea.lines().get(line_idx) {
        Some(s) => s.clone(),
        None => return,
    };
    let prefix: String = line.chars().take(col).collect();
    if app.mention.active {
        // Recompute query from anchor -> cursor on the same line. If the
        // user backspaced past the `@` or moved off-line, dismiss.
        let buffer = app.textarea.lines().join("\n");
        if app.mention.anchor_byte >= buffer.len()
            || !buffer[app.mention.anchor_byte..].starts_with('@')
        {
            app.mention.dismiss();
            return;
        }
        let after_at = &buffer[app.mention.anchor_byte + 1..];
        let q: String = after_at
            .chars()
            .take_while(|c| !c.is_whitespace())
            .collect();
        let all = app.mention_all_files.clone();
        app.mention.update_query(q, &all);
        let after_q_len = app.mention.anchor_byte + 1 + app.mention.query.len();
        if after_q_len < buffer.len() && buffer[after_q_len..].starts_with(char::is_whitespace) {
            app.mention.dismiss();
        }
        return;
    }
    if let Some(anchor) = crate::mentions::should_activate(&prefix) {
        // Lazy-load file list so we don't walk `cwd` on every keystroke.
        if app.mention_all_files.is_empty() {
            let cwd = std::env::current_dir().unwrap_or_else(|_| ".".into());
            app.mention_all_files = crate::mentions::scan_files(&cwd, 5000);
        }
        let all = app.mention_all_files.clone();
        let initial = crate::mentions::filter_candidates(&all, "");
        app.mention.activate(anchor, initial);
    }
}
