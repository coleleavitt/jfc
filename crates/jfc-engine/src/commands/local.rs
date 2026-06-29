use crate::app::EngineState;
use tokio::sync::mpsc;

use crate::runtime::{ControlEvent, EngineEvent};
use jfc_core::ChatMessage;

/// `/dump-context` prints everything jfc would inject into the system prompt
/// into the transcript.
pub(super) async fn handle_dump_context_command(state: &mut EngineState) {
    let mut report = String::new();
    let cwd = std::path::PathBuf::from(&state.cwd);

    report.push_str("**Model context dump**\n\n");
    report.push_str(&format!("- Model: `{}`\n", state.model));
    report.push_str(&format!("- Cwd: `{}`\n", state.cwd));
    report.push_str(&format!("- Provider: `{}`\n", state.provider.name()));
    report.push_str(&format!(
        "- Permission mode: `{:?}`\n",
        state.permission_mode
    ));
    if let Some(ref branch) = state.git_branch {
        report.push_str(&format!("- Git branch: `{branch}`\n"));
    }
    report.push('\n');

    let hierarchy = crate::context::ClaudeMdHierarchy::load(&cwd);
    if let Some(rendered) = hierarchy.render() {
        report.push_str("### CLAUDE.md hierarchy\n\n```\n");
        report.push_str(&rendered);
        report.push_str("\n```\n\n");
    } else {
        report.push_str(
            "### CLAUDE.md hierarchy\n\n_(none — no managed/user/project files found)_\n\n",
        );
    }

    let skills = crate::agents::load_skills(&cwd);
    report.push_str(&format!("### Skills ({})\n\n", skills.len()));
    for skill in &skills {
        report.push_str(&format!("- `{}`\n", skill.name));
    }
    if skills.is_empty() {
        report.push_str("_(none)_\n");
    }
    report.push('\n');

    let memories = crate::memory::load_all_memories(&cwd).await;
    report.push_str(&format!("### Memories ({})\n\n", memories.len()));
    for mem in &memories {
        report.push_str(&format!(
            "- **{}** ({:?}, {:?}/{:?})\n",
            mem.source_name(),
            mem.level,
            mem.frontmatter.memory_type,
            mem.frontmatter.scope,
        ));
    }
    if memories.is_empty() {
        report.push_str("_(none)_\n");
    }
    report.push('\n');

    let tools = crate::tools::model_tool_defs();
    report.push_str(&format!(
        "### Tool definitions sent to API ({})\n\n",
        tools.len()
    ));
    for tool in &tools {
        report.push_str(&format!("- `{}`\n", tool.name));
    }
    report.push('\n');

    let agents = crate::agents::load_agents(&cwd);
    report.push_str(&format!("### Agents ({})\n\n", agents.len()));
    for a in &agents {
        report.push_str(&format!(
            "- **{}** (model: `{}`, isolation: {:?})\n",
            a.name,
            a.model.as_deref().unwrap_or("inherit"),
            a.isolation
        ));
    }
    if agents.is_empty() {
        report.push_str("_(none)_\n");
    }
    report.push('\n');

    state
        .messages
        .push(jfc_core::ChatMessage::user("/dump-context".to_string()));
    state
        .messages
        .push(jfc_core::ChatMessage::assistant(report));
}

/// `/fleet` prints a snapshot of every active teammate.
pub(super) fn handle_fleet_command(state: &mut EngineState) {
    let mut lines: Vec<String> = Vec::new();
    if state.team_context.teammates.is_empty() {
        lines.push("No active teammates.".into());
        lines.push("Spawn one via the Task tool with `name` + `team_name` set.".into());
    } else {
        let active = state
            .team_context
            .teammates
            .values()
            .filter(|tm| tm.abort_tx.is_some())
            .count();
        let inactive = state.team_context.teammates.len().saturating_sub(active);
        lines.push(format!(
            "Fleet: {active} active, {inactive} inactive teammate{}",
            if state.team_context.teammates.len() == 1 {
                ""
            } else {
                "s"
            }
        ));
        lines.push("".into());
        for tm in state.team_context.teammates.values() {
            let elapsed = tm.spawned_at.elapsed();
            let state = if tm.abort_tx.is_some() {
                "active"
            } else {
                "inactive"
            };
            lines.push(format!(
                "  {} · {state} · {} · spawned {}m{}s ago{}",
                tm.name,
                tm.agent_type.as_deref().unwrap_or("(no agent type)"),
                elapsed.as_secs() / 60,
                elapsed.as_secs() % 60,
                tm.color
                    .as_deref()
                    .map(|c| format!(" · color={c}"))
                    .unwrap_or_default(),
            ));
        }
    }
    state
        .messages
        .push(jfc_core::ChatMessage::user("/fleet".into()));
    state
        .messages
        .push(jfc_core::ChatMessage::assistant(lines.join("\n")));
    tracing::info!(
        target: "jfc::ui::fleet",
        teammates = state.team_context.teammates.len(),
        "/fleet rendered"
    );
}

/// `/teleport [branch]` lists jfc-managed branches or checks out the named
/// branch and resumes that session.
pub(super) async fn handle_teleport_command(state: &mut EngineState, target: &str) {
    use std::path::Path;
    let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
    let repo_root: &Path = cwd.as_path();

    if target.is_empty() {
        let targets = crate::swarm::teleport::list_teleport_targets(repo_root);
        let body = if targets.is_empty() {
            "No jfc-managed branches in this repo (looking for `jfc/<session>` branches).\n\
             Spawn a teammate via Task to create one, or check out a branch with `git checkout`."
                .to_string()
        } else {
            let mut s = format!("Teleport targets ({}):\n\n", targets.len());
            for t in &targets {
                s.push_str(&format!(
                    "  {} → /teleport {}\n",
                    t.session_id.as_deref().unwrap_or("(no session id)"),
                    t.branch
                ));
            }
            s.push_str("\nRun `/teleport <branch>` to jump.");
            s
        };
        state
            .messages
            .push(jfc_core::ChatMessage::user("/teleport".into()));
        state.messages.push(jfc_core::ChatMessage::assistant(body));
        return;
    }

    let target_branch = if target.starts_with("jfc/") {
        target.to_string()
    } else {
        format!("jfc/{target}")
    };
    let result = crate::swarm::teleport::teleport_to_session(repo_root, &target_branch, None);
    state
        .messages
        .push(jfc_core::ChatMessage::user(format!("/teleport {target}")));
    state
        .messages
        .push(jfc_core::ChatMessage::assistant(result.message.clone()));
    tracing::info!(
        target: "jfc::ui::teleport",
        target = %target_branch,
        message = %result.message,
        "/teleport executed"
    );
}

/// `/output-style [name]` switches assistant reply style.
pub(super) fn handle_output_style_command(state: &mut EngineState, args: &str) {
    use crate::output_style::{self, OutputStyle};
    let arg = args.trim();
    if arg.is_empty() {
        let cwd = std::env::current_dir().unwrap_or_else(|_| ".".into());
        let active_name = output_style::active().name();
        let mut lines = vec!["Available output styles:".to_string(), "".to_string()];
        for definition in output_style::load_definitions(&cwd) {
            let active = if definition.name.eq_ignore_ascii_case(&active_name) {
                " · ACTIVE"
            } else {
                ""
            };
            lines.push(format!(
                "  {} — {}{active}",
                definition.name,
                definition.summary()
            ));
        }
        lines.push("".into());
        lines.push("Use `/output-style <name>` to switch.".into());
        state
            .messages
            .push(jfc_core::ChatMessage::user("/output-style".into()));
        state
            .messages
            .push(jfc_core::ChatMessage::assistant(lines.join("\n")));
        return;
    }
    let cwd = std::env::current_dir().unwrap_or_else(|_| ".".into());
    let parsed = OutputStyle::from_str_loose(arg);
    let custom = output_style::find_definition(&cwd, arg);
    if parsed == OutputStyle::Default && !arg.eq_ignore_ascii_case("default") && custom.is_none() {
        crate::toast::push_with_cap(
            &mut state.toasts,
            crate::toast::Toast::new(
                crate::toast::ToastKind::Warning,
                format!(
                    "Unknown output style '{arg}' — try one of: {}",
                    output_style::load_definitions(&cwd)
                        .into_iter()
                        .map(|s| s.name)
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
            ),
        );
        return;
    }
    let old_identity = crate::cache_lineage::current_identity(state);
    state.output_style = parsed;
    crate::output_style::set_active_named(arg);
    let style_name = output_style::active().name();
    let new_identity = crate::cache_lineage::current_identity(state);
    let piggyback_drop = if new_identity == old_identity {
        None
    } else {
        let drop = crate::cache_lineage::maybe_piggyback_drop_for_identity_change(
            state,
            &new_identity,
            "output-style switch",
        );
        state.last_response_id = state
            .response_ids_by_cache_identity
            .get(&new_identity)
            .cloned();
        drop
    };
    let persist_msg = match save_output_style(&style_name) {
        Ok(_) => format!("output style: {}", style_name),
        Err(e) => {
            tracing::warn!(target: "jfc::ui::output_style", style = %style_name, error = %e, "applied but not persisted");
            format!("output style: {} (not persisted: {e})", style_name)
        }
    };
    let persist_msg = append_cache_lineage_status(persist_msg, piggyback_drop);
    crate::toast::push_with_cap(
        &mut state.toasts,
        crate::toast::Toast::new(crate::toast::ToastKind::Success, persist_msg),
    );
}

fn append_cache_lineage_status(
    mut message: String,
    piggyback_drop: Option<crate::cache_lineage::PiggybackDrop>,
) -> String {
    if let Some(drop) = piggyback_drop {
        message.push_str(&format!(
            "; trimmed {} incompatible cache-tail messages",
            drop.dropped_messages
        ));
        if let Some(archive_id) = drop.archive_id {
            message.push_str(&format!(" (/expand {archive_id})"));
        }
    }
    message
}

fn save_output_style(name: &str) -> Result<std::path::PathBuf, String> {
    let path = crate::config::config_path();
    if let Some(parent) = path.parent()
        && let Err(e) = std::fs::create_dir_all(parent)
    {
        return Err(format!("cannot create {}: {e}", parent.display()));
    }
    let mut cfg: crate::config::Config = match std::fs::read_to_string(&path) {
        Ok(s) if !s.trim().is_empty() => match toml::from_str(&s) {
            Ok(c) => c,
            Err(e) => {
                return Err(format!(
                    "{} is not valid TOML — fix it first ({e})",
                    path.display()
                ));
            }
        },
        _ => crate::config::Config::default(),
    };
    cfg.output_style = Some(name.to_string());
    let serialized = toml::to_string_pretty(&cfg).map_err(|e| format!("serialize failed: {e}"))?;
    std::fs::write(&path, serialized)
        .map_err(|e| format!("write {} failed: {e}", path.display()))?;
    Ok(path)
}

/// `/plan`, `/roadmap`, `/parity`, `/philosophy`, and `/usage` start a normal
/// model turn that asks JFC to create or update the matching project document.
pub(super) async fn handle_doc_command(
    state: &mut EngineState,
    kind: crate::document_formats::DocKind,
    tx: Option<&mpsc::Sender<EngineEvent>>,
) {
    let cwd = std::path::PathBuf::from(&state.cwd);
    let target = crate::document_formats::doc_target(&cwd, kind);
    let exists = target.is_file();
    let echo = format!("/{}", kind.verb());
    let body = kind.prompt_body(&target, exists);
    let action = if exists { "Updating" } else { "Drafting" };

    let idle = !state.is_streaming
        && state.pending_approval.is_none()
        && state.approval_queue.is_empty()
        && state.pending_tool_calls.is_empty();

    if let (true, Some(tx)) = (idle, tx) {
        state.messages.push(ChatMessage::user(echo));
        state.push_effect(crate::app::EngineEffect::ScrollToBottom);
        let _ = tx
            .send(EngineEvent::Control(ControlEvent::SubmitPrompt(body)))
            .await;
        tracing::info!(
            target: "jfc::doc_command",
            kind = kind.file_name(),
            "doc command dispatched immediately (idle session)"
        );
    } else {
        state.messages.push(ChatMessage::user(echo));
        state.messages.push(ChatMessage::assistant(format!(
            "{action} `{}` … (queued — will run when the current turn finishes)",
            target.display()
        )));
        state.queued_prompts.push(crate::runtime::QueuedPrompt {
            text: body,
            is_meta: false,
            priority: crate::runtime::QueuePriority::Later,
            attachments: Vec::new(),
        });
        state.push_effect(crate::app::EngineEffect::ScrollToBottom);
        tracing::info!(
            target: "jfc::doc_command",
            kind = kind.file_name(),
            "doc command queued (session busy)"
        );
    }
}

/// `/init` bootstraps a CLAUDE.md in the current working directory.
pub(super) async fn handle_init_command(state: &mut EngineState) {
    let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
    let target = cwd.join("CLAUDE.md");

    state
        .messages
        .push(jfc_core::ChatMessage::user("/init".into()));

    let overwrite_note = if target.exists() {
        format!(
            "> **Note:** `{}` already exists and will be overwritten.\n\n",
            target.display()
        )
    } else {
        String::new()
    };

    struct ProjectKind {
        description: &'static str,
        build_cmd: &'static str,
        test_cmd: &'static str,
    }

    let has = |name: &str| cwd.join(name).exists();

    let mut kinds: Vec<ProjectKind> = Vec::new();

    if has("Cargo.toml") {
        kinds.push(ProjectKind {
            description: "Rust (Cargo)",
            build_cmd: "cargo build",
            test_cmd: "cargo test",
        });
    }
    if has("package.json") {
        kinds.push(ProjectKind {
            description: "Node.js / JavaScript",
            build_cmd: "npm run build",
            test_cmd: "npm test",
        });
    }
    if has("go.mod") {
        kinds.push(ProjectKind {
            description: "Go",
            build_cmd: "go build ./...",
            test_cmd: "go test ./...",
        });
    }
    if has("pyproject.toml") || has("requirements.txt") {
        kinds.push(ProjectKind {
            description: "Python",
            build_cmd: "pip install -e .",
            test_cmd: "pytest",
        });
    }

    if kinds.is_empty() {
        kinds.push(ProjectKind {
            description: "Unknown",
            build_cmd: "# add your build command here",
            test_cmd: "# add your test command here",
        });
    }

    let is_polyglot = kinds.len() > 1;
    let type_description = if is_polyglot {
        let names: Vec<&str> = kinds.iter().map(|k| k.description).collect();
        format!("Polyglot project ({})", names.join(", "))
    } else {
        kinds[0].description.to_owned()
    };

    let build_cmd = kinds[0].build_cmd;
    let test_cmd = kinds[0].test_cmd;

    let lint_cmd: Option<&str> = if has("Cargo.toml") {
        Some("cargo clippy")
    } else if has("package.json") {
        Some("npm run lint")
    } else if has("go.mod") {
        Some("golangci-lint run")
    } else if has("pyproject.toml") || has("requirements.txt") {
        Some("ruff check .")
    } else {
        None
    };

    let lint_line = match lint_cmd {
        Some(cmd) => format!("- **Lint**: `{cmd}`\n"),
        None => String::new(),
    };

    let arch_note: String = if has("Cargo.toml") {
        let crate_count = std::fs::read_dir(&cwd)
            .ok()
            .map(|rd| {
                rd.filter_map(|e| e.ok())
                    .filter(|e| {
                        let p = e.path();
                        p.is_dir() && p.join("Cargo.toml").exists()
                    })
                    .count()
            })
            .unwrap_or(0);
        let is_workspace = std::fs::read_to_string(cwd.join("Cargo.toml"))
            .map(|s| s.contains("[workspace]"))
            .unwrap_or(false);
        if is_workspace && crate_count > 0 {
            format!(
                "Cargo workspace with {} member crate(s) found in subdirectories.",
                crate_count
            )
        } else {
            "Single-crate Cargo project.".to_owned()
        }
    } else if has("package.json") {
        std::fs::read_to_string(cwd.join("package.json"))
            .ok()
            .and_then(|s| {
                let start = s.find("\"scripts\"")?;
                let block = &s[start..];
                let open = block.find('{')?;
                let close = block[open..].find('}')?;
                Some(block[open + 1..open + close].to_owned())
            })
            .map(|block| {
                block
                    .lines()
                    .map(|l| l.trim())
                    .filter(|l| l.contains(':'))
                    .map(|l| format!("  {l}"))
                    .collect::<Vec<_>>()
                    .join("\n")
            })
            .filter(|s| !s.is_empty())
            .map(|s| format!("package.json scripts:\n{s}"))
            .unwrap_or_else(|| "Node.js project (no scripts detected).".to_owned())
    } else if has("go.mod") {
        "Go module project.".to_owned()
    } else if has("pyproject.toml") {
        "Python project with pyproject.toml.".to_owned()
    } else if has("requirements.txt") {
        "Python project with requirements.txt.".to_owned()
    } else {
        "Project structure not automatically detected.".to_owned()
    };

    let claude_md = format!(
        "# Project\n\n\
         {type_description}\n\n\
         ## Commands\n\n\
         - **Build**: `{build_cmd}`\n\
         - **Test**: `{test_cmd}`\n\
         {lint_line}\n\
         ## Architecture\n\n\
         {arch_note}\n\n\
         ## Agent Instructions\n\n\
         - Read files before editing\n\
         - Run tests after changes\n\
         - Keep commits atomic\n"
    );

    let body = match tokio::fs::write(&target, &claude_md).await {
        Ok(()) => {
            tracing::info!(
                target: "jfc::ui::init",
                path = %target.display(),
                project_type = %type_description,
                "wrote CLAUDE.md via /init"
            );
            format!(
                "{overwrite_note}✓ CLAUDE.md written to `{}`\n\n\
                 Detected project type: **{type_description}**\n\n\
                 Edit the file to add coding standards, architectural patterns, \
                 or anything you want every AI turn to remember.",
                target.display(),
            )
        }
        Err(e) => format!("**Error:** Failed to write `{}`: {e}", target.display()),
    };

    state.messages.push(jfc_core::ChatMessage::assistant(body));
}

/// `/cost` reports running session cost.
pub(super) fn handle_cost_command(state: &mut EngineState) {
    let mut total = 0.0f64;
    let mut lines: Vec<String> = vec!["Session cost so far:".into(), "".into()];
    if state.usage_by_model.is_empty() {
        lines.push("  (no model usage yet — try a prompt first)".into());
    } else {
        for (model, usage) in &state.usage_by_model {
            let cost = crate::cost::cost_for(model.as_str(), usage);
            total += cost;
            lines.push(format!(
                "  {} · {} in / {} out / {} cache-read / {} cache-write → {}",
                model.as_str(),
                usage.input_tokens,
                usage.output_tokens,
                usage.cache_read_tokens,
                usage.cache_write_tokens,
                crate::cost::fmt_cost(cost),
            ));
        }
    }
    lines.push("".into());
    lines.push(format!("**Total: {}**", crate::cost::fmt_cost(total)));
    state
        .messages
        .push(jfc_core::ChatMessage::user("/cost".into()));
    state
        .messages
        .push(jfc_core::ChatMessage::assistant(lines.join("\n")));
}

/// `/usage-report` shows per-model token usage, cache hits, cost, and budget percent if configured.
pub(super) fn handle_usage_report_command(state: &mut EngineState) {
    let mut total_cost = 0.0f64;
    let mut total_in = 0u64;
    let mut total_out = 0u64;
    let mut total_cr = 0u64;
    let mut total_cw = 0u64;
    let mut lines: Vec<String> = vec!["Usage by model:".into(), "".into()];
    if state.usage_by_model.is_empty() {
        lines.push("  (no usage yet)".into());
    } else {
        let mut models: Vec<_> = state.usage_by_model.iter().collect();
        models.sort_by_key(|(m, _)| (*m).clone());
        for (model, usage) in models {
            let cost = crate::cost::cost_for(model.as_str(), usage);
            total_cost += cost;
            total_in += usage.input_tokens;
            total_out += usage.output_tokens;
            total_cr += usage.cache_read_tokens;
            total_cw += usage.cache_write_tokens;
            lines.push(format!(
                "- {}: {} in / {} out / {} cache-read / {} cache-write → {}",
                model,
                usage.input_tokens,
                usage.output_tokens,
                usage.cache_read_tokens,
                usage.cache_write_tokens,
                crate::cost::fmt_cost(cost)
            ));
        }
    }
    lines.push("".into());
    lines.push(format!(
        "Totals: {} in / {} out / {} cache-read / {} cache-write",
        total_in, total_out, total_cr, total_cw
    ));
    lines.push(format!("Total cost: {}", crate::cost::fmt_cost(total_cost)));
    if let Some(budget_usd) = crate::config::load_arc().session_cost_budget_usd {
        let pct = if budget_usd > 0.0 {
            ((total_cost / budget_usd) * 100.0).round() as u64
        } else {
            0
        };
        lines.push(format!(
            "Budget: {} of {} ({}%)",
            crate::cost::fmt_cost(total_cost),
            crate::cost::fmt_cost(budget_usd),
            pct
        ));
    }
    state
        .messages
        .push(jfc_core::ChatMessage::user("/usage-report".into()));
    state
        .messages
        .push(jfc_core::ChatMessage::assistant(lines.join("\n")));
}

/// `/status` reports rich session status.
pub(super) fn handle_status_command(state: &mut EngineState) {
    let (total_in, total_out, total_cr, total_cw) =
        state
            .usage_by_model
            .values()
            .fold((0u64, 0u64, 0u64, 0u64), |(i, o, cr, cw), u| {
                (
                    i + u.input_tokens,
                    o + u.output_tokens,
                    cr + u.cache_read_tokens,
                    cw + u.cache_write_tokens,
                )
            });
    let total_cost: f64 = state
        .usage_by_model
        .iter()
        .map(|(m, u)| crate::cost::cost_for(m.as_str(), u))
        .sum();

    let model_str = state.model.as_str();
    let provider_label = state.provider.name();
    let turn_count = state
        .messages
        .iter()
        .filter(|m| m.role == jfc_core::Role::User)
        .count();
    let mcp_count = state.mcp_servers.len();
    let effort_label = state.effort_state.status();
    let temperature_label = state.temperature_state.status();
    let exploration_label = state.exploration_state.status();

    let lines = vec![
        format!("**Version:** jfc v{}", env!("CARGO_PKG_VERSION")),
        format!("**Model:** {model_str}"),
        format!("**Provider:** {provider_label}"),
        format!("**Turns:** {turn_count}"),
        format!(
            "**Tokens:** {} in / {} out / {} cache-read / {} cache-write",
            total_in, total_out, total_cr, total_cw
        ),
        format!("**Cost:** {}", crate::cost::fmt_cost(total_cost)),
        format!("**MCP servers:** {mcp_count} active"),
        format!(
            "**Fast mode:** {}",
            if state.fast_mode { "ON" } else { "OFF" }
        ),
        format!("**Effort:** {effort_label}"),
        format!("**Temperature:** {temperature_label}"),
        format!("**Exploration:** {exploration_label}"),
    ];
    state
        .messages
        .push(jfc_core::ChatMessage::user("/status".into()));
    state
        .messages
        .push(jfc_core::ChatMessage::assistant(lines.join("\n")));
}

/// `/bug` opens a pre-filled GitHub issue with environment + session
/// context and echoes the same context into the transcript so the user
/// can copy it if their browser doesn't open. Mirrors `gh issue create
/// --web` and Claude Code's `/bug`: the title carries the user's short
/// summary; the body carries the structured environment block.
pub(super) fn handle_bug_command(state: &mut EngineState, description: String) {
    let session_id = state
        .current_session_id
        .as_ref()
        .map(|s| s.as_str())
        .unwrap_or("(none)");
    let trimmed_desc = description.trim();
    let title = if trimmed_desc.is_empty() {
        String::new()
    } else {
        // GitHub's issue title input caps at ~256 chars; clamp here so
        // a giant paste doesn't produce a 414 URL-too-long response.
        trimmed_desc.chars().take(120).collect()
    };
    let body = format!(
        "**Describe the issue**\n\n\
         {}\n\n\
         **Environment**\n\
         - jfc version: `{}`\n\
         - Provider/model: `{}` / `{}`\n\
         - Permission mode: `{:?}`\n\
         - OS: `{}`\n\
         - Session ID: `{session_id}`\n\n\
         Tip: run `/dump-context` first to attach the full session transcript.",
        if trimmed_desc.is_empty() {
            "(your description here)"
        } else {
            trimmed_desc
        },
        env!("CARGO_PKG_VERSION"),
        state.provider.name(),
        state.model.as_str(),
        state.permission_mode,
        std::env::consts::OS,
    );
    let url = super::support::bug_report_url(&title, &body);
    state.messages.push(jfc_core::ChatMessage::user(
        format!("/bug {description}").trim_end().into(),
    ));
    // No browser launch here: this code is engine-resident (headless and
    // remote frontends run it too), so the honest contract is to hand the
    // user the pre-filled URL rather than claim a browser opened.
    state
        .messages
        .push(jfc_core::ChatMessage::assistant(format!(
            "Pre-filled bug report ready — open this in your browser:\n\n\
             {url}\n\n\
             Context already attached:\n\
             - **Session ID**: `{session_id}`\n\
             - **Provider/model**: `{}` / `{}`\n\
             - **Mode**: {:?}",
            state.provider.name(),
            state.model.as_str(),
            state.permission_mode,
        )));
}

/// `/rewind [N]` drops the last N user/assistant turn pairs from the transcript.
pub(super) fn handle_rewind_command(state: &mut EngineState, n_str: &str) {
    let n: usize = n_str.parse().unwrap_or(1).max(1);
    use jfc_core::Role;
    let mut dropped_pairs = 0usize;
    while dropped_pairs < n {
        let last_user_idx = state.messages.iter().rposition(|m| m.role == Role::User);
        match last_user_idx {
            Some(idx) => {
                let removed = state.messages.split_off(idx).len();
                tracing::info!(
                    target: "jfc::ui::rewind",
                    pair = dropped_pairs + 1,
                    removed,
                    remaining = state.messages.len(),
                    "rewind: dropped a turn pair"
                );
                dropped_pairs += 1;
            }
            None => break,
        }
    }
    let body = if dropped_pairs == 0 {
        "Nothing to rewind — transcript is empty or has no user turns.".to_string()
    } else {
        format!(
            "Rewound {} turn pair{} ({} message{} remaining). Re-prompt to continue \
             from this point — the trimmed history is gone for this session.",
            dropped_pairs,
            if dropped_pairs == 1 { "" } else { "s" },
            state.messages.len(),
            if state.messages.len() == 1 { "" } else { "s" },
        )
    };
    crate::toast::push_with_cap(
        &mut state.toasts,
        crate::toast::Toast::new(crate::toast::ToastKind::Info, body.clone()),
    );
    state.messages.push(jfc_core::ChatMessage::assistant(body));
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use futures::stream::empty;
    use jfc_provider::{
        CompletionResponse, EventStream, ModelInfo, Provider, ProviderMessage, StreamOptions,
        TokenUsage,
    };

    use super::*;

    struct NoopProvider;

    impl jfc_provider::seal::Sealed for NoopProvider {}

    #[async_trait::async_trait]
    impl Provider for NoopProvider {
        fn name(&self) -> &str {
            "noop"
        }

        fn available_models(&self) -> Vec<ModelInfo> {
            Vec::new()
        }

        async fn stream(
            &self,
            _messages: Vec<ProviderMessage>,
            _options: &StreamOptions,
        ) -> anyhow::Result<EventStream> {
            Ok(Box::pin(empty()))
        }

        async fn complete(
            &self,
            _messages: Vec<ProviderMessage>,
            _options: &StreamOptions,
        ) -> anyhow::Result<CompletionResponse> {
            Ok(CompletionResponse {
                content: String::new(),
                usage: TokenUsage::default(),
                context_signals: None,
                reasoning: None,
            })
        }
    }

    #[tokio::test]
    async fn dump_context_uses_descriptor_backed_plugin_agent_skill_roots_normal() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let util_skill = tmp.path().join("plugins/util/skills/format");
        let util_agent = tmp.path().join("plugins/util/agents");
        let sec_root = tmp.path().join("plugins/sec");
        let sec_skill = sec_root.join("skills/audit");
        let sec_agent = sec_root.join("agents");
        std::fs::create_dir_all(&util_skill).expect("create util skill");
        std::fs::create_dir_all(&util_agent).expect("create util agent");
        std::fs::create_dir_all(&sec_skill).expect("create sec skill");
        std::fs::create_dir_all(&sec_agent).expect("create sec agent");
        std::fs::create_dir_all(tmp.path().join(".claude")).expect("create settings dir");

        std::fs::write(
            tmp.path().join("plugins/util/.jfc-plugin.toml"),
            "[plugin]\nname = \"util-plugin\"\n",
        )
        .expect("write util manifest");
        std::fs::write(
            sec_root.join(".jfc-plugin.toml"),
            "[plugin]\nname = \"sec-plugin\"\n",
        )
        .expect("write sec manifest");
        std::fs::write(util_skill.join("SKILL.md"), "---\nname: format\n---\nbody")
            .expect("write util skill");
        std::fs::write(
            util_agent.join("helper.md"),
            "---\nname: helper\n---\nHelp format things.",
        )
        .expect("write util agent");
        std::fs::write(sec_skill.join("SKILL.md"), "---\nname: audit\n---\nbody")
            .expect("write sec skill");
        std::fs::write(
            sec_agent.join("reviewer.md"),
            "---\nname: reviewer\n---\nReview things.",
        )
        .expect("write sec agent");
        std::fs::write(
            tmp.path().join(".claude/settings.json"),
            r#"{ "enabledPlugins": { "sec-plugin@local": false } }"#,
        )
        .expect("write settings");

        let mut state = EngineState::new(Arc::new(NoopProvider), "test-model");
        state.cwd = tmp.path().to_string_lossy().into_owned();

        crate::commands::run_command(&mut state, "/dump-context", None).await;

        let report = state
            .messages
            .last()
            .expect("dump report")
            .parts
            .iter()
            .map(jfc_core::MessagePart::text_only)
            .collect::<String>();
        assert!(report.contains("`util:format`"), "{report}");
        assert!(report.contains("**util:helper**"), "{report}");
        assert!(!report.contains("`sec:audit`"), "{report}");
        assert!(!report.contains("**sec:reviewer**"), "{report}");
    }
}
