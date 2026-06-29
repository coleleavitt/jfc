use jfc_plugin_sdk::PluginId;

use crate::{PluginErrorPhase, PluginHost, PluginHostError, PluginStatusKind, hook::ActivatedHook};

impl PluginHost {
    pub fn activate_all(&mut self) -> Result<(), PluginHostError> {
        let _linkscope_activate = linkscope::phase("plugin_host.activate_all");
        let candidates = self.activation_candidates();
        linkscope::record_items(
            "plugin_host.activation_candidates",
            u64::try_from(candidates.len()).unwrap_or(u64::MAX),
        );
        for plugin_id in candidates {
            self.activate_plugin_key(&plugin_id)?;
        }
        Ok(())
    }

    pub fn enable_plugin(&mut self, plugin_id: &PluginId) -> Result<(), PluginHostError> {
        let _linkscope_enable = linkscope::phase("plugin_host.enable_plugin");
        linkscope::event_fields(
            "plugin_host.enable_plugin",
            [linkscope::TraceField::text(
                "plugin_id",
                plugin_id.as_str().to_owned(),
            )],
        );
        let key = plugin_id.as_str();
        let entry = self.entry_mut(key)?;
        if entry.status != PluginStatusKind::Disabled {
            return Ok(());
        }
        entry.status = PluginStatusKind::Registered;
        self.activate_plugin_key(key)
    }

    pub fn disable_plugin(&mut self, plugin_id: &PluginId) -> Result<(), PluginHostError> {
        let _linkscope_disable = linkscope::phase("plugin_host.disable_plugin");
        linkscope::event_fields(
            "plugin_host.disable_plugin",
            [linkscope::TraceField::text(
                "plugin_id",
                plugin_id.as_str().to_owned(),
            )],
        );
        let key = plugin_id.as_str();
        let should_finalize = self.entry(key)?.status == PluginStatusKind::Active;
        if should_finalize {
            self.finalize_plugin(key)?;
        }
        let entry = self.entry_mut(key)?;
        entry.status = PluginStatusKind::Disabled;
        entry.activation_sequence = None;
        entry.hooks.clear();
        entry.service_descriptors.clear();
        entry.tool_descriptors.clear();
        entry.provider_descriptors.clear();
        entry.resource_descriptors.clear();
        entry.command_descriptors.clear();
        entry.ui_slot_descriptors.clear();
        entry.ui_panel_descriptors.clear();
        entry.ui_widget_descriptors.clear();
        entry.runtime_action_descriptors.clear();
        entry.runtime_extension_descriptors.clear();
        entry.agent_launch_descriptors.clear();
        entry.metric_descriptors.clear();
        Ok(())
    }

    pub fn shutdown(&mut self) -> Result<(), PluginHostError> {
        let _linkscope_shutdown = linkscope::phase("plugin_host.shutdown");
        let mut active = self
            .plugins
            .iter()
            .filter(|(_, entry)| entry.status == PluginStatusKind::Active)
            .map(|(id, entry)| (entry.activation_sort_key(), id.clone()))
            .collect::<Vec<_>>();
        active.sort_by(|left, right| right.0.cmp(&left.0));
        linkscope::record_items(
            "plugin_host.shutdown.active",
            u64::try_from(active.len()).unwrap_or(u64::MAX),
        );

        for (_, plugin_id) in active {
            self.finalize_plugin(&plugin_id)?;
            let entry = self.entry_mut(&plugin_id)?;
            entry.status = PluginStatusKind::Registered;
            entry.activation_sequence = None;
            entry.hooks.clear();
            entry.service_descriptors.clear();
            entry.tool_descriptors.clear();
            entry.provider_descriptors.clear();
            entry.resource_descriptors.clear();
            entry.command_descriptors.clear();
            entry.ui_slot_descriptors.clear();
            entry.ui_panel_descriptors.clear();
            entry.ui_widget_descriptors.clear();
            entry.runtime_action_descriptors.clear();
            entry.runtime_extension_descriptors.clear();
            entry.agent_launch_descriptors.clear();
            entry.metric_descriptors.clear();
        }

        Ok(())
    }

    pub(crate) fn activate_plugin_key(&mut self, key: &str) -> Result<(), PluginHostError> {
        let _linkscope_activate = linkscope::phase("plugin_host.activate_plugin");
        linkscope::event_fields(
            "plugin_host.activate_plugin",
            [linkscope::TraceField::text("plugin_id", key.to_owned())],
        );
        let activation = {
            let entry = self.entry(key)?;
            if entry.status != PluginStatusKind::Registered {
                return Ok(());
            }
            entry.plugin.activate()
        };

        match activation {
            Ok(activation) => self.finish_activation(key, activation),
            Err(error) => {
                let plugin_id = self.entry(key)?.plugin_id.clone();
                let message = error.to_string();
                self.record_error(&plugin_id, PluginErrorPhase::Activation, message.clone());
                self.entry_mut(key)?.status = PluginStatusKind::Failed;
                Err(PluginHostError::ActivationFailed {
                    plugin_id: plugin_id.into_inner(),
                    message,
                })
            }
        }
    }

    fn finish_activation(
        &mut self,
        key: &str,
        activation: crate::PluginActivation,
    ) -> Result<(), PluginHostError> {
        let _linkscope_finish = linkscope::phase("plugin_host.finish_activation");
        let (plugin_id, activation_order) = {
            let entry = self.entry(key)?;
            (entry.plugin_id.clone(), entry.plugin.activation_order())
        };
        linkscope::event_fields(
            "plugin_host.finish_activation.descriptors",
            [
                linkscope::TraceField::text("plugin_id", plugin_id.as_str().to_owned()),
                linkscope::TraceField::count(
                    "hooks",
                    u64::try_from(activation.hooks.len()).unwrap_or(u64::MAX),
                ),
                linkscope::TraceField::count(
                    "tools",
                    u64::try_from(activation.tool_descriptors.len()).unwrap_or(u64::MAX),
                ),
                linkscope::TraceField::count(
                    "commands",
                    u64::try_from(activation.command_descriptors.len()).unwrap_or(u64::MAX),
                ),
                linkscope::TraceField::count(
                    "ui_panels",
                    u64::try_from(activation.ui_panel_descriptors.len()).unwrap_or(u64::MAX),
                ),
            ],
        );
        let activation_sequence = self.next_activation_sequence;
        self.next_activation_sequence = self.next_activation_sequence.saturating_add(1);
        let hooks = self.activate_hooks(
            activation.hooks,
            plugin_id,
            activation_order,
            activation_sequence,
        );
        let entry = self.entry_mut(key)?;
        entry.finalizers = activation.finalizers;
        entry.hooks = hooks;
        entry.service_descriptors = activation.service_descriptors;
        entry.tool_descriptors = activation.tool_descriptors;
        entry.provider_descriptors = activation.provider_descriptors;
        entry.resource_descriptors = activation.resource_descriptors;
        entry.command_descriptors = activation.command_descriptors;
        entry.ui_slot_descriptors = activation.ui_slot_descriptors;
        entry.ui_panel_descriptors = activation.ui_panel_descriptors;
        entry.ui_widget_descriptors = activation.ui_widget_descriptors;
        entry.runtime_action_descriptors = activation.runtime_action_descriptors;
        entry.runtime_extension_descriptors = activation.runtime_extension_descriptors;
        entry.agent_launch_descriptors = activation.agent_launch_descriptors;
        entry.metric_descriptors = activation.metric_descriptors;
        entry.status = PluginStatusKind::Active;
        entry.activation_sequence = Some(activation_sequence);
        tracing::debug!(target: "jfc::plugin_host", plugin_id = %entry.plugin_id, "plugin activated");
        Ok(())
    }

    fn activate_hooks(
        &mut self,
        hooks: Vec<crate::hook::HookDefinition>,
        plugin_id: PluginId,
        activation_order: i32,
        activation_sequence: u64,
    ) -> Vec<ActivatedHook> {
        hooks
            .into_iter()
            .map(|hook| {
                let hook_sequence = self.next_hook_sequence;
                self.next_hook_sequence = self.next_hook_sequence.saturating_add(1);
                ActivatedHook {
                    plugin_id: plugin_id.clone(),
                    name: hook.name,
                    priority: hook.priority,
                    activation_order,
                    activation_sequence,
                    hook_sequence,
                    callback: hook.callback,
                }
            })
            .collect()
    }

    fn activation_candidates(&self) -> Vec<String> {
        let _linkscope_candidates = linkscope::phase("plugin_host.activation_candidates");
        let mut candidates = self
            .plugins
            .iter()
            .filter(|(_, entry)| entry.status == PluginStatusKind::Registered)
            .map(|(id, entry)| (entry.activation_sort_key(), id.clone()))
            .collect::<Vec<_>>();
        candidates.sort_by(|left, right| left.0.cmp(&right.0));
        candidates.into_iter().map(|(_, id)| id).collect()
    }

    fn finalize_plugin(&mut self, key: &str) -> Result<(), PluginHostError> {
        let _linkscope_finalize = linkscope::phase("plugin_host.finalize_plugin");
        let finalizers = std::mem::take(&mut self.entry_mut(key)?.finalizers);
        let plugin_id = self.entry(key)?.plugin_id.clone();
        let mut first_error = None;
        linkscope::event_fields(
            "plugin_host.finalize_plugin",
            [
                linkscope::TraceField::text("plugin_id", plugin_id.as_str().to_owned()),
                linkscope::TraceField::count(
                    "finalizers",
                    u64::try_from(finalizers.len()).unwrap_or(u64::MAX),
                ),
            ],
        );

        for finalizer in finalizers.into_iter().rev() {
            if let Err(error) = finalizer() {
                let message = error.to_string();
                self.record_error(&plugin_id, PluginErrorPhase::Finalizer, message.clone());
                if first_error.is_none() {
                    first_error = Some(message);
                }
            }
        }

        if let Some(message) = first_error {
            return Err(PluginHostError::FinalizerFailed {
                plugin_id: plugin_id.into_inner(),
                message,
            });
        }

        Ok(())
    }
}
