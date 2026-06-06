use clap::Subcommand;

#[derive(Subcommand, Debug)]
pub(super) enum PolicySubcommand {
    /// Show effective managed policy and every source considered.
    Status {
        /// Emit JSON instead of text.
        #[arg(long)]
        json: bool,
    },
    /// Show managed-settings source precedence and parse status.
    Sources {
        /// Emit JSON instead of text.
        #[arg(long)]
        json: bool,
    },
}

pub(super) async fn run_policy_subcommand(sub: PolicySubcommand) -> anyhow::Result<()> {
    match sub {
        PolicySubcommand::Status { json } => print_policy_status(json),
        PolicySubcommand::Sources { json } => print_policy_sources(json),
    }
}

fn print_policy_status(json: bool) -> anyhow::Result<()> {
    let effective = crate::config::load_managed_settings();
    let sources = crate::config::managed_settings_sources();
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "effective": effective,
                "sources": sources,
            }))?
        );
        return Ok(());
    }
    match &effective {
        Some(policy) => {
            println!("managed policy: active");
            if let Some(tier) = policy.policy_tier.as_deref() {
                println!("tier: {tier}");
            }
            if let Some(notice) = policy.security_notice.as_deref() {
                println!("security notice: {notice}");
            }
            if policy.require_oauth {
                println!("requires oauth: yes");
            }
            if policy.require_elevated_auth {
                println!("requires elevated auth: yes");
            }
            if let Some(user) = policy.required_user.as_deref() {
                println!("required user: {user}");
            }
            if !policy.required_env.is_empty() {
                println!("required env: {}", policy.required_env.join(", "));
            }
            if let Some(limit) = policy.max_budget_usd.or(policy.spend_limit_usd) {
                println!("spend limit: ${limit:.2}");
            }
            println!("remote control disabled: {}", policy.disable_remote_control);
            println!("plugin urls disabled: {}", policy.disable_plugin_urls);
            println!("plugin dirs disabled: {}", policy.disable_plugin_dirs);
            println!("plugin updates disabled: {}", policy.disable_plugin_updates);
            if !policy.allowed_tools.is_empty() {
                println!("allowed tools: {}", policy.allowed_tools.join(", "));
            }
            if !policy.disallowed_tools.is_empty() {
                println!("disallowed tools: {}", policy.disallowed_tools.join(", "));
            }
        }
        None => println!("managed policy: inactive"),
    }
    println!();
    print_sources_text(&sources);
    Ok(())
}

fn print_policy_sources(json: bool) -> anyhow::Result<()> {
    let sources = crate::config::managed_settings_sources();
    if json {
        println!("{}", serde_json::to_string_pretty(&sources)?);
    } else {
        print_sources_text(&sources);
    }
    Ok(())
}

fn print_sources_text(sources: &[crate::config::ManagedSettingsSource]) {
    println!("sources:");
    for source in sources {
        let path = source
            .path
            .as_ref()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| "(embedded)".to_owned());
        let status = if source.loaded {
            "loaded"
        } else if source.exists {
            "ignored"
        } else {
            "missing"
        };
        match source.error.as_deref() {
            Some(error) => println!("- {}: {path} [{status}] error={error}", source.label),
            None => println!("- {}: {path} [{status}]", source.label),
        }
    }
}
