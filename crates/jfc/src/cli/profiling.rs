#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LinkscopeMode {
    Phase,
    Trace,
    StackTrace,
    DetailTrace,
    StackDetailTrace,
}

pub(super) fn init_linkscope_from_env() -> Option<linkscope::ReportGuard> {
    let Some(mode) = std::env::var("JFC_LINKSCOPE")
        .ok()
        .as_deref()
        .and_then(parse_linkscope_mode)
    else {
        return None;
    };

    enable_linkscope(mode);
    linkscope::record_rss("process.start");
    tracing::info!(
        target: "jfc::linkscope",
        mode = ?mode,
        "linkscope profiling enabled"
    );

    report_requested().then(linkscope::ReportGuard::new)
}

pub(crate) fn enable_linkscope_for_dashboard() {
    if linkscope::is_enabled() {
        return;
    }
    linkscope::enable();
    linkscope::record_rss("dashboard.start");
    tracing::info!(
        target: "jfc::linkscope",
        "linkscope phase profiling enabled for dashboard"
    );
}

fn enable_linkscope(mode: LinkscopeMode) {
    match mode {
        LinkscopeMode::Phase => linkscope::enable(),
        LinkscopeMode::Trace => linkscope::trace_enable(),
        LinkscopeMode::StackTrace => linkscope::trace_stack_enable(),
        LinkscopeMode::DetailTrace => linkscope::trace_detail_enable(),
        LinkscopeMode::StackDetailTrace => linkscope::trace_stack_detail_enable(),
    }
}

fn parse_linkscope_mode(value: &str) -> Option<LinkscopeMode> {
    match value.trim().to_ascii_lowercase().as_str() {
        "" | "0" | "false" | "off" | "no" => None,
        "1" | "true" | "on" | "yes" | "phase" | "phases" => Some(LinkscopeMode::Phase),
        "trace" => Some(LinkscopeMode::Trace),
        "stack" | "stack-trace" | "trace-stack" => Some(LinkscopeMode::StackTrace),
        "detail" | "trace-detail" | "detail-trace" => Some(LinkscopeMode::DetailTrace),
        "stack-detail" | "detail-stack" | "trace-stack-detail" | "stack-trace-detail" => {
            Some(LinkscopeMode::StackDetailTrace)
        }
        _ => Some(LinkscopeMode::Phase),
    }
}

fn report_requested() -> bool {
    std::env::var("JFC_LINKSCOPE_REPORT")
        .ok()
        .is_some_and(|value| matches_truthy(&value))
}

fn matches_truthy(value: &str) -> bool {
    matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "1" | "true" | "on" | "yes" | "report"
    )
}

#[cfg(test)]
mod tests {
    use super::{LinkscopeMode, parse_linkscope_mode};

    #[test]
    fn parse_linkscope_mode_maps_truthy_values_to_phase_normal() {
        assert_eq!(parse_linkscope_mode("1"), Some(LinkscopeMode::Phase));
        assert_eq!(parse_linkscope_mode("yes"), Some(LinkscopeMode::Phase));
        assert_eq!(parse_linkscope_mode("phase"), Some(LinkscopeMode::Phase));
    }

    #[test]
    fn parse_linkscope_mode_maps_trace_modes_normal() {
        assert_eq!(parse_linkscope_mode("trace"), Some(LinkscopeMode::Trace));
        assert_eq!(
            parse_linkscope_mode("stack"),
            Some(LinkscopeMode::StackTrace)
        );
        assert_eq!(
            parse_linkscope_mode("detail"),
            Some(LinkscopeMode::DetailTrace)
        );
        assert_eq!(
            parse_linkscope_mode("stack-detail"),
            Some(LinkscopeMode::StackDetailTrace)
        );
    }

    #[test]
    fn parse_linkscope_mode_disables_false_values_robust() {
        assert_eq!(parse_linkscope_mode("0"), None);
        assert_eq!(parse_linkscope_mode("false"), None);
        assert_eq!(parse_linkscope_mode("off"), None);
    }
}
