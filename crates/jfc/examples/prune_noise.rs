//! Prune harness-noise preferences from the knowledge ledger.
//!   cargo run -p jfc --example prune_noise
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let store = jfc_knowledge::KnowledgeStore::open_default().await?;
    let n = store.prune_noisy_preferences().await?;
    eprintln!("[prune] superseded {n} harness-noise preferences (reversible)");
    Ok(())
}
