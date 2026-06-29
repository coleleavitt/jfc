use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::Instant;

use jfc_plugin_sdk::{BridgeUiPanelRefreshResult, UiMutationScope, UiPanelDescriptor};

pub(super) fn append_ui_panel_descriptors(
    panels: &mut Vec<UiPanelDescriptor>,
    extra: Vec<UiPanelDescriptor>,
) {
    let mut seen = panels
        .iter()
        .map(ui_panel_key)
        .collect::<HashSet<(String, UiMutationScope, String)>>();
    for panel in extra {
        if seen.insert(ui_panel_key(&panel)) {
            panels.push(panel);
        }
    }
}

fn ui_panel_key(panel: &UiPanelDescriptor) -> (String, UiMutationScope, String) {
    (
        panel.plugin_id.as_str().to_owned(),
        panel.scope,
        panel.id.clone(),
    )
}

#[derive(Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub(crate) struct UiPanelSnapshot {
    pub(crate) body: Option<String>,
    pub(crate) state: Option<serde_json::Value>,
}

impl From<BridgeUiPanelRefreshResult> for UiPanelSnapshot {
    fn from(result: BridgeUiPanelRefreshResult) -> Self {
        Self {
            body: result.body,
            state: result.state,
        }
    }
}

pub(crate) type UiPanelSnapshots = BTreeMap<String, UiPanelSnapshot>;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct UiPanelRefreshStatus {
    pub(crate) last_attempt_at: Option<Instant>,
    pub(crate) last_success_at: Option<Instant>,
    pub(crate) last_error: Option<String>,
    pub(crate) last_skip_reason: Option<String>,
}

impl UiPanelRefreshStatus {
    pub(crate) fn record_attempt(&mut self, at: Instant) {
        self.last_attempt_at = Some(at);
        self.last_skip_reason = None;
    }

    pub(crate) fn record_success(&mut self, at: Instant) {
        self.last_success_at = Some(at);
        self.last_error = None;
        self.last_skip_reason = None;
    }

    pub(crate) fn record_error(&mut self, error: String) {
        self.last_error = Some(error);
        self.last_skip_reason = None;
    }

    pub(crate) fn record_skip(&mut self, reason: String) {
        self.last_skip_reason = Some(reason);
    }
}

pub(crate) type UiPanelRefreshStatuses = BTreeMap<String, UiPanelRefreshStatus>;

pub(crate) fn ui_panel_snapshot_key(panel: &UiPanelDescriptor) -> String {
    format!(
        "{}\0{}\0{}",
        panel.plugin_id.as_str(),
        scope_key(panel.scope),
        panel.id
    )
}

pub(crate) fn load_ui_panel_snapshots(project_root: &Path) -> UiPanelSnapshots {
    load_ui_panel_snapshots_from_path(&panel_snapshot_path(project_root))
}

pub(crate) fn save_ui_panel_snapshots(
    project_root: &Path,
    snapshots: &UiPanelSnapshots,
) -> std::io::Result<()> {
    save_ui_panel_snapshots_to_path(&panel_snapshot_path(project_root), snapshots)
}

fn load_ui_panel_snapshots_from_path(path: &Path) -> UiPanelSnapshots {
    let Ok(raw) = std::fs::read_to_string(path) else {
        return UiPanelSnapshots::default();
    };
    serde_json::from_str(&raw).unwrap_or_default()
}

fn save_ui_panel_snapshots_to_path(
    path: &Path,
    snapshots: &UiPanelSnapshots,
) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let json = serde_json::to_vec_pretty(snapshots)?;
    std::fs::write(path, json)
}

fn panel_snapshot_path(project_root: &Path) -> PathBuf {
    let project_root = project_root
        .canonicalize()
        .unwrap_or_else(|_| project_root.to_path_buf());
    let base = dirs::data_dir().unwrap_or_else(|| project_root.join(".jfc"));
    base.join("jfc")
        .join("panel-snapshots")
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

const fn scope_key(scope: UiMutationScope) -> &'static str {
    match scope {
        UiMutationScope::InfoSidebar => "info_sidebar",
        UiMutationScope::TaskPanel => "task_panel",
        UiMutationScope::SessionSidebar => "session_sidebar",
    }
}

#[cfg(test)]
mod tests {
    use jfc_plugin_sdk::PluginId;

    use super::*;

    #[test]
    fn panel_snapshots_round_trip_to_disk_normal() {
        let tmp = tempfile::TempDir::new().expect("tempdir");
        let path = tmp.path().join("snapshots.json");
        let panel = UiPanelDescriptor::new(
            PluginId::new("demo"),
            UiMutationScope::InfoSidebar,
            "reviews",
            "Reviews",
        );
        let mut snapshots = UiPanelSnapshots::default();
        snapshots.insert(
            ui_panel_snapshot_key(&panel),
            UiPanelSnapshot {
                body: Some("fresh".to_owned()),
                state: Some(serde_json::json!({ "cursor": "abc" })),
            },
        );

        save_ui_panel_snapshots_to_path(&path, &snapshots).expect("save snapshots");
        let loaded = load_ui_panel_snapshots_from_path(&path);

        assert_eq!(loaded, snapshots);
    }
}
