//! Workflow templates: ordered sequences of agent dispatches that can be
//! replayed via `/workflow run <name>`.
//!
//! A workflow lives at `.jfc/workflows/<name>.toml` and contains a list of
//! `[[step]]` tables. Each step names an agent + a prompt. The runner
//! dispatches each step in sequence; later steps see the leader's
//! transcript so they can build on prior work.
//!
//! Example `.jfc/workflows/refactor-auth.toml`:
//! ```toml
//! description = "Audit + refactor the auth module"
//!
//! [[step]]
//! agent = "Explore"
//! prompt = "Map every callsite of auth_middleware in the codebase."
//!
//! [[step]]
//! agent = "Plan"
//! prompt = "Design the refactor based on the explore results."
//!
//! [[step]]
//! agent = "verification"
//! prompt = "Run the full test suite after the refactor."
//! ```

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Workflow {
    #[serde(default)]
    pub description: Option<String>,
    pub step: Vec<WorkflowStep>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowStep {
    pub agent: String,
    pub prompt: String,
    /// Optional flag — if true, this step runs in parallel with the
    /// previous one (multi-Task fan-out). Default false (sequential).
    #[serde(default)]
    pub parallel: bool,
}

pub fn workflows_dir(project_root: &Path) -> PathBuf {
    project_root.join(".jfc").join("workflows")
}

/// List the workflow names available in `<project>/.jfc/workflows/`.
pub fn list(project_root: &Path) -> Vec<String> {
    let dir = workflows_dir(project_root);
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return Vec::new();
    };
    let mut out: Vec<String> = entries
        .flatten()
        .filter_map(|e| {
            let p = e.path();
            if p.extension().and_then(|e| e.to_str()) != Some("toml") {
                return None;
            }
            p.file_stem()
                .and_then(|s| s.to_str())
                .map(str::to_owned)
        })
        .collect();
    out.sort();
    out
}

/// Load a workflow by name from `<project>/.jfc/workflows/<name>.toml`.
pub fn load(project_root: &Path, name: &str) -> Result<Workflow, String> {
    let path = workflows_dir(project_root).join(format!("{name}.toml"));
    let text = std::fs::read_to_string(&path)
        .map_err(|e| format!("read {}: {e}", path.display()))?;
    toml::from_str(&text).map_err(|e| format!("parse {}: {e}", path.display()))
}

/// Render a workflow as a multi-line summary suitable for `/workflow list`.
pub fn render_summary(name: &str, w: &Workflow) -> String {
    let mut out = format!("**{name}**");
    if let Some(d) = &w.description {
        out.push_str(&format!(" — {d}"));
    }
    out.push('\n');
    for (i, step) in w.step.iter().enumerate() {
        out.push_str(&format!(
            "  {idx}. `{agent}`{parallel}: {prompt}\n",
            idx = i + 1,
            agent = step.agent,
            parallel = if step.parallel { " (parallel)" } else { "" },
            prompt = step
                .prompt
                .lines()
                .next()
                .unwrap_or("")
                .chars()
                .take(80)
                .collect::<String>(),
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn write_workflow(dir: &Path, name: &str, body: &str) {
        let wd = workflows_dir(dir);
        fs::create_dir_all(&wd).unwrap();
        fs::write(wd.join(format!("{name}.toml")), body).unwrap();
    }

    #[test]
    fn load_parses_steps_normal() {
        let tmp = TempDir::new().unwrap();
        write_workflow(
            tmp.path(),
            "audit",
            r#"
description = "Test workflow"

[[step]]
agent = "Explore"
prompt = "Find every TODO."

[[step]]
agent = "Plan"
prompt = "Group them by area."
parallel = true
"#,
        );
        let w = load(tmp.path(), "audit").unwrap();
        assert_eq!(w.description.as_deref(), Some("Test workflow"));
        assert_eq!(w.step.len(), 2);
        assert_eq!(w.step[0].agent, "Explore");
        assert!(!w.step[0].parallel);
        assert!(w.step[1].parallel);
    }

    #[test]
    fn list_returns_names_normal() {
        let tmp = TempDir::new().unwrap();
        write_workflow(
            tmp.path(),
            "a",
            "[[step]]\nagent='X'\nprompt='p'",
        );
        write_workflow(
            tmp.path(),
            "b",
            "[[step]]\nagent='X'\nprompt='p'",
        );
        let names = list(tmp.path());
        assert_eq!(names, vec!["a".to_owned(), "b".to_owned()]);
    }

    #[test]
    fn list_missing_dir_is_empty_robust() {
        let tmp = TempDir::new().unwrap();
        assert!(list(tmp.path()).is_empty());
    }

    #[test]
    fn render_summary_includes_steps_normal() {
        let w = Workflow {
            description: Some("D".into()),
            step: vec![WorkflowStep {
                agent: "Explore".into(),
                prompt: "find X".into(),
                parallel: false,
            }],
        };
        let s = render_summary("name", &w);
        assert!(s.contains("**name**"));
        assert!(s.contains("Explore"));
        assert!(s.contains("find X"));
    }
}
