use std::path::PathBuf;

use clap::Subcommand;

mod descriptor_rows;
mod diagnostics;
mod doctor_issue_rows;
mod doctor_rows;
mod doctor_runtime_rows;
mod list;
mod store;
mod template_definitions;
mod templates;

use crate::plugin_smoke::smoke_plugin;
use diagnostics::plugin_doctor;
use list::list_plugins;
use store::{install_plugin, looks_like_git_url, remove_plugin, update_plugins};
use templates::{install_plugin_template, list_plugin_templates};

#[derive(Subcommand, Debug)]
pub(super) enum PluginSubcommand {
    /// List installed local plugins.
    List,
    /// Re-discover plugins and print descriptor reload/cache diagnostics.
    Doctor {
        /// Previous descriptor digest to compare against.
        #[arg(long)]
        previous_digest: Option<String>,
    },
    /// List first-party plugin templates available for installation.
    Templates,
    /// Run process-bridge descriptor smoke checks for an installed plugin.
    Smoke {
        /// Installed plugin name.
        name: String,
    },
    /// Install a plugin from a local directory, git URL, or first-party template.
    Install {
        /// Local directory or git URL.
        #[arg(required_unless_present = "template")]
        source: Option<String>,
        /// First-party SDK template name.
        #[arg(long, value_name = "TEMPLATE", conflicts_with = "source")]
        template: Option<String>,
        /// Override the installed plugin directory name.
        #[arg(long)]
        name: Option<String>,
        /// Replace an existing plugin with the same name.
        #[arg(long)]
        force: bool,
    },
    /// Update one git-backed plugin, or all git-backed plugins when omitted.
    Update {
        /// Installed plugin name.
        name: Option<String>,
    },
    /// Remove an installed local plugin.
    Remove {
        /// Installed plugin name.
        name: String,
    },
}

pub(super) async fn run_plugin_subcommand(sub: PluginSubcommand) -> anyhow::Result<()> {
    let managed = jfc_engine::config::load_managed_settings();
    let safe_mode = jfc_engine::config::safe_mode_enabled();
    match sub {
        PluginSubcommand::List => {
            print!("{}", list_plugins()?);
            Ok(())
        }
        PluginSubcommand::Doctor { previous_digest } => {
            print!("{}", plugin_doctor(previous_digest.as_deref())?);
            Ok(())
        }
        PluginSubcommand::Templates => {
            print!("{}", list_plugin_templates());
            Ok(())
        }
        PluginSubcommand::Smoke { name } => {
            print!("{}", smoke_plugin(&name).await?);
            Ok(())
        }
        PluginSubcommand::Install {
            source,
            template,
            name,
            force,
        } => {
            if safe_mode {
                anyhow::bail!("plugin installs are disabled in safe mode");
            }
            if let Some(template) = template {
                let path = install_plugin_template(&template, name.as_deref(), force)?;
                println!("installed plugin template {template} at {}", path.display());
                return Ok(());
            }
            let Some(source) = source else {
                anyhow::bail!("plugin install requires a source or --template");
            };
            if managed.as_ref().is_some_and(|m| m.disable_plugin_urls)
                && looks_like_git_url(&source)
            {
                anyhow::bail!("plugin URL installs are disabled by managed settings");
            }
            if managed.as_ref().is_some_and(|m| m.disable_plugin_dirs)
                && !looks_like_git_url(&source)
            {
                anyhow::bail!("local plugin directory installs are disabled by managed settings");
            }
            let path = install_plugin(&source, name.as_deref(), force)?;
            println!("installed plugin at {}", path.display());
            Ok(())
        }
        PluginSubcommand::Update { name } => {
            if safe_mode {
                anyhow::bail!("plugin updates are disabled in safe mode");
            }
            if managed.as_ref().is_some_and(|m| m.disable_plugin_updates) {
                anyhow::bail!("plugin updates are disabled by managed settings");
            }
            let updated = update_plugins(name.as_deref())?;
            if updated.is_empty() {
                println!("no git-backed plugins to update");
            } else {
                for item in updated {
                    println!("{item}");
                }
            }
            Ok(())
        }
        PluginSubcommand::Remove { name } => {
            if safe_mode {
                anyhow::bail!("plugin removal is disabled in safe mode");
            }
            let path = remove_plugin(&name)?;
            println!("removed plugin at {}", path.display());
            Ok(())
        }
    }
}

pub(super) fn ensure_plugin_url(url: &str) -> anyhow::Result<PathBuf> {
    store::ensure_plugin_url(url)
}
