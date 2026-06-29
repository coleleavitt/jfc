use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

use super::safe_archive_id;

const SEEN_SCHEMA_VERSION: u32 = 1;
const PROVIDER_HISTORY_ARCHIVE_SEEN_KIND: &str = "provider_history_archive_seen";
const PROVIDER_HISTORY_ARCHIVE_SEEN_KEY: &str = "active";

#[derive(Debug, Clone, Serialize, Deserialize)]
struct StoredProviderHistoryArchiveSeen {
    schema_version: u32,
    ids: Vec<String>,
    updated_at_ms: i64,
}

pub(crate) fn persist_session_provider_history_archive_seen(
    session_id: &str,
    ids: &BTreeSet<String>,
) -> anyhow::Result<()> {
    let normalized = normalize_ids(ids);
    jfc_knowledge::block_on_knowledge(async move {
        let store = jfc_knowledge::KnowledgeStore::open_default().await?;
        let value = StoredProviderHistoryArchiveSeen {
            schema_version: SEEN_SCHEMA_VERSION,
            ids: normalized.into_iter().collect(),
            updated_at_ms: chrono::Utc::now().timestamp_millis(),
        };
        let json = serde_json::to_string(&value)?;
        store
            .upsert_session_artifact(
                session_id,
                PROVIDER_HISTORY_ARCHIVE_SEEN_KIND,
                PROVIDER_HISTORY_ARCHIVE_SEEN_KEY,
                &json,
            )
            .await?;
        Ok(())
    })
}

pub(crate) async fn load_session_provider_history_archive_seen(
    session_id: &str,
) -> anyhow::Result<BTreeSet<String>> {
    let store = jfc_knowledge::KnowledgeStore::open_default().await?;
    let Some(row) = store
        .get_session_artifact(
            session_id,
            PROVIDER_HISTORY_ARCHIVE_SEEN_KIND,
            PROVIDER_HISTORY_ARCHIVE_SEEN_KEY,
        )
        .await?
    else {
        return Ok(BTreeSet::new());
    };
    let stored: StoredProviderHistoryArchiveSeen = serde_json::from_str(&row.value_json)?;
    if stored.schema_version != SEEN_SCHEMA_VERSION {
        return Ok(BTreeSet::new());
    }
    Ok(normalize_ids(stored.ids.iter()))
}

fn normalize_ids<'a, I>(ids: I) -> BTreeSet<String>
where
    I: IntoIterator<Item = &'a String>,
{
    ids.into_iter()
        .filter_map(|id| safe_archive_id(id).map(str::to_owned))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    struct KnowledgeDbEnvGuard {
        prior: Option<std::ffi::OsString>,
        _dir: tempfile::TempDir,
    }

    impl KnowledgeDbEnvGuard {
        fn new() -> Self {
            let dir = tempfile::tempdir().expect("tempdir");
            let prior = std::env::var_os("JFC_KNOWLEDGE_DB");
            unsafe { std::env::set_var("JFC_KNOWLEDGE_DB", dir.path().join("knowledge.db")) };
            Self { prior, _dir: dir }
        }
    }

    impl Drop for KnowledgeDbEnvGuard {
        fn drop(&mut self) {
            unsafe {
                match &self.prior {
                    Some(prior) => std::env::set_var("JFC_KNOWLEDGE_DB", prior),
                    None => std::env::remove_var("JFC_KNOWLEDGE_DB"),
                }
            }
        }
    }

    #[tokio::test]
    #[serial_test::serial]
    async fn provider_history_archive_seen_round_trips_by_session_regression() {
        let _env = KnowledgeDbEnvGuard::new();
        let session_id = "ses_provider_history_seen_roundtrip";
        let ids = BTreeSet::from([
            "provider-history-20260626-aaa111".to_owned(),
            "provider-history-20260626-bbb222.json".to_owned(),
            "../bad".to_owned(),
        ]);

        persist_session_provider_history_archive_seen(session_id, &ids)
            .expect("seen archive ids should persist");

        let loaded = load_session_provider_history_archive_seen(session_id)
            .await
            .expect("seen archive ids should load");

        assert_eq!(
            loaded,
            BTreeSet::from([
                "provider-history-20260626-aaa111".to_owned(),
                "provider-history-20260626-bbb222".to_owned(),
            ])
        );
        assert!(
            load_session_provider_history_archive_seen("ses_other")
                .await
                .expect("empty session lookup should succeed")
                .is_empty()
        );
    }
}
