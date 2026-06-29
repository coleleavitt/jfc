use std::time::{Duration, Instant};

use jfc_plugin_sdk::UiPanelDescriptor;

use super::plugin_panel_state::UiPanelRefreshStatus;

const MIN_REFRESH_INTERVAL: Duration = Duration::from_secs(1);

pub(crate) fn panel_refresh_debounce_remaining(
    panel: &UiPanelDescriptor,
    status: Option<&UiPanelRefreshStatus>,
    now: Instant,
) -> Option<Duration> {
    let interval = panel_refresh_min_interval(panel)?;
    let attempted_at = status.and_then(|status| status.last_attempt_at)?;
    interval.checked_sub(now.duration_since(attempted_at))
}

pub(crate) fn panel_refresh_auto_interval(panel: &UiPanelDescriptor) -> Option<Duration> {
    let refresh = panel.refresh.as_ref()?;
    let requested = refresh.auto_refresh_ms.map(bounded_refresh_interval)?;
    Some(match panel_refresh_min_interval(panel) {
        Some(min_interval) => requested.max(min_interval),
        None => requested,
    })
}

fn panel_refresh_min_interval(panel: &UiPanelDescriptor) -> Option<Duration> {
    panel
        .refresh
        .as_ref()
        .and_then(|refresh| refresh.min_interval_ms)
        .map(bounded_refresh_interval)
}

fn bounded_refresh_interval(ms: u64) -> Duration {
    Duration::from_millis(ms).max(MIN_REFRESH_INTERVAL)
}
