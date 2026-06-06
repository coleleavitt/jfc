use std::io;

use crossterm::{
    execute,
    terminal::{BeginSynchronizedUpdate, EndSynchronizedUpdate, SetTitle},
};
use ratatui::{Terminal, backend::CrosstermBackend};

use crate::{app::App, render};

pub(crate) fn draw_synchronized(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    app: &mut App,
) -> io::Result<()> {
    let _ = execute!(io::stdout(), BeginSynchronizedUpdate);
    let result = terminal.draw(|frame| render::frame(frame, app));
    let _ = execute!(io::stdout(), EndSynchronizedUpdate);
    result.map(|_| ())
}

pub(crate) async fn read_git_branch_from_root(git_root: &std::path::Path) -> Option<String> {
    let head = git_root.join(".git/HEAD");
    if let Ok(content) = tokio::fs::read_to_string(&head).await {
        let trimmed = content.trim();
        if let Some(rest) = trimmed.strip_prefix("ref: refs/heads/") {
            return Some(rest.to_owned());
        }
        return Some("(detached)".to_owned());
    }
    None
}

pub(crate) fn set_terminal_title(app: &App) {
    use std::sync::{Mutex, OnceLock};

    static LAST: OnceLock<Mutex<String>> = OnceLock::new();
    let last = LAST.get_or_init(|| Mutex::new(String::new()));
    let cwd_label = std::path::Path::new(app.cwd.as_str())
        .file_name()
        .and_then(|name| name.to_str())
        .map(str::to_owned)
        .unwrap_or_else(|| app.cwd.clone());
    let lines_below = app
        .total_lines
        .saturating_sub(app.scroll_offset + app.viewport_height);
    let prefix = if !app.follow_bottom && lines_below > 0 {
        format!("({lines_below} new) ")
    } else if app.is_streaming {
        "● ".to_owned()
    } else {
        String::new()
    };
    let title = format!("{}jfc · {} · {}", prefix, app.model, cwd_label);
    let mut guard = match last.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    };
    if *guard == title {
        return;
    }
    *guard = title.clone();
    let _ = execute!(io::stdout(), SetTitle(title));
}
