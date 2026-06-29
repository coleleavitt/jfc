use std::collections::HashSet;

use jfc_plugin_sdk::RuntimeExtensionDescriptor;

pub(crate) fn builtin_runtime_extension_descriptors(
    status_host: &jfc_plugin_host::PluginHost,
) -> Vec<RuntimeExtensionDescriptor> {
    let mut descriptors = status_host.runtime_extension_descriptors();
    match jfc_plugin_host::builtin_prompt_context_plugin_host() {
        Ok(host) => descriptors.extend(host.runtime_extension_descriptors()),
        Err(error) => tracing::warn!(
            target: "jfc::plugins",
            error = %error,
            "failed to activate built-in prompt-context plugin"
        ),
    }
    descriptors
}

pub(crate) fn append_runtime_extension_descriptors(
    extensions: &mut Vec<RuntimeExtensionDescriptor>,
    extra: Vec<RuntimeExtensionDescriptor>,
) {
    let mut seen = extensions
        .iter()
        .map(runtime_extension_key)
        .collect::<HashSet<(String, jfc_plugin_sdk::RuntimeExtensionTarget, String)>>();
    for extension in extra {
        if seen.insert(runtime_extension_key(&extension)) {
            extensions.push(extension);
        }
    }
}

fn runtime_extension_key(
    extension: &RuntimeExtensionDescriptor,
) -> (String, jfc_plugin_sdk::RuntimeExtensionTarget, String) {
    (
        extension.plugin_id.as_str().to_owned(),
        extension.target,
        extension.id.clone(),
    )
}
