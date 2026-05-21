//! Workflow registry — discovers JS workflows from built-in, user, and project
//! sources and resolves a name to its script.
//!
//! Precedence (highest wins): project (`.jfc/workflows/`) > user
//! (`~/.config/jfc/workflows/`) > built-in (embedded). A project workflow with
//! the same name as a built-in shadows it.

use std::path::{Path, PathBuf};

use super::meta::{WorkflowMeta, parse_meta};

/// Where a workflow came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkflowSource {
    BuiltIn,
    User,
    Project,
}

/// A discovered workflow: its metadata, source, and full script text.
#[derive(Debug, Clone)]
pub struct RegisteredWorkflow {
    pub name: String,
    pub description: String,
    pub source: WorkflowSource,
    pub script: String,
    pub path: Option<PathBuf>,
}

/// Built-in bundled workflows, embedded at compile time.
const BUILTINS: &[(&str, &str)] = &[
    ("bughunt", BUILTIN_BUGHUNT),
    ("review-branch", BUILTIN_REVIEW_BRANCH),
    ("deep-research", BUILTIN_DEEP_RESEARCH),
];

const BUILTIN_BUGHUNT: &str = r#"export const meta = {
  name: 'bughunt',
  description: 'Find likely bugs across the codebase via parallel auditors',
  phases: [
    { title: 'Map', detail: 'survey the codebase' },
    { title: 'Investigate', detail: 'parallel bug auditors' },
  ],
}
phase('Map')
const map = await agent('Survey this codebase and list the 3 riskiest modules to audit for bugs. Return a short bullet list.')
log('Map complete')
phase('Investigate')
const findings = await parallel([
  () => agent('Audit error handling and edge cases in the riskiest module. Report concrete bugs with file:line.'),
  () => agent('Audit concurrency and async correctness. Report concrete bugs with file:line.'),
  () => agent('Audit input validation and boundary conditions. Report concrete bugs with file:line.'),
])
return { map, findings: findings.filter(Boolean) }
"#;

const BUILTIN_REVIEW_BRANCH: &str = r#"export const meta = {
  name: 'review-branch',
  description: 'Review the current branch diff across multiple dimensions',
  phases: [
    { title: 'Review' },
    { title: 'Verify' },
  ],
}
const DIMENSIONS = [
  { key: 'bugs', prompt: 'Review the current git diff for bugs and logic errors. Report file:line.' },
  { key: 'style', prompt: 'Review the current git diff for style and idiom issues. Report file:line.' },
  { key: 'tests', prompt: 'Review the current git diff for missing test coverage. Report what is untested.' },
]
phase('Review')
const reviews = await parallel(DIMENSIONS.map(d => () => agent(d.prompt, { label: 'review:' + d.key, phase: 'Review' })))
return { reviews: reviews.filter(Boolean) }
"#;

const BUILTIN_DEEP_RESEARCH: &str = r#"export const meta = {
  name: 'deep-research',
  description: 'Multi-angle research on a question passed via args',
  phases: [
    { title: 'Research' },
    { title: 'Synthesize' },
  ],
}
const question = (args && args.question) || 'the main topic of this project'
phase('Research')
const angles = await parallel([
  () => agent('Research the technical background of: ' + question),
  () => agent('Research the practical tradeoffs of: ' + question),
  () => agent('Research prior art and alternatives for: ' + question),
])
phase('Synthesize')
const synthesis = await agent('Synthesize these research notes into a coherent answer:\n' + JSON.stringify(angles.filter(Boolean)))
return { question, synthesis }
"#;

/// The user-level workflows directory: `~/.config/jfc/workflows/`.
pub fn user_workflows_dir() -> Option<PathBuf> {
    dirs::config_dir().map(|c| c.join("jfc").join("workflows"))
}

/// The project-level workflows directory: `<root>/.jfc/workflows/`.
pub fn project_workflows_dir(project_root: &Path) -> PathBuf {
    project_root.join(".jfc").join("workflows")
}

/// Load all `*.js` workflows from a directory. Invalid scripts are skipped.
fn load_dir(dir: &Path, source: WorkflowSource) -> Vec<RegisteredWorkflow> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("js") {
            continue;
        }
        let Ok(script) = std::fs::read_to_string(&path) else {
            continue;
        };
        if let Ok((meta, _body)) = parse_meta(&script) {
            out.push(RegisteredWorkflow {
                name: meta.name,
                description: meta.description,
                source,
                script,
                path: Some(path),
            });
        }
    }
    out
}

/// Built-in workflows as registered entries.
fn builtins() -> Vec<RegisteredWorkflow> {
    BUILTINS
        .iter()
        .filter_map(|(name, script)| {
            let (meta, _body) = parse_meta(script).ok()?;
            Some(RegisteredWorkflow {
                name: meta.name,
                description: meta.description,
                source: WorkflowSource::BuiltIn,
                script: (*script).to_owned(),
                path: None,
            })
        })
        .collect()
}

/// Discover all workflows, applying precedence project > user > built-in.
/// Returns a name-sorted list with shadowed entries removed.
pub fn discover(project_root: &Path) -> Vec<RegisteredWorkflow> {
    use std::collections::HashMap;
    // Insert in increasing-precedence order so later inserts overwrite.
    let mut by_name: HashMap<String, RegisteredWorkflow> = HashMap::new();
    for wf in builtins() {
        by_name.insert(wf.name.clone(), wf);
    }
    if let Some(udir) = user_workflows_dir() {
        for wf in load_dir(&udir, WorkflowSource::User) {
            by_name.insert(wf.name.clone(), wf);
        }
    }
    for wf in load_dir(
        &project_workflows_dir(project_root),
        WorkflowSource::Project,
    ) {
        by_name.insert(wf.name.clone(), wf);
    }
    let mut out: Vec<RegisteredWorkflow> = by_name.into_values().collect();
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

/// Resolve a workflow name to its full script, honoring precedence.
pub fn resolve(project_root: &Path, name: &str) -> Option<RegisteredWorkflow> {
    discover(project_root)
        .into_iter()
        .find(|w| w.name.eq_ignore_ascii_case(name))
}

/// Metadata for every available workflow (for the slash-command listing).
pub fn list_meta(project_root: &Path) -> Vec<(String, String, WorkflowSource)> {
    discover(project_root)
        .into_iter()
        .map(|w| (w.name, w.description, w.source))
        .collect()
}

#[allow(dead_code)]
fn parse_meta_of(wf: &RegisteredWorkflow) -> Option<WorkflowMeta> {
    parse_meta(&wf.script).ok().map(|(m, _)| m)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtins_parse_normal() {
        let b = builtins();
        assert!(b.iter().any(|w| w.name == "bughunt"));
        assert!(b.iter().any(|w| w.name == "review-branch"));
        assert!(b.iter().any(|w| w.name == "deep-research"));
        for w in &b {
            assert!(!w.description.is_empty());
            assert_eq!(w.source, WorkflowSource::BuiltIn);
        }
    }

    #[test]
    fn resolve_finds_builtin_normal() {
        let tmp = tempfile::TempDir::new().unwrap();
        let wf = resolve(tmp.path(), "bughunt").unwrap();
        assert_eq!(wf.name, "bughunt");
        assert_eq!(wf.source, WorkflowSource::BuiltIn);
        assert!(wf.script.contains("export const meta"));
    }

    #[test]
    fn resolve_is_case_insensitive_normal() {
        let tmp = tempfile::TempDir::new().unwrap();
        assert!(resolve(tmp.path(), "BugHunt").is_some());
    }

    #[test]
    fn project_shadows_builtin_normal() {
        let tmp = tempfile::TempDir::new().unwrap();
        let dir = project_workflows_dir(tmp.path());
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("bughunt.js"),
            "export const meta = { name: 'bughunt', description: 'PROJECT OVERRIDE' }\nreturn 1",
        )
        .unwrap();

        let wf = resolve(tmp.path(), "bughunt").unwrap();
        assert_eq!(wf.source, WorkflowSource::Project);
        assert_eq!(wf.description, "PROJECT OVERRIDE");
    }

    #[test]
    fn unknown_name_returns_none_robust() {
        let tmp = tempfile::TempDir::new().unwrap();
        assert!(resolve(tmp.path(), "nonexistent").is_none());
    }
}
