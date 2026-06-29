use super::test_support::{seat, seat_seq};
use super::*;

#[tokio::test]
async fn blind_map_reduce_runs_full_pipeline_normal() {
    let mut session = CouncilSession::new(
        "Pick the cache backend?",
        CouncilSessionMode::BlindMapReduce,
        vec![
            seat_seq(
                "a",
                "Alpha",
                vec![
                    "VERDICT: HIGH\nWHY: a wrong backend choice is costly",
                    "## Answer\nRedis\n\n## Self-critique\n- Memory cost\n- Ops risk\n\n## Confidence\nRedis: Medium",
                    "## Final answer\nChoose Redis.\n\n## Dissent ledger\nPostgres dissent noted.\n\n## What changed and why\nRed-team caught durability risk.",
                    "Solo baseline says Redis.",
                ],
            ),
            seat_seq(
                "b",
                "Beta",
                vec![
                    "## Answer\nPostgres\n\n## Self-critique\n- Latency\n- Locking\n\n## Confidence\nPostgres: Medium",
                    "## Contradictions\nRedis vs Postgres.\n\n## Shared blind spots\nNo workload numbers.\n\n## Overconfident or unsourced claims\nLatency claims.",
                ],
            ),
            seat_seq(
                "c",
                "Gamma",
                vec![
                    "## Answer\nSQLite\n\n## Self-critique\n- Concurrency\n- Scale\n\n## Confidence\nSQLite: Low",
                ],
            ),
        ],
    );
    session.start();

    let report = session.run_blind_map_reduce(true).await.unwrap();

    assert_eq!(report.blind_answer_count, 3);
    assert!(session.is_concluded());
    let transcript = session.to_markdown();
    assert!(transcript.contains("BLIND MAP-REDUCE"));
    assert!(transcript.contains("Phase 1 · Blind answer A"));
    assert!(transcript.contains("Phase 2 · Red-team report"));
    assert!(transcript.contains("Final answer"));
    assert!(transcript.contains("Baseline"));
}

#[tokio::test]
async fn governance_directives_stage_until_operator_approval_robust() {
    let mut session = CouncilSession::new(
        "Should we ship?",
        CouncilSessionMode::Debate,
        vec![
            seat_seq(
                "a",
                "Alpha",
                vec![
                    "You need to answer this.\nCHALLENGE: Beta | What breaks at 100x?",
                    "ignored",
                ],
            ),
            seat("b", "Beta", "It breaks on latency.\nSTANCE: AGAINST | 80"),
        ],
    );
    session.start();

    let (_turn, notes) = session.run_current_turn().await.unwrap();

    assert!(
        notes.iter().any(|n| n.contains("approval")),
        "notes: {notes:?}"
    );
    assert!(session.pending_action().is_some());
    assert!(!session.to_markdown().contains("CHALLENGE — Alpha to Beta"));

    let approved = session.approve_pending_action().await.unwrap();

    assert!(
        approved.iter().any(|n| n.contains("approved")),
        "approved: {approved:?}"
    );
    assert!(session.to_markdown().contains("CHALLENGE — Alpha to Beta"));
    assert_eq!(session.current_speaker.as_deref(), Some("b"));
}

#[tokio::test]
async fn image_directive_is_visible_unsupported_terminal_action_normal() {
    let mut session = CouncilSession::new(
        "Show the architecture.",
        CouncilSessionMode::Debate,
        vec![
            seat_seq(
                "a",
                "Alpha",
                vec!["A diagram would help.\nGENERATE IMAGE: system diagram, 16:9"],
            ),
            seat("b", "Beta", "ok"),
        ],
    );
    session.start();

    let (_turn, notes) = session.run_current_turn().await.unwrap();

    assert!(
        notes.iter().any(|n| n.contains("Image requested")),
        "notes: {notes:?}"
    );
    let transcript = session.to_markdown();
    assert!(!transcript.contains("GENERATE IMAGE"));
    assert!(transcript.contains("IMAGE REQUEST"));
}
