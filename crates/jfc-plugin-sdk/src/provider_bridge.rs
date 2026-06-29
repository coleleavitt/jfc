use std::collections::HashMap;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BridgeProviderRole {
    User,
    Assistant,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct BridgeProviderMessage {
    pub role: BridgeProviderRole,
    pub content: Vec<BridgeProviderContent>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum BridgeProviderContent {
    Text {
        text: String,
    },
    Thinking {
        text: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        signature: Option<String>,
    },
    ToolResult {
        tool_use_id: String,
        content: String,
        #[serde(default)]
        is_error: bool,
    },
    ToolUse {
        id: String,
        name: String,
        #[serde(default)]
        input: serde_json::Value,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        thought_signature: Option<String>,
    },
    ServerToolUse {
        id: String,
        name: String,
        #[serde(default)]
        input: serde_json::Value,
    },
    ServerToolResult {
        tool_use_id: String,
        tool_kind: String,
        content: serde_json::Value,
    },
    Attachment {
        id: u32,
        mime_type: String,
        data_base64: String,
    },
    RedactedThinking {
        data: String,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct BridgeProviderToolDef {
    pub name: String,
    pub description: String,
    pub input_schema: serde_json::Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct BridgeProviderStreamOptions {
    pub model: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub system: Option<String>,
    pub max_tokens: u32,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tools: Vec<BridgeProviderToolDef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thinking_budget: Option<u32>,
    #[serde(default)]
    pub adaptive_thinking: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thinking_display: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub top_p: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning_effort: Option<String>,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub provider_options: HashMap<String, serde_json::Value>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub custom_betas: Vec<String>,
    #[serde(default)]
    pub fast_mode: bool,
    #[serde(default)]
    pub eager_input_streaming: bool,
    #[serde(default)]
    pub strict_tool_schemas: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_budget_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub previous_message_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_hint_tokens_saved: Option<u64>,
    #[serde(default)]
    pub thinking_token_count: bool,
    #[serde(default)]
    pub mid_conversation_system: bool,
    #[serde(default)]
    pub cache_diagnosis: bool,
    #[serde(default)]
    pub prompt_caching_scope: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub advisor_model: Option<String>,
    #[serde(default)]
    pub narration_summaries: bool,
}

impl BridgeProviderStreamOptions {
    pub fn new(model: impl Into<String>) -> Self {
        let _linkscope_options = linkscope::phase("plugin_sdk.provider_bridge.stream_options.new");
        let model = model.into();
        linkscope::event_fields(
            "plugin_sdk.provider_bridge.stream_options.new",
            [linkscope::TraceField::text("model", model.clone())],
        );
        Self {
            model,
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

    pub const fn max_tokens(mut self, max_tokens: u32) -> Self {
        self.max_tokens = max_tokens;
        self
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum BridgeProviderStreamEvent {
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
        #[serde(default, skip_serializing_if = "Option::is_none")]
        estimated_tokens: Option<u32>,
    },
    ThinkingTokens {
        index: usize,
        delta: u32,
    },
    ThinkingDone {
        index: usize,
        text: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        signature: Option<String>,
    },
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
        #[serde(default, skip_serializing_if = "Option::is_none")]
        thought_signature: Option<String>,
    },
    ServerToolResult {
        tool_use_id: String,
        tool_kind: String,
        content: serde_json::Value,
    },
    Done {
        stop_reason: BridgeStopReason,
    },
    Usage {
        input_tokens: u32,
        output_tokens: u32,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        thinking_tokens: Option<u32>,
        #[serde(default)]
        cache_read_tokens: u32,
        #[serde(default)]
        cache_write_tokens: u32,
    },
    ResponseMetadata {
        response_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        input_tokens: Option<u64>,
    },
    Error {
        message: String,
    },
    Keepalive,
    FallbackTriggered {
        original_model: String,
        fallback_model: String,
        reason: BridgeFallbackReason,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
pub enum BridgeStopReason {
    EndTurn,
    ToolUse,
    PauseTurn,
    Refusal,
    MaxTokens,
    StopSequence,
    Other(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
pub enum BridgeFallbackReason {
    ModelNotFound,
    Overloaded,
    ModelRefusal,
    PermissionDenied,
    ServerError,
    Other(String),
}
