mod model;
mod render;
mod search;
mod seen;
#[cfg(test)]
mod tests;

use jfc_provider::ProviderMessage;
use model::{ArchivedProviderMessage, ProviderHistoryArchive};
use render::render_archive;
pub(crate) use search::{
    ProviderHistoryArchiveHit, list_provider_history_archives,
    provider_history_archive_recall_block, search_provider_history_archives,
    search_provider_history_archives_in,
};
pub(crate) use seen::{
    load_session_provider_history_archive_seen, persist_session_provider_history_archive_seen,
};
use sha2::{Digest, Sha256};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

const ARCHIVE_SCHEMA_VERSION: u32 = 1;
const PROVIDER_HISTORY_ARCHIVE_KIND: &str = "provider_history_archive";

#[derive(Debug, Clone)]
pub(crate) struct ProviderHistoryArchiveMeta {
    pub(crate) id: String,
    pub(crate) path: PathBuf,
    pub(crate) message_count: usize,
}

pub(crate) fn archive_provider_history_current_project(
    messages: &[ProviderMessage],
    pre_tokens: u64,
    summary: &str,
) -> std::io::Result<Option<ProviderHistoryArchiveMeta>> {
    if messages.is_empty() {
        return Ok(None);
    }
    let root = std::env::current_dir()?;
    archive_provider_history(&root, messages, pre_tokens, summary)
}

fn archive_provider_history(
    root: &Path,
    messages: &[ProviderMessage],
    pre_tokens: u64,
    summary: &str,
) -> std::io::Result<Option<ProviderHistoryArchiveMeta>> {
    if messages.is_empty() {
        return Ok(None);
    }

    let created_at = chrono::Utc::now().to_rfc3339();
    let archived_messages: Vec<ArchivedProviderMessage> =
        messages.iter().map(ArchivedProviderMessage::from).collect();
    let id = archive_id(&created_at, &archived_messages);
    let archive = ProviderHistoryArchive {
        schema_version: ARCHIVE_SCHEMA_VERSION,
        id: id.clone(),
        created_at,
        pre_tokens,
        summary: summary.to_owned(),
        messages: archived_messages,
    };
    let store = jfc_knowledge::block_on_knowledge(async {
        jfc_knowledge::KnowledgeStore::open_default()
            .await
            .map_err(std::io::Error::other)
    })?;
    let session_id = project_artifact_session_id(root);
    let json = serde_json::to_string(&archive).map_err(std::io::Error::other)?;
    jfc_knowledge::block_on_knowledge(async {
        store
            .upsert_session_artifact(&session_id, PROVIDER_HISTORY_ARCHIVE_KIND, &id, &json)
            .await
            .map_err(std::io::Error::other)
    })?;

    Ok(Some(ProviderHistoryArchiveMeta {
        id,
        path: PathBuf::from(session_id),
        message_count: archive.messages.len(),
    }))
}

pub(crate) fn render_provider_history_archive_by_id(id: &str) -> Option<String> {
    let root = std::env::current_dir().ok()?;
    let archive = load_archive(&root, id)?;
    Some(render_archive(&archive))
}

fn load_archive(root: &Path, id: &str) -> Option<ProviderHistoryArchive> {
    let id = safe_archive_id(id)?;
    let store = jfc_knowledge::block_on_knowledge(async {
        jfc_knowledge::KnowledgeStore::open_default().await.ok()
    })?;
    let row = jfc_knowledge::block_on_knowledge(async {
        store
            .get_session_artifact(
                &project_artifact_session_id(root),
                PROVIDER_HISTORY_ARCHIVE_KIND,
                id,
            )
            .await
            .ok()
            .flatten()
    })?;
    serde_json::from_str(&row.value_json).ok()
}

fn load_archives(root: &Path) -> Vec<ProviderHistoryArchive> {
    let Ok(store) = jfc_knowledge::block_on_knowledge(async {
        jfc_knowledge::KnowledgeStore::open_default().await
    }) else {
        return Vec::new();
    };
    let Ok(rows) = jfc_knowledge::block_on_knowledge(async {
        store
            .list_session_artifacts(
                &project_artifact_session_id(root),
                PROVIDER_HISTORY_ARCHIVE_KIND,
                500,
            )
            .await
    }) else {
        return Vec::new();
    };
    rows.into_iter()
        .filter_map(|row| serde_json::from_str::<ProviderHistoryArchive>(&row.value_json).ok())
        .collect()
}

fn archive_id(created_at: &str, messages: &[ArchivedProviderMessage]) -> String {
    let stamp = chrono::Utc::now().format("%Y%m%d-%H%M%S");
    let mut hasher = Sha256::new();
    hasher.update(created_at.as_bytes());
    for message in messages {
        if let Ok(bytes) = serde_json::to_vec(message) {
            hasher.update(bytes);
        }
    }
    let digest = hasher.finalize();
    let mut suffix = String::new();
    for byte in digest.iter().take(5) {
        let _ = write!(&mut suffix, "{byte:02x}");
    }
    format!("provider-history-{stamp}-{suffix}")
}

pub(super) fn safe_archive_id(id: &str) -> Option<&str> {
    let id = id.trim().strip_suffix(".json").unwrap_or(id.trim());
    if id.is_empty()
        || id.contains('/')
        || id.contains('\\')
        || id.contains("..")
        || !id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_'))
    {
        return None;
    }
    Some(id)
}

fn project_artifact_session_id(root: &Path) -> String {
    format!("project:{}", jfc_knowledge::project_key(root))
}
