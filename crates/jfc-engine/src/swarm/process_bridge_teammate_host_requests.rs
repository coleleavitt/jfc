use jfc_plugin_sdk::{
    BridgeErrorDto, BridgeMailboxMessage, BridgeMailboxPollRequest, BridgeMailboxSendRequest,
    BridgeRequest, BridgeResponse, BridgeTeammateReady,
};

use super::process_bridge_teammate::ProcessBridgeTeammateConfig;
use super::runner::TeammateEvent;
use super::{mailbox, types::MailboxMessage};

pub(crate) struct TeammateHostRequestOutcome {
    pub(crate) response: BridgeResponse,
    pub(crate) event: Option<TeammateEvent>,
}

pub(crate) async fn handle_teammate_host_request(
    config: &ProcessBridgeTeammateConfig,
    request: BridgeRequest,
) -> TeammateHostRequestOutcome {
    match request {
        BridgeRequest::TeammateMailboxPoll { request } => poll_mailbox(config, request).await,
        BridgeRequest::TeammateMailboxSend { request } => send_mailbox(config, request).await,
        BridgeRequest::TeammateReady { ready } => acknowledge_ready(config, ready).await,
        other => error_outcome(
            "unsupported_teammate_request",
            format!("unsupported teammate host request: {other:?}"),
        ),
    }
}

async fn poll_mailbox(
    config: &ProcessBridgeTeammateConfig,
    request: BridgeMailboxPollRequest,
) -> TeammateHostRequestOutcome {
    let Some(agent_name) = request.agent_name.or_else(|| default_agent_name(config)) else {
        return error_outcome(
            "missing_agent_name",
            "teammate mailbox poll needs an agent name",
        );
    };
    let Some(team_name) = request.team_name.or_else(|| default_team_name(config)) else {
        return error_outcome(
            "missing_team_name",
            "teammate mailbox poll needs a team name",
        );
    };
    let messages = if request.unread_only {
        mailbox::read_unread_messages(&agent_name, &team_name).await
    } else {
        mailbox::read_mailbox(&agent_name, &team_name).await
    };
    if request.mark_read
        && let Err(error) = mailbox::mark_all_read(&agent_name, &team_name).await
    {
        return error_outcome("mailbox_mark_read_failed", error.to_string());
    }
    TeammateHostRequestOutcome {
        response: BridgeResponse::TeammateMailboxMessages {
            messages: messages
                .into_iter()
                .map(BridgeMailboxMessage::from)
                .collect(),
        },
        event: None,
    }
}

async fn send_mailbox(
    config: &ProcessBridgeTeammateConfig,
    request: BridgeMailboxSendRequest,
) -> TeammateHostRequestOutcome {
    let Some(from) = request.from.or_else(|| default_agent_name(config)) else {
        return error_outcome("missing_from", "teammate mailbox send needs a sender");
    };
    let Some(team_name) = request.team_name.or_else(|| default_team_name(config)) else {
        return error_outcome(
            "missing_team_name",
            "teammate mailbox send needs a team name",
        );
    };
    let message = MailboxMessage {
        from,
        text: request.text,
        timestamp: chrono::Utc::now().to_rfc3339(),
        color: request.color,
        summary: request.summary,
        read: false,
    };
    match mailbox::write_to_mailbox(&request.to, message, &team_name).await {
        Ok(()) => ack_outcome("mailbox message sent"),
        Err(error) => error_outcome("mailbox_send_failed", error.to_string()),
    }
}

async fn acknowledge_ready(
    config: &ProcessBridgeTeammateConfig,
    ready: BridgeTeammateReady,
) -> TeammateHostRequestOutcome {
    let Some(agent_name) = ready.agent_name.or_else(|| default_agent_name(config)) else {
        return error_outcome("missing_agent_name", "teammate ready needs an agent name");
    };
    let Some(team_name) = ready.team_name.or_else(|| default_team_name(config)) else {
        return error_outcome("missing_team_name", "teammate ready needs a team name");
    };
    if let Err(error) = mailbox::send_idle_notification(
        &agent_name,
        None,
        &team_name,
        ready.reason.as_deref(),
        ready.summary.as_deref(),
    )
    .await
    {
        return error_outcome("teammate_ready_failed", error.to_string());
    }
    TeammateHostRequestOutcome {
        response: BridgeResponse::Ack {
            message: Some("teammate ready".to_owned()),
        },
        event: Some(TeammateEvent::Idle {
            task_id: config.task_id.clone(),
            agent_id: config.agent_id.clone(),
            agent_name,
            reason: ready.reason,
            summary: ready.summary,
        }),
    }
}

fn default_agent_name(config: &ProcessBridgeTeammateConfig) -> Option<String> {
    config
        .task_input
        .name
        .clone()
        .or_else(|| Some(config.descriptor.label.clone()))
}

fn default_team_name(config: &ProcessBridgeTeammateConfig) -> Option<String> {
    config
        .active_team_name
        .clone()
        .or_else(|| config.task_input.team_name.clone())
}

fn ack_outcome(message: impl Into<String>) -> TeammateHostRequestOutcome {
    TeammateHostRequestOutcome {
        response: BridgeResponse::Ack {
            message: Some(message.into()),
        },
        event: None,
    }
}

fn error_outcome(
    code: impl Into<String>,
    message: impl Into<String>,
) -> TeammateHostRequestOutcome {
    TeammateHostRequestOutcome {
        response: BridgeResponse::Error(BridgeErrorDto::new(code, message)),
        event: None,
    }
}

impl From<MailboxMessage> for BridgeMailboxMessage {
    fn from(message: MailboxMessage) -> Self {
        Self {
            from: message.from,
            text: message.text,
            timestamp: message.timestamp,
            color: message.color,
            summary: message.summary,
            read: message.read,
        }
    }
}
