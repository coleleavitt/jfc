use super::*;

#[test]
fn transcript_trace_extracts_reasoning_tools_correction_and_verification_normal() {
    let messages = vec![
        jfc_knowledge::SessionMessage {
            seq: 0,
            role: "assistant".to_owned(),
            content: "Reasoning text Bash cargo test".to_owned(),
            meta: Some(
                serde_json::json!({
                    "role": "assistant",
                    "model_name": "claude-test",
                    "usage": { "thinking_tokens": 42 },
                    "parts": [
                        { "type": "reasoning", "content": "raw thought" },
                        {
                            "type": "tool",
                            "kind": "Bash",
                            "status": "complete",
                            "input": { "type": "bash", "command": "cargo test -p jfc-learn" }
                        }
                    ]
                })
                .to_string(),
            ),
        },
        jfc_knowledge::SessionMessage {
            seq: 1,
            role: "user".to_owned(),
            content: "actually run clippy too".to_owned(),
            meta: None,
        },
    ];

    let trace = trace_from_messages("s1", &messages);

    assert_eq!(trace.model.as_deref(), Some("claude-test"));
    assert_eq!(trace.thinking_blocks.len(), 1);
    assert_eq!(trace.thinking_tokens, 42);
    assert_eq!(trace.tool_steps.len(), 1);
    assert_eq!(trace.verifications.len(), 1);
    assert_eq!(trace.outcome, Some(RsiOutcome::UserCorrected));
    assert!(trace.user_correction.is_some());
}

#[tokio::test]
async fn build_recent_rsi_job_respects_future_loop_due_state_normal() {
    let store = jfc_knowledge::KnowledgeStore::open_in_memory()
        .await
        .unwrap();
    let cwd = "/tmp/jfc-rsi-loop";
    store
        .replace_transcript(
            &jfc_knowledge::SessionRow {
                id: "s1".to_owned(),
                cwd: Some(cwd.to_owned()),
                model: Some("claude-test".to_owned()),
                created_at: Some("2026-01-01T00:00:00Z".to_owned()),
                updated_at: Some("2026-01-01T00:01:00Z".to_owned()),
                first_prompt: Some("fix".to_owned()),
                title: Some("RSI fixture".to_owned()),
                message_count: 1,
            },
            &[jfc_knowledge::SessionMessage {
                seq: 0,
                role: "assistant".to_owned(),
                content: "ran cargo test".to_owned(),
                meta: Some(
                    serde_json::json!({
                        "parts": [{
                            "type": "tool",
                            "kind": "Bash",
                            "status": "complete",
                            "input": { "command": "cargo test -p jfc-learn" }
                        }]
                    })
                    .to_string(),
                ),
            }],
        )
        .await
        .unwrap();
    let project_key = jfc_knowledge::project_key(std::path::Path::new(cwd));
    store
        .upsert_definition(&jfc_knowledge::NewDefinition {
            kind: crate::rsi_curator::RSI_LOOP_STATE_KIND.to_owned(),
            scope: jfc_knowledge::DefinitionScope::Project,
            project_key: Some(project_key),
            namespace: None,
            name: crate::rsi_curator::RSI_LOOP_STATE_NAME.to_owned(),
            title: None,
            description: None,
            body: "future".to_owned(),
            metadata_json: serde_json::json!({
                "rsi": {
                    "experiment_loop_state": {
                        "run_count": 1,
                        "last_run_at_ms": 1,
                        "next_due_at_ms": u64::MAX,
                        "cadence_seconds": 900,
                        "phase": "branch",
                        "preflight_status": "ready",
                        "candidate_actions": 1,
                        "traces_scored": 1,
                        "candidates_seen": 1,
                        "total_estimated_tokens": 1,
                        "latest_score_milli": 1,
                        "best_score_milli": 1
                    }
                }
            })
            .to_string(),
            source_path: None,
            source_hash: None,
            status: jfc_knowledge::DefinitionStatus::Active,
            created_by: "test".to_owned(),
        })
        .await
        .unwrap();

    let job = build_recent_rsi_job(
        &store,
        Some(cwd),
        50,
        RsiCuratorConfig::default(),
        RsiPromotionPolicy::default(),
    )
    .await
    .unwrap();

    assert!(job.is_none());
}

#[tokio::test]
async fn load_trace_uses_durable_retrievals_without_tool_duplication_normal() {
    let store = jfc_knowledge::KnowledgeStore::open_in_memory()
        .await
        .unwrap();
    store
        .replace_transcript(
            &jfc_knowledge::SessionRow {
                id: "s1".to_owned(),
                cwd: Some("/tmp/jfc-rsi-retrieval".to_owned()),
                model: Some("claude-test".to_owned()),
                created_at: Some("2026-01-01T00:00:00Z".to_owned()),
                updated_at: Some("2026-01-01T00:01:00Z".to_owned()),
                first_prompt: Some("fix".to_owned()),
                title: Some("RSI retrieval fixture".to_owned()),
                message_count: 1,
            },
            &[jfc_knowledge::SessionMessage {
                seq: 0,
                role: "assistant".to_owned(),
                content: "ran cargo test".to_owned(),
                meta: Some(
                    serde_json::json!({
                        "parts": [{
                            "type": "tool",
                            "kind": "Bash",
                            "status": "complete",
                            "input": { "command": "cargo test -p jfc-learn" }
                        }]
                    })
                    .to_string(),
                ),
            }],
        )
        .await
        .unwrap();
    store
        .record_retrieval_event(&jfc_knowledge::SessionRetrievalEvent {
            id: "ret_1".to_owned(),
            session_id: "s1".to_owned(),
            query: "current rsi extractor".to_owned(),
            source: "codegraph".to_owned(),
            result_count: 4,
            payload: "{}".to_owned(),
            created_at_ms: 1,
        })
        .await
        .unwrap();

    let trace = load_trace_from_store(&store, "s1").await.unwrap();

    assert_eq!(trace.tool_steps.len(), 1);
    assert_eq!(trace.verifications.len(), 1);
    assert_eq!(trace.retrieval_steps.len(), 1);
    assert_eq!(trace.retrieval_steps[0].source, "codegraph");
    assert_eq!(trace.outcome, Some(RsiOutcome::Succeeded));
}

#[tokio::test]
async fn build_recent_rsi_job_includes_project_bounty_activity_normal() {
    let store = jfc_knowledge::KnowledgeStore::open_in_memory()
        .await
        .unwrap();
    let cwd = "/home/cole/RustProjects/active/jfc";
    let project_key = jfc_knowledge::project_key(std::path::Path::new(cwd));
    let session_id = format!("project:{project_key}");
    store
        .record_agent_event(&jfc_knowledge::AgentEventRow {
            id: "evt_dispatch".to_owned(),
            session_id: session_id.clone(),
            from_agent: None,
            to_agent: None,
            kind: "bounty.dispatch_started".to_owned(),
            content: serde_json::json!({
                "bounty_id": "bounty_1",
                "kind": "dispatch_started",
                "payload": { "n_solvers": 3 }
            })
            .to_string(),
            turn_id: None,
            causal_parent_id: None,
            created_at_ms: 1,
        })
        .await
        .unwrap();
    store
        .record_agent_event(&jfc_knowledge::AgentEventRow {
            id: "evt_settled".to_owned(),
            session_id,
            from_agent: None,
            to_agent: None,
            kind: "bounty.settled".to_owned(),
            content: serde_json::json!({
                "bounty_id": "bounty_1",
                "kind": "settled",
                "payload": {
                    "winner": "solver_a",
                    "payouts": 3
                }
            })
            .to_string(),
            turn_id: None,
            causal_parent_id: None,
            created_at_ms: 2,
        })
        .await
        .unwrap();

    let job = build_recent_rsi_job(
        &store,
        Some(cwd),
        50,
        RsiCuratorConfig::default(),
        RsiPromotionPolicy::default(),
    )
    .await
    .unwrap()
    .expect("project activity creates an RSI job");

    let trace = job
        .traces
        .iter()
        .find(|trace| trace.session_id.starts_with("project:"))
        .expect("project activity trace");
    assert_eq!(trace.agent_fanouts[0].count, 3);
    assert_eq!(trace.selections[0].winner.as_deref(), Some("solver_a"));
    assert_eq!(trace.outcome, Some(RsiOutcome::Succeeded));
}
