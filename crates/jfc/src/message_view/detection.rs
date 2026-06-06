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

/// Difftastic's side-by-side display is commonly installed as Git's external
/// diff. Its output is not unified diff text: it starts sections with headers
/// like `src/main.rs --- 1/3 --- Rust` or
/// `src/main.rs --- Text (exceeded DFT_GRAPH_LIMIT)` and then emits aligned
/// old/new columns. Treat this as preformatted diff output rather than code.
pub(super) fn looks_like_difftastic_output(text: &str) -> bool {
    for raw in text.lines().take(40) {
        let line = sanitize_terminal_text(raw);
        let trimmed = line.trim();
        if trimmed.starts_with("--- ") || trimmed.starts_with("+++ ") {
            continue;
        }
        if trimmed.contains("exceeded DFT_") {
            return true;
        }
        let Some((path, rest)) = trimmed.split_once(" --- ") else {
            continue;
        };
        if path.trim().is_empty() || rest.trim().is_empty() {
            continue;
        }
        if rest.contains(" --- ") {
            return true;
        }
        let token = rest.split_whitespace().next().unwrap_or("");
        if token.chars().next().is_some_and(|c| c.is_ascii_uppercase()) {
            return true;
        }
    }
    false
}
