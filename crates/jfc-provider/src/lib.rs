#![allow(dead_code)]

use std::{borrow::Borrow, collections::HashMap, fmt, ops::Deref, pin::Pin};

use async_trait::async_trait;
use futures::Stream;
use reqwest::header::HeaderMap;

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
    },
    ThinkingDone {
        index: usize,
        text: String,
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
    },
    Error {
        message: String,
    },
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
    },
    /// Image or PDF attachment carried as base64. Anthropic emits two
    /// distinct content-block shapes — `image` for PNG/JPEG/GIF/WebP
    /// and `document` for PDF — but both share the same source struct,
    /// so we keep one Rust variant and let the provider serializer
    /// decide. Non-Anthropic providers (OpenAI, OpenWebUI/LiteLLM)
    /// either reject these or use bespoke shapes; today they're a
    /// no-op for those providers.
    Attachment(jfc_core::Attachment),
}

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
    /// Optional display mode for adaptive thinking: `"summarized"` or `"omitted"`.
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
    /// When true, adds `fast-mode-2026-02-01` to the `anthropic-beta` header
    /// for lower-latency inference. Mirrors v2.1.139's `/fast` command.
    pub fast_mode: bool,
    /// Optional agentic loop token budget hint (beta: task-budgets-2026-03-13).
    /// Minimum 20_000. The model sees a countdown and self-moderates.
    /// Distinct from max_tokens (which is a hard server-enforced ceiling).
    pub task_budget_tokens: Option<u64>,
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
            fast_mode: false,
            task_budget_tokens: None,
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

    /// Set the display mode for adaptive thinking responses.
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

    /// Enable or disable fast mode (lower-latency inference via `fast-mode-2026-02-01` beta).
    pub fn fast_mode(mut self, v: bool) -> Self {
        self.fast_mode = v;
        self
    }

    /// Set the agentic loop task budget (beta: task-budgets-2026-03-13).
    /// Minimum 20_000 tokens — values below are clamped up.
    pub fn task_budget(mut self, tokens: u64) -> Self {
        self.task_budget_tokens = Some(tokens.max(20_000));
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
}

#[derive(Debug, Clone)]
pub struct ModelInfo {
    pub id: ModelId,
    pub display_name: String,
    pub provider: ProviderId,
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
        Self {
            id: id.into(),
            display_name: display_name.into(),
            provider: provider.into(),
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

    pub fn with_costs(mut self, input: Option<f64>, output: Option<f64>) -> Self {
        self.input_cost = input;
        self.output_cost = output;
        self
    }
}

pub type EventStream = Pin<Box<dyn Stream<Item = anyhow::Result<StreamEvent>> + Send>>;

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
/// Even though jfc-ui isn't a published library today, sealing protects
/// future evolution: if the crate ever splits or is re-exported, downstream
/// callers cannot lock us out of adding new required methods.
pub mod seal {
    pub trait Sealed {}
}

/// Sealed: implementations live inside the jfc-ui crate's `providers/`
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
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
