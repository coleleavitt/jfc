//! Slash command handlers, part 3: /memory through /swarm-*.

use super::*;

pub(super) async fn handle_slash_command_extended2(
    app: &mut App,
    parts: &[&str],
    text: &str,
    tx: Option<&mpsc::Sender<AppEvent>>,
) {
    match parts[0] {
        "/memory" | "/mem" => {
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
        "/commit" => {
            // Generate a conventional commit message for staged changes.
            // 1. Check if anything is staged; bail early if not.
            // 2. Capture `git diff --cached` (capped at 8000 chars).
            // 3. Inject a user prompt so the model generates the message
            //    on the next turn — the user can then copy/run `git commit`.
            app.messages.push(ChatMessage::user("/commit".into()));
            let cwd = app.cwd.clone();
            let stat = tokio::process::Command::new("git")
                .args(["diff", "--cached", "--stat"])
                .current_dir(&cwd)
                .output()
                .await;
            match stat {
                Err(e) => {
                    app.messages.push(ChatMessage::assistant(format!(
                        "Could not run `git diff --cached --stat`: {e}"
                    )));
                }
                Ok(out) => {
                    let stat_str = String::from_utf8_lossy(&out.stdout);
                    if stat_str.trim().is_empty() {
                        app.messages.push(ChatMessage::assistant(
                            "Nothing staged. Stage changes first with `git add <file>` or `git add -p`.".into(),
                        ));
                    } else {
                        // Fetch the full diff, capped at 8000 chars to stay
                        // well within any reasonable context window.
                        let diff_output = tokio::process::Command::new("git")
                            .args(["diff", "--cached"])
                            .current_dir(&cwd)
                            .output()
                            .await
                            .ok();
                        let diff_str = diff_output
                            .map(|o| {
                                let s = String::from_utf8_lossy(&o.stdout).into_owned();
                                if s.len() > 8000 {
                                    // floor_char_boundary instead of a raw
                                    // byte slice — git diff can carry
                                    // non-ASCII filenames or content and
                                    // a fixed-byte cap would panic if a
                                    // multi-byte glyph straddled byte 8000.
                                    let cap = s.floor_char_boundary(8000);
                                    format!(
                                        "{}\n\n[... diff truncated at 8000 chars ...]",
                                        &s[..cap]
                                    )
                                } else {
                                    s
                                }
                            })
                            .unwrap_or_default();
                        let prompt = format!(
                            "Generate a conventional commit message for these staged changes.\n\
                             Format: `type(scope): description`\n\
                             Types: feat / fix / docs / style / refactor / test / chore\n\
                             Rules: imperative mood, ≤72 chars subject, no trailing period.\n\
                             Output ONLY the commit message — no explanation, no markdown fences.\n\n\
                             ```\n{diff_str}\n```"
                        );
                        app.messages
                            .push(ChatMessage::assistant("Analyzing staged changes…".into()));
                        app.queued_prompts.push(crate::app::QueuedPrompt {
                            text: prompt,
                            is_meta: false,
                            priority: crate::app::QueuePriority::Later,
                            attachments: Vec::new(),
                        });
                        app.scroll_to_bottom();
                    }
                }
            }
        }
        "/review" => {
            // Ask the model to review current git changes for bugs, security
            // issues, and code quality problems with file:line specificity.
            app.messages.push(ChatMessage::user("/review".into()));
            let cwd = app.cwd.clone();
            // Prefer staged diff; fall back to HEAD diff; fall back to
            // working-tree diff so /review always finds something useful.
            let diff_output = {
                let staged = tokio::process::Command::new("git")
                    .args(["diff", "--cached"])
                    .current_dir(&cwd)
                    .output()
                    .await
                    .ok();
                let staged_str = staged
                    .as_ref()
                    .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_owned())
                    .unwrap_or_default();
                if !staged_str.is_empty() {
                    staged_str
                } else {
                    tokio::process::Command::new("git")
                        .args(["diff", "HEAD"])
                        .current_dir(&cwd)
                        .output()
                        .await
                        .ok()
                        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_owned())
                        .unwrap_or_default()
                }
            };
            if diff_output.is_empty() {
                app.messages.push(ChatMessage::assistant(
                    "No changes found (`git diff --cached` and `git diff HEAD` are both empty). \
                     Make some changes or stage files first."
                        .into(),
                ));
            } else {
                let capped = if diff_output.len() > 12_000 {
                    format!(
                        "{}\n\n[... diff truncated at 12000 chars ...]",
                        &diff_output[..12_000]
                    )
                } else {
                    diff_output
                };
                let prompt = format!(
                    "Review the following git diff for bugs, security issues, and code quality \
                     problems. Be specific — reference exact file names and line numbers where \
                     relevant. Organise findings by severity (Critical / High / Medium / Low). \
                     If there are no issues worth calling out, say so briefly.\n\n\
                     ```diff\n{capped}\n```"
                );
                app.messages
                    .push(ChatMessage::assistant("Reviewing changes…".into()));
                app.queued_prompts.push(crate::app::QueuedPrompt {
                    text: prompt,
                    is_meta: false,
                    priority: crate::app::QueuePriority::Later,
                    attachments: Vec::new(),
                });
                app.scroll_to_bottom();
            }
        }
        "/skills" => {
            let skills =
                crate::agents::load_skills(&std::env::current_dir().unwrap_or_else(|_| ".".into()));
            let body = if skills.is_empty() {
                "No skills found. Create `.claude/skills/<name>.md` files with \
                 optional YAML frontmatter (`name:`, `description:`) and a markdown \
                 body that becomes the system-prompt fragment."
                    .to_owned()
            } else {
                let mut s = format!("**{} skill(s) loaded:**\n\n", skills.len());
                for sk in &skills {
                    s.push_str(&format!(
                        "- **{}** — {}\n  source: `{}`\n",
                        sk.name,
                        sk.description.as_deref().unwrap_or("(no description)"),
                        sk.source.display()
                    ));
                }
                s
            };
            app.messages.push(ChatMessage::user("/skills".into()));
            app.messages.push(ChatMessage::assistant(body));
        }
        "/agents" => {
            let agents =
                crate::agents::load_agents(&std::env::current_dir().unwrap_or_else(|_| ".".into()));
            let body = if agents.is_empty() {
                "No agent definitions found. Create `.claude/agents/<name>.md` files \
                 with YAML frontmatter (`name:` required, plus optional `model`, \
                 `permissionMode`, `allowedTools`, `disallowedTools`, `skills`, \
                 `isolation`, `forksParentContext`) and a markdown body that becomes \
                 the system prompt for spawned subagents/teammates."
                    .to_owned()
            } else {
                let mut s = format!("**{} agent(s) loaded:**\n\n", agents.len());
                for a in &agents {
                    s.push_str(&format!(
                        "- **{}** — model: {}, permission: {:?}, isolation: {}\n  \
                         tools: allowed={:?}, denied={:?}\n  source: `{}`\n",
                        a.name,
                        a.model.as_deref().unwrap_or("inherit"),
                        a.permission_mode.unwrap_or_default(),
                        a.isolation.as_deref().unwrap_or("none"),
                        a.allowed_tools,
                        a.disallowed_tools,
                        a.source.display(),
                    ));
                }
                s
            };
            app.messages.push(ChatMessage::user("/agents".into()));
            app.messages.push(ChatMessage::assistant(body));
        }
        "/market" => {
            // Surface the agent-economy snapshot — same data the
            // `market_status` tool returns, but framed for the user
            // rather than the model. No bounty_id filter for now.
            let report_str = match crate::tools::market_report_string().await {
                Ok(s) => s,
                Err(e) => format!("Market unavailable: {e}"),
            };
            app.messages.push(ChatMessage::user("/market".into()));
            app.messages.push(ChatMessage::assistant(report_str));
        }
        "/cascade" => {
            // Filter the task store for cascade-tagged entries
            // produced by symbol_edit's `dispatch_cascade=true`. The
            // metadata.kind="cascade" tag is the signal we emit when
            // queuing them. Group by file (one Task ≈ one file) and
            // show status + caller list per group.
            let tasks = app.task_store.list(jfc_session::DeletedFilter::Exclude);
            let cascade: Vec<&jfc_session::Task> = tasks
                .iter()
                .filter(|t| {
                    t.metadata
                        .as_ref()
                        .and_then(|m| m.get("kind"))
                        .and_then(|k| k.as_str())
                        == Some("cascade")
                })
                .collect();
            let body = if cascade.is_empty() {
                "No cascade tasks. Cascade entries are queued by `symbol_edit` \
                 when called with `dispatch_cascade: true` and the edit changes \
                 a function signature with downstream callers."
                    .to_owned()
            } else {
                let mut s = format!(
                    "**{} cascade task{}** (from `symbol_edit dispatch_cascade=true`):\n\n",
                    cascade.len(),
                    if cascade.len() == 1 { "" } else { "s" }
                );
                for t in &cascade {
                    let status_marker = match t.status {
                        jfc_session::TaskStatus::Completed => "✓",
                        jfc_session::TaskStatus::InProgress => "⏵",
                        jfc_session::TaskStatus::Pending => "•",
                        jfc_session::TaskStatus::Failed => "✗",
                        jfc_session::TaskStatus::Deleted => "✗",
                    };
                    let file = t
                        .metadata
                        .as_ref()
                        .and_then(|m| m.get("file"))
                        .and_then(|f| f.as_str())
                        .unwrap_or("<unknown>");
                    let callers = t
                        .metadata
                        .as_ref()
                        .and_then(|m| m.get("callers"))
                        .and_then(|c| c.as_array())
                        .map(|arr| {
                            arr.iter()
                                .filter_map(|v| v.as_str())
                                .collect::<Vec<_>>()
                                .join(", ")
                        })
                        .unwrap_or_default();
                    s.push_str(&format!(
                        "{status_marker} `{}` — {}\n  callers: {callers}\n  → {}\n\n",
                        t.id, file, t.subject,
                    ));
                }
                s
            };
            app.messages.push(ChatMessage::user("/cascade".into()));
            app.messages.push(ChatMessage::assistant(body));
        }
        "/graph-history" => {
            let records = crate::tools::graph_history_snapshot();
            let body = if records.is_empty() {
                "No graph queries recorded yet. Run `graph_query` (via the model) or \
                 ask the model to query the code graph, then re-invoke `/graph-history` \
                 to see the most recent queries with their result counts."
                    .to_owned()
            } else {
                let mut s = format!(
                    "**{} graph quer{} recorded** (most recent first):\n\n",
                    records.len(),
                    if records.len() == 1 { "y" } else { "ies" }
                );
                for record in records.iter().rev().take(20) {
                    let trunc_marker = if record.was_truncated {
                        " [truncated]"
                    } else {
                        ""
                    };
                    let cycle_marker = if record.cycles_detected > 0 {
                        format!(
                            " [{} cycle{} detected]",
                            record.cycles_detected,
                            if record.cycles_detected == 1 { "" } else { "s" }
                        )
                    } else {
                        String::new()
                    };
                    s.push_str(&format!(
                        "- `{}`\n  → {} node{}{}{}\n",
                        record.query_text,
                        record.result_node_count,
                        if record.result_node_count == 1 {
                            ""
                        } else {
                            "s"
                        },
                        trunc_marker,
                        cycle_marker,
                    ));
                }
                s
            };
            app.messages
                .push(ChatMessage::user("/graph-history".into()));
            app.messages.push(ChatMessage::assistant(body));
        }
        "/task-list" | "/tasks" => {
            let tasks = app.task_store.list(jfc_session::DeletedFilter::Exclude);
            let body = if tasks.is_empty() {
                "No tasks. Use `/task-add <subject>` to create one.".to_owned()
            } else {
                let mut s = format!("**{} task(s):**\n\n", tasks.len());
                for t in &tasks {
                    let icon = match t.status {
                        jfc_session::TaskStatus::Pending => "□",
                        jfc_session::TaskStatus::InProgress => "▣",
                        jfc_session::TaskStatus::Completed => "✓",
                        jfc_session::TaskStatus::Failed => "✗",
                        jfc_session::TaskStatus::Deleted => "✗",
                    };
                    let owner = t
                        .owner
                        .as_deref()
                        .map(|o| format!(" (@{o})"))
                        .unwrap_or_default();
                    let blocks = if t.blocked_by.is_empty() {
                        String::new()
                    } else {
                        format!(
                            " · blocked by {}",
                            t.blocked_by
                                .iter()
                                .map(|id| id.as_str())
                                .collect::<Vec<_>>()
                                .join(",")
                        )
                    };
                    s.push_str(&format!(
                        "{} `{}` {}{}{}\n",
                        icon, t.id, t.subject, owner, blocks
                    ));
                }
                let c = app.task_store.counts();
                s.push_str(&format!(
                    "\n*{} pending, {} in progress, {} completed*",
                    c.pending, c.in_progress, c.completed
                ));
                s
            };
            app.messages.push(ChatMessage::user("/tasks".into()));
            app.messages.push(ChatMessage::assistant(body));
        }
        "/task-add" => {
            let subject = parts.get(1).copied().unwrap_or("").trim();
            if subject.is_empty() {
                app.messages.push(ChatMessage::assistant(
                    "Usage: `/task-add <subject>`".into(),
                ));
            } else {
                match app.task_store.create(
                    subject.to_owned(),
                    String::new(),
                    None,
                    Vec::<jfc_session::TaskId>::new(),
                ) {
                    Ok(t) => {
                        app.messages
                            .push(ChatMessage::user(format!("/task-add {subject}")));
                        app.messages.push(ChatMessage::assistant(format!(
                            "Created task `{}`: {}",
                            t.id, t.subject
                        )));
                    }
                    Err(e) => {
                        app.messages
                            .push(ChatMessage::assistant(format!("**Error:** {e}")));
                    }
                }
            }
        }
        "/task-done" => {
            let id = parts.get(1).copied().unwrap_or("").trim();
            if id.is_empty() {
                app.messages.push(ChatMessage::assistant(
                    "Usage: `/task-done <id>` (e.g. `/task-done t3`)".into(),
                ));
            } else {
                match app.task_store.update(
                    id,
                    jfc_session::TaskPatch {
                        status: Some(jfc_session::TaskStatus::Completed),
                        ..Default::default()
                    },
                ) {
                    Ok(t) => {
                        app.messages
                            .push(ChatMessage::user(format!("/task-done {id}")));
                        app.messages.push(ChatMessage::assistant(format!(
                            "✓ Completed `{}`: {}",
                            t.id, t.subject
                        )));
                    }
                    Err(e) => {
                        app.messages
                            .push(ChatMessage::assistant(format!("**Error:** {e}")));
                    }
                }
            }
        }
        "/task-rm" | "/task-delete" => {
            let id = parts.get(1).copied().unwrap_or("").trim();
            if id.is_empty() {
                app.messages
                    .push(ChatMessage::assistant("Usage: `/task-rm <id>`".into()));
            } else {
                match app.task_store.delete(id) {
                    Ok(()) => {
                        app.messages
                            .push(ChatMessage::user(format!("/task-rm {id}")));
                        app.messages
                            .push(ChatMessage::assistant(format!("Deleted task `{id}`.")));
                    }
                    Err(e) => {
                        app.messages
                            .push(ChatMessage::assistant(format!("**Error:** {e}")));
                    }
                }
            }
        }
        "/claude-md" => {
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
        "/mode" => {
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
        "/auto-mode" => {
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
        "/worktree" => {
            handle_worktree_command(app, parts.get(1).copied().unwrap_or("").trim()).await;
        }
        "/mcp" => {
            handle_mcp_command(app, parts.get(1).copied().unwrap_or("").trim()).await;
        }
        "/theme" => {
            handle_theme_command(app, parts.get(1).copied().unwrap_or("").trim());
        }
        "/fleet" | "/fleetview" => {
            handle_fleet_command(app);
        }
        "/teleport" => {
            handle_teleport_command(app, parts.get(1).copied().unwrap_or("").trim()).await;
        }
        "/init" => {
            handle_init_command(app).await;
        }
        "/plan" => {
            handle_doc_command(app, crate::document_formats::DocKind::Plan, tx).await;
        }
        "/roadmap" => {
            handle_doc_command(app, crate::document_formats::DocKind::Roadmap, tx).await;
        }
        "/parity" => {
            handle_doc_command(app, crate::document_formats::DocKind::Parity, tx).await;
        }
        "/philosophy" => {
            handle_doc_command(app, crate::document_formats::DocKind::Philosophy, tx).await;
        }
        "/usage" => {
            handle_doc_command(app, crate::document_formats::DocKind::Usage, tx).await;
        }
        "/cost" | "/stats" => {
            handle_cost_command(app);
        }
        "/status" => {
            handle_status_command(app);
        }
        "/bug" => {
            handle_bug_command(app, parts.get(1..).map(|r| r.join(" ")).unwrap_or_default());
        }
        "/rewind" => {
            handle_rewind_command(app, parts.get(1).copied().unwrap_or("").trim());
        }
        "/output-style" | "/style" | "/brief" => {
            // `/brief` is shorthand for `/output-style brief`. v132
            // exposes the same alias via `tengu_brief_mode_toggled`.
            let alias_brief = parts[0] == "/brief";
            let arg = if alias_brief {
                "brief".to_string()
            } else {
                parts.get(1).copied().unwrap_or("").trim().to_string()
            };
            handle_output_style_command(app, &arg);
        }
        "/dump-context" | "/debug-context" => {
            handle_dump_context_command(app).await;
        }
        "/install-github-app" => {
            handle_install_github_app(app).await;
        }
        "/pr" => {
            handle_pr_view(app, parts.get(1).copied().unwrap_or("").trim()).await;
        }
        "/pr-autofix" => {
            handle_pr_autofix(app, parts.get(1).copied().unwrap_or("").trim(), tx).await;
        }
        "/setup-github-actions" => {
            handle_setup_github_actions(app, parts.get(1).copied().unwrap_or("").trim()).await;
        }
        "/dream" | "/learn" => {
            handle_dream_command(app, parts.get(1).copied().unwrap_or("").trim(), tx).await;
        }
        "/loop" | "/proactive" => {
            handle_loop_command(app, parts.get(1).copied().unwrap_or("").trim(), tx).await;
        }
        "/schedule" | "/routines" => {
            handle_schedule_command(app, parts.get(1).copied().unwrap_or("").trim(), tx).await;
        }
        "/swarm-approve" | "/swarm-deny" => {
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
        _ => {
            // Skill-name fallthrough: `/<skill>` invokes the matching skill
            // body as if the user had pasted it. Mirrors v126 cli.js:226634
            // where slash-name-not-otherwise-bound resolves to a skill or
            // markdown command and either inline-expands or forks a subagent.
            //
            // TODO Phase B: if `frontmatter.context == "fork"` (or the v126
            // equivalent flag), spawn a Task subagent here instead of inline
            // expansion. Schema: cli.js:178962.
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
                app.agentic_turn_count = 0;
                app.thinking_started_at = None;
                app.pre_dispatched_tool_ids.clear();
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
                let messages =
                    crate::stream::build_provider_messages(&app.messages[..assistant_idx]);
                let model = app.model.clone();
                let tx_stream = tx.clone();
                let interrupt = app.interrupt_flag.clone();
                interrupt.store(false, std::sync::atomic::Ordering::SeqCst);
                app.cancel_token = tokio_util::sync::CancellationToken::new();
                let cancel = app.cancel_token.clone();
                // wg-async: retry path mints a fresh cancel token for the
                // new stream so the old (possibly cancelled) one can't
                // racially interrupt the retry.
                tokio::spawn(async move {
                    crate::stream::stream_response(
                        provider,
                        messages,
                        model,
                        tx_stream,
                        interrupt,
                        cancel,
                        None,
                        crate::runtime::StreamRequestOverrides::default(),
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
    }
}
