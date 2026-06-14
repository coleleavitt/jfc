use std::{sync::Arc, time::Duration};

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

use super::navigation::{scan_path_refs, user_prompts};
use super::*;
use crate::app::App;
use crate::runtime::{EngineEvent, ToolEvent};
use jfc_core::*;
use jfc_provider::{EventStream, ModelInfo, Provider, ProviderMessage, StreamOptions};

struct TestProvider;

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
        _messages: Vec<ProviderMessage>,
        _options: &StreamOptions,
    ) -> anyhow::Result<EventStream> {
        Ok(Box::pin(futures::stream::empty()))
    }
}
impl jfc_provider::seal::Sealed for TestProvider {}

struct StaticModelProvider;

#[async_trait::async_trait]
impl Provider for StaticModelProvider {
    fn name(&self) -> &str {
        "static"
    }

    fn available_models(&self) -> Vec<ModelInfo> {
        vec![ModelInfo::new("static-model", "Static Model", "static")]
    }

    async fn stream(
        &self,
        _messages: Vec<ProviderMessage>,
        _options: &StreamOptions,
    ) -> anyhow::Result<EventStream> {
        Ok(Box::pin(futures::stream::empty()))
    }
}
impl jfc_provider::seal::Sealed for StaticModelProvider {}

/// Test fixture: a fresh `App` plus a paired `(tx, rx)` so tests can both
/// drive `handle_key` and inspect the AppEvents it emits. Pulled out so
/// the dozens of tests below don't repeat the boilerplate.
fn test_app() -> App {
    let mut app = App::new(Arc::new(TestProvider), "test-model");
    app.engine.task_store = jfc_session::TaskStore::in_memory();
    app
}

fn test_app_with_input(input: &str, wrap_width: usize) -> App {
    let mut app = test_app();
    app.input_wrap_width = wrap_width;
    app.textarea = TextArea::from(input.lines().map(str::to_string).collect::<Vec<_>>());
    app
}

fn channel() -> (
    tokio::sync::mpsc::Sender<EngineEvent>,
    tokio::sync::mpsc::Receiver<EngineEvent>,
) {
    tokio::sync::mpsc::channel(1024)
}

struct TemperatureGlobalGuard;

impl Drop for TemperatureGlobalGuard {
    fn drop(&mut self) {
        jfc_engine::exploration::set_temperature_global(None);
        jfc_engine::exploration::set_exploration_level_global(None);
    }
}

/// Build a minimal `ToolCall` of the requested kind. The status defaults
/// to `Pending` so tests can drive it through the approval lifecycle
/// without preseeding extra state.
#[tracing::instrument(level = "trace", skip_all)]
fn make_tool(id: &str, kind: ToolKind) -> ToolCall {
    let input = match &kind {
        ToolKind::Bash => ToolInput::Bash {
            command: "ls".into(),
            timeout: None,
            workdir: None,
            run_in_background: None,
        },
        ToolKind::Read => ToolInput::Read {
            file_path: "x".into(),
            offset: None,
            limit: None,
        },
        _ => ToolInput::Generic {
            summary: "tool".into(),
        },
    };
    ToolCall {
        id: id.into(),
        kind,
        status: ToolStatus::Pending,
        input,
        output: ToolOutput::Empty,
        display: jfc_core::ToolDisplayState::DEFAULT,
        elapsed_ms: None,
        started_at: None,
        thought_signature: None,
    }
}

fn make_bash_tool(id: &str, command: &str) -> ToolCall {
    ToolCall {
        id: id.into(),
        kind: ToolKind::Bash,
        status: ToolStatus::Pending,
        input: ToolInput::Bash {
            command: command.into(),
            timeout: None,
            workdir: None,
            run_in_background: None,
        },
        output: ToolOutput::Empty,
        display: jfc_core::ToolDisplayState::DEFAULT,
        elapsed_ms: None,
        started_at: None,
        thought_signature: None,
    }
}

fn make_background_task(
    id: &str,
    description: &str,
    status: TaskLifecycle,
) -> crate::app::BackgroundTask {
    crate::app::BackgroundTask {
        task_id: id.into(),
        description: description.into(),
        status,
        started_at: std::time::Instant::now(),
        completed_at: status.is_terminal().then_some(std::time::Instant::now()),
        summary: None,
        error: None,
        last_tool: None,
        messages: Vec::new(),
        chat_messages: Vec::new(),
        tool_use_count: 0,
        latest_input_tokens: 0,
        latest_cache_read_tokens: 0,
        latest_cache_write_tokens: 0,
        cumulative_output_tokens: 0,
        model_used: None,
        agent_messages: Vec::new(),
        max_input_tokens: None,
        budget_killed: false,
        parent_task_id: None,
        workflow_progress: None,
        last_activity_at: std::time::Instant::now(),
    }
}

/// Convenience to send a single keypress (NONE modifier).
fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

fn key_mod(code: KeyCode, mods: KeyModifiers) -> KeyEvent {
    KeyEvent::new(code, mods)
}

fn key_repeat(code: KeyCode) -> KeyEvent {
    KeyEvent::new_with_kind(code, KeyModifiers::NONE, KeyEventKind::Repeat)
}

// ─────────────────────────────────────────────────────────────────────
// Pure helpers
// ─────────────────────────────────────────────────────────────────────

#[test]
fn input_has_text_normal() {
    let app = test_app_with_input("hi", 80);
    assert!(input_has_text(&app));
}

#[test]
fn input_has_text_robust_empty() {
    let app = test_app();
    assert!(!input_has_text(&app));
}

#[test]
fn input_has_text_robust_only_newlines() {
    // A textarea with multiple empty rows should still report as empty.
    let mut app = test_app();
    app.textarea = TextArea::from(vec![String::new(), String::new()]);
    assert!(!input_has_text(&app));
}

#[test]
fn cursor_move_visual_up_within_wrap_normal() {
    let mut app = test_app_with_input("abcdefghij", 5);
    app.textarea.move_cursor(CursorMove::Jump(0, 7));
    move_input_cursor_visual_up(&mut app);
    assert_eq!(app.textarea.cursor(), (0, 2));
}

#[test]
fn cursor_move_visual_up_jumps_to_head_when_first_line_robust() {
    let mut app = test_app_with_input("abc", 80);
    app.textarea.move_cursor(CursorMove::Jump(0, 2));
    move_input_cursor_visual_up(&mut app);
    assert_eq!(app.textarea.cursor(), (0, 0));
}

#[test]
fn cursor_move_visual_down_jumps_to_end_when_last_line_robust() {
    let mut app = test_app_with_input("abc", 80);
    app.textarea.move_cursor(CursorMove::Jump(0, 1));
    move_input_cursor_visual_down(&mut app);
    assert_eq!(app.textarea.cursor(), (0, 3));
}

#[test]
fn user_prompts_collects_chronologically_normal() {
    let mut app = test_app();
    app.engine.messages.push(ChatMessage::user("first".into()));
    app.engine
        .messages
        .push(ChatMessage::assistant("hi".into()));
    app.engine.messages.push(ChatMessage::user("second".into()));
    let prompts = user_prompts(&app);
    assert_eq!(prompts, vec!["first".to_string(), "second".to_string()]);
}

#[test]
fn user_prompts_skips_empty_robust() {
    let mut app = test_app();
    app.engine.messages.push(ChatMessage::user(String::new()));
    let prompts = user_prompts(&app);
    assert!(prompts.is_empty());
}

#[test]
fn recall_previous_prompt_walks_back_normal() {
    let mut app = test_app();
    app.engine.messages.push(ChatMessage::user("a".into()));
    app.engine.messages.push(ChatMessage::user("b".into()));
    // First press: most recent
    let p1 = recall_previous_prompt(&mut app);
    assert_eq!(p1.as_deref(), Some("b"));
    // Second press: older
    let p2 = recall_previous_prompt(&mut app);
    assert_eq!(p2.as_deref(), Some("a"));
    // Third: stop at oldest
    let p3 = recall_previous_prompt(&mut app);
    assert!(p3.is_none());
}

#[test]
fn recall_previous_prompt_robust_empty_history() {
    let mut app = test_app();
    assert!(recall_previous_prompt(&mut app).is_none());
}

#[test]
fn recall_next_prompt_walks_forward_normal() {
    let mut app = test_app();
    app.engine.messages.push(ChatMessage::user("a".into()));
    app.engine.messages.push(ChatMessage::user("b".into()));
    let _ = recall_previous_prompt(&mut app);
    let _ = recall_previous_prompt(&mut app);
    // Now cursor is at index 0 ("a"); forward → "b"
    let next = recall_next_prompt(&mut app);
    assert_eq!(next.as_deref(), Some("b"));
}

#[test]
fn recall_next_prompt_robust_returns_none_at_end() {
    let mut app = test_app();
    app.engine.messages.push(ChatMessage::user("only".into()));
    let _ = recall_previous_prompt(&mut app);
    // Already at most-recent → next should clear cursor and return None
    assert!(recall_next_prompt(&mut app).is_none());
    assert!(app.history_cursor.is_none());
}

#[test]
fn scan_path_refs_normal() {
    let v = scan_path_refs("see src/lib.rs:42:5 and Cargo.toml:7 here");
    assert!(v.iter().any(|s| s == "src/lib.rs:42:5"));
    assert!(v.iter().any(|s| s == "Cargo.toml:7"));
}

#[test]
fn scan_path_refs_rejects_url_and_pure_numbers_robust() {
    // `12:34` is a pure-number colon-pair — must be rejected. Direct
    // URL strings starting with `http://` / `https://` are also
    // rejected by the top-level guard.
    let v = scan_path_refs("foo 12:34 https://example.com:80/x");
    assert!(!v.iter().any(|s| s == "12:34"));
    assert!(!v.iter().any(|s| s.starts_with("http")));
}

#[test]
fn collect_recent_paths_dedups_normal() {
    let msg = ChatMessage::assistant_parts(vec![MessagePart::tool(ToolCall {
        id: "t1".into(),
        kind: ToolKind::Bash,
        status: ToolStatus::Completed,
        input: ToolInput::Bash {
            command: "echo".into(),
            timeout: None,
            workdir: None,
            run_in_background: None,
        },
        output: ToolOutput::Command {
            stdout: "src/lib.rs:1 and src/lib.rs:1".into(),
            stderr: String::new(),
            exit_code: Some(0),
        },
        display: jfc_core::ToolDisplayState::DEFAULT,
        elapsed_ms: None,
        started_at: None,
        thought_signature: None,
    })]);
    let paths = collect_recent_paths(&[msg]);
    assert_eq!(paths.len(), 1);
    assert_eq!(paths[0], "src/lib.rs:1");
}

// ─────────────────────────────────────────────────────────────────────
// Existing soft-wrap tests
// ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn up_and_down_move_across_soft_wrapped_input_rows() {
    let mut app = test_app_with_input("abcdefghij", 5);
    app.textarea.move_cursor(CursorMove::Jump(0, 8));
    let (tx, _rx) = channel();

    handle_key(&mut app, key(KeyCode::Up), &tx).await.unwrap();
    assert_eq!(app.textarea.cursor(), (0, 3));

    handle_key(&mut app, key(KeyCode::Down), &tx).await.unwrap();
    assert_eq!(app.textarea.cursor(), (0, 8));
}

#[tokio::test]
async fn up_and_down_still_cross_logical_input_lines() {
    let mut app = test_app_with_input("abc\ndefghijkl", 5);
    app.textarea.move_cursor(CursorMove::Jump(0, 2));
    let (tx, _rx) = channel();

    handle_key(&mut app, key(KeyCode::Down), &tx).await.unwrap();
    assert_eq!(app.textarea.cursor(), (1, 2));
    handle_key(&mut app, key(KeyCode::Down), &tx).await.unwrap();
    assert_eq!(app.textarea.cursor(), (1, 7));
    handle_key(&mut app, key(KeyCode::Up), &tx).await.unwrap();
    assert_eq!(app.textarea.cursor(), (1, 2));
}

// ─────────────────────────────────────────────────────────────────────
// Approval modal
// ─────────────────────────────────────────────────────────────────────

fn arm_approval(app: &mut App, kind: ToolKind) {
    app.engine.pending_approval = Some(crate::app::PendingApproval {
        tool: make_tool("t1", kind),
        selected: 0,
    });
}

#[tokio::test]
async fn approval_y_dispatches_and_clears_normal() {
    let mut app = test_app();
    arm_approval(&mut app, ToolKind::Bash);
    let (tx, _rx) = channel();
    handle_key(&mut app, key(KeyCode::Char('y')), &tx)
        .await
        .unwrap();
    assert!(app.engine.pending_approval.is_none());
}

#[tokio::test]
async fn approval_y_does_not_emit_all_complete_before_tool_finishes_robust() {
    let mut app = test_app();
    app.engine.pending_approval = Some(crate::app::PendingApproval {
        tool: make_bash_tool("t1", "sleep 1; echo done"),
        selected: 0,
    });
    let (tx, mut rx) = channel();

    handle_key(&mut app, key(KeyCode::Char('y')), &tx)
        .await
        .unwrap();

    let event = tokio::time::timeout(Duration::from_millis(50), rx.recv()).await;
    assert!(
        !matches!(event, Ok(Some(EngineEvent::Tool(ToolEvent::AllComplete)))),
        "approval injected AllComplete before the dispatched tool completed"
    );
}

#[tokio::test]
async fn approval_n_denies_normal() {
    let mut app = test_app();
    arm_approval(&mut app, ToolKind::Bash);
    let (tx, _rx) = channel();
    handle_key(&mut app, key(KeyCode::Char('n')), &tx)
        .await
        .unwrap();
    assert!(app.engine.pending_approval.is_none());
}

#[tokio::test]
async fn approval_n_last_tool_emits_all_complete_robust() {
    let mut app = test_app();
    arm_approval(&mut app, ToolKind::Bash);
    let (tx, mut rx) = channel();

    handle_key(&mut app, key(KeyCode::Char('n')), &tx)
        .await
        .unwrap();

    let event = rx.recv().await;
    assert!(matches!(
        event,
        Some(EngineEvent::Tool(ToolEvent::AllComplete))
    ));
}

#[tokio::test]
async fn approval_a_promotes_always_normal() {
    let mut app = test_app();
    arm_approval(&mut app, ToolKind::Bash);
    let (tx, _rx) = channel();
    handle_key(&mut app, key(KeyCode::Char('a')), &tx)
        .await
        .unwrap();
    assert!(app.engine.always_approved.iter().any(|n| n == "Bash"));
}

#[tokio::test]
async fn approval_a_batches_following_auto_approved_tools_robust() {
    let mut app = test_app();
    app.engine.pending_approval = Some(crate::app::PendingApproval {
        tool: make_bash_tool("t1", "sleep 1; echo slow"),
        selected: 0,
    });
    app.engine
        .approval_queue
        .push_back(make_bash_tool("t2", "echo quick"));
    let (tx, mut rx) = channel();

    handle_key(&mut app, key(KeyCode::Char('a')), &tx)
        .await
        .unwrap();

    let event = tokio::time::timeout(Duration::from_millis(50), rx.recv()).await;
    assert!(
        !matches!(event, Ok(Some(EngineEvent::Tool(ToolEvent::AllComplete)))),
        "auto-approved queued tools were dispatched as a separate early-completing batch"
    );
}

#[tokio::test]
async fn approval_s_promotes_session_normal() {
    let mut app = test_app();
    arm_approval(&mut app, ToolKind::Bash);
    let (tx, _rx) = channel();
    handle_key(&mut app, key(KeyCode::Char('s')), &tx)
        .await
        .unwrap();
    assert!(app.engine.session_approved.iter().any(|n| n == "Bash"));
}

#[tokio::test]
async fn approval_arrows_move_selection_normal() {
    let mut app = test_app();
    arm_approval(&mut app, ToolKind::Bash);
    let (tx, _rx) = channel();
    handle_key(&mut app, key(KeyCode::Down), &tx).await.unwrap();
    assert_eq!(app.engine.pending_approval.as_ref().unwrap().selected, 1);
    handle_key(&mut app, key(KeyCode::Up), &tx).await.unwrap();
    assert_eq!(app.engine.pending_approval.as_ref().unwrap().selected, 0);
}

#[tokio::test]
async fn approval_enter_uses_selected_choice_normal() {
    let mut app = test_app();
    arm_approval(&mut app, ToolKind::Bash);
    // selected = 1 → No
    app.engine.pending_approval.as_mut().unwrap().selected = 1;
    let (tx, _rx) = channel();
    handle_key(&mut app, key(KeyCode::Enter), &tx)
        .await
        .unwrap();
    assert!(app.engine.pending_approval.is_none());
}

#[tokio::test]
async fn approval_esc_clears_queue_robust() {
    let mut app = test_app();
    arm_approval(&mut app, ToolKind::Bash);
    app.engine
        .approval_queue
        .push_back(make_tool("t2", ToolKind::Bash));
    let (tx, mut rx) = channel();
    handle_key(&mut app, key(KeyCode::Esc), &tx).await.unwrap();
    assert!(app.engine.pending_approval.is_none());
    assert!(app.engine.approval_queue.is_empty());
    let event = rx.recv().await;
    assert!(matches!(
        event,
        Some(EngineEvent::Tool(ToolEvent::AllComplete))
    ));
}

#[tokio::test]
async fn approval_ctrl_c_interrupts_instead_of_being_swallowed_robust() {
    let mut app = test_app();
    app.engine.is_streaming = true;
    arm_approval(&mut app, ToolKind::Bash);
    app.engine
        .approval_queue
        .push_back(make_tool("t2", ToolKind::Bash));
    let (tx, mut rx) = channel();

    let exit = handle_key(
        &mut app,
        key_mod(KeyCode::Char('c'), KeyModifiers::CONTROL),
        &tx,
    )
    .await
    .unwrap();

    assert!(!exit);
    assert!(app.engine.pending_approval.is_none());
    assert!(app.engine.approval_queue.is_empty());
    assert!(
        app.engine
            .interrupt_flag
            .load(std::sync::atomic::Ordering::SeqCst),
        "Ctrl+C in approval modal must still request an active turn interrupt"
    );
    assert!(
        !app.engine.cancel_token.is_cancelled(),
        "App should mint a fresh token after cancelling the active turn clone"
    );
    assert!(matches!(
        rx.recv().await,
        Some(EngineEvent::Tool(ToolEvent::AllComplete))
    ));
}

#[tokio::test]
async fn remote_approval_matches_current_tool_id_normal() {
    let mut app = test_app();
    app.engine.pending_approval = Some(crate::app::PendingApproval {
        tool: make_bash_tool("t1", "echo ok"),
        selected: 0,
    });
    let (tx, mut rx) = channel();

    crate::runtime::approvals::handle_remote_approval_response(
        &mut app.engine,
        &tx,
        "t1".into(),
        true,
    );

    assert!(app.engine.pending_approval.is_none());
    let event = rx.recv().await;
    assert!(matches!(
        event,
        Some(EngineEvent::Tool(ToolEvent::SetInProgressToolUseIds { action, ids }))
            if action == "add" && ids == vec!["t1".to_owned()]
    ));
}

#[tokio::test]
async fn remote_orphaned_permission_response_recovers_unresolved_tool_robust() {
    let mut app = test_app();
    let tool = make_tool("t_orphan", ToolKind::Bash);
    app.engine.messages.push(ChatMessage::user("run".into()));
    app.engine
        .messages
        .push(ChatMessage::assistant_parts(vec![MessagePart::tool(tool)]));
    let (tx, _rx) = channel();

    crate::runtime::approvals::handle_remote_approval_response(
        &mut app.engine,
        &tx,
        "t_orphan".into(),
        false,
    );

    let status = app
        .engine
        .messages
        .iter()
        .flat_map(|msg| msg.parts.iter())
        .find_map(|part| match part {
            MessagePart::Tool(tool) if tool.id.as_str() == "t_orphan" => Some(tool.status),
            _ => None,
        })
        .expect("tool should remain in transcript");
    assert_eq!(status, ToolStatus::Failed);
}

// ─────────────────────────────────────────────────────────────────────
// Task panel modal
// ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn task_panel_esc_closes_normal() {
    let mut app = test_app();
    app.show_task_panel = true;
    let (tx, _rx) = channel();
    handle_key(&mut app, key(KeyCode::Esc), &tx).await.unwrap();
    assert!(!app.show_task_panel);
}

#[tokio::test]
async fn task_panel_arrows_robust_no_tasks() {
    let mut app = test_app();
    app.show_task_panel = true;
    let (tx, _rx) = channel();
    handle_key(&mut app, key(KeyCode::Down), &tx).await.unwrap();
    handle_key(&mut app, key(KeyCode::Up), &tx).await.unwrap();
    assert_eq!(app.task_panel_selected, 0);
}

#[tokio::test]
async fn task_panel_ctrl_t_cycles_to_teammates_when_agents_exist_regression() {
    let mut app = test_app();
    app.show_task_panel = true;
    app.expanded_view = crate::app::ExpandedView::Tasks;
    app.engine.background_tasks.insert(
        "agent-1".into(),
        make_background_task("agent-1", "inspect ui", TaskLifecycle::Running),
    );
    let (tx, _rx) = channel();

    handle_key(
        &mut app,
        key_mod(KeyCode::Char('t'), KeyModifiers::CONTROL),
        &tx,
    )
    .await
    .unwrap();

    assert!(!app.show_task_panel);
    assert_eq!(app.expanded_view, crate::app::ExpandedView::Teammates);
}

#[tokio::test]
async fn teammates_panel_ctrl_t_closes_regression() {
    let mut app = test_app();
    app.expanded_view = crate::app::ExpandedView::Teammates;
    let (tx, _rx) = channel();

    handle_key(
        &mut app,
        key_mod(KeyCode::Char('t'), KeyModifiers::CONTROL),
        &tx,
    )
    .await
    .unwrap();

    assert_eq!(app.expanded_view, crate::app::ExpandedView::None);
}

#[tokio::test]
async fn teammates_panel_down_selects_agent_normal() {
    let mut app = test_app();
    app.expanded_view = crate::app::ExpandedView::Teammates;
    app.engine.background_tasks.insert(
        "agent-1".into(),
        make_background_task("agent-1", "inspect ui", TaskLifecycle::Running),
    );
    let (tx, _rx) = channel();

    handle_key(&mut app, key(KeyCode::Down), &tx).await.unwrap();

    assert_eq!(app.viewing_task_id.as_deref(), Some("agent-1"));
}

// ─────────────────────────────────────────────────────────────────────
// Sidebar (Ctrl+B)
// ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn ctrl_b_toggles_sidebar_normal() {
    let mut app = test_app();
    let (tx, _rx) = channel();
    handle_key(
        &mut app,
        key_mod(KeyCode::Char('b'), KeyModifiers::CONTROL),
        &tx,
    )
    .await
    .unwrap();
    assert!(app.show_sidebar);
    handle_key(
        &mut app,
        key_mod(KeyCode::Char('b'), KeyModifiers::CONTROL),
        &tx,
    )
    .await
    .unwrap();
    assert!(!app.show_sidebar);
}

#[tokio::test]
async fn sidebar_arrows_consumed_robust() {
    let mut app = test_app();
    app.show_sidebar = true;
    let (tx, _rx) = channel();
    handle_key(&mut app, key(KeyCode::Down), &tx).await.unwrap();
    handle_key(&mut app, key(KeyCode::Up), &tx).await.unwrap();
    // No sessions exist → selected stays at 0
    assert_eq!(app.session_selected, 0);
}

// ─────────────────────────────────────────────────────────────────────
// Palette (Ctrl+P)
// ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn ctrl_p_opens_palette_normal() {
    let mut app = test_app();
    let (tx, _rx) = channel();
    handle_key(
        &mut app,
        key_mod(KeyCode::Char('p'), KeyModifiers::CONTROL),
        &tx,
    )
    .await
    .unwrap();
    assert!(app.show_palette);
    assert_eq!(app.palette_selected, 0);
}

#[tokio::test]
async fn palette_typing_filters_normal() {
    let mut app = test_app();
    app.show_palette = true;
    let (tx, _rx) = channel();
    handle_key(&mut app, key(KeyCode::Char('c')), &tx)
        .await
        .unwrap();
    assert_eq!(app.palette_input, "c");
    handle_key(&mut app, key(KeyCode::Backspace), &tx)
        .await
        .unwrap();
    assert_eq!(app.palette_input, "");
}

#[tokio::test]
async fn palette_arrows_change_selection_normal() {
    let mut app = test_app();
    app.show_palette = true;
    let (tx, _rx) = channel();
    handle_key(&mut app, key(KeyCode::Down), &tx).await.unwrap();
    assert_eq!(app.palette_selected, 1);
    handle_key(&mut app, key(KeyCode::Up), &tx).await.unwrap();
    assert_eq!(app.palette_selected, 0);
}

#[tokio::test]
async fn palette_home_end_jump_selection_robust() {
    let mut app = test_app();
    app.show_palette = true;
    let (tx, _rx) = channel();
    // End jumps to the last item; Home back to the first. Parity with the
    // theme/model/session pickers, which already support these keys.
    handle_key(&mut app, key(KeyCode::End), &tx).await.unwrap();
    let last = palette_items(&app).len().saturating_sub(1);
    assert!(last > 0, "fixture should have multiple palette items");
    assert_eq!(app.palette_selected, last);
    handle_key(&mut app, key(KeyCode::Home), &tx).await.unwrap();
    assert_eq!(app.palette_selected, 0);
}

#[tokio::test]
async fn palette_esc_closes_robust() {
    let mut app = test_app();
    app.show_palette = true;
    app.palette_input = "x".into();
    let (tx, _rx) = channel();
    handle_key(&mut app, key(KeyCode::Esc), &tx).await.unwrap();
    assert!(!app.show_palette);
    assert!(app.palette_input.is_empty());
}

#[tokio::test]
async fn palette_enter_executes_action_normal() {
    let mut app = test_app();
    app.show_palette = true;
    // First palette item: "Clear Messages (/clear)"
    let (tx, _rx) = channel();
    app.engine.messages.push(ChatMessage::user("hi".into()));
    handle_key(&mut app, key(KeyCode::Enter), &tx)
        .await
        .unwrap();
    assert!(!app.show_palette);
    // /clear via palette wipes messages
    assert!(app.engine.messages.is_empty());
}

// ─────────────────────────────────────────────────────────────────────
// Model picker (Ctrl+M)
// ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn ctrl_m_opens_model_picker_normal() {
    let mut app = test_app();
    let (tx, _rx) = channel();
    handle_key(
        &mut app,
        key_mod(KeyCode::Char('m'), KeyModifiers::CONTROL),
        &tx,
    )
    .await
    .unwrap();
    assert!(app.show_model_picker);
}

#[test]
fn collect_all_models_empty_cache_falls_back_to_static_robust() {
    let mut app = App::new(Arc::new(StaticModelProvider), "static-model");
    app.engine
        .provider_models
        .insert(jfc_provider::ProviderId::from("static"), Vec::new());

    let models = collect_all_models(&app);

    assert_eq!(models.len(), 1);
    assert_eq!(models[0].id.as_str(), "static-model");
    assert_eq!(models[0].provider.as_str(), "static");
}

#[tokio::test]
async fn model_picker_esc_closes_robust() {
    let mut app = test_app();
    app.show_model_picker = true;
    app.model_picker_filter = "x".into();
    let (tx, _rx) = channel();
    handle_key(&mut app, key(KeyCode::Esc), &tx).await.unwrap();
    assert!(!app.show_model_picker);
    assert!(app.model_picker_filter.is_empty());
}

#[tokio::test]
async fn model_picker_typing_appends_filter_normal() {
    let mut app = test_app();
    app.show_model_picker = true;
    let (tx, _rx) = channel();
    handle_key(&mut app, key(KeyCode::Char('o')), &tx)
        .await
        .unwrap();
    assert_eq!(app.model_picker_filter, "o");
    handle_key(&mut app, key(KeyCode::Backspace), &tx)
        .await
        .unwrap();
    assert!(app.model_picker_filter.is_empty());
}

#[tokio::test]
async fn model_picker_paging_keys_robust_empty_list() {
    let mut app = test_app();
    app.show_model_picker = true;
    let (tx, _rx) = channel();
    // Each navigation key is consumed without panicking on empty list.
    for code in [
        KeyCode::Down,
        KeyCode::Up,
        KeyCode::Home,
        KeyCode::End,
        KeyCode::PageDown,
        KeyCode::PageUp,
    ] {
        handle_key(&mut app, key(code), &tx).await.unwrap();
    }
    assert_eq!(app.model_picker_selected, 0);
}

// ─────────────────────────────────────────────────────────────────────
// Slash autocomplete popup
// ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn slash_popup_down_cycles_normal() {
    // `/c` matches `/clear` and `/compact` so Down should advance
    // selection from 0 to 1 rather than wrapping inside a singleton.
    let mut app = test_app();
    app.textarea = TextArea::from(vec!["/c".to_string()]);
    let (tx, _rx) = channel();
    handle_key(&mut app, key(KeyCode::Down), &tx).await.unwrap();
    assert_eq!(app.slash_popup_selected, Some(1));
}

#[tokio::test]
async fn slash_popup_tab_commits_normal() {
    let mut app = test_app();
    app.textarea = TextArea::from(vec!["/he".to_string()]);
    let (tx, _rx) = channel();
    handle_key(&mut app, key(KeyCode::Tab), &tx).await.unwrap();
    let buf = app.textarea.lines().join("");
    assert!(buf.starts_with('/'));
    assert!(buf.ends_with(' '));
}

// Regression: typing the whole `/compact` then pressing Enter should
// SUBMIT the command instead of re-inserting `/compact ` and eating
// the keystroke. Before the fix the popup ate Enter and the user
// had to press it twice.
#[tokio::test]
async fn slash_popup_enter_on_exact_match_submits_regression() {
    let mut app = test_app();
    app.textarea = TextArea::from(vec!["/compact".to_string()]);
    let (tx, _rx) = channel();
    handle_key(&mut app, key(KeyCode::Enter), &tx)
        .await
        .unwrap();
    // The buffer should NOT be "/compact " (tab-completed) — it
    // should either be cleared (slash command ran and consumed
    // the input) or unchanged. The crucial assertion is that the
    // popup didn't re-insert with a trailing space.
    let buf = app.textarea.lines().join("");
    assert!(
        !buf.ends_with("/compact "),
        "Enter on exact match must not tab-complete; got buf={buf:?}"
    );
    // Popup selection state must be cleared so the next Enter
    // hits the normal submit path.
    assert_eq!(app.slash_popup_selected, None);
}

// ─────────────────────────────────────────────────────────────────────
// Transcript search (Ctrl+F when empty)
// ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn ctrl_f_opens_search_normal() {
    let mut app = test_app();
    let (tx, _rx) = channel();
    handle_key(
        &mut app,
        key_mod(KeyCode::Char('f'), KeyModifiers::CONTROL),
        &tx,
    )
    .await
    .unwrap();
    assert!(app.transcript_search.is_some());
}

#[tokio::test]
async fn search_typing_finds_matches_normal() {
    let mut app = test_app();
    app.engine
        .messages
        .push(ChatMessage::user("hello world".into()));
    app.engine
        .messages
        .push(ChatMessage::assistant("nope".into()));
    app.transcript_search = Some(crate::app::TranscriptSearch::default());
    let (tx, _rx) = channel();
    for c in "hello".chars() {
        handle_key(&mut app, key(KeyCode::Char(c)), &tx)
            .await
            .unwrap();
    }
    let s = app.transcript_search.as_ref().unwrap();
    assert_eq!(s.matches, vec![0]);
    assert_eq!(s.query, "hello");
}

#[tokio::test]
async fn search_backspace_shrinks_query_normal() {
    let mut app = test_app();
    app.transcript_search = Some(crate::app::TranscriptSearch {
        query: "abc".into(),
        ..Default::default()
    });
    let (tx, _rx) = channel();
    handle_key(&mut app, key(KeyCode::Backspace), &tx)
        .await
        .unwrap();
    assert_eq!(app.transcript_search.as_ref().unwrap().query, "ab");
}

#[tokio::test]
async fn search_enter_commits_robust() {
    let mut app = test_app();
    app.engine.messages.push(ChatMessage::user("foo".into()));
    let s = crate::app::TranscriptSearch {
        matches: vec![0],
        ..Default::default()
    };
    app.transcript_search = Some(s);
    let (tx, _rx) = channel();
    handle_key(&mut app, key(KeyCode::Enter), &tx)
        .await
        .unwrap();
    assert!(app.transcript_search.is_none());
}

#[tokio::test]
async fn search_esc_cancels_robust() {
    let mut app = test_app();
    app.transcript_search = Some(crate::app::TranscriptSearch::default());
    let (tx, _rx) = channel();
    handle_key(&mut app, key(KeyCode::Esc), &tx).await.unwrap();
    assert!(app.transcript_search.is_none());
}

#[tokio::test]
async fn search_arrows_cycle_matches_normal() {
    let mut app = test_app();
    app.engine.messages.push(ChatMessage::user("a".into()));
    app.engine.messages.push(ChatMessage::user("a".into()));
    let s = crate::app::TranscriptSearch {
        matches: vec![0, 1],
        ..Default::default()
    };
    app.transcript_search = Some(s);
    let (tx, _rx) = channel();
    handle_key(&mut app, key(KeyCode::Down), &tx).await.unwrap();
    assert_eq!(app.transcript_search.as_ref().unwrap().cursor, 1);
    handle_key(&mut app, key(KeyCode::Up), &tx).await.unwrap();
    assert_eq!(app.transcript_search.as_ref().unwrap().cursor, 0);
}

// ─────────────────────────────────────────────────────────────────────
// Jump (Ctrl+G)
// ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn ctrl_g_arms_jump_mode_normal() {
    let mut app = test_app();
    let (tx, _rx) = channel();
    handle_key(
        &mut app,
        key_mod(KeyCode::Char('g'), KeyModifiers::CONTROL),
        &tx,
    )
    .await
    .unwrap();
    assert!(app.jump_armed);
}

#[tokio::test]
async fn jump_armed_e_jumps_to_error_normal() {
    let mut app = test_app();
    // failed tool in messages → e jumps to it
    app.engine
        .messages
        .push(ChatMessage::assistant_parts(vec![MessagePart::tool(
            ToolCall {
                id: "t1".into(),
                kind: ToolKind::Bash,
                status: ToolStatus::Failed,
                input: ToolInput::Bash {
                    command: "x".into(),
                    timeout: None,
                    workdir: None,
                    run_in_background: None,
                },
                output: ToolOutput::Empty,
                display: jfc_core::ToolDisplayState::DEFAULT,
                elapsed_ms: None,
                started_at: None,
                thought_signature: None,
            },
        )]));
    app.jump_armed = true;
    app.jump_armed_at = Some(std::time::Instant::now());
    let (tx, _rx) = channel();
    handle_key(&mut app, key(KeyCode::Char('e')), &tx)
        .await
        .unwrap();
    assert!(!app.jump_armed);
}

#[tokio::test]
async fn jump_armed_t_jumps_to_tool_robust() {
    let mut app = test_app();
    app.jump_armed = true;
    app.jump_armed_at = Some(std::time::Instant::now());
    let (tx, _rx) = channel();
    handle_key(&mut app, key(KeyCode::Char('t')), &tx)
        .await
        .unwrap();
    assert!(!app.jump_armed);
}

#[tokio::test]
async fn jump_armed_m_jumps_to_user_robust() {
    let mut app = test_app();
    app.jump_armed = true;
    app.jump_armed_at = Some(std::time::Instant::now());
    let (tx, _rx) = channel();
    handle_key(&mut app, key(KeyCode::Char('m')), &tx)
        .await
        .unwrap();
    assert!(!app.jump_armed);
}

#[tokio::test]
async fn jump_armed_a_jumps_to_assistant_robust() {
    let mut app = test_app();
    app.jump_armed = true;
    app.jump_armed_at = Some(std::time::Instant::now());
    let (tx, _rx) = channel();
    handle_key(&mut app, key(KeyCode::Char('a')), &tx)
        .await
        .unwrap();
    assert!(!app.jump_armed);
}

// ─────────────────────────────────────────────────────────────────────
// Leader key (Ctrl+X)
// ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn ctrl_x_arms_leader_normal() {
    let mut app = test_app();
    let (tx, _rx) = channel();
    handle_key(
        &mut app,
        key_mod(KeyCode::Char('x'), KeyModifiers::CONTROL),
        &tx,
    )
    .await
    .unwrap();
    assert!(app.leader_key_active);
}

#[tokio::test]
async fn leader_then_k_exits_task_view_robust() {
    let mut app = test_app();
    app.leader_key_active = true;
    app.leader_key_timeout = Some(std::time::Instant::now());
    app.viewing_task_id = Some("t1".into());
    let (tx, _rx) = channel();
    handle_key(&mut app, key(KeyCode::Char('k')), &tx)
        .await
        .unwrap();
    assert!(app.viewing_task_id.is_none());
    assert!(!app.leader_key_active);
}

// ─────────────────────────────────────────────────────────────────────
// Up history recall on empty input
// ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn up_with_empty_input_recalls_history_normal() {
    let mut app = test_app();
    app.engine.messages.push(ChatMessage::user("first".into()));
    let (tx, _rx) = channel();
    handle_key(&mut app, key(KeyCode::Up), &tx).await.unwrap();
    let txt = app.textarea.lines().join("\n");
    assert_eq!(txt, "first");
}

#[tokio::test]
async fn up_recalls_queued_prompt_robust() {
    let mut app = test_app();
    app.engine.queued_prompts.push(crate::app::QueuedPrompt {
        text: "queued".into(),
        priority: crate::app::QueuePriority::Later,
        is_meta: false,
        attachments: Vec::new(),
    });
    // Push the placeholder user message that recall expects to remove.
    app.engine
        .messages
        .push(ChatMessage::user("⏳ queued".into()));
    let (tx, _rx) = channel();
    handle_key(&mut app, key(KeyCode::Up), &tx).await.unwrap();
    let txt = app.textarea.lines().join("\n");
    // The recall path inserts the prompt then a trailing newline + a
    // `delete_line_by_end` to trim. Some textarea versions leave a
    // sentinel newline; assert containment instead of strict equality.
    assert!(txt.contains("queued"));
    assert!(app.engine.queued_prompts.is_empty());
}

// REGRESSION (prompt-doubling): up-recall must REPLACE the textarea, not
// append into it. The entry guard only checks that every line is empty,
// which a single residual line can satisfy mid-edit; pre-fix `insert_str`
// then appended a second copy, producing the `phasesalright…` doubling that
// compounded each recall→submit cycle. Seed the textarea with the same text
// the queued prompt holds and assert recall yields ONE copy, not two.
#[tokio::test]
async fn up_recall_replaces_textarea_no_double_insert_regression() {
    let mut app = test_app();
    app.engine.queued_prompts.push(crate::app::QueuedPrompt {
        text: "alpha".into(),
        priority: crate::app::QueuePriority::Later,
        is_meta: false,
        attachments: Vec::new(),
    });
    app.engine
        .messages
        .push(ChatMessage::user("⏳ alpha".into()));
    // Residual content already in the textarea (a prior un-submitted recall).
    app.textarea = TextArea::from(vec!["alpha".to_string()]);
    let (tx, _rx) = channel();
    handle_key(&mut app, key(KeyCode::Up), &tx).await.unwrap();
    let txt = app.textarea.lines().join("\n");
    // Exactly one "alpha", never "alphaalpha".
    assert_eq!(
        txt.matches("alpha").count(),
        1,
        "recall must replace, not append (got {txt:?})"
    );
    assert!(
        !txt.contains("alphaalpha"),
        "double-insert regression: {txt:?}"
    );
}

#[tokio::test]
async fn down_after_recall_advances_normal() {
    let mut app = test_app();
    app.engine.messages.push(ChatMessage::user("a".into()));
    app.engine.messages.push(ChatMessage::user("b".into()));
    // Manually seed history_cursor at the older prompt — `Up` after the
    // first recall would otherwise hit `move_input_cursor_visual_up`
    // because `input_has_text` flips to true after the first replay.
    app.history_cursor = Some(0);
    app.textarea = TextArea::from(vec!["a".to_string()]);
    let (tx, _rx) = channel();
    handle_key(&mut app, key(KeyCode::Down), &tx).await.unwrap();
    let txt = app.textarea.lines().join("\n");
    assert_eq!(txt, "b");
}

#[tokio::test]
async fn down_past_recent_clears_input_robust() {
    let mut app = test_app();
    app.engine.messages.push(ChatMessage::user("a".into()));
    let (tx, _rx) = channel();
    handle_key(&mut app, key(KeyCode::Up), &tx).await.unwrap();
    // Down with cursor at most-recent already → clears.
    handle_key(&mut app, key(KeyCode::Down), &tx).await.unwrap();
    assert!(app.history_cursor.is_none());
    assert!(app.textarea.lines().iter().all(|l| l.is_empty()));
}

// ─────────────────────────────────────────────────────────────────────
// Ctrl+Y yank
// ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn ctrl_y_with_no_assistant_message_robust() {
    let mut app = test_app();
    let (tx, _rx) = channel();
    handle_key(
        &mut app,
        key_mod(KeyCode::Char('y'), KeyModifiers::CONTROL),
        &tx,
    )
    .await
    .unwrap();
    // Best-effort: should not panic. No assistant message → no clipboard call.
}

// ─────────────────────────────────────────────────────────────────────
// Ctrl+C
// ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn ctrl_c_clears_input_when_text_present_normal() {
    let mut app = test_app_with_input("hello", 80);
    let (tx, _rx) = channel();
    let exit = handle_key(
        &mut app,
        key_mod(KeyCode::Char('c'), KeyModifiers::CONTROL),
        &tx,
    )
    .await
    .unwrap();
    assert!(!exit);
    assert!(!input_has_text(&app));
}

#[tokio::test]
async fn ctrl_c_exits_when_input_empty_robust() {
    let mut app = test_app();
    let (tx, _rx) = channel();
    let exit = handle_key(
        &mut app,
        key_mod(KeyCode::Char('c'), KeyModifiers::CONTROL),
        &tx,
    )
    .await
    .unwrap();
    assert!(exit);
}

#[tokio::test]
async fn ctrl_c_interrupts_active_work_when_input_empty_normal() {
    let mut app = test_app();
    app.engine.is_streaming = true;
    let (tx, _rx) = channel();
    let exit = handle_key(
        &mut app,
        key_mod(KeyCode::Char('c'), KeyModifiers::CONTROL),
        &tx,
    )
    .await
    .unwrap();
    assert!(!exit);
    assert!(
        app.engine
            .interrupt_flag
            .load(std::sync::atomic::Ordering::SeqCst),
        "Ctrl+C during active work should request interrupt, not exit"
    );
    assert!(
        !app.engine.cancel_token.is_cancelled(),
        "App should mint a fresh token after cancelling the active turn clone"
    );
}

// ─────────────────────────────────────────────────────────────────────
// Ctrl+D
// ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn ctrl_d_deletes_when_text_present_normal() {
    let mut app = test_app_with_input("abc", 80);
    app.textarea.move_cursor(CursorMove::Head);
    let (tx, _rx) = channel();
    let exit = handle_key(
        &mut app,
        key_mod(KeyCode::Char('d'), KeyModifiers::CONTROL),
        &tx,
    )
    .await
    .unwrap();
    assert!(!exit);
}

#[tokio::test]
async fn ctrl_d_exits_on_empty_robust() {
    let mut app = test_app();
    let (tx, _rx) = channel();
    let exit = handle_key(
        &mut app,
        key_mod(KeyCode::Char('d'), KeyModifiers::CONTROL),
        &tx,
    )
    .await
    .unwrap();
    assert!(exit);
}

// ─────────────────────────────────────────────────────────────────────
// Ctrl+E (edit) and slash autocomplete-handled Ctrl+E in textarea
// ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn ctrl_e_edits_last_user_normal() {
    let mut app = test_app();
    app.engine.messages.push(ChatMessage::user("hello".into()));
    let (tx, _rx) = channel();
    handle_key(
        &mut app,
        key_mod(KeyCode::Char('e'), KeyModifiers::CONTROL),
        &tx,
    )
    .await
    .unwrap();
    assert_eq!(app.editing_message_idx, Some(0));
}

#[tokio::test]
async fn ctrl_e_robust_no_user_message() {
    let mut app = test_app();
    let (tx, _rx) = channel();
    handle_key(
        &mut app,
        key_mod(KeyCode::Char('e'), KeyModifiers::CONTROL),
        &tx,
    )
    .await
    .unwrap();
    assert!(app.editing_message_idx.is_none());
}

#[tokio::test]
async fn ctrl_e_blocked_when_streaming_robust() {
    let mut app = test_app();
    app.engine.messages.push(ChatMessage::user("hi".into()));
    app.engine.is_streaming = true;
    let (tx, _rx) = channel();
    handle_key(
        &mut app,
        key_mod(KeyCode::Char('e'), KeyModifiers::CONTROL),
        &tx,
    )
    .await
    .unwrap();
    assert!(app.editing_message_idx.is_none());
}

#[tokio::test]
async fn ctrl_e_with_text_jumps_to_end_normal() {
    // When input has text, Ctrl+E becomes "move to end of line".
    let mut app = test_app_with_input("abc", 80);
    app.textarea.move_cursor(CursorMove::Head);
    let (tx, _rx) = channel();
    handle_key(
        &mut app,
        key_mod(KeyCode::Char('e'), KeyModifiers::CONTROL),
        &tx,
    )
    .await
    .unwrap();
    assert_eq!(app.textarea.cursor(), (0, 3));
}

// ─────────────────────────────────────────────────────────────────────
// Ctrl+R retry
// ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn ctrl_r_opens_prompt_history_search_normal() {
    // Ctrl+R opens reverse-history search (bash convention); the most recent
    // user prompt is the top match, so Ctrl+R then Enter still "retries last".
    let mut app = test_app();
    app.engine.messages.push(ChatMessage::user("ask".into()));
    let (tx, mut rx) = channel();
    handle_key(
        &mut app,
        key_mod(KeyCode::Char('r'), KeyModifiers::CONTROL),
        &tx,
    )
    .await
    .unwrap();
    let search = app.prompt_search.as_ref().expect("prompt search opened");
    assert_eq!(search.selected_text(), Some("ask"));
    // Opening search does not itself submit anything.
    assert!(rx.try_recv().is_err());
}

#[tokio::test]
async fn ctrl_r_robust_no_prompt() {
    let mut app = test_app();
    let (tx, _rx) = channel();
    handle_key(
        &mut app,
        key_mod(KeyCode::Char('r'), KeyModifiers::CONTROL),
        &tx,
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn ctrl_r_blocked_when_streaming_robust() {
    let mut app = test_app();
    app.engine.messages.push(ChatMessage::user("ask".into()));
    app.engine.is_streaming = true;
    let (tx, mut rx) = channel();
    handle_key(
        &mut app,
        key_mod(KeyCode::Char('r'), KeyModifiers::CONTROL),
        &tx,
    )
    .await
    .unwrap();
    // No Submit emitted.
    assert!(rx.try_recv().is_err());
}

// ─────────────────────────────────────────────────────────────────────
// Ctrl+L path yank
// ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn ctrl_l_robust_no_paths() {
    let mut app = test_app();
    let (tx, _rx) = channel();
    handle_key(
        &mut app,
        key_mod(KeyCode::Char('l'), KeyModifiers::CONTROL),
        &tx,
    )
    .await
    .unwrap();
    assert_eq!(app.path_yank_cursor, 0);
}

// ─────────────────────────────────────────────────────────────────────
// Ctrl+Z / Ctrl+Shift+Z (undo / redo)
// ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn ctrl_z_undo_normal() {
    let mut app = test_app();
    let (tx, _rx) = channel();
    handle_key(
        &mut app,
        key_mod(KeyCode::Char('z'), KeyModifiers::CONTROL),
        &tx,
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn ctrl_shift_z_redo_robust() {
    let mut app = test_app();
    let (tx, _rx) = channel();
    handle_key(
        &mut app,
        key_mod(
            KeyCode::Char('Z'),
            KeyModifiers::CONTROL | KeyModifiers::SHIFT,
        ),
        &tx,
    )
    .await
    .unwrap();
}

// ─────────────────────────────────────────────────────────────────────
// Ctrl+I / Ctrl+S info sidebar
// ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn ctrl_i_toggles_info_sidebar_normal() {
    let mut app = test_app();
    let initial = app.show_info_sidebar;
    let (tx, _rx) = channel();
    handle_key(
        &mut app,
        key_mod(KeyCode::Char('i'), KeyModifiers::CONTROL),
        &tx,
    )
    .await
    .unwrap();
    assert_ne!(app.show_info_sidebar, initial);
}

#[tokio::test]
async fn ctrl_s_toggles_info_sidebar_normal() {
    let mut app = test_app();
    let initial = app.show_info_sidebar;
    let (tx, _rx) = channel();
    handle_key(
        &mut app,
        key_mod(KeyCode::Char('s'), KeyModifiers::CONTROL),
        &tx,
    )
    .await
    .unwrap();
    assert_ne!(app.show_info_sidebar, initial);
}

// ─────────────────────────────────────────────────────────────────────
// Ctrl+O diagnostic / reasoning expand
// ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn ctrl_o_opens_diagnostic_panel_when_diagnostics_present_normal() {
    let mut app = test_app();
    app.engine
        .diagnostics
        .push(jfc_engine::diagnostics::DiagnosticEntry {
            file: "src/lib.rs".into(),
            line: 1,
            col: 1,
            severity: jfc_engine::diagnostics::Severity::Error,
            message: "boom".into(),
            code: None,
            source: None,
        });
    let (tx, _rx) = channel();
    handle_key(
        &mut app,
        key_mod(KeyCode::Char('o'), KeyModifiers::CONTROL),
        &tx,
    )
    .await
    .unwrap();
    assert!(app.show_diagnostic_panel);
}

#[tokio::test]
async fn ctrl_o_closes_diagnostic_panel_when_open_robust() {
    let mut app = test_app();
    app.show_diagnostic_panel = true;
    let (tx, _rx) = channel();
    handle_key(
        &mut app,
        key_mod(KeyCode::Char('o'), KeyModifiers::CONTROL),
        &tx,
    )
    .await
    .unwrap();
    assert!(!app.show_diagnostic_panel);
}

#[tokio::test]
async fn ctrl_o_toggles_reasoning_robust_no_diagnostics() {
    // Ctrl+O toggles the reasoning block on the most recent assistant message
    // that has one. The renderer treats a missing entry as expanded, so the
    // first press seeds `true` then flips to `false` (collapse).
    let mut app = test_app();
    app.engine.messages.push(ChatMessage::assistant_parts(vec![
        jfc_core::MessagePart::Reasoning("thinking".into()),
        jfc_core::MessagePart::Text("hi".into()),
    ]));
    let (tx, _rx) = channel();
    handle_key(
        &mut app,
        key_mod(KeyCode::Char('o'), KeyModifiers::CONTROL),
        &tx,
    )
    .await
    .unwrap();
    assert_eq!(app.reasoning_expanded.get(&0), Some(&false));
}

// ─────────────────────────────────────────────────────────────────────
// Diagnostic panel scroll keys
// ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn diagnostic_panel_j_scrolls_down_normal() {
    let mut app = test_app();
    app.show_diagnostic_panel = true;
    let (tx, _rx) = channel();
    handle_key(&mut app, key(KeyCode::Char('j')), &tx)
        .await
        .unwrap();
    assert_eq!(app.diagnostic_panel_scroll, 1);
}

#[tokio::test]
async fn diagnostic_panel_k_scrolls_up_robust() {
    let mut app = test_app();
    app.show_diagnostic_panel = true;
    app.diagnostic_panel_scroll = 5;
    let (tx, _rx) = channel();
    handle_key(&mut app, key(KeyCode::Char('k')), &tx)
        .await
        .unwrap();
    assert_eq!(app.diagnostic_panel_scroll, 4);
}

#[tokio::test]
async fn diagnostic_panel_pagedown_normal() {
    let mut app = test_app();
    app.show_diagnostic_panel = true;
    let (tx, _rx) = channel();
    handle_key(&mut app, key(KeyCode::PageDown), &tx)
        .await
        .unwrap();
    assert_eq!(app.diagnostic_panel_scroll, 10);
}

#[tokio::test]
async fn diagnostic_panel_pageup_robust() {
    let mut app = test_app();
    app.show_diagnostic_panel = true;
    app.diagnostic_panel_scroll = 20;
    let (tx, _rx) = channel();
    handle_key(&mut app, key(KeyCode::PageUp), &tx)
        .await
        .unwrap();
    assert_eq!(app.diagnostic_panel_scroll, 10);
}

#[tokio::test]
async fn diagnostic_panel_home_g_top_normal() {
    let mut app = test_app();
    app.show_diagnostic_panel = true;
    app.diagnostic_panel_scroll = 5;
    let (tx, _rx) = channel();
    handle_key(&mut app, key(KeyCode::Char('g')), &tx)
        .await
        .unwrap();
    assert_eq!(app.diagnostic_panel_scroll, 0);
}

#[tokio::test]
async fn diagnostic_panel_end_capital_g_bottom_robust() {
    let mut app = test_app();
    app.show_diagnostic_panel = true;
    let (tx, _rx) = channel();
    handle_key(&mut app, key(KeyCode::Char('G')), &tx)
        .await
        .unwrap();
    assert!(app.diagnostic_panel_scroll > 1_000_000);
}

#[tokio::test]
async fn diagnostic_panel_esc_closes_normal() {
    let mut app = test_app();
    app.show_diagnostic_panel = true;
    let (tx, _rx) = channel();
    handle_key(&mut app, key(KeyCode::Esc), &tx).await.unwrap();
    assert!(!app.show_diagnostic_panel);
}

// ─────────────────────────────────────────────────────────────────────
// Vim-style transcript navigation (input empty)
// ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn vim_j_scrolls_down_normal() {
    let mut app = test_app();
    app.scroll_offset = 0;
    app.total_lines = 100;
    let (tx, _rx) = channel();
    handle_key(&mut app, key(KeyCode::Char('j')), &tx)
        .await
        .unwrap();
    // Some scroll happened (or 0 if at top with no clamp); just validate
    // behaviour didn't panic and doesn't move down beyond bounds.
    let _ = app.scroll_offset;
}

#[tokio::test]
async fn vim_k_scrolls_up_robust() {
    let mut app = test_app();
    app.scroll_offset = 5;
    let (tx, _rx) = channel();
    handle_key(&mut app, key(KeyCode::Char('k')), &tx)
        .await
        .unwrap();
    assert!(app.scroll_offset <= 5);
}

#[tokio::test]
async fn vim_capital_g_jumps_bottom_normal() {
    let mut app = test_app();
    app.follow_bottom = false;
    let (tx, _rx) = channel();
    handle_key(&mut app, key(KeyCode::Char('G')), &tx)
        .await
        .unwrap();
    assert!(app.follow_bottom);
}

#[tokio::test]
async fn vim_g_jumps_top_normal() {
    let mut app = test_app();
    app.scroll_offset = 50;
    app.follow_bottom = true;
    let (tx, _rx) = channel();
    handle_key(&mut app, key(KeyCode::Char('g')), &tx)
        .await
        .unwrap();
    assert_eq!(app.scroll_offset, 0);
    assert!(!app.follow_bottom);
}

#[tokio::test]
async fn question_toggles_help_normal() {
    let mut app = test_app();
    let (tx, _rx) = channel();
    handle_key(&mut app, key(KeyCode::Char('?')), &tx)
        .await
        .unwrap();
    assert!(app.show_help);
    handle_key(&mut app, key(KeyCode::Char('?')), &tx)
        .await
        .unwrap();
    assert!(!app.show_help);
}

#[tokio::test]
async fn shift_question_toggles_help_robust() {
    let mut app = test_app();
    let (tx, _rx) = channel();
    handle_key(
        &mut app,
        key_mod(KeyCode::Char('?'), KeyModifiers::SHIFT),
        &tx,
    )
    .await
    .unwrap();
    assert!(app.show_help);
}

#[tokio::test]
async fn lower_o_toggles_tool_expand_normal() {
    let mut app = test_app();
    app.engine
        .messages
        .push(ChatMessage::assistant_parts(vec![MessagePart::tool(
            ToolCall {
                id: "t".into(),
                kind: ToolKind::Read,
                status: ToolStatus::Completed,
                input: ToolInput::Read {
                    file_path: "x".into(),
                    offset: None,
                    limit: None,
                },
                output: ToolOutput::Text("hi".into()),
                display: jfc_core::ToolDisplayState::DEFAULT,
                elapsed_ms: None,
                started_at: None,
                thought_signature: None,
            },
        )]));
    let (tx, _rx) = channel();
    handle_key(&mut app, key(KeyCode::Char('o')), &tx)
        .await
        .unwrap();
    let MessagePart::Tool(tc) = &app.engine.messages[0].parts[0] else {
        panic!("tool not found")
    };
    assert!(tc.display.is_expanded());
}

// ─────────────────────────────────────────────────────────────────────
// Esc semantics
// ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn esc_closes_help_normal() {
    let mut app = test_app();
    app.show_help = true;
    let (tx, _rx) = channel();
    handle_key(&mut app, key(KeyCode::Esc), &tx).await.unwrap();
    assert!(!app.show_help);
}

#[tokio::test]
async fn esc_cancels_edit_mode_robust() {
    let mut app = test_app_with_input("draft", 80);
    app.editing_message_idx = Some(7);
    let (tx, _rx) = channel();
    handle_key(&mut app, key(KeyCode::Esc), &tx).await.unwrap();
    assert!(app.editing_message_idx.is_none());
}

#[tokio::test]
async fn esc_exits_task_view_robust() {
    let mut app = test_app();
    app.viewing_task_id = Some("abc".into());
    let (tx, _rx) = channel();
    handle_key(&mut app, key(KeyCode::Esc), &tx).await.unwrap();
    assert!(app.viewing_task_id.is_none());
}

#[tokio::test]
async fn esc_double_tap_while_streaming_interrupts_instantly_normal() {
    let mut app = test_app();
    app.engine.is_streaming = true;
    let (tx, _rx) = channel();
    // 1st ESC: arms the timer, shows hint.
    handle_key(&mut app, key(KeyCode::Esc), &tx).await.unwrap();
    assert!(app.last_esc_at.is_some(), "1st ESC should arm the timer");
    assert!(
        !app.engine
            .interrupt_flag
            .load(std::sync::atomic::Ordering::SeqCst),
        "1st ESC should NOT fire interrupt"
    );
    // 2nd ESC: instantly kills.
    handle_key(&mut app, key(KeyCode::Esc), &tx).await.unwrap();
    assert!(
        app.engine
            .interrupt_flag
            .load(std::sync::atomic::Ordering::SeqCst),
        "2nd ESC must set interrupt_flag"
    );
    assert!(
        !app.engine.cancel_token.is_cancelled(),
        "2nd ESC must leave the App ready with a fresh token"
    );
    assert!(app.last_esc_at.is_none(), "timer cleared after kill");
}

#[tokio::test]
async fn esc_repeat_does_not_confirm_or_spam_interrupt_robust() {
    let mut app = test_app();
    app.engine.is_streaming = true;
    let (tx, _rx) = channel();

    handle_key(&mut app, key(KeyCode::Esc), &tx).await.unwrap();
    let toast_count = app.engine.toasts.len();

    handle_key(&mut app, key_repeat(KeyCode::Esc), &tx)
        .await
        .unwrap();

    assert!(
        !app.engine
            .interrupt_flag
            .load(std::sync::atomic::Ordering::SeqCst),
        "held ESC repeat must not count as the confirming tap"
    );
    assert!(!app.engine.cancel_token.is_cancelled());
    assert_eq!(
        app.engine.toasts.len(),
        toast_count,
        "held ESC repeat should not spam duplicate toasts"
    );
}

#[tokio::test]
async fn esc_resets_input_when_idle_robust() {
    let mut app = test_app_with_input("draft", 80);
    let (tx, _rx) = channel();
    handle_key(&mut app, key(KeyCode::Esc), &tx).await.unwrap();
    assert!(!input_has_text(&app));
}

// ─────────────────────────────────────────────────────────────────────
// Shift+BackTab cycles permission mode
// ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn backtab_cycles_permission_mode_normal() {
    let mut app = test_app();
    let initial = app.engine.permission_mode;
    let (tx, _rx) = channel();
    handle_key(&mut app, key(KeyCode::BackTab), &tx)
        .await
        .unwrap();
    assert_ne!(app.engine.permission_mode, initial);
}

// ─────────────────────────────────────────────────────────────────────
// Page / Home / End
// ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn page_up_down_normal() {
    let mut app = test_app();
    let (tx, _rx) = channel();
    handle_key(&mut app, key(KeyCode::PageUp), &tx)
        .await
        .unwrap();
    handle_key(&mut app, key(KeyCode::PageDown), &tx)
        .await
        .unwrap();
}

#[tokio::test]
async fn ctrl_home_end_normal() {
    let mut app = test_app();
    let (tx, _rx) = channel();
    handle_key(&mut app, key_mod(KeyCode::Home, KeyModifiers::CONTROL), &tx)
        .await
        .unwrap();
    handle_key(&mut app, key_mod(KeyCode::End, KeyModifiers::CONTROL), &tx)
        .await
        .unwrap();
}

#[tokio::test]
async fn home_end_move_cursor_in_textarea_normal() {
    let mut app = test_app_with_input("abcdef", 80);
    app.textarea.move_cursor(CursorMove::Jump(0, 3));
    let (tx, _rx) = channel();
    handle_key(&mut app, key(KeyCode::Home), &tx).await.unwrap();
    assert_eq!(app.textarea.cursor(), (0, 0));
    handle_key(&mut app, key(KeyCode::End), &tx).await.unwrap();
    assert_eq!(app.textarea.cursor(), (0, 6));
}

// ─────────────────────────────────────────────────────────────────────
// Emacs-style movement: Ctrl+a/e/u/k/w
// ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn ctrl_a_moves_to_head_normal() {
    let mut app = test_app_with_input("abc", 80);
    app.textarea.move_cursor(CursorMove::End);
    let (tx, _rx) = channel();
    handle_key(
        &mut app,
        key_mod(KeyCode::Char('a'), KeyModifiers::CONTROL),
        &tx,
    )
    .await
    .unwrap();
    assert_eq!(app.textarea.cursor(), (0, 0));
}

#[tokio::test]
async fn ctrl_u_deletes_to_head_normal() {
    let mut app = test_app_with_input("hello", 80);
    app.textarea.move_cursor(CursorMove::End);
    let (tx, _rx) = channel();
    handle_key(
        &mut app,
        key_mod(KeyCode::Char('u'), KeyModifiers::CONTROL),
        &tx,
    )
    .await
    .unwrap();
    assert!(app.textarea.lines()[0].is_empty());
}

#[tokio::test]
async fn ctrl_k_deletes_to_eol_robust() {
    let mut app = test_app_with_input("hello", 80);
    app.textarea.move_cursor(CursorMove::Head);
    let (tx, _rx) = channel();
    handle_key(
        &mut app,
        key_mod(KeyCode::Char('k'), KeyModifiers::CONTROL),
        &tx,
    )
    .await
    .unwrap();
    assert!(app.textarea.lines()[0].is_empty());
}

#[tokio::test]
async fn ctrl_w_deletes_word_robust() {
    let mut app = test_app_with_input("hello world", 80);
    app.textarea.move_cursor(CursorMove::End);
    let (tx, _rx) = channel();
    handle_key(
        &mut app,
        key_mod(KeyCode::Char('w'), KeyModifiers::CONTROL),
        &tx,
    )
    .await
    .unwrap();
    assert!(!app.textarea.lines()[0].contains("world"));
}

// ─────────────────────────────────────────────────────────────────────
// Alt movement
// ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn alt_b_moves_word_back_normal() {
    let mut app = test_app_with_input("foo bar", 80);
    app.textarea.move_cursor(CursorMove::End);
    let (tx, _rx) = channel();
    handle_key(
        &mut app,
        key_mod(KeyCode::Char('b'), KeyModifiers::ALT),
        &tx,
    )
    .await
    .unwrap();
    assert_eq!(app.textarea.cursor().1, 4);
}

#[tokio::test]
async fn alt_f_moves_word_forward_normal() {
    let mut app = test_app_with_input("foo bar", 80);
    app.textarea.move_cursor(CursorMove::Head);
    let (tx, _rx) = channel();
    handle_key(
        &mut app,
        key_mod(KeyCode::Char('f'), KeyModifiers::ALT),
        &tx,
    )
    .await
    .unwrap();
    assert!(app.textarea.cursor().1 > 0);
}

#[tokio::test]
async fn alt_d_deletes_next_word_robust() {
    let mut app = test_app_with_input("foo bar", 80);
    app.textarea.move_cursor(CursorMove::Head);
    let (tx, _rx) = channel();
    handle_key(
        &mut app,
        key_mod(KeyCode::Char('d'), KeyModifiers::ALT),
        &tx,
    )
    .await
    .unwrap();
    assert!(!app.textarea.lines()[0].contains("foo"));
}

#[tokio::test]
async fn alt_period_raises_reasoning_effort_normal() {
    let mut app = test_app();
    app.engine
        .effort_state
        .set(jfc_engine::effort::ReasoningEffort::Medium);
    let (tx, _rx) = channel();
    handle_key(
        &mut app,
        key_mod(KeyCode::Char('.'), KeyModifiers::ALT),
        &tx,
    )
    .await
    .unwrap();
    assert_eq!(
        app.engine.effort_state.current,
        Some(jfc_engine::effort::ReasoningEffort::High)
    );
}

#[tokio::test]
async fn alt_comma_lowers_reasoning_effort_normal() {
    let mut app = test_app();
    app.engine
        .effort_state
        .set(jfc_engine::effort::ReasoningEffort::Medium);
    let (tx, _rx) = channel();
    handle_key(
        &mut app,
        key_mod(KeyCode::Char(','), KeyModifiers::ALT),
        &tx,
    )
    .await
    .unwrap();
    assert_eq!(
        app.engine.effort_state.current,
        Some(jfc_engine::effort::ReasoningEffort::Low)
    );
}

// ─────────────────────────────────────────────────────────────────────
// Ctrl+F when input non-empty (page down)
// ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn ctrl_f_with_input_pages_down_normal() {
    let mut app = test_app_with_input("hello", 80);
    app.viewport_height = 5;
    let (tx, _rx) = channel();
    handle_key(
        &mut app,
        key_mod(KeyCode::Char('f'), KeyModifiers::CONTROL),
        &tx,
    )
    .await
    .unwrap();
}

// ─────────────────────────────────────────────────────────────────────
// Submit (Enter)
// ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn enter_with_empty_does_nothing_normal() {
    let mut app = test_app();
    let (tx, _rx) = channel();
    handle_key(&mut app, key(KeyCode::Enter), &tx)
        .await
        .unwrap();
    assert!(app.engine.messages.is_empty());
}

#[tokio::test]
async fn enter_queues_when_streaming_normal() {
    let mut app = test_app_with_input("ask", 80);
    app.engine.is_streaming = true;
    app.engine.compacting_started_at = Some(std::time::Instant::now());
    let (tx, _rx) = channel();
    handle_key(&mut app, key(KeyCode::Enter), &tx)
        .await
        .unwrap();
    assert_eq!(app.engine.queued_prompts.len(), 1);
    assert_eq!(app.engine.queued_prompts[0].text, "ask");
    assert!(!app.engine.queued_prompts[0].is_meta);
}

#[tokio::test]
async fn direct_submit_queues_during_compaction_regression() {
    let mut app = test_app();
    app.engine.compacting_started_at = Some(std::time::Instant::now());
    let (tx, _rx) = channel();

    submit::handle_submit(&mut app, "after compact".into(), &tx)
        .await
        .unwrap();

    assert_eq!(app.engine.queued_prompts.len(), 1);
    assert_eq!(app.engine.queued_prompts[0].text, "after compact");
    assert!(!app.engine.queued_prompts[0].is_meta);
    assert!(app.engine.messages.iter().all(|message| message.queued));
    assert!(!app.engine.is_streaming);
}

#[tokio::test]
async fn enter_queues_meta_for_slash_when_streaming_robust() {
    // `/help ` (with trailing space) skips the slash-autocomplete popup
    // because `current_slash_prefix` truncates at whitespace; `slash_matches`
    // would still find `/help` but the popup arm only intercepts when
    // there's at least one match — to bypass we use a verb that matches no
    // command but still starts with `/`.
    let mut app = test_app_with_input("/zzzz", 80);
    app.engine.is_streaming = true;
    app.engine.compacting_started_at = Some(std::time::Instant::now());
    let (tx, _rx) = channel();
    handle_key(&mut app, key(KeyCode::Enter), &tx)
        .await
        .unwrap();
    assert_eq!(app.engine.queued_prompts.len(), 1);
    assert!(app.engine.queued_prompts[0].is_meta);
}

#[tokio::test]
async fn enter_interrupts_and_submits_when_streaming_without_blockers() {
    let mut app = test_app_with_input("ask", 80);
    app.engine.is_streaming = true;
    app.engine.streaming_started_at = Some(std::time::Instant::now());
    // Output has begun — interrupt-on-submit is the right call here (real-time
    // steering). `streaming_response_bytes > 0` is the gate the fix added.
    app.engine.streaming_response_bytes = 128;
    let (tx, _rx) = channel();

    handle_key(&mut app, key(KeyCode::Enter), &tx)
        .await
        .unwrap();

    assert!(app.engine.queued_prompts.is_empty());
    assert_eq!(app.engine.messages.len(), 2);
    assert_eq!(app.engine.messages[0].role, Role::User);
    assert!(matches!(
        app.engine.messages[0].parts.first(),
        Some(MessagePart::Text(text)) if text == "ask"
    ));
    assert_eq!(app.engine.messages[1].role, Role::Assistant);
    assert!(app.engine.is_streaming);
}

// REGRESSION (queueing-during-connect): submitting a second message while the
// first stream is still opening its connection (is_streaming but
// streaming_response_bytes == 0) must QUEUE it, not interrupt. Pre-fix this
// cancelled the still-connecting first stream — which then bailed "Stream
// cancelled before connection opened" onto the new turn — and dropped the
// first message's answer entirely.
#[tokio::test]
async fn enter_queues_when_streaming_before_first_byte_regression() {
    let mut app = test_app_with_input("second message", 80);
    app.engine.is_streaming = true;
    app.engine.streaming_started_at = Some(std::time::Instant::now());
    // No output yet — the connection is still opening.
    app.engine.streaming_response_bytes = 0;
    let (tx, _rx) = channel();

    handle_key(&mut app, key(KeyCode::Enter), &tx)
        .await
        .unwrap();

    // The second message is queued, not submitted as a fresh turn.
    assert_eq!(
        app.engine.queued_prompts.len(),
        1,
        "second message during connect must be queued"
    );
    assert_eq!(app.engine.queued_prompts[0].text, "second message");
    assert!(!app.engine.queued_prompts[0].is_meta);
    // The in-flight first stream is left untouched (not cancelled).
    assert!(app.engine.is_streaming, "first stream must keep running");
}

// ─────────────────────────────────────────────────────────────────────
// Slash command dispatch via run_slash_command
// ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn slash_clear_wipes_messages_normal() {
    let mut app = test_app();
    app.engine.messages.push(ChatMessage::user("hi".into()));
    run_slash_command(&mut app, "/clear").await;
    assert!(app.engine.messages.is_empty());
}

#[tokio::test]
async fn slash_help_sets_show_help_normal() {
    let mut app = test_app();
    run_slash_command(&mut app, "/help").await;
    assert!(app.show_help);
}

#[tokio::test]
async fn slash_compact_sets_pending_robust() {
    let mut app = test_app();
    run_slash_command(&mut app, "/compact").await;
    assert!(app.engine.force_compact_pending);
}

#[tokio::test]
async fn slash_unknown_emits_assistant_message_robust() {
    let mut app = test_app();
    run_slash_command(&mut app, "/no-such-thing").await;
    let last = app.engine.messages.last().expect("message added");
    assert_eq!(last.role, Role::Assistant);
}

#[tokio::test]
async fn slash_mode_sets_permission_mode_normal() {
    let mut app = test_app();
    run_slash_command(&mut app, "/mode plan").await;
    assert_eq!(app.engine.permission_mode, crate::app::PermissionMode::Plan);
}

#[tokio::test]
async fn slash_mode_default_robust() {
    let mut app = test_app();
    run_slash_command(&mut app, "/mode default").await;
    assert_eq!(
        app.engine.permission_mode,
        crate::app::PermissionMode::Default
    );
}

#[tokio::test]
async fn slash_mode_unknown_robust() {
    let mut app = test_app();
    let initial = app.engine.permission_mode;
    run_slash_command(&mut app, "/mode wat").await;
    assert_eq!(app.engine.permission_mode, initial);
}

#[tokio::test]
async fn slash_mode_status_only_robust() {
    let mut app = test_app();
    run_slash_command(&mut app, "/mode").await;
    // Just ensure no panic & assistant message added.
    assert!(!app.engine.messages.is_empty());
}

#[tokio::test]
#[serial_test::serial]
async fn slash_temp_sets_temperature_normal() {
    let _guard = TemperatureGlobalGuard;
    let mut app = test_app();

    run_slash_command(&mut app, "/temp 0.7").await;

    assert_eq!(app.engine.temperature_state.current, Some(0.7));
    assert_eq!(jfc_engine::exploration::active_temperature(), Some(0.7));
    let last_text = app
        .engine
        .messages
        .last()
        .and_then(|m| m.parts.first())
        .map(MessagePart::text_only)
        .unwrap_or_default();
    assert!(last_text.contains("Temperature set to: 0.7"));
}

#[tokio::test]
#[serial_test::serial]
async fn slash_temp_clear_removes_temperature_normal() {
    let _guard = TemperatureGlobalGuard;
    let mut app = test_app();
    app.engine.temperature_state.set(1.1);

    run_slash_command(&mut app, "/temperature clear").await;

    assert_eq!(app.engine.temperature_state.current, None);
    assert_eq!(jfc_engine::exploration::active_temperature(), None);
}

#[tokio::test]
#[serial_test::serial]
async fn slash_temp_rejects_out_of_range_robust() {
    let _guard = TemperatureGlobalGuard;
    let mut app = test_app();

    run_slash_command(&mut app, "/temp 3").await;

    assert_eq!(app.engine.temperature_state.current, None);
    assert_eq!(jfc_engine::exploration::active_temperature(), None);
    let last_text = app
        .engine
        .messages
        .last()
        .and_then(|m| m.parts.first())
        .map(MessagePart::text_only)
        .unwrap_or_default();
    assert!(last_text.contains("between 0.0 and 2.0"));
}

#[tokio::test]
#[serial_test::serial]
async fn slash_explore_raises_sticky_level_normal() {
    let _guard = TemperatureGlobalGuard;
    let mut app = test_app();

    run_slash_command(&mut app, "/explore").await;

    assert_eq!(app.engine.exploration_state.sticky_delta, 1);
    assert_eq!(
        jfc_engine::exploration::active_exploration_level(),
        Some(app.engine.exploration_state.current)
    );
    let last_text = app
        .engine
        .messages
        .last()
        .and_then(|m| m.parts.first())
        .map(MessagePart::text_only)
        .unwrap_or_default();
    assert!(last_text.contains("Exploration raised"));
}

#[tokio::test]
#[serial_test::serial]
async fn slash_focus_lowers_sticky_level_normal() {
    let _guard = TemperatureGlobalGuard;
    let mut app = test_app();
    app.engine.exploration_state.adjust_sticky(1);

    run_slash_command(&mut app, "/focus").await;

    assert_eq!(app.engine.exploration_state.sticky_delta, 0);
    let last_text = app
        .engine
        .messages
        .last()
        .and_then(|m| m.parts.first())
        .map(MessagePart::text_only)
        .unwrap_or_default();
    assert!(last_text.contains("Exploration lowered"));
}

#[tokio::test]
async fn slash_auto_mode_on_robust() {
    let mut app = test_app();
    run_slash_command(&mut app, "/auto-mode on").await;
    assert!(app.engine.auto_mode.enabled);
}

#[tokio::test]
async fn slash_auto_mode_off_robust() {
    let mut app = test_app();
    app.engine.auto_mode.enabled = true;
    run_slash_command(&mut app, "/auto-mode off").await;
    assert!(!app.engine.auto_mode.enabled);
}

#[tokio::test]
async fn slash_auto_mode_status_robust() {
    let mut app = test_app();
    run_slash_command(&mut app, "/auto-mode").await;
    assert!(!app.engine.messages.is_empty());
}

#[tokio::test]
async fn slash_task_add_creates_task_normal() {
    let mut app = test_app();
    run_slash_command(&mut app, "/task-add make tests pass").await;
    let tasks = app
        .engine
        .task_store
        .list(jfc_session::DeletedFilter::Exclude);
    assert_eq!(tasks.len(), 1);
}

#[tokio::test]
async fn slash_task_add_robust_no_args() {
    let mut app = test_app();
    run_slash_command(&mut app, "/task-add").await;
    let tasks = app
        .engine
        .task_store
        .list(jfc_session::DeletedFilter::Exclude);
    assert!(tasks.is_empty());
}

#[tokio::test]
async fn slash_tasks_list_normal() {
    let mut app = test_app();
    run_slash_command(&mut app, "/tasks").await;
    assert!(!app.engine.messages.is_empty());
}

#[tokio::test]
async fn slash_task_done_robust_no_args() {
    let mut app = test_app();
    run_slash_command(&mut app, "/task-done").await;
    assert!(!app.engine.messages.is_empty());
}

#[tokio::test]
async fn slash_task_rm_robust_no_args() {
    let mut app = test_app();
    run_slash_command(&mut app, "/task-rm").await;
    assert!(!app.engine.messages.is_empty());
}

#[tokio::test]
async fn slash_check_emits_assistant_robust() {
    let mut app = test_app();
    run_slash_command(&mut app, "/check").await;
    assert!(
        app.engine
            .messages
            .iter()
            .any(|m| m.role == Role::Assistant)
    );
}

#[tokio::test]
async fn slash_config_reports_path_normal() {
    let mut app = test_app();
    run_slash_command(&mut app, "/config path").await;
    assert!(
        app.engine
            .messages
            .iter()
            .any(|m| matches!(&m.parts[0], MessagePart::Text(s) if s.contains("Config path")))
    );
}

#[tokio::test]
async fn slash_config_dumps_toml_robust() {
    let mut app = test_app();
    run_slash_command(&mut app, "/config").await;
    assert!(!app.engine.messages.is_empty());
}

#[tokio::test]
async fn slash_skills_lists_normal() {
    let mut app = test_app();
    run_slash_command(&mut app, "/skills").await;
    assert!(!app.engine.messages.is_empty());
}

#[tokio::test]
async fn slash_agents_lists_robust() {
    let mut app = test_app();
    run_slash_command(&mut app, "/agents").await;
    assert!(!app.engine.messages.is_empty());
}

#[tokio::test]
async fn slash_claude_md_lists_normal() {
    let mut app = test_app();
    run_slash_command(&mut app, "/claude-md").await;
    assert!(!app.engine.messages.is_empty());
}

#[tokio::test]
async fn slash_dump_context_normal() {
    let mut app = test_app();
    run_slash_command(&mut app, "/dump-context").await;
    assert!(!app.engine.messages.is_empty());
}

#[tokio::test]
async fn slash_theme_opens_picker_when_no_arg_robust() {
    let mut app = test_app();
    run_slash_command(&mut app, "/theme").await;
    assert!(app.show_theme_picker);
    assert!(app.theme_picker_input.is_empty());
    // Opening highlights the active theme (revert-on-cancel preview), so the
    // selection is whichever row matches the current theme — always a valid
    // in-bounds index, not hard-coded to 0.
    let n = crate::theme::Theme::choices().len();
    assert!(
        app.theme_picker_selected < n,
        "selection {} out of bounds (n={n})",
        app.theme_picker_selected
    );
}

#[tokio::test]
async fn slash_theme_unknown_pushes_warning_robust() {
    let mut app = test_app();
    run_slash_command(&mut app, "/theme nonexistent").await;
    // No theme change. Toast added.
    assert!(!app.engine.toasts.is_empty());
}

// Regression: switching the theme MUST invalidate the render cache.
// Without invalidation, cached lines carry baked-in syntect highlight
// colors from the previous theme and the user sees stale colors until
// each entry is naturally evicted by the LRU. For static transcript
// content that staleness would persist until session reload.
//
// We exercise the bug by populating the cache, switching the theme via
// the same `/theme` slash-command path the user types, then re-rendering
// the same `(text, width)` key. The closure passed to
// `get_or_insert_with` runs only on a cache miss, so a post-switch
// closure invocation proves the entry was invalidated.
#[tokio::test]
async fn slash_theme_invalidates_render_cache_regression() {
    let mut app = test_app();
    let text = "hello **world**";
    let width: u16 = 80;

    // Prime the cache.
    {
        let mut cache = app.render_cache.borrow_mut();
        let _ = cache.get_or_insert_with(text, width, |t, _w| {
            vec![ratatui::text::Line::from(t.to_owned())]
        });
        assert_eq!(cache.len(), 1, "prime should populate exactly one entry");
    }

    // Switch theme via the public command surface (mirrors what a user
    // actually types). `dark` is always available, even if the test
    // App already starts on it — `Theme::by_name("light")` is the
    // visually distinct case.
    run_slash_command(&mut app, "/theme light").await;

    // Post-switch: the cache must be empty so the next render runs the
    // syntect pipeline against the new theme.
    {
        let cache = app.render_cache.borrow();
        assert_eq!(
            cache.len(),
            0,
            "theme switch should have cleared the render cache"
        );
    }

    // Stronger assertion: the closure runs again (cache miss) for the
    // exact same (text, width) key it was primed with.
    let mut closure_invocations = 0u32;
    {
        let mut cache = app.render_cache.borrow_mut();
        let _ = cache.get_or_insert_with(text, width, |t, _w| {
            closure_invocations += 1;
            vec![ratatui::text::Line::from(t.to_owned())]
        });
    }
    assert_eq!(
        closure_invocations, 1,
        "post-theme-switch render must miss the cache and rebuild lines"
    );
}

#[tokio::test]
async fn slash_export_creates_file_robust() {
    let mut app = test_app();
    app.engine.messages.push(ChatMessage::user("hi".into()));
    run_slash_command(&mut app, "/export").await;
    // Either a success or error toast was emitted.
    assert!(!app.engine.toasts.is_empty());
}

#[tokio::test]
async fn slash_rename_robust_no_session() {
    let mut app = test_app();
    run_slash_command(&mut app, "/rename my-title").await;
    assert!(!app.engine.messages.is_empty());
}

#[tokio::test]
async fn slash_rename_robust_no_args_with_session() {
    let mut app = test_app();
    app.engine.current_session_id = Some(jfc_engine::ids::SessionId::new("ses_test"));
    run_slash_command(&mut app, "/rename").await;
    assert!(!app.engine.messages.is_empty());
}

#[tokio::test]
async fn slash_resume_lists_when_no_arg_robust() {
    let mut app = test_app();
    run_slash_command(&mut app, "/resume").await;
    assert!(!app.engine.messages.is_empty());
}

#[tokio::test]
async fn slash_resume_unknown_id_robust() {
    let mut app = test_app();
    run_slash_command(&mut app, "/resume ses_does_not_exist").await;
    assert!(
        app.engine
            .messages
            .iter()
            .any(|m| matches!(&m.parts[0], MessagePart::Text(s) if s.contains("not found")))
    );
}

#[tokio::test]
async fn slash_continue_robust_no_sessions() {
    let mut app = test_app();
    run_slash_command(&mut app, "/continue").await;
    assert!(!app.engine.messages.is_empty());
}

#[tokio::test]
async fn slash_sessions_list_robust() {
    let mut app = test_app();
    run_slash_command(&mut app, "/sessions").await;
    assert!(!app.engine.messages.is_empty());
}

#[tokio::test]
async fn slash_worktree_list_normal() {
    let mut app = test_app();
    run_slash_command(&mut app, "/worktree list").await;
    assert!(!app.engine.messages.is_empty());
}

#[tokio::test]
async fn slash_worktree_create_no_arg_robust() {
    let mut app = test_app();
    run_slash_command(&mut app, "/worktree create").await;
    assert!(!app.engine.messages.is_empty());
}

#[tokio::test]
async fn slash_worktree_remove_no_arg_robust() {
    let mut app = test_app();
    run_slash_command(&mut app, "/worktree remove").await;
    assert!(!app.engine.messages.is_empty());
}

#[tokio::test]
async fn slash_worktree_switch_no_arg_robust() {
    let mut app = test_app();
    run_slash_command(&mut app, "/worktree switch").await;
    assert!(!app.engine.messages.is_empty());
}

#[tokio::test]
async fn slash_worktree_unknown_subcommand_robust() {
    let mut app = test_app();
    run_slash_command(&mut app, "/worktree foobar").await;
    assert!(
        app.engine.messages.iter().any(
            |m| matches!(&m.parts[0], MessagePart::Text(s) if s.contains("Unknown subcommand"))
        )
    );
}

#[tokio::test]
async fn slash_swarm_approve_no_args_robust() {
    let mut app = test_app();
    run_slash_command(&mut app, "/swarm-approve").await;
    assert!(!app.engine.messages.is_empty());
}

#[tokio::test]
async fn slash_swarm_deny_no_team_robust() {
    let mut app = test_app();
    run_slash_command(&mut app, "/swarm-deny abc-123").await;
    assert!(!app.engine.messages.is_empty());
}

// Normal: /market renders the agent-economy snapshot via the
// shared market_report_string helper. Even with no bounties
// posted, the report has the standard headers.
#[tokio::test]
async fn slash_market_renders_snapshot_normal() {
    let mut app = test_app();
    run_slash_command(&mut app, "/market").await;
    assert!(!app.engine.messages.is_empty());
    let body: String = app
        .engine
        .messages
        .last()
        .unwrap()
        .parts
        .iter()
        .filter_map(|p| match p {
            jfc_core::MessagePart::Text(t) => Some(t.clone()),
            _ => None,
        })
        .collect();
    assert!(
        body.contains("Agent economy snapshot") || body.contains("Market unavailable"),
        "expected snapshot or error, got: {body}"
    );
}

// ─────────────────────────────────────────────────────────────────────
// Mention (@ autocomplete)
// ─────────────────────────────────────────────────────────────────────

// Mention pick: Esc / Enter / Up / Down with NONE modifier are caught
// by earlier arms in `handle_key` (Esc → reset_input, Enter → submit,
// arrows → cursor move/recall). The popup-active block at line 1895
// is therefore reachable mainly through Tab. Test that path directly.

#[tokio::test]
async fn mention_tab_applies_pick_normal() {
    let mut app = test_app_with_input("@", 80);
    app.textarea.move_cursor(CursorMove::End);
    app.mention.activate(0, vec!["src/lib.rs".into()]);
    let (tx, _rx) = channel();
    handle_key(&mut app, key(KeyCode::Tab), &tx).await.unwrap();
    assert!(!app.mention.active);
    assert!(app.textarea.lines()[0].contains("src/lib.rs"));
}

/// Direct-call tests of the mention pick / state apply helpers — the
/// popup-active dispatch in `handle_key` is mostly unreachable because
/// the global Esc / Enter / arrow arms intercept those keys before
/// the mention block sees them. The helpers themselves are still
/// load-bearing for the `@` autocomplete UX, so we exercise them
/// directly.
#[test]
fn apply_mention_pick_replaces_token_normal() {
    let mut app = test_app_with_input("hi @s", 80);
    app.textarea.move_cursor(CursorMove::End);
    app.mention.anchor_byte = 3;
    app.mention.query = "s".into();
    apply_mention_pick(&mut app, "src/lib.rs");
    let buf = app.textarea.lines().join("\n");
    assert!(buf.contains("src/lib.rs"));
}

// ─────────────────────────────────────────────────────────────────────
// apply_mention_pick / update_mention_state_after_input
// ─────────────────────────────────────────────────────────────────────

#[test]
fn update_mention_state_activates_on_at_normal() {
    let mut app = test_app();
    app.textarea = TextArea::from(vec!["@".to_string()]);
    app.textarea.move_cursor(CursorMove::Jump(0, 1));
    update_mention_state_after_input(&mut app);
    assert!(app.mention.active);
}

#[test]
fn update_mention_state_dismisses_on_whitespace_robust() {
    let mut app = test_app();
    app.textarea = TextArea::from(vec!["@x ".to_string()]);
    app.textarea.move_cursor(CursorMove::End);
    app.mention.active = true;
    app.mention.anchor_byte = 0;
    update_mention_state_after_input(&mut app);
    assert!(!app.mention.active);
}

// ─────────────────────────────────────────────────────────────────────
// Filtered models / palette items
// ─────────────────────────────────────────────────────────────────────

#[test]
fn filtered_models_unfiltered_returns_all_normal() {
    let mut app = test_app();
    app.model_picker_models = vec![ModelInfo::new("m1", "M1", "test")];
    let v = filtered_models(&app);
    assert_eq!(v.len(), 1);
}

#[test]
fn filtered_models_filter_robust() {
    let mut app = test_app();
    app.model_picker_models = vec![
        ModelInfo::new("alpha", "Alpha", "test"),
        ModelInfo::new("beta", "Beta", "test"),
    ];
    app.model_picker_filter = "alp".into();
    let v = filtered_models(&app);
    assert_eq!(v.len(), 1);
    assert_eq!(v[0].id.as_str(), "alpha");
}

#[test]
fn palette_items_filter_normal() {
    let mut app = test_app();
    app.palette_input = "compact".into();
    let v = palette_items(&app);
    assert!(v.iter().any(|s| s.contains("Compact")));
}

#[test]
fn palette_items_unfiltered_robust() {
    let app = test_app();
    let v = palette_items(&app);
    assert!(!v.is_empty());
}

// ─────────────────────────────────────────────────────────────────────
// Drift guard: the SLASH_COMMANDS registry table is the single source of
// truth for dispatch + autocomplete + /help. This test makes drift
// IMPOSSIBLE: every entry in the table must route to a real handler, never
// the skill-fallthrough's "Unknown command" branch. If someone adds a row
// to the table without a handler (or vice-versa), the macro won't compile;
// if the canonical/alias literal in the table ever diverges from the match
// arm, this catches it at test time.
// ─────────────────────────────────────────────────────────────────────

#[tokio::test]
#[serial_test::serial]
async fn slash_registry_every_entry_dispatches_robust() {
    for (name, _help) in crate::input::slash_commands_table() {
        let mut app = test_app();
        run_slash_command(&mut app, name).await;
        let hit_fallthrough = app.engine.messages.iter().any(|m| {
            m.parts.iter().any(|p| {
                if let jfc_core::MessagePart::Text(t) = p {
                    t.contains("Unknown command:")
                } else {
                    false
                }
            })
        });
        assert!(
            !hit_fallthrough,
            "table entry `{name}` fell through to the Unknown-command branch — \
             the SLASH_COMMANDS table has drifted from the dispatch match",
        );
    }
    // Dispatching `/sandbox` installed the process-global sandbox config;
    // restore the baseline so later bash tests aren't forced through bwrap.
    jfc_engine::sandbox::reset_active_bash_sandbox_for_test();
}

#[test]
fn slash_registry_table_is_nonempty_and_unique_normal() {
    // The macro emits one row per canonical name + one per alias. Names must
    // be unique (a duplicate literal would make the dispatch match arm
    // unreachable and silently shadow a command).
    let names: Vec<&str> = crate::input::slash_commands_table()
        .iter()
        .map(|(n, _)| *n)
        .collect();
    assert!(
        names.len() >= 80,
        "expected the full command set, got {}",
        names.len()
    );
    let mut deduped = names.clone();
    deduped.sort_unstable();
    deduped.dedup();
    assert_eq!(
        deduped.len(),
        names.len(),
        "duplicate command literal in SLASH_COMMANDS table",
    );
}

// ── can_interrupt_on_submit (queueing-during-connect fix) ──────────────────

/// Helper: an App in the "mid-stream" state with `n` response bytes already
/// accumulated. `bytes > 0` models "the model has started producing output".
fn streaming_app_with_bytes(bytes: usize) -> App {
    let mut app = test_app();
    app.engine.is_streaming = true;
    app.engine.streaming_response_bytes = bytes;
    app
}

// Normal: once output has begun (bytes > 0) and nothing is gating, a fresh
// submit may interrupt for real-time steering.
#[test]
fn can_interrupt_when_output_has_started_normal() {
    let app = streaming_app_with_bytes(42);
    assert!(key_dispatch::can_interrupt_on_submit(
        &app, /* compacting = */ false
    ));
}

// Normal — REGRESSION (the connect-phase drop): before the first byte
// arrives (bytes == 0) the stream is still opening its connection. Submitting
// must NOT interrupt — there's no output to steer and the first message
// hasn't been answered. The caller falls through to the queue path instead.
#[test]
fn cannot_interrupt_before_first_byte_normal_regression() {
    let app = streaming_app_with_bytes(0);
    assert!(
        !key_dispatch::can_interrupt_on_submit(&app, false),
        "a not-yet-connected stream must be queued behind, not interrupted"
    );
}

// Robust: compaction in progress is never interruptible regardless of bytes.
#[test]
fn cannot_interrupt_while_compacting_robust() {
    let app = streaming_app_with_bytes(999);
    assert!(!key_dispatch::can_interrupt_on_submit(
        &app, /* compacting = */ true
    ));
}

// Robust: not streaming at all ⇒ not interruptible (the busy state was a
// non-streaming pipeline like a pending approval; queue path handles it).
#[test]
fn cannot_interrupt_when_not_streaming_robust() {
    let mut app = streaming_app_with_bytes(100);
    app.engine.is_streaming = false;
    assert!(!key_dispatch::can_interrupt_on_submit(&app, false));
}

// Robust: a pending approval modal blocks interrupt even mid-output — the
// user must resolve the approval first; queueing keeps the contract intact.
#[test]
fn cannot_interrupt_with_pending_approval_robust() {
    let mut app = streaming_app_with_bytes(100);
    arm_approval(&mut app, ToolKind::Bash);
    assert!(
        !key_dispatch::can_interrupt_on_submit(&app, false),
        "pending approval must force the queue path"
    );
}

#[test]
fn cannot_interrupt_with_in_flight_tool_batch_robust() {
    let mut app = streaming_app_with_bytes(100);
    app.engine.in_flight_tool_batches = 1;
    assert!(
        !key_dispatch::can_interrupt_on_submit(&app, false),
        "in-flight tool batches must finish or cancel through the tool pipeline"
    );
}
