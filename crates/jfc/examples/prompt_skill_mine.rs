//! Mine recurring user-prompt intents across all sessions → skill suggestions on
//! the self-improvement backlog.  cargo run -p jfc --example prompt_skill_mine
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let store = jfc_knowledge::KnowledgeStore::open_default().await?;
    let n = jfc_engine::mine_user_prompt_skills_from_store(&store, 5).await;
    eprintln!("[prompt-mine] wrote {n} recurring-request skill suggestions to the backlog");
    Ok(())
}
