//! Team onboarding — generates a setup guide from recent session data.
//!
//! Reads session metadata, MCP configs, and available skills to produce
//! a onboarding document for new team members joining this project.

use std::path::Path;

/// Generate a team onboarding/setup guide based on project state.
///
/// Inspects:
/// - Recent session metadata (most-used commands)
/// - Configured MCP servers
/// - Available skills
/// - CLAUDE.md presence
pub fn generate_onboarding_guide(project_root: &Path) -> String {
    let mut guide = String::from("# Team Onboarding Guide\n\n");

    // Check CLAUDE.md
    let claude_md = project_root.join("CLAUDE.md");
    if claude_md.exists() {
        guide.push_str("## Project Context\n\n");
        guide.push_str("✓ `CLAUDE.md` is present — the assistant loads project-specific instructions automatically.\n\n");
    } else {
        guide.push_str("## Project Context\n\n");
        guide.push_str(
            "⚠ No `CLAUDE.md` found. Run `/init` to scaffold one with project conventions.\n\n",
        );
    }

    // Check MCP servers
    guide.push_str("## MCP Servers\n\n");
    let mcp_config_paths = [
        project_root.join(".claude/settings.json"),
        project_root.join(".jfc/settings.json"),
    ];
    let mut mcp_servers: Vec<String> = Vec::new();
    for path in &mcp_config_paths {
        if let Ok(content) = std::fs::read_to_string(path)
            && let Ok(val) = serde_json::from_str::<serde_json::Value>(&content)
            && let Some(mcp) = val.get("mcpServers").and_then(|m| m.as_object())
        {
            for name in mcp.keys() {
                mcp_servers.push(name.clone());
            }
        }
    }
    if mcp_servers.is_empty() {
        guide.push_str("No MCP servers configured. Add them to `.claude/settings.json` under `mcpServers`.\n\n");
    } else {
        guide.push_str(&format!("Configured servers ({}):\n", mcp_servers.len()));
        for s in &mcp_servers {
            guide.push_str(&format!("  • {s}\n"));
        }
        guide.push('\n');
    }

    // Check skills
    guide.push_str("## Available Skills\n\n");
    let skills_dir = project_root.join(".claude/skills");
    let skills: Vec<String> = if skills_dir.is_dir() {
        std::fs::read_dir(&skills_dir)
            .ok()
            .map(|entries| {
                entries
                    .filter_map(|e| e.ok())
                    .filter(|e| e.path().extension().is_some_and(|ext| ext == "md"))
                    .map(|e| {
                        e.path()
                            .file_stem()
                            .unwrap_or_default()
                            .to_string_lossy()
                            .into_owned()
                    })
                    .collect()
            })
            .unwrap_or_default()
    } else {
        Vec::new()
    };
    if skills.is_empty() {
        guide.push_str("No skills defined yet. Add `.claude/skills/<name>.md` files to teach the assistant project-specific workflows.\n\n");
    } else {
        guide.push_str(&format!("Skills ({}):\n", skills.len()));
        for s in &skills {
            guide.push_str(&format!("  • /{s}\n"));
        }
        guide.push('\n');
    }

    // Check recent sessions for common commands
    guide.push_str("## Quick Start\n\n");
    guide.push_str("1. Run `jfc` to start an interactive session\n");
    guide.push_str("2. Use `/help` to see all available commands\n");
    guide.push_str("3. Use `/skills` to list project-specific skill shortcuts\n");
    if !mcp_servers.is_empty() {
        guide.push_str("4. MCP tools are auto-loaded — use `/mcp` to inspect status\n");
    }
    guide.push_str("\n## Common Workflows\n\n");
    guide.push_str("- `/check` — run cargo check for diagnostics\n");
    guide.push_str("- `/compact` — summarize old context when token usage is high\n");
    guide.push_str("- `/cost` — check token spend for the session\n");
    guide.push_str("- `/dream` — run a self-improvement learning pass\n");

    guide
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn generates_guide_for_empty_project() {
        let tmp = PathBuf::from("/tmp/jfc-onboarding-test-nonexistent");
        let guide = generate_onboarding_guide(&tmp);
        assert!(guide.contains("Team Onboarding Guide"));
        assert!(guide.contains("No `CLAUDE.md` found"));
    }
}
