use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::types::{
    ChatMessage, MessagePart, Role, ToolCall, ToolInput, ToolKind, ToolOutput, ToolStatus,
};

#[derive(Serialize, Deserialize)]
struct SerializedSession {
    id: String,
    created_at: String,
    messages: Vec<SerializedMessage>,
}

#[derive(Serialize, Deserialize)]
struct SerializedMessage {
    role: String,
    parts: Vec<SerializedPart>,
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum SerializedPart {
    Text {
        content: String,
    },
    Reasoning {
        content: String,
    },
    Tool {
        id: String,
        kind: String,
        status: String,
        input_summary: String,
        output: Option<String>,
    },
}

pub fn sessions_dir() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("jfc")
        .join("sessions")
}

pub fn generate_session_id() -> String {
    let now = chrono::Utc::now();
    format!("ses_{}", now.format("%Y%m%d_%H%M%S"))
}

#[tracing::instrument(target = "jfc::session", skip(messages), fields(n = messages.len()))]
pub fn save_session(session_id: &str, messages: &[ChatMessage]) {
    let dir = sessions_dir();
    if std::fs::create_dir_all(&dir).is_err() {
        return;
    }

    let now = chrono::Utc::now();
    let serialized = SerializedSession {
        id: session_id.to_owned(),
        created_at: now.to_rfc3339(),
        messages: messages.iter().map(serialize_message).collect(),
    };

    let path = dir.join(format!("{session_id}.json"));
    if let Ok(json) = serde_json::to_string_pretty(&serialized) {
        let _ = std::fs::write(&path, json);
    }
}

pub fn load_session(session_id: &str) -> Option<Vec<ChatMessage>> {
    let path = sessions_dir().join(format!("{session_id}.json"));
    let content = std::fs::read_to_string(&path).ok()?;
    let session: SerializedSession = serde_json::from_str(&content).ok()?;
    Some(
        session
            .messages
            .into_iter()
            .map(deserialize_message)
            .collect(),
    )
}

pub fn list_sessions() -> Vec<String> {
    let dir = sessions_dir();
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return vec![];
    };
    let mut ids: Vec<String> = entries
        .filter_map(|e| e.ok())
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().to_string();
            name.strip_suffix(".json").map(str::to_owned)
        })
        .collect();
    ids.sort_by(|a, b| b.cmp(a));
    ids
}

fn serialize_message(msg: &ChatMessage) -> SerializedMessage {
    SerializedMessage {
        role: match msg.role {
            Role::User => "user".into(),
            Role::Assistant => "assistant".into(),
        },
        parts: msg.parts.iter().map(serialize_part).collect(),
    }
}

fn serialize_part(part: &MessagePart) -> SerializedPart {
    match part {
        MessagePart::Text(t) => SerializedPart::Text { content: t.clone() },
        MessagePart::Reasoning(t) => SerializedPart::Reasoning { content: t.clone() },
        MessagePart::Tool(tc) => SerializedPart::Tool {
            id: tc.id.clone(),
            kind: tc.kind.label().to_owned(),
            status: format!("{:?}", tc.status),
            input_summary: tc.input.summary(),
            output: match &tc.output {
                ToolOutput::Text(t) => Some(t.clone()),
                _ => None,
            },
        },
        MessagePart::CompactBoundary { pre_tokens } => SerializedPart::Text {
            content: format!("[compact_boundary: pre={pre_tokens}]"),
        },
        MessagePart::TaskStatus(ts) => SerializedPart::Text {
            content: format!(
                "[task {} | {} | {}]",
                ts.task_id,
                ts.status.label(),
                ts.summary.as_deref().unwrap_or(&ts.description)
            ),
        },
    }
}

fn deserialize_message(msg: SerializedMessage) -> ChatMessage {
    let role = if msg.role == "user" {
        Role::User
    } else {
        Role::Assistant
    };
    let parts: Vec<MessagePart> = msg.parts.into_iter().map(deserialize_part).collect();
    ChatMessage {
        role,
        parts,
        agent_name: None,
        model_name: None,
        cost_tier: None,
        elapsed: None,
    }
}

fn deserialize_part(part: SerializedPart) -> MessagePart {
    match part {
        SerializedPart::Text { content } => MessagePart::Text(content),
        SerializedPart::Reasoning { content } => MessagePart::Reasoning(content),
        SerializedPart::Tool {
            id,
            kind,
            status,
            input_summary,
            output,
        } => MessagePart::Tool(ToolCall {
            id,
            kind: ToolKind::from_name(&kind),
            status: match status.as_str() {
                "Complete" => ToolStatus::Complete,
                "Failed" => ToolStatus::Failed,
                "Running" => ToolStatus::Running,
                _ => ToolStatus::Complete,
            },
            input: ToolInput::Generic {
                summary: input_summary,
            },
            output: match output {
                Some(t) => ToolOutput::Text(t),
                None => ToolOutput::Empty,
            },
            is_collapsed: true,
        }),
    }
}
