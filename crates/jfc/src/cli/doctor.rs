//! `jfc doctor` — diagnose filesystem paths for XDG compatibility.
//!
//! Non-destructive: reports where legacy, non-XDG paths are in use and
//! recommends modern locations. The CLI does not move or delete any data.

use clap::Subcommand;

#[derive(Subcommand, Debug)]
pub(super) enum DoctorSubcommand {
    /// Scan common jfc paths and report legacy locations with suggestions.
    Paths,
}

pub(super) async fn run_doctor_subcommand(sub: DoctorSubcommand) -> anyhow::Result<()> {
    match sub {
        DoctorSubcommand::Paths => paths_diagnostic(),
    }
}

fn paths_diagnostic() -> anyhow::Result<()> {
    use std::path::PathBuf;

    // Keybindings: legacy ~/.claude/keybindings.json vs XDG ~/.config/claude/keybindings.json
    let legacy_keybindings = dirs::home_dir()
        .map(|h| h.join(".claude").join("keybindings.json"))
        .unwrap_or_else(|| PathBuf::from("/nonexistent"));
    let xdg_keybindings = dirs::config_dir()
        .map(|c| c.join("claude").join("keybindings.json"))
        .unwrap_or_else(|| PathBuf::from("/nonexistent"));

    println!("jfc doctor — XDG path diagnostics\n");

    // Config base
    if let Some(cfg) = dirs::config_dir() {
        println!("• XDG_CONFIG_HOME: {}", cfg.display());
        println!("  - jfc config dir: {}/jfc", cfg.display());
    } else {
        println!("• XDG_CONFIG_HOME: (not set; dirs::config_dir() unavailable)");
    }

    if let Some(data) = dirs::data_dir() {
        println!("• XDG_DATA_HOME:   {}", data.display());
    } else {
        println!("• XDG_DATA_HOME:   (not set)");
    }

    if let Some(cache) = dirs::cache_dir() {
        println!("• XDG_CACHE_HOME:  {}", cache.display());
    } else {
        println!("• XDG_CACHE_HOME:  (not set)");
    }

    println!("\nChecks:\n");

    // Keybindings check
    let legacy_exists = legacy_keybindings.exists();
    let xdg_exists = xdg_keybindings.exists();
    if legacy_exists && !xdg_exists {
        println!(
            "- Legacy keybindings found at {}\n  Recommendation: move or copy to {}",
            legacy_keybindings.display(),
            xdg_keybindings
                .parent()
                .map(|p| p.display().to_string() + "/keybindings.json")
                .unwrap_or_else(|| "~/.config/claude/keybindings.json".to_string())
        );
    } else if xdg_exists {
        println!(
            "- Keybindings present at modern XDG path: {}",
            xdg_keybindings.display()
        );
    } else {
        println!("- No keybindings file found (optional feature)");
    }

    // Sessions location note (current: ~/.config/jfc/sessions)
    let sessions_legacy = dirs::config_dir()
        .map(|c| c.join("jfc").join("sessions"))
        .unwrap_or_else(|| PathBuf::from("/nonexistent"));
    let sessions_modern = dirs::data_dir()
        .map(|d| d.join("jfc").join("sessions"))
        .unwrap_or_else(|| PathBuf::from("/nonexistent"));
    println!(
        "- Sessions directory (current): {}",
        sessions_legacy.display()
    );
    if !sessions_modern.as_os_str().is_empty() {
        println!(
            "  Suggested future location: {} (XDG_DATA_HOME)",
            sessions_modern.display()
        );
    }

    // Logs recommendation — currently under ~/.config/jfc/logs
    let logs_cfg = dirs::config_dir()
        .map(|c| c.join("jfc").join("logs"))
        .unwrap_or_else(|| PathBuf::from("/nonexistent"));
    let logs_cache = dirs::cache_dir()
        .map(|c| c.join("jfc").join("logs"))
        .unwrap_or_else(|| PathBuf::from("/nonexistent"));
    println!("- Logs directory (current): {}", logs_cfg.display());
    if !logs_cache.as_os_str().is_empty() {
        println!(
            "  Suggested cache location: {} (XDG_CACHE_HOME)",
            logs_cache.display()
        );
    }

    Ok(())
}
