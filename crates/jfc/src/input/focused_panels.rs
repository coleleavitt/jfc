use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use tokio::sync::mpsc;

use crate::app::App;
use crate::runtime::EngineEvent;

use super::runtime_action_panels::{InfoSidebarPanelFocusStep, move_info_sidebar_panel_focus};

pub(super) async fn handle_focused_panel_key(
    app: &mut App,
    key: KeyEvent,
    tx: &mpsc::Sender<EngineEvent>,
) -> Option<anyhow::Result<bool>> {
    if !app.info_sidebar.visible || key.modifiers != KeyModifiers::ALT {
        return None;
    }
    match key.code {
        KeyCode::Up => {
            move_info_sidebar_panel_focus(app, InfoSidebarPanelFocusStep::Previous);
            Some(Ok(false))
        }
        KeyCode::Down => {
            move_info_sidebar_panel_focus(app, InfoSidebarPanelFocusStep::Next);
            Some(Ok(false))
        }
        KeyCode::Enter if app.info_sidebar.focused_panel.is_some() => {
            super::runtime_action_router::execute_focused_info_sidebar_panel_action(app, tx).await;
            Some(Ok(false))
        }
        _ => None,
    }
}
