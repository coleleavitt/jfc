use std::time::Duration;

use super::{SpawnConfig, TransportKind};

pub(super) struct CallToolStart<'a> {
    pub(super) server: &'a str,
    pub(super) tool_name: &'a str,
    pub(super) timeout: Duration,
    pub(super) args: &'a serde_json::Value,
}

pub(super) struct ListResult<'a> {
    pub(super) server: &'a str,
    pub(super) method: &'static str,
    pub(super) status: &'static str,
    pub(super) count: usize,
}

pub(super) fn spawn_start(cfg: &SpawnConfig) {
    if !linkscope::trace_detail_enabled() {
        return;
    }
    linkscope::detail_event_fields(
        "mcp.spawn.start",
        [
            linkscope::TraceField::text("server", cfg.server_name.clone()),
            linkscope::TraceField::text("kind", cfg.kind.label()),
            linkscope::TraceField::bytes("command_bytes", len_to_u64(cfg.command.len())),
            linkscope::TraceField::count("args", len_to_u64(cfg.args.len())),
            linkscope::TraceField::count("env", len_to_u64(cfg.env.len())),
            linkscope::TraceField::count("headers", len_to_u64(cfg.headers.len())),
            linkscope::TraceField::count("has_cwd", u64::from(cfg.cwd.is_some())),
            linkscope::TraceField::count("has_url", u64::from(cfg.url.is_some())),
        ],
    );
}

pub(super) fn spawn_stdio_command(cfg: &SpawnConfig) {
    if !linkscope::trace_detail_enabled() {
        return;
    }
    linkscope::detail_event_fields(
        "mcp.spawn.stdio.command",
        [
            linkscope::TraceField::text("server", cfg.server_name.clone()),
            linkscope::TraceField::bytes("command_bytes", len_to_u64(cfg.command.len())),
            linkscope::TraceField::count("args", len_to_u64(cfg.args.len())),
            linkscope::TraceField::count("env", len_to_u64(cfg.env.len())),
            linkscope::TraceField::count("has_cwd", u64::from(cfg.cwd.is_some())),
        ],
    );
}

pub(super) fn spawn_http_config(cfg: &SpawnConfig, has_auth_header: bool) {
    if !linkscope::trace_detail_enabled() {
        return;
    }
    linkscope::detail_event_fields(
        "mcp.spawn.http.config",
        [
            linkscope::TraceField::text("server", cfg.server_name.clone()),
            linkscope::TraceField::bytes(
                "url_bytes",
                len_to_u64(cfg.url.as_deref().unwrap_or_default().len()),
            ),
            linkscope::TraceField::count("headers", len_to_u64(cfg.headers.len())),
            linkscope::TraceField::count("has_auth_header", u64::from(has_auth_header)),
        ],
    );
}

pub(super) fn spawn_result(server: &str, kind: TransportKind, status: &'static str) {
    if !linkscope::trace_detail_enabled() {
        return;
    }
    linkscope::detail_event_fields(
        "mcp.spawn.result",
        [
            linkscope::TraceField::text("server", server.to_owned()),
            linkscope::TraceField::text("kind", kind.label()),
            linkscope::TraceField::text("status", status),
        ],
    );
}

pub(super) fn list_start(server: &str, method: &'static str) {
    if !linkscope::trace_detail_enabled() {
        return;
    }
    linkscope::detail_event_fields(
        "mcp.list.start",
        [
            linkscope::TraceField::text("server", server.to_owned()),
            linkscope::TraceField::text("method", method),
        ],
    );
}

pub(super) fn list_result(input: ListResult<'_>) {
    if !linkscope::trace_detail_enabled() {
        return;
    }
    linkscope::detail_event_fields(
        "mcp.list.result",
        [
            linkscope::TraceField::text("server", input.server.to_owned()),
            linkscope::TraceField::text("method", input.method),
            linkscope::TraceField::text("status", input.status),
            linkscope::TraceField::count("count", len_to_u64(input.count)),
        ],
    );
}

pub(super) fn call_tool_start(input: CallToolStart<'_>) {
    if !linkscope::trace_detail_enabled() {
        return;
    }
    linkscope::detail_event_fields(
        "mcp.call_tool.start",
        [
            linkscope::TraceField::text("server", input.server.to_owned()),
            linkscope::TraceField::text("tool", input.tool_name.to_owned()),
            linkscope::TraceField::count("timeout_ms", duration_ms(input.timeout)),
            linkscope::TraceField::text("args_kind", value_kind(input.args)),
        ],
    );
}

pub(super) fn call_tool_result(server: &str, tool_name: &str, status: &'static str) {
    if !linkscope::trace_detail_enabled() {
        return;
    }
    linkscope::detail_event_fields(
        "mcp.call_tool.result",
        [
            linkscope::TraceField::text("server", server.to_owned()),
            linkscope::TraceField::text("tool", tool_name.to_owned()),
            linkscope::TraceField::text("status", status),
        ],
    );
}

pub(super) fn read_resource_start(server: &str, uri: &str) {
    if !linkscope::trace_detail_enabled() {
        return;
    }
    linkscope::detail_event_fields(
        "mcp.read_resource.start",
        [
            linkscope::TraceField::text("server", server.to_owned()),
            linkscope::TraceField::bytes("uri_bytes", len_to_u64(uri.len())),
        ],
    );
}

pub(super) fn read_resource_result(server: &str, status: &'static str) {
    if !linkscope::trace_detail_enabled() {
        return;
    }
    linkscope::detail_event_fields(
        "mcp.read_resource.result",
        [
            linkscope::TraceField::text("server", server.to_owned()),
            linkscope::TraceField::text("status", status),
        ],
    );
}

fn value_kind(value: &serde_json::Value) -> &'static str {
    match value {
        serde_json::Value::Null => "null",
        serde_json::Value::Bool(_) => "bool",
        serde_json::Value::Number(_) => "number",
        serde_json::Value::String(_) => "string",
        serde_json::Value::Array(_) => "array",
        serde_json::Value::Object(_) => "object",
    }
}

fn duration_ms(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}

fn len_to_u64(value: usize) -> u64 {
    u64::try_from(value).unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;

    #[test]
    fn spawn_start_records_shape_without_payload_normal() {
        linkscope::trace_detail_enable();
        let cfg = SpawnConfig {
            server_name: "fs".to_owned(),
            kind: TransportKind::Stdio,
            command: "server-bin".to_owned(),
            args: vec!["--mode".to_owned()],
            env: HashMap::from([("TOKEN".to_owned(), "secret".to_owned())]),
            headers: HashMap::new(),
            url: None,
            cwd: None,
        };

        spawn_start(&cfg);

        let snapshot = linkscope::snapshot();
        let trace = snapshot
            .traces
            .iter()
            .find(|trace| trace.label == "mcp.spawn.start")
            .expect("spawn trace should exist");
        assert!(
            trace
                .fields
                .iter()
                .any(|field| field.name == "command_bytes")
        );
        assert!(!trace.fields.iter().any(|field| field.value == "secret"));
    }
}
