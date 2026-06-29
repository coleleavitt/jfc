use jfc_engine::swarm::{mailbox, types::MailboxMessage};
use jfc_plugin_sdk::RuntimeActionDescriptor;
use serde::Deserialize;

pub(super) async fn execute_teammate_message_action(action: &RuntimeActionDescriptor) {
    let Some(payload) = action.payload.clone() else {
        warn_missing_payload(action, "teammate message");
        return;
    };
    let parsed = match serde_json::from_value::<TeammateMessagePayload>(payload) {
        Ok(parsed) => parsed,
        Err(error) => {
            tracing::warn!(
                target: "jfc::palette",
                plugin = action.plugin_id.as_str(),
                action = action.id.as_str(),
                error = %error,
                "invalid teammate-message runtime-action payload"
            );
            return;
        }
    };
    if parsed.to.trim().is_empty()
        || parsed.team_name.trim().is_empty()
        || parsed.text.trim().is_empty()
    {
        tracing::warn!(
            target: "jfc::palette",
            plugin = action.plugin_id.as_str(),
            action = action.id.as_str(),
            "teammate-message runtime action needs non-empty to, team_name, and text"
        );
        return;
    }
    let message = MailboxMessage {
        from: parsed
            .from
            .unwrap_or_else(|| "jfc-runtime-action".to_owned()),
        text: parsed.text,
        timestamp: chrono::Utc::now().to_rfc3339(),
        color: parsed.color,
        summary: parsed.summary,
        read: false,
    };
    if let Err(error) = mailbox::write_to_mailbox(&parsed.to, message, &parsed.team_name).await {
        tracing::warn!(
            target: "jfc::palette",
            plugin = action.plugin_id.as_str(),
            action = action.id.as_str(),
            error = %error,
            "failed to execute teammate-message runtime action"
        );
    }
}

fn warn_missing_payload(action: &RuntimeActionDescriptor, key: &str) {
    tracing::warn!(
        target: "jfc::palette",
        plugin = action.plugin_id.as_str(),
        action = action.id.as_str(),
        key,
        "runtime action is missing required payload field"
    );
}

#[derive(Debug, Deserialize)]
struct TeammateMessagePayload {
    to: String,
    team_name: String,
    text: String,
    from: Option<String>,
    summary: Option<String>,
    color: Option<String>,
}
