use jfc_plugin_host::{
    BUILTIN_CACHE_DIGEST_METRIC_ID, BUILTIN_CACHE_HIT_METRIC_ID,
    BUILTIN_RSI_PROMPT_SECTIONS_METRIC_ID, BUILTIN_RSI_TOOL_VISIBILITY_METRIC_ID,
    builtin_observability_plugin_host,
};
use jfc_plugin_sdk::MetricSurface;

#[test]
fn builtin_observability_plugin_registers_cache_and_rsi_metrics_normal() {
    let host = builtin_observability_plugin_host().expect("host activates");
    let metrics = host.metric_descriptors();
    let ids = metrics
        .iter()
        .map(|metric| metric.id.as_str())
        .collect::<Vec<_>>();

    assert!(ids.contains(&BUILTIN_CACHE_HIT_METRIC_ID));
    assert!(ids.contains(&BUILTIN_CACHE_DIGEST_METRIC_ID));
    assert!(ids.contains(&BUILTIN_RSI_PROMPT_SECTIONS_METRIC_ID));
    assert!(ids.contains(&BUILTIN_RSI_TOOL_VISIBILITY_METRIC_ID));
    assert!(
        metrics
            .iter()
            .any(|metric| metric.id == BUILTIN_CACHE_HIT_METRIC_ID
                && metric.surfaces.contains(&MetricSurface::StatusLine))
    );
    assert_eq!(host.diagnostics().counts.metrics, 4);
}
