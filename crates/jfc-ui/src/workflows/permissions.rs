//! Named workflow permissions + save plumbing.
//!
//! Workflows can be allowed/asked/denied per name. The decision is stored in
//! the project's permission automation config (reusing the existing
//! `[permission_automation]` rules with `tool = "Workflow"` and the workflow
//! name as the rule's `path`/content). Saving a workflow writes the script to
//! the user or project workflow directory.

use std::path::Path;

/// Permission decision for running a named workflow.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkflowPermission {
    Allow,
    Ask,
    Deny,
}

/// Decide whether a named workflow may run, given the loaded config.
///
/// Inline scripts (no name) always require the model to have been explicitly
/// opted-in upstream (ultrawork / direct request), so they default to `Ask`.
/// Named workflows consult the permission rules: a rule `tool = "Workflow"`
/// whose content matches the name yields its action; otherwise `Ask`.
pub fn decide(config: &crate::config::Config, name: Option<&str>) -> WorkflowPermission {
    let Some(pa) = &config.permission_automation else {
        return WorkflowPermission::Ask;
    };
    if !pa.enabled {
        return WorkflowPermission::Ask;
    }

    let Some(name) = name else {
        return WorkflowPermission::Ask;
    };

    // Shorthand lists first (denied wins over allowed).
    if pa
        .denied_tools
        .iter()
        .any(|t| matches_workflow_token(t, name))
    {
        return WorkflowPermission::Deny;
    }
    if pa
        .allowed_tools
        .iter()
        .any(|t| matches_workflow_token(t, name))
    {
        return WorkflowPermission::Allow;
    }

    // Explicit rules: tool == "Workflow", path/content == workflow name.
    for rule in &pa.rules {
        if !rule.tool.eq_ignore_ascii_case("Workflow") {
            continue;
        }
        let target = rule.path.as_deref().unwrap_or("");
        if target == "*" || target.eq_ignore_ascii_case(name) {
            return match rule.action.to_ascii_lowercase().as_str() {
                "allow" => WorkflowPermission::Allow,
                "deny" => WorkflowPermission::Deny,
                _ => WorkflowPermission::Ask,
            };
        }
    }

    WorkflowPermission::Ask
}

/// Does a shorthand token like `Workflow` or `Workflow(bughunt)` match the
/// given workflow name?
fn matches_workflow_token(token: &str, name: &str) -> bool {
    // Bare "Workflow" matches every workflow name.
    if token.eq_ignore_ascii_case("Workflow") {
        return true;
    }
    // "Workflow(name)" matches that specific name.
    if let Some(inner) = token
        .strip_prefix("Workflow(")
        .or_else(|| token.strip_prefix("workflow("))
        .and_then(|s| s.strip_suffix(')'))
    {
        return inner.eq_ignore_ascii_case(name);
    }
    false
}

/// Where to save a workflow.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SaveScope {
    User,
    Project,
}

/// Save a workflow script to the user or project workflow directory under
/// `<name>.js`. Returns the written path.
pub fn save_workflow(
    project_root: &Path,
    scope: SaveScope,
    name: &str,
    script: &str,
) -> Result<std::path::PathBuf, String> {
    // Validate the script's meta block before persisting.
    let (meta, _body) =
        super::meta::parse_meta(script).map_err(|e| format!("invalid workflow: {e}"))?;
    if !meta.name.eq_ignore_ascii_case(name) {
        return Err(format!(
            "workflow meta.name '{}' does not match save name '{}'",
            meta.name, name
        ));
    }

    let dir = match scope {
        SaveScope::User => super::registry::user_workflows_dir()
            .ok_or_else(|| "could not resolve user config dir".to_owned())?,
        SaveScope::Project => super::registry::project_workflows_dir(project_root),
    };
    std::fs::create_dir_all(&dir).map_err(|e| format!("create {}: {e}", dir.display()))?;

    let safe_name = sanitize_name(name);
    let path = dir.join(format!("{safe_name}.js"));
    std::fs::write(&path, script).map_err(|e| format!("write {}: {e}", path.display()))?;
    Ok(path)
}

/// Sanitize a workflow name into a safe filename stem.
fn sanitize_name(name: &str) -> String {
    name.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '-'
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_bare_and_named_tokens_normal() {
        assert!(matches_workflow_token("Workflow", "bughunt"));
        assert!(matches_workflow_token("Workflow(bughunt)", "bughunt"));
        assert!(!matches_workflow_token("Workflow(other)", "bughunt"));
        assert!(!matches_workflow_token("Bash", "bughunt"));
    }

    #[test]
    fn sanitize_strips_unsafe_chars_normal() {
        assert_eq!(sanitize_name("my workflow!"), "my-workflow-");
        assert_eq!(sanitize_name("ok-name_1"), "ok-name_1");
    }

    #[test]
    fn save_round_trips_to_project_normal() {
        let tmp = tempfile::TempDir::new().unwrap();
        let script =
            "export const meta = { name: 'demo', description: 'A demo' }\nreturn await agent('hi')";
        let path = save_workflow(tmp.path(), SaveScope::Project, "demo", script).unwrap();
        assert!(path.exists());
        let resolved = super::super::registry::resolve(tmp.path(), "demo").unwrap();
        assert_eq!(resolved.name, "demo");
        assert_eq!(
            resolved.source,
            super::super::registry::WorkflowSource::Project
        );
    }

    #[test]
    fn save_rejects_name_mismatch_robust() {
        let tmp = tempfile::TempDir::new().unwrap();
        let script = "export const meta = { name: 'demo', description: 'd' }\nreturn 1";
        let err = save_workflow(tmp.path(), SaveScope::Project, "other", script).unwrap_err();
        assert!(err.contains("does not match"));
    }
}
