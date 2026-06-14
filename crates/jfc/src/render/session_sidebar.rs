use ratatui::{
    Frame,
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem},
};

use crate::app::App;
use crate::theme::Theme;

/// Sessions sidebar — toggled with Ctrl+B. Renders the saved-session metadata
/// from `~/.config/jfc/sessions/` (cached on `App::session_meta` so render()
/// does no disk I/O). Sessions whose `cwd` matches `app.engine.cwd` are shown first
/// under a `── This project ──` separator; everything else (including
/// legacy `cwd: None` entries) lands below `── Other projects ──`.
/// Each row is two lines: title (from `display_title()`) on top, a muted
/// `cwd · time · msgs` badge on bottom. Selecting a row with Enter loads
/// its messages into `App::messages`.
pub(super) fn sidebar(f: &mut Frame, app: &mut App, area: Rect) {
    // Record bounds for the click handler. Borders eat one row top and
    // bottom; the click handler subtracts those before computing the
    // row index.
    *app.sidebar_rect.borrow_mut() = Some(area);
    let t = app.theme;
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(t.style_border)
        .title(Span::styled(" sessions ", t.style_accent_bold))
        .title_bottom(Line::from(Span::styled(" ↑↓ · Enter ", t.style_text_muted)).right_aligned())
        .style(Style::default().bg(t.surface));

    let items: Vec<ListItem> = if app.session_meta.is_empty() {
        vec![ListItem::new(Line::from(Span::styled(
            "  (no saved sessions)",
            Style::default()
                .fg(t.text_muted)
                .add_modifier(Modifier::ITALIC),
        )))]
    } else {
        let now = chrono::Utc::now();
        let cwd = app.engine.cwd.clone();
        let (this_project, other) =
            jfc_session::group_by_cwd(app.session_meta.clone(), Some(cwd.as_str()));

        let mut items: Vec<ListItem> = Vec::new();
        if !this_project.is_empty() {
            items.push(separator_row("── This project ──", t));
            for s in &this_project {
                items.push(session_row(s, app, &now, t));
            }
        }
        if !other.is_empty() {
            items.push(separator_row("── Other projects ──", t));
            for s in &other {
                items.push(session_row(s, app, &now, t));
            }
        }
        // Headers aren't selectable; `visible_selected_row` walks the same
        // grouping to translate `app.session_selected` (session-only index)
        // into a row index that includes the header rows.
        items
    };

    // The sidebar's selection state targets sessions, not header rows.
    // Build a parallel "session-only" mapping by computing the visible index
    // for the currently-selected session. We do this by re-grouping (cheap)
    // and counting headers above the target row.
    let highlight_row = visible_selected_row(app);

    let mut state = ratatui::widgets::ListState::default();
    state.select(highlight_row);

    let list = List::new(items)
        .block(block)
        .highlight_style(
            Style::default()
                .fg(t.bg)
                .bg(t.accent)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("▶ ");
    f.render_stateful_widget(list, area, &mut state);
    // Keep `app.session_list_state` aligned for any code that introspects it
    // (mostly historical; the renderer owns the live state above).
    app.session_list_state.select(highlight_row);
}

/// Return the user-visible session id list, in the order rendered by the
/// sidebar (this-project first, then others). Used by Up/Down/Enter so
/// keyboard navigation matches what the user sees.
pub fn ordered_sidebar_sessions(app: &App) -> Vec<jfc_engine::ids::SessionId> {
    let cwd = app.engine.cwd.clone();
    let (this_project, other) =
        jfc_session::group_by_cwd(app.session_meta.clone(), Some(cwd.as_str()));
    this_project
        .into_iter()
        .chain(other)
        .map(|s| s.id)
        .collect()
}

/// Map the `session_selected` index (which counts sessions, not headers)
/// to the `List`'s row index (which includes separator rows). Returns
/// `None` when there are no sessions yet.
fn visible_selected_row(app: &App) -> Option<usize> {
    if app.session_meta.is_empty() {
        return None;
    }
    let cwd = app.engine.cwd.clone();
    let (this_project, other) =
        jfc_session::group_by_cwd(app.session_meta.clone(), Some(cwd.as_str()));
    let sel = app.session_selected;
    // Rows: [hdr1, this_project..., hdr2, other...]
    if !this_project.is_empty() && sel < this_project.len() {
        // 1 header above the this-project block.
        return Some(1 + sel);
    }
    let sel_in_other = sel - this_project.len();
    let mut row = 0usize;
    if !this_project.is_empty() {
        row += 1 + this_project.len();
    }
    if !other.is_empty() {
        row += 1; // header row
    }
    Some(row + sel_in_other)
}

fn separator_row(label: &str, t: Theme) -> ListItem<'static> {
    ListItem::new(Line::from(Span::styled(
        format!("  {label}"),
        t.style_text_muted.add_modifier(Modifier::DIM),
    )))
}

fn session_row(
    s: &jfc_session::SessionMetadata,
    app: &App,
    now: &chrono::DateTime<chrono::Utc>,
    t: Theme,
) -> ListItem<'static> {
    let is_active = app.engine.current_session_id.as_ref() == Some(&s.id);
    let bullet = if is_active { "▣ " } else { "  " };
    let title = s.display_title();
    let cwd_label = jfc_session::shorten_cwd(s.cwd.as_deref());
    let when = jfc_session::relative_time(s.last_activity(), *now);
    let msgs = format!(
        "{} msg{}",
        s.message_count,
        if s.message_count == 1 { "" } else { "s" }
    );
    let secondary = format!("    {cwd_label} · {when} · {msgs}");

    let title_style = if is_active {
        t.style_accent_bold
    } else {
        t.style_text_primary
    };

    let line1 = Line::from(vec![
        Span::styled(bullet.to_owned(), title_style),
        Span::styled(title, title_style),
    ]);
    let line2 = Line::from(Span::styled(secondary, t.style_text_muted));
    ListItem::new(vec![line1, line2])
}
