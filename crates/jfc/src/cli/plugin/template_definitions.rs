use std::path::Path;

mod template_render;

const TEAMMATE_HELPER_TEMPLATE: &str = "teammate-helper";
const UI_DIAGNOSTICS_TEMPLATE: &str = "ui-diagnostics";
const PROMPT_CONTEXT_TEMPLATE: &str = "prompt-context";
const PROCESS_TOOL_TEMPLATE: &str = "process-tool";
const PROCESS_PROVIDER_TEMPLATE: &str = "process-provider";
const TEAMMATE_HELPER_SOURCE: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../jfc-plugin-sdk/examples/teammate_helper_agent.rs"
));
const UI_DIAGNOSTICS_SOURCE: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../jfc-plugin-sdk/examples/ui_diagnostics_panel.rs"
));
const PROMPT_CONTEXT_SOURCE: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../jfc-plugin-sdk/examples/prompt_context_provider.rs"
));
const PROCESS_TOOL_SOURCE: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../jfc-plugin-sdk/examples/process_bridge_tool.rs"
));
const PROCESS_PROVIDER_SOURCE: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../jfc-plugin-sdk/examples/process_bridge_provider.rs"
));

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum PluginTemplate {
    TeammateHelper,
    UiDiagnostics,
    PromptContext,
    ProcessTool,
    ProcessProvider,
}

impl PluginTemplate {
    pub(super) const fn all() -> [Self; 5] {
        [
            Self::TeammateHelper,
            Self::UiDiagnostics,
            Self::PromptContext,
            Self::ProcessTool,
            Self::ProcessProvider,
        ]
    }

    pub(super) fn parse(raw: &str) -> anyhow::Result<Self> {
        match raw.trim() {
            TEAMMATE_HELPER_TEMPLATE | "teammate_helper" | "teammate_helper_agent" => {
                Ok(Self::TeammateHelper)
            }
            UI_DIAGNOSTICS_TEMPLATE | "ui_diagnostics" | "ui_diagnostics_panel" => {
                Ok(Self::UiDiagnostics)
            }
            PROMPT_CONTEXT_TEMPLATE | "prompt_context" | "prompt_context_provider" => {
                Ok(Self::PromptContext)
            }
            PROCESS_TOOL_TEMPLATE | "process_tool" | "process_bridge_tool" | "tool" => {
                Ok(Self::ProcessTool)
            }
            PROCESS_PROVIDER_TEMPLATE
            | "process_provider"
            | "process_bridge_provider"
            | "provider" => Ok(Self::ProcessProvider),
            other => anyhow::bail!(
                "unknown plugin template {other:?}; available templates: {TEAMMATE_HELPER_TEMPLATE}, {UI_DIAGNOSTICS_TEMPLATE}, {PROMPT_CONTEXT_TEMPLATE}, {PROCESS_TOOL_TEMPLATE}, {PROCESS_PROVIDER_TEMPLATE}"
            ),
        }
    }

    pub(super) const fn canonical_name(self) -> &'static str {
        match self {
            Self::TeammateHelper => TEAMMATE_HELPER_TEMPLATE,
            Self::UiDiagnostics => UI_DIAGNOSTICS_TEMPLATE,
            Self::PromptContext => PROMPT_CONTEXT_TEMPLATE,
            Self::ProcessTool => PROCESS_TOOL_TEMPLATE,
            Self::ProcessProvider => PROCESS_PROVIDER_TEMPLATE,
        }
    }

    pub(super) const fn default_plugin_name(self) -> &'static str {
        match self {
            Self::TeammateHelper => "example-teammate-plugin",
            Self::UiDiagnostics => "example-ui-diagnostics-plugin",
            Self::PromptContext => "example-prompt-context-plugin",
            Self::ProcessTool => "example-process-tool-plugin",
            Self::ProcessProvider => "example-process-provider-plugin",
        }
    }

    pub(super) const fn description(self) -> &'static str {
        match self {
            Self::TeammateHelper => "process-bridge teammate launcher with mailbox helpers",
            Self::UiDiagnostics => "refreshable widget plus host-owned panel diagnostics",
            Self::PromptContext => "cached process-bridge prompt-context contributor",
            Self::ProcessTool => "model-visible process-bridge tool descriptor",
            Self::ProcessProvider => "process-bridge provider descriptor and model",
        }
    }

    pub(super) fn manifest(self, dest: &Path, plugin_name: &str) -> anyhow::Result<String> {
        match self {
            Self::TeammateHelper => template_render::teammate_helper_manifest(dest, plugin_name),
            Self::UiDiagnostics => template_render::ui_diagnostics_manifest(dest, plugin_name),
            Self::PromptContext => template_render::prompt_context_manifest(dest, plugin_name),
            Self::ProcessTool => template_render::process_tool_manifest(dest, plugin_name),
            Self::ProcessProvider => template_render::process_provider_manifest(dest, plugin_name),
        }
    }

    pub(super) fn cargo_toml(self) -> String {
        match self {
            Self::TeammateHelper => {
                template_render::cargo_toml("jfc-plugin-template-teammate-helper")
            }
            Self::UiDiagnostics => {
                template_render::cargo_toml("jfc-plugin-template-ui-diagnostics")
            }
            Self::PromptContext => {
                template_render::cargo_toml("jfc-plugin-template-prompt-context")
            }
            Self::ProcessTool => template_render::cargo_toml("jfc-plugin-template-process-tool"),
            Self::ProcessProvider => {
                template_render::cargo_toml("jfc-plugin-template-process-provider")
            }
        }
    }

    pub(super) fn readme(self) -> String {
        match self {
            Self::TeammateHelper => template_render::teammate_helper_readme(),
            Self::UiDiagnostics => template_render::ui_diagnostics_readme(),
            Self::PromptContext => template_render::prompt_context_readme(),
            Self::ProcessTool => template_render::process_tool_readme(),
            Self::ProcessProvider => template_render::process_provider_readme(),
        }
    }

    pub(super) const fn example_file_name(self) -> &'static str {
        match self {
            Self::TeammateHelper => "teammate_helper_agent.rs",
            Self::UiDiagnostics => "ui_diagnostics_panel.rs",
            Self::PromptContext => "prompt_context_provider.rs",
            Self::ProcessTool => "process_bridge_tool.rs",
            Self::ProcessProvider => "process_bridge_provider.rs",
        }
    }

    pub(super) const fn example_source(self) -> &'static str {
        match self {
            Self::TeammateHelper => TEAMMATE_HELPER_SOURCE,
            Self::UiDiagnostics => UI_DIAGNOSTICS_SOURCE,
            Self::PromptContext => PROMPT_CONTEXT_SOURCE,
            Self::ProcessTool => PROCESS_TOOL_SOURCE,
            Self::ProcessProvider => PROCESS_PROVIDER_SOURCE,
        }
    }
}
