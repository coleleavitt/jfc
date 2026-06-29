use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum MessageContentPart {
    Text { content: String },
    Thinking { content: String },
    ThinkingSignature { signature: String },
    RedactedThinking { data: String },
}

impl MessageContentPart {
    pub fn text(content: impl Into<String>) -> Self {
        Self::Text {
            content: content.into(),
        }
    }

    pub fn thinking(content: impl Into<String>) -> Self {
        Self::Thinking {
            content: content.into(),
        }
    }

    pub fn thinking_signature(signature: impl Into<String>) -> Self {
        Self::ThinkingSignature {
            signature: signature.into(),
        }
    }

    pub fn redacted_thinking(data: impl Into<String>) -> Self {
        Self::RedactedThinking { data: data.into() }
    }
}
