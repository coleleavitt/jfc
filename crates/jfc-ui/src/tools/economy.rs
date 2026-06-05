use std::process::Stdio;

use super::registry::{collusion_detector, market_orchestrator, snapshot_event_sender};
use crate::runtime::send_critical;

/// SwarmProvider impl for jfc-ui — delegates to the existing
/// `worktrees` module. Each solver gets a worktree named
/// `economy/<bounty_id>/<agent_id>` so concurrent bounties don't
/// collide. `remove_worktree` is best-effort: a leftover worktree
/// after a crash is cleaned up by the user via `git worktree prune`.
pub(crate) struct EconomySwarmProvider {
    repo_root: std::path::PathBuf,
}

impl EconomySwarmProvider {
    pub fn new(repo_root: std::path::PathBuf) -> Self {
        Self { repo_root }
    }
}

#[async_trait::async_trait]
impl jfc_economy::reporting::SwarmProvider for EconomySwarmProvider {
    async fn create_worktree(
        &self,
        bounty_id: &str,
        agent_id: &jfc_economy::types::AgentId,
    ) -> Option<std::path::PathBuf> {
        let safe_bounty: String = bounty_id
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                    c
                } else {
                    '-'
                }
            })
            .collect();
        let safe_agent: String = agent_id
            .0
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                    c
                } else {
                    '-'
                }
            })
            .collect();
        let name = format!("economy-{safe_bounty}-{safe_agent}");
        match crate::worktrees::create_worktree_async(&self.repo_root, &name).await {
            Ok(info) => Some(std::path::PathBuf::from(info.path)),
            Err(e) => {
                tracing::warn!(
                    target: "jfc::economy",
                    bounty = bounty_id,
                    agent = %agent_id.0,
                    error = %e,
                    "create_worktree failed; solver will run without worktree isolation"
                );
                None
            }
        }
    }

    async fn remove_worktree(&self, path: &std::path::Path) {
        // The underlying `worktrees::remove_worktree` takes the
        // worktree *name* (the branch / dir leaf), not a full path.
        // We named worktrees `economy-<bounty>-<agent>` in
        // `create_worktree`, so the path's last component is the
        // name. If extraction fails (impossible-but-defensive),
        // skip removal — the user can `git worktree prune` later.
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            tracing::warn!(
                target: "jfc::economy",
                path = %path.display(),
                "remove_worktree: cannot extract name from path; skipping"
            );
            return;
        };
        if let Err(e) = crate::worktrees::remove_worktree(&self.repo_root, name) {
            tracing::warn!(
                target: "jfc::economy",
                path = %path.display(),
                error = %e,
                "remove_worktree failed (orphan worktree — `git worktree prune` to clean)"
            );
        }
    }

    fn send_message(&self, agent_id: &jfc_economy::types::AgentId, message: &str) {
        // No mailbox integration in this iteration — log for audit.
        // Wiring to the existing swarm/mailbox system requires
        // routing through main.rs's event channel and is deferred.
        tracing::info!(
            target: "jfc::economy",
            agent = %agent_id.0,
            msg = %message.chars().take(200).collect::<String>(),
            "swarm send_message (audit-only stub)"
        );
    }
}

/// AgentInvoker impl for jfc-ui — runs real LLM calls via the
/// configured Provider trait. Each solver / validator call is one
/// `provider.stream(...)` round-trip; the response text becomes the
/// solution patch (for solvers) or the proposed flaw (for
/// validators). Token counts come from the StreamEvent::Usage
/// callback when the provider emits one, otherwise from a 4-chars-
/// per-token byte estimate.
pub(crate) struct EconomyAgentInvoker {
    provider: std::sync::Arc<dyn jfc_provider::Provider>,
    model: jfc_provider::ModelId,
    /// Optional UI event channel — when set, every solver / validator
    /// invocation emits TaskStarted before streaming, AgentChunk for
    /// each text delta, and TaskCompleted/Failed at the end. This is
    /// what makes bounty subagents show up in the same fan UI / ctrl+X
    /// panel as regular Task-tool subagents. None is fine for tests.
    event_tx: Option<tokio::sync::mpsc::Sender<crate::runtime::AppEvent>>,
}

impl EconomyAgentInvoker {
    pub fn new(
        provider: std::sync::Arc<dyn jfc_provider::Provider>,
        model: jfc_provider::ModelId,
    ) -> Self {
        Self {
            provider,
            model,
            event_tx: snapshot_event_sender(),
        }
    }

    /// Drive a single LLM call and return `(text, tokens_consumed)`.
    /// Tokens fall back to a byte estimate when the provider doesn't
    /// emit a Usage event (most don't on the first chunk). When
    /// `task_id` is provided and the invoker has an event channel,
    /// streams text deltas as `AgentChunk` events keyed by that
    /// task_id so the fan UI fills live.
    async fn one_shot(
        &self,
        system: String,
        user: String,
        max_tokens: u64,
        task_id: Option<&str>,
    ) -> Result<(String, u64), String> {
        use futures::StreamExt;
        use jfc_provider::*;
        let opts = StreamOptions::new(self.model.clone())
            .system(system)
            .max_tokens(max_tokens.min(u32::MAX as u64) as u32);
        let messages = vec![ProviderMessage {
            role: ProviderRole::User,
            content: vec![ProviderContent::Text(user)],
        }];
        let mut stream = self
            .provider
            .stream(messages, &opts)
            .await
            .map_err(|e| format!("provider stream error: {e}"))?;
        let mut text = String::new();
        let mut input_tokens: u64 = 0;
        let mut output_tokens: u64 = 0;
        while let Some(ev) = stream.next().await {
            match ev {
                Ok(StreamEvent::TextDelta { delta, .. }) => {
                    if let (Some(tx), Some(id)) = (&self.event_tx, task_id) {
                        tx.send(crate::runtime::AppEvent::Task(
                            crate::runtime::TaskEvent::AgentChunk {
                                task_id: crate::ids::TaskId::from(id),
                                text: delta.clone(),
                            },
                        ))
                        .await
                        .ok();
                    }
                    text.push_str(&delta);
                }
                Ok(StreamEvent::TextDone { text: t, .. }) => {
                    if text.is_empty() {
                        text = t;
                    }
                }
                Ok(StreamEvent::Usage {
                    input_tokens: i,
                    output_tokens: o,
                    ..
                }) => {
                    input_tokens = i as u64;
                    output_tokens = o as u64;
                    if let (Some(tx), Some(id)) = (&self.event_tx, task_id) {
                        // TaskProgress is non-critical; next progress update supersedes.
                        tx.try_send(crate::runtime::AppEvent::Task(
                            crate::runtime::TaskEvent::Progress {
                                task_id: crate::ids::TaskId::from(id),
                                last_tool: None,
                                elapsed_ms: 0,
                                tool_use_count: None,
                                input_tokens: Some(i as u64),
                                cache_read_tokens: None,
                                cache_write_tokens: None,
                                output_tokens: Some(o as u64),
                            },
                        ))
                        .ok();
                    }
                }
                Ok(StreamEvent::Error { message }) => {
                    return Err(format!("provider stream error: {message}"));
                }
                Err(e) => return Err(format!("provider stream error: {e}")),
                _ => {}
            }
        }
        let tokens = if input_tokens > 0 || output_tokens > 0 {
            input_tokens + output_tokens
        } else {
            // Fallback: 4 chars per token (v131 z_$).
            (text.len() as u64).div_ceil(4)
        };
        Ok((text, tokens))
    }

    /// Emit `TaskStarted` so a `BackgroundTask` shows up in the fan
    /// UI and ctrl+X panel for the duration of this subagent. Call
    /// before starting the actual stream; pair with `emit_completed`
    /// or `emit_failed` after.
    fn emit_started(&self, task_id: &str, description: &str) {
        if let Some(tx) = &self.event_tx {
            send_critical(
                tx,
                crate::runtime::AppEvent::Task(crate::runtime::TaskEvent::Started {
                    task_id: crate::ids::TaskId::from(task_id),
                    description: description.to_owned(),
                    // Report the solver/validator model so the
                    // BackgroundTask's `model_used` is populated. Without
                    // it the per-progress token deltas this invoker emits
                    // never roll into `app.usage_by_model` (the handler at
                    // task.rs only credits usage when model_used is Some),
                    // and the bounty's API spend stays invisible in the
                    // status bar / cost panel — the billing blind spot.
                    model_used: Some(self.model.as_str().to_owned()),
                    max_input_tokens: None,
                    // Economy solver/validator agents run in-process via the
                    // same Task tool path as ordinary subagents.
                    is_detached: false,
                    // Economy agents aren't linked to a user-facing todo —
                    // they're spawned by the bounty market, not the task queue.
                    parent_task_id: None,
                }),
            );
        }
    }

    fn emit_completed(&self, task_id: &str, summary: &str, elapsed_ms: u64) {
        if let Some(tx) = &self.event_tx {
            send_critical(
                tx,
                crate::runtime::AppEvent::Task(crate::runtime::TaskEvent::Completed {
                    task_id: crate::ids::TaskId::from(task_id),
                    summary: summary.to_owned(),
                    elapsed_ms,
                }),
            );
        }
    }

    fn emit_failed(&self, task_id: &str, error: &str) {
        if let Some(tx) = &self.event_tx {
            send_critical(
                tx,
                crate::runtime::AppEvent::Task(crate::runtime::TaskEvent::Failed {
                    task_id: crate::ids::TaskId::from(task_id),
                    error: error.to_owned(),
                }),
            );
        }
    }
}

#[async_trait::async_trait]
impl jfc_economy::reporting::AgentInvoker for EconomyAgentInvoker {
    async fn invoke_solver(
        &self,
        prompt: jfc_economy::reporting::SolverPrompt,
    ) -> Result<jfc_economy::types::Solution, String> {
        let task_id = format!("economy-solver-{}", prompt.agent_id.0);
        let desc = format!(
            "Solver: {}",
            prompt
                .bounty_description
                .lines()
                .next()
                .unwrap_or("")
                .chars()
                .take(60)
                .collect::<String>()
        );
        self.emit_started(&task_id, &desc);
        let started_at = std::time::Instant::now();

        // Build a TaskInput that runs the solver as a full agentic loop
        // inside the assigned worktree.
        let solver_prompt = format!(
            "You are a competitive solver agent in a code-bounty market.\n\n\
             Bounty: {}\n\n\
             Description: {}\n\n\
             Acceptance criteria: {}\n\n\
             You have full access to Read, Write, Edit, Bash, Grep, and Glob tools. \
             Use them to explore the codebase, understand the problem, implement \
             the solution, and verify it compiles/passes tests. Work directly in the \
             current directory — it is your isolated worktree.",
            prompt.bounty_id, prompt.bounty_description, prompt.acceptance_criteria,
        );
        let task_input = jfc_core::TaskInput {
            description: desc.clone(),
            prompt: solver_prompt,
            subagent_type: Some("build".to_string()),
            category: None,
            run_in_background: false,
            model: Some(self.model.as_str().to_string()),
            effort: None,
            name: Some(prompt.agent_id.0.clone()),
            team_name: None,
            mode: Some("default".to_string()),
            isolation: None, // Worktree already created by SwarmProvider
            parent_task_id: None,
            schema: None,
        };

        tracing::info!(
            target: "jfc::ui::economy",
            agent = %prompt.agent_id.0,
            bounty_id = %prompt.bounty_id,
            worktree = ?prompt.worktree,
            "invoke_solver: dispatching agentic loop"
        );

        let result = super::subagent::execute_task(
            &task_input,
            self.provider.as_ref(),
            self.model.clone(),
            self.event_tx.as_ref(),
            Some(&task_id),
            None,
            prompt.worktree.clone(),
            None,
            None,
        )
        .await;

        let elapsed_ms = started_at.elapsed().as_millis() as u64;

        if result.is_error() {
            self.emit_failed(&task_id, &result.output);
            return Err(format!("Solver execution failed: {}", result.output));
        }

        // Collect the solver's work as a git diff from the worktree
        let patch = if let Some(ref wt_path) = prompt.worktree {
            let diff_output = tokio::process::Command::new("git")
                .args(["diff", "HEAD"])
                .current_dir(wt_path)
                .output()
                .await
                .ok();
            diff_output
                .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
                .unwrap_or_default()
        } else {
            String::new()
        };

        let tokens_estimate = (patch.len() as u64).div_ceil(4).max(100);
        let mut solution = jfc_economy::types::Solution {
            agent_id: prompt.agent_id,
            bounty_id: prompt.bounty_id.clone(),
            patch: patch.clone(),
            explanation: result.output.clone(),
            self_assessment: 0.8,
            tokens_consumed: tokens_estimate,
            compiles: None,
            tests_pass: None,
            suspicious: false,
        };

        // Mechanistically verify the solution in the worktree
        if let Some(ref worktree) = prompt.worktree {
            let verification = verify_bounty_solution(worktree, &prompt.bounty_id, &solution).await;
            solution.compiles = Some(verification.passed);
            solution.tests_pass = Some(verification.passed);
            solution.suspicious = !verification.passed;
            solution
                .explanation
                .push_str("\n\nMechanistic verification: ");
            solution.explanation.push_str(&verification.summary);
        } else {
            solution.suspicious = true;
            solution
                .explanation
                .push_str("\n\nMechanistic verification: no solver worktree was available.");
        }

        let summary = format!("{} bytes patch", patch.len());
        self.emit_completed(&task_id, &summary, elapsed_ms);
        Ok(solution)
    }

    async fn invoke_validator(
        &self,
        prompt: jfc_economy::reporting::ValidatorPrompt,
    ) -> Result<jfc_economy::reporting::ValidatorOutcome, String> {
        let task_id = format!("economy-validator-{}", prompt.validator_id.0);
        let desc = format!(
            "Validator: {}",
            prompt
                .bounty_description
                .lines()
                .next()
                .unwrap_or("")
                .chars()
                .take(60)
                .collect::<String>()
        );
        self.emit_started(&task_id, &desc);
        let started_at = std::time::Instant::now();
        tracing::debug!(
            target: "jfc::ui::economy",
            validator = %prompt.validator_id.0,
            bounty_id = %prompt.bounty_id,
            solver = %prompt.solution.agent_id.0,
            max_tokens = prompt.max_tokens,
            "invoke_validator: streaming"
        );
        let system = "You are an adversarial validator in a code-bounty \
             market. Your job: find any flaw in the submitted solution. \
             You earn tokens for VALID flaws (reproducible by a test) and \
             lose trust for invalid challenges. If the solution looks sound, \
             say so explicitly with confidence ≥ 0.95 — early termination \
             saves the bounty pool.\n\n\
             Output format:\n\
             FLAW: <description, or NONE>\n\
             CONFIDENCE: <0.0 to 1.0>\n\
             TEST: <minimal test code that triggers the flaw, or NONE>"
            .to_string();
        let user = format!(
            "Bounty {} — {}\n\nSolution patch:\n```\n{}\n```\n\n\
             Solver's explanation: {}",
            prompt.bounty_id,
            prompt.bounty_description,
            prompt
                .solution
                .patch
                .chars()
                .take(4_000)
                .collect::<String>(),
            prompt
                .solution
                .explanation
                .chars()
                .take(500)
                .collect::<String>(),
        );
        match self
            .one_shot(system, user, prompt.max_tokens, Some(&task_id))
            .await
        {
            Ok((text, tokens)) => {
                let (flaw, confidence, test_code) = parse_validator_output(&text);
                let summary = match (&flaw, &test_code) {
                    (Some(f), Some(_)) => format!(
                        "flaw with reproducible test (conf {confidence:.2}): {}",
                        f.chars().take(80).collect::<String>()
                    ),
                    (Some(f), None) => format!(
                        "flaw without test (conf {confidence:.2}): {}",
                        f.chars().take(80).collect::<String>()
                    ),
                    (None, _) => format!("no flaw found (conf {confidence:.2})"),
                };
                self.emit_completed(&task_id, &summary, started_at.elapsed().as_millis() as u64);
                Ok(jfc_economy::reporting::ValidatorOutcome {
                    flaw,
                    test_code,
                    confidence,
                    tokens_consumed: tokens,
                })
            }
            Err(e) => {
                self.emit_failed(&task_id, &e);
                Err(e)
            }
        }
    }

    async fn adjudicate_test(&self, test_code: &str, worktree: Option<&std::path::Path>) -> bool {
        let Some(wt_path) = worktree else {
            return false;
        };
        // Write the validator's test to a temporary file inside the worktree
        let test_dir = wt_path.join("tests");
        let test_file = test_dir.join("_validator_adjudication_test.rs");
        if std::fs::create_dir_all(&test_dir).is_err() {
            return false;
        }
        if std::fs::write(&test_file, test_code).is_err() {
            return false;
        }
        // Run cargo test against this specific test file
        let result = tokio::process::Command::new("cargo")
            .args([
                "test",
                "--test",
                "_validator_adjudication_test",
                "--",
                "--nocapture",
            ])
            .current_dir(wt_path)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .await;
        // Clean up the test file regardless of outcome
        std::fs::remove_file(&test_file).ok();
        // If cargo test exits non-zero, the flaw is proven
        match result {
            Ok(output) => !output.status.success(),
            Err(_) => false,
        }
    }
}

#[cfg(test)]
pub(crate) fn split_patch_and_explanation(text: &str) -> (String, String) {
    if let Some(start) = text.find("```diff").or_else(|| text.find("```")) {
        let after = &text[start..];
        let body_start = after.find('\n').map(|n| start + n + 1).unwrap_or(start);
        if let Some(end_rel) = text[body_start..].find("```") {
            let patch = text[body_start..body_start + end_rel].trim().to_string();
            let explanation = text[body_start + end_rel + 3..].trim().to_string();
            return (patch, explanation);
        }
    }
    (text.trim().to_string(), String::new())
}

/// Cheap HTML→text fallback used by `WebFetch` when content-type
/// indicates HTML. This is intentionally minimal — drops anything
/// between `<` and `>`, collapses runs of whitespace, normalizes
/// line breaks. Doesn't decode entities or handle `<script>`/`<style>`
/// content cleanly. A proper implementation would use scraper /
/// html5ever, but the dependency cost isn't worth it for an MVP
/// WebFetch — the model can usually reason about even ragged text.
pub(super) fn strip_html_tags(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut in_tag = false;
    let mut last_was_space = false;
    for c in html.chars() {
        match c {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if in_tag => {}
            _ if c.is_whitespace() => {
                if !last_was_space {
                    out.push(' ');
                    last_was_space = true;
                }
            }
            _ => {
                out.push(c);
                last_was_space = false;
            }
        }
    }
    out.trim().to_string()
}

/// Result of applying a winning solver's solution to disk.
pub(crate) struct AppliedSolution {
    /// Files that were created or overwritten, relative to cwd.
    pub files: Vec<std::path::PathBuf>,
    /// Human-readable summary line for the tool result body.
    pub summary: String,
}

/// Apply a winning solver's `solution.patch` to disk under `cwd`.
///
/// Solvers may return either:
///   1. A unified diff (handled by `git apply` if cwd is a git repo).
///   2. One or more `===FILE: <path>===\n<contents>\n===END===\n`
///      blocks — our explicit, parser-friendly format the solver
///      prompt nudges them toward when the bounty is a green-field
///      "create files at <path>" request.
///   3. Raw content — saved to `.jfc/bounties/<id>/winner.patch` as a
///      fallback so the user can inspect it.
///
/// Always writes `winner.patch` and `winner.md` under
/// `.jfc/bounties/<bounty_id>/` for audit. Returns the list of
/// affected paths so the dispatcher can report them to the user.
///
/// This closes the 2026-05-06 HMAC bug where run_bounty reported
/// "settled" but never actually wrote the solver's patch — every
/// successful cycle now produces visible files.
pub(crate) fn apply_winning_solution(
    cwd: &std::path::Path,
    bounty_id: &str,
    solution: Option<&jfc_economy::types::Solution>,
) -> AppliedSolution {
    let Some(sol) = solution else {
        tracing::warn!(
            target: "jfc::ui::bounty",
            bounty_id = %bounty_id,
            "no winning solution to apply (cycle settled with no winner)"
        );
        return AppliedSolution {
            files: vec![],
            summary: "No winning solution — nothing written.".into(),
        };
    };
    // Review/test-before-production gate (mirrors the ChangeSet state machine:
    // a change cannot reach Applied without passing tests). A winning solution
    // that explicitly failed its validator checks — tests failed, or flagged
    // suspicious by the adversarial validator — must NOT be written to the
    // main checkout. `None` (unknown) is permitted: not every bounty runs a
    // test oracle, and that path is unchanged from before.
    if sol.tests_pass == Some(false) || sol.suspicious {
        let reason = if sol.suspicious {
            "flagged suspicious by validation"
        } else {
            "tests failed"
        };
        tracing::warn!(
            target: "jfc::ui::bounty",
            bounty_id = %bounty_id,
            winner = %sol.agent_id.0,
            reason,
            "refusing to apply winning solution to the main checkout (review/test gate)"
        );
        crate::changeset::record_event(
            jfc_changeset::LedgerEvent::new(
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_millis() as u64)
                    .unwrap_or(0),
                jfc_changeset::EventKind::Failure,
                "bounty-apply-blocked",
            )
            .with_detail(format!("{bounty_id}: {reason}")),
        );
        return AppliedSolution {
            files: vec![],
            summary: format!(
                "Refused to apply winning solution ({reason}). \
                 Review/test-before-production gate blocked the write."
            ),
        };
    }
    let audit_dir = cwd.join(".jfc").join("bounties").join(bounty_id);
    if let Err(e) = std::fs::create_dir_all(&audit_dir) {
        tracing::error!(
            target: "jfc::ui::bounty",
            bounty_id = %bounty_id,
            error = %e,
            "failed to create audit dir"
        );
        return AppliedSolution {
            files: vec![],
            summary: format!("Failed to create audit dir: {e}"),
        };
    }
    std::fs::write(audit_dir.join("winner.patch"), &sol.patch).ok();
    std::fs::write(audit_dir.join("winner.md"), &sol.explanation).ok();
    tracing::info!(
        target: "jfc::ui::bounty",
        bounty_id = %bounty_id,
        winner = %sol.agent_id.0,
        patch_bytes = sol.patch.len(),
        audit_dir = %audit_dir.display(),
        "wrote winner audit files"
    );

    // Path 2: explicit FILE blocks — robust against LLMs that don't
    // produce valid diffs.
    let file_blocks = parse_file_blocks(&sol.patch);
    if !file_blocks.is_empty() {
        let mut written = Vec::new();
        for (path, contents) in &file_blocks {
            let Some(abs) = resolve_solution_file_path(cwd, path) else {
                tracing::warn!(
                    target: "jfc::ui::bounty",
                    bounty_id = %bounty_id,
                    path = %path.display(),
                    "rejected solver file path outside bounty worktree"
                );
                continue;
            };
            if let Some(parent) = abs.parent()
                && let Err(e) = std::fs::create_dir_all(parent)
            {
                tracing::warn!(
                    target: "jfc::ui::bounty",
                    bounty_id = %bounty_id,
                    path = %abs.display(),
                    error = %e,
                    "mkdir parent failed"
                );
                continue;
            }
            match std::fs::write(&abs, contents) {
                Ok(_) => {
                    tracing::info!(
                        target: "jfc::ui::bounty",
                        bounty_id = %bounty_id,
                        path = %abs.display(),
                        bytes = contents.len(),
                        "wrote solver file"
                    );
                    written.push(abs);
                }
                Err(e) => tracing::warn!(
                    target: "jfc::ui::bounty",
                    bounty_id = %bounty_id,
                    path = %abs.display(),
                    error = %e,
                    "write failed"
                ),
            }
        }
        let summary = if written.is_empty() {
            format!(
                "Patch saved to {} but no files written (all writes failed).",
                audit_dir.display()
            )
        } else {
            let mut s = format!("Wrote {} file(s):", written.len());
            for p in written.iter().take(10) {
                s.push_str(&format!("\n  - {}", p.display()));
            }
            if written.len() > 10 {
                s.push_str(&format!("\n  ... and {} more", written.len() - 10));
            }
            s.push_str(&format!(
                "\nFull patch + explanation: {}",
                audit_dir.display()
            ));
            s
        };
        return AppliedSolution {
            files: written,
            summary,
        };
    }

    // Path 1: try `git apply` if it looks like a unified diff and cwd
    // is a git repo.
    if looks_like_unified_diff(&sol.patch) {
        let patch_path = audit_dir.join("winner.patch");
        let out = std::process::Command::new("git")
            .arg("-C")
            .arg(cwd)
            .arg("apply")
            .arg("--whitespace=nowarn")
            .arg(&patch_path)
            .output();
        match out {
            Ok(o) if o.status.success() => {
                tracing::info!(
                    target: "jfc::ui::bounty",
                    bounty_id = %bounty_id,
                    "git apply succeeded"
                );
                return AppliedSolution {
                    files: vec![patch_path],
                    summary: format!(
                        "Applied unified diff via `git apply` (audit: {}).",
                        audit_dir.display()
                    ),
                };
            }
            Ok(o) => tracing::warn!(
                target: "jfc::ui::bounty",
                bounty_id = %bounty_id,
                stderr = %String::from_utf8_lossy(&o.stderr),
                "git apply failed; falling back to audit-only"
            ),
            Err(e) => tracing::warn!(
                target: "jfc::ui::bounty",
                bounty_id = %bounty_id,
                error = %e,
                "git apply could not be invoked"
            ),
        }
    }

    // Path 3: audit-only fallback.
    AppliedSolution {
        files: vec![audit_dir.join("winner.patch")],
        summary: format!(
            "Solution didn't parse as a diff or FILE block — audit copy at {}.",
            audit_dir.display()
        ),
    }
}

fn resolve_solution_file_path(
    cwd: &std::path::Path,
    path: &std::path::Path,
) -> Option<std::path::PathBuf> {
    use std::path::Component;

    if path.is_absolute() {
        return None;
    }

    for component in path.components() {
        match component {
            Component::Normal(_) | Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => return None,
        }
    }

    Some(cwd.join(path))
}

#[derive(Debug)]
pub(super) struct MechanisticVerification {
    pub passed: bool,
    pub summary: String,
}

pub(super) async fn verify_bounty_solution(
    worktree: &std::path::Path,
    bounty_id: &str,
    solution: &jfc_economy::types::Solution,
) -> MechanisticVerification {
    // Charter enforcement: reject solutions that delete existing tests.
    let deleted_tests = solution
        .patch
        .lines()
        .filter(|l| l.starts_with('-') && l.contains("#[test]"))
        .count();
    if deleted_tests > 0 {
        tracing::warn!(
            target: "jfc::economy::charter",
            agent = %solution.agent_id.0,
            bounty_id,
            deleted_tests,
            "Solution rejected: patch deletes existing test annotations (Charter: TestDeletion)"
        );
        return MechanisticVerification {
            passed: false,
            summary: format!(
                "Charter violation (TestDeletion): patch deletes {} existing #[test] annotation(s).",
                deleted_tests
            ),
        };
    }

    let applied = apply_winning_solution(worktree, bounty_id, Some(solution));
    let summary = applied.summary.to_lowercase();
    if applied.files.is_empty()
        || summary.contains("failed to")
        || summary.contains("audit-only")
        || summary.contains("audit copy")
        || summary.contains("no concrete file changes")
    {
        return MechanisticVerification {
            passed: false,
            summary: format!(
                "patch application failed or produced no concrete project files ({})",
                applied.summary
            ),
        };
    }

    let Some((program, args, label)) = verification_command(worktree) else {
        return MechanisticVerification {
            passed: false,
            summary: format!(
                "patch applied to {}; no mechanistic build/test command was detected",
                applied
                    .files
                    .iter()
                    .map(|p| p.display().to_string())
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        };
    };

    let mut command = tokio::process::Command::new(program);
    command
        .args(args)
        .current_dir(worktree)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    #[cfg(feature = "landlock-sandbox")]
    {
        use crate::sandbox::{SandboxPolicy, SandboxResult};

        let policy = SandboxPolicy::economy_solver(worktree);
        tracing::info!(
            target: "jfc::sandbox",
            worktree = %worktree.display(),
            "applying sandbox policy to bounty solver verification"
        );
        // Policy is defined; actual enforcement pending landlock crate integration.
        let _result: SandboxResult = policy.apply_to_command(command.as_std_mut());
    }

    match tokio::time::timeout(std::time::Duration::from_secs(120), command.output()).await {
        Ok(Ok(output)) if output.status.success() => MechanisticVerification {
            passed: true,
            summary: format!("{label} passed after applying solution"),
        },
        Ok(Ok(output)) => {
            let mut output_text = String::from_utf8_lossy(&output.stderr).to_string();
            if output_text.trim().is_empty() {
                output_text = String::from_utf8_lossy(&output.stdout).to_string();
            }
            MechanisticVerification {
                passed: false,
                summary: format!(
                    "{label} failed with status {}; output: {}",
                    output.status,
                    truncate_for_verification(output_text.trim(), 800)
                ),
            }
        }
        Ok(Err(e)) => MechanisticVerification {
            passed: false,
            summary: format!("failed to run {label}: {e}"),
        },
        Err(_) => MechanisticVerification {
            passed: false,
            summary: format!("{label} timed out after 120s"),
        },
    }
}

fn verification_command(
    root: &std::path::Path,
) -> Option<(&'static str, &'static [&'static str], &'static str)> {
    if root.join("build.zig").exists() {
        return Some(("zig", &["build"], "zig build"));
    }
    if root.join("Cargo.toml").exists() {
        return Some(("cargo", &["test", "--quiet"], "cargo test --quiet"));
    }
    if root.join("package.json").exists() {
        return Some(("npm", &["test", "--", "--runInBand"], "npm test"));
    }
    if root.join("go.mod").exists() {
        return Some(("go", &["test", "./..."], "go test ./..."));
    }
    if root.join("pyproject.toml").exists() || root.join("pytest.ini").exists() {
        return Some(("python", &["-m", "pytest"], "python -m pytest"));
    }
    None
}

fn truncate_for_verification(text: &str, max: usize) -> String {
    if text.len() <= max {
        return text.to_owned();
    }

    let mut end = max;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}...", &text[..end])
}

/// Recognise our explicit file-block format. Format:
///
///   ===FILE: path/relative/to/cwd===
///   <file contents, any number of lines>
///   ===END===
///
/// Multiple blocks may appear back-to-back. Whitespace around the
/// path is trimmed. Returns (path, contents) pairs in source order.
pub(crate) fn parse_file_blocks(text: &str) -> Vec<(std::path::PathBuf, String)> {
    let mut out = Vec::new();
    let mut rest = text;
    while let Some(start) = rest.find("===FILE:") {
        let after = &rest[start + "===FILE:".len()..];
        let header_end = match after.find("===") {
            Some(i) => i,
            None => break,
        };
        let path_str = after[..header_end].trim();
        let body_start = header_end + 3; // skip "==="
        let body_after_newline = match after[body_start..].find('\n') {
            Some(i) => body_start + i + 1,
            None => break,
        };
        let body_text = &after[body_after_newline..];
        let end_marker = match body_text.find("===END===") {
            Some(i) => i,
            None => break,
        };
        let contents = body_text[..end_marker].to_string();
        if !path_str.is_empty() {
            out.push((std::path::PathBuf::from(path_str), contents));
        }
        rest = &body_text[end_marker + "===END===".len()..];
    }
    out
}

pub(super) fn looks_like_unified_diff(text: &str) -> bool {
    text.lines()
        .any(|l| l.starts_with("diff --git ") || l.starts_with("--- "))
        && text.lines().any(|l| l.starts_with("+++ "))
        && text.lines().any(|l| l.starts_with("@@"))
}

/// Crude parser for the validator's structured output. Tolerant of
/// minor format drift — the model isn't always perfect about
/// "FLAW:" / "CONFIDENCE:" / "TEST:" prefixes. Defaults: confidence
/// 0.0 (low — equivalent to "didn't say"), no flaw, no test.
pub(crate) fn parse_validator_output(text: &str) -> (Option<String>, f32, Option<String>) {
    let mut flaw: Option<String> = None;
    let mut confidence: f32 = 0.0;
    let mut test_code: Option<String> = None;
    let mut current: Option<&str> = None;
    let mut buf = String::new();
    let flush = |k: Option<&str>,
                 buf: &mut String,
                 flaw: &mut Option<String>,
                 conf: &mut f32,
                 test: &mut Option<String>| {
        let v = buf.trim().to_string();
        match k {
            Some("FLAW") => {
                if !v.is_empty() && !v.eq_ignore_ascii_case("none") {
                    *flaw = Some(v);
                }
            }
            Some("CONFIDENCE") => {
                if let Ok(n) = v.trim().parse::<f32>() {
                    *conf = n.clamp(0.0, 1.0);
                }
            }
            Some("TEST") if !v.is_empty() && !v.eq_ignore_ascii_case("none") => {
                *test = Some(v);
            }
            _ => {}
        }
        buf.clear();
    };
    for line in text.lines() {
        let t = line.trim();
        let key = ["FLAW", "CONFIDENCE", "TEST"]
            .iter()
            .find(|k| t.to_uppercase().starts_with(&format!("{k}:")))
            .copied();
        if let Some(k) = key {
            flush(
                current,
                &mut buf,
                &mut flaw,
                &mut confidence,
                &mut test_code,
            );
            current = Some(k);
            if let Some(rest) = t.split_once(':') {
                buf.push_str(rest.1.trim());
            }
        } else if current.is_some() {
            if !buf.is_empty() {
                buf.push('\n');
            }
            buf.push_str(line);
        }
    }
    flush(
        current,
        &mut buf,
        &mut flaw,
        &mut confidence,
        &mut test_code,
    );
    (flaw, confidence, test_code)
}

pub(crate) async fn market_report_string() -> Result<String, String> {
    // Try-lock instead of blocking: when a bounty cycle is running it
    // holds the orchestrator mutex for its full multi-minute duration
    // (solvers + validators round-trip through the LLM under the same
    // lock). Blocking here would freeze every caller — `/market`, the
    // model's own `market_status` tool — until the cycle finishes.
    // Report busy state instead so the user / model can retry.
    let orch = match market_orchestrator().try_lock() {
        Ok(g) => g,
        Err(_) => {
            return Ok("Agent economy is busy executing a bounty cycle. \
                 Spend, trust, and ledger figures will refresh once the cycle \
                 completes — re-run /market in a moment."
                .to_owned());
        }
    };
    let detector = collusion_detector()
        .lock()
        .map_err(|e| format!("collusion detector mutex poisoned: {e}"))?;
    let report = jfc_economy::reporting::MarketReport::generate(&orch, &detector, 0, 0);
    let mut body = format!(
        "**Agent economy snapshot**\n\n\
         - Bounties: {} total ({} active)\n\
         - Spend: {} tok used / {} tok remaining\n\
         - Health (composite): {:.2}{}\n  \
           efficiency={:.2} · fairness={:.2} · trust={:.2} · budget={:.2}",
        report.total_bounties,
        report.active_bounties,
        report.total_spent,
        report.remaining_budget,
        report.health.composite,
        if report.health.is_critical() {
            " **[CRITICAL]**"
        } else {
            ""
        },
        report.health.efficiency,
        report.health.fairness,
        report.health.trust,
        report.health.budget_adherence,
    );
    if !report.flagged_agents.is_empty() {
        body.push_str("\n\n**Flagged agents:**");
        for f in &report.flagged_agents {
            body.push_str(&format!("\n- {f}"));
        }
    }
    Ok(body)
}
