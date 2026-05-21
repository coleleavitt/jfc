use super::super::coordinator::check_task_list_for_work;
use super::super::executor::{TurnResult, run_single_turn};
use super::*;
use crate::swarm::mailbox;
use crate::swarm::test_support::HomeOverride;

fn make_identity() -> TeammateIdentity {
    TeammateIdentity {
        agent_id: "alice@alpha".into(),
        agent_name: "alice".into(),
        team_name: "alpha".into(),
        color: Some("#FF0000".into()),
        plan_mode_required: false,
        parent_session_id: "session-1".into(),
    }
}

#[test]
fn teammate_task_id_format_normal() {
    assert_eq!(teammate_task_id("alice@alpha"), "teammate-alice@alpha");
}

#[test]
fn assign_teammate_color_cycles_through_palette_normal() {
    // Two consecutive calls must return real palette entries (hex strings).
    // We don't lock the order because the COLOR_INDEX is process-global,
    // but every value should start with `#` and be 7 chars long.
    for _ in 0..5 {
        let c = assign_teammate_color();
        assert_eq!(c.len(), 7, "expected `#RRGGBB`, got {c}");
        assert!(c.starts_with('#'));
    }
}

#[tokio::test]
async fn poll_leader_inbox_returns_empty_for_no_messages_normal() {
    let _g = HomeOverride::new();
    // Empty inbox → empty result.
    let incoming = poll_leader_inbox("alpha").await;
    assert!(incoming.is_empty());
}

#[tokio::test]
async fn poll_leader_inbox_filters_idle_notifications_robust() {
    let _g = HomeOverride::new();
    // Idle notifications are informational; they should be silently
    // marked-read and not surface as conversation injections.
    mailbox::send_idle_notification("alice", None, "alpha", Some("done"), None)
        .await
        .unwrap();
    let incoming = poll_leader_inbox("alpha").await;
    assert!(incoming.is_empty());

    // Underlying message should be marked read.
    let msgs = mailbox::read_mailbox(crate::swarm::TEAM_LEAD_NAME, "alpha").await;
    assert_eq!(msgs.len(), 1);
    assert!(msgs[0].read);
}

#[tokio::test]
async fn poll_leader_inbox_returns_unread_real_messages_normal() {
    let _g = HomeOverride::new();
    // Plain text from a teammate → surfaces as IncomingTeammateMessage.
    mailbox::send_to_leader("alice", "got a result", Some("#FF0000"), "alpha")
        .await
        .unwrap();
    let incoming = poll_leader_inbox("alpha").await;
    assert_eq!(incoming.len(), 1);
    assert_eq!(incoming[0].from, "alice");
    assert_eq!(incoming[0].text, "got a result");
    assert!(incoming[0].formatted.contains("teammate_id=\"alice\""));
    assert!(incoming[0].formatted.contains("got a result"));
    assert_eq!(incoming[0].color.as_deref(), Some("#FF0000"));

    // Subsequent poll yields nothing — message was marked read.
    let incoming2 = poll_leader_inbox("alpha").await;
    assert!(incoming2.is_empty());
}

#[tokio::test]
async fn poll_leader_inbox_skips_already_read_messages_robust() {
    let _g = HomeOverride::new();
    mailbox::send_to_leader("alice", "first", None, "alpha")
        .await
        .unwrap();
    // Mark it read manually.
    mailbox::mark_message_read(crate::swarm::TEAM_LEAD_NAME, "alpha", 0)
        .await
        .unwrap();
    let incoming = poll_leader_inbox("alpha").await;
    assert!(incoming.is_empty());
}

#[tokio::test]
async fn check_task_list_for_work_returns_none_when_no_tasks_robust() {
    let _g = HomeOverride::new();
    let identity = make_identity();
    let result = check_task_list_for_work(&identity, None).await;
    assert!(result.is_none());
}

#[tokio::test]
async fn check_task_list_for_work_claims_pending_unowned_task_normal() {
    let _g = HomeOverride::new();
    let identity = make_identity();

    // Set up a task list with one claimable task.
    let tasks_dir = crate::swarm::team_helpers::tasks_dir(&identity.team_name);
    tokio::fs::create_dir_all(&tasks_dir).await.unwrap();
    let tasks = serde_json::json!([
        {
            "id": "1",
            "subject": "Implement feature X",
            "description": "Use the new API.",
            "status": "pending",
            "owner": "",
            "blockedBy": []
        }
    ]);
    tokio::fs::write(
        tasks_dir.join("tasks.json"),
        serde_json::to_string_pretty(&tasks).unwrap(),
    )
    .await
    .unwrap();

    let (_task_id, prompt) = check_task_list_for_work(&identity, None).await.unwrap();
    assert!(prompt.contains("task #1"));
    assert!(prompt.contains("Implement feature X"));
    assert!(prompt.contains("Use the new API"));

    // The task should be marked in_progress with this teammate as owner.
    let updated = jfc_session::TaskStore::open_team(&identity.team_name)
        .get("1")
        .unwrap();
    assert_eq!(updated.status, jfc_session::TaskStatus::InProgress);
    assert_eq!(updated.owner.as_deref(), Some("alice"));
}

#[tokio::test]
async fn check_task_list_for_work_skips_owned_task_robust() {
    let _g = HomeOverride::new();
    let identity = make_identity();

    let tasks_dir = crate::swarm::team_helpers::tasks_dir(&identity.team_name);
    tokio::fs::create_dir_all(&tasks_dir).await.unwrap();
    let tasks = serde_json::json!([
        {
            "id": "1",
            "subject": "Already taken",
            "status": "pending",
            "owner": "bob",
            "blockedBy": []
        }
    ]);
    tokio::fs::write(
        tasks_dir.join("tasks.json"),
        serde_json::to_string_pretty(&tasks).unwrap(),
    )
    .await
    .unwrap();

    // Task already owned by another agent → no claim.
    assert!(check_task_list_for_work(&identity, None).await.is_none());
}

#[tokio::test]
async fn check_task_list_for_work_skips_blocked_task_robust() {
    let _g = HomeOverride::new();
    let identity = make_identity();

    let tasks_dir = crate::swarm::team_helpers::tasks_dir(&identity.team_name);
    tokio::fs::create_dir_all(&tasks_dir).await.unwrap();
    // Task #2 is blocked by task #1 which is still pending.
    let tasks = serde_json::json!([
        {"id": "1", "subject": "Foundation", "status": "pending", "owner": "bob", "blockedBy": []},
        {"id": "2", "subject": "Depends", "status": "pending", "owner": "", "blockedBy": ["1"]}
    ]);
    tokio::fs::write(
        tasks_dir.join("tasks.json"),
        serde_json::to_string_pretty(&tasks).unwrap(),
    )
    .await
    .unwrap();

    // Neither task is claimable for `alice` (1 owned, 2 blocked).
    assert!(check_task_list_for_work(&identity, None).await.is_none());
}

#[tokio::test]
async fn check_task_list_for_work_picks_up_unblocked_after_completion_normal() {
    let _g = HomeOverride::new();
    let identity = make_identity();

    let tasks_dir = crate::swarm::team_helpers::tasks_dir(&identity.team_name);
    tokio::fs::create_dir_all(&tasks_dir).await.unwrap();
    // Task #1 is completed, so #2 is now unblocked and claimable.
    let tasks = serde_json::json!([
        {"id": "1", "subject": "Done", "status": "completed", "owner": "bob", "blockedBy": []},
        {"id": "2", "subject": "Now ready", "status": "pending", "owner": "", "blockedBy": ["1"]}
    ]);
    tokio::fs::write(
        tasks_dir.join("tasks.json"),
        serde_json::to_string_pretty(&tasks).unwrap(),
    )
    .await
    .unwrap();

    let (_task_id, prompt) = check_task_list_for_work(&identity, None).await.unwrap();
    assert!(prompt.contains("Now ready"));
}

#[tokio::test]
async fn check_task_list_for_work_skips_non_pending_status_robust() {
    let _g = HomeOverride::new();
    let identity = make_identity();

    let tasks_dir = crate::swarm::team_helpers::tasks_dir(&identity.team_name);
    tokio::fs::create_dir_all(&tasks_dir).await.unwrap();
    let tasks = serde_json::json!([
        {"id": "1", "subject": "In flight", "status": "in_progress", "owner": "", "blockedBy": []},
        {"id": "2", "subject": "Done", "status": "completed", "owner": "", "blockedBy": []}
    ]);
    tokio::fs::write(
        tasks_dir.join("tasks.json"),
        serde_json::to_string_pretty(&tasks).unwrap(),
    )
    .await
    .unwrap();

    // No `pending` task → nothing to claim.
    assert!(check_task_list_for_work(&identity, None).await.is_none());
}

#[tokio::test]
async fn check_task_list_for_work_handles_missing_optional_fields_robust() {
    // The JSON parser uses `unwrap_or(...)` for description / subject;
    // a sparse task should still be claimable without panicking.
    let _g = HomeOverride::new();
    let identity = make_identity();

    let tasks_dir = crate::swarm::team_helpers::tasks_dir(&identity.team_name);
    tokio::fs::create_dir_all(&tasks_dir).await.unwrap();
    let tasks = serde_json::json!([
        {"id": "1", "status": "pending"}
    ]);
    tokio::fs::write(
        tasks_dir.join("tasks.json"),
        serde_json::to_string_pretty(&tasks).unwrap(),
    )
    .await
    .unwrap();

    let (_task_id, prompt) = check_task_list_for_work(&identity, None).await.unwrap();
    assert!(prompt.contains("task #1"));
    assert!(prompt.contains("(unnamed task)"));
}

#[test]
fn poll_result_variants_constructable_normal() {
    // Smoke test the enum so coverage hits the variant constructors.
    let _ = PollResult::Aborted;
    let _ = PollResult::TaskAvailable {
        task_id: "1".into(),
        prompt: "do it".into(),
    };
    let _ = PollResult::NewMessage {
        message: "hi".into(),
        from: "leader".into(),
        color: None,
        summary: None,
    };
    let _ = PollResult::ShutdownRequest {
        request: None,
        original_message: "x".into(),
    };
}

/// A no-op provider that returns a single, configurable stream.
/// Used to drive `start_teammate` end-to-end without needing a real
/// API. Each `stream()` call returns the events from `script` once,
/// then errors on subsequent calls so the loop doesn't infinitely
/// re-stream a finished turn.
struct StubProvider {
    script: std::sync::Mutex<Option<Vec<jfc_provider::StreamEvent>>>,
}

impl StubProvider {
    fn new(events: Vec<jfc_provider::StreamEvent>) -> Self {
        Self {
            script: std::sync::Mutex::new(Some(events)),
        }
    }
}

#[async_trait::async_trait]
impl jfc_provider::Provider for StubProvider {
    fn name(&self) -> &str {
        "stub"
    }
    fn available_models(&self) -> Vec<jfc_provider::ModelInfo> {
        vec![jfc_provider::ModelInfo::new(
            "stub-model",
            "Stub Model",
            "stub",
        )]
    }
    async fn stream(
        &self,
        #[allow(dead_code)] messages: Vec<jfc_provider::ProviderMessage>,
        #[allow(dead_code)] options: &jfc_provider::StreamOptions,
    ) -> anyhow::Result<jfc_provider::EventStream> {
        use futures::stream;
        let events = self
            .script
            .lock()
            .unwrap()
            .take()
            .ok_or_else(|| anyhow::anyhow!("StubProvider script exhausted"))?;
        let stream = stream::iter(events.into_iter().map(Ok));
        Ok(Box::pin(stream))
    }
}
impl jfc_provider::seal::Sealed for StubProvider {}

#[tokio::test(flavor = "current_thread")]
async fn start_teammate_completes_after_endturn_normal() {
    // Drive a single full agent turn through the runner: text delta +
    // EndTurn (no tools), then immediately abort so the post-turn idle
    // loop exits without polling forever.
    let _g = HomeOverride::new();

    use jfc_provider::{StopReason, StreamEvent};
    let provider: std::sync::Arc<dyn jfc_provider::Provider> =
        std::sync::Arc::new(StubProvider::new(vec![
            StreamEvent::TextDelta {
                index: 0,
                delta: "hi".into(),
            },
            StreamEvent::Done {
                stop_reason: StopReason::EndTurn,
            },
        ]));

    let identity = make_identity();
    let config = TeammateRunnerConfig {
        identity: identity.clone(),
        prompt: "hello".into(),
        description: "test".into(),
        model: None,
        agent_type: None,
        provider,
        model_id: jfc_provider::ModelId::new("stub-model"),
        system_prompt: Some("be brief".into()),
        task_store: None,
    };

    let (event_tx, mut event_rx) = tokio::sync::mpsc::unbounded_channel();
    let (task_id, abort_tx) = start_teammate(config, event_tx);
    assert_eq!(task_id, "teammate-alice@alpha");

    // Give the loop a moment to run the turn, then abort.
    // The stub will exhaust on the second `stream()` call (after Idle).
    // Either way, abort_tx forces a clean exit.
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    let _ = abort_tx.send(true);

    // Drain events with a tight overall timeout.
    let mut got_text_delta = false;
    let mut got_idle = false;
    let mut got_terminal = false;
    let drain = async {
        while let Some(ev) = event_rx.recv().await {
            match ev {
                TeammateEvent::TextDelta { delta, .. } => {
                    if delta == "hi" {
                        got_text_delta = true;
                    }
                }
                TeammateEvent::Idle { .. } => got_idle = true,
                TeammateEvent::Completed { .. }
                | TeammateEvent::Failed { .. }
                | TeammateEvent::Cancelled { .. } => {
                    got_terminal = true;
                    break;
                }
                _ => {}
            }
        }
    };
    let _ = tokio::time::timeout(std::time::Duration::from_secs(2), drain).await;

    assert!(got_text_delta, "expected text delta from stub stream");
    assert!(got_idle, "expected idle event after first turn");
    assert!(got_terminal, "expected Completed or Failed terminal event");
}

/// Make a stub provider that returns multiple scripts in sequence, one
/// per `stream()` call. After the last script is consumed, subsequent
/// calls error.
struct ScriptedProvider {
    scripts: std::sync::Mutex<std::collections::VecDeque<Vec<jfc_provider::StreamEvent>>>,
}

impl ScriptedProvider {
    fn new(scripts: Vec<Vec<jfc_provider::StreamEvent>>) -> Self {
        Self {
            scripts: std::sync::Mutex::new(scripts.into_iter().collect()),
        }
    }
}

#[async_trait::async_trait]
impl jfc_provider::Provider for ScriptedProvider {
    fn name(&self) -> &str {
        "scripted"
    }
    fn available_models(&self) -> Vec<jfc_provider::ModelInfo> {
        vec![]
    }
    async fn stream(
        &self,
        #[allow(dead_code)] messages: Vec<jfc_provider::ProviderMessage>,
        #[allow(dead_code)] options: &jfc_provider::StreamOptions,
    ) -> anyhow::Result<jfc_provider::EventStream> {
        use futures::stream;
        let next = self
            .scripts
            .lock()
            .unwrap()
            .pop_front()
            .ok_or_else(|| anyhow::anyhow!("scripts exhausted"))?;
        Ok(Box::pin(stream::iter(next.into_iter().map(Ok))))
    }
}
impl jfc_provider::seal::Sealed for ScriptedProvider {}

#[tokio::test(flavor = "current_thread")]
async fn start_teammate_executes_tool_then_endturn_normal() {
    // Drive a full tool-use cycle. The first stream returns a tool call
    // (LS — read-only, no side effects), then the runner re-streams; the
    // second script returns text + EndTurn. After idle, we abort to end
    // the loop quickly. This exercises the run_single_turn tool-execution
    // path including Usage / TextDone / ToolDone events.
    let _g = HomeOverride::new();

    use jfc_provider::{StopReason, StreamEvent};

    // Pick a tool that exists and is benign. `Read` requires a path; we
    // pass a non-existent one — `tools::execute_tool` returns an error
    // result but the runner appends it as a tool_result (is_error: true)
    // and continues. That keeps the test hermetic.
    let tool_input = serde_json::json!({"file_path": "/nonexistent-path-for-test"});
    let tool_input_json = tool_input.to_string();

    let scripts = vec![
        vec![
            StreamEvent::Usage {
                input_tokens: 5,
                output_tokens: 3,
                cache_read_tokens: 0,
                cache_write_tokens: 0,
            },
            StreamEvent::ToolDone {
                index: 0,
                tool_name: "Read".into(),
                tool_use_id: "call-1".into(),
                input_json: tool_input_json,
                thought_signature: None,
            },
            StreamEvent::Done {
                stop_reason: StopReason::ToolUse,
            },
        ],
        vec![
            StreamEvent::TextDelta {
                index: 0,
                delta: "ok".into(),
            },
            StreamEvent::Done {
                stop_reason: StopReason::EndTurn,
            },
        ],
    ];

    let provider: std::sync::Arc<dyn jfc_provider::Provider> =
        std::sync::Arc::new(ScriptedProvider::new(scripts));

    let config = TeammateRunnerConfig {
        identity: make_identity(),
        prompt: "do thing".into(),
        description: "test".into(),
        model: None,
        agent_type: None,
        provider,
        model_id: jfc_provider::ModelId::new("stub-model"),
        system_prompt: None,
        task_store: None,
    };

    let (event_tx, mut event_rx) = tokio::sync::mpsc::unbounded_channel();
    let (_task_id, abort_tx) = start_teammate(config, event_tx);

    tokio::time::sleep(std::time::Duration::from_millis(80)).await;
    let _ = abort_tx.send(true);

    let mut got_progress_with_tool = false;
    let mut got_terminal = false;
    let drain = async {
        while let Some(ev) = event_rx.recv().await {
            match ev {
                TeammateEvent::Progress {
                    last_tool: Some(t), ..
                } if t == "Read" => {
                    got_progress_with_tool = true;
                }
                TeammateEvent::Completed { .. }
                | TeammateEvent::Failed { .. }
                | TeammateEvent::Cancelled { .. } => {
                    got_terminal = true;
                    break;
                }
                _ => {}
            }
        }
    };
    let _ = tokio::time::timeout(std::time::Duration::from_secs(3), drain).await;
    assert!(
        got_progress_with_tool,
        "expected Progress event with last_tool=Read"
    );
    assert!(got_terminal);
}

#[tokio::test(flavor = "current_thread")]
async fn run_single_turn_retries_retryable_stream_error_normal() {
    let _g = HomeOverride::new();

    use jfc_provider::{StopReason, StreamEvent};
    let scripts = vec![
        vec![StreamEvent::Error {
            message: format!(
                "{}Anthropic transient API error 529: overloaded",
                crate::providers::anthropic::AUTO_RETRY_SENTINEL
            ),
        }],
        vec![
            StreamEvent::TextDelta {
                index: 0,
                delta: "ok".into(),
            },
            StreamEvent::Done {
                stop_reason: StopReason::EndTurn,
            },
        ],
    ];

    let provider: std::sync::Arc<dyn jfc_provider::Provider> =
        std::sync::Arc::new(ScriptedProvider::new(scripts));
    let config = TeammateRunnerConfig {
        identity: make_identity(),
        prompt: "go".into(),
        description: "test".into(),
        model: None,
        agent_type: None,
        provider,
        model_id: jfc_provider::ModelId::new("stub-model"),
        system_prompt: None,
        task_store: None,
    };
    let (event_tx, mut event_rx) = tokio::sync::mpsc::unbounded_channel();
    let (_abort_tx, mut abort_rx) = tokio::sync::watch::channel(false);
    let mut history = Vec::new();

    let result = run_single_turn(
        &config,
        "go",
        &mut history,
        &event_tx,
        "task",
        &mut abort_rx,
    )
    .await;

    assert!(
        matches!(result, TurnResult::Completed { .. }),
        "turn should recover, got {result:?}"
    );
    let mut saw_ok = false;
    while let Ok(event) = event_rx.try_recv() {
        if let TeammateEvent::TextDelta { delta, .. } = event {
            saw_ok |= delta == "ok";
        }
    }
    assert!(saw_ok, "expected recovered text delta");
    assert_eq!(
        history.len(),
        2,
        "retry should not append a failed assistant turn"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn start_teammate_picks_up_leader_message_after_idle_normal() {
    // First turn streams text + EndTurn. While the loop is idle, we plant
    // a leader message in alice's mailbox. The runner picks it up via
    // poll_for_next_message → priority-2 (leader) branch → triggers a
    // second turn. We then abort. Exercises the leader-message branch
    // and the second-iteration prompt update.
    let _g = HomeOverride::new();

    use jfc_provider::{StopReason, StreamEvent};

    // Two scripts: one per turn. Both end with EndTurn (no tools).
    let scripts = vec![
        vec![
            StreamEvent::TextDelta {
                index: 0,
                delta: "first".into(),
            },
            StreamEvent::Done {
                stop_reason: StopReason::EndTurn,
            },
        ],
        vec![
            StreamEvent::TextDelta {
                index: 0,
                delta: "second".into(),
            },
            StreamEvent::Done {
                stop_reason: StopReason::EndTurn,
            },
        ],
    ];

    let identity = make_identity();
    let provider: std::sync::Arc<dyn jfc_provider::Provider> =
        std::sync::Arc::new(ScriptedProvider::new(scripts));

    let config = TeammateRunnerConfig {
        identity: identity.clone(),
        prompt: "go".into(),
        description: "test".into(),
        model: None,
        agent_type: None,
        provider,
        model_id: jfc_provider::ModelId::new("stub-model"),
        system_prompt: None,
        task_store: None,
    };

    let (event_tx, mut event_rx) = tokio::sync::mpsc::unbounded_channel();
    let (_task_id, abort_tx) = start_teammate(config, event_tx);

    // Wait for the first idle, then plant a follow-up leader message.
    let mut idle_count = 0u32;
    let mut second_text = false;
    let mut terminal = false;
    let drain = async {
        while let Some(ev) = event_rx.recv().await {
            match ev {
                TeammateEvent::Idle { .. } => {
                    idle_count += 1;
                    if idle_count == 1 {
                        // Plant leader message after first idle so the
                        // poll loop picks it up on its next tick.
                        mailbox::write_to_mailbox(
                            &identity.agent_name,
                            crate::swarm::types::MailboxMessage {
                                from: crate::swarm::TEAM_LEAD_NAME.into(),
                                text: "next prompt".into(),
                                timestamp: "t".into(),
                                color: None,
                                summary: None,
                                read: false,
                            },
                            &identity.team_name,
                        )
                        .await
                        .unwrap();
                    } else {
                        // After the second idle, abort to end the loop.
                        let _ = abort_tx.send(true);
                    }
                }
                TeammateEvent::TextDelta { delta, .. } => {
                    if delta == "second" {
                        second_text = true;
                    }
                }
                TeammateEvent::Completed { .. }
                | TeammateEvent::Failed { .. }
                | TeammateEvent::Cancelled { .. } => {
                    terminal = true;
                    break;
                }
                _ => {}
            }
        }
    };
    let _ = tokio::time::timeout(std::time::Duration::from_secs(5), drain).await;
    assert!(idle_count >= 2, "expected at least 2 idle events");
    assert!(second_text, "expected second turn to stream `second`");
    assert!(terminal);
}

#[tokio::test(flavor = "current_thread")]
async fn start_teammate_failed_when_provider_errors_robust() {
    // First stream call exhausts the (empty) script → provider errors →
    // run_single_turn returns TurnResult::Error. The loop logs and continues
    // to idle. Then we abort to force termination, and the runner reports
    // Completed (graceful, since errors don't abort the loop).
    let _g = HomeOverride::new();

    let provider: std::sync::Arc<dyn jfc_provider::Provider> =
        std::sync::Arc::new(StubProvider::new(vec![]));

    let config = TeammateRunnerConfig {
        identity: make_identity(),
        prompt: "hi".into(),
        description: "test".into(),
        model: None,
        agent_type: None,
        provider,
        model_id: jfc_provider::ModelId::new("stub-model"),
        system_prompt: None,
        task_store: None,
    };

    let (event_tx, mut event_rx) = tokio::sync::mpsc::unbounded_channel();
    let (_task_id, abort_tx) = start_teammate(config, event_tx);

    // Let the loop hit its first stream error, then abort.
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    let _ = abort_tx.send(true);

    let mut saw_terminal = false;
    let drain = async {
        while let Some(ev) = event_rx.recv().await {
            if matches!(
                ev,
                TeammateEvent::Completed { .. }
                    | TeammateEvent::Failed { .. }
                    | TeammateEvent::Cancelled { .. }
            ) {
                saw_terminal = true;
                break;
            }
        }
    };
    let _ = tokio::time::timeout(std::time::Duration::from_secs(2), drain).await;
    assert!(saw_terminal);
}

// Regression: dropping the abort_tx must emit Cancelled, NOT Completed.
//
// This is the smoking-gun test for the "all teammates marked Done"
// bug. `start_teammate` returns a `watch::Sender<bool>`; if a caller
// drops it (the original `stream.rs:1962` bug — leading underscore
// made it look like an intentional bind), `watch::Receiver::changed()`
// immediately resolves Err and the runner's `tokio::select! { biased; ...}`
// returns `TurnResult::Aborted` on the FIRST stream poll. The old
// path then ran `Ok(())` → `TeammateEvent::Completed`, which the UI
// rendered as ": Done" before the teammate did any work.
//
// After the fix, the runner returns `Ok(TeammateExit::Cancelled)`
// and start_teammate emits `TeammateEvent::Cancelled`. Verifies the
// distinction at the event-stream level.
#[tokio::test(flavor = "current_thread")]
async fn dropping_abort_tx_emits_cancelled_not_completed_normal() {
    let _g = HomeOverride::new();

    // Provider script: a single text delta then EndTurn — plenty of
    // work for the runner to actually do if it weren't aborted.
    let provider: std::sync::Arc<dyn jfc_provider::Provider> =
        std::sync::Arc::new(StubProvider::new(vec![
            jfc_provider::StreamEvent::TextDelta {
                index: 0,
                delta: "hello".into(),
            },
            jfc_provider::StreamEvent::Done {
                stop_reason: jfc_provider::StopReason::EndTurn,
            },
        ]));
    let identity = make_identity();
    let config = TeammateRunnerConfig {
        identity,
        prompt: "p".into(),
        description: "d".into(),
        model: None,
        agent_type: None,
        provider,
        model_id: jfc_provider::ModelId::new("stub-model"),
        system_prompt: None,
        task_store: None,
    };
    let (event_tx, mut event_rx) = tokio::sync::mpsc::unbounded_channel();
    let (_task_id, abort_tx) = start_teammate(config, event_tx);
    // Drop the abort handle IMMEDIATELY — this is the original bug.
    drop(abort_tx);

    let mut last_terminal: Option<&'static str> = None;
    let drain = async {
        while let Some(ev) = event_rx.recv().await {
            match ev {
                TeammateEvent::Completed { .. } => {
                    last_terminal = Some("Completed");
                    break;
                }
                TeammateEvent::Cancelled { .. } => {
                    last_terminal = Some("Cancelled");
                    break;
                }
                TeammateEvent::Failed { .. } => {
                    last_terminal = Some("Failed");
                    break;
                }
                _ => {}
            }
        }
    };
    let _ = tokio::time::timeout(std::time::Duration::from_secs(2), drain).await;
    assert_eq!(
        last_terminal,
        Some("Cancelled"),
        "dropping abort_tx must surface as Cancelled, not Completed — \
         see TeammateInfo.abort_tx and stream.rs spawn site"
    );
}

#[test]
fn teammate_event_variants_serialize_through_debug_normal() {
    // Coverage for each TeammateEvent variant.
    let events = vec![
        TeammateEvent::Idle {
            task_id: "t".into(),
            agent_id: "a".into(),
            agent_name: "alice".into(),
            reason: None,
            summary: None,
        },
        TeammateEvent::Progress {
            task_id: "t".into(),
            agent_id: "a".into(),
            token_count: 0,
            tool_use_count: 0,
            last_tool: None,
            model_id: None,
            cost_usd: None,
        },
        TeammateEvent::Completed {
            task_id: "t".into(),
            agent_id: "a".into(),
        },
        TeammateEvent::Cancelled {
            task_id: "t".into(),
            agent_id: "a".into(),
        },
        TeammateEvent::Failed {
            task_id: "t".into(),
            agent_id: "a".into(),
            error: "e".into(),
        },
        TeammateEvent::MessageSent {
            from: "alice".into(),
            to: "team-lead".into(),
            text: "ok".into(),
            summary: None,
        },
        TeammateEvent::TextDelta {
            task_id: "t".into(),
            agent_id: "a".into(),
            delta: "x".into(),
        },
    ];
    for ev in &events {
        // Just exercise the Debug impl.
        let s = format!("{ev:?}");
        assert!(!s.is_empty());
    }
}
