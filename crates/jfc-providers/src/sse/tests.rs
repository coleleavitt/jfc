use super::*;
use jfc_provider::{
    ModelId, ProviderContent, ProviderMessage, ProviderRole, ServerToolResultKind, StopReason,
    StreamEvent, ToolDef,
};
use serde_json::Value;

fn make_user_msg(text: &str) -> ProviderMessage {
    ProviderMessage {
        role: ProviderRole::User,
        content: vec![ProviderContent::Text(text.to_owned())],
    }
}

fn make_assistant_msg(text: &str) -> ProviderMessage {
    ProviderMessage {
        role: ProviderRole::Assistant,
        content: vec![ProviderContent::Text(text.to_owned())],
    }
}

fn empty_state() -> (Vec<Option<BlockState>>, Option<StopReason>) {
    (Vec::new(), None)
}

mod blocks;
mod events;
mod finalize;
mod request;
mod server_tools;
mod stop_reason;
