use serde::{Deserialize, Serialize};

use crate::{DescriptorVisibility, PluginId};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeExtensionTarget {
    MessageRenderer,
    PromptContext,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeExtensionExecutorKind {
    BuiltIn,
    StaticText,
    ProcessBridge,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeExtensionRefreshKind {
    ProcessBridge,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct RuntimeExtensionRefreshDescriptor {
    pub kind: RuntimeExtensionRefreshKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_interval_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auto_refresh_ms: Option<u64>,
}

impl RuntimeExtensionRefreshDescriptor {
    pub fn process_bridge() -> Self {
        let _linkscope_refresh =
            linkscope::phase("plugin_sdk.runtime_extension.refresh.process_bridge");
        linkscope::detail_event_fields(
            "plugin_sdk.runtime_extension.refresh.process_bridge",
            [linkscope::TraceField::text("kind", "process_bridge")],
        );
        Self {
            kind: RuntimeExtensionRefreshKind::ProcessBridge,
            min_interval_ms: None,
            auto_refresh_ms: None,
        }
    }

    pub fn with_min_interval_ms(mut self, min_interval_ms: u64) -> Self {
        self.min_interval_ms = Some(min_interval_ms);
        self
    }

    pub fn with_auto_refresh_ms(mut self, auto_refresh_ms: u64) -> Self {
        self.auto_refresh_ms = Some(auto_refresh_ms);
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct RuntimeExtensionExecutorDescriptor {
    pub kind: RuntimeExtensionExecutorKind,
    pub handler: String,
}

impl RuntimeExtensionExecutorDescriptor {
    pub fn new(kind: RuntimeExtensionExecutorKind, handler: impl Into<String>) -> Self {
        let _linkscope_executor = linkscope::phase("plugin_sdk.runtime_extension.executor.new");
        let handler = handler.into();
        linkscope::detail_event_fields(
            "plugin_sdk.runtime_extension.executor.new",
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
        Self::new(RuntimeExtensionExecutorKind::BuiltIn, handler)
    }

    pub fn static_text(body: impl Into<String>) -> Self {
        Self::new(RuntimeExtensionExecutorKind::StaticText, body)
    }

    pub fn process_bridge(handler: impl Into<String>) -> Self {
        Self::new(RuntimeExtensionExecutorKind::ProcessBridge, handler)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct RuntimeExtensionDescriptor {
    pub plugin_id: PluginId,
    pub target: RuntimeExtensionTarget,
    pub id: String,
    pub label: String,
    pub priority: i32,
    pub visibility: DescriptorVisibility,
    pub executor: RuntimeExtensionExecutorDescriptor,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refresh: Option<RuntimeExtensionRefreshDescriptor>,
}

impl RuntimeExtensionDescriptor {
    pub fn new(
        plugin_id: PluginId,
        target: RuntimeExtensionTarget,
        id: impl Into<String>,
        label: impl Into<String>,
    ) -> Self {
        let _linkscope_extension = linkscope::phase("plugin_sdk.runtime_extension.new");
        let id = id.into();
        let label = label.into();
        linkscope::event_fields(
            "plugin_sdk.runtime_extension.new",
            [
                linkscope::TraceField::text("plugin_id", plugin_id.as_str().to_owned()),
                linkscope::TraceField::text("target", format!("{target:?}")),
                linkscope::TraceField::text("id", id.clone()),
                linkscope::TraceField::bytes(
                    "label_bytes",
                    u64::try_from(label.len()).unwrap_or(u64::MAX),
                ),
            ],
        );
        Self {
            plugin_id,
            target,
            id,
            label,
            priority: 0,
            visibility: DescriptorVisibility::HostVisible,
            executor: RuntimeExtensionExecutorDescriptor::built_in(""),
            refresh: None,
        }
    }

    pub fn with_priority(mut self, priority: i32) -> Self {
        self.priority = priority;
        self
    }

    pub fn with_visibility(mut self, visibility: DescriptorVisibility) -> Self {
        self.visibility = visibility;
        self
    }

    pub fn with_executor(mut self, executor: RuntimeExtensionExecutorDescriptor) -> Self {
        self.executor = executor;
        self
    }

    pub fn with_refresh(mut self, refresh: RuntimeExtensionRefreshDescriptor) -> Self {
        self.refresh = Some(refresh);
        self
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct BridgePromptContextRefreshRequest {
    pub extension_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_chars: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state: Option<serde_json::Value>,
}

impl BridgePromptContextRefreshRequest {
    pub fn new(extension_id: impl Into<String>) -> Self {
        let _linkscope_request =
            linkscope::phase("plugin_sdk.runtime_extension.refresh_request.new");
        let extension_id = extension_id.into();
        linkscope::event_fields(
            "plugin_sdk.runtime_extension.refresh_request.new",
            [linkscope::TraceField::text(
                "extension_id",
                extension_id.clone(),
            )],
        );
        Self {
            extension_id,
            cwd: None,
            max_chars: None,
            state: None,
        }
    }

    pub fn with_cwd(mut self, cwd: impl Into<String>) -> Self {
        self.cwd = Some(cwd.into());
        self
    }

    pub fn with_max_chars(mut self, max_chars: usize) -> Self {
        self.max_chars = Some(max_chars);
        self
    }

    pub fn with_state(mut self, state: serde_json::Value) -> Self {
        self.state = Some(state);
        self
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct BridgePromptContextRefreshResult {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state: Option<serde_json::Value>,
}

impl BridgePromptContextRefreshResult {
    pub fn body(body: impl Into<String>) -> Self {
        let _linkscope_result =
            linkscope::phase("plugin_sdk.runtime_extension.refresh_result.body");
        let body = body.into();
        linkscope::event_fields(
            "plugin_sdk.runtime_extension.refresh_result.body",
            [linkscope::TraceField::bytes(
                "body_bytes",
                u64::try_from(body.len()).unwrap_or(u64::MAX),
            )],
        );
        Self {
            body: Some(body),
            state: None,
        }
    }

    pub fn with_state(mut self, state: serde_json::Value) -> Self {
        self.state = Some(state);
        self
    }
}
