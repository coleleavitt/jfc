//! Workflow registry — discovers JS workflows from built-in, user, plugin, and
//! project sources and resolves a name to its script.
//!
//! Precedence (highest wins): project (`.jfc/workflows/`) > user
//! (`~/.config/jfc/workflows/`) > plugin (`~/.config/jfc/plugins/*/`) >
//! built-in (embedded). A project workflow with the same name as a built-in
//! shadows it.

use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use serde::Deserialize;

use super::meta::{WorkflowMeta, parse_meta};

/// Where a workflow came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkflowSource {
    BuiltIn,
    /// A plugin installed under `~/.config/jfc/plugins/`.
    Plugin,
    User,
    Project,
}

// ── Plugin manifest ────────────────────────────────────────────────────────

/// Deserialized form of a `.jfc-plugin.toml` manifest file.
#[derive(Debug, Deserialize)]
struct PluginManifest {
    plugin: PluginMeta,
}

#[derive(Debug, Deserialize)]
struct PluginMeta {
    name: String,
    workflows_dir: String,
}

static EXTRA_PLUGIN_DIRS: OnceLock<Mutex<Vec<PathBuf>>> = OnceLock::new();

/// Register a plugin/workflow directory for the current process. This backs
/// CLI `--plugin-dir` and the local-first equivalent of upstream `--plugin-url`.
pub fn register_extra_plugin_dir(path: PathBuf) {
    let slot = EXTRA_PLUGIN_DIRS.get_or_init(|| Mutex::new(Vec::new()));
    let mut dirs = slot.lock().unwrap_or_else(|e| e.into_inner());
    if !dirs.iter().any(|p| p == &path) {
        dirs.push(path);
    }
}

fn extra_plugin_dirs() -> Vec<PathBuf> {
    EXTRA_PLUGIN_DIRS
        .get()
        .and_then(|slot| slot.lock().ok().map(|dirs| dirs.clone()))
        .unwrap_or_default()
}

fn workflow_dir_for_plugin_root(path: &Path) -> PathBuf {
    let manifest_path = path.join(".jfc-plugin.toml");
    if let Ok(text) = std::fs::read_to_string(&manifest_path)
        && let Ok(manifest) = toml::from_str::<PluginManifest>(&text)
    {
        let plugin = manifest.plugin;
        let workflow_dir = plugin.workflows_dir;
        let _plugin_name = plugin.name;
        return path.join(workflow_dir);
    }
    let workflows = path.join("workflows");
    if workflows.is_dir() {
        workflows
    } else {
        path.to_path_buf()
    }
}

/// Scan `~/.config/jfc/plugins/*/` for `.jfc-plugin.toml` manifests and
/// return the resolved `workflows_dir` paths for every valid manifest found.
pub fn plugin_workflow_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    let Some(plugins_root) = dirs::config_dir().map(|c| c.join("jfc").join("plugins")) else {
        return extra_plugin_dirs()
            .into_iter()
            .map(|path| workflow_dir_for_plugin_root(&path))
            .collect();
    };
    if let Ok(entries) = std::fs::read_dir(&plugins_root) {
        for entry in entries.flatten() {
            dirs.push(workflow_dir_for_plugin_root(&entry.path()));
        }
    }
    for path in extra_plugin_dirs() {
        dirs.push(workflow_dir_for_plugin_root(&path));
    }
    dirs.sort();
    dirs.dedup();
    dirs
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
        .filter_map(|(_name, script)| {
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

/// Discover all workflows, applying precedence project > user > plugin > built-in.
/// Returns a name-sorted list with shadowed entries removed.
pub fn discover(project_root: &Path) -> Vec<RegisteredWorkflow> {
    use std::collections::HashMap;
    // Insert in increasing-precedence order so later inserts overwrite:
    // 1. builtins
    // 2. plugins  ← new
    // 3. user
    // 4. project
    let mut by_name: HashMap<String, RegisteredWorkflow> = HashMap::new();
    for wf in builtins() {
        by_name.insert(wf.name.clone(), wf);
    }
    for plugin_dir in plugin_workflow_dirs() {
        for wf in load_dir(&plugin_dir, WorkflowSource::Plugin) {
            by_name.insert(wf.name.clone(), wf);
        }
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
pub fn parse_meta_of(wf: &RegisteredWorkflow) -> Option<WorkflowMeta> {
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

    #[test]
    fn plugin_workflows_load_normal() {
        // Simulate ~/.config/jfc/plugins/my-plugin/ with a manifest and workflow.
        let tmp = tempfile::TempDir::new().unwrap();
        let plugin_dir = tmp.path().join("plugins").join("my-plugin");
        let workflows_dir = plugin_dir.join("workflows");
        std::fs::create_dir_all(&workflows_dir).unwrap();

        // Write the plugin manifest.
        std::fs::write(
            plugin_dir.join(".jfc-plugin.toml"),
            "[plugin]\nname = \"my-plugin\"\nworkflows_dir = \"workflows\"\n",
        )
        .unwrap();

        // Write a simple workflow script.
        std::fs::write(
            workflows_dir.join("plugin-demo.js"),
            "export const meta = { name: 'plugin-demo', description: 'Plugin demo workflow' }\nreturn 42",
        )
        .unwrap();

        // Use load_dir directly (avoids needing HOME set for full discover).
        let loaded = load_dir(&workflows_dir, WorkflowSource::Plugin);
        assert_eq!(loaded.len(), 1);
        let wf = &loaded[0];
        assert_eq!(wf.name, "plugin-demo");
        assert_eq!(wf.description, "Plugin demo workflow");
        assert_eq!(wf.source, WorkflowSource::Plugin);
        assert!(wf.script.contains("return 42"));
    }
}
