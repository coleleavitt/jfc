//! Workflow registry — discovers JS workflows from built-in, user, plugin, and
//! project sources and resolves a name to its script.
//!
//! Precedence (highest wins): project (`.jfc/workflows/`, `.claude/workflows/`) >
//! user (`~/.config/jfc/workflows/`, `~/.claude/workflows/`) > plugin >
//! built-in (embedded). A project workflow with the same name as a built-in
//! shadows it.

use std::path::{Path, PathBuf};

use super::meta::{WorkflowMeta, parse_meta};
use super::plugin_discovery::{WorkflowResource, plugin_workflow_resources_for};
pub use super::plugin_discovery::{plugin_discovery_options_for, register_extra_plugin_dir};

/// Where a workflow came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkflowSource {
    BuiltIn,
    /// A plugin installed under `~/.config/jfc/plugins/`.
    Plugin,
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
    pub resource_path: Option<String>,
}

/// Built-in bundled workflows, embedded at compile time.
const BUILTINS: &[(&str, &str)] = &[
    ("bughunt", BUILTIN_BUGHUNT),
    ("code-review", BUILTIN_CODE_REVIEW),
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

const BUILTIN_CODE_REVIEW: &str = r#"export const meta = {
  name: 'code-review',
  description: 'Run a multi-phase code review with candidate finding, verification, sweep, and synthesis',
  phases: [
    { title: 'Scope', detail: 'identify review target and depth' },
    { title: 'Find', detail: 'parallel candidate discovery' },
    { title: 'Verify', detail: 'deduplicate and validate findings' },
    { title: 'Sweep', detail: 'bounded blind-spot pass' },
    { title: 'Synthesize', detail: 'produce final review report' },
  ],
}

function argValue(name, fallback) {
  if (args && typeof args === 'object' && Object.prototype.hasOwnProperty.call(args, name)) {
    return args[name]
  }
  return fallback
}

function parseArgs() {
  if (typeof args === 'string') {
    const parts = args.trim().split(/\s+/).filter(Boolean)
    const first = (parts[0] || '').toLowerCase()
    const levels = ['low', 'medium', 'high', 'xhigh', 'max']
    if (levels.indexOf(first) >= 0) {
      return { level: first, target: parts.slice(1).join(' ') || 'current git diff' }
    }
    return { level: 'high', target: parts.join(' ') || 'current git diff' }
  }
  return {
    level: String(argValue('level', 'high')).toLowerCase(),
    target: String(argValue('target', 'current git diff')),
  }
}

function parseJson(value, fallback) {
  if (value && typeof value === 'object') return value
  if (typeof value === 'string') {
    try { return JSON.parse(value) } catch (_) { return fallback }
  }
  return fallback
}

function flatten(groups) {
  const out = []
  ;(groups || []).forEach(function(group) {
    ;(group || []).forEach(function(item) { out.push(item) })
  })
  return out
}

function candidateKey(c) {
  return [
    String(c.file || '').toLowerCase(),
    String(c.line || 0),
    String(c.summary || '').toLowerCase().replace(/\s+/g, ' ').trim(),
  ].join(':')
}

function dedup(candidates) {
  const seen = {}
  const out = []
  ;(candidates || []).forEach(function(c) {
    if (!c || !c.file || !c.summary) return
    const key = candidateKey(c)
    if (seen[key]) return
    seen[key] = true
    out.push(c)
  })
  return out
}

const parsed = parseArgs()
// Deterministic proof-oracle findings attached by auto-review (cargo
// test/clippy results). Injected into the finder prompts so the LLM reviews
// with real compiler/test evidence in hand.
const proofFindings = String(argValue('proof_findings_text', '') || '')
const effort = {
  low: { maxVerify: 4, sweepMax: 0, angles: ['bugs'] },
  medium: { maxVerify: 8, sweepMax: 0, angles: ['bugs', 'tests'] },
  high: { maxVerify: 14, sweepMax: 4, angles: ['bugs', 'tests', 'regressions'] },
  xhigh: { maxVerify: 22, sweepMax: 8, angles: ['bugs', 'tests', 'regressions', 'security', 'maintainability'] },
  max: { maxVerify: 34, sweepMax: 12, angles: ['bugs', 'tests', 'regressions', 'security', 'maintainability', 'compatibility'] },
}[parsed.level] || { maxVerify: 14, sweepMax: 4, angles: ['bugs', 'tests', 'regressions'] }
const diagnostics = []

const FIND_SCHEMA = {
  type: 'object',
  additionalProperties: false,
  properties: {
    candidates: {
      type: 'array',
      items: {
        type: 'object',
        additionalProperties: false,
        properties: {
          file: { type: 'string' },
          line: { type: 'integer' },
          summary: { type: 'string' },
          severity: { type: 'string', enum: ['low', 'medium', 'high', 'critical'] },
          category: { type: 'string' },
          evidence: { type: 'string' },
          confidence: { type: 'number' },
          angle: { type: 'string' },
        },
        required: ['file', 'line', 'summary', 'severity', 'category', 'evidence', 'confidence'],
      },
    },
  },
  required: ['candidates'],
}

const VERIFY_SCHEMA = {
  type: 'object',
  additionalProperties: false,
  properties: {
    valid: { type: 'boolean' },
    file: { type: 'string' },
    line: { type: 'integer' },
    severity: { type: 'string', enum: ['low', 'medium', 'high', 'critical'] },
    category: { type: 'string' },
    summary: { type: 'string' },
    evidence: { type: 'string' },
    fix: { type: 'string' },
    confidence: { type: 'number' },
    reason: { type: 'string' },
  },
  required: ['valid', 'file', 'line', 'severity', 'category', 'summary', 'evidence', 'fix', 'confidence', 'reason'],
}

phase('Scope')
const scope = await agent(
  'Scope a code review for target: ' + parsed.target + '\n' +
  'Return the likely files, comparison base, and any constraints. Keep it concise.',
  { label: 'scope', phase: 'Scope' }
)

const ANGLE_PROMPTS = {
  bugs: 'Find concrete correctness bugs, logic errors, edge cases, and broken invariants.',
  tests: 'Find missing or weak tests that would fail to catch risky behavior changes.',
  regressions: 'Find compatibility, migration, API, persistence, or behavior regressions.',
  security: 'Find high-confidence security issues only. Avoid speculative or non-exploitable concerns.',
  maintainability: 'Find maintainability issues only when they create real defect risk.',
  compatibility: 'Find platform, provider, schema, CLI, or wire-format compatibility problems.',
}

phase('Find')
const finderGroups = await pipeline(effort.angles, async function(angle) {
  const prompt = [
    'Review target: ' + parsed.target,
    'Scope notes: ' + scope,
    proofFindings ? proofFindings : null,
    ANGLE_PROMPTS[angle] || angle,
    'Return candidates only when they name a specific file, line, evidence, and failure mode.',
    'If there are no credible candidates, return an empty candidates array.',
  ].filter(Boolean).join('\n\n')
  const raw = await agent(prompt, {
    label: 'find:' + angle,
    phase: 'Find',
    schema: FIND_SCHEMA,
  })
  const parsedResult = parseJson(raw, { candidates: [] })
  return (parsedResult.candidates || []).map(function(c) {
    c.angle = c.angle || angle
    return c
  })
})

;(finderGroups || []).forEach(function(group, index) {
  if (group === null) {
    diagnostics.push({
      stage: 'Find',
      angle: effort.angles[index] || ('angle-' + index),
      message: 'finder agent failed, timed out, or returned invalid structured output',
    })
  }
})
const candidates = dedup(flatten(finderGroups))
const verifyInput = candidates.slice(0, effort.maxVerify)

phase('Verify')
const verifiedRows = await pipeline(verifyInput, async function(candidate) {
  const prompt = [
    'Verify this code-review candidate against the repository.',
    'Target: ' + parsed.target,
    'Candidate JSON: ' + JSON.stringify(candidate),
    'Only mark valid=true when the issue is concrete, reproducible from the code, and worth reporting.',
    'For invalid candidates, set valid=false and explain why in reason.',
  ].join('\n\n')
  return parseJson(await agent(prompt, {
    label: 'verify:' + candidate.file + ':' + candidate.line,
    phase: 'Verify',
    schema: VERIFY_SCHEMA,
  }), null)
})

const verifierFailures = []
;(verifiedRows || []).forEach(function(row, index) {
  if (row === null) {
    const candidate = verifyInput[index] || {}
    const reason = 'verifier agent failed, timed out, or returned invalid structured output'
    diagnostics.push({
      stage: 'Verify',
      file: candidate.file || '',
      line: candidate.line || 0,
      summary: candidate.summary || '',
      message: reason,
    })
    verifierFailures.push({
      valid: false,
      file: candidate.file || '',
      line: candidate.line || 0,
      severity: candidate.severity || 'medium',
      category: candidate.category || 'verification',
      summary: candidate.summary || 'unverified candidate',
      evidence: candidate.evidence || '',
      fix: 'Re-run verification before acting on this candidate.',
      confidence: 0,
      reason: reason,
    })
  }
})
let findings = (verifiedRows || []).filter(function(v) { return v && v.valid })
let dismissed = (verifiedRows || []).filter(function(v) { return v && !v.valid }).concat(verifierFailures)
let sweep_notes = []

phase('Sweep')
if (effort.sweepMax > 0) {
  const sweepRaw = await agent([
    'Perform a bounded blind-spot sweep for target: ' + parsed.target,
    'Already verified findings: ' + JSON.stringify(findings),
    'Dismissed candidates: ' + JSON.stringify(dismissed),
    'Look for at most ' + effort.sweepMax + ' additional high-signal candidates. Return an empty array if none.',
  ].join('\n\n'), {
    label: 'sweep',
    phase: 'Sweep',
    schema: FIND_SCHEMA,
  })
  const sweepCandidates = dedup(parseJson(sweepRaw, { candidates: [] }).candidates || [])
    .filter(function(c) { return candidates.map(candidateKey).indexOf(candidateKey(c)) < 0 })
    .slice(0, effort.sweepMax)
  sweep_notes = sweepCandidates.map(function(c) { return c.file + ':' + c.line + ' ' + c.summary })
  const sweepVerified = await pipeline(sweepCandidates, async function(candidate) {
    return parseJson(await agent(
      'Verify this sweep candidate. Target: ' + parsed.target + '\nCandidate JSON: ' + JSON.stringify(candidate),
      { label: 'verify-sweep:' + candidate.file + ':' + candidate.line, phase: 'Verify', schema: VERIFY_SCHEMA }
    ), null)
  })
  const sweepFailures = []
  ;(sweepVerified || []).forEach(function(row, index) {
    if (row === null) {
      const candidate = sweepCandidates[index] || {}
      const reason = 'sweep verifier agent failed, timed out, or returned invalid structured output'
      diagnostics.push({
        stage: 'Verify',
        file: candidate.file || '',
        line: candidate.line || 0,
        summary: candidate.summary || '',
        message: reason,
      })
      sweepFailures.push({
        valid: false,
        file: candidate.file || '',
        line: candidate.line || 0,
        severity: candidate.severity || 'medium',
        category: candidate.category || 'verification',
        summary: candidate.summary || 'unverified sweep candidate',
        evidence: candidate.evidence || '',
        fix: 'Re-run verification before acting on this candidate.',
        confidence: 0,
        reason: reason,
      })
    }
  })
  findings = findings.concat((sweepVerified || []).filter(function(v) { return v && v.valid }))
  dismissed = dismissed.concat((sweepVerified || []).filter(function(v) { return v && !v.valid })).concat(sweepFailures)
}

// Marginal reporting (auto-review memoization): files whose content hash is
// unchanged since the last review carry pre-existing findings; the report should
// foreground findings on *changed* files and mark the rest as carried-over.
const priorHashes = parseJson(argValue('prior_content_hashes', null), null) || {}
const priorFiles = Object.keys(priorHashes)
const isCarriedOver = function(f) {
  const file = String((f && f.file) || '')
  return priorFiles.indexOf(file) >= 0 || priorFiles.some(function(p) { return file.endsWith(p) || p.endsWith(file) })
}
const marginalFindings = findings.filter(function(f) { return !isCarriedOver(f) })

phase('Synthesize')
const final_report = await agent([
  'Write the final code review report for target: ' + parsed.target,
  'Findings JSON: ' + JSON.stringify(findings),
  priorFiles.length ? ('Files reviewed before (unchanged content carries pre-existing findings): ' + JSON.stringify(priorFiles)) : null,
  priorFiles.length ? ('Marginal (newly introduced this change) findings JSON: ' + JSON.stringify(marginalFindings)) : null,
  priorFiles.length ? 'Lead with the marginal findings (newly introduced by this change). List carried-over findings separately and briefly so the report does not re-litigate pre-existing issues every run.' : null,
  'Dismissed count: ' + dismissed.length,
  'Workflow diagnostics JSON: ' + JSON.stringify(diagnostics),
  'If diagnostics is non-empty, include a short review-coverage note so the report does not imply failed stages were checked.',
  'Use a concise code-review style: findings first, ordered by severity, with file:line and concrete fix guidance.',
  'If there are no findings, say that clearly and note residual risk or test gaps.',
].filter(Boolean).join('\n\n'), { label: 'synthesize', phase: 'Synthesize' })

return {
  level: parsed.level,
  target: parsed.target,
  candidates: candidates,
  findings: findings,
  marginal_findings: marginalFindings,
  dismissed: dismissed,
  diagnostics: diagnostics,
  sweep_notes: sweep_notes,
  final_report: final_report,
}
"#;

const BUILTIN_DEEP_RESEARCH: &str = r#"export const meta = {
  name: 'deep-research',
  description: 'Multi-angle research on a question passed via args',
  phases: [
    { title: 'Research' },
    { title: 'Synthesize' },
  ],
}
// Single source of truth: each angle delegates to the Research tool, which runs
// the agentic loop (LLM planner reformulates queries from evidence, web+codebase
// search, LLM synthesis with citations). The workflow only fans out the angles
// and consolidates them — it does not reimplement the research loop.
const question = (args && args.question) || 'the main topic of this project'
phase('Research')
const angles = await parallel([
  () => agent('Use the Research tool to investigate the technical background of: ' + question + '\nReturn the tool\'s cited report verbatim.'),
  () => agent('Use the Research tool to investigate the practical tradeoffs of: ' + question + '\nReturn the tool\'s cited report verbatim.'),
  () => agent('Use the Research tool to investigate prior art and alternatives for: ' + question + '\nReturn the tool\'s cited report verbatim.'),
])
phase('Synthesize')
const synthesis = await agent('Synthesize these cited research reports into one coherent, well-cited answer:\n' + JSON.stringify(angles.filter(Boolean)))
return { question, synthesis }
"#;

pub fn user_workflows_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(config) = dirs::config_dir() {
        dirs.push(config.join("jfc/workflows"));
    }
    if let Some(home) = dirs::home_dir() {
        dirs.push(home.join(".claude/workflows"));
    }
    dirs
}

pub fn user_workflows_dir() -> Option<PathBuf> {
    dirs::config_dir().map(|c| c.join("jfc").join("workflows"))
}

pub fn project_workflows_dirs(project_root: &Path) -> Vec<PathBuf> {
    vec![
        project_workflows_dir(project_root),
        project_root.join(".claude/workflows"),
    ]
}

pub fn project_workflows_dir(project_root: &Path) -> PathBuf {
    project_root.join(".jfc").join("workflows")
}

/// Load all `*.js` workflows from a directory. Invalid scripts are skipped.
fn load_dir(dir: &Path, source: WorkflowSource) -> Vec<RegisteredWorkflow> {
    load_dir_with_resource_path(dir, source, None)
}

fn load_resource_dir(
    resource: &WorkflowResource,
    source: WorkflowSource,
) -> Vec<RegisteredWorkflow> {
    load_dir_with_resource_path(
        resource.path.as_path(),
        source,
        Some(resource.resource_path.as_str()),
    )
}

fn load_dir_with_resource_path(
    dir: &Path,
    source: WorkflowSource,
    resource_path: Option<&str>,
) -> Vec<RegisteredWorkflow> {
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
                resource_path: resource_path.map(str::to_owned),
            });
        }
    }
    out
}

/// Built-in workflows as registered entries.
fn builtins() -> Vec<RegisteredWorkflow> {
    let resource_path = builtin_workflow_resource_path();
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
                resource_path: resource_path.clone(),
            })
        })
        .collect()
}

fn builtin_workflow_resource_path() -> Option<String> {
    super::plugin_discovery::builtin_workflow_resource_descriptors()
        .into_iter()
        .next()
        .map(|descriptor| descriptor.path)
}

/// Discover all workflows, applying precedence project > user > plugin > built-in.
/// Returns a name-sorted list with shadowed entries removed.
pub fn discover(project_root: &Path) -> Vec<RegisteredWorkflow> {
    use std::collections::HashMap;
    // Insert in increasing-precedence order so later inserts overwrite:
    // 1. builtins
    // 2. plugins
    // 3. user
    // 4. project
    let mut by_name: HashMap<String, RegisteredWorkflow> = HashMap::new();
    for wf in builtins() {
        by_name.insert(wf.name.clone(), wf);
    }
    for plugin_resource in plugin_workflow_resources_for(project_root) {
        for wf in load_resource_dir(&plugin_resource, WorkflowSource::Plugin) {
            by_name.insert(wf.name.clone(), wf);
        }
    }
    for udir in user_workflows_dirs() {
        for wf in load_dir(&udir, WorkflowSource::User) {
            by_name.insert(wf.name.clone(), wf);
        }
    }
    for project_dir in project_workflows_dirs(project_root) {
        for wf in load_dir(&project_dir, WorkflowSource::Project) {
            by_name.insert(wf.name.clone(), wf);
        }
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
        assert!(b.iter().any(|w| w.name == "code-review"));
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
        assert_eq!(
            wf.resource_path.as_deref(),
            Some(jfc_plugin_host::BUILTIN_WORKFLOW_RESOURCE_PATH)
        );
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
    fn dot_claude_workflow_shadows_legacy_project_normal() {
        let tmp = tempfile::TempDir::new().unwrap();
        let legacy = project_workflows_dir(tmp.path());
        let dot = tmp.path().join(".claude/workflows");
        std::fs::create_dir_all(&legacy).unwrap();
        std::fs::create_dir_all(&dot).unwrap();
        std::fs::write(
            legacy.join("demo.js"),
            "export const meta = { name: 'demo', description: 'legacy' }\nreturn 1",
        )
        .unwrap();
        std::fs::write(
            dot.join("demo.js"),
            "export const meta = { name: 'demo', description: 'dot' }\nreturn 2",
        )
        .unwrap();

        let wf = resolve(tmp.path(), "demo").unwrap();
        assert_eq!(wf.source, WorkflowSource::Project);
        assert_eq!(wf.description, "dot");
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
        assert_eq!(wf.resource_path, None);
        assert!(wf.script.contains("return 42"));
    }

    #[test]
    fn resolve_plugin_workflow_keeps_resource_descriptor_path_normal() {
        let tmp = tempfile::TempDir::new().unwrap();
        let plugin_dir = tmp.path().join("plugins").join("my-plugin");
        let workflows_dir = plugin_dir.join("workflows");
        std::fs::create_dir_all(&workflows_dir).unwrap();
        std::fs::write(
            plugin_dir.join(".jfc-plugin.toml"),
            "[plugin]\nname = \"my-plugin\"\nworkflows_dir = \"workflows\"\n",
        )
        .unwrap();
        std::fs::write(
            workflows_dir.join("plugin-demo.js"),
            "export const meta = { name: 'plugin-demo', description: 'Plugin demo workflow' }\nreturn 42",
        )
        .unwrap();

        let wf = resolve(tmp.path(), "plugin-demo").unwrap();
        let expected = workflows_dir.to_string_lossy().into_owned();
        assert_eq!(wf.source, WorkflowSource::Plugin);
        assert_eq!(wf.resource_path.as_deref(), Some(expected.as_str()));
    }

    #[test]
    fn enabled_plugins_false_disables_plugin_workflows_normal() {
        let tmp = tempfile::TempDir::new().unwrap();
        let plugin_dir = tmp.path().join("plugins").join("my-plugin");
        let workflows_dir = plugin_dir.join("workflows");
        std::fs::create_dir_all(&workflows_dir).unwrap();
        std::fs::create_dir_all(tmp.path().join(".claude")).unwrap();
        std::fs::write(
            plugin_dir.join(".jfc-plugin.toml"),
            "[plugin]\nname = \"my-plugin\"\nworkflows_dir = \"workflows\"\n",
        )
        .unwrap();
        std::fs::write(
            workflows_dir.join("plugin-demo.js"),
            "export const meta = { name: 'plugin-demo', description: 'Plugin demo workflow' }\nreturn 42",
        )
        .unwrap();
        std::fs::write(
            tmp.path().join(".claude/settings.json"),
            r#"{ "enabledPlugins": { "my-plugin@local": false } }"#,
        )
        .unwrap();

        assert!(resolve(tmp.path(), "plugin-demo").is_none());
    }
}
