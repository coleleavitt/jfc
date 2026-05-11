use super::assistant_parts::sanitize_terminal_text;
use super::*;

pub(super) fn looks_like_git_diff_output(text: &str) -> bool {
    let mut saw_diff_header = false;
    let mut saw_hunk_header = false;
    let mut saw_file_marker = false;
    let mut saw_diffstat = false;

    for raw in text.lines().take(80) {
        let line = sanitize_terminal_text(raw);
        if line.starts_with("diff --git ") || line.starts_with("index ") {
            saw_diff_header = true;
        } else if line.starts_with("@@") {
            saw_hunk_header = true;
        } else if line.starts_with("--- ") || line.starts_with("+++ ") {
            saw_file_marker = true;
        } else if terminal_output::colorize_diffstat_line(&line, Color::Reset, Theme::dark())
            .is_some()
        {
            saw_diffstat = true;
        }
    }

    saw_diff_header || (saw_hunk_header && saw_file_marker) || saw_diffstat
}
