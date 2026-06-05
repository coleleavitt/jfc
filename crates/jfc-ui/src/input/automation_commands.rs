use tokio::sync::mpsc;

use crate::app::App;
use crate::runtime::AppEvent;
use crate::types::ChatMessage;

// /dream — memory consolidation
// ---------------------------------------------------------------------------

/// `/dream [nightly]` — inject a user message asking the model to review the
/// session and consolidate key learnings into typed memory files.
///
/// With `nightly` as the argument, also instructs the model to schedule itself
/// via `CronCreate` so consolidation runs automatically every night at 02:00.
pub(super) async fn handle_dream_command(
    app: &mut App,
    arg: &str,
    tx: Option<&mpsc::Sender<AppEvent>>,
) {
    let nightly = arg.trim().eq_ignore_ascii_case("nightly");
    let echo = if nightly {
        "/dream nightly".to_owned()
    } else {
        "/dream".to_owned()
    };
    app.messages.push(ChatMessage::user(echo));

    let cron_instruction = if nightly {
        "\n\nAlso use the CronCreate tool to schedule this same /dream command to run \
nightly at 2 AM:\n- schedule: \"0 2 * * *\"\n- command: \"dream consolidate\"\n\
- description: \"Nightly memory consolidation\""
    } else {
        ""
    };

    let prompt = format!(
        "# Memory Consolidation (/dream)\n\n\
Review this session's conversation and your memory files in ~/.config/jfc/memory/.\n\
1. Identify key learnings, patterns, and facts worth preserving\n\
2. Create or update typed memory files: context/, preference/, project/, feedback/\n\
3. Prune outdated or redundant entries\n\
4. Summarize what you consolidated\n\n\
Use the MemoryCreate tool for new memories and MemoryDelete for stale ones.{cron_instruction}"
    );

    let Some(tx) = tx else {
        app.messages.push(ChatMessage::assistant(
            "Running memory consolidation…\n\n\
*(no stream channel — submit `/dream` from the input bar to drive the model)*"
                .into(),
        ));
        app.scroll_to_bottom();
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
    app.turn_start_cost = crate::cost::total_cost(&app.usage_by_model);
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
    tokio::spawn(async move {
        crate::stream::stream_response(
            provider, messages, model, tx_stream, interrupt, cancel, None, overrides,
        )
        .await;
    });
}

// ---------------------------------------------------------------------------
// /loop — recurring cron prompt
// ---------------------------------------------------------------------------

/// Parse an optional leading interval token (`5m`, `2h`, `1d`, `30s`) from the
/// argument string.  Returns `(interval_str, rest_of_prompt)`.
fn parse_loop_interval(args: &str) -> (&str, &str) {
    // Regex-free: scan the first "word" for digits followed by s/m/h/d.
    let args = args.trim();
    let end = args.find(|c: char| c.is_whitespace()).unwrap_or(args.len());
    let candidate = &args[..end];
    let valid = candidate.len() >= 2
        && candidate[..candidate.len() - 1]
            .chars()
            .all(|c| c.is_ascii_digit())
        && matches!(
            candidate.chars().last(),
            Some('s') | Some('m') | Some('h') | Some('d')
        );
    if valid {
        let rest = args[end..].trim();
        (candidate, rest)
    } else {
        ("10m", args)
    }
}

/// Convert a simple interval string (`5m`, `2h`, `1d`, `90s`) to a cron
/// expression.  Seconds are rounded up to the nearest minute (minimum 1 min).
fn interval_to_cron(interval: &str) -> String {
    let n: u64 = interval[..interval.len() - 1].parse().unwrap_or(10);
    match interval.chars().last() {
        Some('s') => {
            let mins = n.div_ceil(60).max(1);
            format!("*/{mins} * * * *")
        }
        Some('m') => format!("*/{n} * * * *"),
        Some('h') => format!("0 */{n} * * *"),
        Some('d') => format!("0 0 */{n} * *"),
        _ => format!("*/{n} * * * *"),
    }
}

/// `/loop [interval] <prompt>` — set up a recurring cron job that fires
/// `<prompt>` and immediately execute the prompt once now.
pub(super) async fn handle_loop_command(
    app: &mut App,
    args: &str,
    tx: Option<&mpsc::Sender<AppEvent>>,
) {
    if args.trim().is_empty() {
        app.messages.push(ChatMessage::user("/loop".to_owned()));
        app.messages.push(ChatMessage::assistant(
            "Usage: `/loop [interval] <prompt>`\n\n\
Examples:\n\
- `/loop 5m check the deploy`\n\
- `/loop 1h /review`\n\
- `/loop check the deploy`  (defaults to 10 m)\n\n\
Supported intervals: `Xs` (seconds), `Xm` (minutes), `Xh` (hours), `Xd` (days)."
                .into(),
        ));
        app.scroll_to_bottom();
        return;
    }

    let (interval, user_prompt) = parse_loop_interval(args);
    if user_prompt.is_empty() {
        app.messages
            .push(ChatMessage::user(format!("/loop {args}")));
        app.messages.push(ChatMessage::assistant(
            "No prompt found after the interval. Usage: `/loop [interval] <prompt>`".into(),
        ));
        app.scroll_to_bottom();
        return;
    }
    let cron_expr = interval_to_cron(interval);
    let description_prefix: String = user_prompt.chars().take(40).collect();

    let echo = format!("/loop {args}");
    app.messages.push(ChatMessage::user(echo));

    let prompt = format!(
        "# /loop — Schedule recurring prompt\n\n\
Set up a recurring cron for the following:\n\
- Interval: {interval} → cron: {cron_expr}\n\
- Prompt: \"{user_prompt}\"\n\n\
Use the CronCreate tool with:\n\
- schedule: \"{cron_expr}\"\n\
- command: the prompt text above\n\
- description: \"Loop: {description_prefix}\"\n\n\
Then immediately execute the prompt now (do not wait for the first cron fire)."
    );

    let Some(tx) = tx else {
        app.messages.push(ChatMessage::assistant(format!(
            "Setting up loop every {interval}: {user_prompt}\n\n\
*(no stream channel — submit from the input bar to drive the model)*"
        )));
        app.scroll_to_bottom();
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
    app.turn_start_cost = crate::cost::total_cost(&app.usage_by_model);
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
    tokio::spawn(async move {
        crate::stream::stream_response(
            provider, messages, model, tx_stream, interrupt, cancel, None, overrides,
        )
        .await;
    });
}

// ---------------------------------------------------------------------------
// /schedule — view and manage cron schedules
// ---------------------------------------------------------------------------

/// `/schedule [list|cancel <id>]` — list or cancel scheduled cron jobs.
///
/// - No arg / `list` → inject a message asking the model to call `CronList`
/// - `cancel <id>` → inject a message asking the model to call `CronDelete`
pub(super) async fn handle_schedule_command(
    app: &mut App,
    arg: &str,
    tx: Option<&mpsc::Sender<AppEvent>>,
) {
    let arg = arg.trim();
    let (echo, prompt, status_msg) = if arg.is_empty() || arg == "list" {
        (
            "/schedule".to_owned(),
            "# /schedule list\n\nUse the CronList tool to list all registered cron jobs \
and display the results in a readable table with columns: id, schedule, command, description."
                .to_owned(),
            "Listing scheduled cron jobs…".to_owned(),
        )
    } else if let Some(id) = arg
        .strip_prefix("cancel")
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        let id = id.to_owned();
        (
            format!("/schedule cancel {id}"),
            format!(
                "# /schedule cancel\n\nUse the CronDelete tool to cancel the cron job with id \
`{id}`. Confirm the deletion to the user after the tool call succeeds."
            ),
            format!("Cancelling cron job {id}…"),
        )
    } else {
        // Unknown subcommand — show help inline, no model turn needed.
        app.messages
            .push(ChatMessage::user(format!("/schedule {arg}")));
        app.messages.push(ChatMessage::assistant(
            "Usage:\n\
  `/schedule` or `/schedule list` — list all scheduled cron jobs\n\
  `/schedule cancel <id>` — cancel a cron job by id"
                .into(),
        ));
        app.scroll_to_bottom();
        return;
    };

    app.messages.push(ChatMessage::user(echo));

    let Some(tx) = tx else {
        app.messages.push(ChatMessage::assistant(format!(
            "{status_msg}\n\n*(no stream channel — submit from the input bar to drive the model)*"
        )));
        app.scroll_to_bottom();
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
    app.turn_start_cost = crate::cost::total_cost(&app.usage_by_model);
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
    tokio::spawn(async move {
        crate::stream::stream_response(
            provider, messages, model, tx_stream, interrupt, cancel, None, overrides,
        )
        .await;
    });
}
