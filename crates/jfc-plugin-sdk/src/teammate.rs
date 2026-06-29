use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct BridgeMailboxMessage {
    pub from: String,
    pub text: String,
    pub timestamp: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    #[serde(default)]
    pub read: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct BridgeMailboxPollRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub team_name: Option<String>,
    #[serde(default)]
    pub unread_only: bool,
    #[serde(default)]
    pub mark_read: bool,
}

impl BridgeMailboxPollRequest {
    pub fn unread() -> Self {
        let _linkscope_poll = linkscope::phase("plugin_sdk.teammate.mailbox_poll.unread");
        linkscope::detail_event_fields(
            "plugin_sdk.teammate.mailbox_poll.unread",
            [linkscope::TraceField::count("unread_only", 1)],
        );
        Self {
            agent_name: None,
            team_name: None,
            unread_only: true,
            mark_read: false,
        }
    }

    pub fn with_agent_name(mut self, agent_name: impl Into<String>) -> Self {
        self.agent_name = Some(agent_name.into());
        self
    }

    pub fn with_team_name(mut self, team_name: impl Into<String>) -> Self {
        self.team_name = Some(team_name.into());
        self
    }

    pub const fn mark_read(mut self, mark_read: bool) -> Self {
        self.mark_read = mark_read;
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct BridgeMailboxSendRequest {
    pub to: String,
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub team_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
}

impl BridgeMailboxSendRequest {
    pub fn new(to: impl Into<String>, text: impl Into<String>) -> Self {
        let _linkscope_send = linkscope::phase("plugin_sdk.teammate.mailbox_send.new");
        let to = to.into();
        let text = text.into();
        linkscope::event_fields(
            "plugin_sdk.teammate.mailbox_send.new",
            [
                linkscope::TraceField::text("to", to.clone()),
                linkscope::TraceField::bytes(
                    "text_bytes",
                    u64::try_from(text.len()).unwrap_or(u64::MAX),
                ),
            ],
        );
        Self {
            to,
            text,
            from: None,
            team_name: None,
            color: None,
            summary: None,
        }
    }

    pub fn with_from(mut self, from: impl Into<String>) -> Self {
        self.from = Some(from.into());
        self
    }

    pub fn with_team_name(mut self, team_name: impl Into<String>) -> Self {
        self.team_name = Some(team_name.into());
        self
    }

    pub fn with_summary(mut self, summary: impl Into<String>) -> Self {
        self.summary = Some(summary.into());
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct BridgeTeammateReady {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub team_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
}

impl BridgeTeammateReady {
    pub fn new() -> Self {
        let _linkscope_ready = linkscope::phase("plugin_sdk.teammate.ready.new");
        linkscope::detail_event_fields(
            "plugin_sdk.teammate.ready.new",
            [linkscope::TraceField::text("status", "empty")],
        );
        Self {
            agent_name: None,
            team_name: None,
            reason: None,
            summary: None,
        }
    }

    pub fn with_reason(mut self, reason: impl Into<String>) -> Self {
        self.reason = Some(reason.into());
        self
    }

    pub fn with_summary(mut self, summary: impl Into<String>) -> Self {
        self.summary = Some(summary.into());
        self
    }
}

impl Default for BridgeTeammateReady {
    fn default() -> Self {
        Self::new()
    }
}
