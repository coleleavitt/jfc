use std::time::{Duration, Instant};

use jfc_plugin_sdk::UiWidgetDescriptor;

use super::plugin_widget_state::UiWidgetRefreshStatus;

const MIN_REFRESH_INTERVAL: Duration = Duration::from_secs(1);

pub(crate) fn refresh_debounce_remaining(
    widget: &UiWidgetDescriptor,
    status: Option<&UiWidgetRefreshStatus>,
    now: Instant,
) -> Option<Duration> {
    let interval = widget_refresh_min_interval(widget)?;
    let attempted_at = status.and_then(|status| status.last_attempt_at)?;
    interval.checked_sub(now.duration_since(attempted_at))
}

pub(crate) fn widget_refresh_auto_interval(widget: &UiWidgetDescriptor) -> Option<Duration> {
    let refresh = widget.refresh.as_ref()?;
    let requested = refresh.auto_refresh_ms.map(bounded_refresh_interval)?;
    Some(match widget_refresh_min_interval(widget) {
        Some(min_interval) => requested.max(min_interval),
        None => requested,
    })
}

fn widget_refresh_min_interval(widget: &UiWidgetDescriptor) -> Option<Duration> {
    widget
        .refresh
        .as_ref()
        .and_then(|refresh| refresh.min_interval_ms)
        .map(bounded_refresh_interval)
}

fn bounded_refresh_interval(ms: u64) -> Duration {
    Duration::from_millis(ms).max(MIN_REFRESH_INTERVAL)
}
