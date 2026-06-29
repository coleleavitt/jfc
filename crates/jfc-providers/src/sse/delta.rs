use serde::{Deserialize, de::Error as DeError};
use serde_json::Value;

#[derive(Debug)]
#[allow(clippy::enum_variant_names)]
pub enum Delta {
    TextDelta {
        text: String,
    },
    ThinkingDelta {
        thinking: String,
        estimated_tokens: Option<u32>,
    },
    InputJsonDelta {
        partial_json: String,
    },
    SignatureDelta {
        signature: String,
    },
    CitationsDelta {},
    ConnectorTextDelta {
        connector_text: String,
    },
    CompactionContentBlockDelta {
        content: String,
    },
    Unknown {
        kind: String,
        raw: Value,
    },
}

impl<'de> Deserialize<'de> for Delta {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = Value::deserialize(deserializer)?;
        let kind = value
            .get("type")
            .and_then(Value::as_str)
            .ok_or_else(|| D::Error::missing_field("type"))?
            .to_owned();

        let field = |name: &str| -> String {
            value
                .get(name)
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned()
        };

        Ok(match kind.as_str() {
            "text_delta" => Self::TextDelta {
                text: field("text"),
            },
            "thinking_delta" => Self::ThinkingDelta {
                thinking: field("thinking"),
                estimated_tokens: value
                    .get("estimated_tokens")
                    .and_then(Value::as_u64)
                    .map(|u| u32::try_from(u).unwrap_or(u32::MAX)),
            },
            "input_json_delta" => Self::InputJsonDelta {
                partial_json: field("partial_json"),
            },
            "signature_delta" => Self::SignatureDelta {
                signature: field("signature"),
            },
            "citations_delta" => Self::CitationsDelta {},
            "connector_text_delta" => Self::ConnectorTextDelta {
                connector_text: field("connector_text"),
            },
            "compaction_content_block_delta" => Self::CompactionContentBlockDelta {
                content: field("content"),
            },
            _ => Self::Unknown { kind, raw: value },
        })
    }
}
