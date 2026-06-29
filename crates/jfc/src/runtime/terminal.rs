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
    let message_count = usize_to_u64_saturating(app.engine.messages.len());
    let total_lines = usize_to_u64_saturating(app.total_lines);
    let _linkscope_draw = linkscope::phase("ui.draw");
    let _linkscope_draw_trace = linkscope::trace_fields(
        "ui.draw",
        [
            linkscope::TraceField::count("messages", message_count),
            linkscope::TraceField::count("total_lines", total_lines),
        ],
    );
    linkscope::record_items("ui.draw", 1);
    {
        let _linkscope_begin = linkscope::phase("ui.draw.begin_synchronized_update");
        let _ = execute!(io::stdout(), BeginSynchronizedUpdate);
    }
    let result = {
        let _linkscope_render = linkscope::phase("ui.draw.render_frame");
        terminal.draw(|frame| render::frame(frame, app))
    };
    {
        let _linkscope_end = linkscope::phase("ui.draw.end_synchronized_update");
        let _ = execute!(io::stdout(), EndSynchronizedUpdate);
    }
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
    let cwd_label = std::path::Path::new(app.engine.cwd.as_str())
        .file_name()
        .and_then(|name| name.to_str())
        .map(str::to_owned)
        .unwrap_or_else(|| app.engine.cwd.clone());
    let lines_below = app
        .total_lines
        .saturating_sub(app.scroll_offset + app.viewport_height);
    let prefix = if !app.follow_bottom && lines_below > 0 {
        format!("({lines_below} new) ")
    } else if app.engine.is_streaming {
        "● ".to_owned()
    } else {
        String::new()
    };
    let title = format!("{}jfc · {} · {}", prefix, app.engine.model, cwd_label);
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

fn usize_to_u64_saturating(value: usize) -> u64 {
    u64::try_from(value).unwrap_or(u64::MAX)
}
