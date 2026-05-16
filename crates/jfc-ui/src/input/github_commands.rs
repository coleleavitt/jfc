use tokio::sync::mpsc;

use crate::{app::App, runtime::AppEvent, types::ChatMessage};

fn push_gh_unavailable(app: &mut App, cmd: &str) {
    let msg = "GitHub CLI not found on PATH. Install via <https://cli.github.com> \
               or set `JFC_GH_BIN_OVERRIDE` to a `gh` binary path."
        .to_owned();
    crate::toast::push_with_cap(
        &mut app.toasts,
        crate::toast::Toast::new(crate::toast::ToastKind::Error, "gh not installed"),
    );
    app.messages.push(ChatMessage::user(cmd.to_owned()));
    app.messages.push(ChatMessage::assistant(msg));
}

pub(super) async fn handle_install_github_app(app: &mut App) {
    if !crate::github::is_gh_installed() {
        push_gh_unavailable(app, "/install-github-app");
        return;
    }
    let Some(ctx) = crate::github::current_repo().await else {
        app.messages
            .push(ChatMessage::user("/install-github-app".into()));
        app.messages.push(ChatMessage::assistant(
            "Could not determine GitHub repo from `git remote get-url origin`. \
             Run this command from inside a checkout whose `origin` points at GitHub."
                .into(),
        ));
        return;
    };
    let url = crate::github::install::install_url(&ctx);

    let client = crate::github::GhClient::new();
    let already = crate::github::install::check_installed(&client, &ctx).await;
    let body = match already {
        Ok(Some(value)) => {
            let id = value.get("id").and_then(|number| number.as_u64());
            crate::github::install::already_installed_message(&ctx, id)
        }
        Ok(None) | Err(crate::github::client::GhError::NotAuthenticated) => {
            if let Err(error) = crate::github::install::open_browser(&url).await {
                tracing::warn!(target: "jfc::github", err = %error, "failed to open browser");
            }
            crate::github::install::install_message(&ctx, &url)
        }
        Err(error) => format!("**Error checking install state:** {error}"),
    };
    app.messages
        .push(ChatMessage::user("/install-github-app".into()));
    app.messages.push(ChatMessage::assistant(body));
}

fn parse_pr_num(arg: &str, cmd: &str) -> Result<u64, String> {
    let trimmed = arg.trim().trim_start_matches('#');
    if trimmed.is_empty() {
        return Err(format!("Usage: `{cmd} <pr-number>` (e.g. `{cmd} 42`)"));
    }
    trimmed
        .parse::<u64>()
        .map_err(|_| format!("`{trimmed}` is not a valid PR number."))
}

pub(super) async fn handle_pr_view(app: &mut App, arg: &str) {
    if !crate::github::is_gh_installed() {
        push_gh_unavailable(app, &format!("/pr {arg}"));
        return;
    }
    let cmd = format!("/pr {arg}");
    let number = match parse_pr_num(arg, "/pr") {
        Ok(number) => number,
        Err(error) => {
            app.messages.push(ChatMessage::user(cmd));
            app.messages.push(ChatMessage::assistant(error));
            return;
        }
    };
    let client = crate::github::GhClient::new();
    let body = match client.gh_pr_view(number).await {
        Ok(pr) => {
            let mut text = format!(
                "**PR #{n}** ({state}) - {title}\n\
                 Author: @{author}  .  {head} -> {base}\n\
                 URL: <{url}>\n",
                n = pr.number,
                state = pr.state,
                title = pr.title,
                author = pr.author.login,
                head = pr.head_ref_name,
                base = pr.base_ref_name,
                url = pr.url,
            );
            if !pr.body.trim().is_empty() {
                text.push_str("\n## Description\n\n");
                text.push_str(pr.body.trim());
                text.push('\n');
            }
            if !pr.comments.is_empty() {
                text.push_str(&format!("\n## Issue comments ({})\n\n", pr.comments.len()));
                for comment in &pr.comments {
                    text.push_str(&format!(
                        "- **@{}**: {}\n",
                        comment.author.login,
                        comment.body.lines().next().unwrap_or(""),
                    ));
                }
            }
            let review_total: usize = pr.reviews.iter().map(|review| review.comments.len()).sum();
            if !pr.reviews.is_empty() {
                text.push_str(&format!(
                    "\n## Reviews ({}, {} inline comment{})\n\n",
                    pr.reviews.len(),
                    review_total,
                    if review_total == 1 { "" } else { "s" }
                ));
                for review in &pr.reviews {
                    text.push_str(&format!(
                        "- @{} ({}): {}\n",
                        review.author.login,
                        if review.state.is_empty() {
                            "COMMENTED"
                        } else {
                            &review.state
                        },
                        review.body.lines().next().unwrap_or("")
                    ));
                }
            }
            text.push_str(
                "\n_Tip: run `/pr-autofix <num>` to ask the model to address review comments._",
            );
            text
        }
        Err(crate::github::client::GhError::NotAuthenticated) => {
            "`gh` is not authenticated - run `gh auth login` and try again.".into()
        }
        Err(crate::github::client::GhError::RateLimited { reminder }) => {
            format!("**GitHub API rate limit hit.**\n\n{reminder}")
        }
        Err(error) => format!("**Error:** {error}"),
    };
    app.messages.push(ChatMessage::user(cmd));
    app.messages.push(ChatMessage::assistant(body));
}

pub(super) async fn handle_pr_autofix(
    app: &mut App,
    arg: &str,
    tx: Option<&mpsc::Sender<AppEvent>>,
) {
    if !crate::github::is_gh_installed() {
        push_gh_unavailable(app, &format!("/pr-autofix {arg}"));
        return;
    }
    let cmd = format!("/pr-autofix {arg}");
    let number = match parse_pr_num(arg, "/pr-autofix") {
        Ok(number) => number,
        Err(error) => {
            app.messages.push(ChatMessage::user(cmd));
            app.messages.push(ChatMessage::assistant(error));
            return;
        }
    };
    let client = crate::github::GhClient::new();
    let prompt = match crate::github::autofix::run(&client, number).await {
        Ok(prompt) => prompt,
        Err(crate::github::client::GhError::NotAuthenticated) => {
            app.messages.push(ChatMessage::user(cmd));
            app.messages.push(ChatMessage::assistant(
                "`gh` is not authenticated - run `gh auth login` and try again.".into(),
            ));
            return;
        }
        Err(crate::github::client::GhError::RateLimited { reminder }) => {
            app.messages.push(ChatMessage::user(cmd));
            app.messages.push(ChatMessage::assistant(format!(
                "Rate limited.\n\n{reminder}"
            )));
            return;
        }
        Err(error) => {
            app.messages.push(ChatMessage::user(cmd));
            app.messages
                .push(ChatMessage::assistant(format!("**Error:** {error}")));
            return;
        }
    };

    app.messages.push(ChatMessage::user(cmd));

    let Some(tx) = tx else {
        app.messages.push(ChatMessage::assistant(format!(
            "Autofix prompt prepared (no stream channel - submit `/pr-autofix {number}` from the input bar to drive the model):\n\n{prompt}"
        )));
        return;
    };

    let assistant_idx = app.messages.len() + 1;
    app.messages.push(ChatMessage::user(prompt));
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
    app.thinking_started_at = None;
    app.thinking_ended_at = None;
    app.last_usage_output = 0;
    app.usage_apply_baseline = (0, 0, 0, 0);
    app.scroll_to_bottom();

    let session_id = app
        .current_session_id
        .clone()
        .unwrap_or_else(jfc_session::generate_session_id);
    {
        let session_id = session_id.clone();
        let messages = app.messages.clone();
        let model = app.model.clone();
        tokio::spawn(async move {
            crate::session::save_session(&session_id, &messages, None, Some(model.as_str())).await;
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
    tokio::spawn(async move {
        crate::stream::stream_response(provider, messages, model, tx_stream, interrupt, cancel, None)
            .await;
    });
}

pub(super) async fn handle_setup_github_actions(app: &mut App, arg: &str) {
    let force = matches!(arg, "force" | "--force" | "-f" | "overwrite");
    let echo = if force {
        "/setup-github-actions force".to_owned()
    } else {
        "/setup-github-actions".to_owned()
    };
    let repo_root = std::path::PathBuf::from(&app.cwd);
    let body = match crate::github::actions::write_workflow(&repo_root, force) {
        Ok(outcome) => crate::github::actions::success_message(&outcome),
        Err(error) => format!("**Error writing workflow:** {error}"),
    };
    app.messages.push(ChatMessage::user(echo));
    app.messages.push(ChatMessage::assistant(body));
}
