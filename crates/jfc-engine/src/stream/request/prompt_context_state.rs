use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use jfc_plugin_sdk::{
    BridgePromptContextRefreshResult, RuntimeExtensionDescriptor, RuntimeExtensionTarget,
};

const PROMPT_CONTEXT_TARGET_KEY: &str = "prompt_context";

#[derive(Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub(super) struct PromptContextSnapshot {
    pub(super) body: Option<String>,
    pub(super) state: Option<serde_json::Value>,
    pub(super) refreshed_at_ms: Option<u64>,
}

impl PromptContextSnapshot {
    pub(super) fn from_refresh_result(
        result: BridgePromptContextRefreshResult,
        refreshed_at_ms: u64,
        max_chars: usize,
    ) -> Self {
        Self {
            body: bounded_prompt_context_body(result.body, max_chars),
            state: result.state,
            refreshed_at_ms: Some(refreshed_at_ms),
        }
    }
}

pub(super) type PromptContextSnapshots = BTreeMap<String, PromptContextSnapshot>;

pub(super) struct PromptContextSnapshotStore {
    path: PathBuf,
    snapshots: PromptContextSnapshots,
    changed: bool,
}

impl PromptContextSnapshotStore {
    pub(super) fn open(project_root: &Path) -> Self {
        let path = prompt_context_snapshot_path(project_root);
        let snapshots = load_prompt_context_snapshots_from_path(&path);
        Self {
            path,
            snapshots,
            changed: false,
        }
    }

    pub(super) fn get(&self, key: &str) -> Option<&PromptContextSnapshot> {
        self.snapshots.get(key)
    }

    pub(super) fn insert(&mut self, key: String, snapshot: PromptContextSnapshot) {
        if self.snapshots.get(&key) == Some(&snapshot) {
            return;
        }
        self.snapshots.insert(key, snapshot);
        self.changed = true;
    }

    pub(super) fn save_if_changed(&self) -> std::io::Result<()> {
        if !self.changed {
            return Ok(());
        }
        save_prompt_context_snapshots_to_path(&self.path, &self.snapshots)
    }
}

pub(super) fn prompt_context_snapshot_key(extension: &RuntimeExtensionDescriptor) -> String {
    format!(
        "{}\0{}\0{}",
        extension.plugin_id.as_str(),
        target_key(extension.target),
        extension.id
    )
}

pub(super) fn snapshot_is_fresh(
    extension: &RuntimeExtensionDescriptor,
    snapshot: Option<&PromptContextSnapshot>,
    now_ms: u64,
) -> bool {
    let Some(snapshot) = snapshot else {
        return false;
    };
    let Some(refreshed_at_ms) = snapshot.refreshed_at_ms else {
        return false;
    };
    let Some(interval_ms) = refresh_interval_ms(extension) else {
        return false;
    };
    now_ms.saturating_sub(refreshed_at_ms) < interval_ms
}

pub(super) fn snapshot_body(snapshot: &PromptContextSnapshot) -> Option<&str> {
    snapshot
        .body
        .as_deref()
        .filter(|body| !body.trim().is_empty())
}

pub(super) fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| u64::try_from(duration.as_millis()).unwrap_or(u64::MAX))
        .unwrap_or(0)
}

fn refresh_interval_ms(extension: &RuntimeExtensionDescriptor) -> Option<u64> {
    extension
        .refresh
        .as_ref()
        .and_then(|refresh| refresh.auto_refresh_ms.or(refresh.min_interval_ms))
}

fn bounded_prompt_context_body(body: Option<String>, max_chars: usize) -> Option<String> {
    let body = body?;
    let bounded = body.chars().take(max_chars).collect::<String>();
    if bounded.trim().is_empty() {
        return None;
    }
    Some(bounded)
}

fn load_prompt_context_snapshots_from_path(path: &Path) -> PromptContextSnapshots {
    let Ok(raw) = std::fs::read_to_string(path) else {
        return PromptContextSnapshots::default();
    };
    serde_json::from_str(&raw).unwrap_or_default()
}

fn save_prompt_context_snapshots_to_path(
    path: &Path,
    snapshots: &PromptContextSnapshots,
) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let json = serde_json::to_vec_pretty(snapshots)?;
    std::fs::write(path, json)
}

fn prompt_context_snapshot_path(project_root: &Path) -> PathBuf {
    let project_root = project_root
        .canonicalize()
        .unwrap_or_else(|_| project_root.to_path_buf());
    let base = dirs::data_dir().unwrap_or_else(|| project_root.join(".jfc"));
    base.join("jfc")
        .join("prompt-context-snapshots")
        .join(format!("{}.json", stable_project_key(&project_root)))
}

fn stable_project_key(project_root: &Path) -> String {
    let mut hash = 0xcbf29ce484222325_u64;
    for byte in project_root.to_string_lossy().as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{hash:016x}")
}

const fn target_key(target: RuntimeExtensionTarget) -> &'static str {
    match target {
        RuntimeExtensionTarget::MessageRenderer => "message_renderer",
        RuntimeExtensionTarget::PromptContext => PROMPT_CONTEXT_TARGET_KEY,
    }
}

#[cfg(test)]
mod tests {
    use jfc_plugin_sdk::{
        PluginId, RuntimeExtensionExecutorDescriptor, RuntimeExtensionRefreshDescriptor,
    };

    use super::*;

    #[test]
    fn prompt_context_snapshots_round_trip_to_disk_normal() {
        let tmp = tempfile::TempDir::new().expect("tempdir");
        let path = tmp.path().join("snapshots.json");
        let extension = prompt_context_extension().with_refresh(
            RuntimeExtensionRefreshDescriptor::process_bridge().with_min_interval_ms(60_000),
        );
        let mut snapshots = PromptContextSnapshots::default();
        snapshots.insert(
            prompt_context_snapshot_key(&extension),
            PromptContextSnapshot {
                body: Some("fresh context".to_owned()),
                state: Some(serde_json::json!({ "cursor": "abc" })),
                refreshed_at_ms: Some(123),
            },
        );

        save_prompt_context_snapshots_to_path(&path, &snapshots).expect("save snapshots");
        let loaded = load_prompt_context_snapshots_from_path(&path);

        assert_eq!(loaded, snapshots);
    }

    #[test]
    fn snapshot_is_fresh_uses_refresh_interval_normal() {
        let extension = prompt_context_extension().with_refresh(
            RuntimeExtensionRefreshDescriptor::process_bridge().with_min_interval_ms(60_000),
        );
        let snapshot = PromptContextSnapshot {
            body: Some("fresh context".to_owned()),
            state: None,
            refreshed_at_ms: Some(1_000),
        };

        assert!(snapshot_is_fresh(&extension, Some(&snapshot), 60_999));
        assert!(!snapshot_is_fresh(&extension, Some(&snapshot), 61_000));
    }

    fn prompt_context_extension() -> RuntimeExtensionDescriptor {
        RuntimeExtensionDescriptor::new(
            PluginId::new("demo"),
            RuntimeExtensionTarget::PromptContext,
            "context.demo",
            "Demo Context",
        )
        .with_executor(RuntimeExtensionExecutorDescriptor::process_bridge(
            "context.sh",
        ))
    }
}
