use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use tokio::sync::mpsc;

use crate::app::App;
use crate::runtime::EngineEvent;

use super::runtime_action_widgets::{InfoSidebarWidgetFocusStep, move_info_sidebar_widget_focus};

pub(super) async fn handle_focused_widget_key(
    app: &mut App,
    key: KeyEvent,
    tx: &mpsc::Sender<EngineEvent>,
) -> Option<anyhow::Result<bool>> {
    if let Some(result) = super::focused_panels::handle_focused_panel_key(app, key, tx).await {
        return Some(result);
    }
    if !app.info_sidebar.visible || key.modifiers != KeyModifiers::ALT {
        return None;
    }
    match key.code {
        KeyCode::Left => {
            move_info_sidebar_widget_focus(app, InfoSidebarWidgetFocusStep::Previous);
            Some(Ok(false))
        }
        KeyCode::Right => {
            move_info_sidebar_widget_focus(app, InfoSidebarWidgetFocusStep::Next);
            Some(Ok(false))
        }
        KeyCode::Enter => {
            super::runtime_action_router::execute_focused_info_sidebar_widget_action(app, tx).await;
            Some(Ok(false))
        }
        _ => None,
    }
}
