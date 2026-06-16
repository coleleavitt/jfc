//! Engine-side slash-command semantics — stage 8 of the jfc-engine
//! extraction. Every command whose behavior is engine state + events lives
//! here, dispatched through [`run_command`]; frontends keep only their
//! view commands (theme/vim/help/copy/panels) and fall through to this
//! registry, so headless and remote surfaces get the same `/compact`,
//! `/model`, `/task-*`, ... vocabulary as the TUI.

pub mod account;
pub mod automation;
pub mod context;
pub mod delegating;
pub mod github;
pub mod info;
pub mod local;
pub mod markdown;
pub mod mcp;
pub mod session;
pub mod support;
pub mod task;
pub mod worktree;

/// Shared imports for the command handler modules (they historically lived
/// inside the TUI's `input` module and leaned on its glob surface).
pub mod prelude {
    pub use std::{path::PathBuf, sync::Arc};

    pub use tokio::sync::mpsc;

    pub use crate::app::{EngineEffect, EngineEvent, EngineState, PendingApproval, PermissionMode};
    pub use crate::runtime::{
        ControlEvent, EventSender, FrontendEvent, QueuePriority, QueuedPrompt, StreamEvent,
        ToolEvent,
    };
    pub use crate::types::*;
    pub use crate::{config, toast};
}

use prelude::*;

use account::*;
use context::*;
use delegating::*;
use info::*;
use session::*;
use task::*;

/// What [`run_command`] did with the line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandOutcome {
    /// The engine executed the command (including skill fallthrough — even
    /// an unknown name produces a transcript reply).
    Handled,
    /// The name is not an engine command: the frontend should try its own
    /// view-command table before calling this.
    Unknown,
}

macro_rules! engine_commands {
    (
        $( $canon:literal $([ $($alias:literal),* $(,)? ])? $help:literal => $handler:ident ),* $(,)?
    ) => {
        /// Every engine-resident slash command with a one-line description.
        /// Frontends merge this with their view-command table for
        /// autocomplete/help.
        pub const ENGINE_SLASH_COMMANDS: &[(&str, &str)] = &[
            $(
                ($canon, $help),
                $( $( ($alias, $help), )* )?
            )*
        ];

        async fn dispatch_engine(
            state: &mut EngineState,
            parts: &[&str],
            text: &str,
            tx: Option<&mpsc::Sender<EngineEvent>>,
        ) -> bool {
            match parts[0] {
                $(
                    $canon $( $(| $alias)* )? => {
                        $handler(state, parts, text, tx).await;
                        true
                    }
                )*
                _ => false,
            }
        }
    };
}

engine_commands! {
        "/rename" [] "set a custom title on the current session" => cmd_rename,
        "/clear" [] "clear the conversation and start fresh" => cmd_clear,
        "/recap" [] "generate a one-line session recap now" => cmd_recap,
        "/check" [] "re-run cargo-check diagnostics" => cmd_check,
        "/compact" [] "summarize earlier messages to free context" => cmd_compact,
        "/expand" [] "open raw messages saved before compaction (`/expand <archive-id>`)" => cmd_expand,
        "/advisor" [] "ask a parallel advisor without disturbing the main agent" => cmd_advisor,
        "/council" [] "convene a multi-model council: fan a question to N models, synthesise (`/council <model-a,model-b> <question>`)" => cmd_council,
        "/research" [] "deep research: plan sub-queries, search the web in steps, synthesise (`/research <question>`)" => cmd_research,
        "/brief" [] "toggle brief-only mode (hide plain text, only show SendUserMessage output)" => cmd_brief,
        "/autoloop" [] "start an autonomous loop tick (reads loop.md)" => cmd_autoloop,
        "/sandbox" [] "toggle bash sandbox (bwrap network isolation)" => cmd_sandbox,
        "/team-onboarding" [] "generate a team onboarding guide from project state" => cmd_team_onboarding,
        "/coach" [] "show coaching tips based on this session's tool usage" => cmd_coach,
        "/remote" [] "spawn a remote CCR session (requires ANTHROPIC_API_KEY)" => cmd_remote,
        "/factory" [] "show factory throughput, success rate, rework, and attempt metrics" => cmd_factory,
        "/oauth-login" [] "start OAuth device-flow login (RFC 8628)" => cmd_oauth_login,
        "/config" [] "show parsed config (`/config path` for the file location)" => cmd_config,
        "/continue" ["/c"] "resume the most recent session (`/continue all` for any cwd)" => cmd_continue,
        "/resume" [] "resume a specific session by id" => cmd_resume,
        "/sessions" [] "list all saved sessions" => cmd_sessions,
        "/workflow" ["/wf", "/workflows"] "list running + available workflows; run a named workflow" => cmd_workflow,
        "/login" [] "authenticate with a provider (browser flow)" => cmd_login,
        "/logout" [] "wipe stored credentials" => cmd_logout,
        "/release-notes" ["/releasenotes", "/changelog"] "show the changelog" => cmd_release_notes,
        "/feedback" [] "open the GitHub issue tracker" => cmd_feedback,
        "/upgrade" [] "show how to upgrade jfc" => cmd_upgrade,
        "/fork" [] "snapshot the first N messages as a new session" => cmd_fork,
        "/batch" [] "submit a prompt-file via the Message Batches API" => cmd_batch,
        "/diff" [] "show pending uncommitted changes" => cmd_diff,
        "/turn-diff" ["/td"] "diff only the files edited in the current turn" => cmd_turn_diff,
        "/undo" [] "revert the most recent file mutation by a tool" => cmd_undo,
        "/export" [] "save the transcript as markdown" => cmd_export,
        "/fast" ["/f"] "toggle low-latency fast-mode inference" => cmd_fast,
        "/model" [] "switch model immediately (`/model <name>`)" => cmd_model,
        "/pin" [] "pin a message so compaction can't drop it" => cmd_pin,
        "/unpin" [] "remove a message pin" => cmd_unpin,
        "/timeline" [] "tool-call timeline for the last assistant turn" => cmd_timeline,
        "/doctor" [] "health-check the jfc setup" => cmd_doctor,
        "/effort" [] "pin reasoning effort (low/medium/high/xhigh/max)" => cmd_effort,
        "/temp" ["/temperature"] "pin sampling temperature (0..2, clear)" => cmd_temp,
        "/explore" [] "raise adaptive exploration level" => cmd_explore,
        "/focus" [] "lower adaptive exploration level" => cmd_focus,
        "/feature" [] "list / toggle feature gates" => cmd_feature,
        "/goal" [] "set a session stop-condition the agent works toward" => cmd_goal,
        "/memory" ["/mem"] "list memory files / toggle two-phase recall" => cmd_memory,
        "/commit" [] "generate a conventional commit message for staged changes" => cmd_commit,
        "/review" ["/code-review", "/ultrareview"] "ask the model to review current git changes" => cmd_review,
        "/skills" [] "list available skills (.claude/skills/*.md)" => cmd_skills,
        "/recall" ["/search-sessions"] "search past sessions + commits (`/recall <query>`)" => cmd_recall,
        "/agents" [] "list available agent definitions (.claude/agents/*.md)" => cmd_agents,
        "/market" [] "show the agent-economy snapshot" => cmd_market,
        "/task-list" ["/tasks"] "list todo/task items" => cmd_task_list,
        "/task-add" [] "create a new task" => cmd_task_add,
        "/task-done" [] "mark a task completed" => cmd_task_done,
        "/task-rm" ["/task-delete"] "delete a task" => cmd_task_rm,
        "/claude-md" [] "show which CLAUDE.md layers are loaded" => cmd_claude_md,
        "/mode" [] "switch permission mode (default/plan/accept/auto/bypass)" => cmd_mode,
        "/auto-mode" [] "toggle the autonomous tool classifier" => cmd_auto_mode,
        "/worktree" [] "create / list / remove a git worktree" => cmd_worktree,
        "/mcp" [] "list / inspect configured MCP servers" => cmd_mcp,
        "/teleport" [] "jump into a teammate's context" => cmd_teleport,
        "/fleet" ["/fleetview"] "show the teammate fleet view" => cmd_fleet,
        "/init" [] "scaffold a CLAUDE.md for this project" => cmd_init,
        "/plan" [] "draft or update PLAN.md (Atlas-compatible)" => cmd_plan,
        "/roadmap" [] "draft or update ROADMAP.md (stable decimal IDs)" => cmd_roadmap,
        "/parity" [] "draft or update PARITY.md (evidence required)" => cmd_parity,
        "/philosophy" [] "draft or update PHILOSOPHY.md" => cmd_philosophy,
        "/usage" [] "draft or update USAGE.md (operator commands)" => cmd_usage,
        "/cost" ["/stats"] "show session cost / token usage" => cmd_cost,
        "/audit" [] "show the runtime audit ledger (agent actions)" => cmd_audit,
        "/changes" [] "list/show/apply/revert agent change-sets" => cmd_changes,
        "/commands" [] "unified command/tool list across CLI, slash, and tools" => cmd_commands,
        "/status" [] "show current session status" => cmd_status,
        "/bug" [] "file a bug report with session context" => cmd_bug,
        "/rewind" [] "rewind the transcript to an earlier checkpoint" => cmd_rewind,
        "/output-style" ["/style"] "switch output style (e.g. brief)" => cmd_output_style,
        "/dump-context" ["/debug-context"] "show what the model sees: memories, skills, tools" => cmd_dump_context,
        "/install-github-app" [] "install the Claude GitHub App on this repo" => cmd_install_github_app,
        "/pr" [] "show a PR + review comments (`/pr <num>`)" => cmd_pr,
        "/pr-autofix" [] "ask the model to fix PR review comments" => cmd_pr_autofix,
        "/setup-github-actions" [] "scaffold .github/workflows/jfc-review.yml" => cmd_setup_github_actions,
        "/dream" ["/learn"] "run a background self-improvement / learning pass" => cmd_dream,
        "/loop" ["/proactive"] "toggle proactive autonomous looping" => cmd_loop,
        "/schedule" ["/routines"] "manage scheduled routines" => cmd_schedule,
        "/swarm-approve" ["/swarm-deny"] "approve a pending swarm tool request" => cmd_swarm_approve,
        "/permissions" [] "list/add permission allow/deny rules" => cmd_permissions,
        "/stuck" [] "run diagnostic checks (processes, memory, streams)" => cmd_stuck,
        "/teleport-export" [] "export current plan/context as importable JSON" => cmd_teleport_export,
        "/babysit-prs" [] "watch open PRs (optional schedule arg, `/babysit-prs stop` to cancel)" => cmd_babysit_prs,
        "/morning-checkin" [] "daily brief of PRs, issues, and recent commits" => cmd_morning_checkin,
         "/btw" [] "ask a quick side question without interrupting current work" => cmd_btw,
        "/cd" [] "change the engine working directory mid-session (`/cd <path>`)" => cmd_cd,
         "/queue" [] "show queued messages (or `/queue clear` to discard them)" => cmd_queue,
        "/hooks" [] "show registered hooks with per-rule activation metrics" => cmd_hooks,
}

/// Run one slash-command line against the engine. Unknown names resolve as
/// skills (inline expansion); names that are neither commands nor skills
/// still produce a transcript reply, so the outcome is `Handled` for every
/// engine path — `Unknown` is reserved for view commands the frontend owns.
pub async fn run_command(
    state: &mut EngineState,
    text: &str,
    tx: Option<&mpsc::Sender<EngineEvent>>,
) -> CommandOutcome {
    let parts: Vec<&str> = text.splitn(2, ' ').collect();
    if dispatch_engine(state, &parts, text, tx).await {
        return CommandOutcome::Handled;
    }
    skill_fallthrough(state, &parts, text, tx).await;
    CommandOutcome::Handled
}

/// Skill-name fallthrough for any `/<name>` not bound above: `/<skill>`
/// invokes the matching skill body as if the user had pasted it. Mirrors
/// v126 cli.js:226634 where a slash-name not otherwise bound resolves to a
/// skill or markdown command and either inline-expands or forks a subagent.
///
/// When `frontmatter.context == "fork"`, dispatch the rendered skill body to
/// the existing Task subagent executor. Inline skills still stream through
/// the normal provider path as a synthetic user turn.
pub async fn skill_fallthrough(
    state: &mut EngineState,
    parts: &[&str],
    _text: &str,
    tx: Option<&mpsc::Sender<EngineEvent>>,
) {
    let name = parts[0].trim_start_matches('/');
    let cwd = PathBuf::from(&state.cwd);
    let mut markdown_commands = markdown::load_markdown_commands(&cwd);
    let mut skills = crate::agents::load_skills(&cwd);
    if markdown::find_markdown_command(&markdown_commands, name).is_none()
        && crate::agents::find_skill_by_name(&skills, name).is_none()
        && refresh_plugins_on_miss(&cwd, name).await
    {
        markdown_commands = markdown::load_markdown_commands(&cwd);
        skills = crate::agents::load_skills(&cwd);
    }

    if let Some(command) = markdown::find_markdown_command(&markdown_commands, name).cloned() {
        let echo = if let Some(rest) = parts.get(1) {
            let trimmed = rest.trim();
            if trimmed.is_empty() {
                format!("/{name}")
            } else {
                format!("/{name} {trimmed}")
            }
        } else {
            format!("/{name}")
        };
        let Some(tx) = tx else {
            state.messages.push(ChatMessage::assistant(format!(
                "Markdown command `/{name}` cannot be invoked from this context (no stream channel). \
                 Submit `/{name}` directly from the input bar instead."
            )));
            state.push_effect(crate::app::EngineEffect::ScrollToBottom);
            return;
        };
        let args = parts.get(1).map(|s| s.trim()).filter(|s| !s.is_empty());
        let body = markdown::render_markdown_command(&command, args);

        // CC 2.1.167 UserPromptExpansion hook for markdown commands.
        {
            let session_id = state
                .current_session_id
                .as_ref()
                .map(|s| s.as_str().to_owned())
                .unwrap_or_default();
            crate::hooks::fire_async(
                crate::hooks::HookPoint::OnUserPromptExpansion,
                &crate::hooks::HookContext::for_session(&session_id)
                    .with_extra("expansion_type", "markdown_command")
                    .with_extra("command_name", name.to_owned())
                    .with_extra("command_source", "markdown"),
            );
        }

        state.messages.push(ChatMessage::user(echo));
        start_synthetic_user_turn(state, body, tx);
        return;
    }

    if let Some(skill) = crate::agents::find_skill_by_name(&skills, name).cloned() {
        if !skill.is_user_invocable() {
            state.messages.push(ChatMessage::assistant(format!(
                "Skill `/{name}` is installed but is not user-invocable."
            )));
            state.push_effect(crate::app::EngineEffect::ScrollToBottom);
            return;
        }

        // Echo the user's invocation so the chat shows what they
        // typed (with optional args) — same pattern as the other
        // slash arms. The injected user message that follows carries
        // the skill body, which is what the model actually sees.
        let echo = if let Some(rest) = parts.get(1) {
            let trimmed = rest.trim();
            if trimmed.is_empty() {
                format!("/{name}")
            } else {
                format!("/{name} {trimmed}")
            }
        } else {
            format!("/{name}")
        };
        state.messages.push(ChatMessage::user(echo));

        let memory_root = jfc_memory::project_memory_dir(&cwd);
        let render_context = crate::agents::SkillRenderContext::new(Some(&cwd), Some(&memory_root));
        let args = parts.get(1).map(|s| s.trim()).filter(|s| !s.is_empty());
        let body = crate::agents::render_skill_invocation(&skill, render_context, args);

        // CC 2.1.167 UserPromptExpansion hook — fires after the skill body is
        // rendered but before it is submitted to the model. Fire-and-forget;
        // hooks cannot currently veto a skill expansion (non-blocking path).
        {
            let session_id = state
                .current_session_id
                .as_ref()
                .map(|s| s.as_str().to_owned())
                .unwrap_or_default();
            crate::hooks::fire_async(
                crate::hooks::HookPoint::OnUserPromptExpansion,
                &crate::hooks::HookContext::for_session(&session_id)
                    .with_extra("expansion_type", "skill")
                    .with_extra("command_name", name.to_owned())
                    .with_extra("command_source", "skill_registry"),
            );
        }

        if skill.context.is_fork() {
            let Some(tx) = tx else {
                state.messages.push(ChatMessage::assistant(format!(
                    "Skill `/{name}` requires a forked subagent, but this context has no stream channel. \
                         Submit `/{name}` directly from the input bar instead."
                )));
                state.push_effect(crate::app::EngineEffect::ScrollToBottom);
                return;
            };

            let task_input = TaskInput {
                description: format!("Run skill /{}", skill.name),
                prompt: body,
                subagent_type: None,
                category: Some("skill".to_owned()),
                run_in_background: false,
                model: None,
                effort: None,
                name: None,
                team_name: None,
                mode: None,
                isolation: None,
                parent_task_id: None,
                schema: skill.input_schema.clone(),
            };
            let provider = state.provider.clone();
            let model = state.model.clone();
            let task_store = Some(state.task_store.clone());
            let result = crate::tools::execute_task(
                &task_input,
                provider.as_ref(),
                model,
                Some(tx),
                None,
                None,
                Some(cwd.clone()),
                task_store,
                None,
            )
            .await;
            let reply = if result.is_error() {
                format!(
                    "Skill `/{name}` failed in forked context:\n\n{}",
                    result.output
                )
            } else {
                result.output
            };
            state.messages.push(ChatMessage::assistant(reply));
            state.push_effect(crate::app::EngineEffect::ScrollToBottom);
            return;
        }

        let Some(tx) = tx else {
            // No tx in this dispatch path (e.g. queued-prompt drain).
            // Fall back to a hint rather than silently swallowing the
            // invocation.
            state.messages.push(ChatMessage::assistant(format!(
                "Skill `/{name}` cannot be invoked from this context (no stream channel). \
                         Submit `/{name}` directly from the input bar instead."
            )));
            state.push_effect(crate::app::EngineEffect::ScrollToBottom);
            return;
        };

        start_synthetic_user_turn(state, body, tx);
        return;
    }

    state.messages.push(ChatMessage::assistant(format!(
        "Unknown command: `{}`. Type `/help` for available commands.",
        parts[0]
    )));
}

async fn refresh_plugins_on_miss(cwd: &std::path::Path, command_name: &str) -> bool {
    if !plugin_refresh_on_miss_enabled() {
        return false;
    }
    if crate::config::load_managed_settings()
        .as_ref()
        .is_some_and(|settings| settings.disable_plugin_updates)
    {
        tracing::debug!(
            target: "jfc::plugins",
            command = command_name,
            "plugin refresh-on-miss skipped by managed settings"
        );
        return false;
    }

    let roots = git_plugin_roots(cwd);
    if roots.is_empty() {
        return false;
    }
    let mut changed = false;
    for root in roots {
        let status = tokio::time::timeout(
            std::time::Duration::from_secs(8),
            tokio::process::Command::new("git")
                .arg("-C")
                .arg(&root)
                .arg("pull")
                .arg("--ff-only")
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status(),
        )
        .await;
        match status {
            Ok(Ok(status)) if status.success() => {
                changed = true;
                tracing::info!(
                    target: "jfc::plugins",
                    command = command_name,
                    plugin = %root.display(),
                    "refreshed git plugin after slash-command miss"
                );
            }
            Ok(Ok(status)) => {
                tracing::warn!(
                    target: "jfc::plugins",
                    command = command_name,
                    plugin = %root.display(),
                    %status,
                    "plugin refresh-on-miss git pull failed"
                );
            }
            Ok(Err(error)) => {
                tracing::warn!(
                    target: "jfc::plugins",
                    command = command_name,
                    plugin = %root.display(),
                    %error,
                    "plugin refresh-on-miss could not run git"
                );
            }
            Err(_) => {
                tracing::warn!(
                    target: "jfc::plugins",
                    command = command_name,
                    plugin = %root.display(),
                    "plugin refresh-on-miss timed out"
                );
            }
        }
    }
    changed
}

fn plugin_refresh_on_miss_enabled() -> bool {
    [
        "JFC_PLUGIN_REFRESH_ON_MISS",
        "JFC_PLUGIN_AUTOUPDATE",
        "CLAUDE_CODE_PLUGIN_REFRESH_ON_MISS",
        "CLAUDE_CODE_ENABLE_BACKGROUND_PLUGIN_REFRESH",
    ]
    .iter()
    .find_map(|key| std::env::var(key).ok())
    .map(|value| {
        let normalized = value.trim().to_ascii_lowercase();
        matches!(normalized.as_str(), "1" | "true" | "yes" | "on")
    })
    .unwrap_or(false)
}

fn git_plugin_roots(project_root: &std::path::Path) -> Vec<PathBuf> {
    let settings = crate::config::claude_settings::load_merged(project_root);
    let mut plugin_dirs = Vec::new();
    if let Some(home) = dirs::home_dir() {
        plugin_dirs.push(home.join(".claude/plugins"));
    }
    if let Some(config) = dirs::config_dir() {
        plugin_dirs.push(config.join("jfc/plugins"));
    }
    plugin_dirs.push(project_root.join("plugins"));
    plugin_dirs.push(project_root.join(".claude/plugins"));
    plugin_dirs.push(project_root.join(".agents/plugins"));
    plugin_dirs.push(project_root.join(".codex/plugins"));

    let mut roots = Vec::new();
    for plugin_dir in plugin_dirs {
        let Ok(entries) = std::fs::read_dir(plugin_dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.join(".git").is_dir() {
                continue;
            }
            let plugin_name = path
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("");
            if settings.plugin_enabled(plugin_name) {
                roots.push(path);
            }
        }
    }
    roots.sort();
    roots.dedup();
    roots
}

fn start_synthetic_user_turn(
    state: &mut EngineState,
    body: String,
    tx: &mpsc::Sender<EngineEvent>,
) {
    let assistant_idx = state.messages.len() + 1;
    state.messages.push(ChatMessage::user(body));
    state.tool_ctx.total_user_turns += 1;
    state.messages.push(ChatMessage::assistant(String::new()));
    let identity = crate::cache_lineage::current_identity(state);
    crate::cache_lineage::stamp_assistant(&mut state.messages, assistant_idx, &identity);
    state.streaming_text.clear();
    state.streaming_reasoning.clear();
    state.streaming_response_bytes = 0;
    state.network_recovery_status = None;
    state.network_recovery_attempts = 0;
    state.streaming_assistant_idx = Some(assistant_idx);
    state.is_streaming = true;
    let now = std::time::Instant::now();
    state.streaming_started_at = Some(now);
    state.last_stream_event_at = Some(now);
    state.streaming_last_token_at = Some(now);
    state.turn_started_at = Some(now);
    state.turn_start_cost = crate::cost::total_cost(&state.usage_by_model);
    state.agentic_turn_count = 0;
    state.thinking_started_at = None;
    state.pre_dispatched_tool_ids.clear();
    state.deferred_tool_uses.clear();
    state.in_progress_tool_use_ids.clear();
    state.in_flight_eager_dispatches = 0;
    state.in_flight_tool_batches = 0;
    state.thinking_ended_at = None;
    state.last_usage_output = 0;
    state.usage_apply_baseline = (0, 0, 0, 0);
    state.push_effect(crate::app::EngineEffect::ScrollToBottom);

    let session_id = state
        .current_session_id
        .clone()
        .unwrap_or_else(jfc_session::generate_session_id);
    {
        let sid = session_id.clone();
        let msgs = state.messages.clone();
        let model = state.model.clone();
        tokio::spawn(async move {
            crate::session::save_session(&sid, &msgs, None, Some(model.as_str())).await;
        });
    }
    state.current_session_id = Some(session_id);

    let provider = state.provider.clone();
    let messages = crate::stream::build_provider_messages(&state.messages[..assistant_idx]);
    let model = state.model.clone();
    let interrupt = state.interrupt_flag.clone();
    interrupt.store(false, std::sync::atomic::Ordering::SeqCst);
    state.cancel_token = tokio_util::sync::CancellationToken::new();
    let cancel = state.cancel_token.clone();
    let overrides = crate::runtime::StreamRequestOverrides {
        background_reminders: state.take_background_reminders(),
        disallowed_tools: state.effective_disallowed_tools(),
        allowed_tools: state.allowed_tools.clone(),
        custom_betas: state.custom_betas.clone(),
        fine_grained_tool_streaming: state.fine_grained_tool_streaming,
        strict_tool_schemas: state.strict_tool_schemas,
        task_budget: state.cli_task_budget,
        max_thinking_tokens: state.cli_max_thinking_tokens,
        thinking_display: state.cli_thinking_display.clone(),
        brief_mode: state.brief_mode,
        last_usage_input_tokens: Some(state.last_usage_input as u64),
        context_window_tokens: Some(state.max_context_tokens as u64),
        ..Default::default()
    };
    crate::runtime::spawn_stream_response_scoped(
        state, tx, provider, messages, model, interrupt, cancel, None, overrides,
    );
}

#[cfg(test)]
mod plugin_refresh_tests {
    use super::*;

    struct EnvGuard {
        key: &'static str,
        prev: Option<String>,
    }

    impl EnvGuard {
        fn set(key: &'static str, value: &str) -> Self {
            let prev = std::env::var(key).ok();
            unsafe { std::env::set_var(key, value) };
            Self { key, prev }
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            unsafe {
                match &self.prev {
                    Some(value) => std::env::set_var(self.key, value),
                    None => std::env::remove_var(self.key),
                }
            }
        }
    }

    #[test]
    #[serial_test::serial]
    fn plugin_refresh_gate_accepts_background_refresh_alias_normal() {
        let _jfc = EnvGuard::set("JFC_PLUGIN_REFRESH_ON_MISS", "0");
        unsafe { std::env::remove_var("JFC_PLUGIN_REFRESH_ON_MISS") };
        let _alias = EnvGuard::set("CLAUDE_CODE_ENABLE_BACKGROUND_PLUGIN_REFRESH", "true");
        assert!(plugin_refresh_on_miss_enabled());
    }

    #[test]
    fn git_plugin_roots_discovers_project_git_plugin_normal() {
        let root = std::env::temp_dir().join(format!(
            "jfc_plugin_roots_test_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let plugin = root.join("plugins").join("demo");
        std::fs::create_dir_all(plugin.join(".git")).unwrap();

        let roots = git_plugin_roots(&root);

        assert!(roots.iter().any(|path| path == &plugin));
        let _ = std::fs::remove_dir_all(root);
    }
}
