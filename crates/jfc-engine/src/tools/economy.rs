use std::{path::Path, process::Stdio};

use jfc_agent::{AgentRegistry, AgentResult, AgentRole, AgentState};

use super::registry::{
    agent_registry, collusion_detector, market_orchestrator, snapshot_event_sender,
};
use crate::runtime::send_critical;

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis().min(i64::MAX as u128) as i64)
        .unwrap_or(0)
}

fn bounty_id_for_role(role: &AgentRole) -> Option<&str> {
    match role {
        AgentRole::Solver { bounty_id, .. } | AgentRole::Validator { bounty_id } => {
            Some(bounty_id.as_str())
        }
        _ => None,
    }
}

fn role_label(role: &AgentRole) -> &'static str {
    match role {
        AgentRole::Solver { .. } => "solver",
        AgentRole::Validator { .. } => "validator",
        AgentRole::Solo => "solo",
        AgentRole::Teammate { .. } => "teammate",
        AgentRole::Council { .. } => "council",
    }
}

async fn persist_economy_agent_session(
    id: &jfc_agent::AgentId,
    role: &AgentRole,
    task_id: &str,
    model: Option<&str>,
    status: &str,
) {
    let Some(bounty_id) = bounty_id_for_role(role).map(str::to_owned) else {
        return;
    };
    let now = now_ms();
    let row = jfc_knowledge::AgentSessionRow {
        id: id.label().to_owned(),
        parent_session_id: Some(format!("bounty:{bounty_id}")),
        role: role_label(role).to_owned(),
        model: model.map(str::to_owned),
        status: status.to_owned(),
        budget_tokens: None,
        task_id: Some(task_id.to_owned()),
        team_id: Some(bounty_id),
        created_at_ms: now,
        updated_at_ms: now,
    };
    let result = async {
        let store = jfc_knowledge::KnowledgeStore::open_default().await?;
        store.upsert_agent_session(&row).await
    }
    .await;
    if let Err(error) = result {
        tracing::warn!(
            target: "jfc::economy::db",
            agent = %id.label(),
            status,
            error = %error,
            "failed to persist economy agent session"
        );
    }
}

async fn persist_economy_agent_event(
    id: &jfc_agent::AgentId,
    bounty_id: &str,
    kind: &str,
    content: serde_json::Value,
) {
    let row = jfc_knowledge::AgentEventRow {
        id: format!("evt_{}", uuid::Uuid::new_v4().as_simple()),
        session_id: format!("bounty:{bounty_id}"),
        from_agent: Some(id.label().to_owned()),
        to_agent: None,
        kind: kind.to_owned(),
        content: content.to_string(),
        turn_id: None,
        causal_parent_id: None,
        created_at_ms: now_ms(),
    };
    let result = async {
        let store = jfc_knowledge::KnowledgeStore::open_default().await?;
        store.record_agent_event(&row).await
    }
    .await;
    if let Err(error) = result {
        tracing::warn!(
            target: "jfc::economy::db",
            agent = %id.label(),
            bounty_id,
            kind,
            error = %error,
            "failed to persist economy agent event"
        );
    }
}

pub(crate) async fn persist_bounty_event(
    cwd: &std::path::Path,
    bounty_id: &str,
    kind: &str,
    payload: serde_json::Value,
) {
    let project_session_id = format!("project:{}", jfc_knowledge::project_key(cwd));
    let artifact_key = bounty_id.to_owned();
    let value = serde_json::json!({
        "bounty_id": bounty_id,
        "kind": kind,
        "cwd": cwd,
        "payload": payload,
        "updated_at_ms": now_ms(),
    });
    let event = jfc_knowledge::AgentEventRow {
        id: format!("evt_{}", uuid::Uuid::new_v4().as_simple()),
        session_id: project_session_id.clone(),
        from_agent: None,
        to_agent: None,
        kind: format!("bounty.{kind}"),
        content: value.to_string(),
        turn_id: None,
        causal_parent_id: None,
        created_at_ms: now_ms(),
    };
    let result = async {
        let store = jfc_knowledge::KnowledgeStore::open_default().await?;
        let value_json = value.to_string();
        store
            .upsert_session_artifact(&project_session_id, "bounty", &artifact_key, &value_json)
            .await?;
        store
            .append_session_artifact_event(
                &project_session_id,
                "bounty",
                &artifact_key,
                &value_json,
            )
            .await?;
        store.record_agent_event(&event).await
    }
    .await;
    if let Err(error) = result {
        tracing::warn!(
            target: "jfc::economy::db",
            bounty_id,
            kind,
            error = %error,
            "failed to persist bounty lifecycle event"
        );
    }
}

/// Register an economy agent (solver/validator) in the unified registry and
/// mark it `Running`, so it appears in the same roster as every other agent.
///
/// Idempotent across a cycle: `register` overwrites any prior entry for the
/// same stable id, which is what we want when a stable solver/validator id is
/// reused across bounties.
async fn register_economy_agent(
    id: &jfc_agent::AgentId,
    role: AgentRole,
    description: &str,
    task_id: &str,
    model: Option<&str>,
) {
    let registry = agent_registry();
    registry
        .register(AgentState::new(
            id.clone(),
            role.clone(),
            description.to_string(),
        ))
        .await;
    registry
        .update_status(id, jfc_agent::AgentStatus::Running)
        .await;
    persist_economy_agent_session(id, &role, task_id, model, "running").await;
    if let Some(bounty_id) = bounty_id_for_role(&role) {
        persist_economy_agent_event(
            id,
            bounty_id,
            "agent.started",
            serde_json::json!({
                "description": description,
                "role": role_label(&role),
                "task_id": task_id,
                "model": model,
            }),
        )
        .await;
    }
}

/// Mark an economy agent completed in the unified registry, recording the
/// token spend and a short summary.
async fn complete_economy_agent(
    id: &jfc_agent::AgentId,
    bounty_id: &str,
    role: &AgentRole,
    task_id: &str,
    summary: &str,
    tokens: u64,
    elapsed_ms: u64,
) {
    agent_registry()
        .complete(
            id,
            AgentResult {
                id: id.clone(),
                output: summary.to_string(),
                tokens_used: tokens,
                elapsed_ms,
                patch: None,
            },
        )
        .await;
    persist_economy_agent_session(id, role, task_id, None, "completed").await;
    persist_economy_agent_event(
        id,
        bounty_id,
        "agent.completed",
        serde_json::json!({
            "summary": summary,
            "tokens": tokens,
            "elapsed_ms": elapsed_ms,
            "role": role_label(role),
            "task_id": task_id,
        }),
    )
    .await;
}

async fn fail_economy_agent(
    id: &jfc_agent::AgentId,
    bounty_id: &str,
    role: &AgentRole,
    task_id: &str,
    error: &str,
) {
    agent_registry().fail(id, error.to_owned()).await;
    persist_economy_agent_session(id, role, task_id, None, "failed").await;
    persist_economy_agent_event(
        id,
        bounty_id,
        "agent.failed",
        serde_json::json!({
            "error": error,
            "role": role_label(role),
            "task_id": task_id,
        }),
    )
    .await;
}

/// SwarmProvider impl for jfc — delegates to the existing
/// `worktrees` module. Each solver gets a worktree named
/// `economy/<bounty_id>/<agent_id>` so concurrent bounties don't
/// collide. `remove_worktree` is best-effort: a leftover worktree
/// after a crash is cleaned up by the user via `git worktree prune`.
pub struct EconomySwarmProvider {
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
            .label()
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
                    agent = %agent_id.label(),
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
            agent = %agent_id.label(),
            msg = %message.chars().take(200).collect::<String>(),
            "swarm send_message (audit-only stub)"
        );
    }
}

/// AgentInvoker impl for jfc — runs real LLM calls via the
/// configured Provider trait. Each solver / validator call is one
/// `provider.stream(...)` round-trip; the response text becomes the
/// solution patch (for solvers) or the proposed flaw (for
/// validators). Token counts come from the StreamEvent::Usage
/// callback when the provider emits one, otherwise from a 4-chars-
/// per-token byte estimate.
pub struct EconomyAgentInvoker {
    provider: std::sync::Arc<dyn jfc_provider::Provider>,
    model: jfc_provider::ModelId,
    /// Optional UI event channel — when set, every solver / validator
    /// invocation emits TaskStarted before streaming, AgentChunk for
    /// each text delta, and TaskCompleted/Failed at the end. This is
    /// what makes bounty subagents show up in the same fan UI / ctrl+X
    /// panel as regular Task-tool subagents. None is fine for tests.
    event_tx: Option<tokio::sync::mpsc::Sender<crate::runtime::EngineEvent>>,
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
                        tx.send(crate::runtime::EngineEvent::Task(
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
                        tx.try_send(crate::runtime::EngineEvent::Task(
                            crate::runtime::TaskEvent::Progress {
                                task_id: crate::ids::TaskId::from(id),
                                last_tool: None,
                                last_tool_info: None,
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
        // Shared canonical derivation: provider-reported tokens, else
        // floor(chars/4). Routed through TokenUsage::billable_tokens so the
        // economy ledger stays aligned with the advisor and council budgets
        // (previously this path used div_ceil, drifting by up to one token).
        let usage = jfc_provider::TokenUsage {
            input_tokens: input_tokens as usize,
            output_tokens: output_tokens as usize,
            thinking_tokens: None,
            cache_read_tokens: 0,
            cache_creation_tokens: 0,
        };
        let (tokens, _source) = usage.billable_tokens(text.len());
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
                crate::runtime::EngineEvent::Task(crate::runtime::TaskEvent::Started {
                    task_id: crate::ids::TaskId::from(task_id),
                    description: description.to_owned(),
                    // Report the solver/validator model so the
                    // BackgroundTask's `model_used` is populated. Without
                    // it the per-progress token deltas this invoker emits
                    // never roll into `app.engine.usage_by_model` (the handler at
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
                crate::runtime::EngineEvent::Task(crate::runtime::TaskEvent::Completed {
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
                crate::runtime::EngineEvent::Task(crate::runtime::TaskEvent::Failed {
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
        let task_id = format!("economy-solver-{}", prompt.agent_id.label());
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
        // Mirror into the unified agent registry so the solver shows up in the
        // same roster as every other agent, with its bounty + worktree role.
        register_economy_agent(
            &prompt.agent_id,
            jfc_agent::AgentRole::Solver {
                bounty_id: prompt.bounty_id.clone(),
                worktree: prompt.worktree.clone(),
            },
            &desc,
            &task_id,
            Some(self.model.as_str()),
        )
        .await;
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
            launcher: None,
            effort: None,
            name: Some(prompt.agent_id.label().to_string()),
            team_name: None,
            mode: Some("default".to_string()),
            isolation: None, // Worktree already created by SwarmProvider
            parent_task_id: None,
            schema: None,
            allowed_tools: vec![
                "Read".to_string(),
                "Write".to_string(),
                "Edit".to_string(),
                "Bash".to_string(),
                "Grep".to_string(),
                "Glob".to_string(),
                "codegraph_explore".to_string(),
                "codegraph_search".to_string(),
                "codegraph_node".to_string(),
                "codegraph_files".to_string(),
                "codegraph_arch".to_string(),
                "codegraph_callers".to_string(),
                "codegraph_callees".to_string(),
                "codegraph_impact".to_string(),
                "codegraph_paths".to_string(),
                "codegraph_xref".to_string(),
            ],
            disallowed_tools: Vec::new(),
            cwd: None,
        };

        tracing::info!(
            target: "jfc::ui::economy",
            agent = %prompt.agent_id.label(),
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
            let role = jfc_agent::AgentRole::Solver {
                bounty_id: prompt.bounty_id.clone(),
                worktree: prompt.worktree.clone(),
            };
            fail_economy_agent(
                &prompt.agent_id,
                &prompt.bounty_id,
                &role,
                &task_id,
                &result.output,
            )
            .await;
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
        let role = jfc_agent::AgentRole::Solver {
            bounty_id: prompt.bounty_id.clone(),
            worktree: prompt.worktree.clone(),
        };
        complete_economy_agent(
            &solution.agent_id,
            &prompt.bounty_id,
            &role,
            &task_id,
            &summary,
            tokens_estimate,
            elapsed_ms,
        )
        .await;
        Ok(solution)
    }

    async fn invoke_validator(
        &self,
        prompt: jfc_economy::reporting::ValidatorPrompt,
    ) -> Result<jfc_economy::reporting::ValidatorOutcome, String> {
        let task_id = format!("economy-validator-{}", prompt.validator_id.label());
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
        register_economy_agent(
            &prompt.validator_id,
            jfc_agent::AgentRole::Validator {
                bounty_id: prompt.bounty_id.clone(),
            },
            &desc,
            &task_id,
            Some(self.model.as_str()),
        )
        .await;
        let started_at = std::time::Instant::now();
        tracing::debug!(
            target: "jfc::ui::economy",
            validator = %prompt.validator_id.label(),
            bounty_id = %prompt.bounty_id,
            solver = %prompt.solution.agent_id.label(),
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
                let elapsed_ms = started_at.elapsed().as_millis() as u64;
                self.emit_completed(&task_id, &summary, elapsed_ms);
                let role = jfc_agent::AgentRole::Validator {
                    bounty_id: prompt.bounty_id.clone(),
                };
                complete_economy_agent(
                    &prompt.validator_id,
                    &prompt.bounty_id,
                    &role,
                    &task_id,
                    &summary,
                    tokens,
                    elapsed_ms,
                )
                .await;
                Ok(jfc_economy::reporting::ValidatorOutcome {
                    flaw,
                    test_code,
                    confidence,
                    tokens_consumed: tokens,
                })
            }
            Err(e) => {
                self.emit_failed(&task_id, &e);
                let role = jfc_agent::AgentRole::Validator {
                    bounty_id: prompt.bounty_id.clone(),
                };
                fail_economy_agent(&prompt.validator_id, &prompt.bounty_id, &role, &task_id, &e)
                    .await;
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
pub fn split_patch_and_explanation(text: &str) -> (String, String) {
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
pub fn strip_html_tags(html: &str) -> String {
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
pub struct AppliedSolution {
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
pub fn apply_winning_solution(
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
            winner = %sol.agent_id.label(),
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
        winner = %sol.agent_id.label(),
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
pub struct MechanisticVerification {
    pub passed: bool,
    pub summary: String,
}

pub async fn verify_bounty_solution(
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
            agent = %solution.agent_id.label(),
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
pub fn parse_file_blocks(text: &str) -> Vec<(std::path::PathBuf, String)> {
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

pub fn looks_like_unified_diff(text: &str) -> bool {
    text.lines()
        .any(|l| l.starts_with("diff --git ") || l.starts_with("--- "))
        && text.lines().any(|l| l.starts_with("+++ "))
        && text.lines().any(|l| l.starts_with("@@"))
}

/// Crude parser for the validator's structured output. Tolerant of
/// minor format drift — the model isn't always perfect about
/// "FLAW:" / "CONFIDENCE:" / "TEST:" prefixes. Defaults: confidence
/// 0.0 (low — equivalent to "didn't say"), no flaw, no test.
pub fn parse_validator_output(text: &str) -> (Option<String>, f32, Option<String>) {
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

#[derive(Debug, Clone, PartialEq, Eq)]
struct DurableBountyActivity {
    bounty_id: String,
    phase: String,
    task_id: Option<String>,
    winner: Option<String>,
    total_cost: Option<u64>,
    error: Option<String>,
    updated_at_ms: i64,
    agents: Vec<jfc_knowledge::AgentSessionRow>,
    lifecycle: Vec<DurableBountyEvent>,
    agent_events: Vec<DurableAgentEvent>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct DurableBountyEvent {
    kind: String,
    created_at_ms: i64,
    winner: Option<String>,
    total_cost: Option<u64>,
    solver_count: Option<u64>,
    validator_count: Option<u64>,
    error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct DurableAgentEvent {
    agent_id: Option<String>,
    kind: String,
    summary: Option<String>,
    tokens: Option<u64>,
    elapsed_ms: Option<u64>,
}

#[derive(Debug, Clone, PartialEq)]
struct DurableLearningActivity {
    id: String,
    status: String,
    candidate_kind: Option<String>,
    title: String,
    recurrence_count: i64,
    score: Option<f64>,
    fixtures_run: Option<u64>,
    fixtures_passed: Option<u64>,
    updated_at_ms: i64,
}

fn payload_string(
    payload: &serde_json::Map<String, serde_json::Value>,
    key: &str,
) -> Option<String> {
    payload
        .get(key)
        .and_then(serde_json::Value::as_str)
        .map(str::to_owned)
}

fn payload_u64(payload: &serde_json::Map<String, serde_json::Value>, key: &str) -> Option<u64> {
    payload.get(key).and_then(serde_json::Value::as_u64)
}

fn bounty_payload(value: &serde_json::Value) -> serde_json::Map<String, serde_json::Value> {
    value
        .get("payload")
        .and_then(serde_json::Value::as_object)
        .cloned()
        .unwrap_or_default()
}

fn bounty_kind(value: &serde_json::Value) -> String {
    value
        .get("kind")
        .and_then(serde_json::Value::as_str)
        .or_else(|| {
            value
                .get("event_kind")
                .and_then(serde_json::Value::as_str)
                .map(|kind| kind.strip_prefix("bounty.").unwrap_or(kind))
        })
        .unwrap_or("unknown")
        .to_owned()
}

fn durable_bounty_event_from_row(
    row: &jfc_knowledge::SessionArtifactEventRow,
) -> DurableBountyEvent {
    let value = serde_json::from_str::<serde_json::Value>(&row.value_json)
        .unwrap_or(serde_json::Value::Null);
    let payload = bounty_payload(&value);
    DurableBountyEvent {
        kind: bounty_kind(&value),
        created_at_ms: row.created_at_ms,
        winner: payload_string(&payload, "winner"),
        total_cost: payload_u64(&payload, "total_cost"),
        solver_count: payload_u64(&payload, "n_solvers"),
        validator_count: payload_u64(&payload, "n_validators"),
        error: payload_string(&payload, "error"),
    }
}

fn durable_agent_event_from_row(row: &jfc_knowledge::AgentEventRow) -> DurableAgentEvent {
    let value =
        serde_json::from_str::<serde_json::Value>(&row.content).unwrap_or(serde_json::Value::Null);
    DurableAgentEvent {
        agent_id: row.from_agent.clone(),
        kind: row
            .kind
            .strip_prefix("agent.")
            .unwrap_or(row.kind.as_str())
            .to_owned(),
        summary: value
            .get("summary")
            .or_else(|| value.get("error"))
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned),
        tokens: value.get("tokens").and_then(serde_json::Value::as_u64),
        elapsed_ms: value.get("elapsed_ms").and_then(serde_json::Value::as_u64),
    }
}

fn durable_bounty_activity_from_artifact(
    artifact: &jfc_knowledge::SessionArtifactRow,
    agents: Vec<jfc_knowledge::AgentSessionRow>,
    lifecycle: Vec<DurableBountyEvent>,
    agent_events: Vec<DurableAgentEvent>,
) -> DurableBountyActivity {
    let value = serde_json::from_str::<serde_json::Value>(&artifact.value_json)
        .unwrap_or(serde_json::Value::Null);
    let payload = bounty_payload(&value);
    let phase = bounty_kind(&value);
    let bounty_id = value
        .get("bounty_id")
        .and_then(serde_json::Value::as_str)
        .unwrap_or(&artifact.key)
        .to_owned();
    DurableBountyActivity {
        bounty_id,
        phase,
        task_id: payload_string(&payload, "task_id"),
        winner: payload_string(&payload, "winner"),
        total_cost: payload_u64(&payload, "total_cost"),
        error: payload_string(&payload, "error"),
        updated_at_ms: artifact.updated_at_ms,
        agents,
        lifecycle,
        agent_events,
    }
}

fn learning_activity_from_row(
    row: &jfc_knowledge::LearningEventRow,
    project_key: &str,
) -> Option<DurableLearningActivity> {
    let evidence = serde_json::from_str::<serde_json::Value>(&row.verifier_evidence).ok()?;
    if evidence
        .get("project_key")
        .and_then(serde_json::Value::as_str)
        .is_some_and(|key| key != project_key)
    {
        return None;
    }
    let eval = evidence.get("eval").unwrap_or(&serde_json::Value::Null);
    let title = row
        .candidate_rule
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or(&row.id)
        .chars()
        .take(96)
        .collect();
    Some(DurableLearningActivity {
        id: row.id.clone(),
        status: row.status.clone(),
        candidate_kind: evidence
            .get("candidate_kind")
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned),
        title,
        recurrence_count: row.recurrence_count,
        score: eval.get("score").and_then(serde_json::Value::as_f64),
        fixtures_run: eval.get("fixtures_run").and_then(serde_json::Value::as_u64),
        fixtures_passed: eval
            .get("fixtures_passed")
            .and_then(serde_json::Value::as_u64),
        updated_at_ms: row.updated_at_ms,
    })
}

fn format_ms_timestamp(ms: i64) -> String {
    if ms <= 0 {
        return "unknown time".to_owned();
    }
    if let Some(dt) = chrono::DateTime::<chrono::Utc>::from_timestamp_millis(ms) {
        return dt.format("%Y-%m-%d %H:%M UTC").to_string();
    }
    format!("{ms}ms")
}

fn status_counts(agents: &[jfc_knowledge::AgentSessionRow]) -> Vec<String> {
    let mut counts = std::collections::BTreeMap::<(String, String), usize>::new();
    for agent in agents {
        *counts
            .entry((agent.role.clone(), agent.status.clone()))
            .or_default() += 1;
    }
    counts
        .into_iter()
        .map(|((role, status), count)| format!("{count} {role} {status}"))
        .collect()
}

fn lifecycle_label(event: &DurableBountyEvent) -> String {
    let mut label = event.kind.clone();
    if let Some(count) = event.solver_count {
        label.push_str(&format!(
            " · {count} solver{}",
            if count == 1 { "" } else { "s" }
        ));
    }
    if let Some(count) = event.validator_count {
        label.push_str(&format!(
            " · {count} validator{}",
            if count == 1 { "" } else { "s" }
        ));
    }
    if let Some(winner) = &event.winner {
        label.push_str(&format!(" · winner `{winner}`"));
    }
    if let Some(cost) = event.total_cost {
        label.push_str(&format!(" · cost {cost} tok"));
    }
    if let Some(error) = &event.error {
        let preview: String = error.chars().take(96).collect();
        label.push_str(&format!(" · error: {preview}"));
    }
    label
}

fn format_durable_market_activity(
    entries: &[DurableBountyActivity],
    learning: &[DurableLearningActivity],
) -> String {
    if entries.is_empty() && learning.is_empty() {
        return String::new();
    }
    let mut body = String::from("\n\n**Durable market activity**");
    for entry in entries {
        body.push_str(&format!(
            "\n- bounty `{}` · {} · updated {}",
            entry.bounty_id,
            entry.phase,
            format_ms_timestamp(entry.updated_at_ms)
        ));
        if let Some(task_id) = &entry.task_id {
            body.push_str(&format!(" · task `{task_id}`"));
        }
        if let Some(winner) = &entry.winner {
            body.push_str(&format!(" · winner `{winner}`"));
        }
        if let Some(total_cost) = entry.total_cost {
            body.push_str(&format!(" · cost {total_cost} tok"));
        }
        if let Some(error) = &entry.error {
            let preview: String = error.chars().take(120).collect();
            body.push_str(&format!(" · error: {preview}"));
        }
        if !entry.lifecycle.is_empty() {
            let lifecycle = entry
                .lifecycle
                .iter()
                .take(5)
                .map(lifecycle_label)
                .collect::<Vec<_>>()
                .join(" -> ");
            body.push_str(&format!("\n  lifecycle: {lifecycle}"));
        }
        let counts = status_counts(&entry.agents);
        if !counts.is_empty() {
            body.push_str(&format!("\n  agents: {}", counts.join(" · ")));
        }
        if !entry.agents.is_empty() {
            for agent in entry.agents.iter().take(6) {
                body.push_str(&format!(
                    "\n  - {} `{}` {}{}",
                    agent.role,
                    agent.id,
                    agent.status,
                    agent
                        .model
                        .as_deref()
                        .map(|model| format!(" · {model}"))
                        .unwrap_or_default()
                ));
            }
        }
        let completed_events: Vec<_> = entry
            .agent_events
            .iter()
            .filter(|event| event.kind == "completed" || event.kind == "failed")
            .take(4)
            .collect();
        if !completed_events.is_empty() {
            body.push_str("\n  settlement rows:");
            for event in completed_events {
                let mut line = format!(
                    "\n  - {} {}",
                    event.agent_id.as_deref().unwrap_or("agent"),
                    event.kind
                );
                if let Some(tokens) = event.tokens {
                    line.push_str(&format!(" · {tokens} tok"));
                }
                if let Some(elapsed_ms) = event.elapsed_ms {
                    line.push_str(&format!(" · {:.1}s", elapsed_ms as f64 / 1000.0));
                }
                if let Some(summary) = &event.summary {
                    let preview: String = summary.chars().take(96).collect();
                    line.push_str(&format!(" · {preview}"));
                }
                body.push_str(&line);
            }
        }
    }
    if !learning.is_empty() {
        body.push_str(&format!("\n- learning rows · {} recent", learning.len()));
        for row in learning.iter().take(6) {
            body.push_str(&format!(
                "\n  - {} `{}` · {} · recur {} · updated {}",
                row.candidate_kind.as_deref().unwrap_or("candidate"),
                row.id,
                row.status,
                row.recurrence_count,
                format_ms_timestamp(row.updated_at_ms)
            ));
            if let Some(score) = row.score {
                body.push_str(&format!(" · score {score:.2}"));
            }
            if let (Some(passed), Some(run)) = (row.fixtures_passed, row.fixtures_run) {
                body.push_str(&format!(" · fixtures {passed}/{run}"));
            }
            body.push_str(&format!("\n    {}", row.title));
        }
    }
    body
}

async fn durable_market_activity_from_store(
    store: &jfc_knowledge::KnowledgeStore,
    cwd: &Path,
    limit: usize,
) -> Result<String, String> {
    let session_id = format!("project:{}", jfc_knowledge::project_key(cwd));
    let project_key = jfc_knowledge::project_key(cwd);
    let artifacts = store
        .list_session_artifacts(&session_id, "bounty", limit)
        .await
        .map_err(|error| error.to_string())?;
    let mut entries = Vec::new();
    for artifact in artifacts {
        let bounty_id = serde_json::from_str::<serde_json::Value>(&artifact.value_json)
            .ok()
            .and_then(|value| {
                value
                    .get("bounty_id")
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_owned)
            })
            .unwrap_or_else(|| artifact.key.clone());
        let agents = store
            .list_agent_sessions_by_team(&bounty_id, 12)
            .await
            .map_err(|error| error.to_string())?;
        let lifecycle = store
            .list_session_artifact_events(&session_id, "bounty", Some(&artifact.key), 30)
            .await
            .map_err(|error| error.to_string())?
            .iter()
            .map(durable_bounty_event_from_row)
            .collect();
        let agent_events = store
            .list_agent_events(&format!("bounty:{bounty_id}"), 30)
            .await
            .map_err(|error| error.to_string())?
            .iter()
            .map(durable_agent_event_from_row)
            .collect();
        entries.push(durable_bounty_activity_from_artifact(
            &artifact,
            agents,
            lifecycle,
            agent_events,
        ));
    }
    let learning = store
        .list_learning_events(None, 12)
        .await
        .map_err(|error| error.to_string())?
        .iter()
        .filter_map(|row| learning_activity_from_row(row, &project_key))
        .collect::<Vec<_>>();
    Ok(format_durable_market_activity(&entries, &learning))
}

async fn durable_market_activity(cwd: &Path) -> Result<String, String> {
    let store = jfc_knowledge::KnowledgeStore::open_default()
        .await
        .map_err(|error| error.to_string())?;
    durable_market_activity_from_store(&store, cwd, 10).await
}

pub async fn market_report_string() -> Result<String, String> {
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

pub async fn market_report_string_for_cwd(cwd: &Path) -> Result<String, String> {
    let mut body = market_report_string().await?;
    match durable_market_activity(cwd).await {
        Ok(activity) => body.push_str(&activity),
        Err(error) => {
            body.push_str(&format!(
                "\n\n**Durable bounty activity**\n- unavailable: {error}"
            ));
        }
    }
    Ok(body)
}

#[cfg(test)]
mod durable_market_tests {
    use super::*;

    #[tokio::test]
    async fn durable_market_activity_groups_bounty_agents_normal() {
        let store = jfc_knowledge::KnowledgeStore::open_in_memory()
            .await
            .unwrap();
        let cwd = Path::new("/tmp/jfc-market-test");
        let session_id = format!("project:{}", jfc_knowledge::project_key(cwd));
        let value = serde_json::json!({
            "bounty_id": "bounty_1",
            "kind": "settled",
            "payload": {
                "task_id": "t3",
                "winner": "solver-0",
                "total_cost": 321
            }
        })
        .to_string();
        store
            .upsert_session_artifact(&session_id, "bounty", "bounty_1", &value)
            .await
            .unwrap();
        let posted = serde_json::json!({
            "bounty_id": "bounty_1",
            "kind": "posted",
            "payload": {
                "task_id": "t3"
            }
        })
        .to_string();
        store
            .append_session_artifact_event(&session_id, "bounty", "bounty_1", &posted)
            .await
            .unwrap();
        let started = serde_json::json!({
            "bounty_id": "bounty_1",
            "kind": "dispatch_started",
            "payload": {
                "n_solvers": 2,
                "n_validators": 1
            }
        })
        .to_string();
        store
            .append_session_artifact_event(&session_id, "bounty", "bounty_1", &started)
            .await
            .unwrap();
        store
            .append_session_artifact_event(&session_id, "bounty", "bounty_1", &value)
            .await
            .unwrap();
        let now = now_ms();
        store
            .upsert_agent_session(&jfc_knowledge::AgentSessionRow {
                id: "solver-0".into(),
                parent_session_id: Some("bounty:bounty_1".into()),
                role: "solver".into(),
                model: Some("haiku".into()),
                status: "completed".into(),
                budget_tokens: None,
                task_id: Some("economy-solver-0".into()),
                team_id: Some("bounty_1".into()),
                created_at_ms: now,
                updated_at_ms: now,
            })
            .await
            .unwrap();
        store
            .record_agent_event(&jfc_knowledge::AgentEventRow {
                id: "evt_solver_done".into(),
                session_id: "bounty:bounty_1".into(),
                from_agent: Some("solver-0".into()),
                to_agent: None,
                kind: "agent.completed".into(),
                content: serde_json::json!({
                    "summary": "12 bytes patch",
                    "tokens": 144,
                    "elapsed_ms": 2300
                })
                .to_string(),
                turn_id: None,
                causal_parent_id: None,
                created_at_ms: now,
            })
            .await
            .unwrap();
        store
            .record_learning_event(&jfc_knowledge::LearningEventRow {
                id: "rsi:candidate-1".into(),
                source_session_id: Some("session-1".into()),
                source_turn_id: None,
                source_tool_run_id: None,
                candidate_rule:
                    "Context Playbook: use bounty fanout when validators can check the result."
                        .into(),
                status: "candidate".into(),
                verifier_evidence: serde_json::json!({
                    "project_key": jfc_knowledge::project_key(cwd),
                    "candidate_kind": "context_playbook",
                    "eval": {
                        "score": 0.82,
                        "fixtures_run": 3,
                        "fixtures_passed": 3
                    }
                })
                .to_string(),
                recurrence_count: 2,
                created_at_ms: now,
                updated_at_ms: now,
            })
            .await
            .unwrap();

        let rendered = durable_market_activity_from_store(&store, cwd, 10)
            .await
            .unwrap();

        assert!(rendered.contains("**Durable market activity**"));
        assert!(rendered.contains("bounty `bounty_1` · settled"));
        assert!(rendered.contains("task `t3`"));
        assert!(rendered.contains("winner `solver-0`"));
        assert!(rendered.contains("cost 321 tok"));
        assert!(rendered.contains("lifecycle: posted -> dispatch_started"));
        assert!(rendered.contains("2 solvers"));
        assert!(rendered.contains("agents: 1 solver completed"));
        assert!(rendered.contains("solver `solver-0` completed · haiku"));
        assert!(rendered.contains("settlement rows:"));
        assert!(rendered.contains("solver-0 completed · 144 tok · 2.3s · 12 bytes patch"));
        assert!(rendered.contains("learning rows · 1 recent"));
        assert!(rendered.contains("context_playbook `rsi:candidate-1` · candidate"));
        assert!(rendered.contains("fixtures 3/3"));
    }
}
