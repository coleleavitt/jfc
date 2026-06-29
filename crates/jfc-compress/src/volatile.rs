//! Volatile-content detector — ported from headroom-proxy's
//! `cache_stabilization::volatile_detector` (Apache-2.0).
//!
//! Scans an outbound LLM request body for substrings that bust
//! prompt-cache hits when they appear inside the cached prefix (system
//! prompt, tool definitions, historical messages):
//!
//!   1. **ISO-8601 timestamps** (`YYYY-MM-DDTHH:MM:SS...`) — rendered
//!      freshly per request, so a cache hit on a prefix containing one
//!      is accidental.
//!   2. **UUID v4** — the version-4 nibble distinguishes per-request
//!      generated UUIDs from fixed identifiers.
//!   3. **ID-named JSON fields** (`request_id`, `trace_id`, `session_id`,
//!      `correlation_id`) — catches non-UUID per-request IDs the
//!      substring scan would miss.
//!
//! # Non-mutation invariant
//!
//! This module **never** mutates the request body. It takes
//! `&serde_json::Value` and walks read-only.
//!
//! # jfc port notes
//!
//! The proxy crate's `CompressibleEndpoint`-based `from_endpoint`
//! constructor is replaced with [`ApiKind::from_path`], which classifies
//! by the request path string jfc's providers already have. Everything
//! else is byte-for-byte the headroom detector.

use serde_json::Value;

/// Maximum findings reported per request. See module docs for rationale.
pub const MAX_FINDINGS_PER_REQUEST: usize = 10;

/// Maximum bytes of `sample` we log per finding.
pub const SAMPLE_TRUNCATE_BYTES: usize = 80;

/// JSON field names that are conventionally per-request unique IDs.
const ID_FIELD_NEEDLES: &[&str] = &["request_id", "trace_id", "session_id", "correlation_id"];

/// What kind of volatile content we found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VolatileKind {
    /// ISO-8601 timestamp shape: positions 4=`-`, 7=`-`, 10=`T`, 13=`:`, 16=`:`.
    Timestamp,
    /// UUID v4 shape: 36 chars, hex, hyphens at 8/13/18/23, version
    /// nibble `4` at position 14.
    Uuid,
    /// JSON key whose name contains one of the conventionally
    /// per-request ID needles.
    IdField,
}

impl VolatileKind {
    /// Stable string representation for structured logging.
    pub fn as_str(self) -> &'static str {
        match self {
            VolatileKind::Timestamp => "iso8601_timestamp",
            VolatileKind::Uuid => "uuid_v4",
            VolatileKind::IdField => "id_field",
        }
    }
}

/// One volatile-content finding. `location` is a JSON-pointer-style path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VolatileFinding {
    pub kind: VolatileKind,
    pub location: String,
    pub sample: String,
}

/// Which provider's body shape to walk.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApiKind {
    /// Anthropic `/v1/messages` shape: top-level `system`,
    /// `messages[].content`, `tools[].description` + `tools[].input_schema`.
    Anthropic,
    /// OpenAI Chat Completions / Responses shape: `messages[].content`,
    /// `tools[].function.description` + `tools[].function.parameters`.
    OpenAi,
}

impl ApiKind {
    /// Classify by request path. Anything mentioning `messages` under an
    /// Anthropic-style path is Anthropic; OpenAI chat/responses paths are
    /// OpenAI. Defaults to Anthropic (jfc's primary provider shape).
    pub fn from_path(path: &str) -> Self {
        let p = path.to_ascii_lowercase();
        if p.contains("/chat/completions") || p.contains("/responses") {
            ApiKind::OpenAi
        } else {
            ApiKind::Anthropic
        }
    }
}

/// Public detection entry point. Walks the parsed body for the given API
/// shape and returns up to [`MAX_FINDINGS_PER_REQUEST`] findings.
pub fn detect_volatile_content(body: &Value, kind: ApiKind) -> Vec<VolatileFinding> {
    let _linkscope_detect = linkscope::phase("compress.volatile.detect");
    let mut findings: Vec<VolatileFinding> = Vec::new();
    match kind {
        ApiKind::Anthropic => walk_anthropic(body, &mut findings),
        ApiKind::OpenAi => walk_openai(body, &mut findings),
    }
    linkscope::record_items(
        "compress.volatile.findings",
        usize_to_u64_saturating(findings.len()),
    );
    findings
}

/// Emit one `tracing::warn!` per finding with a stable structured shape.
pub fn emit_volatile_warnings(findings: &[VolatileFinding], request_id: &str) {
    let _linkscope_emit = linkscope::phase("compress.volatile.emit_warnings");
    linkscope::record_items(
        "compress.volatile.warnings",
        usize_to_u64_saturating(findings.len()),
    );
    for finding in findings {
        tracing::warn!(
            target: "jfc::cache",
            event = "volatile_content_detected",
            request_id = %request_id,
            kind = finding.kind.as_str(),
            location = %finding.location,
            sample = %finding.sample,
            "volatile content in cached prefix will bust prompt-cache hits; \
             move per-request IDs/timestamps to message metadata or post-prefix \
             fields"
        );
    }
}

fn usize_to_u64_saturating(value: usize) -> u64 {
    u64::try_from(value).unwrap_or(u64::MAX)
}

// ─── Anthropic walker ──────────────────────────────────────────────────

fn walk_anthropic(body: &Value, out: &mut Vec<VolatileFinding>) {
    if !out.is_empty() && out.len() >= MAX_FINDINGS_PER_REQUEST {
        return;
    }
    if let Some(system) = body.get("system") {
        scan_value_for_strings(system, "system", out);
    }
    if let Some(Value::Array(messages)) = body.get("messages") {
        for (i, msg) in messages.iter().enumerate() {
            if out.len() >= MAX_FINDINGS_PER_REQUEST {
                return;
            }
            if let Some(content) = msg.get("content") {
                let loc = format!("messages[{i}].content");
                scan_value_for_strings(content, &loc, out);
            }
        }
    }
    if let Some(Value::Array(tools)) = body.get("tools") {
        for (i, tool) in tools.iter().enumerate() {
            if out.len() >= MAX_FINDINGS_PER_REQUEST {
                return;
            }
            if let Some(Value::String(desc)) = tool.get("description") {
                let loc = format!("tools[{i}].description");
                scan_string(desc, &loc, out);
            }
            if let Some(schema) = tool.get("input_schema") {
                let loc = format!("tools[{i}].input_schema");
                scan_value_recursive(schema, &loc, out);
            }
        }
    }
}

// ─── OpenAI walker ─────────────────────────────────────────────────────

fn walk_openai(body: &Value, out: &mut Vec<VolatileFinding>) {
    scan_message_contents(body, out);
    let Some(Value::Array(tools)) = body.get("tools") else {
        return;
    };
    for (i, tool) in tools.iter().enumerate() {
        if out.len() >= MAX_FINDINGS_PER_REQUEST {
            return;
        }
        let Some(function) = tool.get("function") else {
            continue;
        };
        if let Some(Value::String(desc)) = function.get("description") {
            scan_string(desc, &format!("tools[{i}].function.description"), out);
        }
        if let Some(params) = function.get("parameters") {
            scan_value_recursive(params, &format!("tools[{i}].function.parameters"), out);
        }
    }
}

/// Walk `messages[].content` (string or content-block array) — the shape
/// shared by both the Anthropic and OpenAI request bodies.
fn scan_message_contents(body: &Value, out: &mut Vec<VolatileFinding>) {
    let Some(Value::Array(messages)) = body.get("messages") else {
        return;
    };
    for (i, msg) in messages.iter().enumerate() {
        if out.len() >= MAX_FINDINGS_PER_REQUEST {
            return;
        }
        if let Some(content) = msg.get("content") {
            scan_value_for_strings(content, &format!("messages[{i}].content"), out);
        }
    }
}

// ─── Generic walkers ───────────────────────────────────────────────────

fn scan_value_for_strings(v: &Value, location: &str, out: &mut Vec<VolatileFinding>) {
    if out.len() >= MAX_FINDINGS_PER_REQUEST {
        return;
    }
    match v {
        Value::String(s) => scan_string(s, location, out),
        Value::Array(items) => scan_array(items, location, out),
        Value::Object(_) => scan_value_recursive(v, location, out),
        // Numbers / bools / null carry no scannable text and no keys.
        Value::Number(_) | Value::Bool(_) | Value::Null => {}
    }
}

/// Scan each element of an array, threading the global finding cap.
fn scan_array(items: &[Value], location: &str, out: &mut Vec<VolatileFinding>) {
    for (i, item) in items.iter().enumerate() {
        if out.len() >= MAX_FINDINGS_PER_REQUEST {
            return;
        }
        scan_value_recursive(item, &format!("{location}[{i}]"), out);
    }
}

fn scan_value_recursive(v: &Value, location: &str, out: &mut Vec<VolatileFinding>) {
    if out.len() >= MAX_FINDINGS_PER_REQUEST {
        return;
    }
    match v {
        Value::String(s) => scan_string(s, location, out),
        Value::Array(items) => scan_array(items, location, out),
        Value::Object(map) => scan_object(map, location, out),
        // Numbers / bools / null carry no scannable text and no keys.
        Value::Number(_) | Value::Bool(_) | Value::Null => {}
    }
}

/// Scan a JSON object: flag any non-empty ID-named key, then recurse into
/// every value. The only walker that inspects keys.
fn scan_object(
    map: &serde_json::Map<String, Value>,
    location: &str,
    out: &mut Vec<VolatileFinding>,
) {
    for (k, sub) in map.iter() {
        if out.len() >= MAX_FINDINGS_PER_REQUEST {
            return;
        }
        if is_id_named_key(k) && !is_json_schema_properties_map(location) && !is_value_empty(sub) {
            out.push(VolatileFinding {
                kind: VolatileKind::IdField,
                location: format!("{location}.{k}"),
                sample: truncate_sample(&value_to_sample(sub)),
            });
            if out.len() >= MAX_FINDINGS_PER_REQUEST {
                return;
            }
        }
        scan_value_recursive(sub, &format!("{location}.{k}"), out);
    }
}

fn scan_string(s: &str, location: &str, out: &mut Vec<VolatileFinding>) {
    let bytes = s.as_bytes();
    let len = bytes.len();
    let mut i = 0usize;
    while i < len {
        if out.len() >= MAX_FINDINGS_PER_REQUEST {
            return;
        }
        if i + 19 <= len && looks_like_iso8601(&bytes[i..i + 19]) {
            let end = (i + 19).min(len);
            out.push(VolatileFinding {
                kind: VolatileKind::Timestamp,
                location: location.to_string(),
                sample: truncate_sample(&s[i..end]),
            });
            i += 19;
            continue;
        }
        if i + 36 <= len && looks_like_uuid_v4(&bytes[i..i + 36]) {
            out.push(VolatileFinding {
                kind: VolatileKind::Uuid,
                location: location.to_string(),
                sample: truncate_sample(&s[i..i + 36]),
            });
            i += 36;
            continue;
        }
        i += 1;
    }
}

fn looks_like_iso8601(window: &[u8]) -> bool {
    if window.len() < 19 {
        return false;
    }
    let digits_in =
        |range: std::ops::Range<usize>| -> bool { window[range].iter().all(u8::is_ascii_digit) };
    digits_in(0..4)
        && window[4] == b'-'
        && digits_in(5..7)
        && window[7] == b'-'
        && digits_in(8..10)
        && (window[10] == b'T' || window[10] == b't' || window[10] == b' ')
        && digits_in(11..13)
        && window[13] == b':'
        && digits_in(14..16)
        && window[16] == b':'
        && digits_in(17..19)
}

fn looks_like_uuid_v4(window: &[u8]) -> bool {
    if window.len() < 36 {
        return false;
    }
    if window[8] != b'-' || window[13] != b'-' || window[18] != b'-' || window[23] != b'-' {
        return false;
    }
    if window[14] != b'4' {
        return false;
    }
    match window[19] {
        b'8' | b'9' | b'a' | b'b' | b'A' | b'B' => {}
        _ => return false,
    }
    for (i, &c) in window.iter().enumerate().take(36) {
        if i == 8 || i == 13 || i == 18 || i == 23 {
            continue;
        }
        if !c.is_ascii_hexdigit() {
            return false;
        }
    }
    true
}

fn is_id_named_key(key: &str) -> bool {
    let lowered = key.to_ascii_lowercase();
    ID_FIELD_NEEDLES
        .iter()
        .any(|needle| lowered.contains(needle))
}

fn is_json_schema_properties_map(location: &str) -> bool {
    location
        .rsplit('.')
        .next()
        .is_some_and(|segment| segment == "properties")
}

fn is_value_empty(v: &Value) -> bool {
    match v {
        Value::Null => true,
        Value::String(s) => s.is_empty(),
        Value::Array(a) => a.is_empty(),
        Value::Object(m) => m.is_empty(),
        _ => false,
    }
}

fn value_to_sample(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Null => "null".to_string(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => n.to_string(),
        _ => v.to_string(),
    }
}

fn truncate_sample(s: &str) -> String {
    if s.len() <= SAMPLE_TRUNCATE_BYTES {
        return s.to_string();
    }
    let mut cut = SAMPLE_TRUNCATE_BYTES;
    while cut > 0 && !s.is_char_boundary(cut) {
        cut -= 1;
    }
    let mut out = String::with_capacity(cut + 1);
    out.push_str(&s[..cut]);
    out.push('…');
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn detects_iso8601_timestamp_in_system_prompt() {
        let body = json!({
            "system": "Today is 2026-05-04T14:30:00Z. Be concise.",
            "messages": [],
        });
        let findings = detect_volatile_content(&body, ApiKind::Anthropic);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].kind, VolatileKind::Timestamp);
        assert_eq!(findings[0].location, "system");
        assert!(findings[0].sample.starts_with("2026-05-04T14:30:00"));
    }

    #[test]
    fn detects_uuid_v4_in_user_message() {
        let body = json!({
            "messages": [
                {"role": "user", "content": "trace=550e8400-e29b-41d4-a716-446655440000"},
            ],
        });
        let findings = detect_volatile_content(&body, ApiKind::Anthropic);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].kind, VolatileKind::Uuid);
        assert_eq!(findings[0].location, "messages[0].content");
        assert_eq!(findings[0].sample, "550e8400-e29b-41d4-a716-446655440000");
    }

    #[test]
    fn detects_request_id_field_in_nested_object() {
        let body = json!({
            "messages": [
                {
                    "role": "user",
                    "content": [
                        {
                            "type": "tool_result",
                            "request_id": "req-2026-abc-12345"
                        }
                    ]
                }
            ],
        });
        let findings = detect_volatile_content(&body, ApiKind::Anthropic);
        let id_field = findings
            .iter()
            .find(|f| f.kind == VolatileKind::IdField)
            .expect("expected an IdField finding");
        assert!(id_field.location.ends_with(".request_id"));
        assert!(id_field.sample.contains("req-2026-abc-12345"));
    }

    #[test]
    fn json_schema_property_names_are_not_runtime_id_fields_regression() {
        let body = json!({
            "tools": [{
                "name": "lookup",
                "description": "Look up a user.",
                "input_schema": {
                    "type": "object",
                    "properties": {
                        "request_id": {"type": "string"},
                        "trace_id": {"type": "string"},
                        "nested": {
                            "type": "object",
                            "properties": {
                                "correlation_id": {"type": "string"}
                            }
                        }
                    }
                }
            }],
            "messages": [],
        });
        let findings = detect_volatile_content(&body, ApiKind::Anthropic);
        assert!(
            findings
                .iter()
                .all(|finding| finding.kind != VolatileKind::IdField),
            "schema property names should not be flagged: {findings:?}"
        );
    }

    #[test]
    fn stable_content_yields_zero_findings() {
        let body = json!({
            "system": "You are a helpful assistant. Be concise.",
            "messages": [
                {"role": "user", "content": "Summarize the document below."},
                {"role": "assistant", "content": "Sure — please paste it."},
            ],
            "tools": [{
                "name": "search",
                "description": "Search the corpus.",
                "input_schema": {
                    "type": "object",
                    "properties": {"query": {"type": "string"}}
                }
            }],
        });
        let findings = detect_volatile_content(&body, ApiKind::Anthropic);
        assert!(findings.is_empty(), "got {findings:?}");
    }

    #[test]
    fn caps_findings_at_ten() {
        let mut messages = Vec::new();
        for i in 0..30 {
            messages.push(json!({
                "role": "user",
                "content": format!("turn {i}: 550e8400-e29b-41d4-a716-446655440000"),
            }));
        }
        let body = json!({"messages": messages});
        let findings = detect_volatile_content(&body, ApiKind::Anthropic);
        assert_eq!(findings.len(), MAX_FINDINGS_PER_REQUEST);
    }

    #[test]
    fn does_not_mutate_input() {
        let body = json!({
            "system": "Today is 2026-05-04T14:30:00Z.",
            "messages": [{
                "role": "user",
                "content": "trace=550e8400-e29b-41d4-a716-446655440000",
            }],
            "tools": [{
                "name": "lookup",
                "description": "Look up a user.",
                "input_schema": {
                    "type": "object",
                    "properties": {"request_id": "req-abc"}
                }
            }],
        });
        let before = serde_json::to_vec(&body).expect("serialize before");
        let _findings = detect_volatile_content(&body, ApiKind::Anthropic);
        let after = serde_json::to_vec(&body).expect("serialize after");
        assert_eq!(before, after, "detector must NOT mutate input body bytes");
    }

    #[test]
    fn apikind_shapes_scan_correct_paths() {
        let body = json!({
            "tools": [{
                "name": "lookup",
                "description": "scheduled at 2026-05-04T10:00:00Z",
                "input_schema": {"type": "object"}
            }],
        });
        let anthropic_findings = detect_volatile_content(&body, ApiKind::Anthropic);
        assert_eq!(anthropic_findings.len(), 1);
        assert_eq!(anthropic_findings[0].location, "tools[0].description");

        let openai_findings = detect_volatile_content(&body, ApiKind::OpenAi);
        assert!(openai_findings.is_empty(), "got {openai_findings:?}");

        let openai_body = json!({
            "tools": [{
                "type": "function",
                "function": {
                    "name": "lookup",
                    "description": "scheduled at 2026-05-04T10:00:00Z",
                    "parameters": {"type": "object"}
                }
            }],
        });
        let openai_findings = detect_volatile_content(&openai_body, ApiKind::OpenAi);
        assert_eq!(openai_findings.len(), 1);
        assert_eq!(openai_findings[0].location, "tools[0].function.description");
    }

    #[test]
    fn id_field_with_empty_value_does_not_fire() {
        let body = json!({
            "tools": [{
                "input_schema": {
                    "properties": {"request_id": ""}
                }
            }],
        });
        let findings = detect_volatile_content(&body, ApiKind::Anthropic);
        assert!(findings.iter().all(|f| f.kind != VolatileKind::IdField));
    }

    #[test]
    fn iso8601_with_space_separator_recognized() {
        let body = json!({"system": "started at 2026-05-04 14:30:00"});
        let findings = detect_volatile_content(&body, ApiKind::Anthropic);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].kind, VolatileKind::Timestamp);
    }

    #[test]
    fn random_hex_without_v4_nibble_is_not_a_uuid() {
        let body = json!({
            "messages": [{
                "role": "user",
                "content": "id=550e8400-e29b-01d4-a716-446655440000",
            }],
        });
        let findings = detect_volatile_content(&body, ApiKind::Anthropic);
        assert!(findings.iter().all(|f| f.kind != VolatileKind::Uuid));
    }

    #[test]
    fn truncate_sample_respects_utf8_boundaries() {
        let mut s = "a".repeat(SAMPLE_TRUNCATE_BYTES);
        s.push('é');
        let out = truncate_sample(&s);
        let _ = out.as_bytes();
        assert!(out.ends_with('…'));
    }

    #[test]
    fn from_path_classifies_provider_shape() {
        assert_eq!(ApiKind::from_path("/v1/messages"), ApiKind::Anthropic);
        assert_eq!(ApiKind::from_path("/v1/chat/completions"), ApiKind::OpenAi);
        assert_eq!(ApiKind::from_path("/v1/responses"), ApiKind::OpenAi);
    }
}
