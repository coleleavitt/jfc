use std::pin::Pin;

use async_trait::async_trait;
use futures::Stream;

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
    },
    Error {
        message: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StopReason {
    EndTurn,
    ToolUse,
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
}

#[derive(Debug, Clone)]
pub struct ToolDef {
    pub name: String,
    pub description: String,
    pub input_schema: serde_json::Value,
}

#[derive(Debug, Clone)]
pub struct StreamOptions {
    pub model: String,
    pub system: Option<String>,
    pub max_tokens: u32,
    pub tools: Vec<ToolDef>,
    pub thinking_budget: Option<u32>,
}

impl StreamOptions {
    pub fn new(model: impl Into<String>) -> Self {
        Self {
            model: model.into(),
            system: None,
            max_tokens: 8192,
            tools: Vec::new(),
            thinking_budget: None,
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

    pub fn tools(mut self, tools: Vec<ToolDef>) -> Self {
        self.tools = tools;
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
    pub id: String,
    pub display_name: String,
    pub provider: String,
}

impl ModelInfo {
    pub fn new(
        id: impl Into<String>,
        display_name: impl Into<String>,
        provider: impl Into<String>,
    ) -> Self {
        Self {
            id: id.into(),
            display_name: display_name.into(),
            provider: provider.into(),
        }
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

#[async_trait]
pub trait Provider: Send + Sync {
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
