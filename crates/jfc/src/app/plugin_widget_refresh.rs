use std::time::Instant;

use anyhow::Context;
use jfc_plugin_host::PluginRuntime;
use jfc_plugin_sdk::{UiWidgetDescriptor, UiWidgetRefreshKind};

use super::plugin_widget_refresh_policy::{
    refresh_debounce_remaining, widget_refresh_auto_interval,
};
use super::plugin_widget_state::{
    UiWidgetSnapshot, save_ui_widget_snapshots, ui_widget_snapshot_key,
};
use super::state::App;

impl App {
    pub(crate) async fn refresh_ui_widget_snapshot(
        &mut self,
        widget: &UiWidgetDescriptor,
    ) -> anyhow::Result<bool> {
        let Some(refresh) = widget.refresh.as_ref() else {
            return Ok(false);
        };
        if refresh.kind != UiWidgetRefreshKind::ProcessBridge {
            return Ok(false);
        }
        let snapshot_key = ui_widget_snapshot_key(widget);
        let now = Instant::now();
        if let Some(remaining) = refresh_debounce_remaining(
            widget,
            self.plugins.ui_widget_refresh_status.get(&snapshot_key),
            now,
        ) {
            self.plugins
                .ui_widget_refresh_status
                .entry(snapshot_key)
                .or_default()
                .record_skip(format!("debounced {}ms", remaining.as_millis()));
            return Ok(false);
        }
        self.plugins
            .ui_widget_refresh_status
            .entry(snapshot_key.clone())
            .or_default()
            .record_attempt(now);
        let state = self
            .plugins
            .ui_widget_snapshots
            .get(&snapshot_key)
            .and_then(|snapshot| snapshot.state.clone());
        let runtime = PluginRuntime::from_ui_widget_descriptors(std::iter::once(widget.clone()))?;
        let result = match runtime
            .refresh_ui_widget_snapshot(&widget.plugin_id, widget.scope, &widget.id, state)
            .await
        {
            Ok(result) => result,
            Err(error) => {
                self.plugins
                    .ui_widget_refresh_status
                    .entry(snapshot_key)
                    .or_default()
                    .record_error(error.to_string());
                return Err(error.into());
            }
        };
        self.plugins
            .ui_widget_snapshots
            .insert(snapshot_key.clone(), UiWidgetSnapshot::from(result));
        if let Err(error) = save_ui_widget_snapshots(
            std::path::Path::new(&self.engine.cwd),
            &self.plugins.ui_widget_snapshots,
        )
        .context("persist UI widget snapshots")
        {
            self.plugins
                .ui_widget_refresh_status
                .entry(snapshot_key)
                .or_default()
                .record_error(error.to_string());
            return Err(error);
        }
        self.plugins
            .ui_widget_refresh_status
            .entry(snapshot_key)
            .or_default()
            .record_success(Instant::now());
        Ok(true)
    }

    pub(crate) async fn refresh_focused_info_sidebar_widget_snapshot(
        &mut self,
    ) -> anyhow::Result<bool> {
        let Some(focused) = self.info_sidebar.focused_widget.as_ref() else {
            return Ok(false);
        };
        let Some(widget) = self
            .plugins
            .ui_widget_descriptors
            .iter()
            .find(|widget| {
                widget.scope == jfc_plugin_sdk::UiMutationScope::InfoSidebar
                    && widget.plugin_id.as_str() == focused.plugin_id.as_str()
                    && widget.id.as_str() == focused.widget_id.as_str()
            })
            .cloned()
        else {
            self.info_sidebar.clear_widget_focus();
            return Ok(false);
        };
        self.refresh_ui_widget_snapshot(&widget).await
    }

    pub(crate) async fn refresh_due_ui_widget_snapshots(&mut self) -> bool {
        let due_widgets = self
            .plugins
            .ui_widget_descriptors
            .iter()
            .filter(|widget| self.ui_widget_auto_refresh_due(widget))
            .cloned()
            .collect::<Vec<_>>();
        let mut changed = false;
        for widget in due_widgets {
            match self.refresh_ui_widget_snapshot(&widget).await {
                Ok(refreshed) => changed |= refreshed,
                Err(error) => {
                    tracing::warn!(
                        target: "jfc::plugins",
                        plugin = widget.plugin_id.as_str(),
                        widget = widget.id.as_str(),
                        error = %error,
                        "failed to auto-refresh plugin widget snapshot"
                    );
                    changed = true;
                }
            }
        }
        changed
    }

    fn ui_widget_auto_refresh_due(&self, widget: &UiWidgetDescriptor) -> bool {
        let Some(interval) = widget_refresh_auto_interval(widget) else {
            return false;
        };
        let key = ui_widget_snapshot_key(widget);
        let Some(status) = self.plugins.ui_widget_refresh_status.get(&key) else {
            return true;
        };
        status
            .last_attempt_at
            .map(|attempted_at| attempted_at.elapsed() >= interval)
            .unwrap_or(true)
    }
}
