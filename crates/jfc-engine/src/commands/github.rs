use tokio::sync::mpsc;

use crate::app::EngineState;
use crate::runtime::EngineEvent;
use jfc_core::ChatMessage;

fn push_gh_unavailable(state: &mut EngineState, cmd: &str) {
    let msg = "GitHub CLI not found on PATH. Install via <https://cli.github.com> \
               or set `JFC_GH_BIN_OVERRIDE` to a `gh` binary path."
        .to_owned();
    crate::toast::push_with_cap(
        &mut state.toasts,
        crate::toast::Toast::new(
            crate::toast::ToastKind::Error,
            "GitHub CLI (gh) not installed",
        ),
    );
    state.messages.push(ChatMessage::user(cmd.to_owned()));
    state.messages.push(ChatMessage::assistant(msg));
}

pub(super) async fn handle_install_github_app(state: &mut EngineState) {
    if !crate::github::is_gh_installed() {
        push_gh_unavailable(state, "/install-github-app");
        return;
    }
    let Some(ctx) = crate::github::current_repo().await else {
        state
            .messages
            .push(ChatMessage::user("/install-github-app".into()));
        state.messages.push(ChatMessage::assistant(
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
    state
        .messages
        .push(ChatMessage::user("/install-github-app".into()));
    state.messages.push(ChatMessage::assistant(body));
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

pub(super) async fn handle_pr_view(state: &mut EngineState, arg: &str) {
    if !crate::github::is_gh_installed() {
        push_gh_unavailable(state, &format!("/pr {arg}"));
        return;
    }
    let cmd = format!("/pr {arg}");
    let number = match parse_pr_num(arg, "/pr") {
        Ok(number) => number,
        Err(error) => {
            state.messages.push(ChatMessage::user(cmd));
            state.messages.push(ChatMessage::assistant(error));
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
    state.messages.push(ChatMessage::user(cmd));
    state.messages.push(ChatMessage::assistant(body));
}

pub(super) async fn handle_pr_autofix(
    state: &mut EngineState,
    arg: &str,
    tx: Option<&mpsc::Sender<EngineEvent>>,
) {
    if !crate::github::is_gh_installed() {
        push_gh_unavailable(state, &format!("/pr-autofix {arg}"));
        return;
    }
    let cmd = format!("/pr-autofix {arg}");
    let number = match parse_pr_num(arg, "/pr-autofix") {
        Ok(number) => number,
        Err(error) => {
            state.messages.push(ChatMessage::user(cmd));
            state.messages.push(ChatMessage::assistant(error));
            return;
        }
    };
    let client = crate::github::GhClient::new();
    let prompt = match crate::github::autofix::run(&client, number).await {
        Ok(prompt) => prompt,
        Err(crate::github::client::GhError::NotAuthenticated) => {
            state.messages.push(ChatMessage::user(cmd));
            state.messages.push(ChatMessage::assistant(
                "`gh` is not authenticated - run `gh auth login` and try again.".into(),
            ));
            return;
        }
        Err(crate::github::client::GhError::RateLimited { reminder }) => {
            state.messages.push(ChatMessage::user(cmd));
            state.messages.push(ChatMessage::assistant(format!(
                "Rate limited.\n\n{reminder}"
            )));
            return;
        }
        Err(error) => {
            state.messages.push(ChatMessage::user(cmd));
            state
                .messages
                .push(ChatMessage::assistant(format!("**Error:** {error}")));
            return;
        }
    };

    state.messages.push(ChatMessage::user(cmd));

    let Some(tx) = tx else {
        state.messages.push(ChatMessage::assistant(format!(
            "Autofix prompt prepared (no stream channel - submit `/pr-autofix {number}` from the input bar to drive the model):\n\n{prompt}"
        )));
        return;
    };

    if crate::runtime::ops::refuse_budget_cap_if_reached(state) {
        return;
    }

    let assistant_idx = state.messages.len() + 1;
    state.messages.push(ChatMessage::user(prompt));
    state.tool_ctx.total_user_turns += 1;
    state.messages.push(ChatMessage::assistant(String::new()));
    let identity = crate::cache_lineage::current_identity(state);
    crate::cache_lineage::stamp_assistant(&mut state.messages, assistant_idx, &identity);
    state.streaming_text.clear();
    state.streaming_reasoning.clear();
    state.streaming_response_bytes = 0;
    state.streaming_response_baseline = 0;
    state.streaming_thinking_tokens = 0;
    state.token_rate_samples.clear();
    state.token_rate_sample_thinking = None;
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
    state.thinking_started_at = None;
    state.thinking_ended_at = None;
    state.last_usage_output = 0;
    state.usage_apply_baseline = (0, 0, 0, 0);
    state.push_effect(crate::app::EngineEffect::ScrollToBottom);

    let session_id = state
        .current_session_id
        .clone()
        .unwrap_or_else(jfc_session::generate_session_id);
    {
        let session_id = session_id.clone();
        let messages = state.messages.clone();
        let model = state.model.clone();
        tokio::spawn(async move {
            crate::session::save_session(&session_id, &messages, None, Some(model.as_str())).await;
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
        provider_history_archive_seen: state.provider_history_archive_seen(),
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

pub(super) async fn handle_setup_github_actions(state: &mut EngineState, arg: &str) {
    let force = matches!(arg, "force" | "--force" | "-f" | "overwrite");
    let echo = if force {
        "/setup-github-actions force".to_owned()
    } else {
        "/setup-github-actions".to_owned()
    };
    let repo_root = std::path::PathBuf::from(&state.cwd);
    let body = match crate::github::actions::write_workflow(&repo_root, force) {
        Ok(outcome) => crate::github::actions::success_message(&outcome),
        Err(error) => format!("**Error writing workflow:** {error}"),
    };
    state.messages.push(ChatMessage::user(echo));
    state.messages.push(ChatMessage::assistant(body));
}
