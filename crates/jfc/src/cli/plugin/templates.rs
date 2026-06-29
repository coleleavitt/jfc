use std::path::PathBuf;

use super::store::{plugins_root, prepare_dest, sanitize_plugin_name};
use super::template_definitions::PluginTemplate;

pub(super) fn install_plugin_template(
    template: &str,
    name: Option<&str>,
    force: bool,
) -> anyhow::Result<PathBuf> {
    let template = PluginTemplate::parse(template)?;
    let plugin_name = match name {
        Some(name) => sanitize_plugin_name(name)?,
        None => template.default_plugin_name().to_owned(),
    };
    let dest = plugins_root()?.join(&plugin_name);
    prepare_dest(&dest, force)?;
    std::fs::create_dir_all(dest.join("examples"))?;
    std::fs::write(
        dest.join(".jfc-plugin.toml"),
        template.manifest(&dest, &plugin_name)?,
    )?;
    std::fs::write(dest.join("Cargo.toml"), template.cargo_toml())?;
    std::fs::write(dest.join("README.md"), template.readme())?;
    std::fs::write(
        dest.join("examples").join(template.example_file_name()),
        template.example_source(),
    )?;
    Ok(dest)
}

pub(super) fn list_plugin_templates() -> String {
    let mut out = String::from("plugin templates:\n");
    for template in PluginTemplate::all() {
        out.push_str(&format!(
            "- {} default={} install=\"jfc plugin install --template {}\" {}\n",
            template.canonical_name(),
            template.default_plugin_name(),
            template.canonical_name(),
            template.description()
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn list_plugin_templates_includes_ui_diagnostics_normal() {
        let output = list_plugin_templates();

        assert!(output.contains("teammate-helper"));
        assert!(output.contains("ui-diagnostics"));
        assert!(output.contains("example-ui-diagnostics-plugin"));
    }

    #[test]
    fn list_plugin_templates_includes_prompt_context_normal() {
        let output = list_plugin_templates();

        assert!(output.contains("prompt-context"));
        assert!(output.contains("example-prompt-context-plugin"));
    }
}
