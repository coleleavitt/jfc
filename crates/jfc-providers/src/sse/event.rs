use serde::{Deserialize, de::Error as DeError};
use serde_json::Value;

use super::{
    ContentBlock, ContextManagement, Delta, ErrorBody, MessageDeltaData, MessageStart, MessageUsage,
};

#[derive(Debug)]
pub enum SseEvent {
    MessageStart {
        message: MessageStart,
    },
    ContentBlockStart {
        index: usize,
        content_block: ContentBlock,
    },
    ContentBlockDelta {
        index: usize,
        delta: Delta,
    },
    ContentBlockStop {
        index: usize,
    },
    MessageDelta {
        delta: MessageDeltaData,
        usage: Option<MessageUsage>,
        /// Present when Anthropic server-side context management is active.
        context_management: Option<ContextManagement>,
    },
    MessageStop,
    Ping,
    Error {
        error: ErrorBody,
    },
    Unknown {
        kind: String,
        raw: Value,
    },
}

impl<'de> Deserialize<'de> for SseEvent {
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

        macro_rules! frame {
            ($ty:ty) => {
                serde_json::from_value::<$ty>(value).map_err(D::Error::custom)?
            };
        }

        match kind.as_str() {
            "message_start" => {
                #[derive(Deserialize)]
                struct Frame {
                    message: MessageStart,
                }
                let frame = frame!(Frame);
                Ok(Self::MessageStart {
                    message: frame.message,
                })
            }
            "content_block_start" => {
                #[derive(Deserialize)]
                struct Frame {
                    index: usize,
                    content_block: ContentBlock,
                }
                let frame = frame!(Frame);
                Ok(Self::ContentBlockStart {
                    index: frame.index,
                    content_block: frame.content_block,
                })
            }
            "content_block_delta" => {
                #[derive(Deserialize)]
                struct Frame {
                    index: usize,
                    delta: Delta,
                }
                let frame = frame!(Frame);
                Ok(Self::ContentBlockDelta {
                    index: frame.index,
                    delta: frame.delta,
                })
            }
            "content_block_stop" => {
                #[derive(Deserialize)]
                struct Frame {
                    index: usize,
                }
                let frame = frame!(Frame);
                Ok(Self::ContentBlockStop { index: frame.index })
            }
            "message_delta" => {
                #[derive(Deserialize)]
                struct Frame {
                    delta: MessageDeltaData,
                    #[serde(default)]
                    usage: Option<MessageUsage>,
                    #[serde(default)]
                    context_management: Option<ContextManagement>,
                }
                let frame = frame!(Frame);
                Ok(Self::MessageDelta {
                    delta: frame.delta,
                    usage: frame.usage,
                    context_management: frame.context_management,
                })
            }
            "message_stop" => Ok(Self::MessageStop),
            "ping" => Ok(Self::Ping),
            "error" => {
                #[derive(Deserialize)]
                struct Frame {
                    error: ErrorBody,
                }
                let frame = frame!(Frame);
                Ok(Self::Error { error: frame.error })
            }
            _ => Ok(Self::Unknown { kind, raw: value }),
        }
    }
}
