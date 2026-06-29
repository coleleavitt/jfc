use std::sync::Arc;

use jfc_plugin_sdk::{
    AgentLaunchDescriptor, CommandDescriptor, HookName, MetricDescriptor, PluginManifest,
    ProviderDescriptor, ResourceDescriptor, RuntimeActionDescriptor, RuntimeExtensionDescriptor,
    ServiceDescriptor, ToolDescriptor, UiPanelDescriptor, UiSlotDescriptor, UiWidgetDescriptor,
};

use crate::{
    HookInvocation, HookValue, PluginHostError,
    hook::{HookCallback, HookDefinition},
    registration_activation::run_partial_finalizers,
};

pub type PluginFinalizer = Arc<dyn Fn() -> Result<(), PluginHostError> + Send + Sync>;
type ActivationCallback =
    Arc<dyn Fn(&mut PluginActivation) -> Result<(), PluginHostError> + Send + Sync>;

pub trait InternalPlugin: Send + Sync {
    fn manifest(&self) -> &PluginManifest;

    fn activation_order(&self) -> i32 {
        0
    }

    fn activate(&self) -> Result<PluginActivation, PluginHostError>;
}

#[derive(Clone, Default)]
pub struct PluginActivation {
    pub(crate) hooks: Vec<HookDefinition>,
    pub(crate) service_descriptors: Vec<ServiceDescriptor>,
    pub(crate) tool_descriptors: Vec<ToolDescriptor>,
    pub(crate) provider_descriptors: Vec<ProviderDescriptor>,
    pub(crate) resource_descriptors: Vec<ResourceDescriptor>,
    pub(crate) command_descriptors: Vec<CommandDescriptor>,
    pub(crate) ui_slot_descriptors: Vec<UiSlotDescriptor>,
    pub(crate) ui_panel_descriptors: Vec<UiPanelDescriptor>,
    pub(crate) ui_widget_descriptors: Vec<UiWidgetDescriptor>,
    pub(crate) runtime_action_descriptors: Vec<RuntimeActionDescriptor>,
    pub(crate) runtime_extension_descriptors: Vec<RuntimeExtensionDescriptor>,
    pub(crate) agent_launch_descriptors: Vec<AgentLaunchDescriptor>,
    pub(crate) metric_descriptors: Vec<MetricDescriptor>,
    pub(crate) finalizers: Vec<PluginFinalizer>,
}

pub struct PluginRegistration {
    manifest: PluginManifest,
    activation_order: i32,
    hooks: Vec<HookDefinition>,
    service_descriptors: Vec<ServiceDescriptor>,
    tool_descriptors: Vec<ToolDescriptor>,
    provider_descriptors: Vec<ProviderDescriptor>,
    resource_descriptors: Vec<ResourceDescriptor>,
    command_descriptors: Vec<CommandDescriptor>,
    ui_slot_descriptors: Vec<UiSlotDescriptor>,
    ui_panel_descriptors: Vec<UiPanelDescriptor>,
    ui_widget_descriptors: Vec<UiWidgetDescriptor>,
    runtime_action_descriptors: Vec<RuntimeActionDescriptor>,
    runtime_extension_descriptors: Vec<RuntimeExtensionDescriptor>,
    agent_launch_descriptors: Vec<AgentLaunchDescriptor>,
    metric_descriptors: Vec<MetricDescriptor>,
    finalizers: Vec<PluginFinalizer>,
    activation: Option<ActivationCallback>,
}

impl PluginRegistration {
    pub fn new(manifest: PluginManifest) -> Self {
        Self {
            manifest,
            activation_order: 0,
            hooks: Vec::new(),
            service_descriptors: Vec::new(),
            tool_descriptors: Vec::new(),
            provider_descriptors: Vec::new(),
            resource_descriptors: Vec::new(),
            command_descriptors: Vec::new(),
            ui_slot_descriptors: Vec::new(),
            ui_panel_descriptors: Vec::new(),
            ui_widget_descriptors: Vec::new(),
            runtime_action_descriptors: Vec::new(),
            runtime_extension_descriptors: Vec::new(),
            agent_launch_descriptors: Vec::new(),
            metric_descriptors: Vec::new(),
            finalizers: Vec::new(),
            activation: None,
        }
    }

    pub fn with_activation_order(mut self, activation_order: i32) -> Self {
        self.activation_order = activation_order;
        self
    }

    pub fn with_hook<F>(mut self, name: HookName, priority: i32, callback: F) -> Self
    where
        F: for<'a> Fn(HookInvocation<'a>) -> Result<HookValue, PluginHostError>
            + Send
            + Sync
            + 'static,
    {
        let callback: HookCallback = Arc::new(callback);
        self.hooks
            .push(HookDefinition::new(name, priority, callback));
        self
    }

    pub fn with_finalizer<F>(mut self, finalizer: F) -> Self
    where
        F: Fn() -> Result<(), PluginHostError> + Send + Sync + 'static,
    {
        self.finalizers.push(Arc::new(finalizer));
        self
    }

    pub fn with_tool_descriptor(mut self, descriptor: ToolDescriptor) -> Self {
        self.tool_descriptors.push(descriptor);
        self
    }

    pub fn with_service_descriptor(mut self, descriptor: ServiceDescriptor) -> Self {
        self.service_descriptors.push(descriptor);
        self
    }

    pub fn with_service_descriptors<I>(mut self, descriptors: I) -> Self
    where
        I: IntoIterator<Item = ServiceDescriptor>,
    {
        self.service_descriptors.extend(descriptors);
        self
    }

    pub fn with_tool_descriptors<I>(mut self, descriptors: I) -> Self
    where
        I: IntoIterator<Item = ToolDescriptor>,
    {
        self.tool_descriptors.extend(descriptors);
        self
    }

    pub fn with_provider_descriptor(mut self, descriptor: ProviderDescriptor) -> Self {
        self.provider_descriptors.push(descriptor);
        self
    }

    pub fn with_provider_descriptors<I>(mut self, descriptors: I) -> Self
    where
        I: IntoIterator<Item = ProviderDescriptor>,
    {
        self.provider_descriptors.extend(descriptors);
        self
    }

    pub fn with_resource_descriptor(mut self, descriptor: ResourceDescriptor) -> Self {
        self.resource_descriptors.push(descriptor);
        self
    }

    pub fn with_resource_descriptors<I>(mut self, descriptors: I) -> Self
    where
        I: IntoIterator<Item = ResourceDescriptor>,
    {
        self.resource_descriptors.extend(descriptors);
        self
    }

    pub fn with_command_descriptor(mut self, descriptor: CommandDescriptor) -> Self {
        self.command_descriptors.push(descriptor);
        self
    }

    pub fn with_command_descriptors<I>(mut self, descriptors: I) -> Self
    where
        I: IntoIterator<Item = CommandDescriptor>,
    {
        self.command_descriptors.extend(descriptors);
        self
    }

    pub fn with_ui_slot_descriptor(mut self, descriptor: UiSlotDescriptor) -> Self {
        self.ui_slot_descriptors.push(descriptor);
        self
    }

    pub fn with_ui_slot_descriptors<I>(mut self, descriptors: I) -> Self
    where
        I: IntoIterator<Item = UiSlotDescriptor>,
    {
        self.ui_slot_descriptors.extend(descriptors);
        self
    }

    pub fn with_ui_panel_descriptor(mut self, descriptor: UiPanelDescriptor) -> Self {
        self.ui_panel_descriptors.push(descriptor);
        self
    }

    pub fn with_ui_panel_descriptors<I>(mut self, descriptors: I) -> Self
    where
        I: IntoIterator<Item = UiPanelDescriptor>,
    {
        self.ui_panel_descriptors.extend(descriptors);
        self
    }

    pub fn with_ui_widget_descriptor(mut self, descriptor: UiWidgetDescriptor) -> Self {
        self.ui_widget_descriptors.push(descriptor);
        self
    }

    pub fn with_ui_widget_descriptors<I>(mut self, descriptors: I) -> Self
    where
        I: IntoIterator<Item = UiWidgetDescriptor>,
    {
        self.ui_widget_descriptors.extend(descriptors);
        self
    }

    pub fn with_runtime_action_descriptor(mut self, descriptor: RuntimeActionDescriptor) -> Self {
        self.runtime_action_descriptors.push(descriptor);
        self
    }

    pub fn with_runtime_action_descriptors<I>(mut self, descriptors: I) -> Self
    where
        I: IntoIterator<Item = RuntimeActionDescriptor>,
    {
        self.runtime_action_descriptors.extend(descriptors);
        self
    }

    pub fn with_runtime_extension_descriptor(
        mut self,
        descriptor: RuntimeExtensionDescriptor,
    ) -> Self {
        self.runtime_extension_descriptors.push(descriptor);
        self
    }

    pub fn with_runtime_extension_descriptors<I>(mut self, descriptors: I) -> Self
    where
        I: IntoIterator<Item = RuntimeExtensionDescriptor>,
    {
        self.runtime_extension_descriptors.extend(descriptors);
        self
    }

    pub fn with_agent_launch_descriptor(mut self, descriptor: AgentLaunchDescriptor) -> Self {
        self.agent_launch_descriptors.push(descriptor);
        self
    }

    pub fn with_agent_launch_descriptors<I>(mut self, descriptors: I) -> Self
    where
        I: IntoIterator<Item = AgentLaunchDescriptor>,
    {
        self.agent_launch_descriptors.extend(descriptors);
        self
    }

    pub fn with_metric_descriptor(mut self, descriptor: MetricDescriptor) -> Self {
        self.metric_descriptors.push(descriptor);
        self
    }

    pub fn with_metric_descriptors<I>(mut self, descriptors: I) -> Self
    where
        I: IntoIterator<Item = MetricDescriptor>,
    {
        self.metric_descriptors.extend(descriptors);
        self
    }

    pub fn with_activation<F>(mut self, activation: F) -> Self
    where
        F: Fn(&mut PluginActivation) -> Result<(), PluginHostError> + Send + Sync + 'static,
    {
        self.activation = Some(Arc::new(activation));
        self
    }
}

impl InternalPlugin for PluginRegistration {
    fn manifest(&self) -> &PluginManifest {
        &self.manifest
    }

    fn activation_order(&self) -> i32 {
        self.activation_order
    }

    fn activate(&self) -> Result<PluginActivation, PluginHostError> {
        let mut activation = PluginActivation {
            hooks: self.hooks.clone(),
            service_descriptors: self.service_descriptors.clone(),
            tool_descriptors: self.tool_descriptors.clone(),
            provider_descriptors: self.provider_descriptors.clone(),
            resource_descriptors: self.resource_descriptors.clone(),
            command_descriptors: self.command_descriptors.clone(),
            ui_slot_descriptors: self.ui_slot_descriptors.clone(),
            ui_panel_descriptors: self.ui_panel_descriptors.clone(),
            ui_widget_descriptors: self.ui_widget_descriptors.clone(),
            runtime_action_descriptors: self.runtime_action_descriptors.clone(),
            runtime_extension_descriptors: self.runtime_extension_descriptors.clone(),
            agent_launch_descriptors: self.agent_launch_descriptors.clone(),
            metric_descriptors: self.metric_descriptors.clone(),
            finalizers: self.finalizers.clone(),
        };

        if let Some(callback) = &self.activation {
            if let Err(error) = callback(&mut activation) {
                run_partial_finalizers(&activation.finalizers);
                return Err(error);
            }
        }

        Ok(activation)
    }
}
