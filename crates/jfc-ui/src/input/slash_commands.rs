//! Slash-command registry — the SINGLE source of truth for command
//! dispatch AND autocomplete/help metadata.
//!
//! Design note (grounded in the rust-lang Zulip wg-macros thread "Please
//! help me figure out these very old macro guidelines"): the per-command
//! LOGIC lives in plain `async fn` handlers (functions are preferred over
//! macros — kpreid). The `slash_commands!` macro exists ONLY for the one
//! job a macro is actually blessed for: generating "a static table AND a
//! function guaranteed to take action on all of the items in that list,
//! significantly reducing potential for human error." Adding a row here
//! wires up dispatch, autocomplete, and `/help` simultaneously, so the
//! table and the match can never drift. The macro is defined and used in
//! the same module, per the same thread's "favor narrow-purpose macros
//! defined next to their use site" guidance.

use super::*;

// Pull every `cmd_*` handler into scope so the macro-generated `dispatch`
// match can name them unqualified. Handlers are grouped into cohesion-named
// sibling modules (session/context/info/task/account/delegating).
use super::account_commands::*;
use super::context_commands::*;
use super::delegating_commands::*;
use super::info_commands::*;
use super::session_commands::*;
use super::task_commands::*;

/// Generate the `SLASH_COMMANDS` metadata table and the `dispatch` match
/// from a single declarative list. Each row is:
///
/// ```ignore
/// "/canonical" ["/alias", ...] "help text" => handler_fn,
/// ```
///
/// The macro emits (1) `SLASH_COMMANDS`: one `(name, help)` entry per
/// canonical name AND per alias, so autocomplete sees every invokable name;
/// (2) `dispatch`: a `match parts[0]` routing each name (canonical + aliases)
/// to its async handler, with a trailing skill-name fallthrough.
macro_rules! slash_commands {
    (
        $( $canon:literal $([ $($alias:literal),* $(,)? ])? $help:literal => $handler:ident ),* $(,)?
    ) => {
        /// Every invokable slash command with a one-line description, for
        /// the autocomplete popup and `/help`. Macro-generated — DO NOT
        /// hand-edit; add a row to `slash_commands!` instead.
        pub(crate) const SLASH_COMMANDS: &[(&str, &str)] = &[
            $(
                ($canon, $help),
                $( $( ($alias, $help), )* )?
            )*
        ];

        /// Route a parsed slash command to its handler. Generated from the
        /// same list as `SLASH_COMMANDS`, so a live dispatch arm exists for
        /// every table entry (verified by `slash_registry_has_no_drift`).
        pub(super) async fn dispatch(
            app: &mut App,
            parts: &[&str],
            text: &str,
            tx: Option<&mpsc::Sender<AppEvent>>,
        ) {
            match parts[0] {
                $(
                    $canon $( $(| $alias)* )? => $handler(app, parts, text, tx).await,
                )*
                _ => skill_fallthrough(app, parts, text, tx).await,
            }
        }
    };
}

pub async fn run_slash_command(app: &mut App, text: &str) {
    handle_slash_command(app, text, None).await
}

pub(super) async fn handle_slash_command(
    app: &mut App,
    text: &str,
    tx: Option<&mpsc::Sender<AppEvent>>,
) {
    let parts: Vec<&str> = text.splitn(2, ' ').collect();
    dispatch(app, &parts, text, tx).await;
    app.scroll_to_bottom();
}

/// Minimal application/x-www-form-urlencoded encoder for query strings.
/// Pulling in `urlencoding` or `url` for the two callers (`/bug` form
/// link generation) is overkill — the encoder only needs to handle ASCII
/// + UTF-8 bytes that browsers reliably decode.
#[allow(dead_code)]
pub(super) fn url_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char);
            }
            b' ' => out.push('+'),
            _ => out.push_str(&format!("%{:02X}", b)),
        }
    }
    out
}

/// Skill-name fallthrough for any `/<name>` not bound above: `/<skill>`
/// invokes the matching skill body as if the user had pasted it. Mirrors
/// v126 cli.js:226634 where a slash-name not otherwise bound resolves to a
/// skill or markdown command and either inline-expands or forks a subagent.
///
/// Phase B (not yet implemented): when `frontmatter.context == "fork"` (the
/// v126 flag at cli.js:178962), spawn a Task subagent here instead of
/// inline-expanding the body. For now every match inline-expands.
pub(super) async fn skill_fallthrough(
    app: &mut App,
    parts: &[&str],
    _text: &str,
    tx: Option<&mpsc::Sender<AppEvent>>,
) {
    let name = parts[0].trim_start_matches('/');
    let cwd = std::env::current_dir().unwrap_or_else(|_| ".".into());
    let skills = crate::agents::load_skills(&cwd);
    if let Some(skill) = crate::agents::find_skill_by_name(&skills, name) {
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
        app.messages.push(ChatMessage::user(echo));

        // Phase A: inline-expand the body. If the user passed args
        // after the skill name, append them under an `# Args` heading
        // so the skill prompt can reference them without us having to
        // template-substitute.
        let mut body = skill.body.clone();
        if let Some(rest) = parts.get(1) {
            let trimmed = rest.trim();
            if !trimmed.is_empty() {
                body.push_str("\n\n# Args\n");
                body.push_str(trimmed);
            }
        }

        let Some(tx) = tx else {
            // No tx in this dispatch path (e.g. queued-prompt drain).
            // Fall back to a hint rather than silently swallowing the
            // invocation.
            app.messages.push(ChatMessage::assistant(format!(
                "Skill `/{name}` cannot be invoked from this context (no stream channel). \
                         Submit `/{name}` directly from the input bar instead."
            )));
            app.scroll_to_bottom();
            return;
        };

        // Drive the same streaming setup as `handle_submit` for a
        // fresh user turn: push the synthetic user message, push the
        // empty assistant placeholder, prime streaming flags, persist
        // the session, then spawn the provider stream.
        let assistant_idx = app.messages.len() + 1;
        app.messages.push(ChatMessage::user(body));
        app.tool_ctx.total_user_turns += 1;
        app.messages.push(ChatMessage::assistant(String::new()));
        app.streaming_text.clear();
        app.streaming_reasoning.clear();
        app.streaming_response_bytes = 0;
        app.network_recovery_status = None;
        app.network_recovery_attempts = 0;
        app.streaming_assistant_idx = Some(assistant_idx);
        app.is_streaming = true;
        let now = std::time::Instant::now();
        app.streaming_started_at = Some(now);
        app.last_stream_event_at = Some(now);
        app.streaming_last_token_at = Some(now);
        app.turn_started_at = Some(now);
        app.turn_start_cost = crate::cost::total_cost(&app.usage_by_model);
        app.agentic_turn_count = 0;
        app.thinking_started_at = None;
        app.pre_dispatched_tool_ids.clear();
        app.deferred_tool_uses.clear();
        app.in_progress_tool_use_ids.clear();
        app.in_flight_eager_dispatches = 0;
        app.in_flight_tool_batches = 0;
        app.thinking_ended_at = None;
        app.last_usage_output = 0;
        app.usage_apply_baseline = (0, 0, 0, 0);
        app.scroll_to_bottom();

        let session_id = app
            .current_session_id
            .clone()
            .unwrap_or_else(jfc_session::generate_session_id);
        // Fire-and-forget — don't block UI on disk I/O
        {
            let sid = session_id.clone();
            let msgs = app.messages.clone();
            let model = app.model.clone();
            tokio::spawn(async move {
                crate::session::save_session(&sid, &msgs, None, Some(model.as_str())).await;
            });
        }
        app.current_session_id = Some(session_id);

        let provider = app.provider.clone();
        let messages = crate::stream::build_provider_messages(&app.messages[..assistant_idx]);
        let model = app.model.clone();
        let tx_stream = tx.clone();
        let interrupt = app.interrupt_flag.clone();
        interrupt.store(false, std::sync::atomic::Ordering::SeqCst);
        app.cancel_token = tokio_util::sync::CancellationToken::new();
        let cancel = app.cancel_token.clone();
        let overrides = crate::runtime::StreamRequestOverrides {
            background_reminders: app.take_background_reminders(),
            disallowed_tools: app.effective_disallowed_tools(),
            allowed_tools: app.allowed_tools.clone(),
            custom_betas: app.custom_betas.clone(),
            fine_grained_tool_streaming: app.fine_grained_tool_streaming,
            strict_tool_schemas: app.strict_tool_schemas,
            task_budget: app.cli_task_budget,
            max_thinking_tokens: app.cli_max_thinking_tokens,
            thinking_display: app.cli_thinking_display.clone(),
            brief_mode: app.brief_mode,
            ..Default::default()
        };
        // wg-async: retry path mints a fresh cancel token for the
        // new stream so the old (possibly cancelled) one can't
        // racially interrupt the retry.
        tokio::spawn(async move {
            crate::stream::stream_response(
                provider, messages, model, tx_stream, interrupt, cancel, None, overrides,
            )
            .await;
        });
        return;
    }

    app.messages.push(ChatMessage::assistant(format!(
        "Unknown command: `{}`. Type `/help` for available commands.",
        parts[0]
    )));
}

slash_commands! {
        "/rename" [] "set a custom title on the current session" => cmd_rename,
        "/clear" [] "clear the conversation and start fresh" => cmd_clear,
        "/check" [] "re-run cargo-check diagnostics" => cmd_check,
        "/compact" [] "summarize earlier messages to free context" => cmd_compact,
        "/advisor" [] "ask a parallel advisor without disturbing the main agent" => cmd_advisor,
        "/brief" [] "toggle brief-only mode (hide plain text, only show SendUserMessage output)" => cmd_brief,
        "/autoloop" [] "start an autonomous loop tick (reads loop.md)" => cmd_autoloop,
        "/sandbox" [] "toggle bash sandbox (bwrap network isolation)" => cmd_sandbox,
        "/team-onboarding" [] "generate a team onboarding guide from project state" => cmd_team_onboarding,
        "/coach" [] "show coaching tips based on this session's tool usage" => cmd_coach,
        "/remote" [] "spawn a remote CCR session (requires ANTHROPIC_API_KEY)" => cmd_remote,
        "/remote-control" ["/rc"] "toggle remote-control server (WS on port 4242)" => cmd_remote_control,
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
        "/copy" [] "copy transcript text to the clipboard (last/all/N)" => cmd_copy,
        "/fork" [] "snapshot the first N messages as a new session" => cmd_fork,
        "/batch" [] "submit a prompt-file via the Message Batches API" => cmd_batch,
        "/diff" [] "show pending uncommitted changes" => cmd_diff,
        "/undo" [] "revert the most recent file mutation by a tool" => cmd_undo,
        "/export" [] "save the transcript as markdown" => cmd_export,
        "/verbose" [] "toggle expanded-by-default tool blocks" => cmd_verbose,
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
        "/help" [] "show jfc help" => cmd_help,
        "/memory" ["/mem"] "list memory files / toggle two-phase recall" => cmd_memory,
        "/commit" [] "generate a conventional commit message for staged changes" => cmd_commit,
        "/review" [] "ask the model to review current git changes" => cmd_review,
        "/skills" [] "list available skills (.claude/skills/*.md)" => cmd_skills,
        "/agents" [] "list available agent definitions (.claude/agents/*.md)" => cmd_agents,
        "/market" [] "show the agent-economy snapshot" => cmd_market,
        "/cascade" [] "list cascade tasks queued by symbol_edit" => cmd_cascade,
        "/graph-history" [] "list recent code-graph queries" => cmd_graph_history,
        "/task-list" ["/tasks"] "list todo/task items" => cmd_task_list,
        "/task-add" [] "create a new task" => cmd_task_add,
        "/task-done" [] "mark a task completed" => cmd_task_done,
        "/task-rm" ["/task-delete"] "delete a task" => cmd_task_rm,
        "/claude-md" [] "show which CLAUDE.md layers are loaded" => cmd_claude_md,
        "/mode" [] "switch permission mode (default/plan/accept/auto/bypass)" => cmd_mode,
        "/auto-mode" [] "toggle the autonomous tool classifier" => cmd_auto_mode,
        "/worktree" [] "create / list / remove a git worktree" => cmd_worktree,
        "/mcp" [] "list / inspect configured MCP servers" => cmd_mcp,
        "/theme" [] "open picker or switch theme" => cmd_theme,
        "/fleet" ["/fleetview"] "show the teammate fleet view" => cmd_fleet,
        "/teleport" [] "jump into a teammate's context" => cmd_teleport,
        "/init" [] "scaffold a CLAUDE.md for this project" => cmd_init,
        "/plan" [] "draft or update PLAN.md (Atlas-compatible)" => cmd_plan,
        "/roadmap" [] "draft or update ROADMAP.md (stable decimal IDs)" => cmd_roadmap,
        "/parity" [] "draft or update PARITY.md (evidence required)" => cmd_parity,
        "/philosophy" [] "draft or update PHILOSOPHY.md" => cmd_philosophy,
        "/usage" [] "draft or update USAGE.md (operator commands)" => cmd_usage,
        "/cost" ["/stats"] "show session cost / token usage" => cmd_cost,
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
}
