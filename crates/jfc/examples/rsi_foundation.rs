//! Build + RUN the RSI foundation over the live DB: seed eval suite, run the
//! deterministic recall-coverage eval, consolidate prefs, seed the curriculum.
//!   cargo run -p jfc --example rsi_foundation
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let store = jfc_knowledge::KnowledgeStore::open_default().await?;
    let seeded = store.seed_eval_cases_from_findings().await?;
    let total = store.eval_case_count().await?;
    eprintln!("[eval] +{seeded} eval cases (held-out suite now {total})");
    let (n, passed) = store.run_recall_coverage_eval().await?;
    let pct = if n > 0 {
        100.0 * passed as f64 / n as f64
    } else {
        0.0
    };
    eprintln!(
        "[eval-RUN] recall-coverage: {passed}/{n} known failure modes have a recallable lesson ({pct:.0}%)"
    );
    let collapsed = store.consolidate_duplicate_preferences().await?;
    eprintln!("[consolidate] collapsed {collapsed} duplicate preferences");
    let gaps = store.seed_knowledge_gaps_from_failures(10).await?;
    eprintln!("[curriculum] {gaps} knowledge gaps recorded");
    Ok(())
}
