//! Antigravity request/response transform for the Code Assist API.
//!
//! Two concerns live here:
//!
//! 1. **Request envelope** ([`build_gemini_request`]): wraps the
//!    `Vec<ProviderMessage>` + `StreamOptions` in the Code Assist payload
//!    Google's `v1internal:streamGenerateContent` expects:
//!
//!    ```jsonc
//!    {
//!      "project":  "...",
//!      "model":    "gemini-3-pro",
//!      "userAgent":"antigravity",
//!      "requestId":"<uuid>",
//!      "request":  { /* GenerateContentRequest: contents/tools/etc. */ }
//!    }
//!    ```
//!
//! 2. **Response parser** ([`parse_sse_chunk`] + [`into_event_stream`]):
//!    consumes Gemini SSE `data: {...}` frames carrying `candidates[].content`
//!    + `usageMetadata` and converts them into jfc's [`StreamEvent`] enum.
//!
//! Both pieces are pure functions (modulo HTTP) so they're unit-testable
//! without a live Google account. The Claude-via-Antigravity request shape
//! (`gemini-claude-*` models) is NOT handled here — those models would need
//! the Anthropic Messages-format envelope from `transform/claude.ts`; this
//! module raises a clear error in `build_gemini_request` if asked for one,
//! and the provider falls back to the auth-only behaviour for them.

use std::sync::LazyLock;

use futures::StreamExt;
use jfc_provider::{
    EventStream, ProviderContent, ProviderMessage, ProviderRole, StopReason, StreamEvent,
    StreamOptions, ToolDef,
};
use serde::Deserialize;
use serde_json::{Value, json};
use uuid::Uuid;

// ─── Per-process session ID (mirrors TS plugin's random session token) ───────

static SESSION_ID: LazyLock<String> = LazyLock::new(|| {
    // Mirrors the TS plugin's per-process random session token.
    // Use UUID-based randomness since rand version compatibility is tricky.
    let id = Uuid::new_v4().as_u128() % 9_000_000_000_000_000 + 1_000_000_000_000_000;
    format!("-{id}")
});

// ─── Antigravity mandatory system instruction ────────────────────────────────

const ANTIGRAVITY_BASE_SYSTEM_INSTRUCTION: &str = "You are Antigravity, a powerful agentic AI coding assistant designed by the \
     Google Deepmind team working on Advanced Agentic Coding. You are pair \
     programming with a USER to solve their coding task. The task may require \
     creating a new codebase, modifying or debugging an existing codebase, or \
     simply answering a question. **Absolute paths only** **Proactiveness**";

// ─── Tool schema system instruction for Gemini 3 ─────────────────────────────

const GEMINI_TOOL_SCHEMA_SYSTEM_INSTRUCTION: &str = r#"<CRITICAL_TOOL_USAGE_INSTRUCTIONS>
You are operating in a CUSTOM ENVIRONMENT where tool definitions COMPLETELY DIFFER from your training data.
VIOLATION OF THESE RULES WILL CAUSE IMMEDIATE SYSTEM FAILURE.

## ABSOLUTE RULES - NO EXCEPTIONS

1. **SCHEMA IS LAW**: The JSON schema in each tool definition is the ONLY source of truth.
   - Your pre-trained knowledge about tools like 'read_file', 'apply_diff', 'write_to_file', 'bash', etc. is INVALID here.
   - Every tool has been REDEFINED with different parameters than what you learned during training.

2. **PARAMETER NAMES ARE EXACT**: Use ONLY the parameter names from the schema.
   - The schema's 'required' array tells you which parameters are mandatory

3. **ARRAY PARAMETERS**: When a parameter has "type": "array", check the 'items' field:
   - If items.type is "object", you MUST provide an array of objects with the EXACT properties listed
   - If items.type is "string", you MUST provide an array of strings
   - NEVER provide a single object when an array is expected

4. **BEFORE EVERY TOOL CALL**:
   a. Read the tool's schema completely
   b. Identify ALL required parameters
   c. Verify your parameter names match EXACTLY (case-sensitive)
   d. For arrays, verify you're providing the correct item structure
   e. Do NOT add parameters that don't exist in the schema
</CRITICAL_TOOL_USAGE_INSTRUCTIONS>"#;

/// Translate a model id like `gemini-3-pro` / `gemini-claude-sonnet-4-5` into
/// the Code Assist short name. Mirrors `resolveModelName` in the TS plugin's
/// `request-helpers.ts` for the names we actually expose; unknown ids pass
/// through unchanged so the upstream returns a clean error.
pub fn resolve_model_name(model: &str) -> &str {
    match model {
        // Common short forms → canonical names for Code Assist API.
        "gemini-pro" => "gemini-3-pro",
        "gemini-flash" => "gemini-3-flash",
        "gemini-3.5" => "gemini-3.5-flash",
        "gemini-3.1-pro" => "gemini-3.1-pro-preview",
        "gemini-3.1-flash" => "gemini-3.1-flash-lite",
        "gemini-3-pro" => "gemini-3-pro-preview",
        "gemini-3-flash" => "gemini-3-flash-preview",
        // gemini-claude-* and explicit full names pass through verbatim.
        other => other,
    }
}

/// Is this a Claude-via-Gemini model id?
pub fn is_claude_model(model: &str) -> bool {
    model.contains("claude")
}

/// Is this a Gemini 3 model? (uses thinkingLevel instead of thinkingBudget)
pub(crate) fn is_gemini_3(model: &str) -> bool {
    model.contains("gemini-3") || model.contains("gemini_3")
}

/// Does this model need the Antigravity system instruction injected?
fn needs_antigravity_system_instruction(model: &str) -> bool {
    is_claude_model(model) || model.contains("gemini-3") || model.contains("antigravity")
}

/// Default thinking budget for Claude `-thinking` models when the caller
/// hasn't supplied one. Mirrors the upstream's 16384 fallback in
/// `transform/claude.ts`.
const CLAUDE_DEFAULT_THINKING_BUDGET: u32 = 16_384;

/// Claude requires `max_output_tokens > thinking.budget_tokens`. When the
/// caller's `max_tokens` is below this safe ceiling the upstream bumps it
/// to 64k; we do the same so users don't have to know about the constraint.
const CLAUDE_SAFE_MAX_OUTPUT_TOKENS: u32 = 64_000;

/// Build the Code Assist `streamGenerateContent` request body. Dispatches by
/// model id between the Gemini-native and Claude-via-Antigravity paths —
/// both end up in the same `{project, model, request:{…}}` envelope, but the
/// `generationConfig` + `toolConfig` shape differs.
///
/// This is the single entry point the provider calls; callers shouldn't pick
/// between the two builders themselves.
pub fn build_request(
    project_id: &str,
    messages: &[ProviderMessage],
    options: &StreamOptions,
) -> anyhow::Result<Value> {
    let model = resolve_model_name(options.model.as_str()).to_owned();
    if is_claude_model(&model) {
        build_claude_request(project_id, &model, messages, options)
    } else {
        build_gemini_request(project_id, &model, messages, options)
    }
}

/// Build the Code Assist `streamGenerateContent` request body for a
/// Gemini-native model. Exposed for the tests; production callers should go
/// through [`build_request`] which auto-dispatches by model id.
pub fn build_gemini_request(
    project_id: &str,
    model: &str,
    messages: &[ProviderMessage],
    options: &StreamOptions,
) -> anyhow::Result<Value> {
    let mut request = build_core_request(model, messages, options);

    // Gemini also benefits from VALIDATED mode for tool calling
    request["toolConfig"] = json!({
        "functionCallingConfig": { "mode": "VALIDATED" },
    });

    let mut generation_config = base_generation_config(options);
    if let Some(budget) = options.thinking_budget {
        if is_gemini_3(model) {
            // Gemini 3 models use thinkingLevel ('low'|'medium'|'high')
            let level = budget_to_thinking_level(budget);
            generation_config.insert("thinkingConfig".into(), json!({ "thinkingLevel": level }));
        } else {
            // Gemini 2.5 models use thinkingBudget (number)
            generation_config.insert("thinkingConfig".into(), json!({ "thinkingBudget": budget }));
        }
    }
    if !generation_config.is_empty() {
        request["generationConfig"] = Value::Object(generation_config);
    }
    Ok(wrap_envelope(project_id, model, request))
}

/// Build the Code Assist `streamGenerateContent` request body for a Claude-
/// via-Antigravity model. Mirrors `transform/claude.ts`:
///
/// * adds `toolConfig.functionCallingConfig.mode = "VALIDATED"` so Claude's
///   validator rejects malformed tool calls server-side,
/// * for `*-thinking` models, emits `thinkingConfig` with snake_case
///   `thinking_budget` + `include_thoughts: true`, and bumps
///   `maxOutputTokens` to a safe value if the caller's max is at or below
///   the thinking budget (Claude requires `max_output_tokens > budget`),
/// * for the non-thinking `gemini-claude-sonnet-4-5` model id, explicitly
///   omits `thinkingConfig` (the upstream deletes it as a defensive step).
pub fn build_claude_request(
    project_id: &str,
    model: &str,
    messages: &[ProviderMessage],
    options: &StreamOptions,
) -> anyhow::Result<Value> {
    let mut request = build_core_request(model, messages, options);

    // Claude requires the VALIDATED tool-calling mode so the upstream rejects
    // malformed parameters server-side rather than letting them through.
    request["toolConfig"] = json!({
        "functionCallingConfig": { "mode": "VALIDATED" },
    });

    let mut generation_config = base_generation_config(options);
    let is_thinking_model = model.contains("-thinking");
    if is_thinking_model {
        let budget = options
            .thinking_budget
            .unwrap_or(CLAUDE_DEFAULT_THINKING_BUDGET);
        generation_config.insert(
            "thinkingConfig".into(),
            json!({
                "include_thoughts": true,
                "thinking_budget": budget,
            }),
        );
        // Bump max_output_tokens to a safe value if the caller's limit is
        // at or below the thinking budget — Claude rejects requests where
        // max_output_tokens <= thinking.budget_tokens.
        let caller_max = options.max_tokens;
        if caller_max <= budget {
            generation_config.insert(
                "maxOutputTokens".into(),
                json!(CLAUDE_SAFE_MAX_OUTPUT_TOKENS),
            );
        }
    }
    if !generation_config.is_empty() {
        request["generationConfig"] = Value::Object(generation_config);
    }

    Ok(wrap_envelope(project_id, model, request))
}

/// Common `{contents, systemInstruction, tools}` core shared by both
/// builders. Each variant adds its own `generationConfig` + extras on top.
///
/// Handles:
/// - Antigravity system instruction injection (for Claude and Gemini 3)
/// - Tool schema system instruction (for Gemini 3 with tools)
/// - Tool name sanitization (Gemini requires `^[a-zA-Z_][a-zA-Z0-9_-]*$`)
/// - STRICT PARAMETERS description augmentation (for Gemini-native models)
fn build_core_request(model: &str, messages: &[ProviderMessage], options: &StreamOptions) -> Value {
    let contents = messages
        .iter()
        .filter_map(|m| message_to_content(m, messages))
        .collect::<Vec<_>>();
    let mut request = json!({ "contents": contents });

    // Build the systemInstruction parts array
    let mut sys_parts: Vec<Value> = Vec::new();

    // 1. Antigravity system instruction (prepended first for Claude + Gemini 3)
    if needs_antigravity_system_instruction(model) {
        sys_parts.push(json!({ "text": ANTIGRAVITY_BASE_SYSTEM_INSTRUCTION }));
    }

    // 2. Tool schema system instruction (for Gemini 3 with tools)
    if is_gemini_3(model) && !options.tools.is_empty() {
        sys_parts.push(json!({ "text": GEMINI_TOOL_SCHEMA_SYSTEM_INSTRUCTION }));
    }

    // 3. User-provided system prompt
    if let Some(sys) = options.system.as_deref().filter(|s| !s.is_empty()) {
        sys_parts.push(json!({ "text": sys }));
    }

    if !sys_parts.is_empty() {
        request["systemInstruction"] = json!({ "role": "user", "parts": sys_parts });
    }

    // Build tools with sanitization + STRICT PARAMETERS augmentation
    if !options.tools.is_empty() {
        let is_gemini_native = !is_claude_model(model);
        let decls: Vec<Value> = options
            .tools
            .iter()
            .map(|tool| tool_to_decl_augmented(tool, is_gemini_native))
            .collect();
        request["tools"] = json!([ { "functionDeclarations": decls } ]);
    }
    request
}

/// `generationConfig` fields common to both transforms (everything except
/// the per-model `thinkingConfig` shape).
fn base_generation_config(options: &StreamOptions) -> serde_json::Map<String, Value> {
    let mut cfg = serde_json::Map::new();
    cfg.insert("maxOutputTokens".into(), json!(options.max_tokens));
    if let Some(temp) = options.temperature {
        cfg.insert("temperature".into(), json!(temp));
    }
    if let Some(top_p) = options.top_p {
        cfg.insert("topP".into(), json!(top_p));
    }
    cfg
}

/// Wrap a core request body in the Code Assist envelope every Antigravity
/// streaming call uses (`{project, model, userAgent, requestId, sessionId, request}`).
fn wrap_envelope(project_id: &str, model: &str, mut request: Value) -> Value {
    // Inject sessionId into the request payload (inside the envelope)
    request["sessionId"] = json!(*SESSION_ID);
    json!({
        "project": project_id,
        "model": model,
        "userAgent": "antigravity",
        "requestType": "agent",
        "requestId": Uuid::new_v4().to_string(),
        "request": request,
    })
}

/// Synthetic fallback when no real thought signature is captured.
///
/// Per Google's gemini-cli (`packages/core/src/utils/historyHardening.ts:10`,
/// `core/geminiChat.ts:860`), this token is used only as a safety net for the
/// FIRST `functionCall` in each model turn when the original signature is
/// unavailable (cold-start, history truncation past the active loop). The
/// happy path always echoes the real signature captured from the SSE stream.
const SYNTHETIC_THOUGHT_SIGNATURE: &str = "skip_thought_signature_validator";

/// Walk the message history to map a transient `tool_use_id` (e.g.
/// `toolu_abc123`) back to the original tool name (e.g. `read_file`).
///
/// Gemini matches a `functionResponse` to its `functionCall` by NAME, not by
/// id — so a historical `ToolResult` must serialize with the same name the
/// model emitted on the `functionCall`, otherwise the next turn 400s. The
/// stored `ToolUse.name` is exactly that name (it round-trips verbatim from
/// the SSE stream), so we reuse it as-is rather than re-sanitizing.
pub(crate) fn find_tool_name_by_id<'a>(
    messages: &'a [ProviderMessage],
    id: &str,
) -> Option<&'a str> {
    messages
        .iter()
        .flat_map(|m| m.content.iter())
        .find_map(|c| match c {
            ProviderContent::ToolUse { id: tid, name, .. } if tid == id => Some(name.as_str()),
            _ => None,
        })
}

fn message_to_content(message: &ProviderMessage, all: &[ProviderMessage]) -> Option<Value> {
    let role = match message.role {
        ProviderRole::User => "user",
        ProviderRole::Assistant => "model",
    };

    // For assistant ("model") turns we apply the "first functionCall must have
    // a thoughtSignature" rule (gemini-cli historyHardening.ts:101-113). Walk
    // parts in order, tracking whether we've already emitted a functionCall
    // for this turn; only the first one falls back to the synthetic token.
    let is_model = matches!(message.role, ProviderRole::Assistant);
    let mut first_function_call_seen = false;

    let parts: Vec<Value> = message
        .content
        .iter()
        .filter_map(|c| {
            let needs_synthetic_fallback = is_model && !first_function_call_seen;
            let part = content_to_part(c, message.role, needs_synthetic_fallback, all);
            if part.is_some() && matches!(c, ProviderContent::ToolUse { .. }) {
                first_function_call_seen = true;
            }
            part
        })
        .collect();
    if parts.is_empty() {
        return None;
    }
    Some(json!({ "role": role, "parts": parts }))
}

fn content_to_part(
    content: &ProviderContent,
    role: ProviderRole,
    is_first_function_call_in_model_turn: bool,
    all: &[ProviderMessage],
) -> Option<Value> {
    match content {
        ProviderContent::Text(text) if text.is_empty() => None,
        ProviderContent::Text(text) => Some(json!({ "text": text })),
        ProviderContent::ToolUse {
            id: _,
            name,
            input,
            thought_signature,
        } => {
            // Gemini 3.x requires `thoughtSignature` on functionCall parts in
            // model-role history. The captured signature (round-tripped from
            // the SSE stream) is always preferred. The synthetic fallback is
            // applied ONLY to the first functionCall of a model turn when no
            // real signature is available (cold-start, history restore).
            // See research/gemini-cli/packages/core/src/core/geminiChat.ts:850-872
            //     research/gemini-cli/packages/core/src/utils/historyHardening.ts:101-113
            let mut part = json!({
                "functionCall": { "name": name, "args": input },
            });
            if role == ProviderRole::Assistant {
                if let Some(sig) = thought_signature.as_deref() {
                    part["thoughtSignature"] = json!(sig);
                } else if is_first_function_call_in_model_turn {
                    part["thoughtSignature"] = json!(SYNTHETIC_THOUGHT_SIGNATURE);
                }
            }
            Some(part)
        }
        ProviderContent::ToolResult {
            tool_use_id,
            content,
            is_error,
        } => {
            // Gemini maps the result back to its call by NAME — resolve the
            // original tool name from history; fall back to the raw id only
            // when the matching ToolUse isn't in scope (truncated history).
            let name = find_tool_name_by_id(all, tool_use_id).unwrap_or(tool_use_id.as_str());
            Some(json!({
                "functionResponse": {
                    "name": name,
                    "response": { "content": content, "isError": is_error },
                },
            }))
        }
        // RedactedThinking blocks from Anthropic don't have a Gemini equivalent.
        // Thought blocks without a valid thoughtSignature are rejected by the
        // Gemini server — strip them from history to avoid 400 errors.
        ProviderContent::RedactedThinking { .. } => None,
        // ServerToolUse/ServerToolResult/Attachment — no Gemini analog, drop.
        _ => None,
    }
}

fn tool_to_decl_augmented(tool: &ToolDef, augment_description: bool) -> Value {
    let name = sanitize_tool_name(&tool.name);
    let description = if augment_description {
        augment_tool_description(&tool.description, &tool.input_schema)
    } else {
        tool.description.clone()
    };
    json!({
        "name": name,
        "description": description,
        "parameters": tool.input_schema,
    })
}

/// Sanitize a tool name for Gemini API compatibility.
/// Gemini requires: `^[a-zA-Z_][a-zA-Z0-9_-]*$`
pub fn sanitize_tool_name(name: &str) -> String {
    if name.is_empty() {
        return "unnamed_tool".to_owned();
    }
    let mut out = String::with_capacity(name.len() + 2);
    let first = name.chars().next().unwrap();
    if first.is_ascii_digit() {
        out.push_str("t_");
    }
    for c in name.chars() {
        if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
            out.push(c);
        } else {
            out.push('_');
        }
    }
    out
}

/// Append a STRICT PARAMETERS summary to the tool description for Gemini models.
fn augment_tool_description(description: &str, schema: &Value) -> String {
    if description.contains("STRICT PARAMETERS:") {
        return description.to_owned();
    }
    let summary = summarize_schema_params(schema);
    if summary.is_empty() {
        return description.to_owned();
    }
    format!("{description}\n\nSTRICT PARAMETERS: {summary}")
}

/// Build a concise parameter summary from the tool's JSON schema.
fn summarize_schema_params(schema: &Value) -> String {
    let Some(props) = schema.get("properties").and_then(|v| v.as_object()) else {
        return String::new();
    };
    let required: Vec<&str> = schema
        .get("required")
        .and_then(|v| v.as_array())
        .map(|arr| arr.iter().filter_map(|v| v.as_str()).collect())
        .unwrap_or_default();

    let mut parts: Vec<String> = Vec::new();
    for (key, prop) in props.iter().take(10) {
        let typ = prop
            .get("type")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");
        let req = if required.contains(&key.as_str()) {
            " REQUIRED"
        } else {
            ""
        };
        parts.push(format!("{key}: {typ}{req}"));
    }
    if props.len() > 10 {
        parts.push(format!("…+{} more", props.len() - 10));
    }
    parts.join(", ")
}

/// Map a thinking budget number to a Gemini 3 thinkingLevel string.
pub(crate) fn budget_to_thinking_level(budget: u32) -> &'static str {
    match budget {
        0..=4096 => "low",
        4097..=16384 => "medium",
        _ => "high",
    }
}

// ─── SSE response parsing ────────────────────────────────────────────────────

/// One parsed Gemini SSE frame — the shape we care about from
/// `candidates[].content.parts[]` plus optional usage metadata.
#[derive(Debug, Deserialize, Default)]
struct GeminiSseFrame {
    #[serde(default)]
    candidates: Vec<GeminiCandidate>,
    #[serde(default, rename = "usageMetadata")]
    usage_metadata: Option<GeminiUsage>,
}

#[derive(Debug, Deserialize, Default)]
struct GeminiCandidate {
    #[serde(default)]
    content: Option<GeminiContent>,
    #[serde(default, rename = "finishReason")]
    finish_reason: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
struct GeminiContent {
    #[serde(default)]
    parts: Vec<GeminiPart>,
}

#[derive(Debug, Deserialize, Default)]
struct GeminiPart {
    #[serde(default)]
    text: Option<String>,
    #[serde(default)]
    thought: Option<bool>,
    #[serde(default, rename = "functionCall")]
    function_call: Option<GeminiFunctionCall>,
    /// Opaque base64 signature emitted by Gemini 3.x on `functionCall` and
    /// `thought` parts. Round-tripped verbatim on replay; see
    /// https://ai.google.dev/gemini-api/docs/thought-signatures
    #[serde(default, rename = "thoughtSignature")]
    thought_signature: Option<String>,
}

#[derive(Debug, Deserialize)]
struct GeminiFunctionCall {
    name: String,
    #[serde(default)]
    args: Value,
}

#[derive(Debug, Deserialize, Default)]
struct GeminiUsage {
    #[serde(default, rename = "promptTokenCount")]
    prompt_tokens: Option<u32>,
    #[serde(default, rename = "candidatesTokenCount")]
    candidates_tokens: Option<u32>,
    #[serde(default, rename = "cachedContentTokenCount")]
    cached_tokens: Option<u32>,
}

/// Per-stream cursor state needed to emit `TextDelta`s with monotonically
/// increasing indices, and to convert Gemini `finishReason` strings into a
/// jfc-side [`StopReason`] at end-of-stream.
#[derive(Debug, Default)]
struct ParserState {
    text_index: usize,
    text_started: bool,
    tool_index: usize,
}

impl ParserState {
    fn next_text_idx(&mut self) -> usize {
        if !self.text_started {
            self.text_started = true;
            self.text_index
        } else {
            self.text_index
        }
    }
    fn fresh_tool_idx(&mut self) -> usize {
        let n = self.tool_index;
        self.tool_index += 1;
        n
    }
}

/// Parse a single decoded SSE frame body (the part after `data: `) into a
/// flat sequence of [`StreamEvent`]s. Pure; tested without a live endpoint.
fn parse_frame(json: &str, state: &mut ParserState) -> Vec<StreamEvent> {
    let trimmed = json.trim();
    if trimmed.is_empty() || trimmed == "[DONE]" {
        return Vec::new();
    }
    let frame: GeminiSseFrame = match serde_json::from_str(trimmed) {
        Ok(f) => f,
        Err(err) => {
            return vec![StreamEvent::Error {
                message: format!("malformed Gemini SSE frame: {err}"),
            }];
        }
    };

    let mut out = Vec::new();
    for candidate in frame.candidates {
        emit_candidate_events(candidate, state, &mut out);
    }
    if let Some(usage) = frame.usage_metadata {
        out.push(StreamEvent::Usage {
            input_tokens: usage.prompt_tokens.unwrap_or(0),
            output_tokens: usage.candidates_tokens.unwrap_or(0),
            cache_read_tokens: usage.cached_tokens.unwrap_or(0),
            cache_write_tokens: 0,
        });
    }
    out
}

fn emit_candidate_events(
    candidate: GeminiCandidate,
    state: &mut ParserState,
    out: &mut Vec<StreamEvent>,
) {
    if let Some(content) = candidate.content {
        for part in content.parts {
            if let Some(ev) = part_to_event(part, state) {
                out.push(ev);
            }
        }
    }
    if let Some(reason) = candidate.finish_reason {
        out.push(StreamEvent::Done {
            stop_reason: map_finish_reason(&reason),
        });
    }
}

fn part_to_event(part: GeminiPart, state: &mut ParserState) -> Option<StreamEvent> {
    if let Some(call) = part.function_call {
        let idx = state.fresh_tool_idx();
        let input_json = serde_json::to_string(&call.args).unwrap_or_else(|_| "{}".into());
        return Some(StreamEvent::ToolDone {
            index: idx,
            tool_name: call.name,
            tool_use_id: format!("toolu_{}", Uuid::new_v4().simple()),
            input_json,
            // Capture the signature so it can be echoed back on replay.
            thought_signature: part.thought_signature,
        });
    }
    let text = part.text?;
    if text.is_empty() {
        return None;
    }
    if part.thought == Some(true) {
        Some(StreamEvent::ThinkingDelta {
            index: state.text_index,
            delta: text,
            estimated_tokens: None,
        })
    } else {
        let idx = state.next_text_idx();
        Some(StreamEvent::TextDelta {
            index: idx,
            delta: text,
        })
    }
}

fn map_finish_reason(reason: &str) -> StopReason {
    match reason {
        "STOP" | "" => StopReason::EndTurn,
        "MAX_TOKENS" => StopReason::MaxTokens,
        "TOOL_CODE" | "FUNCTION_CALL" | "TOOL_USE" => StopReason::ToolUse,
        "STOP_SEQUENCE" => StopReason::StopSequence,
        other => StopReason::Other(other.to_owned()),
    }
}

/// Wrap a `reqwest::Response` SSE stream as an [`EventStream`] of jfc
/// [`StreamEvent`]s by piping each parsed frame through [`parse_frame`].
/// Reuses the byte-level SSE parser from `jfc_anthropic_sdk::sse` so we
/// don't reinvent line-buffering or split-across-chunks handling.
pub fn into_event_stream(resp: reqwest::Response) -> EventStream {
    let raw = jfc_anthropic_sdk::sse::response_event_stream(resp);
    let stream = raw
        .scan(ParserState::default(), |state, frame| {
            let events = match frame {
                Ok(frame) if frame.data.is_empty() || frame.data == "[DONE]" => Vec::new(),
                Ok(frame) => parse_frame(&frame.data, state),
                Err(err) => vec![StreamEvent::Error {
                    message: format!("Antigravity SSE transport error: {err}"),
                }],
            };
            futures::future::ready(Some(events))
        })
        .flat_map(futures::stream::iter)
        .map(Ok);
    stream.boxed()
}

#[cfg(test)]
mod tests {
    use super::*;
    use jfc_provider::{ProviderContent, ProviderMessage, ProviderRole, StreamOptions};

    fn user_msg(text: &str) -> ProviderMessage {
        ProviderMessage {
            role: ProviderRole::User,
            content: vec![ProviderContent::Text(text.into())],
        }
    }

    #[test]
    fn build_gemini_request_wraps_with_project_and_model_normal() {
        let mut opts = StreamOptions::new("gemini-3-pro");
        opts.system = Some("be helpful".into());
        let body = build_request("proj-123", &[user_msg("hi")], &opts).expect("gemini req");
        assert_eq!(body["project"], "proj-123");
        // resolve_model_name maps "gemini-3-pro" → "gemini-3-pro-preview"
        assert_eq!(body["model"], "gemini-3-pro-preview");
        assert_eq!(body["userAgent"], "antigravity");
        assert_eq!(body["request"]["contents"][0]["role"], "user");
        assert_eq!(body["request"]["contents"][0]["parts"][0]["text"], "hi");
        // Antigravity system instruction is prepended for gemini-3-pro
        let sys_parts = body["request"]["systemInstruction"]["parts"]
            .as_array()
            .unwrap();
        assert!(
            sys_parts[0]["text"]
                .as_str()
                .unwrap()
                .contains("Antigravity")
        );
        // User system prompt is last
        assert_eq!(sys_parts.last().unwrap()["text"], "be helpful");
    }

    #[test]
    fn build_gemini_request_resolves_short_aliases_normal() {
        let opts = StreamOptions::new("gemini-pro");
        let body = build_request("p", &[user_msg("x")], &opts).unwrap();
        assert_eq!(body["model"], "gemini-3-pro");
    }

    #[test]
    fn build_request_routes_claude_models_to_claude_builder_normal() {
        let opts = StreamOptions::new("gemini-claude-sonnet-4-5");
        let body = build_request("p", &[user_msg("x")], &opts).unwrap();
        assert_eq!(body["model"], "gemini-claude-sonnet-4-5");
        // Claude builder always sets the VALIDATED tool-calling mode.
        assert_eq!(
            body["request"]["toolConfig"]["functionCallingConfig"]["mode"],
            "VALIDATED"
        );
    }

    #[test]
    fn build_claude_request_adds_thinking_config_for_thinking_models_normal() {
        let opts = StreamOptions::new("gemini-claude-sonnet-4-5-thinking");
        let body = build_request("p", &[user_msg("x")], &opts).unwrap();
        let thinking = &body["request"]["generationConfig"]["thinkingConfig"];
        assert_eq!(thinking["include_thoughts"], true);
        assert_eq!(thinking["thinking_budget"], 16384);
    }

    #[test]
    fn build_claude_request_omits_thinking_config_for_non_thinking_normal() {
        let opts = StreamOptions::new("gemini-claude-sonnet-4-5");
        let body = build_request("p", &[user_msg("x")], &opts).unwrap();
        assert!(
            body["request"]["generationConfig"]
                .get("thinkingConfig")
                .is_none()
        );
    }

    #[test]
    fn build_claude_request_bumps_max_output_when_below_budget_robust() {
        let mut opts = StreamOptions::new("gemini-claude-opus-4-5-thinking");
        opts.max_tokens = 4096; // below the default 16k thinking budget
        let body = build_request("p", &[user_msg("x")], &opts).unwrap();
        assert_eq!(
            body["request"]["generationConfig"]["maxOutputTokens"],
            64000
        );
    }

    #[test]
    fn build_claude_request_keeps_max_output_when_above_budget_normal() {
        let mut opts = StreamOptions::new("gemini-claude-opus-4-5-thinking");
        opts.thinking_budget = Some(8000);
        opts.max_tokens = 32000; // above the budget — leave it alone
        let body = build_request("p", &[user_msg("x")], &opts).unwrap();
        assert_eq!(
            body["request"]["generationConfig"]["maxOutputTokens"],
            32000
        );
        assert_eq!(
            body["request"]["generationConfig"]["thinkingConfig"]["thinking_budget"],
            8000
        );
    }

    #[test]
    fn build_gemini_request_omits_empty_text_and_handles_tool_use_normal() {
        // No captured signature → first model-role functionCall gets the
        // synthetic fallback (gemini-cli historyHardening.ts:101-113).
        let opts = StreamOptions::new("gemini-3-pro");
        let msg = ProviderMessage {
            role: ProviderRole::Assistant,
            content: vec![
                ProviderContent::Text("".into()),
                ProviderContent::ToolUse {
                    id: "x".into(),
                    name: "search".into(),
                    input: json!({ "q": "rust" }),
                    thought_signature: None,
                },
            ],
        };
        let body = build_request("p", &[msg], &opts).unwrap();
        let parts = &body["request"]["contents"][0]["parts"];
        assert_eq!(parts.as_array().unwrap().len(), 1);
        assert_eq!(parts[0]["functionCall"]["name"], "search");
        assert_eq!(
            parts[0]["thoughtSignature"], SYNTHETIC_THOUGHT_SIGNATURE,
            "first model-role functionCall without real signature gets the synthetic fallback"
        );
    }

    #[test]
    fn thought_signature_round_trip_normal() {
        // Real captured signature is echoed back verbatim — this is the happy
        // path. The synthetic fallback is NOT used when a real one exists.
        let real_sig = "EpgDCpUDAb4+9vtATVUpO7R/Du3cyW+qLtXqHV5MxjoY";
        let opts = StreamOptions::new("gemini-3-pro");
        let msg = ProviderMessage {
            role: ProviderRole::Assistant,
            content: vec![ProviderContent::ToolUse {
                id: "x".into(),
                name: "search".into(),
                input: json!({ "q": "rust" }),
                thought_signature: Some(real_sig.into()),
            }],
        };
        let body = build_request("p", &[msg], &opts).unwrap();
        let parts = &body["request"]["contents"][0]["parts"];
        assert_eq!(
            parts[0]["thoughtSignature"], real_sig,
            "captured signature must round-trip verbatim"
        );
    }

    #[test]
    fn thought_signature_only_first_function_call_in_turn_robust() {
        // Per Google's historyHardening.ts:105-113 — only the FIRST functionCall
        // in a model turn falls back to the synthetic token. Subsequent calls
        // in the same turn without real signatures get no signature.
        let opts = StreamOptions::new("gemini-3-pro");
        let msg = ProviderMessage {
            role: ProviderRole::Assistant,
            content: vec![
                ProviderContent::ToolUse {
                    id: "a".into(),
                    name: "first".into(),
                    input: json!({}),
                    thought_signature: None,
                },
                ProviderContent::ToolUse {
                    id: "b".into(),
                    name: "second".into(),
                    input: json!({}),
                    thought_signature: None,
                },
            ],
        };
        let body = build_request("p", &[msg], &opts).unwrap();
        let parts = body["request"]["contents"][0]["parts"].as_array().unwrap();
        assert_eq!(
            parts[0]["thoughtSignature"], SYNTHETIC_THOUGHT_SIGNATURE,
            "first functionCall must have the synthetic signature"
        );
        assert!(
            parts[1].get("thoughtSignature").is_none(),
            "subsequent functionCalls in the same turn must not have a synthetic signature"
        );
    }

    #[test]
    fn thought_signature_is_omitted_for_user_role_robust() {
        let opts = StreamOptions::new("gemini-3-pro");
        let msg = ProviderMessage {
            role: ProviderRole::User,
            content: vec![ProviderContent::ToolUse {
                id: "x".into(),
                name: "noop".into(),
                input: json!({}),
                thought_signature: None,
            }],
        };
        let body = build_request("p", &[msg], &opts).unwrap();
        let parts = &body["request"]["contents"][0]["parts"];
        assert!(
            parts[0].get("thoughtSignature").is_none(),
            "user-role functionCalls must NOT carry a thoughtSignature"
        );
    }

    #[test]
    fn tool_result_resolves_function_name_from_history_normal() {
        // A historical ToolResult must serialize its functionResponse with the
        // ORIGINAL tool name (resolved from the matching ToolUse), not the raw
        // transient id — otherwise Gemini 400s on the next turn.
        let opts = StreamOptions::new("gemini-3-pro");
        let history = vec![
            ProviderMessage {
                role: ProviderRole::Assistant,
                content: vec![ProviderContent::ToolUse {
                    id: "toolu_abc123".into(),
                    name: "read_file".into(),
                    input: json!({ "path": "x" }),
                    thought_signature: Some("sig".into()),
                }],
            },
            ProviderMessage {
                role: ProviderRole::User,
                content: vec![ProviderContent::ToolResult {
                    tool_use_id: "toolu_abc123".into(),
                    content: "file contents".into(),
                    is_error: false,
                }],
            },
        ];
        let body = build_request("p", &history, &opts).unwrap();
        let contents = body["request"]["contents"].as_array().unwrap();
        let func_resp = &contents[1]["parts"][0]["functionResponse"];
        assert_eq!(
            func_resp["name"], "read_file",
            "functionResponse.name must resolve to the original tool name"
        );
        assert_eq!(func_resp["response"]["content"], "file contents");
    }

    #[test]
    fn tool_result_unknown_id_falls_back_to_id_robust() {
        // When the matching ToolUse isn't in scope (truncated history), fall
        // back to the raw id rather than dropping the result entirely.
        let opts = StreamOptions::new("gemini-3-pro");
        let history = vec![ProviderMessage {
            role: ProviderRole::User,
            content: vec![ProviderContent::ToolResult {
                tool_use_id: "toolu_missing".into(),
                content: "x".into(),
                is_error: false,
            }],
        }];
        let body = build_request("p", &history, &opts).unwrap();
        let func_resp = &body["request"]["contents"][0]["parts"][0]["functionResponse"];
        assert_eq!(func_resp["name"], "toolu_missing");
    }

    #[test]
    fn parse_frame_handles_text_delta_normal() {
        let mut state = ParserState::default();
        let events = parse_frame(
            r#"{"candidates":[{"content":{"parts":[{"text":"hello"}]}}]}"#,
            &mut state,
        );
        assert_eq!(events.len(), 1);
        match &events[0] {
            StreamEvent::TextDelta { delta, .. } => assert_eq!(delta, "hello"),
            other => panic!("expected TextDelta, got {other:?}"),
        }
    }

    #[test]
    fn parse_frame_handles_function_call_normal() {
        let mut state = ParserState::default();
        let events = parse_frame(
            r#"{"candidates":[{"content":{"parts":[{"functionCall":{"name":"search","args":{"q":"x"}}}]}}]}"#,
            &mut state,
        );
        assert_eq!(events.len(), 1);
        match &events[0] {
            StreamEvent::ToolDone {
                tool_name,
                input_json,
                ..
            } => {
                assert_eq!(tool_name, "search");
                assert!(input_json.contains("\"q\""));
            }
            other => panic!("expected ToolDone, got {other:?}"),
        }
    }

    #[test]
    fn parse_frame_captures_thought_signature_normal() {
        // Real wire shape from Google's gemini-cli integration test corpus
        // (research/gemini-cli/integration-tests/browser-agent.persistent-session.responses).
        // The server attaches `thoughtSignature` to each functionCall part; we
        // must capture it so it can be echoed back on replay.
        let mut state = ParserState::default();
        let events = parse_frame(
            r#"{"candidates":[{"content":{"parts":[{"functionCall":{"name":"invoke_agent","args":{"prompt":"go"}},"thoughtSignature":"EpgDCpUDAb4+9vtATVUpO7R"}],"role":"model"}}]}"#,
            &mut state,
        );
        assert_eq!(events.len(), 1);
        match &events[0] {
            StreamEvent::ToolDone {
                tool_name,
                thought_signature,
                ..
            } => {
                assert_eq!(tool_name, "invoke_agent");
                assert_eq!(
                    thought_signature.as_deref(),
                    Some("EpgDCpUDAb4+9vtATVUpO7R"),
                    "signature must be captured from the SSE frame"
                );
            }
            other => panic!("expected ToolDone, got {other:?}"),
        }
    }

    #[test]
    fn parse_frame_handles_thinking_delta_normal() {
        let mut state = ParserState::default();
        let events = parse_frame(
            r#"{"candidates":[{"content":{"parts":[{"text":"thought","thought":true}]}}]}"#,
            &mut state,
        );
        assert_eq!(events.len(), 1);
        assert!(matches!(events[0], StreamEvent::ThinkingDelta { .. }));
    }

    #[test]
    fn parse_frame_emits_done_on_finish_reason_normal() {
        let mut state = ParserState::default();
        let events = parse_frame(r#"{"candidates":[{"finishReason":"STOP"}]}"#, &mut state);
        assert!(events.iter().any(|e| matches!(
            e,
            StreamEvent::Done {
                stop_reason: StopReason::EndTurn
            }
        )));
    }

    #[test]
    fn parse_frame_emits_usage_normal() {
        let mut state = ParserState::default();
        let events = parse_frame(
            r#"{"usageMetadata":{"promptTokenCount":42,"candidatesTokenCount":7}}"#,
            &mut state,
        );
        match events.last().expect("usage event") {
            StreamEvent::Usage {
                input_tokens,
                output_tokens,
                ..
            } => {
                assert_eq!(*input_tokens, 42);
                assert_eq!(*output_tokens, 7);
            }
            other => panic!("expected Usage, got {other:?}"),
        }
    }

    #[test]
    fn parse_frame_reports_malformed_json_as_error_event_robust() {
        let mut state = ParserState::default();
        let events = parse_frame("{not json", &mut state);
        assert_eq!(events.len(), 1);
        assert!(matches!(events[0], StreamEvent::Error { .. }));
    }
}
