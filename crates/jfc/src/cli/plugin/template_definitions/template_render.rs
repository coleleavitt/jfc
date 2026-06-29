use std::path::{Path, PathBuf};

use jfc_plugin_sdk::ProcessBridgeCommand;

use super::PluginTemplate;

pub(super) fn teammate_helper_manifest(dest: &Path, plugin_name: &str) -> anyhow::Result<String> {
    let handler_json = template_handler(dest, "teammate_helper_agent")?;
    Ok(format!(
        "[plugin]\nname = \"{}\"\n\n[[agent_launches]]\nname = \"helper-agent\"\nlabel = \"Helper Agent\"\ndescription = \"Mailbox-aware process-bridge teammate.\"\n\n[agent_launches.executor]\nkind = \"process_bridge\"\nhandler = '{}'\n",
        plugin_name, handler_json
    ))
}

pub(super) fn ui_diagnostics_manifest(dest: &Path, plugin_name: &str) -> anyhow::Result<String> {
    let handler_json = template_handler(dest, "ui_diagnostics_panel")?;
    Ok(format!(
        "[plugin]\nname = \"{}\"\n\n[[runtime_actions]]\nid = \"diagnostics.refresh\"\nlabel = \"Refresh Diagnostics\"\ndescription = \"Refresh plugin diagnostics descriptors.\"\nkind = \"refresh_metrics\"\npriority = 20\n\n[[ui_panels]]\nscope = \"info_sidebar\"\nid = \"diagnostics.summary\"\ntitle = \"Diagnostics Summary\"\nbody = \"not refreshed yet\"\nruntime_action_id = \"diagnostics.refresh\"\nrefresh = {{ kind = \"process_bridge\", handler = '{}', min_interval_ms = 1000, auto_refresh_ms = 60000 }}\npriority = 50\n\n[[ui_widgets]]\nscope = \"info_sidebar\"\nid = \"diagnostics.counter\"\nlabel = \"Refresh Counter\"\nkind = \"text\"\nbody = \"not refreshed yet\"\nruntime_action_id = \"diagnostics.refresh\"\nrefresh = {{ kind = \"process_bridge\", handler = '{}', min_interval_ms = 1000, auto_refresh_ms = 60000 }}\npriority = 40\n",
        plugin_name, handler_json, handler_json
    ))
}

pub(super) fn prompt_context_manifest(dest: &Path, plugin_name: &str) -> anyhow::Result<String> {
    let handler_json = template_handler(dest, "prompt_context_provider")?;
    Ok(format!(
        "[plugin]\nname = \"{}\"\n\n[[runtime_extensions]]\ntarget = \"prompt_context\"\nid = \"context.cached-note\"\nlabel = \"Cached Note\"\npriority = 60\nrefresh = {{ kind = \"process_bridge\", min_interval_ms = 1000, auto_refresh_ms = 60000 }}\n\n[runtime_extensions.executor]\nkind = \"process_bridge\"\nhandler = '{}'\n",
        plugin_name, handler_json
    ))
}

pub(super) fn process_tool_manifest(dest: &Path, plugin_name: &str) -> anyhow::Result<String> {
    let handler_json = template_handler(dest, "process_bridge_tool")?;
    Ok(format!(
        "[plugin]\nname = \"{}\"\n\n[[runtime_actions]]\nid = \"plugin.smoke\"\nlabel = \"Smoke Plugin\"\ndescription = \"Run process-bridge smoke checks for this plugin.\"\nkind = \"plugin_smoke\"\npriority = 30\npayload = {{ plugin = \"{}\" }}\n\n[[ui_slots]]\nslot = \"command_palette\"\nid = \"plugin.smoke\"\nlabel = \"Smoke Plugin\"\npriority = 30\n\n[[tools]]\nname = \"external_echo\"\ndescription = \"External Echo\"\nvisibility = \"model_visible\"\napproval_policy = \"read_only\"\ninput_schema = {{ type = \"object\", properties = {{ message = {{ type = \"string\", description = \"Message to echo.\" }} }}, required = [\"message\"], additionalProperties = false }}\n\n[tools.executor]\nkind = \"process_bridge\"\nhandler = '{}'\n",
        plugin_name, plugin_name, handler_json
    ))
}

pub(super) fn process_provider_manifest(dest: &Path, plugin_name: &str) -> anyhow::Result<String> {
    let handler_json = template_handler(dest, "process_bridge_provider")?;
    Ok(format!(
        "[plugin]\nname = \"{}\"\n\n[[runtime_actions]]\nid = \"plugin.smoke\"\nlabel = \"Smoke Plugin\"\ndescription = \"Run process-bridge smoke checks for this plugin.\"\nkind = \"plugin_smoke\"\npriority = 30\npayload = {{ plugin = \"{}\" }}\n\n[[ui_slots]]\nslot = \"command_palette\"\nid = \"plugin.smoke\"\nlabel = \"Smoke Plugin\"\npriority = 30\n\n[[providers]]\nprovider = \"external-demo\"\nvisibility = \"host_visible\"\nmodels = [{{ id = \"external-demo-chat\", display_name = \"External Demo Chat\", context_window_tokens = 8192, max_output_tokens = 1024 }}]\n\n[providers.executor]\nkind = \"process_bridge\"\nhandler = '{}'\n",
        plugin_name, plugin_name, handler_json
    ))
}

pub(super) fn cargo_toml(package_name: &str) -> String {
    let sdk_path = sdk_crate_path();
    format!(
        "[package]\nname = \"{package_name}\"\nversion = \"0.1.0\"\nedition = \"2024\"\n\n[dependencies]\njfc-plugin-sdk = {{ path = \"{}\" }}\nserde_json = {{ version = \"1.0.144\", features = [\"preserve_order\", \"raw_value\"] }}\n",
        sdk_path.display()
    )
}

pub(super) fn teammate_helper_readme() -> String {
    format!(
        "# JFC Teammate Helper Plugin\n\nInstalled from the `{}` first-party SDK template.\n\nThe `helper-agent` launcher runs `examples/teammate_helper_agent.rs` as a process-bridge teammate. It can poll the host mailbox, send a mailbox reply, declare ready/idle state, and emit teammate lifecycle events without touching JFC internals.\n",
        PluginTemplate::TeammateHelper.canonical_name()
    )
}

pub(super) fn ui_diagnostics_readme() -> String {
    format!(
        "# JFC UI Diagnostics Plugin\n\nInstalled from the `{}` first-party SDK template.\n\nThis plugin contributes a refreshable host-owned info-sidebar panel, a refreshable info-sidebar widget, and a runtime action descriptor. The panel and widget run `examples/ui_diagnostics_panel.rs` as a process-bridge refresh handler and persist their counters in host-owned snapshot state.\n",
        PluginTemplate::UiDiagnostics.canonical_name()
    )
}

pub(super) fn prompt_context_readme() -> String {
    format!(
        "# JFC Prompt Context Plugin\n\nInstalled from the `{}` first-party SDK template.\n\nThis plugin contributes a cached process-bridge prompt-context runtime extension. JFC owns the cadence, persists returned state, and passes that state back on the next refresh so the plugin can evolve context without touching engine internals.\n",
        PluginTemplate::PromptContext.canonical_name()
    )
}

pub(super) fn process_tool_readme() -> String {
    format!(
        "# JFC Process Tool Plugin\n\nInstalled from the `{}` first-party SDK template.\n\nThis plugin contributes the model-visible `external_echo` tool plus a `Smoke Plugin` command-palette action. JFC owns descriptor discovery, tool approval policy, and the smoke runner; the example binary only receives `tool_call` frames and returns `tool_result` frames over the process-bridge JSONL ABI.\n",
        PluginTemplate::ProcessTool.canonical_name()
    )
}

pub(super) fn process_provider_readme() -> String {
    format!(
        "# JFC Process Provider Plugin\n\nInstalled from the `{}` first-party SDK template.\n\nThis plugin contributes the `external-demo` provider with one model plus a `Smoke Plugin` command-palette action. JFC owns provider registration and the smoke runner; the example binary receives `provider_stream` frames and emits provider stream events over the process-bridge JSONL ABI.\n",
        PluginTemplate::ProcessProvider.canonical_name()
    )
}

fn template_handler(dest: &Path, example_name: &str) -> anyhow::Result<String> {
    let manifest_path = dest.join("Cargo.toml");
    let handler = ProcessBridgeCommand::new("cargo").with_args([
        "run".to_owned(),
        "--manifest-path".to_owned(),
        manifest_path.to_string_lossy().into_owned(),
        "--example".to_owned(),
        example_name.to_owned(),
        "--quiet".to_owned(),
    ]);
    Ok(serde_json::to_string(&handler)?)
}

fn sdk_crate_path() -> PathBuf {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../jfc-plugin-sdk");
    std::fs::canonicalize(&path).unwrap_or(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn teammate_helper_manifest_uses_absolute_manifest_path_normal() {
        let root = PathBuf::from("/tmp/demo-helper");

        let manifest = teammate_helper_manifest(&root, "demo-helper").expect("manifest");

        assert!(manifest.contains("name = \"demo-helper\""));
        assert!(manifest.contains("helper-agent"));
        assert!(manifest.contains("/tmp/demo-helper/Cargo.toml"));
        assert!(manifest.contains("teammate_helper_agent"));
    }

    #[test]
    fn ui_diagnostics_manifest_uses_absolute_manifest_path_normal() {
        let root = PathBuf::from("/tmp/demo-ui");

        let manifest = ui_diagnostics_manifest(&root, "demo-ui").expect("manifest");

        assert!(manifest.contains("name = \"demo-ui\""));
        assert!(manifest.contains("[[ui_panels]]"));
        assert!(manifest.contains("[[ui_widgets]]"));
        assert!(manifest.contains("min_interval_ms = 1000"));
        assert!(manifest.contains("auto_refresh_ms = 60000"));
        assert!(manifest.contains("/tmp/demo-ui/Cargo.toml"));
        assert!(manifest.contains("ui_diagnostics_panel"));
    }

    #[test]
    fn prompt_context_manifest_uses_absolute_manifest_path_normal() {
        let root = PathBuf::from("/tmp/demo-prompt");

        let manifest = prompt_context_manifest(&root, "demo-prompt").expect("manifest");

        assert!(manifest.contains("name = \"demo-prompt\""));
        assert!(manifest.contains("[[runtime_extensions]]"));
        assert!(manifest.contains("target = \"prompt_context\""));
        assert!(manifest.contains("min_interval_ms = 1000"));
        assert!(manifest.contains("auto_refresh_ms = 60000"));
        assert!(manifest.contains("/tmp/demo-prompt/Cargo.toml"));
        assert!(manifest.contains("prompt_context_provider"));
    }

    #[test]
    fn process_tool_manifest_uses_absolute_manifest_path_normal() {
        let root = PathBuf::from("/tmp/demo-tool");

        let manifest = process_tool_manifest(&root, "demo-tool").expect("manifest");

        assert!(manifest.contains("name = \"demo-tool\""));
        assert!(manifest.contains("[[tools]]"));
        assert!(manifest.contains("[[runtime_actions]]"));
        assert!(manifest.contains("kind = \"plugin_smoke\""));
        assert!(manifest.contains("[[ui_slots]]"));
        assert!(manifest.contains("visibility = \"model_visible\""));
        assert!(manifest.contains("/tmp/demo-tool/Cargo.toml"));
        assert!(manifest.contains("process_bridge_tool"));
    }

    #[test]
    fn process_provider_manifest_uses_absolute_manifest_path_normal() {
        let root = PathBuf::from("/tmp/demo-provider");

        let manifest = process_provider_manifest(&root, "demo-provider").expect("manifest");

        assert!(manifest.contains("name = \"demo-provider\""));
        assert!(manifest.contains("[[providers]]"));
        assert!(manifest.contains("[[runtime_actions]]"));
        assert!(manifest.contains("kind = \"plugin_smoke\""));
        assert!(manifest.contains("[[ui_slots]]"));
        assert!(manifest.contains("provider = \"external-demo\""));
        assert!(manifest.contains("/tmp/demo-provider/Cargo.toml"));
        assert!(manifest.contains("process_bridge_provider"));
    }
}
