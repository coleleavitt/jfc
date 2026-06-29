use serde::{Deserialize, Serialize};

use jfc_core::{SessionId, TaskInput};

use crate::{DescriptorVisibility, PluginId};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentLaunchExecutorKind {
    BuiltIn,
    ProcessBridge,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct AgentLaunchExecutorDescriptor {
    pub kind: AgentLaunchExecutorKind,
    pub handler: String,
}

impl AgentLaunchExecutorDescriptor {
    pub fn new(kind: AgentLaunchExecutorKind, handler: impl Into<String>) -> Self {
        let _linkscope_executor = linkscope::phase("plugin_sdk.agent_launch.executor.new");
        let handler = handler.into();
        linkscope::detail_event_fields(
            "plugin_sdk.agent_launch.executor.new",
            [
                linkscope::TraceField::text("kind", format!("{kind:?}")),
                linkscope::TraceField::bytes(
                    "handler_bytes",
                    u64::try_from(handler.len()).unwrap_or(u64::MAX),
                ),
            ],
        );
        Self { kind, handler }
    }

    pub fn built_in(handler: impl Into<String>) -> Self {
        Self::new(AgentLaunchExecutorKind::BuiltIn, handler)
    }

    pub fn process_bridge(handler: impl Into<String>) -> Self {
        Self::new(AgentLaunchExecutorKind::ProcessBridge, handler)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct AgentLaunchDescriptor {
    pub plugin_id: PluginId,
    pub name: String,
    pub label: String,
    pub description: String,
    pub visibility: DescriptorVisibility,
    pub executor: AgentLaunchExecutorDescriptor,
}

impl AgentLaunchDescriptor {
    pub fn new(
        plugin_id: PluginId,
        name: impl Into<String>,
        label: impl Into<String>,
        description: impl Into<String>,
    ) -> Self {
        let _linkscope_descriptor = linkscope::phase("plugin_sdk.agent_launch.descriptor.new");
        let name = name.into();
        let label = label.into();
        let description = description.into();
        linkscope::event_fields(
            "plugin_sdk.agent_launch.descriptor.new",
            [
                linkscope::TraceField::text("plugin_id", plugin_id.as_str().to_owned()),
                linkscope::TraceField::text("name", name.clone()),
                linkscope::TraceField::bytes(
                    "label_bytes",
                    u64::try_from(label.len()).unwrap_or(u64::MAX),
                ),
                linkscope::TraceField::bytes(
                    "description_bytes",
                    u64::try_from(description.len()).unwrap_or(u64::MAX),
                ),
            ],
        );
        Self {
            plugin_id,
            name,
            label,
            description,
            visibility: DescriptorVisibility::HostVisible,
            executor: AgentLaunchExecutorDescriptor::built_in(""),
        }
    }

    pub fn with_visibility(mut self, visibility: DescriptorVisibility) -> Self {
        self.visibility = visibility;
        self
    }

    pub fn with_executor(mut self, executor: AgentLaunchExecutorDescriptor) -> Self {
        self.executor = executor;
        self
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct BridgeAgentLaunchRequest {
    pub launcher: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_id: Option<String>,
    pub task: TaskInput,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_session_id: Option<SessionId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_team_name: Option<String>,
}

impl BridgeAgentLaunchRequest {
    pub fn new(launcher: impl Into<String>, task: TaskInput) -> Self {
        let _linkscope_request = linkscope::phase("plugin_sdk.agent_launch.request.new");
        let launcher = launcher.into();
        linkscope::event_fields(
            "plugin_sdk.agent_launch.request.new",
            [
                linkscope::TraceField::text("launcher", launcher.clone()),
                linkscope::TraceField::bytes(
                    "description_bytes",
                    u64::try_from(task.description.len()).unwrap_or(u64::MAX),
                ),
                linkscope::TraceField::bytes(
                    "prompt_bytes",
                    u64::try_from(task.prompt.len()).unwrap_or(u64::MAX),
                ),
            ],
        );
        Self {
            launcher,
            task_id: None,
            task,
            cwd: None,
            parent_session_id: None,
            model: None,
            provider: None,
            active_team_name: None,
        }
    }

    pub fn with_task_id(mut self, task_id: impl Into<String>) -> Self {
        self.task_id = Some(task_id.into());
        self
    }

    pub fn with_cwd(mut self, cwd: impl Into<String>) -> Self {
        self.cwd = Some(cwd.into());
        self
    }

    pub fn with_parent_session_id(mut self, parent_session_id: SessionId) -> Self {
        self.parent_session_id = Some(parent_session_id);
        self
    }

    pub fn with_model(mut self, model: impl Into<String>) -> Self {
        self.model = Some(model.into());
        self
    }

    pub fn with_provider(mut self, provider: impl Into<String>) -> Self {
        self.provider = Some(provider.into());
        self
    }

    pub fn with_active_team_name(mut self, active_team_name: impl Into<String>) -> Self {
        self.active_team_name = Some(active_team_name.into());
        self
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct BridgeAgentLaunchResult {
    pub output: String,
    #[serde(default)]
    pub is_error: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub payload: Option<serde_json::Value>,
}

impl BridgeAgentLaunchResult {
    pub fn success(output: impl Into<String>) -> Self {
        let _linkscope_result = linkscope::phase("plugin_sdk.agent_launch.result.success");
        let output = output.into();
        linkscope::event_fields(
            "plugin_sdk.agent_launch.result.success",
            [linkscope::TraceField::bytes(
                "output_bytes",
                u64::try_from(output.len()).unwrap_or(u64::MAX),
            )],
        );
        Self {
            output,
            is_error: false,
            payload: None,
        }
    }

    pub fn failure(output: impl Into<String>) -> Self {
        let _linkscope_result = linkscope::phase("plugin_sdk.agent_launch.result.failure");
        let output = output.into();
        linkscope::event_fields(
            "plugin_sdk.agent_launch.result.failure",
            [linkscope::TraceField::bytes(
                "output_bytes",
                u64::try_from(output.len()).unwrap_or(u64::MAX),
            )],
        );
        Self {
            output,
            is_error: true,
            payload: None,
        }
    }

    pub fn with_payload(mut self, payload: serde_json::Value) -> Self {
        self.payload = Some(payload);
        self
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum BridgeTeammateEvent {
    TextDelta {
        delta: String,
    },
    Progress {
        #[serde(default)]
        token_count: u64,
        #[serde(default)]
        tool_use_count: u64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        last_tool: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        model_id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        cost_usd: Option<f64>,
    },
    Idle {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        agent_name: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reason: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        summary: Option<String>,
    },
    MessageSent {
        from: String,
        to: String,
        text: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        summary: Option<String>,
    },
    Completed,
    Cancelled,
    Failed {
        error: String,
    },
}
