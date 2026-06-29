use crate::runtime::StreamRequestOverrides;
use jfc_provider::StreamOptions;

fn normalize_thinking_display(value: &str) -> Option<&'static str> {
    match value.trim().to_ascii_lowercase().as_str() {
        "show" | "shown" | "visible" | "on" | "true" | "summary" | "summarize" | "summarized" => {
            Some("summarized")
        }
        "hide" | "hidden" | "off" | "false" | "omit" | "omitted" => Some("omitted"),
        _ => None,
    }
}

pub(super) fn requested_thinking_display(overrides: &StreamRequestOverrides) -> Option<String> {
    if let Some(value) = overrides.thinking_display.as_deref() {
        return match normalize_thinking_display(value) {
            Some(display) => Some(display.to_owned()),
            None => {
                tracing::warn!(
                    target: "jfc::stream",
                    value,
                    "ignoring unsupported thinking display mode"
                );
                None
            }
        };
    }
    std::env::var("JFC_THINKING_DISPLAY")
        .ok()
        .and_then(|value| normalize_thinking_display(&value).map(str::to_owned))
}

pub(super) fn enforce_thinking_budget_fits_max_tokens(opts: &mut StreamOptions) {
    let Some(budget) = opts.thinking_budget.as_mut() else {
        return;
    };
    if opts.max_tokens <= 1 {
        tracing::warn!(
            target: "jfc::stream",
            max_tokens = opts.max_tokens,
            thinking_budget = *budget,
            "disabling explicit thinking budget because max_tokens is too low"
        );
        opts.thinking_budget = None;
        return;
    }
    if *budget >= opts.max_tokens {
        let adjusted = opts.max_tokens - 1;
        tracing::warn!(
            target: "jfc::stream",
            max_tokens = opts.max_tokens,
            requested_thinking_budget = *budget,
            adjusted_thinking_budget = adjusted,
            "clamped thinking budget below max_tokens"
        );
        *budget = adjusted;
    }
}
