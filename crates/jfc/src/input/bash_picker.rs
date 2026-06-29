//! Background-shell picker input handler.
//!
//! Lists running + recently-settled background shells (the `/bashes` roster as a
//! modal). ↑/↓ navigate, `x`/`d` cancel the selected *running* shell, Enter or
//! `o` open its output via `/bashes`, Esc closes. Opened from the Ctrl+X leader
//! chord then `b`.
//!
//! The shell roster lives in the engine (`jfc_engine::tools::list_bash_tasks`,
//! which is async); the modal is sync, so we snapshot the list when the modal
//! opens and on each cancel. Cancellation is routed over the event bus
//! (`ControlEvent::CancelBashTask`) because `cancel_bash_task` is async.

use crossterm::event::KeyCode;
use tokio::sync::mpsc;

use crate::app::App;
use crate::runtime::ControlEvent;
use crate::runtime::{EngineEvent, send_critical};

/// Refresh the roster snapshot and open the modal.
pub(super) async fn open_bash_picker(app: &mut App) {
    let tasks = jfc_engine::tools::list_bash_tasks().await;
    app.bash_picker.open_with_tasks(tasks);
}

fn close_bash_picker(app: &mut App) {
    app.bash_picker.close();
}

pub(super) fn handle_bash_picker_key(
    app: &mut App,
    key: crossterm::event::KeyEvent,
    tx: &mpsc::Sender<EngineEvent>,
) -> bool {
    if !app.bash_picker.visible {
        return false;
    }
    let total = app.bash_picker.tasks.len();
    let current = app.bash_picker.table.selected().unwrap_or(0);
    match key.code {
        KeyCode::Esc | KeyCode::Char('q') => close_bash_picker(app),
        KeyCode::Up | KeyCode::Char('k') if current > 0 => {
            app.bash_picker.table.select(Some(current - 1));
        }
        KeyCode::Down | KeyCode::Char('j') => {
            let max = total.saturating_sub(1);
            if current < max {
                app.bash_picker.table.select(Some(current + 1));
            }
        }
        KeyCode::Home => app.bash_picker.table.select(Some(0)),
        KeyCode::End => app.bash_picker.table.select(Some(total.saturating_sub(1))),
        // Cancel the selected *running* shell.
        KeyCode::Char('x') | KeyCode::Char('d') => cancel_selected(app, current, tx),
        other => {
            tracing::trace!(target: "jfc::bash_picker", ?other, "ignored key in bash picker");
        }
    }
    true
}

/// Dispatch a cancel for the selected row when it's a *running* shell, and
/// optimistically flip the snapshot row so the UI reacts immediately (the real
/// status settles via the toast + the next modal open). No-op for a finished
/// task or an out-of-range index.
fn cancel_selected(app: &mut App, index: usize, tx: &mpsc::Sender<EngineEvent>) {
    let Some(task) = app.bash_picker.tasks.get_mut(index) else {
        return;
    };
    if !task.running {
        return;
    }
    let id = task.id.clone();
    task.running = false;
    task.status = "cancelling…".to_owned();
    send_critical(tx, EngineEvent::Control(ControlEvent::CancelBashTask(id)));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::App;
    use jfc_engine::tools::BashTaskSnapshot;
    use jfc_provider::{EventStream, ModelInfo, Provider, ProviderMessage, StreamOptions};
    use std::sync::Arc;

    struct TestProvider;
    impl jfc_provider::seal::Sealed for TestProvider {}
    #[async_trait::async_trait]
    impl Provider for TestProvider {
        fn name(&self) -> &str {
            "test"
        }
        fn available_models(&self) -> Vec<ModelInfo> {
            Vec::new()
        }
        async fn stream(
            &self,
            _m: Vec<ProviderMessage>,
            _o: &StreamOptions,
        ) -> anyhow::Result<EventStream> {
            Ok(Box::pin(futures::stream::empty()))
        }
    }

    fn snap(id: &str, running: bool) -> BashTaskSnapshot {
        BashTaskSnapshot {
            id: id.into(),
            command: "sleep 60".into(),
            cwd: std::path::PathBuf::from("/tmp"),
            output_path: std::path::PathBuf::from("/tmp/x.log"),
            status: if running {
                "running"
            } else {
                "completed exit=0"
            }
            .into(),
            running,
            started_at_ms: 0,
            completed_at_ms: None,
            total_bytes: 0,
            total_lines: 0,
        }
    }

    fn app_with_tasks(tasks: Vec<BashTaskSnapshot>) -> App {
        let mut app = App::new(Arc::new(TestProvider), "test-model");
        app.bash_picker.open_with_tasks(tasks);
        app
    }

    fn key(code: KeyCode) -> crossterm::event::KeyEvent {
        crossterm::event::KeyEvent::new(code, crossterm::event::KeyModifiers::NONE)
    }

    #[test]
    fn bash_picker_arrow_navigation_clamps_normal() {
        let mut app = app_with_tasks(vec![snap("bash_a", true), snap("bash_b", false)]);
        let (tx, _rx) = mpsc::channel::<EngineEvent>(8);

        // Down moves to 1, then clamps.
        assert!(handle_bash_picker_key(&mut app, key(KeyCode::Down), &tx));
        assert_eq!(app.bash_picker.table.selected(), Some(1));
        handle_bash_picker_key(&mut app, key(KeyCode::Down), &tx);
        assert_eq!(app.bash_picker.table.selected(), Some(1), "clamps at end");

        // Up moves back to 0, then clamps.
        handle_bash_picker_key(&mut app, key(KeyCode::Up), &tx);
        assert_eq!(app.bash_picker.table.selected(), Some(0));
        handle_bash_picker_key(&mut app, key(KeyCode::Up), &tx);
        assert_eq!(app.bash_picker.table.selected(), Some(0), "clamps at start");
    }

    #[test]
    fn bash_picker_cancel_dispatches_for_running_only_normal() {
        let mut app = app_with_tasks(vec![snap("bash_run", true)]);
        let (tx, mut rx) = mpsc::channel::<EngineEvent>(8);

        assert!(handle_bash_picker_key(
            &mut app,
            key(KeyCode::Char('x')),
            &tx
        ));
        // A cancel control event was dispatched for the running task.
        let got = rx.try_recv();
        assert!(
            matches!(
                &got,
                Ok(EngineEvent::Control(ControlEvent::CancelBashTask(id))) if id == "bash_run"
            ),
            "expected CancelBashTask(bash_run)"
        );
        // The row is optimistically flipped to not-running.
        assert!(!app.bash_picker.tasks[0].running);
    }

    #[test]
    fn bash_picker_cancel_noop_on_finished_task_robust() {
        let mut app = app_with_tasks(vec![snap("bash_done", false)]);
        let (tx, mut rx) = mpsc::channel::<EngineEvent>(8);

        handle_bash_picker_key(&mut app, key(KeyCode::Char('x')), &tx);
        assert!(
            rx.try_recv().is_err(),
            "no cancel for an already-finished task"
        );
    }

    #[test]
    fn bash_picker_esc_closes_normal() {
        let mut app = app_with_tasks(vec![snap("bash_a", true)]);
        let (tx, _rx) = mpsc::channel::<EngineEvent>(8);
        handle_bash_picker_key(&mut app, key(KeyCode::Esc), &tx);
        assert!(!app.bash_picker.visible);
    }
}
