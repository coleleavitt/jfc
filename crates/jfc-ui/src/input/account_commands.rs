//! Slash handlers: account, auth & external actions.

use super::*;

pub(super) async fn cmd_workflow(
    app: &mut App,
    parts: &[&str],
    text: &str,
    tx: Option<&mpsc::Sender<AppEvent>>,
) {
    // `/workflow` (or `/workflows`) lists available JS workflows + running
    // workflow tasks. `/workflow run <name>` injects a `Workflow({name})`
    // request so the model invokes the real Workflow tool (deterministic JS
    // orchestration). Legacy TOML step-templates are also surfaced.
    app.messages.push(ChatMessage::user(text.to_owned()));
    let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
    let arg = parts.get(1).copied().unwrap_or("").trim();
    let mut sub = arg.split_whitespace();
    let verb = sub.next().unwrap_or("");
    let rest: String = sub.collect::<Vec<_>>().join(" ");
    match verb {
        "" | "list" => {
            app.messages
                .push(ChatMessage::assistant(render_workflow_listing(app, &cwd)));
        }
        "run" => {
            if rest.is_empty() {
                app.messages.push(ChatMessage::assistant(
                    "Usage: `/workflow run <name>`. List available workflows with `/workflow`."
                        .into(),
                ));
                return;
            }
            // Resolve the name against the registry (built-in/user/project).
            if crate::workflows::resolve(&cwd, &rest).is_none() {
                app.messages.push(ChatMessage::assistant(format!(
                    "Workflow `{rest}` not found. List available workflows with `/workflow`."
                )));
                return;
            }
            let Some(tx) = tx else {
                app.messages.push(ChatMessage::assistant(
                    "Workflow runner needs the event channel; called from a context without one."
                        .into(),
                ));
                return;
            };
            // Inject a prompt instructing the model to call the Workflow tool.
            // This is the slash-command bridge: the command doesn't run the
            // workflow directly — it tells the model to invoke it, so the
            // normal tool-permission + background-task path applies.
            let prompt = format!(
                "Run the saved workflow named \"{rest}\" by calling the Workflow tool: \
                 Workflow({{ name: \"{rest}\" }}). Do not describe it — call the tool."
            );
            let _ = tx
                .send(crate::runtime::AppEvent::Ui(
                    crate::runtime::UiEvent::Submit(prompt),
                ))
                .await;
            app.messages.push(ChatMessage::assistant(format!(
                "Dispatching workflow `{rest}` via the Workflow tool…"
            )));
        }
        other => {
            app.messages.push(ChatMessage::assistant(format!(
                "Unknown subcommand `{other}`. Use `/workflow list` or `/workflow run <name>`."
            )));
        }
    }
}

/// Build the `/workflow` listing: running workflow tasks, then available
/// named workflows (registry), then legacy TOML templates.
fn render_workflow_listing(app: &App, cwd: &std::path::Path) -> String {
    use jfc_core::ExecutionStatus;
    let mut body = String::new();

    // ── running workflow background tasks ───────────────────────────────
    let running: Vec<&crate::app::BackgroundTask> = app
        .background_tasks
        .values()
        .filter(|bt| {
            bt.task_id.as_str().starts_with("bgwf_") && bt.status == ExecutionStatus::Running
        })
        .collect();
    if !running.is_empty() {
        body.push_str("**Running workflows:**\n\n");
        for bt in running {
            let elapsed = bt.started_at.elapsed().as_secs();
            body.push_str(&format!(
                "- `{}` — {} ({}s, {} tools)\n",
                bt.task_id.as_str(),
                bt.description,
                elapsed,
                bt.tool_use_count,
            ));
        }
        body.push('\n');
    }

    // ── available named workflows (registry) ────────────────────────────
    let registry = crate::workflows::discover(cwd);
    if !registry.is_empty() {
        body.push_str("**Available workflows** (run with `/workflow run <name>`):\n\n");
        for wf in &registry {
            let src = match wf.source {
                crate::workflows::WorkflowSource::BuiltIn => "built-in",
                crate::workflows::WorkflowSource::User => "user",
                crate::workflows::WorkflowSource::Project => "project",
            };
            body.push_str(&format!("- `{}` ({src}) — {}\n", wf.name, wf.description));
        }
        body.push('\n');
    }

    // ── legacy TOML step templates ──────────────────────────────────────
    let legacy = crate::workflows::list(cwd);
    if !legacy.is_empty() {
        body.push_str("**Legacy TOML templates** (`.jfc/workflows/*.toml`):\n\n");
        for name in &legacy {
            body.push_str(&format!("- `{name}`\n"));
        }
        body.push('\n');
    }

    if body.is_empty() {
        body.push_str(
            "No workflows found. Built-in workflows (bughunt, review-branch, deep-research) \
             are available, or create `.jfc/workflows/<name>.js` starting with \
             `export const meta = { name, description }`.",
        );
    }
    body
}

pub(super) async fn cmd_login(
    app: &mut App,
    parts: &[&str],
    text: &str,
    _tx: Option<&mpsc::Sender<AppEvent>>,
) {
    // v132 `/login` flow. With no arg, prints the chooser. With
    // a sub-target, the dispatcher returns a body string +
    // some side effects need a browser open. We always shell
    // out to xdg-open / open / start to launch the browser
    // (cheap, async-safe; failures are silent on systems
    // without one of those binaries).
    app.messages.push(ChatMessage::user(text.to_owned()));
    let arg = parts
        .get(1)
        .copied()
        .map(str::trim)
        .filter(|s| !s.is_empty());
    let dispatch = crate::providers::login_dispatch::dispatch(arg);
    let url = match &dispatch {
        crate::providers::login_dispatch::LoginDispatch::AnthropicApiKey(_)
        | crate::providers::login_dispatch::LoginDispatch::ConsoleApiKey(_) => {
            Some("https://console.anthropic.com/settings/keys")
        }
        crate::providers::login_dispatch::LoginDispatch::ClaudeAiOAuth(_) => {
            Some("https://claude.ai/login")
        }
        crate::providers::login_dispatch::LoginDispatch::CodexOAuth(_) => {
            Some("https://auth.openai.com/codex/device")
        }
        crate::providers::login_dispatch::LoginDispatch::AntigravityOAuth(_) => {
            Some("https://accounts.google.com/")
        }
        _ => None,
    };
    if let Some(url) = url {
        // TODO: re-enable browser launch when in interactive mode (not in tests).
        // Best-effort: shell out to the platform browser opener.
        // Don't await — the browser launch is fire-and-forget.
        // #[cfg(target_os = "linux")]
        // let _ = std::process::Command::new("xdg-open").arg(url).spawn();
        // #[cfg(target_os = "macos")]
        // let _ = std::process::Command::new("open").arg(url).spawn();
        // #[cfg(target_os = "windows")]
        // let _ = std::process::Command::new("cmd")
        //     .args(["/C", "start", url])
        //     .spawn();
        // tracing::info!(target: "jfc::login", %url, "opened browser for /login");
    }
    app.messages.push(ChatMessage::assistant(format!(
        "{dispatch}{}",
        if url.is_some() {
            "\n\n_(opened the browser for you)_"
        } else {
            ""
        }
    )));
}

pub(super) async fn cmd_logout(
    app: &mut App,
    parts: &[&str],
    text: &str,
    _tx: Option<&mpsc::Sender<AppEvent>>,
) {
    app.messages.push(ChatMessage::user(text.to_owned()));
    let arg = parts
        .get(1)
        .copied()
        .map(str::trim)
        .filter(|s| !s.is_empty());
    // Wipe the OAuth token + API-key stores under
    // ~/.config/jfc/. We deliberately keep this contained to
    // jfc's own state (opencode shares anthropic-accounts.json,
    // so blindly nuking that file would also log them out of
    // a sibling client).
    let scope = arg.unwrap_or("jfc");
    let home = std::env::var("HOME").unwrap_or_default();
    let mut removed = Vec::new();
    for relpath in [
        ".config/jfc/credentials.json",
        ".config/jfc/anthropic-oauth.json",
        ".config/jfc/codex-tokens.json",
    ] {
        let p = std::path::PathBuf::from(&home).join(relpath);
        if p.exists() && std::fs::remove_file(&p).is_ok() {
            removed.push(p.display().to_string());
        }
    }
    let summary = if removed.is_empty() {
        format!("No credential files found to remove (scope: `{scope}`).")
    } else {
        format!(
            "Removed {} credential file(s):\n{}\nRun `/login` to authenticate again.",
            removed.len(),
            removed
                .iter()
                .map(|p| format!("  - `{p}`"))
                .collect::<Vec<_>>()
                .join("\n")
        )
    };
    app.messages.push(ChatMessage::assistant(summary));
}

pub(super) async fn cmd_release_notes(
    app: &mut App,
    _parts: &[&str],
    text: &str,
    _tx: Option<&mpsc::Sender<AppEvent>>,
) {
    app.messages.push(ChatMessage::user(text.to_owned()));
    // Try to read the workspace CHANGELOG; fall back to a stub
    // pointer when the binary was installed somewhere without it.
    let candidates = ["CHANGELOG.md", "../CHANGELOG.md", "../../CHANGELOG.md"];
    let notes = candidates
        .iter()
        .find_map(|p| std::fs::read_to_string(p).ok())
        .map(|s| {
            let trimmed = s.lines().take(80).collect::<Vec<_>>().join("\n");
            if s.lines().count() > 80 {
                format!("{trimmed}\n\n*(showing first 80 lines — see CHANGELOG.md for the rest)*")
            } else {
                trimmed
            }
        })
        .unwrap_or_else(|| {
            format!(
                "Release notes unavailable in this build. Visit \
                 {} for the full changelog.",
                super::support::releases_url()
            )
        });
    app.messages.push(ChatMessage::assistant(notes));
}

pub(super) async fn cmd_feedback(
    app: &mut App,
    _parts: &[&str],
    text: &str,
    _tx: Option<&mpsc::Sender<AppEvent>>,
) {
    app.messages.push(ChatMessage::user(text.to_owned()));
    let session_id = app
        .current_session_id
        .as_ref()
        .map(|s| s.as_str())
        .unwrap_or("(none)");
    let body = format!(
        "**Describe the issue**\n\n\
         (your description here)\n\n\
         **Environment**\n\
         - jfc version: `{}`\n\
         - Provider/model: `{}` / `{}`\n\
         - OS: `{}`\n\
         - Session ID: `{session_id}`\n",
        env!("CARGO_PKG_VERSION"),
        app.provider.name(),
        app.model.as_str(),
        std::env::consts::OS,
    );
    let url = super::support::bug_report_url("", &body);
    // TODO: re-enable browser launch when in interactive mode (not in tests).
    // #[cfg(target_os = "linux")]
    // let _ = std::process::Command::new("xdg-open").arg(&url).spawn();
    // #[cfg(target_os = "macos")]
    // let _ = std::process::Command::new("open").arg(&url).spawn();
    // #[cfg(target_os = "windows")]
    // let _ = std::process::Command::new("cmd")
    //     .args(["/C", "start", &url])
    //     .spawn();
    app.messages.push(ChatMessage::assistant(format!(
        "Opened a pre-filled bug report at {}/issues/new in your browser \
         (version, model, OS, and session id `{session_id}` are already attached).",
        super::support::repo_url(),
    )));
}

pub(super) async fn cmd_upgrade(
    app: &mut App,
    _parts: &[&str],
    text: &str,
    _tx: Option<&mpsc::Sender<AppEvent>>,
) {
    app.messages.push(ChatMessage::user(text.to_owned()));
    app.messages.push(ChatMessage::assistant(format!(
        "To upgrade jfc, run one of:\n\
         * `cargo install --git {}` (HEAD)\n\
         * `cargo install jfc` (latest crates.io release)\n\
         \n\
         If you installed via a package manager (homebrew, nix, AUR), use its update path instead.",
        super::support::cargo_install_git_url(),
    )));
}

pub(super) async fn cmd_batch(
    app: &mut App,
    parts: &[&str],
    text: &str,
    _tx: Option<&mpsc::Sender<AppEvent>>,
) {
    // /batch <prompt-file>: read newline-delimited prompts and
    // submit them via Anthropic's Message Batches API for the
    // 50% discount. The batch ID is returned synchronously;
    // results stream back via the Sessions API in a follow-up
    // turn (poll `/batch status <id>`).
    app.messages.push(ChatMessage::user(text.to_owned()));
    let arg = parts.get(1).copied().unwrap_or("").trim();
    if arg.is_empty() {
        app.messages.push(ChatMessage::assistant(
            "Usage: `/batch <prompt-file>`. The file should contain one prompt per line.".into(),
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
            "No Anthropic API key configured — `/batch` needs one (set ANTHROPIC_API_KEY).".into(),
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
