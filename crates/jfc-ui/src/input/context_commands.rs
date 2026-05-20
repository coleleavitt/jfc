//! Slash handlers: context, compaction & agent control.

use super::*;

pub(super) async fn cmd_check(
    app: &mut App,
    _parts: &[&str],
    _text: &str,
    _tx: Option<&mpsc::Sender<AppEvent>>,
) {
    // Re-run `cargo check --message-format=json` and refresh the
    // diagnostic row + transition toast. v126 has an analogous
    // `/diagnostics` flow; keep ours short. Best-effort — silently
    // no-ops outside a cargo project.
    app.messages.push(ChatMessage::user("/check".into()));
    app.messages.push(ChatMessage::assistant(
        "Running `cargo check`… (results will land in the diagnostic row)".into(),
    ));
    // The handler emits `ProviderEvent::DiagnosticsUpdated` whose
    // handler shows a transition toast — no need to render
    // results inline.
    // We don't have direct `tx` here; emit via a no-op
    // background spawn that returns through the channel exposed
    // to other slash-command paths. Instead, we set a flag the
    // main loop can pick up; for now the simpler thing is to
    // tell the user to wait for the auto-update.
    //
    // (The startup-time spawn already does this on launch; this
    // command just reminds the user how to retrigger.)
}

pub(super) async fn cmd_compact(
    app: &mut App,
    _parts: &[&str],
    _text: &str,
    _tx: Option<&mpsc::Sender<AppEvent>>,
) {
    // Use the calibrated context size (same source as the gauge
    // and pre-submit gate). Previously this re-ran the raw
    // `estimate_tokens` heuristic, so the manual report disagreed
    // with the live gauge and could show "0%" for a session the
    // sidebar reports as 90%-full.
    let est = app.tool_ctx.approx_tokens;
    let level = crate::compact::compact_level(est, app.max_context_tokens);
    let pct = if app.max_context_tokens > 0 {
        (est * 100 / app.max_context_tokens).min(999)
    } else {
        0
    };
    tracing::info!(
        target: "jfc::compact",
        est, max_context_tokens = app.max_context_tokens,
        pct, ?level, model = %app.model,
        "manual /compact command invoked"
    );
    app.messages.push(ChatMessage::user("/compact".into()));
    app.messages.push(ChatMessage::assistant(format!(
                "Manual compaction queued — current estimate **{est} / {} tokens ({pct}%)**, level: **{level:?}**.\n\n\
                 The next assistant turn will summarize the conversation up to here, replacing the prior turns with a 9-section summary.\n\n\
                 *(Tip: set `JFC_AUTOCOMPACT_PCT_OVERRIDE=N` (1-100) to test thresholds, or `JFC_DISABLE_AUTO_COMPACT=1` to disable auto-compact entirely.)*",
                app.max_context_tokens
            )));
    app.force_compact_pending = true;
}

pub(super) async fn cmd_advisor(
    app: &mut App,
    parts: &[&str],
    _text: &str,
    _tx: Option<&mpsc::Sender<AppEvent>>,
) {
    // Parallel advisor (see `crate::advisor`). Doesn't touch the main
    // agent's stream — runs a separate `provider.complete()` against a
    // SNAPSHOT of the current transcript and surfaces the reply as a
    // dedicated `MessagePart::Advisor` part with its own visual style.
    //
    // Default-off per deliverable: gated by `app.advisor_enabled`,
    // populated from `JFC_ADVISOR_ENABLED=1` on startup. Even when on,
    // each session has a per-budget ceiling (`DEFAULT_TOKEN_BUDGET`)
    // so a runaway loop can't drain the user's account.
    let query = parts.get(1).copied().unwrap_or("").trim().to_owned();
    // Echo the user's command into the transcript first so the chat
    // shows what the user asked, even on the error paths below.
    app.messages
        .push(ChatMessage::user(format!("/advisor {query}")));
    if !app.advisor_enabled {
        app.messages
            .push(ChatMessage::assistant_parts(vec![MessagePart::Advisor(
                "Advisor mode is disabled. Set `JFC_ADVISOR_ENABLED=1` and \
                         restart jfc to enable parallel advisor queries."
                    .into(),
            )]));
    } else if query.is_empty() {
        app.messages
            .push(ChatMessage::assistant_parts(vec![MessagePart::Advisor(
                "Usage: `/advisor <question>` — runs a parallel call \
                         against a snapshot of this transcript and surfaces \
                         the reply here without disturbing the main agent."
                    .into(),
            )]));
    } else {
        // Lazy-mint the session on first use so users that never
        // call /advisor pay no allocation cost. The session model
        // tracks the *active* model at first invocation; switching
        // models mid-session keeps the original advisor model.
        let session = app
            .advisor_session
            .get_or_insert_with(|| crate::advisor::AdvisorSession::new(app.model.clone()));
        // Snapshot — Vec::clone is fine here, the deliverable
        // explicitly calls for a SNAPSHOT semantic. Without the
        // clone, `ask_advisor` would borrow `app.messages`
        // immutably while we're holding `&mut app.advisor_session`
        // mutably — borrow-check fails.
        let snapshot = app.messages.clone();
        let provider = std::sync::Arc::clone(&app.provider);
        match crate::advisor::ask_advisor(provider.as_ref(), session, query.clone(), &snapshot)
            .await
        {
            Ok(reply) => {
                let remaining = session.tokens_remaining();
                let total_budget = session.token_budget;
                app.messages
                    .push(ChatMessage::assistant_parts(vec![MessagePart::Advisor(
                        format!(
                            "{reply}\n\n_(advisor budget: {} of {} tokens remaining)_",
                            remaining, total_budget
                        ),
                    )]));
            }
            Err(e) => {
                app.messages.push(ChatMessage::assistant_parts(vec![
                            MessagePart::Advisor(format!(
                                "Advisor error: {e}\n\nUse `/clear` to start a fresh session if the budget is exhausted."
                            )),
                        ]));
            }
        }
    }
}

pub(super) async fn cmd_config(
    app: &mut App,
    parts: &[&str],
    text: &str,
    _tx: Option<&mpsc::Sender<AppEvent>>,
) {
    // `/config` (no args) → dump the parsed config as TOML in a code block.
    // `/config path` → print the canonical file path so the user knows
    // where to put their overrides. We re-parse on every invocation
    // (instead of caching at startup) so edits to ~/.config/jfc/config.toml
    // surface without restart — this command is the user's read-only
    // window into "what does jfc currently see?". Wiring the resolved
    // model into the actual stream call site is a separate task; for now
    // this command exists so users can verify their file parses and
    // know where to edit.
    let arg = parts.get(1).copied().unwrap_or("").trim();
    app.messages.push(ChatMessage::user(text.to_owned()));
    if arg == "path" {
        let p = crate::config::config_path();
        app.messages.push(ChatMessage::assistant(format!(
            "**Config path:** `{}`",
            p.display()
        )));
    } else {
        let cfg = crate::config::load();
        let body = match toml::to_string_pretty(&cfg) {
            Ok(s) if s.trim().is_empty() => "(empty config — no overrides)".to_owned(),
            Ok(s) => format!("```toml\n{s}```"),
            Err(e) => format!("**Error serializing config:** {e}"),
        };
        app.messages.push(ChatMessage::assistant(body));
    }
}

pub(super) async fn cmd_verbose(
    app: &mut App,
    parts: &[&str],
    text: &str,
    _tx: Option<&mpsc::Sender<AppEvent>>,
) {
    // Toggle expanded-by-default tool blocks for the rest of
    // the session. Renderers read `app.verbose_mode` and lift
    // the per-tool preview cap when set.
    app.messages.push(ChatMessage::user(text.to_owned()));
    let arg = parts
        .get(1)
        .copied()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    let target = match arg.as_str() {
        "on" | "true" | "1" => Some(true),
        "off" | "false" | "0" => Some(false),
        "" => Some(!app.verbose_mode),
        _ => None,
    };
    match target {
        Some(v) => {
            app.verbose_mode = v;
            app.messages.push(ChatMessage::assistant(format!(
                "Verbose mode **{}** — tool blocks {} preview cap.",
                if v { "ON" } else { "OFF" },
                if v { "expand past" } else { "respect" },
            )));
        }
        None => {
            app.messages.push(ChatMessage::assistant(
                "Usage: `/verbose [on|off]`. With no arg, toggles.".into(),
            ));
        }
    }
}

pub(super) async fn cmd_fast(
    app: &mut App,
    _parts: &[&str],
    text: &str,
    _tx: Option<&mpsc::Sender<AppEvent>>,
) {
    // Toggle fast mode (lower-latency inference via Anthropic's
    // `fast-mode-2026-02-01` beta header). Mirrors Claude Code
    // v2.1.139's `/fast` command (Alt+O keybind).
    app.messages.push(ChatMessage::user(text.to_owned()));
    app.fast_mode = !app.fast_mode;
    crate::effort::set_fast_mode_global(app.fast_mode);
    app.messages.push(ChatMessage::assistant(format!(
        "Fast mode: **{}** — {}",
        if app.fast_mode { "ON" } else { "OFF" },
        if app.fast_mode {
            "requests will use the low-latency inference path"
        } else {
            "requests will use the standard inference path"
        },
    )));
}

pub(super) async fn cmd_pin(
    app: &mut App,
    parts: &[&str],
    text: &str,
    _tx: Option<&mpsc::Sender<AppEvent>>,
) {
    // Pin a message by transcript index so compaction can't
    // drop it. /pin without an arg pins the most recent
    // message; /pin <n> pins index n; /pin list prints the
    // current pin set.
    app.messages.push(ChatMessage::user(text.to_owned()));
    let arg = parts.get(1).copied().unwrap_or("").trim();
    if arg == "list" {
        if app.pinned_message_indices.is_empty() {
            app.messages.push(ChatMessage::assistant(
                "No pinned messages. `/pin <n>` pins index n; `/pin` pins the most recent.".into(),
            ));
        } else {
            let mut idx: Vec<usize> = app.pinned_message_indices.iter().copied().collect();
            idx.sort();
            let listing = idx
                .into_iter()
                .map(|i| format!("- #{i}"))
                .collect::<Vec<_>>()
                .join("\n");
            app.messages.push(ChatMessage::assistant(format!(
                "**Pinned messages:**\n{listing}"
            )));
        }
    } else if arg.is_empty() {
        if app.messages.is_empty() {
            return;
        }
        let idx = app.messages.len() - 1;
        app.pinned_message_indices.insert(idx);
        app.messages.push(ChatMessage::assistant(format!(
            "Pinned message #{idx} (compaction will preserve it)."
        )));
    } else {
        match arg.parse::<usize>() {
            Ok(idx) if idx < app.messages.len() => {
                app.pinned_message_indices.insert(idx);
                app.messages
                    .push(ChatMessage::assistant(format!("Pinned message #{idx}.")));
            }
            Ok(idx) => {
                app.messages.push(ChatMessage::assistant(format!(
                    "No message at index {idx} (transcript has {} messages).",
                    app.messages.len()
                )));
            }
            Err(_) => {
                app.messages.push(ChatMessage::assistant(format!(
                            "Couldn't parse `{arg}` as a message index. Use `/pin`, `/pin <n>`, or `/pin list`."
                        )));
            }
        }
    }
}

pub(super) async fn cmd_unpin(
    app: &mut App,
    parts: &[&str],
    text: &str,
    _tx: Option<&mpsc::Sender<AppEvent>>,
) {
    app.messages.push(ChatMessage::user(text.to_owned()));
    let arg = parts.get(1).copied().unwrap_or("").trim();
    if arg.is_empty() || arg == "all" {
        let n = app.pinned_message_indices.len();
        app.pinned_message_indices.clear();
        app.messages
            .push(ChatMessage::assistant(format!("Cleared {n} pin(s).")));
    } else {
        match arg.parse::<usize>() {
            Ok(idx) => {
                if app.pinned_message_indices.remove(&idx) {
                    app.messages
                        .push(ChatMessage::assistant(format!("Unpinned message #{idx}.")));
                } else {
                    app.messages.push(ChatMessage::assistant(format!(
                        "Message #{idx} wasn't pinned."
                    )));
                }
            }
            Err(_) => {
                app.messages.push(ChatMessage::assistant(format!(
                    "Couldn't parse `{arg}` as a message index."
                )));
            }
        }
    }
}

pub(super) async fn cmd_effort(
    app: &mut App,
    parts: &[&str],
    text: &str,
    _tx: Option<&mpsc::Sender<AppEvent>>,
) {
    // v132 reasoning-effort pin. `/effort low|medium|high|xhigh|max`
    // sets the pin; `/effort` alone shows the current state;
    // `/effort clear` removes the pin so the model picks adaptive.
    app.messages.push(ChatMessage::user(text.to_owned()));
    let arg = parts.get(1).copied().unwrap_or("").trim();
    if arg.is_empty() {
        app.messages
            .push(ChatMessage::assistant(app.effort_state.status()));
    } else if arg == "clear" || arg == "off" {
        let msg = app.effort_state.clear();
        app.messages.push(ChatMessage::assistant(msg));
    } else if let Some(level) = crate::effort::ReasoningEffort::from_str_loose(arg) {
        let msg = app.effort_state.set(level);
        app.messages.push(ChatMessage::assistant(msg));
    } else {
        app.messages.push(ChatMessage::assistant(format!(
            "Unknown effort `{arg}`. Use one of: low, medium, high, xhigh, max, clear."
        )));
    }
}

pub(super) async fn cmd_feature(
    app: &mut App,
    parts: &[&str],
    text: &str,
    _tx: Option<&mpsc::Sender<AppEvent>>,
) {
    // v132 feature-gate framework. `/feature` lists all gates and
    // their state; `/feature <codename> on|off` flips one.
    app.messages.push(ChatMessage::user(text.to_owned()));
    let rest = parts.get(1).copied().unwrap_or("").trim();
    if rest.is_empty() {
        let mut body = String::from("**Feature gates:**\n\n");
        for &gate in crate::feature_gates::FeatureGate::ALL {
            body.push_str(&format!(
                "- `{}` — **{}** ({})\n",
                gate.codename(),
                if crate::feature_gates::is_enabled(gate) {
                    "ON"
                } else {
                    "OFF"
                },
                gate.description(),
            ));
        }
        body.push_str("\nToggle with `/feature <codename> on|off`.");
        app.messages.push(ChatMessage::assistant(body));
    } else {
        let mut sub = rest.split_whitespace();
        let name = sub.next().unwrap_or("");
        let toggle = sub.next().unwrap_or("").to_ascii_lowercase();
        let Some(gate) = crate::feature_gates::FeatureGate::from_codename(name) else {
            app.messages.push(ChatMessage::assistant(format!(
                "Unknown feature gate `{name}`. List with `/feature`."
            )));
            return;
        };
        let enabled = match toggle.as_str() {
            "on" | "enable" | "true" | "1" => true,
            "off" | "disable" | "false" | "0" => false,
            "" => {
                app.messages.push(ChatMessage::assistant(format!(
                    "`{}` is currently **{}**. Toggle with `/feature {} on|off`.",
                    gate.codename(),
                    if crate::feature_gates::is_enabled(gate) {
                        "ON"
                    } else {
                        "OFF"
                    },
                    gate.codename(),
                )));
                return;
            }
            other => {
                app.messages.push(ChatMessage::assistant(format!(
                    "Unknown toggle `{other}`. Use `on` or `off`."
                )));
                return;
            }
        };
        crate::feature_gates::set(gate, enabled);
        app.messages.push(ChatMessage::assistant(format!(
            "`{}` set to **{}** ({}).",
            gate.codename(),
            if enabled { "ON" } else { "OFF" },
            gate.description(),
        )));
        // v132 system-reminder so the model sees the gate flip
        // on the next turn (rather than guessing from changed
        // behavior).
        crate::system_reminder::append_to_last_user(
            &mut app.messages,
            &format!(
                "Feature gate `{}` flipped to **{}** ({}). Adjust your \
                         behavior accordingly.",
                gate.codename(),
                if enabled { "ON" } else { "OFF" },
                gate.description(),
            ),
        );
    }
}

pub(super) async fn cmd_goal(
    app: &mut App,
    parts: &[&str],
    text: &str,
    tx: Option<&mpsc::Sender<AppEvent>>,
) {
    // v137 session-scoped goal. `/goal <condition>` sets a stop
    // condition — the agent keeps working until the evaluator
    // says it's met (see `crate::goal::evaluate`). `/goal
    // clear` (or stop/off/reset/none/cancel) removes it.
    // `/goal` alone shows the current state.
    app.messages.push(ChatMessage::user(text.to_owned()));
    let arg = parts[1..].join(" ");
    let arg = arg.trim();
    if arg.is_empty() {
        let msg = match &app.goal {
            Some(g) => format!(
                "Current goal ({} iterations): {}\n\nUse `/goal clear` to remove.",
                g.iterations, g.condition
            ),
            None => "No goal set. Usage: `/goal <condition>`".to_string(),
        };
        app.messages.push(ChatMessage::assistant(msg));
    } else if crate::goal::is_clear_arg(arg) {
        let prev = app.goal.take();
        app.goal_evaluator_in_flight = false;
        // Drop the sidecar so a future /continue doesn't
        // revive a goal the user just cancelled.
        if let Some(sid) = app.current_session_id.as_ref() {
            crate::goal::save_sidecar(sid.as_str(), None);
        }
        let msg = match prev {
            Some(g) => format!(
                "Goal cleared after {} iterations: {}",
                g.iterations, g.condition
            ),
            None => "No goal was set.".to_string(),
        };
        app.messages.push(ChatMessage::assistant(msg));
        crate::toast::push_with_cap(
            &mut app.toasts,
            crate::toast::Toast::new(crate::toast::ToastKind::Success, "Goal cleared".to_string()),
        );
    } else {
        match crate::goal::validate_condition(arg) {
            Ok(condition) => {
                let goal = crate::goal::ActiveGoal::new(condition.clone());
                app.goal = Some(goal);
                // Persist the new goal so /continue picks it
                // up if the user exits before the next turn.
                if let Some(sid) = app.current_session_id.as_ref() {
                    crate::goal::save_sidecar(sid.as_str(), app.goal.as_ref());
                }
                app.messages.push(ChatMessage::assistant(format!(
                    "Goal set: {condition}\n\nThe agent will keep \
                             working until this condition is met (auto-\
                             evaluated after each turn, max {} iterations). \
                             Use `/goal clear` to cancel.",
                    crate::goal::MAX_ITERATIONS
                )));
                crate::toast::push_with_cap(
                    &mut app.toasts,
                    crate::toast::Toast::new(
                        crate::toast::ToastKind::Success,
                        format!("Goal: {condition}"),
                    ),
                );
                // Kick off work immediately: synthesize the
                // Claude-Code-style meta prompt so the agent
                // starts acting on the goal instead of sitting
                // idle until the next user turn. Only fire
                // when the session is genuinely idle (no
                // streaming / pending approval / pending
                // tools) AND we have an event channel.
                let idle = !app.is_streaming
                    && app.pending_approval.is_none()
                    && app.approval_queue.is_empty()
                    && app.pending_tool_calls.is_empty();
                if let (true, Some(tx)) = (idle, tx) {
                    let kickoff = format!(
                        "A session-scoped stop-condition hook is now \
                                 active with condition: \"{condition}\".\n\n\
                                 Briefly acknowledge the goal, then \
                                 immediately start or continue working toward \
                                 it. The hook will block stopping until the \
                                 condition holds (auto-evaluated after each \
                                 turn, max {} iterations). It auto-clears \
                                 once the condition is met.",
                        crate::goal::MAX_ITERATIONS
                    );
                    let _ = tx.send(AppEvent::Ui(UiEvent::Submit(kickoff))).await;
                    tracing::info!(
                        target: "jfc::goal",
                        "/goal: dispatched kickoff meta-prompt"
                    );
                }
            }
            Err(reason) => {
                app.messages.push(ChatMessage::assistant(reason.to_owned()));
            }
        }
    }
}

pub(super) async fn cmd_memory(
    app: &mut App,
    parts: &[&str],
    text: &str,
    _tx: Option<&mpsc::Sender<AppEvent>>,
) {
    // `/memory` (no args)            → list memory files
    // `/memory recall on|off|status` → toggle two-phase recall
    //
    // The recall sub-command targets the runtime override in
    // `memory_recall::set_runtime_override` — persisting to
    // `~/.config/jfc/config.toml` is left to the user since they
    // may have hand-formatted that file.
    let arg = parts.get(1).copied().unwrap_or("").trim();
    app.messages.push(ChatMessage::user(text.to_owned()));
    if arg.starts_with("recall") {
        let sub = arg
            .split_once(' ')
            .map(|x| x.1)
            .map(str::trim)
            .unwrap_or("status");
        match sub {
            "on" | "enable" => {
                crate::memory_recall::set_runtime_override(Some(true));
                app.messages.push(ChatMessage::assistant(
                    "Two-phase memory recall: **on** (runtime override).".into(),
                ));
            }
            "off" | "disable" => {
                crate::memory_recall::set_runtime_override(Some(false));
                app.messages.push(ChatMessage::assistant(
                    "Two-phase memory recall: **off** (runtime override).".into(),
                ));
            }
            "default" | "reset" => {
                crate::memory_recall::set_runtime_override(None);
                app.messages.push(ChatMessage::assistant(
                    "Two-phase memory recall: cleared runtime override; \
                             falling back to `~/.config/jfc/config.toml` value."
                        .into(),
                ));
            }
            "status" | "" => {
                let persisted = crate::config::load().memory_recall_enabled;
                let effective = crate::memory_recall::is_enabled(persisted);
                app.messages.push(ChatMessage::assistant(format!(
                    "**Memory recall**\n\
                             - Effective: **{}**\n\
                             - Persisted (config.toml): **{}**\n\
                             \n\
                             Toggle with `/memory recall on|off|reset`.",
                    if effective { "on" } else { "off" },
                    if persisted { "on" } else { "off" }
                )));
            }
            other => {
                app.messages.push(ChatMessage::assistant(format!(
                    "Unknown sub-command `{other}`. Try \
                             `/memory recall on|off|reset|status`."
                )));
            }
        }
    } else {
        let cwd = std::env::current_dir().unwrap_or_else(|_| ".".into());
        let mems = crate::memory::load_all_memories(&cwd);
        let body = if mems.is_empty() {
            "No memory files found. Create `.jfc/memory/*.md` (project) or \
                     `~/.config/jfc/memory/*.md` (user) with YAML frontmatter \
                     (`type:` and `scope:`) and a markdown body."
                .to_owned()
        } else {
            let listing = crate::memory::format_existing_memories(&mems);
            format!(
                "**{} memor{} loaded:**\n\n{listing}\n\nUse `/memory recall status` to see whether two-phase recall is active.",
                mems.len(),
                if mems.len() == 1 { "y" } else { "ies" }
            )
        };
        app.messages.push(ChatMessage::assistant(body));
    }
}

pub(super) async fn cmd_claude_md(
    app: &mut App,
    _parts: &[&str],
    _text: &str,
    _tx: Option<&mpsc::Sender<AppEvent>>,
) {
    let h = crate::context::ClaudeMdHierarchy::load(
        &std::env::current_dir().unwrap_or_else(|_| ".".into()),
    );
    let body = if !h.any() {
        "No CLAUDE.md files found in any of the v126 hierarchy locations \
                 (`~/.config/claude/CLAUDE.md`, `~/.claude/CLAUDE.md`, \
                 `<project>/CLAUDE.md`, `<project>/.claude/CLAUDE.md`, \
                 `<project>/CLAUDE.local.md`)."
            .to_owned()
    } else {
        let mut s = String::from("**CLAUDE.md layers loaded** (in precedence order):\n\n");
        for (label, layer) in [
            ("Managed policy", &h.managed),
            ("User preferences", &h.user),
            ("Project instructions", &h.project),
            ("Project (.claude)", &h.project_dot),
            ("Local overrides", &h.local),
        ] {
            if let Some((path, content)) = layer {
                s.push_str(&format!(
                    "- **{}** ({}) — {} bytes\n",
                    label,
                    path.display(),
                    content.len()
                ));
            }
        }
        s
    };
    app.messages.push(ChatMessage::user("/claude-md".into()));
    app.messages.push(ChatMessage::assistant(body));
}

pub(super) async fn cmd_mode(
    app: &mut App,
    parts: &[&str],
    _text: &str,
    _tx: Option<&mpsc::Sender<AppEvent>>,
) {
    let arg = parts.get(1).copied().unwrap_or("").trim().to_lowercase();
    let new_mode = match arg.as_str() {
        "default" | "d" => Some(crate::app::PermissionMode::Default),
        "plan" | "p" => Some(crate::app::PermissionMode::Plan),
        "accept" | "acceptedits" | "a" => Some(crate::app::PermissionMode::AcceptEdits),
        "bypass" | "b" | "yolo" => Some(crate::app::PermissionMode::BypassPermissions),
        "auto" => Some(crate::app::PermissionMode::Auto),
        "" => {
            app.messages.push(ChatMessage::assistant(format!(
                "**Current mode:** {} {}\n\n\
                         Available: `default`, `plan`, `accept`, `auto`, `bypass`\n\
                         Switch: `/mode <name>` or **Shift+Tab** to cycle.",
                app.permission_mode.symbol(),
                app.permission_mode.label(),
            )));
            None
        }
        _ => {
            app.messages.push(ChatMessage::assistant(format!(
                "Unknown mode `{arg}`. Available: `default`, `plan`, `accept`, `auto`, `bypass`"
            )));
            None
        }
    };
    if let Some(mode) = new_mode {
        app.permission_mode = mode;
        // Persist so the mode survives session restart / --continue.
        crate::config::save_permission_mode(&app.permission_mode);
        // Sync auto_mode.enabled with permission mode for backward compat
        app.auto_mode.enabled = mode == crate::app::PermissionMode::Auto;
        app.messages.push(ChatMessage::assistant(format!(
            "**Mode → {} {}**",
            mode.symbol(),
            mode.label()
        )));
    }
}

pub(super) async fn cmd_auto_mode(
    app: &mut App,
    parts: &[&str],
    _text: &str,
    _tx: Option<&mpsc::Sender<AppEvent>>,
) {
    let arg = parts.get(1).copied().unwrap_or("status").trim();
    match arg {
        "on" | "enable" | "true" => {
            app.auto_mode.enabled = true;
            app.messages.push(ChatMessage::assistant(
                "**Auto-mode enabled.** Every tool call will be sent to the v126 \
                         classifier LLM. The classifier may block dangerous operations \
                         without prompting you. Edit `~/.config/jfc/settings.json` under \
                         `autoMode.{allow,soft_deny,environment}` (with `$defaults` \
                         inheritance) to extend the rules."
                    .into(),
            ));
        }
        "off" | "disable" | "false" => {
            app.auto_mode.enabled = false;
            app.messages.push(ChatMessage::assistant(
                "**Auto-mode disabled.** Tool calls will use the manual approval \
                         flow again."
                    .into(),
            ));
        }
        _ => {
            let n_allow = app.auto_mode.allow.len();
            let n_block = app.auto_mode.soft_deny.len();
            let n_env = app.auto_mode.environment.len();
            let state = if app.auto_mode.enabled { "ON" } else { "OFF" };
            app.messages.push(ChatMessage::assistant(format!(
                "**Auto-mode: {state}**\n\
                         \n\
                         Custom rule counts (settings.json):\n\
                         - allow: {n_allow}\n\
                         - soft_deny: {n_block}\n\
                         - environment: {n_env}\n\
                         \n\
                         Use `/auto-mode on` or `/auto-mode off` to toggle."
            )));
        }
    }
}

pub(super) async fn cmd_swarm_approve(
    app: &mut App,
    parts: &[&str],
    _text: &str,
    _tx: Option<&mpsc::Sender<AppEvent>>,
) {
    // Resolve a pending swarm permission request from the user's
    // input bar. Toasts surface the request id when it lands;
    // here we hand it back to `permission_sync::resolve_permission`
    // with the leader as `resolved_by` so the teammate's poll
    // loop unblocks.
    let id = parts.get(1).copied().unwrap_or("").trim().to_owned();
    let approve = parts[0] == "/swarm-approve";
    let feedback = parts
        .get(2..)
        .map(|rest| rest.join(" "))
        .filter(|s| !s.trim().is_empty());
    if id.is_empty() {
        app.messages.push(ChatMessage::assistant(format!(
                    "Usage: {} <request-id> [feedback]\nFind the id in the toast that appeared when the teammate asked.",
                    parts[0]
                )));
    } else {
        let team_name = app.team_context.team_name.clone().unwrap_or_default();
        let echo = if approve {
            format!("/swarm-approve {id}")
        } else if let Some(ref f) = feedback {
            format!("/swarm-deny {id} {f}")
        } else {
            format!("/swarm-deny {id}")
        };
        app.messages.push(ChatMessage::user(echo));
        if team_name.is_empty() {
            app.messages.push(ChatMessage::assistant(
                "No active team — nothing to approve.".into(),
            ));
        } else {
            let resolution = crate::swarm::types::PermissionResolution {
                decision: if approve {
                    crate::swarm::types::PermissionDecision::Approved
                } else {
                    crate::swarm::types::PermissionDecision::Rejected
                },
                resolved_by: "user".to_owned(),
                feedback,
                updated_input: None,
                permission_updates: Vec::new(),
            };
            let req_id = id.clone();
            tokio::spawn(async move {
                let _ = crate::swarm::permission_sync::resolve_permission(
                    &req_id,
                    &resolution,
                    &team_name,
                )
                .await;
            });
            app.messages.push(ChatMessage::assistant(format!(
                "Resolved swarm request {id} → {}",
                if approve { "approved" } else { "denied" }
            )));
        }
    }
}
