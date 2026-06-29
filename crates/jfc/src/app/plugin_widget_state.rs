use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Instant;

use jfc_plugin_sdk::BridgeUiWidgetRefreshResult;
use jfc_plugin_sdk::{UiMutationScope, UiWidgetDescriptor};
use serde::{Deserialize, Serialize};

use super::plugin_status::PluginUiState;

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub(crate) struct UiWidgetSnapshot {
    pub(crate) body: Option<String>,
    pub(crate) state: Option<serde_json::Value>,
}

impl From<BridgeUiWidgetRefreshResult> for UiWidgetSnapshot {
    fn from(result: BridgeUiWidgetRefreshResult) -> Self {
        Self {
            body: result.body,
            state: result.state,
        }
    }
}

pub(crate) type UiWidgetSnapshots = BTreeMap<String, UiWidgetSnapshot>;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct UiWidgetRefreshStatus {
    pub(crate) last_attempt_at: Option<Instant>,
    pub(crate) last_success_at: Option<Instant>,
    pub(crate) last_error: Option<String>,
    pub(crate) last_skip_reason: Option<String>,
}

impl UiWidgetRefreshStatus {
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

pub(crate) type UiWidgetRefreshStatuses = BTreeMap<String, UiWidgetRefreshStatus>;

impl PluginUiState {
    pub(crate) fn preserve_ui_widget_snapshots_from(&mut self, previous: &PluginUiState) {
        self.ui_panel_snapshots = previous.ui_panel_snapshots.clone();
        self.ui_panel_refresh_status = previous.ui_panel_refresh_status.clone();
        self.ui_widget_snapshots = previous.ui_widget_snapshots.clone();
        self.ui_widget_refresh_status = previous.ui_widget_refresh_status.clone();
    }
}

pub(crate) fn ui_widget_snapshot_key(widget: &UiWidgetDescriptor) -> String {
    format!(
        "{}\0{}\0{}",
        widget.plugin_id.as_str(),
        scope_key(widget.scope),
        widget.id
    )
}

pub(crate) fn load_ui_widget_snapshots(project_root: &Path) -> UiWidgetSnapshots {
    load_ui_widget_snapshots_from_path(&widget_snapshot_path(project_root))
}

pub(crate) fn save_ui_widget_snapshots(
    project_root: &Path,
    snapshots: &UiWidgetSnapshots,
) -> std::io::Result<()> {
    save_ui_widget_snapshots_to_path(&widget_snapshot_path(project_root), snapshots)
}

fn load_ui_widget_snapshots_from_path(path: &Path) -> UiWidgetSnapshots {
    let Ok(raw) = std::fs::read_to_string(path) else {
        return UiWidgetSnapshots::default();
    };
    serde_json::from_str(&raw).unwrap_or_default()
}

fn save_ui_widget_snapshots_to_path(
    path: &Path,
    snapshots: &UiWidgetSnapshots,
) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let json = serde_json::to_vec_pretty(snapshots)?;
    std::fs::write(path, json)
}

fn widget_snapshot_path(project_root: &Path) -> PathBuf {
    let project_root = project_root
        .canonicalize()
        .unwrap_or_else(|_| project_root.to_path_buf());
    let base = dirs::data_dir().unwrap_or_else(|| project_root.join(".jfc"));
    base.join("jfc")
        .join("widget-snapshots")
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
    use jfc_plugin_sdk::{PluginId, UiWidgetKind};

    use super::*;

    #[test]
    fn widget_snapshots_round_trip_to_disk_normal() {
        let tmp = tempfile::TempDir::new().expect("tempdir");
        let path = tmp.path().join("snapshots.json");
        let widget = UiWidgetDescriptor::new(
            PluginId::new("demo"),
            UiMutationScope::InfoSidebar,
            "reviews",
            "Reviews",
            UiWidgetKind::Text,
        );
        let mut snapshots = UiWidgetSnapshots::default();
        snapshots.insert(
            ui_widget_snapshot_key(&widget),
            UiWidgetSnapshot {
                body: Some("fresh".to_owned()),
                state: Some(serde_json::json!({ "cursor": "abc" })),
            },
        );

        save_ui_widget_snapshots_to_path(&path, &snapshots).expect("save snapshots");
        let loaded = load_ui_widget_snapshots_from_path(&path);

        assert_eq!(loaded, snapshots);
    }
}
