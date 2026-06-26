//! Backfill the self-critique loop over ALL existing sessions in the knowledge
//! DB — retroactively populate the self-improvement backlog + reasoning/output
//! lessons + staged candidates (heuristic judge, no LLM / no network). Opening
//! the store applies migration v11 (creates `improvement_backlog`).
//!
//!   cargo run -p jfc --example self_critique_backfill
//!
//! Idempotent: lessons/definitions/backlog all dedup, so re-running just bumps
//! recurrence (evidence) rather than duplicating rows.

use std::path::Path;
use std::time::Instant;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let store = jfc_knowledge::KnowledgeStore::open_default().await?;
    let sessions = store.list_sessions(None, 1_000_000).await?;
    eprintln!("[backfill] scanning {} sessions…", sessions.len());

    let t0 = Instant::now();
    let (mut critiqued, mut proposals, mut lessons, mut defs) = (0usize, 0usize, 0usize, 0usize);
    for (i, s) in sessions.iter().enumerate() {
        let Some(cwd) = s.cwd.as_deref() else {
            continue;
        };
        let project_key = jfc_knowledge::project_key(Path::new(cwd));
        let messages = match store.load_transcript(&s.id).await {
            Ok(m) => m,
            Err(_) => continue,
        };
        let (p, l, d) =
            jfc_engine::run_self_critique_pass(&store, &project_key, &s.id, &messages).await;
        if p > 0 {
            critiqued += 1;
        }
        proposals += p;
        lessons += l;
        defs += d;
        if i.is_multiple_of(50) {
            eprintln!("[backfill] {}/{} …", i, sessions.len());
        }
    }
    // Evidence-gated promotion: graduate well-recurring candidates to ACTIVE so
    // they take effect in the live prompt.
    let promoted = jfc_engine::promote_evidenced_self_critique(
        &store,
        jfc_engine::SELF_CRITIQUE_PROMOTE_MIN_RECURRENCE,
    )
    .await;
    eprintln!(
        "[backfill] done in {:?}: {critiqued} sessions yielded critique · {proposals} proposals · \
         {lessons} lessons folded · {defs} candidates staged · {promoted} PROMOTED TO ACTIVE",
        t0.elapsed()
    );
    Ok(())
}
