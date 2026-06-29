use std::path::Path;

use crate::{Config, feature_config::FeatureConfig};

pub(crate) struct ConfigCacheTrace {
    pub label: &'static str,
    pub generation: u64,
    pub generation_only: bool,
    pub mtime_known: bool,
}

pub(crate) struct ConfigLoadTrace {
    pub label: &'static str,
    pub depth: u8,
    pub bytes: usize,
}

pub(crate) fn record_path_shape(label: &'static str, path: &Path) {
    if !linkscope::trace_detail_enabled() {
        return;
    }
    linkscope::detail_event_fields(
        label,
        [linkscope::TraceField::bytes(
            "path_bytes",
            usize_to_u64_saturating(path.as_os_str().len()),
        )],
    );
}

pub(crate) fn record_config_load(input: ConfigLoadTrace) {
    linkscope::record_bytes(input.label, usize_to_u64_saturating(input.bytes));
    if !linkscope::trace_detail_enabled() {
        return;
    }
    linkscope::detail_event_fields(
        input.label,
        [
            linkscope::TraceField::count("depth", u64::from(input.depth)),
            linkscope::TraceField::bytes("toml_bytes", usize_to_u64_saturating(input.bytes)),
        ],
    );
}

pub(crate) fn record_config_shape(label: &'static str, cfg: &Config, depth: u8) {
    linkscope::record_items(label, usize_to_u64_saturating(cfg.agents.len()));
    if !linkscope::trace_detail_enabled() {
        return;
    }
    linkscope::detail_event_fields(
        label,
        [
            linkscope::TraceField::count("depth", u64::from(depth)),
            linkscope::TraceField::count("agents", usize_to_u64_saturating(cfg.agents.len())),
            linkscope::TraceField::count(
                "categories",
                usize_to_u64_saturating(cfg.categories.len()),
            ),
            linkscope::TraceField::count("mcp", usize_to_u64_saturating(cfg.mcp.len())),
            linkscope::TraceField::count(
                "disabled_agents",
                usize_to_u64_saturating(cfg.disabled_agents.len()),
            ),
            linkscope::TraceField::count("safe_mode", bool_to_u64(cfg.safe_mode)),
            linkscope::TraceField::count("has_extends", bool_to_u64(cfg.extends.is_some())),
        ],
    );
}

pub(crate) fn record_cache_probe(input: ConfigCacheTrace) {
    linkscope::record_items(input.label, 1);
    if !linkscope::trace_detail_enabled() {
        return;
    }
    linkscope::detail_event_fields(
        input.label,
        [
            linkscope::TraceField::count("generation", input.generation),
            linkscope::TraceField::count("generation_only", bool_to_u64(input.generation_only)),
            linkscope::TraceField::count("mtime_known", bool_to_u64(input.mtime_known)),
        ],
    );
}

pub(crate) fn record_feature_shape(label: &'static str, cfg: &FeatureConfig) {
    linkscope::record_items(label, usize_to_u64_saturating(cfg.permissions.rules.len()));
    if !linkscope::trace_detail_enabled() {
        return;
    }
    linkscope::detail_event_fields(
        label,
        [
            linkscope::TraceField::count("permissions", bool_to_u64(cfg.permissions.enabled)),
            linkscope::TraceField::count("hooks", bool_to_u64(cfg.hooks.enabled)),
            linkscope::TraceField::count("intent", bool_to_u64(cfg.intent.enabled)),
            linkscope::TraceField::count(
                "allowed_tools",
                usize_to_u64_saturating(cfg.permissions.allowed_tools.len()),
            ),
            linkscope::TraceField::count(
                "denied_tools",
                usize_to_u64_saturating(cfg.permissions.denied_tools.len()),
            ),
            linkscope::TraceField::count(
                "permission_rules",
                usize_to_u64_saturating(cfg.permissions.rules.len()),
            ),
            linkscope::TraceField::count(
                "comment_patterns",
                usize_to_u64_saturating(cfg.hooks.comment_check.patterns.len()),
            ),
            linkscope::TraceField::count(
                "max_concurrent",
                usize_to_u64_saturating(cfg.background.max_concurrent),
            ),
        ],
    );
}

pub(crate) fn record_status(label: &'static str, status: &'static str) {
    linkscope::record_items(label, 1);
    if !linkscope::trace_detail_enabled() {
        return;
    }
    linkscope::detail_event_fields(label, [linkscope::TraceField::text("status", status)]);
}

fn bool_to_u64(value: bool) -> u64 {
    u64::from(value)
}

fn usize_to_u64_saturating(value: usize) -> u64 {
    u64::try_from(value).unwrap_or(u64::MAX)
}
