use std::path::Path;

use crate::tools::ExecutionResult;

mod audit;
mod health;

pub fn execute_learn_rsi_list(
    project_root: &Path,
    status: Option<&str>,
    limit: Option<u64>,
) -> ExecutionResult {
    let project_key = jfc_knowledge::project_key(project_root);
    let result = jfc_knowledge::block_on_knowledge(async {
        let store = jfc_knowledge::KnowledgeStore::open_default().await?;
        audit::render_rsi_audit(&store, &project_key, status, limit).await
    });
    match result {
        Ok(report) => ExecutionResult::success(report),
        Err(error) => ExecutionResult::failure(format!("RSI list failed: {error}")),
    }
}

pub fn execute_learn_rsi_promote(project_root: &Path, kind: &str, name: &str) -> ExecutionResult {
    let project_key = jfc_knowledge::project_key(project_root);
    let definition = jfc_learn::RsiDefinitionRef::new(kind, name);
    let result = jfc_knowledge::block_on_knowledge(async {
        let store = jfc_knowledge::KnowledgeStore::open_default().await?;
        jfc_learn::promote_rsi_definition(&store, &project_key, &definition).await
    });
    match result {
        Ok(report) => ExecutionResult::success(format!(
            "RSI definition promoted: {}/{} status={} action={}",
            report.kind,
            report.name,
            report.status,
            report.action.slug()
        )),
        Err(error) => ExecutionResult::failure(format!("RSI promotion failed: {error}")),
    }
}

pub fn execute_learn_rsi_rollback(project_root: &Path, kind: &str, name: &str) -> ExecutionResult {
    let project_key = jfc_knowledge::project_key(project_root);
    let definition = jfc_learn::RsiDefinitionRef::new(kind, name);
    let result = jfc_knowledge::block_on_knowledge(async {
        let store = jfc_knowledge::KnowledgeStore::open_default().await?;
        jfc_learn::rollback_rsi_definition(&store, &project_key, &definition).await
    });
    match result {
        Ok(report) => ExecutionResult::success(format!(
            "RSI definition rolled back: {}/{} status={} action={}",
            report.kind,
            report.name,
            report.status,
            report.action.slug()
        )),
        Err(error) => ExecutionResult::failure(format!("RSI rollback failed: {error}")),
    }
}

#[cfg(test)]
mod tests;
