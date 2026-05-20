//! Slash command handlers, part 2: /batch through /timeline.

use super::*;

pub(super) async fn handle_slash_command_extended(
    app: &mut App,
    parts: &[&str],
    text: &str,
    tx: Option<&mpsc::Sender<AppEvent>>,
) {
    match parts[0] {
        "/batch" => {
            // /batch <prompt-file>: read newline-delimited prompts and
            // submit them via Anthropic's Message Batches API for the
            // 50% discount. The batch ID is returned synchronously;
            // results stream back via the Sessions API in a follow-up
            // turn (poll `/batch status <id>`).
            app.messages.push(ChatMessage::user(text.to_owned()));
            let arg = parts.get(1).copied().unwrap_or("").trim();
            if arg.is_empty() {
                app.messages.push(ChatMessage::assistant(
                    "Usage: `/batch <prompt-file>`. The file should contain one prompt per line."
                        .into(),
                ));
                return;
            }
            let path = std::path::PathBuf::from(arg);
            let content = match std::fs::read_to_string(&path) {
                Ok(c) => c,
                Err(e) => {
                    app.messages.push(ChatMessage::assistant(format!(
                        "Failed to read `{}`: {e}",
                        path.display(),
                    )));
                    return;
                }
            };
            let prompts: Vec<String> = content
                .lines()
                .map(|l| l.trim().to_owned())
                .filter(|l| !l.is_empty() && !l.starts_with('#'))
                .collect();
            if prompts.is_empty() {
                app.messages.push(ChatMessage::assistant(
                    "No prompts found (each non-empty, non-`#`-comment line counts as one).".into(),
                ));
                return;
            }
            let Some(client) = crate::sdk_bridge::build_client() else {
                app.messages.push(ChatMessage::assistant(
                    "No Anthropic API key configured — `/batch` needs one (set ANTHROPIC_API_KEY)."
                        .into(),
                ));
                return;
            };
            let model = app.model.as_str().to_owned();
            let prompt_count = prompts.len();
            let path_for_msg = path.display().to_string();
            tokio::spawn(async move {
                use jfc_anthropic_sdk::batches::{BatchRequest, MessageBatchService};
                use jfc_anthropic_sdk::messages::{ContentBlock, Message, MessageRequest, Role};
                let svc = MessageBatchService::new(client);
                let requests: Vec<BatchRequest> = prompts
                    .into_iter()
                    .enumerate()
                    .map(|(i, p)| BatchRequest {
                        custom_id: format!("batch-{i}"),
                        params: MessageRequest {
                            model: model.clone(),
                            messages: vec![Message {
                                role: Role::User,
                                content: vec![ContentBlock::Text { text: p }],
                            }],
                            max_tokens: 4096,
                            system: None,
                            temperature: None,
                            top_p: None,
                            stop_sequences: Vec::new(),
                            tools: Vec::new(),
                            tool_choice: None,
                            stream: Some(false),
                            thinking: None,
                            reasoning_effort: None,
                        },
                    })
                    .collect();
                match svc.create(requests).await {
                    Ok(batch) => {
                        tracing::info!(
                            target: "jfc::batch",
                            batch_id = %batch.id,
                            count = prompt_count,
                            "batch submitted"
                        );
                        eprintln!(
                            "[batch] submitted {prompt_count} prompts from {path_for_msg} → batch {}",
                            batch.id
                        );
                    }
                    Err(e) => {
                        eprintln!("[batch] failed: {e}");
                    }
                }
            });
            app.messages.push(ChatMessage::assistant(format!(
                "Queued {prompt_count} prompts from `{}` for batch processing. \
                 Watch stderr / `/doctor` for the batch ID.",
                path.display()
            )));
        }
        "/diff" => {
            // Show pending uncommitted + unstaged changes via `git diff
            // HEAD --stat`. Read-only; doesn't run unless we're in a
            // git repo. Surface in the transcript as an assistant
            // message (markdown code block) so the user — and the
            // model on the next turn — can see what's pending.
            app.messages.push(ChatMessage::user(text.to_owned()));
            let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
            let in_repo = std::process::Command::new("git")
                .args(["rev-parse", "--is-inside-work-tree"])
                .current_dir(&cwd)
                .output()
                .map(|o| o.status.success())
                .unwrap_or(false);
            if !in_repo {
                app.messages.push(ChatMessage::assistant(
                    "Not inside a git repository — `/diff` has nothing to show.".into(),
                ));
                return;
            }
            let stat = std::process::Command::new("git")
                .args(["diff", "HEAD", "--stat"])
                .current_dir(&cwd)
                .output()
                .ok()
                .and_then(|o| String::from_utf8(o.stdout).ok())
                .unwrap_or_default();
            let untracked = std::process::Command::new("git")
                .args(["ls-files", "--others", "--exclude-standard"])
                .current_dir(&cwd)
                .output()
                .ok()
                .and_then(|o| String::from_utf8(o.stdout).ok())
                .unwrap_or_default();
            if stat.trim().is_empty() && untracked.trim().is_empty() {
                app.messages.push(ChatMessage::assistant(
                    "Working tree is clean — no pending changes.".into(),
                ));
            } else {
                let mut body = String::from("**Pending changes (`git diff HEAD`):**\n\n```\n");
                if !stat.trim().is_empty() {
                    body.push_str(&stat);
                } else {
                    body.push_str("(no tracked-file changes)\n");
                }
                if !untracked.trim().is_empty() {
                    body.push_str("\n--- untracked ---\n");
                    body.push_str(&untracked);
                }
                body.push_str("```\n");
                app.messages.push(ChatMessage::assistant(body));
            }
        }
        "/undo" => {
            // Revert the most recent Edit / Write / MultiEdit /
            // ApplyPatch tool's filesystem mutation. Pulls from
            // `app.tool_undo_history` which the tool dispatcher
            // populates by capturing pre-mutation file content
            // before the tool executes. Only undoes ONE step;
            // run /undo repeatedly to walk back further.
            app.messages.push(ChatMessage::user(text.to_owned()));
            let entry = crate::tools::pop_undo_entry();
            let Some(entry) = entry else {
                app.messages.push(ChatMessage::assistant(
                    "Nothing to undo — no recent file mutation captured this session.".into(),
                ));
                return;
            };
            let path = std::path::PathBuf::from(&entry.file_path);
            match entry.previous_content.clone() {
                Some(prev) => match std::fs::write(&path, &prev) {
                    Ok(()) => {
                        app.messages.push(ChatMessage::assistant(format!(
                            "Reverted `{}` to its pre-{} state ({} bytes restored).",
                            path.display(),
                            entry.op_label,
                            prev.len()
                        )));
                    }
                    Err(e) => {
                        crate::tools::restore_undo_entry(entry.clone());
                        app.messages.push(ChatMessage::assistant(format!(
                            "Failed to write `{}`: {e} (kept the entry, run /undo again after fixing)",
                            path.display(),
                        )));
                    }
                },
                None => match std::fs::remove_file(&path) {
                    Ok(()) => {
                        app.messages.push(ChatMessage::assistant(format!(
                            "Reverted `{}` (deleted; was newly-created by `{}`).",
                            path.display(),
                            entry.op_label
                        )));
                    }
                    Err(e) => {
                        crate::tools::restore_undo_entry(entry.clone());
                        app.messages.push(ChatMessage::assistant(format!(
                            "Failed to remove `{}`: {e}",
                            path.display(),
                        )));
                    }
                },
            }
        }
        "/export" => {
            // /export <path>: write the transcript as markdown to the
            // given path (defaults to ./jfc-transcript.md).
            app.messages.push(ChatMessage::user(text.to_owned()));
            let raw_path = parts.get(1).copied().unwrap_or("").trim();
            let path: std::path::PathBuf = if raw_path.is_empty() {
                std::path::PathBuf::from("jfc-transcript.md")
            } else {
                std::path::PathBuf::from(raw_path)
            };
            let mut body = String::from("# jfc transcript\n\n");
            for msg in &app.messages {
                let role = match msg.role {
                    crate::types::Role::User => "User",
                    crate::types::Role::Assistant => "Assistant",
                };
                body.push_str(&format!("## {role}\n\n"));
                for part in &msg.parts {
                    match part {
                        crate::types::MessagePart::Text(t) => {
                            body.push_str(t);
                            body.push_str("\n\n");
                        }
                        crate::types::MessagePart::Reasoning(t) => {
                            body.push_str("> _thinking_\n> \n> ");
                            body.push_str(&t.replace('\n', "\n> "));
                            body.push_str("\n\n");
                        }
                        crate::types::MessagePart::Tool(tc) => {
                            body.push_str(&format!(
                                "- **Tool: {}** ({})\n",
                                tc.kind.label(),
                                tc.status.label()
                            ));
                            body.push_str(&format!("  Input: {}\n", tc.input.summary()));
                            body.push('\n');
                        }
                        _ => {}
                    }
                }
            }
            match std::fs::write(&path, &body) {
                Ok(()) => {
                    let message = format!(
                        "Wrote transcript ({} bytes) to `{}`.",
                        body.len(),
                        path.display()
                    );
                    app.messages.push(ChatMessage::assistant(message.clone()));
                    crate::toast::push_with_cap(
                        &mut app.toasts,
                        crate::toast::Toast::new(crate::toast::ToastKind::Success, message),
                    );
                }
                Err(e) => {
                    let message = format!("Failed to write `{}`: {e}", path.display());
                    app.messages.push(ChatMessage::assistant(message.clone()));
                    crate::toast::push_with_cap(
                        &mut app.toasts,
                        crate::toast::Toast::new(crate::toast::ToastKind::Error, message),
                    );
                }
            }
        }
        "/verbose" => {
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
        "/fast" | "/f" => {
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
        "/pin" => {
            // Pin a message by transcript index so compaction can't
            // drop it. /pin without an arg pins the most recent
            // message; /pin <n> pins index n; /pin list prints the
            // current pin set.
            app.messages.push(ChatMessage::user(text.to_owned()));
            let arg = parts.get(1).copied().unwrap_or("").trim();
            if arg == "list" {
                if app.pinned_message_indices.is_empty() {
                    app.messages.push(ChatMessage::assistant(
                        "No pinned messages. `/pin <n>` pins index n; `/pin` pins the most recent."
                            .into(),
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
        "/unpin" => {
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
        "/timeline" => {
            // Render a chronological tool-call timeline for the most
            // recent assistant turn. For each Tool part, emit one row
            // with "kind │ summary │ Δms" so the user can spot slow
            // tools at a glance.
            app.messages.push(ChatMessage::user(text.to_owned()));
            let last_assistant = app
                .messages
                .iter()
                .rposition(|m| matches!(m.role, crate::types::Role::Assistant));
            let Some(idx) = last_assistant else {
                app.messages.push(ChatMessage::assistant(
                    "No assistant turn yet — nothing to timeline.".into(),
                ));
                return;
            };
            let msg = &app.messages[idx];
            let mut rows: Vec<String> = Vec::new();
            for part in &msg.parts {
                if let crate::types::MessagePart::Tool(tc) = part {
                    let elapsed = tc
                        .elapsed_ms
                        .map(|ms| {
                            if ms >= 1_000 {
                                format!("{:.1}s", ms as f64 / 1000.0)
                            } else {
                                format!("{ms}ms")
                            }
                        })
                        .unwrap_or_else(|| "—".to_owned());
                    let summary = tc.input.summary();
                    let summary: String = summary.chars().take(60).collect();
                    rows.push(format!(
                        "  - **{}** · `{}` · {elapsed}",
                        tc.kind.label(),
                        summary,
                    ));
                }
            }
            if rows.is_empty() {
                app.messages.push(ChatMessage::assistant(
                    "Most recent assistant turn ran no tools.".into(),
                ));
            } else {
                app.messages.push(ChatMessage::assistant(format!(
                    "**Tool timeline (last assistant turn, {} tools):**\n{}",
                    rows.len(),
                    rows.join("\n"),
                )));
            }
        }
        "/doctor" => {
            // Mirrors Claude Code 2.1.139's /doctor command.
            // Health check: scan the most-likely failure modes for an
            // out-of-the-box jfc setup and surface a single status
            // block. Read-only; no fixes applied automatically — the
            // user opts in to remedies after seeing the report.
            app.messages.push(ChatMessage::user(text.to_owned()));

            let check = |ok: bool| if ok { "✓" } else { "✗" };

            let mut report = String::from("jfc doctor report\n─────────────────\n");

            // ── 1. Config file ────────────────────────────────────────────────
            {
                let cfg_path = crate::config::config_path();
                let cfg_display = cfg_path.display().to_string();
                // Tilde-shorten for readability
                let cfg_display = if let Some(home) = dirs::home_dir() {
                    cfg_display.replacen(&home.display().to_string(), "~", 1)
                } else {
                    cfg_display
                };
                let cfg_ok = cfg_path.exists() && {
                    // Try a parse round-trip to catch TOML errors
                    std::fs::read_to_string(&cfg_path)
                        .ok()
                        .and_then(|s| toml::from_str::<crate::config::Config>(&s).ok())
                        .is_some()
                };
                report.push_str(&format!(
                    "{} Config: {}{}\n",
                    check(cfg_ok),
                    cfg_display,
                    if cfg_ok {
                        ""
                    } else if !cfg_path.exists() {
                        " (not found)"
                    } else {
                        " (parse error)"
                    },
                ));
            }

            // ── 2. Auth: ANTHROPIC_API_KEY env ───────────────────────────────
            {
                let api_key_set = std::env::var("ANTHROPIC_API_KEY").is_ok();
                report.push_str(&format!(
                    "{} Auth: ANTHROPIC_API_KEY {}\n",
                    check(api_key_set),
                    if api_key_set { "set" } else { "not set" },
                ));
            }

            // ── 3. Auth: ~/.config/jfc/anthropic-accounts.json ───────────────
            {
                let accounts_path = dirs::config_dir()
                    .unwrap_or_else(|| std::path::PathBuf::from("."))
                    .join("jfc")
                    .join("anthropic-accounts.json");
                let accounts_ok = accounts_path.exists();
                let accounts_display = {
                    let s = accounts_path.display().to_string();
                    if let Some(home) = dirs::home_dir() {
                        s.replacen(&home.display().to_string(), "~", 1)
                    } else {
                        s
                    }
                };
                report.push_str(&format!(
                    "{} Auth: accounts file {} {}\n",
                    check(accounts_ok),
                    accounts_display,
                    if accounts_ok {
                        "(found)"
                    } else {
                        "(not found)"
                    },
                ));
            }

            // ── 4. CLAUDE.md in project root ──────────────────────────────────
            {
                let project_root = std::path::PathBuf::from(&app.cwd);
                let claude_md = project_root.join("CLAUDE.md");
                let md_ok = claude_md.exists();
                let md_display = format!(
                    "{}{}",
                    "./",
                    claude_md
                        .file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or("CLAUDE.md")
                );
                report.push_str(&format!(
                    "{} CLAUDE.md: {}\n",
                    check(md_ok),
                    if md_ok {
                        md_display
                    } else {
                        format!("{} (not found)", md_display)
                    },
                ));
            }

            // ── 5. MCP servers ────────────────────────────────────────────────
            {
                let cfg = crate::config::load();
                if cfg.mcp.is_empty() {
                    report.push_str("  MCP: no servers configured\n");
                } else {
                    for (name, server) in &cfg.mcp {
                        // Determine the binary to probe: use `command` if set,
                        // otherwise the first element of `args` (e.g. npx), and
                        // fall back to the server name itself.
                        let probe_bin = server
                            .command
                            .as_deref()
                            .filter(|s| !s.is_empty())
                            .or_else(|| server.args.first().map(|s| s.as_str()))
                            .unwrap_or(name.as_str());
                        let found = std::process::Command::new("which")
                            .arg(probe_bin)
                            .output()
                            .map(|o| o.status.success())
                            .unwrap_or(false);
                        report.push_str(&format!(
                            "{} MCP: {} ({} {})\n",
                            check(found),
                            name,
                            probe_bin,
                            if found { "found" } else { "not found" },
                        ));
                    }
                }
            }

            // ── 6. Working directory + git repo ───────────────────────────────
            {
                let cwd = std::path::PathBuf::from(&app.cwd);
                let git_ok = std::process::Command::new("git")
                    .args(["rev-parse", "--git-dir"])
                    .current_dir(&cwd)
                    .output()
                    .map(|o| o.status.success())
                    .unwrap_or(false);
                // Grab current branch name when inside a git repo
                let branch = if git_ok {
                    std::process::Command::new("git")
                        .args(["rev-parse", "--abbrev-ref", "HEAD"])
                        .current_dir(&cwd)
                        .output()
                        .ok()
                        .and_then(|o| {
                            if o.status.success() {
                                String::from_utf8(o.stdout)
                                    .ok()
                                    .map(|s| s.trim().to_owned())
                            } else {
                                None
                            }
                        })
                        .unwrap_or_else(|| "unknown".to_owned())
                } else {
                    String::new()
                };
                let git_label = if git_ok {
                    format!("yes (branch: {branch})")
                } else {
                    "no".to_owned()
                };
                report.push_str(&format!("{} Git repo: {}\n", check(git_ok), git_label));
                report.push_str(&format!("  cwd: {}\n", cwd.display()));
            }

            // ── 7. Version ────────────────────────────────────────────────────
            report.push_str(&format!("  Version: {}\n", env!("CARGO_PKG_VERSION")));

            // ── 8. Bonus: active provider + permission mode ───────────────────
            report.push_str(&format!("  Provider: {}\n", app.provider.name()));
            report.push_str(&format!("  Permission mode: {:?}\n", app.permission_mode));

            // ── 9. Session cost so far ────────────────────────────────────────
            let total = crate::cost::total_cost(&app.usage_by_model);
            report.push_str(&format!(
                "  Session cost: {}\n",
                crate::cost::fmt_cost(total)
            ));

            app.messages.push(ChatMessage::assistant(report));
        }
        "/effort" => {
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
        "/feature" => {
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
        "/goal" => {
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
                    crate::toast::Toast::new(
                        crate::toast::ToastKind::Success,
                        "Goal cleared".to_string(),
                    ),
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
        "/help" => {
            // Also flip the visual overlay so users get the same
            // keybindings table they'd see from `?`. The text dump
            // below is kept for searchability + transcript export.
            app.show_help = true;
            app.messages.push(ChatMessage::user("/help".into()));
            app.messages.push(ChatMessage::assistant(
                "**Available commands:**\n\
                 - `/clear` — Clear conversation and start fresh\n\
                 - `/compact` — Manually compact the conversation\n\
                 - `/advisor <question>` — Ask a parallel advisor without disturbing the main agent (set `JFC_ADVISOR_ENABLED=1`)\n\
                 - `/check` — Re-run cargo-check diagnostics\n\
                 - `/config` — Show parsed `~/.config/jfc/config.toml` (use `/config path` for the file location)\n\
                 - `/continue` (or `/c`) — Resume most recent session\n\
                 - `/resume <id>` — Resume a specific session by id\n\
                 - `/sessions` — List all saved sessions\n\
                 - `/theme [name]` — Open theme picker or switch/persist a theme\n\
                 - `/auto-mode on` — Enable v126-style LLM tool classifier (no user prompts)\n\
                 - `/auto-mode off` — Disable auto-mode, restore manual approval\n\
                 - `/auto-mode status` — Show current state + rule sources\n\
                 - `/skills` — List available skills (.claude/skills/*.md)\n\
                 - `/agents` — List available agent definitions (.claude/agents/*.md)\n\
                 - `/claude-md` — Show which CLAUDE.md layers are loaded\n\
                 - `/tasks` — List todo/task items\n\
                 - `/task-add <subject>` — Create a new task\n\
                 - `/task-done <id>` — Mark task completed\n\
                 - `/task-rm <id>` — Delete task\n\
                 - `/worktree [list|create <name>|remove <name>|switch <name>]` — Manage `.jfc-worktrees/<name>` checkouts on `jfc/<name>` branches\n\
                 - `/install-github-app` — Install Claude GitHub App on the current repo (browser flow)\n\
                 - `/pr <num>` — Show PR title, description, and review comments\n\
                 - `/pr-autofix <num>` — Build a model prompt that addresses PR review comments\n\
                 - `/setup-github-actions [force]` — Write `.github/workflows/jfc-review.yml`\n\
                 - `/help` — Show this message\n\
                 \n\
                 **Keys:**\n\
                 - Ctrl+B — Toggle sessions sidebar\n\
                 - Ctrl+M — Model picker\n\
                 - Ctrl+P — Command palette\n\
                 - Ctrl+O — Expand reasoning / open diagnostic panel\n\
                 - Alt+. / Alt+, — Raise / lower reasoning effort\n\
                 - Ctrl+Y — Yank last assistant message to clipboard\n\
                 - Ctrl+S — Toggle info sidebar\n\
                 - `@` — Autocomplete file paths from cwd\n\
                 - Up — Recall most recent queued prompt / cycle history (when input empty)\n\
                 - Esc — Dismiss popup / close diagnostic panel\n\
                 \n\
                 **Env knobs:**\n\
                 - `JFC_DISABLE_BELL=1` — silence terminal bell on tool completion\n\
                 - `JFC_DISABLE_AUTO_COMPACT=1` — disable auto-compaction\n\
                 - `JFC_DISABLE_CARGO_CHECK=1` — skip startup `cargo check`\n\
                 - `JFC_AUTOCOMPACT_PCT_OVERRIDE=N` — force compact threshold\n\
                 - `JFC_TOOL_TITLE_WIDTH=N` — cap tool title length (default 100)\n\
                 - `JFC_ADVISOR_ENABLED=1` — enable the `/advisor` parallel-advice slash command"
                    .into(),
            ));
        }
        _ => {
            super::slash_commands_ext2::handle_slash_command_extended2(app, parts, text, tx).await;
        }
    }
}
