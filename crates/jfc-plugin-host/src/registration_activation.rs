use std::sync::Arc;

use jfc_plugin_sdk::{
    AgentLaunchDescriptor, CommandDescriptor, HookName, MetricDescriptor, ProviderDescriptor,
    ResourceDescriptor, RuntimeActionDescriptor, RuntimeExtensionDescriptor, ServiceDescriptor,
    ToolDescriptor, UiPanelDescriptor, UiSlotDescriptor, UiWidgetDescriptor,
};

use crate::{
    HookInvocation, HookValue, PluginActivation, PluginFinalizer, PluginHostError,
    hook::HookDefinition,
};

impl PluginActivation {
    pub fn add_hook<F>(&mut self, name: HookName, priority: i32, callback: F)
    where
        F: for<'a> Fn(HookInvocation<'a>) -> Result<HookValue, PluginHostError>
            + Send
            + Sync
            + 'static,
    {
        self.hooks
            .push(HookDefinition::new(name, priority, Arc::new(callback)));
    }

    pub fn add_finalizer<F>(&mut self, finalizer: F)
    where
        F: Fn() -> Result<(), PluginHostError> + Send + Sync + 'static,
    {
        self.finalizers.push(Arc::new(finalizer));
    }

    pub fn add_tool_descriptor(&mut self, descriptor: ToolDescriptor) {
        self.tool_descriptors.push(descriptor);
    }

    pub fn add_provider_descriptor(&mut self, descriptor: ProviderDescriptor) {
        self.provider_descriptors.push(descriptor);
    }

    pub fn add_service_descriptor(&mut self, descriptor: ServiceDescriptor) {
        self.service_descriptors.push(descriptor);
    }

    pub fn add_resource_descriptor(&mut self, descriptor: ResourceDescriptor) {
        self.resource_descriptors.push(descriptor);
    }

    pub fn add_command_descriptor(&mut self, descriptor: CommandDescriptor) {
        self.command_descriptors.push(descriptor);
    }

    pub fn add_ui_slot_descriptor(&mut self, descriptor: UiSlotDescriptor) {
        self.ui_slot_descriptors.push(descriptor);
    }

    pub fn add_ui_panel_descriptor(&mut self, descriptor: UiPanelDescriptor) {
        self.ui_panel_descriptors.push(descriptor);
    }

    pub fn add_ui_widget_descriptor(&mut self, descriptor: UiWidgetDescriptor) {
        self.ui_widget_descriptors.push(descriptor);
    }

    pub fn add_runtime_action_descriptor(&mut self, descriptor: RuntimeActionDescriptor) {
        self.runtime_action_descriptors.push(descriptor);
    }

    pub fn add_runtime_extension_descriptor(&mut self, descriptor: RuntimeExtensionDescriptor) {
        self.runtime_extension_descriptors.push(descriptor);
    }

    pub fn add_agent_launch_descriptor(&mut self, descriptor: AgentLaunchDescriptor) {
        self.agent_launch_descriptors.push(descriptor);
    }

    pub fn add_metric_descriptor(&mut self, descriptor: MetricDescriptor) {
        self.metric_descriptors.push(descriptor);
    }
}

pub(crate) fn run_partial_finalizers(finalizers: &[PluginFinalizer]) {
    for finalizer in finalizers.iter().rev() {
        if let Err(error) = finalizer() {
            tracing::warn!(target: "jfc::plugin_host", error = %error, "partial activation finalizer failed");
        }
    }
}
