//! Architecture graph — generate a Mermaid overview of the workspace's real
//! crate-dependency wiring, plus a coverage/wiring overlay.
//!
//! The diagram is generated from the actual `path = "../<crate>"` dependencies
//! declared in each crate's `Cargo.toml`, so it stays accurate by construction
//! (no hand-maintained diagram to drift). This is the data half; a command or
//! dreamer task supplies the parsed [`CrateNode`]s and writes the rendered
//! Mermaid to `ARCHITECTURE.md`.
//!
//! Pure + deterministic: callers feed in parsed crate nodes, get back a Mermaid
//! string. No filesystem access here (the caller reads Cargo.tomls), so the
//! rendering logic is unit-testable in isolation.

/// One workspace crate and the in-workspace crates it depends on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CrateNode {
    /// Crate name, e.g. "jfc-engine".
    pub name: String,
    /// Names of other workspace crates this one depends on (path deps).
    pub deps: Vec<String>,
    /// Optional coverage signal: `true` if the crate has tests, `false` if a
    /// scan found none, `None` if unknown. Drives the coverage overlay styling.
    pub has_tests: Option<bool>,
}

impl CrateNode {
    pub fn new(name: impl Into<String>, deps: Vec<String>) -> Self {
        Self {
            name: name.into(),
            deps,
            has_tests: None,
        }
    }

    pub fn with_tests(mut self, has_tests: bool) -> Self {
        self.has_tests = Some(has_tests);
        self
    }
}

/// Extract the in-workspace path-dependency names from one `Cargo.toml`'s text.
/// Matches lines like `jfc-core = { path = "../jfc-core" }` and returns the
/// dependency crate names (the trailing path segment). Deterministic and
/// dependency-free (no toml parser needed for this narrow shape).
pub fn parse_path_deps(cargo_toml: &str) -> Vec<String> {
    let mut deps = Vec::new();
    for line in cargo_toml.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('#') {
            continue;
        }
        // Find `path = "..."` and take the final path segment as the crate name.
        let Some(idx) = trimmed.find("path") else {
            continue;
        };
        let after = &trimmed[idx..];
        let Some(open) = after.find('"') else { continue };
        let rest = &after[open + 1..];
        let Some(close) = rest.find('"') else { continue };
        let path = &rest[..close];
        // Only in-workspace path deps (relative). Take the last segment.
        if let Some(name) = path.rsplit('/').next()
            && !name.is_empty()
            && name != ".."
        {
            deps.push(name.to_owned());
        }
    }
    deps.sort();
    deps.dedup();
    deps
}

/// Extract the crate name from a `Cargo.toml`'s `[package] name = "..."`.
pub fn parse_crate_name(cargo_toml: &str) -> Option<String> {
    let mut in_package = false;
    for line in cargo_toml.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            in_package = trimmed == "[package]";
            continue;
        }
        if in_package && let Some(rest) = trimmed.strip_prefix("name") {
            // name = "x"
            if let Some(open) = rest.find('"') {
                let after = &rest[open + 1..];
                if let Some(close) = after.find('"') {
                    return Some(after[..close].to_owned());
                }
            }
        }
    }
    None
}

/// Render the crate-dependency graph as a Mermaid `flowchart`. Edges point from
/// a crate to each crate it depends on. Deterministic ordering (nodes + edges
/// sorted) so the output is diff-friendly across regenerations.
pub fn render_mermaid(nodes: &[CrateNode]) -> String {
    let mut sorted: Vec<&CrateNode> = nodes.iter().collect();
    sorted.sort_by(|a, b| a.name.cmp(&b.name));

    let mut out = String::from("```mermaid\nflowchart TD\n");

    // Node declarations (with a stable sanitized id) + optional coverage class.
    for node in &sorted {
        let id = sanitize_id(&node.name);
        out.push_str(&format!("    {id}[\"{}\"]\n", node.name));
    }

    // Edges, sorted for stability.
    let mut edges: Vec<(String, String)> = Vec::new();
    for node in &sorted {
        for dep in &node.deps {
            edges.push((node.name.clone(), dep.clone()));
        }
    }
    edges.sort();
    edges.dedup();
    for (from, to) in &edges {
        out.push_str(&format!(
            "    {} --> {}\n",
            sanitize_id(from),
            sanitize_id(to)
        ));
    }

    // Coverage overlay: crates with no tests get a flagged class so the diagram
    // doubles as a coverage map.
    let untested: Vec<&&CrateNode> = sorted
        .iter()
        .filter(|n| n.has_tests == Some(false))
        .collect();
    if !untested.is_empty() {
        out.push_str("    classDef untested fill:#3a1a1a,stroke:#ff6b6b,color:#fff;\n");
        for node in &untested {
            out.push_str(&format!("    class {} untested;\n", sanitize_id(&node.name)));
        }
    }

    out.push_str("```\n");
    out
}

/// A short human-readable coverage/wiring summary to accompany the diagram.
pub fn coverage_summary(nodes: &[CrateNode]) -> String {
    let total = nodes.len();
    let untested: Vec<&str> = nodes
        .iter()
        .filter(|n| n.has_tests == Some(false))
        .map(|n| n.name.as_str())
        .collect();
    let unknown = nodes.iter().filter(|n| n.has_tests.is_none()).count();
    if untested.is_empty() {
        format!(
            "{total} crates; all crates with a known coverage signal have tests ({unknown} unknown)."
        )
    } else {
        format!(
            "{total} crates; {} without tests: {} ({unknown} unknown).",
            untested.len(),
            untested.join(", ")
        )
    }
}

/// Scan a workspace `crates/` directory into [`CrateNode`]s by reading each
/// crate's `Cargo.toml` (name + path deps) and probing for tests (a `tests/`
/// dir or a `#[cfg(test)]`/`#[test]` marker anywhere under `src/`). Returns the
/// nodes sorted by name. Best-effort: unreadable crates are skipped.
pub fn scan_workspace(crates_dir: &std::path::Path) -> std::io::Result<Vec<CrateNode>> {
    let mut nodes = Vec::new();
    for entry in std::fs::read_dir(crates_dir)? {
        let entry = entry?;
        let dir = entry.path();
        if !dir.is_dir() {
            continue;
        }
        let cargo = dir.join("Cargo.toml");
        let Ok(text) = std::fs::read_to_string(&cargo) else {
            continue;
        };
        let Some(name) = parse_crate_name(&text) else {
            continue;
        };
        let deps = parse_path_deps(&text);
        let has_tests = crate_has_tests(&dir);
        nodes.push(CrateNode {
            name,
            deps,
            has_tests: Some(has_tests),
        });
    }
    nodes.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(nodes)
}

/// Heuristic test probe: a `tests/` dir, or any `#[cfg(test)]` / `#[test]`
/// marker in the crate's `src/` tree.
fn crate_has_tests(crate_dir: &std::path::Path) -> bool {
    if crate_dir.join("tests").is_dir() {
        return true;
    }
    let src = crate_dir.join("src");
    src_tree_has_test_marker(&src)
}

fn src_tree_has_test_marker(dir: &std::path::Path) -> bool {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return false;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if src_tree_has_test_marker(&path) {
                return true;
            }
        } else if path.extension().is_some_and(|e| e == "rs")
            && let Ok(text) = std::fs::read_to_string(&path)
            && (text.contains("#[cfg(test)]") || text.contains("#[test]"))
        {
            return true;
        }
    }
    false
}

/// Render a full `ARCHITECTURE.md` body: a heading, the coverage summary, and
/// the Mermaid diagram. Suitable for writing to disk by a command/dreamer task.
pub fn render_architecture_md(nodes: &[CrateNode]) -> String {
    format!(
        "# Architecture\n\n\
         _Auto-generated from workspace `Cargo.toml` path-dependencies. \
         Regenerate rather than editing by hand._\n\n\
         {}\n\n\
         ## Crate dependency graph\n\n\
         {}\n",
        coverage_summary(nodes),
        render_mermaid(nodes)
    )
}

/// Mermaid node ids can't contain hyphens/dots; map them to underscores.
fn sanitize_id(name: &str) -> String {
    name.chars()
        .map(|c| if c.is_alphanumeric() { c } else { '_' })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_path_deps_extracts_workspace_crates_normal() {
        let toml = r#"
[package]
name = "jfc-engine"

[dependencies]
jfc-core = { path = "../jfc-core" }
jfc-learn = { path = "../jfc-learn" }
serde = { workspace = true }
tokio = "1"
"#;
        let deps = parse_path_deps(toml);
        assert_eq!(deps, vec!["jfc-core", "jfc-learn"]);
    }

    #[test]
    fn parse_path_deps_ignores_non_path_and_comments_robust() {
        let toml = r#"
# jfc-ghost = { path = "../jfc-ghost" }
serde = { workspace = true }
anyhow = "1"
"#;
        assert!(parse_path_deps(toml).is_empty());
    }

    #[test]
    fn parse_crate_name_reads_package_name_normal() {
        let toml = "[package]\nname = \"jfc-core\"\nversion = \"0.1.0\"\n";
        assert_eq!(parse_crate_name(toml).as_deref(), Some("jfc-core"));
    }

    #[test]
    fn parse_crate_name_ignores_dependency_name_robust() {
        // A `name` under [dependencies] must NOT be mistaken for the package.
        let toml = "[dependencies]\nname = \"not-the-package\"\n";
        assert_eq!(parse_crate_name(toml), None);
    }

    #[test]
    fn render_mermaid_is_deterministic_normal() {
        let nodes = vec![
            CrateNode::new("jfc-engine", vec!["jfc-core".into(), "jfc-learn".into()]),
            CrateNode::new("jfc-learn", vec!["jfc-core".into()]),
            CrateNode::new("jfc-core", vec![]),
        ];
        let a = render_mermaid(&nodes);
        let b = render_mermaid(&nodes);
        assert_eq!(a, b, "render must be deterministic");
        assert!(a.contains("flowchart TD"));
        assert!(a.contains("jfc_engine --> jfc_core"));
        assert!(a.contains("jfc_engine --> jfc_learn"));
        assert!(a.contains("jfc_learn --> jfc_core"));
    }

    #[test]
    fn render_mermaid_overlays_untested_normal() {
        let nodes = vec![
            CrateNode::new("jfc-core", vec![]).with_tests(true),
            CrateNode::new("jfc-ghost", vec!["jfc-core".into()]).with_tests(false),
        ];
        let out = render_mermaid(&nodes);
        assert!(out.contains("classDef untested"));
        assert!(out.contains("class jfc_ghost untested;"));
        assert!(!out.contains("class jfc_core untested;"));
    }

    #[test]
    fn coverage_summary_reports_untested_normal() {
        let nodes = vec![
            CrateNode::new("a", vec![]).with_tests(true),
            CrateNode::new("b", vec![]).with_tests(false),
        ];
        let summary = coverage_summary(&nodes);
        assert!(summary.contains("2 crates"));
        assert!(summary.contains("1 without tests: b"));
    }

    #[test]
    fn sanitize_id_replaces_punctuation_robust() {
        assert_eq!(sanitize_id("jfc-anthropic-sdk"), "jfc_anthropic_sdk");
    }
}

#[cfg(test)]
mod workspace_smoke {
    use super::*;

    // Smoke: scanning the real workspace finds jfc-core with no path deps and
    // jfc-engine depending on jfc-core, and produces valid Mermaid.
    #[test]
    fn scan_real_workspace_smoke_normal() {
        // Walk up from this crate (crates/jfc-learn) to the workspace crates dir.
        let here = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let crates_dir = here.parent().expect("crates parent");
        let nodes = scan_workspace(crates_dir).expect("scan");
        assert!(nodes.len() >= 20, "expected many crates, got {}", nodes.len());
        let core = nodes.iter().find(|n| n.name == "jfc-core").expect("jfc-core");
        assert!(core.deps.is_empty(), "jfc-core should have no path deps");
        let engine = nodes.iter().find(|n| n.name == "jfc-engine").expect("jfc-engine");
        assert!(engine.deps.iter().any(|d| d == "jfc-core"));
        let md = render_architecture_md(&nodes);
        assert!(md.contains("flowchart TD"));
        assert!(md.contains("jfc_engine --> jfc_core"));
    }
}
