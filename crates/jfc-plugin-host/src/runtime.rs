use std::collections::{BTreeMap, HashMap};

use jfc_plugin_sdk::{
    CommandDescriptor, DescriptorVisibility, ExtensionSlot, PluginId, PluginSource,
    ProviderDescriptor, RuntimeActionDescriptor, ToolDescriptor, UiMutationScope, UiSlotDescriptor,
    UiWidgetDescriptor,
};

use crate::{PluginHost, PluginHostError, PluginStatusKind, host::PluginEntry};

pub type UiSlotKey = (ExtensionSlot, String);
pub type UiWidgetRuntimeKey = (PluginId, UiMutationScope, String);

#[derive(Debug, Clone)]
pub struct RuntimeDescriptor<T> {
    plugin_id: PluginId,
    source: PluginSource,
    descriptor: T,
}

impl<T> RuntimeDescriptor<T> {
    pub fn new(plugin_id: PluginId, source: PluginSource, descriptor: T) -> Self {
        Self {
            plugin_id,
            source,
            descriptor,
        }
    }

    pub fn plugin_id(&self) -> &PluginId {
        &self.plugin_id
    }

    pub fn source(&self) -> &PluginSource {
        &self.source
    }

    pub fn descriptor(&self) -> &T {
        &self.descriptor
    }

    pub fn into_descriptor(self) -> T {
        self.descriptor
    }
}

#[derive(Debug, Clone, Default)]
pub struct PluginRuntime {
    tools: BTreeMap<String, RuntimeDescriptor<ToolDescriptor>>,
    commands: BTreeMap<String, RuntimeDescriptor<CommandDescriptor>>,
    providers: BTreeMap<String, RuntimeDescriptor<ProviderDescriptor>>,
    ui_slots: HashMap<UiSlotKey, RuntimeDescriptor<UiSlotDescriptor>>,
    pub(crate) ui_widgets: HashMap<UiWidgetRuntimeKey, RuntimeDescriptor<UiWidgetDescriptor>>,
    runtime_actions: BTreeMap<String, RuntimeDescriptor<RuntimeActionDescriptor>>,
}

impl PluginRuntime {
    pub fn from_host(host: &PluginHost) -> Result<Self, PluginHostError> {
        let mut runtime = Self::default();

        let mut entries = host
            .plugins
            .values()
            .filter(|entry| entry.status == PluginStatusKind::Active)
            .collect::<Vec<_>>();
        entries.sort_by_key(|entry| entry.activation_sort_key());

        for entry in entries {
            runtime.register_entry(entry)?;
        }

        Ok(runtime)
    }

    pub fn from_ui_widget_descriptors<I>(descriptors: I) -> Result<Self, PluginHostError>
    where
        I: IntoIterator<Item = UiWidgetDescriptor>,
    {
        let mut runtime = Self::default();
        for descriptor in descriptors {
            runtime.register_ui_widget_descriptor(
                descriptor.plugin_id.clone(),
                PluginSource::built_in("ui-widget-descriptor"),
                descriptor,
            )?;
        }
        Ok(runtime)
    }

    pub fn tools(&self) -> &BTreeMap<String, RuntimeDescriptor<ToolDescriptor>> {
        &self.tools
    }

    pub fn commands(&self) -> &BTreeMap<String, RuntimeDescriptor<CommandDescriptor>> {
        &self.commands
    }

    pub fn providers(&self) -> &BTreeMap<String, RuntimeDescriptor<ProviderDescriptor>> {
        &self.providers
    }

    pub fn ui_slots(&self) -> &HashMap<UiSlotKey, RuntimeDescriptor<UiSlotDescriptor>> {
        &self.ui_slots
    }

    pub fn ui_widgets(
        &self,
    ) -> &HashMap<UiWidgetRuntimeKey, RuntimeDescriptor<UiWidgetDescriptor>> {
        &self.ui_widgets
    }

    pub fn runtime_actions(&self) -> &BTreeMap<String, RuntimeDescriptor<RuntimeActionDescriptor>> {
        &self.runtime_actions
    }

    fn register_entry(&mut self, entry: &PluginEntry) -> Result<(), PluginHostError> {
        let plugin_id = entry.plugin_id.clone();
        let manifest_source = entry.plugin.manifest().source.clone();

        for descriptor in &entry.tool_descriptors {
            if descriptor.visibility == DescriptorVisibility::Internal {
                continue;
            }
            insert_descriptor(
                &mut self.tools,
                "tool",
                descriptor.name.clone(),
                RuntimeDescriptor::new(
                    plugin_id.clone(),
                    manifest_source.clone(),
                    descriptor.clone(),
                ),
            )?;
        }

        for descriptor in &entry.command_descriptors {
            let source = descriptor
                .source
                .clone()
                .unwrap_or_else(|| manifest_source.clone());
            insert_descriptor(
                &mut self.commands,
                "command",
                descriptor.name.clone(),
                RuntimeDescriptor::new(plugin_id.clone(), source, descriptor.clone()),
            )?;
        }

        for descriptor in &entry.provider_descriptors {
            if descriptor.visibility == DescriptorVisibility::Internal {
                continue;
            }
            insert_descriptor(
                &mut self.providers,
                "provider",
                descriptor.provider.clone(),
                RuntimeDescriptor::new(
                    plugin_id.clone(),
                    manifest_source.clone(),
                    descriptor.clone(),
                ),
            )?;
        }

        for descriptor in &entry.ui_slot_descriptors {
            if descriptor.visibility == DescriptorVisibility::Internal {
                continue;
            }
            let key = (descriptor.slot, descriptor.id.clone());
            insert_hash_descriptor(
                &mut self.ui_slots,
                "ui_slot",
                format!("{:?}:{}", key.0, key.1),
                key,
                RuntimeDescriptor::new(
                    plugin_id.clone(),
                    manifest_source.clone(),
                    descriptor.clone(),
                ),
            )?;
        }

        for descriptor in &entry.ui_widget_descriptors {
            self.register_ui_widget_descriptor(
                plugin_id.clone(),
                manifest_source.clone(),
                descriptor.clone(),
            )?;
        }

        for descriptor in &entry.runtime_action_descriptors {
            if descriptor.visibility == DescriptorVisibility::Internal {
                continue;
            }
            insert_descriptor(
                &mut self.runtime_actions,
                "runtime_action",
                descriptor.id.clone(),
                RuntimeDescriptor::new(
                    plugin_id.clone(),
                    manifest_source.clone(),
                    descriptor.clone(),
                ),
            )?;
        }

        Ok(())
    }

    fn register_ui_widget_descriptor(
        &mut self,
        plugin_id: PluginId,
        source: PluginSource,
        descriptor: UiWidgetDescriptor,
    ) -> Result<(), PluginHostError> {
        if descriptor.visibility == DescriptorVisibility::Internal {
            return Ok(());
        }
        let key = (
            descriptor.plugin_id.clone(),
            descriptor.scope,
            descriptor.id.clone(),
        );
        insert_hash_descriptor(
            &mut self.ui_widgets,
            "ui_widget",
            format!("{}:{:?}:{}", key.0.as_str(), key.1, key.2),
            key,
            RuntimeDescriptor::new(plugin_id, source, descriptor),
        )
    }
}

fn insert_descriptor<K, T>(
    descriptors: &mut BTreeMap<K, RuntimeDescriptor<T>>,
    descriptor_kind: &'static str,
    descriptor_id: K,
    descriptor: RuntimeDescriptor<T>,
) -> Result<(), PluginHostError>
where
    K: Clone + Ord + ToString,
{
    if let Some(existing) = descriptors.get(&descriptor_id) {
        return Err(PluginHostError::DuplicateDescriptorId {
            descriptor_kind: descriptor_kind.to_owned(),
            descriptor_id: descriptor_id.to_string(),
            first_plugin_id: existing.plugin_id.as_str().to_owned(),
            duplicate_plugin_id: descriptor.plugin_id.as_str().to_owned(),
        });
    }

    descriptors.insert(descriptor_id, descriptor);
    Ok(())
}

fn insert_hash_descriptor<K, T>(
    descriptors: &mut HashMap<K, RuntimeDescriptor<T>>,
    descriptor_kind: &'static str,
    descriptor_id_label: String,
    descriptor_id: K,
    descriptor: RuntimeDescriptor<T>,
) -> Result<(), PluginHostError>
where
    K: Eq + std::hash::Hash,
{
    if let Some(existing) = descriptors.get(&descriptor_id) {
        return Err(PluginHostError::DuplicateDescriptorId {
            descriptor_kind: descriptor_kind.to_owned(),
            descriptor_id: descriptor_id_label,
            first_plugin_id: existing.plugin_id.as_str().to_owned(),
            duplicate_plugin_id: descriptor.plugin_id.as_str().to_owned(),
        });
    }

    descriptors.insert(descriptor_id, descriptor);
    Ok(())
}
