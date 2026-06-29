use std::path::Path;

use jfc_plugin_host::{
    PluginDiscoveryOptions, PluginDiscoverySearchRoot, builtin_prompt_context_plugin_host,
    cached_discovered_resource_plugin_state,
};
use jfc_plugin_sdk::{
    RuntimeExtensionDescriptor, RuntimeExtensionExecutorKind, RuntimeExtensionTarget,
};

use super::prompt_context_bridge::{
    PromptContextBridgeInvocation, refresh_process_bridge_prompt_context,
};
use super::prompt_context_state::{
    PromptContextSnapshot, PromptContextSnapshotStore, now_ms, prompt_context_snapshot_key,
    snapshot_body, snapshot_is_fresh,
};
use super::runtime_prompt_context_builtins::{
    BuiltinPromptContextState, builtin_prompt_context_body,
};

const MAX_PROMPT_CONTEXT_CHARS: usize = 12_000;

pub(super) async fn append_prompt_context_extensions(
    system_prompt: &mut String,
    cwd: &Path,
    builtins: &BuiltinPromptContextState<'_>,
) {
    let mut extensions = prompt_context_extensions(cwd);
    let mut snapshots = PromptContextSnapshotStore::open(cwd);
    extensions.sort_by(|left, right| {
        right
            .priority
            .cmp(&left.priority)
            .then_with(|| left.plugin_id.as_str().cmp(right.plugin_id.as_str()))
            .then_with(|| left.id.cmp(&right.id))
    });

    for extension in extensions {
        match extension.executor.kind {
            RuntimeExtensionExecutorKind::BuiltIn => {
                let system_prompt_tokens = system_prompt.len() / 4;
                let Some(body) =
                    builtin_prompt_context_body(&extension, cwd, builtins, system_prompt_tokens)
                else {
                    continue;
                };
                append_prompt_context(system_prompt, &extension.label, &body);
            }
            RuntimeExtensionExecutorKind::StaticText => {
                let body = extension.executor.handler.trim();
                if body.is_empty() {
                    continue;
                }
                append_prompt_context(system_prompt, &extension.label, body);
            }
            RuntimeExtensionExecutorKind::ProcessBridge => {
                let key = prompt_context_snapshot_key(&extension);
                let snapshot = snapshots.get(&key).cloned();
                let timestamp_ms = now_ms();
                if snapshot_is_fresh(&extension, snapshot.as_ref(), timestamp_ms) {
                    if let Some(snapshot) = snapshot.as_ref()
                        && let Some(body) = snapshot_body(snapshot)
                    {
                        append_prompt_context(system_prompt, &extension.label, body);
                    }
                    continue;
                }
                let invocation = PromptContextBridgeInvocation {
                    extension: &extension,
                    cwd,
                    state: snapshot
                        .as_ref()
                        .and_then(|snapshot| snapshot.state.clone()),
                    max_chars: MAX_PROMPT_CONTEXT_CHARS,
                };
                match refresh_process_bridge_prompt_context(invocation).await {
                    Ok(result) => {
                        let snapshot = PromptContextSnapshot::from_refresh_result(
                            result,
                            timestamp_ms,
                            MAX_PROMPT_CONTEXT_CHARS,
                        );
                        if let Some(body) = snapshot_body(&snapshot) {
                            append_prompt_context(system_prompt, &extension.label, body);
                        }
                        snapshots.insert(key, snapshot);
                    }
                    Err(error) => tracing::warn!(
                        target: "jfc::plugin_runtime",
                        plugin_id = %extension.plugin_id,
                        extension_id = %extension.id,
                        error = %error,
                        "prompt-context process bridge failed"
                    ),
                }
            }
        }
    }
    if let Err(error) = snapshots.save_if_changed() {
        tracing::warn!(
            target: "jfc::plugin_runtime",
            error = %error,
            "failed to save prompt-context snapshots"
        );
    }
}

fn prompt_context_extensions(cwd: &Path) -> Vec<RuntimeExtensionDescriptor> {
    let mut extensions = builtin_prompt_context_plugin_host()
        .map(|host| host.runtime_extension_descriptors())
        .unwrap_or_else(|error| {
            tracing::warn!(
                target: "jfc::plugin_runtime",
                error = %error,
                "failed to activate built-in prompt-context plugin"
            );
            Vec::new()
        });
    let mut options = PluginDiscoveryOptions::new().with_search_root(
        PluginDiscoverySearchRoot::project_plugins_dir(cwd.join(".jfc/plugins")),
    );
    if let Some(config_dir) = dirs::config_dir() {
        options = options.with_search_root(PluginDiscoverySearchRoot::global_plugins_dir(
            config_dir.join("jfc/plugins"),
        ));
    }

    if let Ok(state) = cached_discovered_resource_plugin_state(options) {
        extensions.extend(state.host.runtime_extension_descriptors());
    }
    extensions
        .into_iter()
        .filter(|descriptor| descriptor.target == RuntimeExtensionTarget::PromptContext)
        .collect()
}

fn append_prompt_context(system_prompt: &mut String, label: &str, body: &str) {
    system_prompt.push_str("\n\n## Plugin Prompt Context: ");
    system_prompt.push_str(label);
    system_prompt.push('\n');
    system_prompt.push_str(body);
}
