//! Core provider abstraction: the `Provider` trait, `ModelSpec` parsing,
//! `StreamOptions` builder, cost accounting, and retry primitives shared by
//! every concrete backend in `jfc-providers`.
//!
//! This crate defines the contract — request/response shapes, streaming event
//! types, model identification, and pricing — without binding to any specific
//! API. Concrete implementations (Anthropic, OpenAI, Bedrock, Gemini, etc.)
//! live in `jfc-providers` and implement these traits.
#![allow(dead_code)]

use std::{borrow::Borrow, collections::HashMap, fmt, ops::Deref, pin::Pin};

use async_trait::async_trait;
use futures::Stream;
use reqwest::header::HeaderMap;
use sha2::{Digest, Sha256};

pub mod http;
pub mod retry;

macro_rules! string_id {
    ($name:ident) => {
        #[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
        #[serde(transparent)]
        pub struct $name(String);

        impl $name {
            pub fn new(value: impl Into<String>) -> Self {
                Self(value.into())
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl AsRef<str> for $name {
            fn as_ref(&self) -> &str {
                self.as_str()
            }
        }

        impl Borrow<str> for $name {
            fn borrow(&self) -> &str {
                self.as_str()
            }
        }

        impl Deref for $name {
            type Target = str;

            fn deref(&self) -> &Self::Target {
                self.as_str()
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(self.as_str())
            }
        }

        impl From<String> for $name {
            fn from(value: String) -> Self {
                Self(value)
            }
        }

        impl From<&str> for $name {
            fn from(value: &str) -> Self {
                Self(value.to_string())
            }
        }

        impl PartialEq<&str> for $name {
            fn eq(&self, other: &&str) -> bool {
                self.as_str() == *other
            }
        }

        impl PartialEq<$name> for &str {
            fn eq(&self, other: &$name) -> bool {
                *self == other.as_str()
            }
        }
    };
}

string_id!(ProviderId);
string_id!(ModelId);

/// A qualified model specifier: optionally prefixed with a provider name.
///
/// Parsed from strings in one of two forms:
///   - `"provider/model-id"` → explicit provider routing (e.g. `"openwebui/bedrock-claude-4-6-opus"`)
///   - `"model-id"`          → bare model, provider resolved by heuristic
///
/// Inspired by Cargo's `PackageIdSpec` (`name@version`) and Rust target triples
/// (`arch-vendor-os`): a structured identifier parsed from a single string with
/// `FromStr`, round-tripped via `Display`, and carrying enough type information
/// to route without ambient guessing.
///
/// The `/` separator was chosen because:
///   1. It's already the convention in the config (`"anthropic/claude-opus-4-7"`)
///   2. No known model id starts with a provider name followed by `/`
///   3. It mirrors container image naming (`registry/image:tag`)
///
/// # Examples
/// ```
/// use jfc_provider::ModelSpec;
///
/// let spec: ModelSpec = "anthropic/claude-opus-4-7".parse().unwrap();
/// assert_eq!(spec.provider().map(|p| p.as_str()), Some("anthropic"));
/// assert_eq!(spec.model().as_str(), "claude-opus-4-7");
/// assert_eq!(spec.to_string(), "anthropic/claude-opus-4-7");
///
/// let bare: ModelSpec = "claude-opus-4-7".parse().unwrap();
/// assert_eq!(bare.provider(), None);
/// assert_eq!(bare.model().as_str(), "claude-opus-4-7");
/// assert_eq!(bare.to_string(), "claude-opus-4-7");
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ModelSpec {
    provider: Option<ProviderId>,
    model: ModelId,
}

impl ModelSpec {
    /// Construct from parts explicitly.
    pub fn new(provider: Option<ProviderId>, model: ModelId) -> Self {
        Self { provider, model }
    }

    /// Construct a bare (provider-less) spec from a model id.
    pub fn bare(model: impl Into<ModelId>) -> Self {
        Self {
            provider: None,
            model: model.into(),
        }
    }

    /// Construct a fully qualified spec.
    pub fn qualified(provider: impl Into<ProviderId>, model: impl Into<ModelId>) -> Self {
        Self {
            provider: Some(provider.into()),
            model: model.into(),
        }
    }

    /// The explicit provider prefix, if present.
    pub fn provider(&self) -> Option<&ProviderId> {
        self.provider.as_ref()
    }

    /// The model identifier (the part after the `/`, or the whole string if bare).
    pub fn model(&self) -> &ModelId {
        &self.model
    }

    /// Consume self and return the model id (discarding provider info).
    pub fn into_model(self) -> ModelId {
        self.model
    }

    /// Whether this spec has an explicit provider prefix.
    pub fn is_qualified(&self) -> bool {
        self.provider.is_some()
    }
}

/// Parsing: split on the *first* `/`.
///
/// - Empty string → error
/// - No `/` → bare model
/// - `/` present → left = provider, right = model (both must be non-empty)
impl std::str::FromStr for ModelSpec {
    type Err = ModelSpecParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.is_empty() {
            return Err(ModelSpecParseError::Empty);
        }
        if let Some((provider, model)) = s.split_once('/') {
            if provider.is_empty() {
                return Err(ModelSpecParseError::EmptyProvider(s.to_owned()));
            }
            if model.is_empty() {
                return Err(ModelSpecParseError::EmptyModel(s.to_owned()));
            }
            Ok(ModelSpec {
                provider: Some(ProviderId::new(provider)),
                model: ModelId::new(model),
            })
        } else {
            Ok(ModelSpec {
                provider: None,
                model: ModelId::new(s),
            })
        }
    }
}

/// Display round-trips: `provider/model` when qualified, bare `model` otherwise.
impl fmt::Display for ModelSpec {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(ref p) = self.provider {
            write!(f, "{}/{}", p, self.model)
        } else {
            write!(f, "{}", self.model)
        }
    }
}

impl ModelSpec {
    /// Parse leniently: if `s` doesn't look like a `provider/model` spec
    /// (e.g. it has an empty provider or empty model component), treat the
    /// whole thing as a bare model id. Empty input still returns
    /// `Err(ModelSpecParseError::Empty)` — silently producing a spec with an
    /// empty `ModelId` would hide upstream "no model configured" bugs.
    ///
    /// For strict parsing prefer `s.parse::<ModelSpec>()`. Use this method
    /// only when the caller has a documented reason to fall back to a bare
    /// model id (e.g. an end-user-typed string from the model picker that
    /// might contain stray slashes).
    pub fn parse_lenient(s: &str) -> Result<Self, ModelSpecParseError> {
        if s.is_empty() {
            return Err(ModelSpecParseError::Empty);
        }
        Ok(s.parse()
            .unwrap_or_else(|_| ModelSpec::bare(ModelId::new(s))))
    }
}

impl serde::Serialize for ModelSpec {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> serde::Deserialize<'de> for ModelSpec {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = String::deserialize(deserializer)?;
        s.parse().map_err(serde::de::Error::custom)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModelSpecParseError {
    /// The input string was empty.
    Empty,
    /// Provider portion (before `/`) was empty: `"/model-id"`.
    EmptyProvider(String),
    /// Model portion (after `/`) was empty: `"provider/"`.
    EmptyModel(String),
}

impl fmt::Display for ModelSpecParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => write!(f, "model spec cannot be empty"),
            Self::EmptyProvider(s) => {
                write!(f, "model spec has empty provider: {:?}", s)
            }
            Self::EmptyModel(s) => {
                write!(f, "model spec has empty model after '/': {:?}", s)
            }
        }
    }
}

impl std::error::Error for ModelSpecParseError {}

/// Feature flags attached to a model, inspired by Koog's `LLMCapability`.
///
/// These are capability signals for request construction and routing policy,
/// not a pricing or entitlement source of truth. Provider implementations may
/// refine the inferred defaults with live catalog data over time.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize, serde::Deserialize,
)]
#[serde(rename_all = "snake_case")]
pub enum ModelCapability {
    Tools,
    ToolChoice,
    ToolStreaming,
    Vision,
    Documents,
    Audio,
    JsonSchema,
    StructuredOutput,
    Reasoning,
    PromptCaching,
    Moderation,
    Embeddings,
    OpenAiChatCompletions,
    OpenAiResponses,
    ServerTools,
}

/// Small, sorted capability set with stable serialization.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(transparent)]
pub struct ModelCapabilities(Vec<ModelCapability>);

impl ModelCapabilities {
    pub fn new<I>(capabilities: I) -> Self
    where
        I: IntoIterator<Item = ModelCapability>,
    {
        let mut values = capabilities.into_iter().collect::<Vec<_>>();
        values.sort();
        values.dedup();
        Self(values)
    }

    pub fn inferred(provider: &str, model: &str) -> Self {
        let provider = provider.to_ascii_lowercase();
        let model = model.to_ascii_lowercase();
        let mut caps = vec![ModelCapability::Tools, ModelCapability::ToolChoice];

        if matches!(
            provider.as_str(),
            "anthropic" | "anthropic-oauth" | "bedrock" | "vertex"
        ) || model.contains("claude")
            || model.contains("opus")
            || model.contains("sonnet")
            || model.contains("haiku")
            || model.contains("fable")
            || model.contains("mythos")
        {
            caps.extend([
                ModelCapability::ToolStreaming,
                ModelCapability::Vision,
                ModelCapability::Documents,
                ModelCapability::PromptCaching,
                ModelCapability::JsonSchema,
                ModelCapability::StructuredOutput,
                ModelCapability::ServerTools,
            ]);
            if model.contains("opus")
                || model.contains("sonnet")
                || model.contains("fable")
                || model.contains("mythos")
            {
                caps.push(ModelCapability::Reasoning);
            }
        }

        if matches!(
            provider.as_str(),
            "openai" | "codex" | "openrouter" | "openwebui" | "litellm"
        ) {
            caps.extend([
                ModelCapability::OpenAiChatCompletions,
                ModelCapability::JsonSchema,
                ModelCapability::StructuredOutput,
            ]);
        }

        if matches!(provider.as_str(), "gemini" | "antigravity") || model.contains("gemini") {
            caps.extend([
                ModelCapability::ToolStreaming,
                ModelCapability::Vision,
                ModelCapability::JsonSchema,
                ModelCapability::StructuredOutput,
                ModelCapability::Reasoning,
            ]);
        }

        if model.contains("embed") || model.contains("embedding") {
            caps.push(ModelCapability::Embeddings);
        }
        if model.contains("moderation") {
            caps.push(ModelCapability::Moderation);
        }
        if model.contains("audio") || model.contains("realtime") {
            caps.push(ModelCapability::Audio);
        }

        Self::new(caps)
    }

    pub fn contains(&self, capability: ModelCapability) -> bool {
        self.0.binary_search(&capability).is_ok()
    }

    pub fn insert(&mut self, capability: ModelCapability) {
        if self.contains(capability) {
            return;
        }
        self.0.push(capability);
        self.0.sort();
    }

    pub fn iter(&self) -> impl Iterator<Item = ModelCapability> + '_ {
        self.0.iter().copied()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl IntoIterator for ModelCapabilities {
    type Item = ModelCapability;
    type IntoIter = std::vec::IntoIter<ModelCapability>;

    fn into_iter(self) -> Self::IntoIter {
        self.0.into_iter()
    }
}

impl From<Vec<ModelCapability>> for ModelCapabilities {
    fn from(value: Vec<ModelCapability>) -> Self {
        Self::new(value)
    }
}

impl<const N: usize> From<[ModelCapability; N]> for ModelCapabilities {
    fn from(value: [ModelCapability; N]) -> Self {
        Self::new(value)
    }
}

/// Why a requested model resolved to the effective model JFC will call.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ModelResolutionReason {
    Requested,
    ExplicitProvider,
    CatalogMatch,
    Heuristic { rule: String },
    Fallback { reason: String },
    ProviderDefault { reason: String },
}

/// First-class model resolution record for runtime/council/economy/review
/// accounting. This mirrors Koog's `ResolvedModel`: keep the requested identity,
/// the effective identity, and the reason together instead of scattering that
/// context through logs.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ResolvedModel {
    pub requested: ModelSpec,
    pub effective: ModelSpec,
    pub reason: ModelResolutionReason,
    pub capabilities: ModelCapabilities,
    pub context_window_tokens: Option<usize>,
    pub max_output_tokens: Option<usize>,
}

impl ResolvedModel {
    pub fn new(
        requested: ModelSpec,
        effective: ModelSpec,
        reason: ModelResolutionReason,
        info: Option<&ModelInfo>,
    ) -> Self {
        let capabilities = info
            .map(|info| info.capabilities.clone())
            .unwrap_or_else(|| {
                let provider = effective.provider().map(ProviderId::as_str).unwrap_or("");
                ModelCapabilities::inferred(provider, effective.model().as_str())
            });
        Self {
            requested,
            effective,
            reason,
            capabilities,
            context_window_tokens: info.and_then(|info| info.context_window_tokens),
            max_output_tokens: info.and_then(|info| info.max_output_tokens),
        }
    }

    pub fn effective_model_id(&self) -> &ModelId {
        self.effective.model()
    }

    pub fn effective_provider(&self) -> Option<&ProviderId> {
        self.effective.provider()
    }
}

/// Stable cache-key material for future response/prompt caches.
///
/// Koog's cached executor resolves a model but does not include the effective
/// model or full tool schema in the key. JFC's key material keeps those inputs
/// mandatory so council/economy/advisor/review caches cannot cross-contaminate
/// answers between models, endpoints, params, or tool schema versions.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct PromptCacheKey {
    pub prompt_version: String,
    pub provider: ProviderId,
    pub endpoint: Option<String>,
    pub effective_model: ModelSpec,
    pub params_hash: String,
    pub prompt_hash: String,
    pub tool_schema_hash: String,
}

impl PromptCacheKey {
    pub fn new(
        prompt_version: impl Into<String>,
        provider: ProviderId,
        endpoint: Option<String>,
        effective_model: ModelSpec,
        params: &serde_json::Value,
        prompt: &str,
        tools: &[ToolDef],
    ) -> Self {
        Self {
            prompt_version: prompt_version.into(),
            provider,
            endpoint,
            effective_model,
            params_hash: stable_json_hash(params),
            prompt_hash: stable_bytes_hash(prompt.as_bytes()),
            tool_schema_hash: tool_schema_hash(tools),
        }
    }

    pub fn stable_string(&self) -> String {
        stable_json_hash(&serde_json::to_value(self).unwrap_or(serde_json::Value::Null))
    }
}

pub fn tool_schema_hash(tools: &[ToolDef]) -> String {
    let mut normalized = tools
        .iter()
        .map(|tool| {
            serde_json::json!({
                "name": &tool.name,
                "description": &tool.description,
                "input_schema": &tool.input_schema,
            })
        })
        .collect::<Vec<_>>();
    normalized.sort_by(|left, right| {
        left.get("name")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("")
            .cmp(
                right
                    .get("name")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or(""),
            )
    });
    stable_json_hash(&serde_json::Value::Array(normalized))
}

fn stable_json_hash(value: &serde_json::Value) -> String {
    stable_bytes_hash(value.to_string().as_bytes())
}

fn stable_bytes_hash(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hex::encode(&hasher.finalize()[..16])
}

#[derive(Debug, Clone)]
pub enum StreamEvent {
    TextDelta {
        index: usize,
        delta: String,
    },
    TextDone {
        index: usize,
        text: String,
    },
    ThinkingDelta {
        index: usize,
        delta: String,
        /// Server-provided thinking-token estimate: the *cumulative running
        /// total* for the current thinking block (the API's `estimated_tokens`,
        /// NOT the per-event `estimated_tokens_delta`). Consumers must take the
        /// delta against the previous value rather than summing, or they'll
        /// over-count. Approximate progress for spinners, not billed tokens.
        /// (Anthropic's official client accumulates these to report live thinking
        /// tok/s).
        estimated_tokens: Option<u32>,
    },
    ThinkingDone {
        index: usize,
        text: String,
    },
    /// Server-redacted thinking block — opaque base64 blob that must be
    /// round-tripped verbatim in subsequent requests. No deltas; the
    /// block arrives complete at content_block_start.
    RedactedThinkingDone {
        index: usize,
        data: String,
    },
    ToolDelta {
        index: usize,
        delta: String,
    },
    ToolDone {
        index: usize,
        tool_name: String,
        tool_use_id: String,
        input_json: String,
        /// Gemini 3.x thought signature attached to this function call.
        /// The server emits an opaque base64 blob on each `functionCall`
        /// part that must be echoed back verbatim when the turn is replayed
        /// (see https://ai.google.dev/gemini-api/docs/thought-signatures).
        /// `None` for non-Gemini providers.
        thought_signature: Option<String>,
    },
    /// Anthropic emitted a server-side tool result block (e.g.
    /// `web_search_tool_result`). These are produced server-side as
    /// part of the same sampling loop that emitted the matching
    /// `server_tool_use` block — they are NOT caller `tool_result`s.
    /// The provider parser captures the raw JSON content so the
    /// runtime can attach it to the streaming assistant message and
    /// re-emit it byte-faithfully on a `pause_turn` resend. See
    /// cli.js v142:394261 (consumer) and :441375 (round-trip).
    ServerToolResult {
        tool_use_id: String,
        tool_kind: ServerToolResultKind,
        content: serde_json::Value,
    },
    Done {
        stop_reason: StopReason,
    },
    Usage {
        input_tokens: u32,
        output_tokens: u32,
        cache_read_tokens: u32,
        cache_write_tokens: u32,
    },
    ResponseMetadata {
        response_id: String,
        /// Input token count from `message_start`. Populated early so
        /// context-window estimates are available even if the stream
        /// aborts before `message_delta`.
        input_tokens: Option<u64>,
    },
    Error {
        message: String,
    },
    /// Wire keep-alive — a provider liveness signal that carries no content
    /// (Anthropic SSE `ping` events, empty SSE comment frames). It exists so
    /// the runtime can prove the socket is alive and reset its stream idle
    /// watchdog even during phases that emit no semantic deltas (e.g. a long
    /// server-side thinking pause). Consumers MUST treat it as activity-only:
    /// no text, no tokens, no message mutation. Mirrors how Claude Code's
    /// byte-level watchdog resets on every raw byte (including pings).
    Keepalive,
    /// Emitted when a model fallback occurs — the requested model was
    /// unavailable (e.g. 529 overload) and a fallback was used instead.
    FallbackTriggered(FallbackTriggered),
}

/// Canonical, provider-neutral category for a stream frame.
///
/// `StreamEvent` is already the wire-neutral frame enum every provider parses
/// into, but its variants are fine-grained (separate delta/done/redacted
/// variants, ping, response-metadata). Consumers that reason about a frame's
/// *role* rather than its exact wire shape — telemetry, the idle watchdog, and
/// the "did this frame commit billable output" check — want the coarse
/// taxonomy the Koog-style canonical frame layer defines: text, reasoning,
/// tool-call, tool-result, usage, model-resolution, finish, and control. This
/// is that taxonomy, derived from a `StreamEvent` via [`StreamEvent::category`]
/// so the mapping lives in one place instead of being re-derived ad hoc at
/// each match site.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameCategory {
    /// Assistant-visible text (delta or completed block).
    Text,
    /// Model reasoning/thinking content (delta, completed, or redacted).
    Reasoning,
    /// A tool/function call the model is emitting (delta or completed).
    ToolCall,
    /// A server-side tool result block produced within the sampling loop.
    ToolResult,
    /// Token usage accounting (per-call or early input-token metadata).
    Usage,
    /// A change in the model that is serving the turn (e.g. a fallback).
    ModelResolution,
    /// A terminal frame ending the turn (carries a [`StopReason`]).
    Finish,
    /// Transport/error control with no assistant content: keepalive pings and
    /// error frames.
    Control,
}

impl StreamEvent {
    /// Classify this frame into its canonical [`FrameCategory`]. One source of
    /// truth for cross-provider frame-role reasoning.
    pub fn category(&self) -> FrameCategory {
        match self {
            StreamEvent::TextDelta { .. } | StreamEvent::TextDone { .. } => FrameCategory::Text,
            StreamEvent::ThinkingDelta { .. }
            | StreamEvent::ThinkingDone { .. }
            | StreamEvent::RedactedThinkingDone { .. } => FrameCategory::Reasoning,
            StreamEvent::ToolDelta { .. } | StreamEvent::ToolDone { .. } => FrameCategory::ToolCall,
            StreamEvent::ServerToolResult { .. } => FrameCategory::ToolResult,
            StreamEvent::Usage { .. } | StreamEvent::ResponseMetadata { .. } => {
                FrameCategory::Usage
            }
            StreamEvent::FallbackTriggered(_) => FrameCategory::ModelResolution,
            StreamEvent::Done { .. } => FrameCategory::Finish,
            StreamEvent::Keepalive | StreamEvent::Error { .. } => FrameCategory::Control,
        }
    }

    /// Whether this frame carries billable assistant output (text, reasoning,
    /// a tool call, or a server tool result). Used by the runtime to decide
    /// whether a turn produced committed output. Usage/metadata, finish,
    /// model-resolution, and control frames do not commit output.
    pub fn commits_output(&self) -> bool {
        matches!(
            self.category(),
            FrameCategory::Text
                | FrameCategory::Reasoning
                | FrameCategory::ToolCall
                | FrameCategory::ToolResult
        )
    }
}

/// Why a model fallback was triggered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FallbackReason {
    /// The requested model was not found / not enabled on the account (404).
    ModelNotFound,
    /// The model endpoint returned 529 (overloaded).
    Overloaded,
    /// The model refused the request (content policy, refusal stop_reason, etc.).
    ModelRefusal,
    /// The account lacks permission for the requested model (403 referencing the
    /// model) — distinct from "not found": the model exists but isn't allowed.
    PermissionDenied,
    /// Last-resort fallback: a non-retryable server error (5xx that isn't
    /// transient-retryable) exhausted the primary model and a fallback was
    /// configured, so we try it rather than fail the turn outright.
    ServerError,
}

impl fmt::Display for FallbackReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ModelNotFound => f.write_str("model not found"),
            Self::Overloaded => f.write_str("overloaded (529 threshold crossed)"),
            Self::ModelRefusal => f.write_str("model refused request"),
            Self::PermissionDenied => f.write_str("model access denied (403)"),
            Self::ServerError => f.write_str("server error (last-resort fallback)"),
        }
    }
}

/// Emitted when a model fallback occurs — the requested model was unavailable
/// and a fallback was used instead.
#[derive(Debug, Clone)]
pub struct FallbackTriggered {
    pub original_model: ModelId,
    pub fallback_model: ModelId,
    pub reason: FallbackReason,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StopReason {
    EndTurn,
    ToolUse,
    /// Anthropic's server-side sampling loop hit its iteration cap (e.g.
    /// after 10 server_tool_use rounds for web_search) and is asking the
    /// caller to re-send the conversation so the loop can resume. Per the
    /// Anthropic Messages API spec (mirrored verbatim in claude-code v142
    /// `cli.beautified.js:622686`): "To continue, re-send the user message
    /// and assistant response — the server will resume where it left off.
    /// Do NOT add an extra user message like 'Continue.'"
    PauseTurn,
    /// The provider ended the turn with an explicit refusal and produced no
    /// usable assistant content. This is distinct from an HTTP/content-policy
    /// fallback error: Anthropic can emit it as a successful SSE stop_reason.
    Refusal,
    MaxTokens,
    StopSequence,
    Other(String),
}

#[derive(Debug, Clone)]
pub struct ProviderMessage {
    pub role: ProviderRole,
    pub content: Vec<ProviderContent>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderRole {
    User,
    Assistant,
}

#[derive(Debug, Clone)]
pub enum ProviderContent {
    Text(String),
    ToolResult {
        tool_use_id: String,
        content: String,
        is_error: bool,
    },
    ToolUse {
        id: String,
        name: String,
        input: serde_json::Value,
        /// Gemini 3.x thought signature captured from the streaming response.
        /// Echoed back verbatim when the turn is replayed in conversation
        /// history (per https://ai.google.dev/gemini-api/docs/thought-signatures).
        /// `None` for non-Gemini providers and for legacy/pre-3.x Gemini turns.
        thought_signature: Option<String>,
    },
    /// Anthropic server-side tool invocation block. Wire type is
    /// `server_tool_use` (NOT `tool_use`). Per cli.js v142:7057 and
    /// :441090, the server expects these to round-trip unchanged on
    /// resend — re-serializing them as plain `tool_use` blocks
    /// breaks the server-side sampling loop's resumption logic, and
    /// fabricating a paired `tool_result` user message is forbidden
    /// (the server pairs `server_tool_use` with its own
    /// `web_search_tool_result` / equivalent block, not a caller
    /// tool_result).
    ServerToolUse {
        id: String,
        name: String,
        input: serde_json::Value,
    },
    /// Anthropic server-side tool result block (e.g.
    /// `web_search_tool_result`). The `tool_kind` discriminates the
    /// wire `"type"` field (`web_search_tool_result`,
    /// `code_execution_tool_result`, ...), and `content` is the raw
    /// JSON value the server returned (array of results, or
    /// `{ error_code }`). Round-trip lossless: parsed from SSE,
    /// stored on the assistant message, re-emitted verbatim on resend.
    /// See cli.js v142:394261 for the consumer shape and :441375 for
    /// the round-trip path.
    ServerToolResult {
        tool_use_id: String,
        tool_kind: ServerToolResultKind,
        content: serde_json::Value,
    },
    /// Image or PDF attachment carried as base64. Anthropic emits two
    /// distinct content-block shapes — `image` for PNG/JPEG/GIF/WebP
    /// and `document` for PDF — but both share the same source struct,
    /// so we keep one Rust variant and let the provider serializer
    /// decide. Non-Anthropic providers (OpenAI, OpenWebUI/LiteLLM)
    /// either reject these or use bespoke shapes; today they're a
    /// no-op for those providers.
    Attachment(jfc_core::Attachment),
    /// Server-redacted thinking block — opaque base64 blob. Must be
    /// round-tripped verbatim on subsequent requests (the API uses it
    /// to reconstruct thinking context server-side). No text content
    /// is ever shown to the user.
    RedactedThinking {
        data: String,
    },
}

// Canonical definition lives in jfc-core (shared with `ToolOutput` there);
// re-exported so provider-facing code keeps its `jfc_provider::` path.
pub use jfc_core::ServerToolResultKind;

#[derive(Debug, Clone)]
pub struct ToolDef {
    pub name: String,
    pub description: String,
    pub input_schema: serde_json::Value,
}

#[derive(Debug, Clone)]
pub struct StreamOptions {
    pub model: ModelId,
    pub system: Option<String>,
    pub max_tokens: u32,
    pub tools: Vec<ToolDef>,
    pub thinking_budget: Option<u32>,
    /// When true, use `{"type": "adaptive"}` instead of budget_tokens.
    /// Required for Opus 4.6+ and Sonnet 4.6+ which reject budget_tokens.
    pub adaptive_thinking: bool,
    /// Optional display mode for thinking: `"summarized"` or `"omitted"`.
    /// When `None`, the field is omitted from the request (Anthropic defaults to `"omitted"`).
    /// Set to `"summarized"` to receive thinking text in the response.
    pub thinking_display: Option<String>,
    /// Sampling temperature (0.0 - 2.0).
    pub temperature: Option<f64>,
    /// Nucleus sampling parameter (0.0 - 1.0).
    pub top_p: Option<f64>,
    /// Provider reasoning effort, e.g. "low", "medium", "high", "xhigh", "max".
    pub reasoning_effort: Option<String>,
    /// Provider-specific options merged into the request body.
    pub provider_options: HashMap<String, serde_json::Value>,
    /// Extra provider beta tokens appended to the Anthropic beta header.
    /// This mirrors Claude Code's `--betas` SDK passthrough while keeping the
    /// common provider abstraction provider-neutral.
    pub custom_betas: Vec<String>,
    /// When true, adds `fast-mode-2026-02-01` to the `anthropic-beta` header
    /// for lower-latency inference. Mirrors v2.1.139's `/fast` command.
    pub fast_mode: bool,
    /// When true, Anthropic native requests attach `eager_input_streaming`
    /// to local tool definitions and send the fine-grained tool streaming
    /// beta token when it is still required by the target model.
    pub eager_input_streaming: bool,
    /// When true, Anthropic native requests attach `strict: true` to local
    /// tool definitions and opt into structured-output validation.
    pub strict_tool_schemas: bool,
    /// Optional agentic loop token budget hint (beta: task-budgets-2026-03-13).
    /// Minimum 20_000. The model sees a countdown and self-moderates.
    /// Distinct from max_tokens (which is a hard server-enforced ceiling).
    pub task_budget_tokens: Option<u64>,
    /// Last assistant message ID from the previous turn. Sent in `diagnostics`
    /// so the server can track conversation flow for debugging/billing.
    pub previous_message_id: Option<String>,
    /// When compaction has saved significant tokens, hint to the API how many
    /// tokens we'd like it to help manage. Maps to `context_hint.target_tokens_saved`
    /// in the request body (context-hint-2026-04-09 beta).
    pub context_hint_tokens_saved: Option<u64>,
    /// When true, enables the `thinking-token-count-2026-05-13` beta which reports
    /// per-message thinking token counts in the response metadata.
    pub thinking_token_count: bool,
    /// When true, enables `mid-conversation-system-2026-04-07` beta allowing system
    /// messages to be injected mid-conversation (e.g. for context hints, reminders).
    pub mid_conversation_system: bool,
    /// When true, enables `cache-diagnosis-2026-04-07` beta which returns cache
    /// hit/miss diagnostics in the response for prompt caching tuning.
    pub cache_diagnosis: bool,
    /// When true, enables `prompt-caching-scope-2026-01-05` beta for scoped
    /// prompt caching. Always enabled by default since we always want cache hits.
    pub prompt_caching_scope: bool,
    /// Session ID for server-side request correlation (X-Claude-Code-Session-Id header).
    pub session_id: Option<String>,
    /// Optional Anthropic server-side advisor model. When set, Anthropic
    /// providers inject the `advisor_20260301` server tool and the matching
    /// beta token for this request.
    pub advisor_model: Option<ModelId>,
    /// When true, enables `summarize-connector-text-2026-03-13` so the
    /// server can return narration summaries for connector/user-message
    /// flows. Mirrors Claude Code 2.1.159's `narration_summaries` rollout.
    pub narration_summaries: bool,
}

impl StreamOptions {
    pub fn new(model: impl Into<ModelId>) -> Self {
        Self {
            model: model.into(),
            system: None,
            max_tokens: 8192,
            tools: Vec::new(),
            thinking_budget: None,
            adaptive_thinking: false,
            thinking_display: None,
            temperature: None,
            top_p: None,
            reasoning_effort: None,
            provider_options: HashMap::new(),
            custom_betas: Vec::new(),
            fast_mode: false,
            eager_input_streaming: false,
            strict_tool_schemas: false,
            task_budget_tokens: None,
            previous_message_id: None,
            context_hint_tokens_saved: None,
            thinking_token_count: false,
            mid_conversation_system: false,
            cache_diagnosis: false,
            prompt_caching_scope: true,
            session_id: None,
            advisor_model: None,
            narration_summaries: false,
        }
    }

    pub fn system(mut self, prompt: impl Into<String>) -> Self {
        self.system = Some(prompt.into());
        self
    }

    pub fn max_tokens(mut self, n: u32) -> Self {
        self.max_tokens = n;
        self
    }

    pub fn thinking(mut self, budget: u32) -> Self {
        self.thinking_budget = Some(budget);
        self
    }

    /// Use adaptive thinking (Opus 4.6+, Sonnet 4.6+). Ignores budget_tokens.
    pub fn adaptive(mut self) -> Self {
        self.adaptive_thinking = true;
        self
    }

    /// Set the display mode for thinking responses.
    /// Use `"summarized"` to receive thinking text; `"omitted"` (the default) suppresses it.
    pub fn thinking_display(mut self, display: impl Into<String>) -> Self {
        self.thinking_display = Some(display.into());
        self
    }

    pub fn temperature(mut self, t: f64) -> Self {
        self.temperature = Some(t);
        self
    }

    pub fn top_p(mut self, p: f64) -> Self {
        self.top_p = Some(p);
        self
    }

    pub fn reasoning_effort(mut self, effort: impl Into<String>) -> Self {
        self.reasoning_effort = Some(effort.into());
        self
    }

    pub fn tools(mut self, tools: Vec<ToolDef>) -> Self {
        self.tools = tools;
        self
    }

    pub fn custom_betas<I, S>(mut self, betas: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.custom_betas = betas
            .into_iter()
            .map(Into::into)
            .map(|beta| beta.trim().to_owned())
            .filter(|beta| !beta.is_empty())
            .collect();
        self
    }

    /// Enable or disable fast mode (lower-latency inference via `fast-mode-2026-02-01` beta).
    pub fn fast_mode(mut self, v: bool) -> Self {
        self.fast_mode = v;
        self
    }

    /// Enable or disable Anthropic fine-grained tool input streaming.
    pub fn eager_input_streaming(mut self, v: bool) -> Self {
        self.eager_input_streaming = v;
        self
    }

    /// Enable or disable strict Anthropic tool schema validation.
    pub fn strict_tool_schemas(mut self, v: bool) -> Self {
        self.strict_tool_schemas = v;
        self
    }

    /// Set the agentic loop task budget (beta: task-budgets-2026-03-13).
    /// Minimum 20_000 tokens — values below are clamped up.
    pub fn task_budget(mut self, tokens: u64) -> Self {
        self.task_budget_tokens = Some(tokens.max(20_000));
        self
    }

    pub fn previous_message_id(mut self, id: impl Into<String>) -> Self {
        self.previous_message_id = Some(id.into());
        self
    }

    pub fn advisor_model(mut self, model: impl Into<ModelId>) -> Self {
        self.advisor_model = Some(model.into());
        self
    }

    pub fn narration_summaries(mut self, enabled: bool) -> Self {
        self.narration_summaries = enabled;
        self
    }

    /// Enable the `thinking-token-count-2026-05-13` beta so the server reports
    /// per-delta thinking token estimates (`thinking_delta.estimated_tokens`).
    /// Without this the field is never present on the wire and the spinner's
    /// thinking-token chip stays at 0.
    pub fn thinking_token_count(mut self, enabled: bool) -> Self {
        self.thinking_token_count = enabled;
        self
    }
}

/// Non-streaming response for use by compaction and other single-shot API calls.
#[derive(Debug, Clone)]
pub struct CompletionResponse {
    pub content: String,
    pub usage: TokenUsage,
}

#[derive(Debug, Clone, Default)]
pub struct TokenUsage {
    pub input_tokens: usize,
    pub output_tokens: usize,
    pub cache_read_tokens: usize,
    pub cache_creation_tokens: usize,
}

impl TokenUsage {
    pub fn total_input(&self) -> usize {
        self.input_tokens + self.cache_read_tokens + self.cache_creation_tokens
    }

    /// Canonical "billable tokens for this call" derivation, shared by every
    /// usage sink (advisor budget, council budget, economy ledger).
    ///
    /// When the provider reported a real token count (`input + output > 0`) it
    /// is authoritative. Otherwise the count is estimated from `fallback_chars`
    /// at the workspace-wide 4-chars-per-token ratio using **floor** division
    /// (`chars / 4`), which is the rounding the budget gates have always used.
    /// The returned [`TokenSource`] tells each sink whether the number is
    /// authoritative or an estimate so it can apply its own policy (a cost
    /// ledger may treat an estimate as provisional).
    ///
    /// Before this method the same logic was copy-pasted three different ways
    /// (advisor inlined `/4`, council's `estimate_tokens`, economy's
    /// `div_ceil`); centralizing it keeps the budget gate and the cost ledger
    /// from drifting apart on the boundary token.
    pub fn billable_tokens(&self, fallback_chars: usize) -> (u64, TokenSource) {
        let reported = self.input_tokens + self.output_tokens;
        if reported > 0 {
            (reported as u64, TokenSource::Provider)
        } else {
            ((fallback_chars / 4) as u64, TokenSource::EstimatedFromChars)
        }
    }
}

/// Provenance of a billable-token count: whether it came from the provider's
/// own usage reporting or was estimated from character length. Sinks that
/// distinguish authoritative spend from provisional estimates (e.g. a USD cost
/// ledger) branch on this.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenSource {
    /// The provider reported a real token count.
    Provider,
    /// No usage was reported; the count is a chars/4 estimate.
    EstimatedFromChars,
}

/// One call's usage attributed to the model that actually served it.
///
/// Pairs the [`ResolvedModel`] (requested vs effective identity + reason) with
/// the call's [`TokenUsage`] and the provenance of the billable count. This is
/// the shared *fact* — "this much usage occurred against this resolved model" —
/// that advisor, council, and economy each record into their own sink. It is
/// deliberately a value type, not a recording trait: each sink still owns what
/// it does with the report (gate a budget, debit a USD balance, accumulate
/// per-account stats).
#[derive(Debug, Clone)]
pub struct UsageReport {
    pub resolved: ResolvedModel,
    pub usage: TokenUsage,
    /// Billable token count and its provenance, derived once via
    /// [`TokenUsage::billable_tokens`] against the call's text fallback.
    pub billable_tokens: u64,
    pub token_source: TokenSource,
}

impl UsageReport {
    /// Assemble a report from a resolved model, the call's usage, and the text
    /// fallback used to estimate tokens when the provider reported none.
    pub fn new(resolved: ResolvedModel, usage: TokenUsage, fallback_chars: usize) -> Self {
        let (billable_tokens, token_source) = usage.billable_tokens(fallback_chars);
        Self {
            resolved,
            usage,
            billable_tokens,
            token_source,
        }
    }

    /// The model id that actually served the call (the effective model).
    pub fn effective_model(&self) -> &ModelId {
        self.resolved.effective_model_id()
    }
}

#[derive(Debug, Clone)]
pub struct ModelInfo {
    pub id: ModelId,
    pub display_name: String,
    pub provider: ProviderId,
    pub capabilities: ModelCapabilities,
    pub context_window_tokens: Option<usize>,
    pub max_output_tokens: Option<usize>,
    /// Cost per million input tokens (USD)
    pub input_cost: Option<f64>,
    /// Cost per million output tokens (USD)
    pub output_cost: Option<f64>,
}

impl ModelInfo {
    pub fn new(
        id: impl Into<ModelId>,
        display_name: impl Into<String>,
        provider: impl Into<ProviderId>,
    ) -> Self {
        let id = id.into();
        let provider = provider.into();
        let capabilities = ModelCapabilities::inferred(provider.as_str(), id.as_str());
        Self {
            id,
            display_name: display_name.into(),
            provider,
            capabilities,
            context_window_tokens: None,
            max_output_tokens: None,
            input_cost: None,
            output_cost: None,
        }
    }

    pub fn with_context_window_tokens(mut self, tokens: impl Into<Option<usize>>) -> Self {
        self.context_window_tokens = tokens.into();
        self
    }

    pub fn with_max_output_tokens(mut self, tokens: impl Into<Option<usize>>) -> Self {
        self.max_output_tokens = tokens.into();
        self
    }

    pub fn with_capabilities(mut self, capabilities: impl Into<ModelCapabilities>) -> Self {
        self.capabilities = capabilities.into();
        self
    }

    pub fn with_added_capability(mut self, capability: ModelCapability) -> Self {
        self.capabilities.insert(capability);
        self
    }

    pub fn supports(&self, capability: ModelCapability) -> bool {
        self.capabilities.contains(capability)
    }

    pub fn with_costs(mut self, input: Option<f64>, output: Option<f64>) -> Self {
        self.input_cost = input;
        self.output_cost = output;
        self
    }
}

pub type EventStream = Pin<Box<dyn Stream<Item = anyhow::Result<StreamEvent>> + Send>>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderErrorKind {
    Authentication,
    Permission,
    RateLimit,
    Overloaded,
    NotFound,
    InvalidRequest,
    Network,
    Server,
    Unknown,
}

#[derive(Debug, Clone)]
pub struct ProviderError {
    pub provider: String,
    pub kind: ProviderErrorKind,
    pub status: Option<u16>,
    pub message: String,
    pub raw: Option<String>,
}

impl ProviderError {
    pub fn network(provider: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            provider: provider.into(),
            kind: ProviderErrorKind::Network,
            status: None,
            message: message.into(),
            raw: None,
        }
    }

    pub fn api_status(provider: impl Into<String>, status: u16, raw: impl Into<String>) -> Self {
        let raw = raw.into();
        let message = extract_provider_error_message(&raw)
            .filter(|m| !m.trim().is_empty())
            .unwrap_or_else(|| retry::friendly_error_message(status, &raw));
        Self {
            provider: provider.into(),
            kind: kind_from_status_and_body(status, &raw),
            status: Some(status),
            message,
            raw: Some(raw),
        }
    }

    pub fn with_raw(mut self, raw: impl Into<String>) -> Self {
        self.raw = Some(raw.into());
        self
    }

    pub fn is_retryable(&self) -> bool {
        matches!(
            self.kind,
            ProviderErrorKind::RateLimit
                | ProviderErrorKind::Overloaded
                | ProviderErrorKind::Network
                | ProviderErrorKind::Server
        )
    }
}

impl fmt::Display for ProviderError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.status {
            Some(status) => write!(f, "{} API error {status}: {}", self.provider, self.message)?,
            None => write!(f, "{} error: {}", self.provider, self.message)?,
        }
        if let Some(raw) = self.raw.as_deref().filter(|raw| !raw.trim().is_empty()) {
            write!(f, "\n  raw: {raw}")?;
        }
        Ok(())
    }
}

impl std::error::Error for ProviderError {}

fn kind_from_status_and_body(status: u16, raw: &str) -> ProviderErrorKind {
    let lower = raw.to_ascii_lowercase();
    if lower.contains("authentication_error") || matches!(status, 401) {
        ProviderErrorKind::Authentication
    } else if lower.contains("permission_error") || matches!(status, 403) {
        ProviderErrorKind::Permission
    } else if lower.contains("rate_limit_error") || matches!(status, 429) {
        ProviderErrorKind::RateLimit
    } else if lower.contains("overloaded_error") {
        ProviderErrorKind::Overloaded
    } else if lower.contains("not_found_error") || matches!(status, 404) {
        ProviderErrorKind::NotFound
    } else if lower.contains("invalid_request_error") || matches!(status, 400 | 422) {
        ProviderErrorKind::InvalidRequest
    } else if matches!(status, 500..=599) {
        ProviderErrorKind::Server
    } else {
        ProviderErrorKind::Unknown
    }
}

fn extract_provider_error_message(raw: &str) -> Option<String> {
    let value = serde_json::from_str::<serde_json::Value>(raw).ok()?;
    for pointer in [
        "/error/message",
        "/error",
        "/detail/message",
        "/detail/error/message",
        "/detail",
        "/message",
    ] {
        if let Some(value) = value.pointer(pointer) {
            if let Some(message) = value.as_str() {
                return Some(message.to_owned());
            }
            if value.is_object() || value.is_array() {
                return Some(value.to_string());
            }
        }
    }
    None
}

/// How a provider's stream encodes tool activity. Used by the renderer to decide
/// whether assistant text needs post-parsing (some backends interleave tool data
/// inline as text instead of using the API's structured event types).
///
/// We treat this as a value, not a trait object, because there are only a handful
/// of conventions in the wild and they're easy to enumerate exhaustively.
/// Adding a new one is a single arm + a renderer dispatch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StreamConvention {
    /// Native Anthropic Messages API: structured `content_block_start`/`delta`/`stop`
    /// events for `tool_use` blocks. No inline parsing of assistant text needed.
    /// Matches CC v126's behavior described in the user's research notes.
    AnthropicNative,
    /// OpenAI-compatible streaming with structured `delta.tool_calls`. The text
    /// stream itself is plain text — no inline tool tags. (jfc doesn't surface
    /// these tool calls yet for OpenAI-compatible providers, but the convention
    /// is recorded so the renderer doesn't trip on inline-tag detection.)
    OpenAiNative,
    /// Model emits XML-ish `<tool_call>{...}</tool_call>` and
    /// `<tool_result>...</tool_result>` blocks interleaved with prose. Triggered
    /// when the model wasn't sent a structured `tools` array and falls back to
    /// its training-time XML convention; an upstream shim (e.g. on the user's
    /// OpenWebUI instance) may then execute them and re-inject the result tags.
    /// Renderer must split text into segments and render tool blocks distinctly.
    InlineXmlTags,
}

/// Sealed-trait machinery for `Provider`.
///
/// Following the `t-libs-api` "sealed traits" guidance: implementations of
/// `Provider` live exclusively inside this crate's `providers/` module.
/// External crates can still *reference* the trait — call its methods, hold
/// `Arc<dyn Provider>` — but cannot add their own impls, because
/// `seal::Sealed` is only implementable from within this crate.
///
/// Even though jfc isn't a published library today, sealing protects
/// future evolution: if the crate ever splits or is re-exported, downstream
/// callers cannot lock us out of adding new required methods.
pub mod seal {
    pub trait Sealed {}
}

/// Sealed: implementations live inside the jfc crate's `providers/`
/// module. External crates cannot implement `Provider` directly — extend by
/// adding a new module under `providers/` and registering it in the
/// dispatch table in `main.rs`.
#[async_trait]
pub trait Provider: Send + Sync + seal::Sealed {
    fn name(&self) -> &str;

    fn available_models(&self) -> Vec<ModelInfo>;

    /// Declare how this provider encodes tool activity in its stream. The
    /// renderer uses this to decide whether to invoke the inline-tag parser on
    /// assistant text. Defaults to `AnthropicNative` — opt in to other
    /// conventions per provider.
    fn stream_convention(&self) -> StreamConvention {
        StreamConvention::AnthropicNative
    }

    /// Fetch models dynamically (e.g. from an API). Defaults to the static list.
    async fn fetch_models(&self) -> anyhow::Result<Vec<ModelInfo>> {
        Ok(self.available_models())
    }

    /// Ensure provider authentication is currently usable.
    ///
    /// API-key providers usually keep the default no-op implementation. OAuth
    /// providers override this to refresh access tokens before requests.
    async fn ensure_auth(&self) -> anyhow::Result<()> {
        Ok(())
    }

    /// Additional auth headers for provider-specific transports.
    fn auth_headers(&self) -> HeaderMap {
        HeaderMap::new()
    }

    /// Optional URL rewrite hook for providers whose auth mode targets a
    /// different backend than their OpenAI-compatible public API surface.
    fn rewrite_url(&self, _original: &str) -> Option<String> {
        None
    }

    async fn stream(
        &self,
        messages: Vec<ProviderMessage>,
        options: &StreamOptions,
    ) -> anyhow::Result<EventStream>;

    /// Non-streaming completion for compaction summarization.
    ///
    /// Default impl returns an error — providers must opt in.
    async fn complete(
        &self,
        _messages: Vec<ProviderMessage>,
        _options: &StreamOptions,
    ) -> anyhow::Result<CompletionResponse> {
        anyhow::bail!("{} does not support non-streaming completion", self.name())
    }

    /// Count input tokens for a request via the provider's tokenizer/endpoint
    /// (e.g. Anthropic's `/v1/messages/count_tokens`). Returns the true input
    /// token count. Default errors — callers fall back to a chars/4 estimate,
    /// so only providers with a real count endpoint need to override.
    async fn count_tokens(
        &self,
        _model: &str,
        _system: Option<String>,
        _messages: Vec<ProviderMessage>,
    ) -> anyhow::Result<u64> {
        anyhow::bail!("{} does not support count_tokens", self.name())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fallback_reason_display_is_distinct_normal() {
        // Every variant must render a distinct, human-readable label so the
        // fallback toast tells the user *why* the model switched.
        let labels: Vec<String> = [
            FallbackReason::ModelNotFound,
            FallbackReason::Overloaded,
            FallbackReason::ModelRefusal,
            FallbackReason::PermissionDenied,
            FallbackReason::ServerError,
        ]
        .iter()
        .map(|r| r.to_string())
        .collect();
        // All distinct.
        let mut deduped = labels.clone();
        deduped.sort();
        deduped.dedup();
        assert_eq!(
            deduped.len(),
            labels.len(),
            "labels must be distinct: {labels:?}"
        );
        assert!(labels[3].contains("access denied"));
        assert!(labels[4].contains("last-resort"));
    }

    #[test]
    fn model_info_infers_capabilities_normal() {
        let claude = ModelInfo::new("claude-opus-4-8", "Claude Opus", "anthropic");
        assert!(claude.supports(ModelCapability::Tools));
        assert!(claude.supports(ModelCapability::PromptCaching));
        assert!(claude.supports(ModelCapability::Reasoning));
        assert!(claude.supports(ModelCapability::JsonSchema));

        let openai = ModelInfo::new("gpt-5.1", "GPT", "openai");
        assert!(openai.supports(ModelCapability::OpenAiChatCompletions));
        assert!(openai.supports(ModelCapability::StructuredOutput));
    }

    #[test]
    fn resolved_model_keeps_requested_and_effective_normal() {
        let info = ModelInfo::new("claude-sonnet-4-6", "Sonnet", "anthropic")
            .with_context_window_tokens(Some(200_000usize))
            .with_max_output_tokens(Some(128_000usize));
        let resolved = ResolvedModel::new(
            ModelSpec::qualified("openrouter", "anthropic/claude-sonnet-4-6"),
            ModelSpec::qualified("anthropic", "claude-sonnet-4-6"),
            ModelResolutionReason::Fallback {
                reason: "primary provider unavailable".to_owned(),
            },
            Some(&info),
        );
        assert_eq!(
            resolved.requested.to_string(),
            "openrouter/anthropic/claude-sonnet-4-6"
        );
        assert_eq!(
            resolved.effective.to_string(),
            "anthropic/claude-sonnet-4-6"
        );
        assert_eq!(resolved.context_window_tokens, Some(200_000));
        assert!(resolved.capabilities.contains(ModelCapability::Reasoning));
    }

    #[test]
    fn prompt_cache_key_changes_with_model_and_tool_schema_robust() {
        let params = serde_json::json!({"temperature": 0.2});
        let tools = vec![ToolDef {
            name: "Read".into(),
            description: "Read a file".into(),
            input_schema: serde_json::json!({"type": "object", "required": ["file_path"]}),
        }];
        let key_a = PromptCacheKey::new(
            "system-v1",
            ProviderId::new("anthropic"),
            Some("https://api.anthropic.com".into()),
            ModelSpec::qualified("anthropic", "claude-sonnet-4-6"),
            &params,
            "hello",
            &tools,
        );
        let key_b = PromptCacheKey::new(
            "system-v1",
            ProviderId::new("anthropic"),
            Some("https://api.anthropic.com".into()),
            ModelSpec::qualified("anthropic", "claude-opus-4-8"),
            &params,
            "hello",
            &tools,
        );
        assert_ne!(key_a.stable_string(), key_b.stable_string());

        let mut changed_tools = tools;
        changed_tools[0].input_schema = serde_json::json!({"type": "object"});
        let key_c = PromptCacheKey::new(
            "system-v1",
            ProviderId::new("anthropic"),
            Some("https://api.anthropic.com".into()),
            ModelSpec::qualified("anthropic", "claude-sonnet-4-6"),
            &params,
            "hello",
            &changed_tools,
        );
        assert_ne!(key_a.tool_schema_hash, key_c.tool_schema_hash);
    }

    // ─── ModelSpec parsing ────────────────────────────────────────────────

    #[test]
    fn parse_qualified_spec_normal() {
        let spec: ModelSpec = "anthropic/claude-opus-4-7".parse().unwrap();
        assert_eq!(spec.provider().unwrap().as_str(), "anthropic");
        assert_eq!(spec.model().as_str(), "claude-opus-4-7");
        assert!(spec.is_qualified());
    }

    #[test]
    fn parse_bare_spec_normal() {
        let spec: ModelSpec = "claude-opus-4-7".parse().unwrap();
        assert_eq!(spec.provider(), None);
        assert_eq!(spec.model().as_str(), "claude-opus-4-7");
        assert!(!spec.is_qualified());
    }

    #[test]
    fn parse_openwebui_prefix_normal() {
        let spec: ModelSpec = "openwebui/bedrock-claude-4-6-opus".parse().unwrap();
        assert_eq!(spec.provider().unwrap().as_str(), "openwebui");
        assert_eq!(spec.model().as_str(), "bedrock-claude-4-6-opus");
    }

    #[test]
    fn parse_anthropic_oauth_prefix_normal() {
        let spec: ModelSpec = "anthropic-oauth/claude-sonnet-4-6".parse().unwrap();
        assert_eq!(spec.provider().unwrap().as_str(), "anthropic-oauth");
        assert_eq!(spec.model().as_str(), "claude-sonnet-4-6");
    }

    #[test]
    fn provider_error_extracts_common_json_shapes() {
        let err = ProviderError::api_status(
            "openrouter",
            429,
            r#"{"error":{"message":"rate limited","type":"rate_limit_error"}}"#,
        );
        assert_eq!(err.kind, ProviderErrorKind::RateLimit);
        assert_eq!(err.message, "rate limited");

        let err = ProviderError::api_status("openwebui", 400, r#"{"detail":"Model not found"}"#);
        assert_eq!(err.kind, ProviderErrorKind::InvalidRequest);
        assert_eq!(err.message, "Model not found");
    }

    #[test]
    fn parse_empty_is_error_robust() {
        let err = "".parse::<ModelSpec>().unwrap_err();
        assert_eq!(err, ModelSpecParseError::Empty);
    }

    #[test]
    fn parse_leading_slash_is_error_robust() {
        let err = "/claude-opus-4-7".parse::<ModelSpec>().unwrap_err();
        assert!(matches!(err, ModelSpecParseError::EmptyProvider(_)));
    }

    #[test]
    fn parse_trailing_slash_is_error_robust() {
        let err = "anthropic/".parse::<ModelSpec>().unwrap_err();
        assert!(matches!(err, ModelSpecParseError::EmptyModel(_)));
    }

    #[test]
    fn parse_multiple_slashes_takes_first_normal() {
        // "openrouter/anthropic/claude-3.5-sonnet" → provider="openrouter", model="anthropic/claude-3.5-sonnet"
        let spec: ModelSpec = "openrouter/anthropic/claude-3.5-sonnet".parse().unwrap();
        assert_eq!(spec.provider().unwrap().as_str(), "openrouter");
        assert_eq!(spec.model().as_str(), "anthropic/claude-3.5-sonnet");
    }

    // ─── ModelSpec display round-trip ─────────────────────────────────────

    #[test]
    fn display_qualified_roundtrips_normal() {
        let input = "anthropic/claude-opus-4-7";
        let spec: ModelSpec = input.parse().unwrap();
        assert_eq!(spec.to_string(), input);
    }

    #[test]
    fn display_bare_roundtrips_normal() {
        let input = "claude-opus-4-7";
        let spec: ModelSpec = input.parse().unwrap();
        assert_eq!(spec.to_string(), input);
    }

    // ─── ModelSpec serde ──────────────────────────────────────────────────

    #[test]
    fn serde_roundtrip_qualified_normal() {
        let spec = ModelSpec::qualified("openwebui", "bedrock-claude-4-6-opus");
        let json = serde_json::to_string(&spec).unwrap();
        assert_eq!(json, "\"openwebui/bedrock-claude-4-6-opus\"");
        let back: ModelSpec = serde_json::from_str(&json).unwrap();
        assert_eq!(back, spec);
    }

    #[test]
    fn serde_roundtrip_bare_normal() {
        let spec = ModelSpec::bare("claude-opus-4-7");
        let json = serde_json::to_string(&spec).unwrap();
        assert_eq!(json, "\"claude-opus-4-7\"");
        let back: ModelSpec = serde_json::from_str(&json).unwrap();
        assert_eq!(back, spec);
    }

    #[test]
    fn serde_deserialize_empty_is_error_robust() {
        let result = serde_json::from_str::<ModelSpec>("\"\"");
        assert!(result.is_err());
    }

    // ─── ModelSpec constructors ───────────────────────────────────────────

    // Normal: parse_lenient threads a qualified spec through to the strict
    // FromStr path (no fallback needed).
    #[test]
    fn parse_lenient_qualified_normal() {
        let spec = ModelSpec::parse_lenient("openwebui/gpt-4o").unwrap();
        assert_eq!(spec.provider().unwrap().as_str(), "openwebui");
        assert_eq!(spec.model().as_str(), "gpt-4o");
    }

    // Normal: parse_lenient on a bare id still routes through FromStr —
    // bare ids parse cleanly without needing the fallback.
    #[test]
    fn parse_lenient_bare_normal() {
        let spec = ModelSpec::parse_lenient("claude-opus-4-7").unwrap();
        assert_eq!(spec.provider(), None);
        assert_eq!(spec.model().as_str(), "claude-opus-4-7");
    }

    // Robust: parse_lenient on a malformed `provider/model` (empty model
    // after the slash) falls back to treating the whole string as a bare
    // model id — the lenient contract — instead of erroring.
    #[test]
    fn parse_lenient_empty_model_falls_back_robust() {
        let spec = ModelSpec::parse_lenient("anthropic/").unwrap();
        assert!(!spec.is_qualified());
        assert_eq!(spec.model().as_str(), "anthropic/");
    }

    // Robust: parse_lenient on the empty string still errors — silently
    // producing a spec with an empty ModelId would hide a "no model
    // configured" bug, and is the entire reason this method is a separate
    // call instead of a `From<String>` impl.
    #[test]
    fn parse_lenient_empty_is_error_robust() {
        let err = ModelSpec::parse_lenient("").unwrap_err();
        assert_eq!(err, ModelSpecParseError::Empty);
    }

    // ─── ModelSpec error Display ──────────────────────────────────────────

    #[test]
    fn error_display_messages_robust() {
        assert!(ModelSpecParseError::Empty.to_string().contains("empty"));
        assert!(
            ModelSpecParseError::EmptyProvider("/foo".to_owned())
                .to_string()
                .contains("empty provider")
        );
        assert!(
            ModelSpecParseError::EmptyModel("foo/".to_owned())
                .to_string()
                .contains("empty model")
        );
    }

    // ─── ModelSpec constructors (new / qualified / bare / into_model) ──────

    // Normal: ModelSpec::new threads explicit provider + model through. Bypasses
    // the parser so callers that already have typed ids skip the FromStr round-trip.
    #[test]
    fn model_spec_new_with_provider_normal() {
        let spec = ModelSpec::new(
            Some(ProviderId::new("anthropic")),
            ModelId::new("claude-opus-4-7"),
        );
        assert!(spec.is_qualified());
        assert_eq!(spec.provider().unwrap().as_str(), "anthropic");
        assert_eq!(spec.model().as_str(), "claude-opus-4-7");
    }

    // Normal: ModelSpec::new with provider=None matches the bare form so
    // serializing a programmatically-built spec round-trips correctly.
    #[test]
    fn model_spec_new_no_provider_normal() {
        let spec = ModelSpec::new(None, ModelId::new("bare-model"));
        assert!(!spec.is_qualified());
        assert!(spec.provider().is_none());
    }

    // Normal: into_model discards the provider prefix — used by callers that
    // need only the model id (e.g. stream.rs's max_output_tokens_for).
    #[test]
    fn model_spec_into_model_strips_provider_normal() {
        let spec = ModelSpec::qualified("anthropic", "claude-opus-4-7");
        let id = spec.into_model();
        assert_eq!(id.as_str(), "claude-opus-4-7");
    }

    // ─── ModelId / ProviderId trait impls ──────────────────────────────────

    // Normal: ProviderId AsRef<str> + Display match — both produce the same
    // string representation. Verifies the macro-generated impls land correctly.
    #[test]
    fn provider_id_display_and_asref_match_normal() {
        let p = ProviderId::new("openwebui");
        assert_eq!(p.as_ref(), "openwebui");
        assert_eq!(p.to_string(), "openwebui");
    }

    // Normal: ModelId implements Deref<Target=str> so it can be passed where
    // &str is expected without an explicit `.as_str()`.
    #[test]
    fn model_id_deref_to_str_normal() {
        let id = ModelId::new("claude-opus-4-7");
        // Use Deref coercion implicitly.
        fn takes_str(s: &str) -> usize {
            s.len()
        }
        assert_eq!(takes_str(&id), "claude-opus-4-7".len());
    }

    // Normal: ProviderId equality with raw &str works in both directions
    // — required for the dispatcher in main.rs which matches against literals.
    #[test]
    fn provider_id_str_equality_normal() {
        let p = ProviderId::new("anthropic");
        assert_eq!(p, "anthropic");
        // The reverse impl exists for ergonomic comparisons.
        let _: &str = "anthropic";
        assert!(p == "anthropic");
    }

    // ─── StreamOptions builder ─────────────────────────────────────────────

    // Normal: a fresh StreamOptions has the documented defaults — 8192 max
    // tokens, no system prompt, no tools, no thinking.
    #[test]
    fn stream_options_new_defaults_normal() {
        let opts = StreamOptions::new("any-model");
        assert_eq!(opts.model.as_str(), "any-model");
        assert_eq!(opts.max_tokens, 8192);
        assert!(opts.system.is_none());
        assert!(opts.tools.is_empty());
        assert!(opts.thinking_budget.is_none());
        assert!(!opts.adaptive_thinking);
    }

    // Normal: every builder method is chainable and sets the documented field.
    #[test]
    fn stream_options_builder_chains_normal() {
        let opts = StreamOptions::new("m")
            .system("be helpful")
            .max_tokens(64_000)
            .thinking(8_192)
            .tools(vec![ToolDef {
                name: "Bash".into(),
                description: "exec".into(),
                input_schema: serde_json::json!({}),
            }]);
        assert_eq!(opts.system.as_deref(), Some("be helpful"));
        assert_eq!(opts.max_tokens, 64_000);
        assert_eq!(opts.thinking_budget, Some(8_192));
        assert_eq!(opts.tools.len(), 1);
        assert!(!opts.adaptive_thinking);
    }

    // Normal: .adaptive() flips the flag without clearing the legacy budget,
    // so callers (e.g. anthropic.rs::build_body) get to decide which one
    // actually goes on the wire.
    #[test]
    fn stream_options_adaptive_flag_normal() {
        let opts = StreamOptions::new("m").thinking(4096).adaptive();
        assert!(opts.adaptive_thinking);
        assert_eq!(opts.thinking_budget, Some(4096));
    }

    // Robust: passing 0 for max_tokens is allowed at the type level (u32) —
    // the caller is responsible for rejecting it before sending. We verify
    // the builder doesn't reinterpret 0 to mean "use the default", which
    // would be a subtle and impossible-to-debug behavior change.
    #[test]
    fn stream_options_max_tokens_zero_is_preserved_robust() {
        let opts = StreamOptions::new("m").max_tokens(0);
        assert_eq!(opts.max_tokens, 0);
    }

    // ─── ProviderRole / ProviderContent equality ───────────────────────────

    // Normal: ProviderRole values implement Copy + Eq. The stream pipeline
    // relies on these so an `if role == ProviderRole::User` branch compiles
    // without clones.
    #[test]
    fn provider_role_copy_and_eq_normal() {
        let user = ProviderRole::User;
        let copy = user;
        assert_eq!(user, copy);
        assert_ne!(ProviderRole::User, ProviderRole::Assistant);
    }

    // ─── StreamConvention ──────────────────────────────────────────────────

    // Normal: every documented convention compares equal to itself and
    // unequal to its peers — the renderer dispatches on these so a typo
    // would silently route the wrong code path.
    #[test]
    fn stream_convention_distinct_variants_normal() {
        assert_ne!(
            StreamConvention::AnthropicNative,
            StreamConvention::OpenAiNative
        );
        assert_ne!(
            StreamConvention::OpenAiNative,
            StreamConvention::InlineXmlTags
        );
        assert_eq!(
            StreamConvention::AnthropicNative,
            StreamConvention::AnthropicNative
        );
    }

    // ─── Default Provider::complete returns NotSupported error ─────────────

    // Robust: the default complete() impl bails with a clear "not supported"
    // message that names the provider. Used by compaction to gracefully fall
    // back when a provider hasn't opted in to non-streaming completion.
    // Implements via a hand-rolled stub provider so we can exercise the
    // default-method body without needing a real network connection.
    #[tokio::test]
    async fn default_complete_returns_not_supported_robust() {
        struct StubProvider;
        impl seal::Sealed for StubProvider {}

        #[async_trait]
        impl Provider for StubProvider {
            fn name(&self) -> &str {
                "stub"
            }
            fn available_models(&self) -> Vec<ModelInfo> {
                Vec::new()
            }
            async fn stream(
                &self,
                _: Vec<ProviderMessage>,
                _: &StreamOptions,
            ) -> anyhow::Result<EventStream> {
                anyhow::bail!("stream not implemented");
            }
            // NOTE: complete() intentionally NOT overridden — we want the trait default.
        }

        let p = StubProvider;
        let result = p.complete(vec![], &StreamOptions::new("m")).await;
        let err = result.expect_err("default complete must error");
        let msg = err.to_string();
        assert!(
            msg.contains("stub") && msg.contains("not support"),
            "expected default error to mention provider name + 'not support', got: {msg}"
        );
    }

    // Normal: the default Provider::stream_convention is AnthropicNative —
    // a provider that doesn't override gets the safe Anthropic structured
    // tool-call interpretation by default.
    #[tokio::test]
    async fn default_stream_convention_is_anthropic_native_normal() {
        struct MinimalProvider;
        impl seal::Sealed for MinimalProvider {}

        #[async_trait]
        impl Provider for MinimalProvider {
            fn name(&self) -> &str {
                "min"
            }
            fn available_models(&self) -> Vec<ModelInfo> {
                Vec::new()
            }
            async fn stream(
                &self,
                _: Vec<ProviderMessage>,
                _: &StreamOptions,
            ) -> anyhow::Result<EventStream> {
                anyhow::bail!("not implemented");
            }
        }

        let p = MinimalProvider;
        assert_eq!(p.stream_convention(), StreamConvention::AnthropicNative);
        // Default fetch_models returns the static list.
        let models = p.fetch_models().await.unwrap();
        assert!(models.is_empty());
    }

    // ─── ModelInfo builder ─────────────────────────────────────────────────

    // Normal: ModelInfo::new sets the three required fields and leaves all
    // optionals as None — verified independently because the picker reads
    // these for the cost / context-window column.
    #[test]
    fn model_info_new_defaults_normal() {
        let info = ModelInfo::new("claude-opus-4-7", "Opus 4.7", "anthropic");
        assert_eq!(info.id.as_str(), "claude-opus-4-7");
        assert_eq!(info.display_name, "Opus 4.7");
        assert_eq!(info.provider.as_str(), "anthropic");
        assert!(info.context_window_tokens.is_none());
        assert!(info.max_output_tokens.is_none());
        assert!(info.input_cost.is_none());
        assert!(info.output_cost.is_none());
    }

    // Normal: every with_* builder method threads the optional through.
    #[test]
    fn model_info_builder_chains_normal() {
        let info = ModelInfo::new("m", "M", "p")
            .with_context_window_tokens(200_000usize)
            .with_max_output_tokens(128_000usize)
            .with_costs(Some(15.0), Some(75.0));
        assert_eq!(info.context_window_tokens, Some(200_000));
        assert_eq!(info.max_output_tokens, Some(128_000));
        assert_eq!(info.input_cost, Some(15.0));
        assert_eq!(info.output_cost, Some(75.0));
    }

    // ─── TokenUsage ────────────────────────────────────────────────────────

    // Normal: TokenUsage::total_input sums all three input components — used
    // by the cost panel to surface the cache-discounted true input total.
    #[test]
    fn token_usage_total_input_sums_components_normal() {
        let u = TokenUsage {
            input_tokens: 100,
            output_tokens: 50,
            cache_read_tokens: 200,
            cache_creation_tokens: 30,
        };
        assert_eq!(u.total_input(), 330);
    }

    // Robust: a default TokenUsage has all-zero counts so calling total_input
    // on a fresh struct doesn't double-count any synthetic baseline.
    #[test]
    fn token_usage_default_is_all_zero_robust() {
        let u = TokenUsage::default();
        assert_eq!(u.total_input(), 0);
        assert_eq!(u.input_tokens, 0);
        assert_eq!(u.output_tokens, 0);
    }

    // ─── billable_tokens / UsageReport ──────────────────────────────────────

    // Normal: when the provider reported tokens, billable_tokens returns their
    // sum and marks the count authoritative — the fallback char count is ignored.
    #[test]
    fn billable_tokens_prefers_provider_count_normal() {
        let u = TokenUsage {
            input_tokens: 100,
            output_tokens: 50,
            cache_read_tokens: 0,
            cache_creation_tokens: 0,
        };
        let (tokens, source) = u.billable_tokens(40_000);
        assert_eq!(tokens, 150);
        assert_eq!(source, TokenSource::Provider);
    }

    // Robust: with no reported usage, billable_tokens estimates at floor(chars/4)
    // and flags the count as an estimate. Floor (not ceil) is the pinned ratio
    // shared by every budget gate.
    #[test]
    fn billable_tokens_estimates_floor_chars_over_four_robust() {
        let u = TokenUsage::default();
        let (tokens, source) = u.billable_tokens(403);
        assert_eq!(tokens, 100, "403 / 4 floors to 100");
        assert_eq!(source, TokenSource::EstimatedFromChars);
    }

    // Robust: zero fallback chars and zero usage yields zero billable tokens
    // (no synthetic baseline), still flagged as an estimate.
    #[test]
    fn billable_tokens_zero_everything_is_zero_estimate_robust() {
        let (tokens, source) = TokenUsage::default().billable_tokens(0);
        assert_eq!(tokens, 0);
        assert_eq!(source, TokenSource::EstimatedFromChars);
    }

    // ─── FrameCategory taxonomy ─────────────────────────────────────────────

    // Normal: every StreamEvent variant maps to the expected canonical category.
    #[test]
    fn frame_category_maps_each_variant_normal() {
        use FrameCategory::*;
        let cases: Vec<(StreamEvent, FrameCategory)> = vec![
            (
                StreamEvent::TextDelta {
                    index: 0,
                    delta: "x".into(),
                },
                Text,
            ),
            (
                StreamEvent::TextDone {
                    index: 0,
                    text: "x".into(),
                },
                Text,
            ),
            (
                StreamEvent::ThinkingDelta {
                    index: 0,
                    delta: "x".into(),
                    estimated_tokens: None,
                },
                Reasoning,
            ),
            (
                StreamEvent::ThinkingDone {
                    index: 0,
                    text: "x".into(),
                },
                Reasoning,
            ),
            (
                StreamEvent::RedactedThinkingDone {
                    index: 0,
                    data: "x".into(),
                },
                Reasoning,
            ),
            (
                StreamEvent::ToolDelta {
                    index: 0,
                    delta: "x".into(),
                },
                ToolCall,
            ),
            (
                StreamEvent::ToolDone {
                    index: 0,
                    tool_name: "t".into(),
                    tool_use_id: "id".into(),
                    input_json: "{}".into(),
                    thought_signature: None,
                },
                ToolCall,
            ),
            (
                StreamEvent::ServerToolResult {
                    tool_use_id: "id".into(),
                    tool_kind: ServerToolResultKind::WebSearch,
                    content: serde_json::Value::Null,
                },
                ToolResult,
            ),
            (
                StreamEvent::Usage {
                    input_tokens: 1,
                    output_tokens: 1,
                    cache_read_tokens: 0,
                    cache_write_tokens: 0,
                },
                Usage,
            ),
            (
                StreamEvent::ResponseMetadata {
                    response_id: "r".into(),
                    input_tokens: None,
                },
                Usage,
            ),
            (
                StreamEvent::Done {
                    stop_reason: StopReason::EndTurn,
                },
                Finish,
            ),
            (StreamEvent::Keepalive, Control),
            (
                StreamEvent::Error {
                    message: "e".into(),
                },
                Control,
            ),
            (
                StreamEvent::FallbackTriggered(FallbackTriggered {
                    original_model: ModelId::new("a"),
                    fallback_model: ModelId::new("b"),
                    reason: FallbackReason::Overloaded,
                }),
                ModelResolution,
            ),
        ];
        for (event, expected) in cases {
            assert_eq!(event.category(), expected, "miscategorized: {event:?}");
        }
    }

    // Normal: commits_output is true exactly for content-bearing categories
    // (text, reasoning, tool call, tool result) and false otherwise — the same
    // predicate the runtime's committed_output flag encodes per-arm.
    #[test]
    fn frame_commits_output_only_for_content_normal() {
        assert!(
            StreamEvent::TextDelta {
                index: 0,
                delta: "x".into()
            }
            .commits_output()
        );
        assert!(
            StreamEvent::ToolDelta {
                index: 0,
                delta: "x".into()
            }
            .commits_output()
        );
        assert!(
            StreamEvent::ServerToolResult {
                tool_use_id: "id".into(),
                tool_kind: ServerToolResultKind::WebSearch,
                content: serde_json::Value::Null,
            }
            .commits_output()
        );
        assert!(
            !StreamEvent::Usage {
                input_tokens: 1,
                output_tokens: 1,
                cache_read_tokens: 0,
                cache_write_tokens: 0,
            }
            .commits_output()
        );
        assert!(!StreamEvent::Keepalive.commits_output());
        assert!(
            !StreamEvent::Done {
                stop_reason: StopReason::EndTurn
            }
            .commits_output()
        );
    }

    // Normal: a UsageReport pairs the resolved model with the call usage and
    // derives the billable count once, exposing the effective model id.
    #[test]
    fn usage_report_pairs_resolved_model_and_usage_normal() {
        let info = ModelInfo::new("claude-sonnet-4-6", "Sonnet", "anthropic");
        let resolved = ResolvedModel::new(
            ModelSpec::bare("sonnet"),
            ModelSpec::qualified("anthropic", "claude-sonnet-4-6"),
            ModelResolutionReason::ExplicitProvider,
            Some(&info),
        );
        let usage = TokenUsage {
            input_tokens: 10,
            output_tokens: 5,
            cache_read_tokens: 0,
            cache_creation_tokens: 0,
        };
        let report = UsageReport::new(resolved, usage, 999);
        assert_eq!(report.billable_tokens, 15);
        assert_eq!(report.token_source, TokenSource::Provider);
        assert_eq!(report.effective_model().as_str(), "claude-sonnet-4-6");
    }
}

pub mod content;
pub mod cost;
