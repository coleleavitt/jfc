use jfc_plugin_host::{
    BUILTIN_BACKGROUND_REMINDERS_PROMPT_CONTEXT_ID, BUILTIN_BACKGROUND_REMINDERS_PROMPT_HANDLER,
    BUILTIN_BRIEF_MODE_PROMPT_CONTEXT_ID, BUILTIN_BRIEF_MODE_PROMPT_HANDLER,
    BUILTIN_DOCUMENT_FORMATS_PROMPT_CONTEXT_ID, BUILTIN_DOCUMENT_FORMATS_PROMPT_HANDLER,
    BUILTIN_FEATURE_GATES_PROMPT_CONTEXT_ID, BUILTIN_FEATURE_GATES_PROMPT_HANDLER,
    BUILTIN_HARRIER_PROMPT_CONTEXT_ID, BUILTIN_HARRIER_PROMPT_HANDLER,
    BUILTIN_INTERACTION_MODE_PROMPT_CONTEXT_ID, BUILTIN_INTERACTION_MODE_PROMPT_HANDLER,
    BUILTIN_LOCAL_ADVISOR_PROMPT_CONTEXT_ID, BUILTIN_LOCAL_ADVISOR_PROMPT_HANDLER,
    BUILTIN_MARSH_PROMPT_CONTEXT_ID, BUILTIN_MARSH_PROMPT_HANDLER,
    BUILTIN_OUTPUT_STYLE_PROMPT_CONTEXT_ID, BUILTIN_OUTPUT_STYLE_PROMPT_HANDLER,
    BUILTIN_PEWTER_OWL_PROMPT_CONTEXT_ID, BUILTIN_PEWTER_OWL_PROMPT_HANDLER,
    BUILTIN_PREVIOUS_HANDOFF_PROMPT_CONTEXT_ID, BUILTIN_PREVIOUS_HANDOFF_PROMPT_HANDLER,
    BUILTIN_PROMPT_CONTEXT_PLUGIN_ID, BUILTIN_SERVER_ADVISOR_PROMPT_CONTEXT_ID,
    BUILTIN_SERVER_ADVISOR_PROMPT_HANDLER, BUILTIN_TOTAL_TOKENS_PROMPT_CONTEXT_ID,
    BUILTIN_TOTAL_TOKENS_PROMPT_HANDLER, builtin_prompt_context_plugin_host,
};
use jfc_plugin_sdk::{PluginCapability, RuntimeExtensionExecutorKind, RuntimeExtensionTarget};

#[test]
fn builtin_prompt_context_registers_first_party_contributors_normal() {
    let host = builtin_prompt_context_plugin_host().expect("prompt-context plugin activates");
    let mut extensions = host.runtime_extension_descriptors();
    extensions.sort_by(|left, right| left.id.cmp(&right.id));
    let snapshot = host.status_snapshot();

    assert_eq!(extensions.len(), 13);
    let expected = [
        (
            BUILTIN_BACKGROUND_REMINDERS_PROMPT_CONTEXT_ID,
            BUILTIN_BACKGROUND_REMINDERS_PROMPT_HANDLER,
        ),
        (
            BUILTIN_BRIEF_MODE_PROMPT_CONTEXT_ID,
            BUILTIN_BRIEF_MODE_PROMPT_HANDLER,
        ),
        (
            BUILTIN_FEATURE_GATES_PROMPT_CONTEXT_ID,
            BUILTIN_FEATURE_GATES_PROMPT_HANDLER,
        ),
        (
            BUILTIN_HARRIER_PROMPT_CONTEXT_ID,
            BUILTIN_HARRIER_PROMPT_HANDLER,
        ),
        (
            BUILTIN_INTERACTION_MODE_PROMPT_CONTEXT_ID,
            BUILTIN_INTERACTION_MODE_PROMPT_HANDLER,
        ),
        (
            BUILTIN_LOCAL_ADVISOR_PROMPT_CONTEXT_ID,
            BUILTIN_LOCAL_ADVISOR_PROMPT_HANDLER,
        ),
        (
            BUILTIN_MARSH_PROMPT_CONTEXT_ID,
            BUILTIN_MARSH_PROMPT_HANDLER,
        ),
        (
            BUILTIN_OUTPUT_STYLE_PROMPT_CONTEXT_ID,
            BUILTIN_OUTPUT_STYLE_PROMPT_HANDLER,
        ),
        (
            BUILTIN_PEWTER_OWL_PROMPT_CONTEXT_ID,
            BUILTIN_PEWTER_OWL_PROMPT_HANDLER,
        ),
        (
            BUILTIN_PREVIOUS_HANDOFF_PROMPT_CONTEXT_ID,
            BUILTIN_PREVIOUS_HANDOFF_PROMPT_HANDLER,
        ),
        (
            BUILTIN_DOCUMENT_FORMATS_PROMPT_CONTEXT_ID,
            BUILTIN_DOCUMENT_FORMATS_PROMPT_HANDLER,
        ),
        (
            BUILTIN_SERVER_ADVISOR_PROMPT_CONTEXT_ID,
            BUILTIN_SERVER_ADVISOR_PROMPT_HANDLER,
        ),
        (
            BUILTIN_TOTAL_TOKENS_PROMPT_CONTEXT_ID,
            BUILTIN_TOTAL_TOKENS_PROMPT_HANDLER,
        ),
    ];
    for (extension, (id, handler)) in extensions.iter().zip(expected) {
        assert_eq!(
            extension.plugin_id.as_str(),
            BUILTIN_PROMPT_CONTEXT_PLUGIN_ID
        );
        assert_eq!(extension.target, RuntimeExtensionTarget::PromptContext);
        assert_eq!(extension.id, id);
        assert_eq!(
            extension.executor.kind,
            RuntimeExtensionExecutorKind::BuiltIn
        );
        assert_eq!(extension.executor.handler, handler);
    }
    assert!(snapshot.plugins[0].manifest.capabilities.contains(
        &PluginCapability::RuntimeExtensions {
            targets: vec![RuntimeExtensionTarget::PromptContext],
        }
    ));
}
