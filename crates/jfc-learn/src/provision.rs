//! Project provisioning — scaffold runtime config (CLAUDE.md, AGENTS.md) for an
//! uninitialized project from a workspace scan.
//!
//! This is the *content-generation* half (pure + testable). The CLI/first-launch
//! flow that confirms with the user and writes the files is the integration
//! layer; it calls [`scaffold_claude_md`] / [`scaffold_agents_md`] and persists
//! the returned strings. We never overwrite existing config — the caller checks
//! existence first (provisioning is opt-in and user-confirmed because it touches
//! tracked files).
//!
//! Reuses [`crate::arch_graph`] for the crate map rather than re-scanning, so the
//! generated docs reflect the same architecture the diagram does.

use crate::arch_graph::CrateNode;

/// A minimal summary of what was detected about a project, used to fill the
/// scaffolded config. Kept small + serializable-friendly.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectSummary {
    /// Project / workspace name (e.g. the root dir or root Cargo.toml name).
    pub name: String,
    /// Detected primary language/toolchain label (e.g. "Rust workspace").
    pub kind: String,
    /// The workspace crates (for a Rust workspace); empty for non-Rust.
    pub crates: Vec<CrateNode>,
}

/// Build the body of a starter `CLAUDE.md` from a project summary. Includes the
/// project description, a crate inventory (if any), and the standard working
/// rules JFC expects. Deterministic.
pub fn scaffold_claude_md(summary: &ProjectSummary) -> String {
    let mut out = String::new();
    out.push_str(&format!("# {} Project Context\n\n", summary.name));
    out.push_str(&format!(
        "{} is a {}. This file is the canonical shared project instruction \
         file loaded into context.\n\n",
        summary.name, summary.kind
    ));

    if !summary.crates.is_empty() {
        out.push_str("## Workspace Crates\n\n");
        let mut sorted = summary.crates.clone();
        sorted.sort_by(|a, b| a.name.cmp(&b.name));
        for c in &sorted {
            let dep_note = if c.deps.is_empty() {
                String::new()
            } else {
                format!(" — depends on {}", c.deps.join(", "))
            };
            out.push_str(&format!("- `{}`{}\n", c.name, dep_note));
        }
        out.push('\n');
    }

    out.push_str(
        "## Working Rules\n\n\
         - Build and test from the workspace root before handing off changes.\n\
         - Keep edits scoped to the crate that owns the behavior.\n\
         - Prefer structural code tools for navigation and `rg` for literal text.\n\n\
         ## Design Biases\n\n\
         - Architecture beats feature velocity: identify the owner of shared state \
         and the integration path before adding a feature.\n\
         - Avoid god objects; preserve focused ownership boundaries.\n",
    );
    out
}

/// Build the body of a starter `AGENTS.md` (the import/init source many agent
/// tools read) from a project summary.
pub fn scaffold_agents_md(summary: &ProjectSummary) -> String {
    let mut out = String::new();
    out.push_str(&format!("# {}\n\n", summary.name));
    out.push_str(&format!("- Project type: {}.\n", summary.kind));
    if !summary.crates.is_empty() {
        out.push_str(&format!("- Workspace with {} crates.\n", summary.crates.len()));
    }
    out.push_str(
        "- Build/test from the workspace root.\n\
         - Keep modules focused — one responsibility per module.\n\
         - Root `CLAUDE.md` is the canonical shared project instruction file.\n",
    );
    out
}

/// Build a seed `MEMORY.md` index header. The memory store appends pointers to
/// this; provisioning just creates the scaffold if absent.
pub fn scaffold_memory_md(summary: &ProjectSummary) -> String {
    format!(
        "# Project Memory\n\n\
         ## Durable Facts\n\n\
         - {} is a {}.\n\
         - This index is auto-maintained; memory bodies live under the memory store, \
         not inline here.\n",
        summary.name, summary.kind
    )
}

/// Decide which config files are missing and therefore safe to scaffold. The
/// caller supplies which paths already exist; we never propose overwriting one.
/// Returns the list of file stems to create (e.g. ["CLAUDE.md", "AGENTS.md"]).
pub fn files_to_scaffold(exists: &dyn Fn(&str) -> bool) -> Vec<&'static str> {
    ["CLAUDE.md", "AGENTS.md", "MEMORY.md"]
        .into_iter()
        .filter(|f| !exists(f))
        .collect()
}

/// Outcome of a provisioning run: which files were written and which were
/// skipped (already present).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ProvisionResult {
    pub written: Vec<String>,
    pub skipped: Vec<String>,
}

/// Provision a project root: for each standard config file that does NOT already
/// exist, generate its scaffold and write it — but only after `confirm` returns
/// true (the caller supplies the user-confirmation gate). Existing files are
/// never touched. Returns which files were written vs skipped.
///
/// This is the thin glue over the tested generators: a CLI/first-launch flow
/// calls it with a real confirm prompt; tests call it with a closure.
pub fn provision_project(
    root: &std::path::Path,
    summary: &ProjectSummary,
    confirm: &dyn Fn(&[&str]) -> bool,
) -> std::io::Result<ProvisionResult> {
    let exists = |f: &str| root.join(f).exists();
    let to_make = files_to_scaffold(&exists);
    let mut result = ProvisionResult::default();

    if to_make.is_empty() {
        return Ok(result);
    }
    if !confirm(&to_make) {
        // User declined: everything stays skipped.
        result.skipped = to_make.iter().map(|s| (*s).to_owned()).collect();
        return Ok(result);
    }

    for file in to_make {
        let body = match file {
            "CLAUDE.md" => scaffold_claude_md(summary),
            "AGENTS.md" => scaffold_agents_md(summary),
            "MEMORY.md" => scaffold_memory_md(summary),
            _ => continue,
        };
        std::fs::write(root.join(file), body)?;
        result.written.push(file.to_owned());
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn summary() -> ProjectSummary {
        ProjectSummary {
            name: "jfc".into(),
            kind: "Rust workspace".into(),
            crates: vec![
                CrateNode::new("jfc-core", vec![]),
                CrateNode::new("jfc-engine", vec!["jfc-core".into()]),
            ],
        }
    }

    #[test]
    fn scaffold_claude_md_includes_crates_and_rules_normal() {
        let md = scaffold_claude_md(&summary());
        assert!(md.contains("# jfc Project Context"));
        assert!(md.contains("Rust workspace"));
        assert!(md.contains("`jfc-engine`"));
        assert!(md.contains("depends on jfc-core"));
        assert!(md.contains("## Working Rules"));
        assert!(md.contains("## Design Biases"));
    }

    #[test]
    fn scaffold_claude_md_handles_no_crates_robust() {
        let s = ProjectSummary {
            name: "site".into(),
            kind: "Node project".into(),
            crates: vec![],
        };
        let md = scaffold_claude_md(&s);
        assert!(md.contains("# site Project Context"));
        assert!(!md.contains("## Workspace Crates"));
        assert!(md.contains("## Working Rules"));
    }

    #[test]
    fn scaffold_agents_md_reports_crate_count_normal() {
        let md = scaffold_agents_md(&summary());
        assert!(md.contains("# jfc"));
        assert!(md.contains("2 crates"));
    }

    #[test]
    fn scaffold_memory_md_has_index_header_normal() {
        let md = scaffold_memory_md(&summary());
        assert!(md.contains("# Project Memory"));
        assert!(md.contains("jfc is a Rust workspace"));
    }

    #[test]
    fn files_to_scaffold_skips_existing_normal() {
        // CLAUDE.md already exists → only the other two are proposed.
        let to_make = files_to_scaffold(&|f| f == "CLAUDE.md");
        assert_eq!(to_make, vec!["AGENTS.md", "MEMORY.md"]);
    }

    #[test]
    fn files_to_scaffold_all_when_none_exist_robust() {
        let to_make = files_to_scaffold(&|_| false);
        assert_eq!(to_make, vec!["CLAUDE.md", "AGENTS.md", "MEMORY.md"]);
    }

    #[test]
    fn files_to_scaffold_none_when_all_exist_robust() {
        let to_make = files_to_scaffold(&|_| true);
        assert!(to_make.is_empty());
    }

    #[test]
    fn provision_project_writes_missing_files_when_confirmed_normal() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        let res = provision_project(root, &summary(), &|_| true).unwrap();
        assert_eq!(res.written, vec!["CLAUDE.md", "AGENTS.md", "MEMORY.md"]);
        assert!(root.join("CLAUDE.md").exists());
        assert!(root.join("AGENTS.md").exists());
        assert!(root.join("MEMORY.md").exists());
        let claude = std::fs::read_to_string(root.join("CLAUDE.md")).unwrap();
        assert!(claude.contains("jfc Project Context"));
    }

    #[test]
    fn provision_project_skips_existing_files_robust() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        std::fs::write(root.join("CLAUDE.md"), "PRE-EXISTING").unwrap();
        let res = provision_project(root, &summary(), &|_| true).unwrap();
        assert!(!res.written.contains(&"CLAUDE.md".to_owned()));
        // The pre-existing file is untouched.
        assert_eq!(
            std::fs::read_to_string(root.join("CLAUDE.md")).unwrap(),
            "PRE-EXISTING"
        );
        assert!(res.written.contains(&"AGENTS.md".to_owned()));
    }

    #[test]
    fn provision_project_writes_nothing_when_declined_robust() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        let res = provision_project(root, &summary(), &|_| false).unwrap();
        assert!(res.written.is_empty());
        assert!(!root.join("CLAUDE.md").exists());
        assert_eq!(res.skipped.len(), 3);
    }
}
