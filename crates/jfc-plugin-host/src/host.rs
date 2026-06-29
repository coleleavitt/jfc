use std::collections::BTreeMap;

use jfc_plugin_sdk::{
    AgentLaunchDescriptor, CommandDescriptor, HookDescriptor, HookName, MetricDescriptor, PluginId,
    ProviderDescriptor, ResourceDescriptor, RuntimeActionDescriptor, RuntimeExtensionDescriptor,
    ServiceDescriptor, ToolDescriptor, UiPanelDescriptor, UiSlotDescriptor, UiWidgetDescriptor,
};

use crate::{
    HookInvocation, HookValue, InternalPlugin, PluginErrorPhase, PluginErrorReport,
    PluginFinalizer, PluginHostError, PluginHostSnapshot, PluginStatusEntry, PluginStatusKind,
    hook::ActivatedHook,
};

pub struct PluginHost {
    pub(crate) plugins: BTreeMap<String, PluginEntry>,
    next_registration_sequence: u64,
    pub(crate) next_activation_sequence: u64,
    pub(crate) next_hook_sequence: u64,
}

impl PluginHost {
    pub fn new() -> Self {
        Self {
            plugins: BTreeMap::new(),
            next_registration_sequence: 0,
            next_activation_sequence: 0,
            next_hook_sequence: 0,
        }
    }

    pub fn register_internal<P>(&mut self, plugin: P) -> Result<(), PluginHostError>
    where
        P: InternalPlugin + 'static,
    {
        let _linkscope_register = linkscope::phase("plugin_host.register_internal");
        let plugin_id = plugin.manifest().id.clone();
        let key = plugin_id.as_str().to_owned();
        linkscope::event_fields(
            "plugin_host.register_internal",
            [
                linkscope::TraceField::text("plugin_id", key.clone()),
                linkscope::TraceField::count(
                    "plugins_before",
                    u64::try_from(self.plugins.len()).unwrap_or(u64::MAX),
                ),
            ],
        );
        if self.plugins.contains_key(&key) {
            return Err(PluginHostError::DuplicatePluginId { plugin_id: key });
        }

        let entry = PluginEntry {
            plugin_id,
            plugin: Box::new(plugin),
            status: PluginStatusKind::Registered,
            registration_sequence: self.next_registration_sequence,
            activation_sequence: None,
            finalizers: Vec::new(),
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
            errors: Vec::new(),
        };
        self.next_registration_sequence = self.next_registration_sequence.saturating_add(1);
        self.plugins.insert(key, entry);
        linkscope::record_items(
            "plugin_host.registered",
            u64::try_from(self.plugins.len()).unwrap_or(u64::MAX),
        );
        Ok(())
    }

    pub fn trigger_hook(
        &mut self,
        name: HookName,
        value: HookValue,
    ) -> Result<HookValue, PluginHostError> {
        self.trigger_hook_until(name, value, |_| false)
    }

    pub fn trigger_hook_until<F>(
        &mut self,
        name: HookName,
        value: HookValue,
        mut should_stop: F,
    ) -> Result<HookValue, PluginHostError>
    where
        F: FnMut(&HookValue) -> bool,
    {
        let _linkscope_hook = linkscope::phase("plugin_host.trigger_hook");
        let plan = self.hook_plan(name);
        linkscope::event_fields(
            "plugin_host.trigger_hook.plan",
            [
                linkscope::TraceField::text("hook", format!("{name:?}")),
                linkscope::TraceField::count(
                    "callbacks",
                    u64::try_from(plan.len()).unwrap_or(u64::MAX),
                ),
            ],
        );
        let mut current_value = value;

        for hook in plan {
            let _linkscope_callback = linkscope::phase("plugin_host.hook_callback");
            linkscope::detail_event_fields(
                "plugin_host.hook_callback",
                [
                    linkscope::TraceField::text("plugin_id", hook.plugin_id.as_str().to_owned()),
                    linkscope::TraceField::text("hook", format!("{name:?}")),
                ],
            );
            let invocation = HookInvocation::new(&hook.plugin_id, name, &current_value);
            match (hook.callback)(invocation) {
                Ok(next_value) => {
                    current_value = next_value;
                    if should_stop(&current_value) {
                        linkscope::event_fields(
                            "plugin_host.trigger_hook.stop",
                            [linkscope::TraceField::text("hook", format!("{name:?}"))],
                        );
                        break;
                    }
                }
                Err(error) => {
                    let message = error.to_string();
                    self.record_error(&hook.plugin_id, PluginErrorPhase::Hook, message.clone());
                    return Err(PluginHostError::HookFailed {
                        plugin_id: hook.plugin_id.into_inner(),
                        hook: name,
                        message,
                    });
                }
            }
        }

        Ok(current_value)
    }

    pub fn has_hook(&self, name: HookName) -> bool {
        self.plugins
            .values()
            .filter(|entry| entry.status == PluginStatusKind::Active)
            .any(|entry| entry.hooks.iter().any(|hook| hook.name == name))
    }

    pub fn status_snapshot(&self) -> PluginHostSnapshot {
        let _linkscope_snapshot = linkscope::phase("plugin_host.status_snapshot");
        let mut plugins = self
            .plugins
            .values()
            .map(PluginEntry::status_entry)
            .collect::<Vec<_>>();
        plugins.sort_by(|left, right| {
            (left.activation_order, left.plugin_id.as_str())
                .cmp(&(right.activation_order, right.plugin_id.as_str()))
        });
        linkscope::record_items(
            "plugin_host.snapshot.plugins",
            u64::try_from(plugins.len()).unwrap_or(u64::MAX),
        );
        PluginHostSnapshot { plugins }
    }

    fn hook_plan(&self, name: HookName) -> Vec<ActivatedHook> {
        let _linkscope_plan = linkscope::phase("plugin_host.hook_plan");
        let mut hooks = self
            .plugins
            .values()
            .filter(|entry| entry.status == PluginStatusKind::Active)
            .flat_map(|entry| entry.hooks.iter())
            .filter(|hook| hook.name == name)
            .cloned()
            .collect::<Vec<_>>();
        hooks.sort_by(|left, right| {
            (
                left.priority,
                left.activation_order,
                left.activation_sequence,
                left.hook_sequence,
                left.plugin_id.as_str(),
            )
                .cmp(&(
                    right.priority,
                    right.activation_order,
                    right.activation_sequence,
                    right.hook_sequence,
                    right.plugin_id.as_str(),
                ))
        });
        linkscope::record_items(
            "plugin_host.hook_plan.callbacks",
            u64::try_from(hooks.len()).unwrap_or(u64::MAX),
        );
        hooks
    }

    pub(crate) fn record_error(
        &mut self,
        plugin_id: &PluginId,
        phase: PluginErrorPhase,
        message: String,
    ) {
        let _linkscope_error = linkscope::phase("plugin_host.record_error");
        linkscope::event_fields(
            "plugin_host.record_error",
            [
                linkscope::TraceField::text("plugin_id", plugin_id.as_str().to_owned()),
                linkscope::TraceField::text("phase", format!("{phase:?}")),
                linkscope::TraceField::bytes(
                    "message_bytes",
                    u64::try_from(message.len()).unwrap_or(u64::MAX),
                ),
            ],
        );
        if let Some(entry) = self.plugins.get_mut(plugin_id.as_str()) {
            entry.errors.push(PluginErrorReport {
                plugin_id: plugin_id.clone(),
                phase,
                message,
            });
        }
    }

    pub(crate) fn entry(&self, key: &str) -> Result<&PluginEntry, PluginHostError> {
        self.plugins
            .get(key)
            .ok_or_else(|| PluginHostError::PluginNotFound {
                plugin_id: key.to_owned(),
            })
    }

    pub(crate) fn entry_mut(&mut self, key: &str) -> Result<&mut PluginEntry, PluginHostError> {
        self.plugins
            .get_mut(key)
            .ok_or_else(|| PluginHostError::PluginNotFound {
                plugin_id: key.to_owned(),
            })
    }
}

impl Default for PluginHost {
    fn default() -> Self {
        Self::new()
    }
}

pub(crate) struct PluginEntry {
    pub(crate) plugin_id: PluginId,
    pub(crate) plugin: Box<dyn InternalPlugin>,
    pub(crate) status: PluginStatusKind,
    registration_sequence: u64,
    pub(crate) activation_sequence: Option<u64>,
    pub(crate) finalizers: Vec<PluginFinalizer>,
    pub(crate) hooks: Vec<ActivatedHook>,
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
    errors: Vec<PluginErrorReport>,
}

impl PluginEntry {
    pub(crate) fn activation_sort_key(&self) -> (i32, String, u64) {
        (
            self.plugin.activation_order(),
            self.plugin_id.as_str().to_owned(),
            self.registration_sequence,
        )
    }

    fn status_entry(&self) -> PluginStatusEntry {
        PluginStatusEntry {
            plugin_id: self.plugin_id.clone(),
            manifest: self.plugin.manifest().clone(),
            source: self.plugin.manifest().source.clone(),
            status: self.status,
            activation_order: self.plugin.activation_order(),
            hooks: self
                .hooks
                .iter()
                .map(|hook| {
                    HookDescriptor::new(self.plugin_id.clone(), hook.name)
                        .with_priority(hook.priority)
                })
                .collect(),
            errors: self.errors.clone(),
        }
    }
}
