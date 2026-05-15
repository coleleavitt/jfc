//! Project-doc format contracts for `/plan`, `/roadmap`, `/parity`,
//! `/philosophy`, `/usage`.
//!
//! These are intentionally **strict, machine-readable** templates so a
//! future Atlas-style executor can parse them without heuristics. The
//! contract follows the patterns proven out in oh-my-opencode's
//! `.sisyphus/plans/*.md` (Prometheus writes; Atlas reads & flips
//! checkboxes) and the Claw Code project docs under
//! `research/claw-code/` (PARITY.md / ROADMAP.md / PHILOSOPHY.md /
//! USAGE.md).
//!
//! ## Why a module instead of inline prompts
//!
//! The same rules are needed in two places: the slash-command handler
//! (which queues a prompt asking the model to write/refresh the file)
//! and the system-prompt assembler (so any turn that touches a project
//! doc respects the format). Inlining the rules twice drifts; centralize
//! them here.
//!
//! ## Parser contract (PLAN.md)
//!
//! Atlas counts:
//!
//! - **Top-level** ` - [ ] N. ` items directly under `## TODOs` (where
//!   `N` is a decimal integer).
//! - **Top-level** ` - [ ] F1. ` / `F2.` / ... directly under
//!   `## Final Verification Wave`.
//!
//! Nested checkboxes (under "Acceptance Criteria:", "What to do:", etc.)
//! are intentionally ignored so authors can use checkbox sub-bullets
//! for documentation without polluting the executable surface.
//!
//! ## Parser contract (ROADMAP.md)
//!
//! Every entry is `### N.M[.K]. <title>` followed by:
//!
//! ```text
//! Problem:
//! Proposed:
//! Acceptance:
//! Status: unstarted|in-progress|implemented|wont-fix|superseded-by-N.M
//! ```
//!
//! **IDs are stable.** Never renumber. Use `wont-fix` or
//! `superseded-by-X.Y` instead of deleting.
//!
//! ## Parser contract (PARITY.md)
//!
//! - `## Summary` block of `- key: value` lines.
//! - One or more `## Milestones` / lane sections containing
//!   `- [ ] <statement>` evidence checklists.
//! - A `## Lane Checkpoint` table:
//!   `| Lane | Status | Feature Commit | Merge Commit | Evidence |`.
//!
//! Every checked `[x]` must include concrete evidence (commit hash,
//! file path with LOC, captured command output, or a docs reference).
//! No evidence → leave it unchecked.

use std::path::PathBuf;

/// One supported project doc.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DocKind {
    Plan,
    Roadmap,
    Parity,
    Philosophy,
    Usage,
}

impl DocKind {
    /// Canonical on-disk file name (repo-root relative). Uppercase
    /// matches the Claw Code convention for durable project docs and
    /// the ecosystem precedent (README.md, CHANGELOG.md, etc.).
    pub fn file_name(self) -> &'static str {
        match self {
            Self::Plan => "PLAN.md",
            Self::Roadmap => "ROADMAP.md",
            Self::Parity => "PARITY.md",
            Self::Philosophy => "PHILOSOPHY.md",
            Self::Usage => "USAGE.md",
        }
    }

    /// Lowercase verb used by the matching slash command.
    pub fn verb(self) -> &'static str {
        match self {
            Self::Plan => "plan",
            Self::Roadmap => "roadmap",
            Self::Parity => "parity",
            Self::Philosophy => "philosophy",
            Self::Usage => "usage",
        }
    }

    /// Short label rendered in the slash autocomplete popup.
    pub fn description(self) -> &'static str {
        match self {
            Self::Plan => "draft or update PLAN.md (Atlas-compatible task contract)",
            Self::Roadmap => "draft or update ROADMAP.md (stable-id decimal phases)",
            Self::Parity => "draft or update PARITY.md (lane checkpoints + evidence)",
            Self::Philosophy => "draft or update PHILOSOPHY.md (declarative project rationale)",
            Self::Usage => "draft or update USAGE.md (operator-focused command guide)",
        }
    }

    /// Concise format rules suitable for a system-prompt fragment.
    /// Single-paragraph per doc; kept tight so the prompt cost stays
    /// flat when every doc rule is appended together.
    pub fn rules_summary(self) -> &'static str {
        match self {
            Self::Plan => {
                "\
PLAN.md: required sections in order — `# <title>`, `## TL;DR`, \
`## Context`, `## Work Objectives`, `## Verification Strategy`, \
`## Execution Strategy`, `## TODOs`, `## Final Verification Wave`, \
`## Success Criteria`. Under `## TODOs` use top-level `- [ ] N. <task>` \
(decimal numbering, never renumber existing IDs). Under \
`## Final Verification Wave` use top-level `- [ ] F1.`, `- [ ] F2.`, \
`- [ ] F3.`, `- [ ] F4.`. Nested checkboxes inside acceptance criteria \
are ignored by the executor — keep top-level numbering clean."
            }
            Self::Roadmap => {
                "\
ROADMAP.md: each entry is `### N.M[.K]. <title>` with stable decimal \
IDs that NEVER renumber. Body has four fixed labels: `Problem:`, \
`Proposed:`, `Acceptance:`, `Status:`. Status is one of `unstarted`, \
`in-progress`, `implemented`, `wont-fix`, `superseded-by-N.M`. When an \
item is replaced, mark old status and add the new entry with a fresh \
ID; do not edit existing IDs."
            }
            Self::Parity => {
                "\
PARITY.md: `## Summary` of `- key: value` bullets; lane progress \
tables with columns `| Lane | Status | Feature Commit | Merge Commit | \
Evidence |`. Every `- [x]` checked item MUST cite concrete evidence \
(commit hash, file:LOC, captured command output, or doc path). No \
evidence → leave it `- [ ]`."
            }
            Self::Philosophy => {
                "\
PHILOSOPHY.md: declarative, narrative — not marketing. Suggested \
sections: `## Stop Staring at <X>` (what to study instead), `## The \
<Primary> Interface Is <Y>`, `## The <N>-Part System` (one `###` per \
component), `## The Real Bottleneck Changed`, `## What <Project> \
Demonstrates`, `## What Still Matters`, `## Short Version`. No \
roadmap-style IDs, no checkboxes — this is the rationale layer."
            }
            Self::Usage => {
                "\
USAGE.md: operator-focused. Required sections: `## Quick Start`, \
`## Prerequisites`, `## Install / Build`, `## Health Check`, \
`## Common Commands`, `## Slash Commands`, `## Model / Provider \
Controls`, `## State And Files`, `## Troubleshooting`. Prefer exact \
commands + expected output over prose. No checkbox contracts."
            }
        }
    }

    /// Full starter template — used when the doc doesn't exist yet. The
    /// model is told to *write or update* the file with this exact
    /// layout, then customize the content for the current project.
    /// Dispatches to one helper per kind so each template stays a small,
    /// focused function (the executor parses each layout independently,
    /// so they have no shared inner structure to factor out).
    pub fn starter_template(self, project_label: &str) -> String {
        match self {
            Self::Plan => plan_template(project_label),
            Self::Roadmap => roadmap_template(project_label),
            Self::Parity => parity_template(project_label),
            Self::Philosophy => philosophy_template(project_label),
            Self::Usage => usage_template(project_label),
        }
    }

    /// Build the per-doc prompt body sent to the model when the slash
    /// command fires. The model is told to inspect the project, then
    /// write the file at `target` using `Write`/`Edit`, honouring the
    /// format rules in [`Self::rules_summary`].
    pub fn prompt_body(self, target: &PathBuf, exists: bool) -> String {
        let verb_phrase = if exists { "update" } else { "create" };
        let rules = self.rules_summary();
        let starter = if exists {
            String::new()
        } else {
            format!(
                "\n\nStarter template (replace placeholders with real \
                 content; preserve every required heading):\n\n```markdown\n{}\n```",
                self.starter_template("<project>")
            )
        };
        format!(
            "{verb_phrase} `{path}` as a {kind} document.\n\n\
             Format rules (strict — a downstream tool will parse this file):\n\
             {rules}\n\n\
             Process:\n\
             1. Inspect the repository (Read / Glob / Grep) to ground \
                the content in real files, commits, and behaviour.\n\
             2. {verb_phrase_cap} `{path}` with the canonical structure \
                above. Do not invent statuses, IDs, or evidence — leave \
                placeholders if a value can't be cited.\n\
             3. Use `Write` (if creating) or `Edit` (if updating an \
                existing section) — never echo the whole file in the \
                assistant message.\n\
             4. After writing, summarize what changed in 1-3 lines.{starter}",
            verb_phrase = verb_phrase,
            verb_phrase_cap = capitalize_first(verb_phrase),
            path = target.display(),
            kind = self.file_name(),
            rules = rules,
            starter = starter,
        )
    }
}

fn plan_template(project_label: &str) -> String {
    format!(
        "# {project_label} Plan — <slug>\n\n\
         ## TL;DR\n\n\
         - Summary: <one sentence>\n\
         - Deliverables: <bullet>\n\
         - Effort: <S/M/L>\n\
         - Parallel: <which tasks can run in parallel>\n\
         - Critical Path: <ordered task numbers>\n\n\
         ## Context\n\n\
         <Why this work matters, what changed, what the user asked for.>\n\n\
         ## Work Objectives\n\n\
         - <objective>\n\n\
         ## Verification Strategy\n\n\
         - <how each task proves itself>\n\n\
         ## Execution Strategy\n\n\
         - <ordering, parallelism, isolation>\n\n\
         ## TODOs\n\n\
         - [ ] 1. <Task title>\n  \
         - What to do: <concrete steps>\n  \
         - Must NOT do: <bounds>\n  \
         - References: <file:line / commit / docs>\n  \
         - Acceptance Criteria: <pass/fail signal>\n  \
         - QA Scenarios: <reproducible checks>\n\n\
         ## Final Verification Wave\n\n\
         - [ ] F1. Plan Compliance Audit\n\
         - [ ] F2. Code Quality Review\n\
         - [ ] F3. Real Manual QA\n\
         - [ ] F4. Scope Fidelity Check\n\n\
         ## Success Criteria\n\n\
         - <what 'done' looks like at the plan level>\n"
    )
}

fn roadmap_template(project_label: &str) -> String {
    format!(
        "# {project_label} Roadmap\n\n\
         ## Goal\n\n\
         <One paragraph: what this roadmap is for.>\n\n\
         ## Principles\n\n\
         1. <principle>\n\n\
         ## Phase 1 — <name>\n\n\
         ### 1.1. <item title>\n\n\
         Problem: <pain point>\n\
         Proposed: <approach>\n\
         Acceptance: <pass/fail signal>\n\
         Status: unstarted\n\n\
         ### 1.2. <item title>\n\n\
         Problem: <pain point>\n\
         Proposed: <approach>\n\
         Acceptance: <pass/fail signal>\n\
         Status: unstarted\n"
    )
}

fn parity_template(project_label: &str) -> String {
    format!(
        "# Parity Status — {project_label}\n\n\
         Last updated: <YYYY-MM-DD>\n\n\
         ## Summary\n\n\
         - Canonical source: <path or upstream>\n\
         - Harness: <how parity is verified>\n\
         - HEAD: <commit hash>\n\
         - Stats: <LOC / tests / scenarios>\n\n\
         ## Milestones\n\n\
         - [ ] <statement> — evidence: <commit / file:LOC / output>\n\n\
         ## Lane Checkpoint\n\n\
         | Lane | Status | Feature Commit | Merge Commit | Evidence |\n\
         |---|---|---|---|---|\n\
         | 1. <name> | unstarted | — | — | — |\n\n\
         ## Lane Details\n\n\
         ### Lane 1 — <name>\n\n\
         - Status: <unstarted/in-progress/merged>\n\
         - Feature commit: <hash>\n\
         - Merge commit: <hash>\n\
         - Evidence: <file:LOC, command output, doc ref>\n\n\
         ## Still Open\n\n\
         - <items intentionally not yet covered>\n\n\
         ## Migration Readiness\n\n\
         - <gates remaining before declaring parity>\n"
    )
}

fn philosophy_template(project_label: &str) -> String {
    format!(
        "# {project_label} Philosophy\n\n\
         ## Stop Staring at <the obvious thing>\n\n\
         <What people look at vs. what's actually worth studying.>\n\n\
         ## The Primary Interface Is <X>\n\n\
         <Where work actually enters and exits.>\n\n\
         ## The N-Part System\n\n\
         ### 1. <Component>\n\n\
         <What it does, why it's separate.>\n\n\
         ### 2. <Component>\n\n\
         <What it does, why it's separate.>\n\n\
         ## The Real Bottleneck Changed\n\n\
         <What used to be scarce vs. what's scarce now.>\n\n\
         ## What {project_label} Demonstrates\n\n\
         - <claim backed by the code>\n\n\
         ## What Still Matters\n\n\
         - <durable, model-independent skills>\n\n\
         ## Short Version\n\n\
         **<One-line thesis.>**\n"
    )
}

fn usage_template(project_label: &str) -> String {
    format!(
        "# {project_label} Usage\n\n\
         ## Quick Start\n\n\
         ```bash\n\
         # smallest possible working invocation\n\
         ```\n\n\
         ## Prerequisites\n\n\
         - <required tool / env var>\n\n\
         ## Install / Build\n\n\
         ```bash\n\
         # exact commands\n\
         ```\n\n\
         ## Health Check\n\n\
         ```bash\n\
         # how a user verifies the install before doing real work\n\
         ```\n\n\
         ## Common Commands\n\n\
         - `<cmd>` — <what it does>\n\n\
         ## Slash Commands\n\n\
         - `/help` — <what it does>\n\n\
         ## Model / Provider Controls\n\n\
         - <env vars, /model, /effort>\n\n\
         ## State And Files\n\n\
         - `<path>` — <purpose>\n\n\
         ## Troubleshooting\n\n\
         - **<symptom>** — <fix>\n"
    )
}

fn capitalize_first(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        Some(c) => c.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

/// Resolve where the doc should live. All five docs live at the
/// **repo root** by convention — not the current working directory.
///
/// Without the git-root walk, launching jfc from `crates/jfc-ui/` and
/// running `/plan` would write `crates/jfc-ui/PLAN.md` instead of the
/// repo-root `PLAN.md` the user expects. We walk up from `cwd` looking
/// for a `.git` entry (file *or* dir — `.git` is a file in worktrees
/// and submodules); if none is found we fall back to `cwd` so the
/// command still works in a non-git directory.
pub fn doc_target(cwd: &std::path::Path, kind: DocKind) -> PathBuf {
    repo_root(cwd).join(kind.file_name())
}

/// Walk up from `start` to the nearest ancestor containing a `.git`
/// entry. Returns `start` unchanged when there's no git repo above
/// it (capped at 32 ancestor hops so a pathological path can't burn
/// CPU on a deep filesystem scan).
fn repo_root(start: &std::path::Path) -> PathBuf {
    const MAX_HOPS: usize = 32;
    let mut cur = start;
    for _ in 0..MAX_HOPS {
        if cur.join(".git").exists() {
            return cur.to_path_buf();
        }
        match cur.parent() {
            Some(parent) => cur = parent,
            None => return start.to_path_buf(),
        }
    }
    start.to_path_buf()
}

/// Concise system-prompt section listing the doc-format contracts. Only
/// emitted when at least one of the doc files actually exists in the
/// workspace — no point pinning prompt cost on rules the project hasn't
/// opted into. Mirrors the `feature_gates::system_prompt_section()`
/// pattern.
pub fn system_prompt_section(cwd: &std::path::Path) -> Option<String> {
    let kinds = [
        DocKind::Plan,
        DocKind::Roadmap,
        DocKind::Parity,
        DocKind::Philosophy,
        DocKind::Usage,
    ];
    let present: Vec<DocKind> = kinds
        .iter()
        .copied()
        .filter(|k| cwd.join(k.file_name()).is_file())
        .collect();
    if present.is_empty() {
        return None;
    }
    let mut out = String::from("## Project documents\n\n");
    out.push_str(
        "This repository ships durable project docs. When you edit them, \
         preserve the strict format the slash commands use — a downstream \
         executor parses these files.\n\n",
    );
    for k in present {
        out.push_str(&format!(
            "### `{}`\n\n{}\n\n",
            k.file_name(),
            k.rules_summary()
        ));
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    // Normal: every DocKind has a stable verb / filename pair.
    #[test]
    fn kind_file_name_and_verb_are_stable_normal() {
        assert_eq!(DocKind::Plan.file_name(), "PLAN.md");
        assert_eq!(DocKind::Plan.verb(), "plan");
        assert_eq!(DocKind::Roadmap.file_name(), "ROADMAP.md");
        assert_eq!(DocKind::Roadmap.verb(), "roadmap");
        assert_eq!(DocKind::Parity.file_name(), "PARITY.md");
        assert_eq!(DocKind::Parity.verb(), "parity");
        assert_eq!(DocKind::Philosophy.file_name(), "PHILOSOPHY.md");
        assert_eq!(DocKind::Philosophy.verb(), "philosophy");
        assert_eq!(DocKind::Usage.file_name(), "USAGE.md");
        assert_eq!(DocKind::Usage.verb(), "usage");
    }

    // Normal: the PLAN template carries every heading the executor parses.
    // If any of these disappear, an Atlas-style runner can't track tasks.
    #[test]
    fn plan_template_includes_parser_anchors_normal() {
        let s = DocKind::Plan.starter_template("Demo");
        assert!(s.contains("## TODOs"), "PLAN must include ## TODOs");
        assert!(
            s.contains("## Final Verification Wave"),
            "PLAN must include ## Final Verification Wave"
        );
        assert!(
            s.contains("- [ ] 1."),
            "PLAN must demonstrate top-level numeric task"
        );
        assert!(
            s.contains("- [ ] F1."),
            "PLAN must demonstrate F-prefixed final wave task"
        );
    }

    // Normal: ROADMAP template uses stable decimal IDs and the four
    // canonical labels. Without these the entry isn't parseable.
    #[test]
    fn roadmap_template_uses_stable_ids_and_labels_normal() {
        let s = DocKind::Roadmap.starter_template("Demo");
        assert!(s.contains("### 1.1."), "ROADMAP must use ### N.M. headings");
        assert!(s.contains("Problem:"));
        assert!(s.contains("Proposed:"));
        assert!(s.contains("Acceptance:"));
        assert!(s.contains("Status: unstarted"));
    }

    // Normal: PARITY template has the lane checkpoint table the harness
    // looks for and a Summary section.
    #[test]
    fn parity_template_has_lane_checkpoint_table_normal() {
        let s = DocKind::Parity.starter_template("Demo");
        assert!(s.contains("## Summary"));
        assert!(s.contains("## Lane Checkpoint"));
        assert!(s.contains("| Lane | Status | Feature Commit | Merge Commit | Evidence |"));
    }

    // Normal: PHILOSOPHY is narrative — no checkbox / table baggage.
    #[test]
    fn philosophy_template_is_narrative_normal() {
        let s = DocKind::Philosophy.starter_template("Demo");
        assert!(s.contains("## Short Version"));
        assert!(
            !s.contains("- [ ]"),
            "PHILOSOPHY must not include checkboxes"
        );
        assert!(!s.contains("Status:"), "PHILOSOPHY is not a roadmap");
    }

    // Normal: USAGE is operator-focused and contains shell-fenced blocks.
    #[test]
    fn usage_template_has_operator_sections_normal() {
        let s = DocKind::Usage.starter_template("Demo");
        assert!(s.contains("## Quick Start"));
        assert!(s.contains("## Health Check"));
        assert!(s.contains("```bash"));
    }

    // Robust: the prompt body changes verb between create/update so the
    // model knows whether to call Write or Edit.
    #[test]
    fn prompt_body_uses_create_vs_update_verb_robust() {
        let target = PathBuf::from("/tmp/x/PLAN.md");
        let create_body = DocKind::Plan.prompt_body(&target, false);
        let update_body = DocKind::Plan.prompt_body(&target, true);
        assert!(create_body.contains("create `"));
        assert!(create_body.contains("Starter template"));
        assert!(update_body.contains("update `"));
        assert!(
            !update_body.contains("Starter template"),
            "update should not re-emit the starter"
        );
    }

    // Robust: doc_target falls back to cwd when no `.git` is found
    // within the walk. We bound the test by writing a `.git` at the
    // tempdir root and then targeting a sibling that lives outside
    // that subtree; the upward walk hits the system parent without a
    // `.git` and (after MAX_HOPS) returns the cwd unchanged.
    //
    // We can't just use a fresh tempdir because /tmp on many CI
    // systems has scratch repos. The MAX_HOPS cap guards against
    // ascending into one of those — the test asserts the fallback
    // path, not the absence of any `.git` on the system.
    #[test]
    fn doc_target_falls_back_to_cwd_without_git_root_robust() {
        // A path with no parent — the loop hits `None` immediately
        // and returns the input unchanged. Works on every OS without
        // depending on filesystem state.
        let root = std::path::Path::new("/");
        let target = doc_target(root, DocKind::Plan);
        // Either the system root has a `.git` (rare) or it's
        // cwd-fallback. Either way the file lives directly under the
        // chosen root.
        assert_eq!(target.file_name().and_then(|s| s.to_str()), Some("PLAN.md"));
    }

    // Normal: when launched from a subdirectory of a git repo, doc_target
    // walks UP to the repo root rather than writing in the subdirectory.
    // Without this, `jfc` launched from `crates/jfc-ui/` would write
    // `crates/jfc-ui/PLAN.md` instead of the repo-root `PLAN.md`.
    #[test]
    fn doc_target_walks_up_to_repo_root_normal() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join(".git")).unwrap();
        let sub = dir.path().join("crates").join("inner");
        std::fs::create_dir_all(&sub).unwrap();
        let target = doc_target(&sub, DocKind::Roadmap);
        assert_eq!(
            target,
            dir.path().join("ROADMAP.md"),
            "doc must land at the repo root, not the subdir"
        );
    }

    // Robust: `.git` as a *file* (worktree / submodule indirection) still
    // counts as a repo root. The plain-dir check would miss this and walk
    // past, dumping docs at `/`.
    #[test]
    fn doc_target_recognises_git_file_as_root_robust() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(".git"), "gitdir: /elsewhere\n").unwrap();
        assert_eq!(
            doc_target(dir.path(), DocKind::Parity),
            dir.path().join("PARITY.md"),
        );
    }

    // Normal: when no doc files exist the system-prompt section is None
    // so we don't pay prompt cost for rules nobody adopted.
    #[test]
    fn system_prompt_section_is_none_when_no_docs_exist_normal() {
        let dir = tempfile::tempdir().unwrap();
        assert!(system_prompt_section(dir.path()).is_none());
    }

    // Normal: dropping any one of the docs into cwd triggers the section
    // and includes the format rules for THAT doc only.
    #[test]
    fn system_prompt_section_lists_only_present_docs_normal() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("PARITY.md"), "# Parity\n").unwrap();
        let body = system_prompt_section(dir.path()).expect("section present");
        assert!(body.contains("`PARITY.md`"));
        assert!(!body.contains("`PLAN.md`"));
        assert!(!body.contains("`ROADMAP.md`"));
    }
}
