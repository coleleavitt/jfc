use std::time::Instant;

use anyhow::Context;
use jfc_plugin_sdk::{UiMutationScope, UiPanelDescriptor, UiPanelRefreshKind};

use super::plugin_panel_refresh_policy::{
    panel_refresh_auto_interval, panel_refresh_debounce_remaining,
};
use super::plugin_panel_state::{UiPanelSnapshot, save_ui_panel_snapshots, ui_panel_snapshot_key};
use super::plugin_widget_bridge::execute_process_bridge_panel_refresh;
use super::state::App;

impl App {
    pub(crate) async fn refresh_ui_panel_snapshot(
        &mut self,
        panel: &UiPanelDescriptor,
    ) -> anyhow::Result<bool> {
        let Some(refresh) = panel.refresh.as_ref() else {
            return Ok(false);
        };
        if refresh.kind != UiPanelRefreshKind::ProcessBridge {
            return Ok(false);
        }
        let snapshot_key = ui_panel_snapshot_key(panel);
        let now = Instant::now();
        if let Some(remaining) = panel_refresh_debounce_remaining(
            panel,
            self.plugins.ui_panel_refresh_status.get(&snapshot_key),
            now,
        ) {
            self.plugins
                .ui_panel_refresh_status
                .entry(snapshot_key)
                .or_default()
                .record_skip(format!("debounced {}ms", remaining.as_millis()));
            return Ok(false);
        }
        self.plugins
            .ui_panel_refresh_status
            .entry(snapshot_key.clone())
            .or_default()
            .record_attempt(now);
        let state = self
            .plugins
            .ui_panel_snapshots
            .get(&snapshot_key)
            .and_then(|snapshot| snapshot.state.clone());
        let result =
            match execute_process_bridge_panel_refresh(panel, &refresh.handler, state).await {
                Ok(result) => result,
                Err(error) => {
                    self.plugins
                        .ui_panel_refresh_status
                        .entry(snapshot_key)
                        .or_default()
                        .record_error(error.to_string());
                    return Err(error);
                }
            };
        self.plugins
            .ui_panel_snapshots
            .insert(snapshot_key.clone(), UiPanelSnapshot::from(result));
        if let Err(error) = save_ui_panel_snapshots(
            std::path::Path::new(&self.engine.cwd),
            &self.plugins.ui_panel_snapshots,
        )
        .context("persist UI panel snapshots")
        {
            self.plugins
                .ui_panel_refresh_status
                .entry(snapshot_key)
                .or_default()
                .record_error(error.to_string());
            return Err(error);
        }
        self.plugins
            .ui_panel_refresh_status
            .entry(snapshot_key)
            .or_default()
            .record_success(Instant::now());
        Ok(true)
    }

    pub(crate) async fn refresh_focused_info_sidebar_panel_snapshot(
        &mut self,
    ) -> anyhow::Result<bool> {
        let Some(focused) = self.info_sidebar.focused_panel.as_ref() else {
            return Ok(false);
        };
        let Some(panel) = self
            .plugins
            .ui_panel_descriptors
            .iter()
            .find(|panel| {
                panel.scope == UiMutationScope::InfoSidebar
                    && panel.plugin_id.as_str() == focused.plugin_id.as_str()
                    && panel.id.as_str() == focused.panel_id.as_str()
            })
            .cloned()
        else {
            self.info_sidebar.clear_panel_focus();
            return Ok(false);
        };
        self.refresh_ui_panel_snapshot(&panel).await
    }

    pub(crate) async fn refresh_due_ui_panel_snapshots(&mut self) -> bool {
        let due_panels = self
            .plugins
            .ui_panel_descriptors
            .iter()
            .filter(|panel| self.ui_panel_auto_refresh_due(panel))
            .cloned()
            .collect::<Vec<_>>();
        let mut changed = false;
        for panel in due_panels {
            match self.refresh_ui_panel_snapshot(&panel).await {
                Ok(refreshed) => changed |= refreshed,
                Err(error) => {
                    tracing::warn!(
                        target: "jfc::plugins",
                        plugin = panel.plugin_id.as_str(),
                        panel = panel.id.as_str(),
                        error = %error,
                        "failed to auto-refresh plugin panel snapshot"
                    );
                    changed = true;
                }
            }
        }
        changed
    }

    fn ui_panel_auto_refresh_due(&self, panel: &UiPanelDescriptor) -> bool {
        let Some(interval) = panel_refresh_auto_interval(panel) else {
            return false;
        };
        let key = ui_panel_snapshot_key(panel);
        let Some(status) = self.plugins.ui_panel_refresh_status.get(&key) else {
            return true;
        };
        status
            .last_attempt_at
            .map(|attempted_at| attempted_at.elapsed() >= interval)
            .unwrap_or(true)
    }
}
