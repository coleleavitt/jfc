use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct MessageStart {
    pub id: String,
    #[serde(default)]
    pub usage: Option<MessageUsage>,
}

#[derive(Debug, Deserialize)]
pub struct MessageUsage {
    #[serde(default)]
    pub input_tokens: Option<u32>,
    #[serde(default)]
    pub output_tokens: Option<u32>,
    #[serde(default)]
    pub output_tokens_details: Option<OutputTokensDetails>,
    #[serde(default)]
    pub cache_creation_input_tokens: Option<u32>,
    #[serde(default)]
    pub cache_read_input_tokens: Option<u32>,
}

#[derive(Debug, Deserialize)]
pub struct OutputTokensDetails {
    #[serde(default)]
    pub thinking_tokens: Option<u32>,
}

impl MessageUsage {
    pub(crate) fn input_tokens(&self) -> u32 {
        self.input_tokens.unwrap_or_default()
    }

    pub(crate) fn output_total(&self) -> u32 {
        self.output_tokens.unwrap_or_default()
    }

    pub(crate) fn thinking_tokens(&self) -> Option<u32> {
        self.output_tokens_details
            .as_ref()
            .and_then(|details| details.thinking_tokens)
    }
}

#[derive(Debug, Deserialize)]
pub struct MessageDeltaData {
    pub stop_reason: Option<String>,
}

/// Optional server-side context management metadata that Anthropic may attach
/// to a `message_delta` event when it is managing the context window on behalf
/// of the caller. The shape is deliberately left open (`Value`) so that new
/// fields (e.g. `compacted`, `removed_tokens`) don't cause parse failures.
#[derive(Debug, Deserialize)]
pub struct ContextManagement {
    /// True when Anthropic has already compacted earlier turns on the server.
    #[serde(default)]
    pub compacted: bool,
    /// Number of tokens removed by server-side compaction, if reported.
    #[serde(default)]
    pub removed_tokens: Option<u32>,
}

#[derive(Debug, Deserialize)]
pub struct ErrorBody {
    #[serde(default, rename = "type")]
    pub kind: Option<String>,
    pub message: String,
}
