use std::collections::BTreeSet;

use jfc_plugin_sdk::{
    DescriptorVisibility, ExtensionSlot, PluginCapability, PluginId, PluginManifest, PluginScope,
    PluginSource, PluginVersion, RuntimeActionKind, RuntimeExtensionDescriptor,
    RuntimeExtensionExecutorDescriptor, RuntimeExtensionTarget, UiSlotDescriptor,
};

use crate::builtin_palette::{
    command_palette_runtime_action_descriptors, command_palette_slot_descriptors,
};
use crate::{PluginHost, PluginHostError, PluginRegistration};

const UX_PLUGIN_VERSION: &str = "0.1.0";
const STATUS_LINE_PLUGIN_ID: &str = "builtin.jfc-status-line";
pub const BUILTIN_GOAL_STATUS_SLOT_ID: &str = "goal.elapsed";
pub const BUILTIN_PLUGIN_HEALTH_SLOT_ID: &str = "plugin.health";
pub const BUILTIN_MESSAGE_RENDERER_SLOT_ID: &str = "message_renderer.markdown";

const UX_CAPABILITIES: &[BuiltinUxCapability] = &[
    BuiltinUxCapability {
        crate_name: "jfc-design",
        plugin_id: "builtin.jfc-design",
        display_name: "JFC Design",
        description: "Built-in design artifact, export, preview, and design-system product descriptors",
        capability: PluginCapability::Design,
        optional_stale_reference: false,
    },
    BuiltinUxCapability {
        crate_name: "jfc-voice",
        plugin_id: "builtin.jfc-voice",
        display_name: "JFC Voice",
        description: "Built-in voice capture, VAD, speaker gate, and realtime audio product descriptors",
        capability: PluginCapability::Voice,
        optional_stale_reference: true,
    },
    BuiltinUxCapability {
        crate_name: "jfc-markdown",
        plugin_id: "builtin.jfc-markdown",
        display_name: "JFC Markdown",
        description: "Built-in markdown rendering and syntax-highlight frontend support descriptors",
        capability: PluginCapability::FrontendSupport,
        optional_stale_reference: false,
    },
    BuiltinUxCapability {
        crate_name: "jfc-theme",
        plugin_id: "builtin.jfc-theme",
        display_name: "JFC Theme",
        description: "Built-in terminal theme and palette frontend support descriptors",
        capability: PluginCapability::FrontendSupport,
        optional_stale_reference: false,
    },
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuiltinUxRegistrationReport {
    pub registered_crates: Vec<&'static str>,
    pub missing_optional_crates: Vec<&'static str>,
}

pub fn builtin_ux_plugin_host<I, S>(
    workspace_members: I,
) -> Result<(PluginHost, BuiltinUxRegistrationReport), PluginHostError>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let mut host = PluginHost::new();
    let report = register_builtin_ux_plugins(&mut host, workspace_members)?;
    host.activate_all()?;
    Ok((host, report))
}

pub fn builtin_status_line_plugin_host() -> Result<PluginHost, PluginHostError> {
    let mut host = PluginHost::new();
    host.register_internal(status_line_plugin_registration())?;
    host.activate_all()?;
    Ok(host)
}

pub fn register_builtin_ux_plugins<I, S>(
    host: &mut PluginHost,
    workspace_members: I,
) -> Result<BuiltinUxRegistrationReport, PluginHostError>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let workspace_members = workspace_members
        .into_iter()
        .map(|member| member.as_ref().to_owned())
        .collect::<BTreeSet<_>>();
    let mut registered_crates = Vec::new();
    let mut missing_optional_crates = Vec::new();

    for capability in UX_CAPABILITIES {
        if workspace_members.contains(capability.crate_name) {
            host.register_internal(capability.plugin_registration())?;
            registered_crates.push(capability.crate_name);
        } else if capability.optional_stale_reference {
            missing_optional_crates.push(capability.crate_name);
        }
    }

    Ok(BuiltinUxRegistrationReport {
        registered_crates,
        missing_optional_crates,
    })
}

struct BuiltinUxCapability {
    crate_name: &'static str,
    plugin_id: &'static str,
    display_name: &'static str,
    description: &'static str,
    capability: PluginCapability,
    optional_stale_reference: bool,
}

impl BuiltinUxCapability {
    fn plugin_registration(&self) -> PluginRegistration {
        let manifest = PluginManifest::new(
            PluginId::new(self.plugin_id),
            PluginVersion::new(UX_PLUGIN_VERSION),
            PluginSource::built_in(self.crate_name),
        )
        .with_display_name(self.display_name)
        .with_description(self.description)
        .with_scope(PluginScope::Workspace)
        .with_capability(self.capability.clone());

        PluginRegistration::new(manifest)
    }
}

fn status_line_plugin_registration() -> PluginRegistration {
    let plugin_id = PluginId::new(STATUS_LINE_PLUGIN_ID);
    let manifest = PluginManifest::new(
        plugin_id.clone(),
        PluginVersion::new(UX_PLUGIN_VERSION),
        PluginSource::built_in("jfc"),
    )
    .with_display_name("JFC Status Line")
    .with_description("Built-in UI slots for status, command palette, and message rendering")
    .with_scope(PluginScope::Workspace)
    .with_capability(PluginCapability::UiSlots {
        slots: vec![
            ExtensionSlot::StatusLine,
            ExtensionSlot::CommandPalette,
            ExtensionSlot::MessageRenderer,
        ],
    })
    .with_capability(PluginCapability::RuntimeExtensions {
        targets: vec![RuntimeExtensionTarget::MessageRenderer],
    })
    .with_capability(PluginCapability::RuntimeActions {
        actions: vec![
            RuntimeActionKind::HostAction,
            RuntimeActionKind::SlashCommand,
            RuntimeActionKind::PluginDiagnostics,
        ],
    });

    let mut slots = vec![
        UiSlotDescriptor::new(
            plugin_id.clone(),
            ExtensionSlot::StatusLine,
            BUILTIN_GOAL_STATUS_SLOT_ID,
            "Goal elapsed time",
        )
        .with_priority(92)
        .with_visibility(DescriptorVisibility::ModelVisible),
        UiSlotDescriptor::new(
            plugin_id.clone(),
            ExtensionSlot::StatusLine,
            BUILTIN_PLUGIN_HEALTH_SLOT_ID,
            "Plugin health",
        )
        .with_priority(63)
        .with_visibility(DescriptorVisibility::HostVisible),
    ];
    slots.extend(command_palette_slot_descriptors(plugin_id.clone()));
    slots.push(
        UiSlotDescriptor::new(
            plugin_id,
            ExtensionSlot::MessageRenderer,
            BUILTIN_MESSAGE_RENDERER_SLOT_ID,
            "Markdown message renderer",
        )
        .with_priority(100)
        .with_visibility(DescriptorVisibility::HostVisible),
    );

    PluginRegistration::new(manifest)
        .with_ui_slot_descriptors(slots)
        .with_runtime_action_descriptors(command_palette_runtime_action_descriptors(PluginId::new(
            STATUS_LINE_PLUGIN_ID,
        )))
        .with_runtime_extension_descriptor(
            RuntimeExtensionDescriptor::new(
                PluginId::new(STATUS_LINE_PLUGIN_ID),
                RuntimeExtensionTarget::MessageRenderer,
                BUILTIN_MESSAGE_RENDERER_SLOT_ID,
                "Markdown message renderer",
            )
            .with_priority(100)
            .with_visibility(DescriptorVisibility::HostVisible)
            .with_executor(RuntimeExtensionExecutorDescriptor::built_in(
                "jfc-markdown::message_renderer",
            )),
        )
}
