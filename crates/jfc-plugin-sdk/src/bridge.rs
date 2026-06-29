use serde::{Deserialize, Serialize};

use jfc_core::{SessionId, ToolId};

use crate::{
    BridgeAgentLaunchRequest, BridgeAgentLaunchResult, BridgeMailboxMessage,
    BridgeMailboxPollRequest, BridgeMailboxSendRequest, BridgeProviderMessage,
    BridgeProviderStreamEvent, BridgeProviderStreamOptions, BridgeTeammateEvent,
    BridgeTeammateReady, BridgeUiPanelRefreshRequest, BridgeUiPanelRefreshResult,
    BridgeUiWidgetRefreshRequest, BridgeUiWidgetRefreshResult, HookName, PluginManifest,
    runtime_extension::{BridgePromptContextRefreshRequest, BridgePromptContextRefreshResult},
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ProcessBridgeCommand {
    pub command: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub args: Vec<String>,
}

impl ProcessBridgeCommand {
    pub fn new(command: impl Into<String>) -> Self {
        Self {
            command: command.into(),
            args: Vec::new(),
        }
    }

    pub fn with_args<I, S>(mut self, args: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.args = args.into_iter().map(Into::into).collect();
        self
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum BridgeEnvelope {
    Request {
        id: String,
        request: BridgeRequest,
    },
    Response {
        id: String,
        response: BridgeResponse,
    },
}

impl BridgeEnvelope {
    pub fn request(id: impl Into<String>, request: BridgeRequest) -> Self {
        Self::Request {
            id: id.into(),
            request,
        }
    }

    pub fn response(id: impl Into<String>, response: BridgeResponse) -> Self {
        Self::Response {
            id: id.into(),
            response,
        }
    }

    pub fn id(&self) -> &str {
        match self {
            Self::Request { id, request: _ } | Self::Response { id, response: _ } => id,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum BridgeRequest {
    Manifest,
    Describe,
    ToolCall {
        tool: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tool_id: Option<ToolId>,
        #[serde(default)]
        input: serde_json::Value,
    },
    ProviderStream {
        provider: String,
        #[serde(default)]
        messages: Vec<BridgeProviderMessage>,
        options: BridgeProviderStreamOptions,
    },
    AgentLaunch {
        launch: BridgeAgentLaunchRequest,
    },
    TeammateMailboxPoll {
        request: BridgeMailboxPollRequest,
    },
    TeammateMailboxSend {
        request: BridgeMailboxSendRequest,
    },
    TeammateReady {
        ready: BridgeTeammateReady,
    },
    UiWidgetRefresh {
        refresh: BridgeUiWidgetRefreshRequest,
    },
    UiPanelRefresh {
        refresh: BridgeUiPanelRefreshRequest,
    },
    PromptContextRefresh {
        refresh: BridgePromptContextRefreshRequest,
    },
    Hook {
        hook: HookName,
        session_id: SessionId,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tool_id: Option<ToolId>,
        #[serde(default)]
        payload: serde_json::Value,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum BridgeResponse {
    Manifest {
        manifest: PluginManifest,
    },
    Descriptors {
        descriptors: serde_json::Value,
    },
    ToolResult {
        output: String,
        #[serde(default)]
        is_error: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        payload: Option<serde_json::Value>,
    },
    ProviderEvent {
        event: BridgeProviderStreamEvent,
    },
    AgentLaunchResult {
        result: BridgeAgentLaunchResult,
    },
    TeammateEvent {
        event: BridgeTeammateEvent,
    },
    TeammateMailboxMessages {
        messages: Vec<BridgeMailboxMessage>,
    },
    UiWidgetRefresh {
        result: BridgeUiWidgetRefreshResult,
    },
    UiPanelRefresh {
        result: BridgeUiPanelRefreshResult,
    },
    PromptContextRefresh {
        result: BridgePromptContextRefreshResult,
    },
    Ack {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        message: Option<String>,
    },
    Hook {
        payload: serde_json::Value,
    },
    Error(BridgeErrorDto),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct BridgeErrorDto {
    pub code: String,
    pub message: String,
}

impl BridgeErrorDto {
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
        }
    }
}
