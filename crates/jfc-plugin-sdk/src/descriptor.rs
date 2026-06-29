use serde::{Deserialize, Serialize};

mod resource;

pub use resource::{CommandDescriptor, ResourceDescriptor};

use crate::PluginId;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DescriptorVisibility {
    Internal,
    HostVisible,
    ModelVisible,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResourceKind {
    Agent,
    Skill,
    Workflow,
    Prompt,
    McpServer,
    Memory,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ToolDescriptor {
    pub plugin_id: PluginId,
    pub name: String,
    pub description: String,
    pub input_schema: serde_json::Value,
    #[serde(default)]
    pub executor: ToolExecutorDescriptor,
    #[serde(default)]
    pub approval_policy: ToolApprovalPolicy,
    pub visibility: DescriptorVisibility,
}

impl ToolDescriptor {
    pub fn new(
        plugin_id: PluginId,
        name: impl Into<String>,
        description: impl Into<String>,
        input_schema: serde_json::Value,
    ) -> Self {
        Self {
            plugin_id,
            name: name.into(),
            description: description.into(),
            input_schema,
            executor: ToolExecutorDescriptor::built_in(""),
            approval_policy: ToolApprovalPolicy::ReadOnly,
            visibility: DescriptorVisibility::HostVisible,
        }
    }

    pub fn with_executor(mut self, kind: ToolExecutorKind, handler: impl Into<String>) -> Self {
        self.executor = ToolExecutorDescriptor::new(kind, handler);
        self
    }

    pub fn with_approval_policy(mut self, approval_policy: ToolApprovalPolicy) -> Self {
        self.approval_policy = approval_policy;
        self
    }

    pub fn with_visibility(mut self, visibility: DescriptorVisibility) -> Self {
        self.visibility = visibility;
        self
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolExecutorKind {
    #[default]
    BuiltIn,
    ProcessBridge,
    Mcp,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ToolExecutorDescriptor {
    pub kind: ToolExecutorKind,
    pub handler: String,
}

impl ToolExecutorDescriptor {
    pub fn new(kind: ToolExecutorKind, handler: impl Into<String>) -> Self {
        Self {
            kind,
            handler: handler.into(),
        }
    }

    pub fn built_in(handler: impl Into<String>) -> Self {
        Self::new(ToolExecutorKind::BuiltIn, handler)
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolApprovalPolicy {
    #[default]
    ReadOnly,
    Mutating,
    Management,
}

impl ToolApprovalPolicy {
    pub const fn mutates_user_state(self) -> bool {
        matches!(self, Self::Mutating)
    }

    pub const fn plan_mode_allowed(self) -> bool {
        matches!(self, Self::ReadOnly | Self::Management)
    }

    pub const fn needs_interactive_approval(self) -> bool {
        matches!(self, Self::Mutating)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ProviderDescriptor {
    pub plugin_id: PluginId,
    pub provider: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub models: Vec<ProviderModelDescriptor>,
    #[serde(default)]
    pub executor: ProviderExecutorDescriptor,
    pub visibility: DescriptorVisibility,
}

impl ProviderDescriptor {
    pub fn new(plugin_id: PluginId, provider: impl Into<String>) -> Self {
        Self {
            plugin_id,
            provider: provider.into(),
            models: Vec::new(),
            executor: ProviderExecutorDescriptor::built_in(""),
            visibility: DescriptorVisibility::HostVisible,
        }
    }

    pub fn with_model(mut self, model: impl Into<String>) -> Self {
        self.models.push(ProviderModelDescriptor::new(model));
        self
    }

    pub fn with_model_info(
        mut self,
        id: impl Into<String>,
        display_name: impl Into<String>,
        context_window_tokens: Option<usize>,
        max_output_tokens: Option<usize>,
    ) -> Self {
        self.models.push(
            ProviderModelDescriptor::new(id)
                .with_display_name(display_name)
                .with_context_window_tokens(context_window_tokens)
                .with_max_output_tokens(max_output_tokens),
        );
        self
    }

    pub fn with_executor(mut self, kind: ProviderExecutorKind, handler: impl Into<String>) -> Self {
        self.executor = ProviderExecutorDescriptor::new(kind, handler);
        self
    }

    pub fn with_visibility(mut self, visibility: DescriptorVisibility) -> Self {
        self.visibility = visibility;
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct ProviderModelDescriptor {
    pub id: String,
    pub display_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_window_tokens: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_output_tokens: Option<usize>,
}

impl ProviderModelDescriptor {
    pub fn new(id: impl Into<String>) -> Self {
        let id = id.into();
        Self {
            display_name: id.clone(),
            id,
            context_window_tokens: None,
            max_output_tokens: None,
        }
    }

    pub fn with_display_name(mut self, display_name: impl Into<String>) -> Self {
        self.display_name = display_name.into();
        self
    }

    pub fn with_context_window_tokens(mut self, tokens: Option<usize>) -> Self {
        self.context_window_tokens = tokens;
        self
    }

    pub fn with_max_output_tokens(mut self, tokens: Option<usize>) -> Self {
        self.max_output_tokens = tokens;
        self
    }
}

impl<'de> Deserialize<'de> for ProviderModelDescriptor {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum ProviderModelWire {
            Id(String),
            Detailed {
                id: String,
                #[serde(default)]
                display_name: Option<String>,
                #[serde(default)]
                context_window_tokens: Option<usize>,
                #[serde(default)]
                max_output_tokens: Option<usize>,
            },
        }

        match ProviderModelWire::deserialize(deserializer)? {
            ProviderModelWire::Id(id) => Ok(Self::new(id)),
            ProviderModelWire::Detailed {
                id,
                display_name,
                context_window_tokens,
                max_output_tokens,
            } => Ok(Self {
                display_name: display_name.unwrap_or_else(|| id.clone()),
                id,
                context_window_tokens,
                max_output_tokens,
            }),
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderExecutorKind {
    #[default]
    BuiltIn,
    ProcessBridge,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ProviderExecutorDescriptor {
    pub kind: ProviderExecutorKind,
    pub handler: String,
}

impl ProviderExecutorDescriptor {
    pub fn new(kind: ProviderExecutorKind, handler: impl Into<String>) -> Self {
        Self {
            kind,
            handler: handler.into(),
        }
    }

    pub fn built_in(handler: impl Into<String>) -> Self {
        Self::new(ProviderExecutorKind::BuiltIn, handler)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct AuthDescriptor {
    pub plugin_id: PluginId,
    pub provider: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub methods: Vec<AuthMethodDescriptor>,
}

impl AuthDescriptor {
    pub fn new(plugin_id: PluginId, provider: impl Into<String>) -> Self {
        Self {
            plugin_id,
            provider: provider.into(),
            methods: Vec::new(),
        }
    }

    pub fn with_method(mut self, method: AuthMethodDescriptor) -> Self {
        self.methods.push(method);
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AuthMethodDescriptor {
    ApiKey {
        label: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        env_var: Option<String>,
    },
    OAuth {
        label: String,
        authorize_url: String,
    },
    DeviceCode {
        label: String,
        verification_url: String,
    },
}
