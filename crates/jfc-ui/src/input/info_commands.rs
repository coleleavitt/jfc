//! Slash handlers: inspection, diagnostics & VCS review.

use super::*;

pub(super) async fn cmd_diff(
    app: &mut App,
    _parts: &[&str],
    text: &str,
    _tx: Option<&mpsc::Sender<AppEvent>>,
) {
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

pub(super) async fn cmd_timeline(
    app: &mut App,
    _parts: &[&str],
    text: &str,
    _tx: Option<&mpsc::Sender<AppEvent>>,
) {
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

pub(super) async fn cmd_doctor(
    app: &mut App,
    _parts: &[&str],
    text: &str,
    _tx: Option<&mpsc::Sender<AppEvent>>,
) {
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

pub(super) async fn cmd_help(
    app: &mut App,
    _parts: &[&str],
    _text: &str,
    _tx: Option<&mpsc::Sender<AppEvent>>,
) {
    // Also flip the visual overlay so users get the same
    // keybindings table they'd see from `?`. The text dump
    // below is kept for searchability + transcript export.
    app.show_help = true;
    app.messages.push(ChatMessage::user("/help".into()));

    // Command list is generated from the SLASH_COMMANDS registry table — the
    // same single source of truth that drives dispatch and autocomplete — so
    // /help can never list a command that doesn't exist (or miss one). Each
    // alias collapses onto its canonical row's help text, so we de-dup by
    // help string to avoid printing the same description once per alias.
    let mut body = String::from("**Available commands:**\n");
    let mut seen_help: std::collections::HashSet<&str> = std::collections::HashSet::new();
    for (name, help) in crate::input::SLASH_COMMANDS {
        if seen_help.insert(help) {
            body.push_str(&format!("- `{name}` — {help}\n"));
        }
    }
    body.push_str(
        "\n\
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
         - `JFC_ADVISOR_ENABLED=1` — enable the `/advisor` parallel-advice slash command",
    );
    app.messages.push(ChatMessage::assistant(body));
}

pub(super) async fn cmd_commit(
    app: &mut App,
    _parts: &[&str],
    _text: &str,
    _tx: Option<&mpsc::Sender<AppEvent>>,
) {
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
                    "Nothing staged. Stage changes first with `git add <file>` or `git add -p`."
                        .into(),
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
                            format!("{}\n\n[... diff truncated at 8000 chars ...]", &s[..cap])
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

pub(super) async fn cmd_review(
    app: &mut App,
    _parts: &[&str],
    _text: &str,
    _tx: Option<&mpsc::Sender<AppEvent>>,
) {
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

pub(super) async fn cmd_skills(
    app: &mut App,
    _parts: &[&str],
    _text: &str,
    _tx: Option<&mpsc::Sender<AppEvent>>,
) {
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

pub(super) async fn cmd_agents(
    app: &mut App,
    _parts: &[&str],
    _text: &str,
    _tx: Option<&mpsc::Sender<AppEvent>>,
) {
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

pub(super) async fn cmd_market(
    app: &mut App,
    _parts: &[&str],
    _text: &str,
    _tx: Option<&mpsc::Sender<AppEvent>>,
) {
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

pub(super) async fn cmd_cascade(
    app: &mut App,
    _parts: &[&str],
    _text: &str,
    _tx: Option<&mpsc::Sender<AppEvent>>,
) {
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

pub(super) async fn cmd_graph_history(
    app: &mut App,
    _parts: &[&str],
    _text: &str,
    _tx: Option<&mpsc::Sender<AppEvent>>,
) {
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
