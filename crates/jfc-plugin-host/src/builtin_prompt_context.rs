use jfc_plugin_sdk::{
    DescriptorVisibility, PluginCapability, PluginId, PluginManifest, PluginScope, PluginSource,
    PluginVersion, RuntimeExtensionDescriptor, RuntimeExtensionExecutorDescriptor,
    RuntimeExtensionTarget,
};

use crate::{PluginHost, PluginHostError, PluginRegistration};

const PROMPT_CONTEXT_PLUGIN_VERSION: &str = "0.1.0";
pub const BUILTIN_PROMPT_CONTEXT_PLUGIN_ID: &str = "builtin.jfc-prompt-context";
pub const BUILTIN_DOCUMENT_FORMATS_PROMPT_CONTEXT_ID: &str = "context.project-documents";
pub const BUILTIN_DOCUMENT_FORMATS_PROMPT_HANDLER: &str =
    "jfc-engine::document_formats::system_prompt_section";
pub const BUILTIN_FEATURE_GATES_PROMPT_CONTEXT_ID: &str = "context.feature-gates";
pub const BUILTIN_FEATURE_GATES_PROMPT_HANDLER: &str =
    "jfc-engine::feature_gates::system_prompt_section";
pub const BUILTIN_BACKGROUND_REMINDERS_PROMPT_CONTEXT_ID: &str = "context.background-reminders";
pub const BUILTIN_BACKGROUND_REMINDERS_PROMPT_HANDLER: &str =
    "jfc-engine::runtime::background_reminders";
pub const BUILTIN_BRIEF_MODE_PROMPT_CONTEXT_ID: &str = "context.brief-user-messages";
pub const BUILTIN_BRIEF_MODE_PROMPT_HANDLER: &str = "jfc-engine::behavior::brief_user_messages";
pub const BUILTIN_HARRIER_PROMPT_CONTEXT_ID: &str = "context.harrier-investigation";
pub const BUILTIN_HARRIER_PROMPT_HANDLER: &str =
    "jfc-engine::feature_gates::harrier_prompt_section";
pub const BUILTIN_LOCAL_ADVISOR_PROMPT_CONTEXT_ID: &str = "context.local-advisor";
pub const BUILTIN_LOCAL_ADVISOR_PROMPT_HANDLER: &str = "jfc-engine::advisor::local_prompt_section";
pub const BUILTIN_MARSH_PROMPT_CONTEXT_ID: &str = "context.marsh-bash-output";
pub const BUILTIN_MARSH_PROMPT_HANDLER: &str = "jfc-engine::feature_gates::marsh_prompt_section";
pub const BUILTIN_OUTPUT_STYLE_PROMPT_CONTEXT_ID: &str = "context.output-style";
pub const BUILTIN_OUTPUT_STYLE_PROMPT_HANDLER: &str = "jfc-engine::output_style::active_suffix";
pub const BUILTIN_PEWTER_OWL_PROMPT_CONTEXT_ID: &str = "context.pewter-owl-messaging";
pub const BUILTIN_PEWTER_OWL_PROMPT_HANDLER: &str = "jfc-engine::behavior::pewter_owl_messaging";
pub const BUILTIN_PREVIOUS_HANDOFF_PROMPT_CONTEXT_ID: &str = "context.previous-session-handoff";
pub const BUILTIN_PREVIOUS_HANDOFF_PROMPT_HANDLER: &str =
    "jfc-engine::sprint::previous_session_handoff";
pub const BUILTIN_SERVER_ADVISOR_PROMPT_CONTEXT_ID: &str = "context.server-advisor";
pub const BUILTIN_SERVER_ADVISOR_PROMPT_HANDLER: &str =
    "jfc-engine::advisor::server_prompt_section";
pub const BUILTIN_TOTAL_TOKENS_PROMPT_CONTEXT_ID: &str = "context.total-tokens";
pub const BUILTIN_TOTAL_TOKENS_PROMPT_HANDLER: &str =
    "jfc-engine::total_tokens_reminder::render_for_request";
pub const BUILTIN_INTERACTION_MODE_PROMPT_CONTEXT_ID: &str = "context.interaction-mode";
pub const BUILTIN_INTERACTION_MODE_PROMPT_HANDLER: &str =
    "jfc-engine::interaction_mode::prompt_section";

pub fn builtin_prompt_context_plugin_host() -> Result<PluginHost, PluginHostError> {
    let mut host = PluginHost::new();
    host.register_internal(builtin_prompt_context_plugin())?;
    host.activate_all()?;
    Ok(host)
}

pub fn builtin_prompt_context_plugin() -> PluginRegistration {
    let plugin_id = PluginId::new(BUILTIN_PROMPT_CONTEXT_PLUGIN_ID);
    let manifest = PluginManifest::new(
        plugin_id.clone(),
        PluginVersion::new(PROMPT_CONTEXT_PLUGIN_VERSION),
        PluginSource::built_in("jfc-engine"),
    )
    .with_display_name("JFC Prompt Context")
    .with_description("Built-in prompt-context contributors registered through plugin descriptors")
    .with_scope(PluginScope::Workspace)
    .with_capability(PluginCapability::RuntimeExtensions {
        targets: vec![RuntimeExtensionTarget::PromptContext],
    });

    PluginRegistration::new(manifest)
        .with_runtime_extension_descriptor(
            RuntimeExtensionDescriptor::new(
                plugin_id.clone(),
                RuntimeExtensionTarget::PromptContext,
                BUILTIN_MARSH_PROMPT_CONTEXT_ID,
                "Bash subprocess output",
            )
            .with_priority(100)
            .with_visibility(DescriptorVisibility::HostVisible)
            .with_executor(RuntimeExtensionExecutorDescriptor::built_in(
                BUILTIN_MARSH_PROMPT_HANDLER,
            )),
        )
        .with_runtime_extension_descriptor(
            RuntimeExtensionDescriptor::new(
                plugin_id.clone(),
                RuntimeExtensionTarget::PromptContext,
                BUILTIN_FEATURE_GATES_PROMPT_CONTEXT_ID,
                "Feature gates",
            )
            .with_priority(90)
            .with_visibility(DescriptorVisibility::HostVisible)
            .with_executor(RuntimeExtensionExecutorDescriptor::built_in(
                BUILTIN_FEATURE_GATES_PROMPT_HANDLER,
            )),
        )
        .with_runtime_extension_descriptor(
            RuntimeExtensionDescriptor::new(
                plugin_id.clone(),
                RuntimeExtensionTarget::PromptContext,
                BUILTIN_HARRIER_PROMPT_CONTEXT_ID,
                "Investigate before asking",
            )
            .with_priority(85)
            .with_visibility(DescriptorVisibility::HostVisible)
            .with_executor(RuntimeExtensionExecutorDescriptor::built_in(
                BUILTIN_HARRIER_PROMPT_HANDLER,
            )),
        )
        .with_runtime_extension_descriptor(
            RuntimeExtensionDescriptor::new(
                plugin_id.clone(),
                RuntimeExtensionTarget::PromptContext,
                BUILTIN_LOCAL_ADVISOR_PROMPT_CONTEXT_ID,
                "Local Advisor Tool",
            )
            .with_priority(82)
            .with_visibility(DescriptorVisibility::HostVisible)
            .with_executor(RuntimeExtensionExecutorDescriptor::built_in(
                BUILTIN_LOCAL_ADVISOR_PROMPT_HANDLER,
            )),
        )
        .with_runtime_extension_descriptor(
            RuntimeExtensionDescriptor::new(
                plugin_id.clone(),
                RuntimeExtensionTarget::PromptContext,
                BUILTIN_SERVER_ADVISOR_PROMPT_CONTEXT_ID,
                "Server Advisor Tool",
            )
            .with_priority(81)
            .with_visibility(DescriptorVisibility::HostVisible)
            .with_executor(RuntimeExtensionExecutorDescriptor::built_in(
                BUILTIN_SERVER_ADVISOR_PROMPT_HANDLER,
            )),
        )
        .with_runtime_extension_descriptor(
            RuntimeExtensionDescriptor::new(
                plugin_id.clone(),
                RuntimeExtensionTarget::PromptContext,
                BUILTIN_BACKGROUND_REMINDERS_PROMPT_CONTEXT_ID,
                "Background reminders",
            )
            .with_priority(60)
            .with_visibility(DescriptorVisibility::HostVisible)
            .with_executor(RuntimeExtensionExecutorDescriptor::built_in(
                BUILTIN_BACKGROUND_REMINDERS_PROMPT_HANDLER,
            )),
        )
        .with_runtime_extension_descriptor(
            RuntimeExtensionDescriptor::new(
                plugin_id.clone(),
                RuntimeExtensionTarget::PromptContext,
                BUILTIN_TOTAL_TOKENS_PROMPT_CONTEXT_ID,
                "Total tokens",
            )
            .with_priority(55)
            .with_visibility(DescriptorVisibility::HostVisible)
            .with_executor(RuntimeExtensionExecutorDescriptor::built_in(
                BUILTIN_TOTAL_TOKENS_PROMPT_HANDLER,
            )),
        )
        .with_runtime_extension_descriptor(
            RuntimeExtensionDescriptor::new(
                plugin_id.clone(),
                RuntimeExtensionTarget::PromptContext,
                BUILTIN_BRIEF_MODE_PROMPT_CONTEXT_ID,
                "Brief user messages",
            )
            .with_priority(45)
            .with_visibility(DescriptorVisibility::HostVisible)
            .with_executor(RuntimeExtensionExecutorDescriptor::built_in(
                BUILTIN_BRIEF_MODE_PROMPT_HANDLER,
            )),
        )
        .with_runtime_extension_descriptor(
            RuntimeExtensionDescriptor::new(
                plugin_id.clone(),
                RuntimeExtensionTarget::PromptContext,
                BUILTIN_PEWTER_OWL_PROMPT_CONTEXT_ID,
                "Pewter Owl messaging",
            )
            .with_priority(44)
            .with_visibility(DescriptorVisibility::HostVisible)
            .with_executor(RuntimeExtensionExecutorDescriptor::built_in(
                BUILTIN_PEWTER_OWL_PROMPT_HANDLER,
            )),
        )
        .with_runtime_extension_descriptor(
            RuntimeExtensionDescriptor::new(
                plugin_id.clone(),
                RuntimeExtensionTarget::PromptContext,
                BUILTIN_INTERACTION_MODE_PROMPT_CONTEXT_ID,
                "Interaction mode",
            )
            .with_priority(43)
            .with_visibility(DescriptorVisibility::HostVisible)
            .with_executor(RuntimeExtensionExecutorDescriptor::built_in(
                BUILTIN_INTERACTION_MODE_PROMPT_HANDLER,
            )),
        )
        .with_runtime_extension_descriptor(
            RuntimeExtensionDescriptor::new(
                plugin_id.clone(),
                RuntimeExtensionTarget::PromptContext,
                BUILTIN_PREVIOUS_HANDOFF_PROMPT_CONTEXT_ID,
                "Previous Session Handoff",
            )
            .with_priority(75)
            .with_visibility(DescriptorVisibility::HostVisible)
            .with_executor(RuntimeExtensionExecutorDescriptor::built_in(
                BUILTIN_PREVIOUS_HANDOFF_PROMPT_HANDLER,
            )),
        )
        .with_runtime_extension_descriptor(
            RuntimeExtensionDescriptor::new(
                plugin_id.clone(),
                RuntimeExtensionTarget::PromptContext,
                BUILTIN_OUTPUT_STYLE_PROMPT_CONTEXT_ID,
                "Output style",
            )
            .with_priority(80)
            .with_visibility(DescriptorVisibility::HostVisible)
            .with_executor(RuntimeExtensionExecutorDescriptor::built_in(
                BUILTIN_OUTPUT_STYLE_PROMPT_HANDLER,
            )),
        )
        .with_runtime_extension_descriptor(
            RuntimeExtensionDescriptor::new(
                plugin_id,
                RuntimeExtensionTarget::PromptContext,
                BUILTIN_DOCUMENT_FORMATS_PROMPT_CONTEXT_ID,
                "Project documents",
            )
            .with_priority(70)
            .with_visibility(DescriptorVisibility::HostVisible)
            .with_executor(RuntimeExtensionExecutorDescriptor::built_in(
                BUILTIN_DOCUMENT_FORMATS_PROMPT_HANDLER,
            )),
        )
}
