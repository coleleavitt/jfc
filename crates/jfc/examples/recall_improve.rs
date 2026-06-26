//! Close the first verifiable self-improvement loop: measure recall coverage,
//! apply the fix (promote verified findings to global), re-measure, confirm lift.
//!   cargo run -p jfc --example recall_improve
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let store = jfc_knowledge::KnowledgeStore::open_default().await?;
    let (n0, p0) = store.run_recall_coverage_eval().await?;
    eprintln!(
        "[BEFORE] recall coverage: {p0}/{n0} = {:.0}%",
        100.0 * p0 as f64 / n0.max(1) as f64
    );
    let promoted = store.promote_verified_findings_to_global().await?;
    eprintln!("[FIX]    promoted {promoted} verified findings to global (cross-project recall)");
    let (n1, p1) = store.run_recall_coverage_eval().await?;
    eprintln!(
        "[AFTER]  recall coverage: {p1}/{n1} = {:.0}%",
        100.0 * p1 as f64 / n1.max(1) as f64
    );
    let lift = 100.0 * p1 as f64 / n1.max(1) as f64 - 100.0 * p0 as f64 / n0.max(1) as f64;
    eprintln!("[CONFIRM] lift = +{lift:.0} percentage points ({p0}→{p1} of {n1})");
    Ok(())
}
