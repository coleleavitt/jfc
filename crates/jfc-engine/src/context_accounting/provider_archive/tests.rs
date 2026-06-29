use super::{
    archive_provider_history_current_project, provider_history_archive_recall_block,
    render_provider_history_archive_by_id, search_provider_history_archives,
};
use jfc_provider::{ProviderContent, ProviderMessage, ProviderRole};
use std::collections::BTreeSet;
use std::time::{SystemTime, UNIX_EPOCH};

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

#[test]
#[serial_test::serial]
fn archived_provider_history_is_searchable_and_recallable_regression() {
    let _env = KnowledgeDbEnvGuard::new();
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    let marker = format!("jfc-provider-history-recall-marker-{nonce}");
    let messages = vec![ProviderMessage {
        role: ProviderRole::User,
        content: vec![ProviderContent::Text(format!(
            "Please remember {marker} when future turns ask about archived overflow."
        ))],
    }];

    let meta = archive_provider_history_current_project(&messages, 42, "test overflow summary")
        .expect("archive write should not fail")
        .expect("nonempty history should archive");

    let hits = search_provider_history_archives(&marker, 1);
    assert_eq!(
        hits.first().map(|hit| hit.id.as_str()),
        Some(meta.id.as_str())
    );

    let recall = provider_history_archive_recall_block(&marker, 1, &BTreeSet::new())
        .expect("matching archive should produce recall block");
    assert!(recall.block.contains(&meta.id));
    assert!(recall.block.contains(&marker));
    assert_eq!(recall.archive_ids, vec![meta.id.clone()]);
    assert!(recall.block.len() <= 6_000);

    let seen = BTreeSet::from([meta.id.clone()]);
    assert_eq!(
        provider_history_archive_recall_block(&marker, 1, &seen),
        None
    );

    let rendered =
        render_provider_history_archive_by_id(&meta.id).expect("archive id should render");
    assert!(rendered.contains(&marker));
}
