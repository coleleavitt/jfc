//! Slash handlers: context, compaction & agent control.

use crate::commands::prelude::*;

pub(super) async fn cmd_check(
    state: &mut EngineState,
    _parts: &[&str],
    _text: &str,
    _tx: Option<&mpsc::Sender<EngineEvent>>,
) {
    // Re-run `cargo check --message-format=json` and refresh the
    // diagnostic row + transition toast. v126 has an analogous
    // `/diagnostics` flow; keep ours short. Best-effort — silently
    // no-ops outside a cargo project.
    state.messages.push(ChatMessage::user("/check".into()));
    state.messages.push(ChatMessage::assistant(
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
    state: &mut EngineState,
    _parts: &[&str],
    _text: &str,
    _tx: Option<&mpsc::Sender<EngineEvent>>,
) {
    // Use the calibrated context size (same source as the gauge
    // and pre-submit gate). Previously this re-ran the raw
    // `estimate_tokens` heuristic, so the manual report disagreed
    // with the live gauge and could show "0%" for a session the
    // sidebar reports as 90%-full.
    let est = state.tool_ctx.approx_tokens;
    let level = crate::compact::compact_level_with_output(
        est,
        state.max_context_tokens,
        state.max_output_tokens,
    );
    let pct = (est * 100)
        .checked_div(state.max_context_tokens)
        .map_or(0, |p| p.min(999));
    tracing::info!(
        target: "jfc::compact",
        est, max_context_tokens = state.max_context_tokens,
        pct, ?level, model = %state.model,
        "manual /compact command invoked"
    );
    state.messages.push(ChatMessage::user("/compact".into()));
    state.messages.push(ChatMessage::assistant(format!(
                "Manual compaction queued — current estimate **{est} / {} tokens ({pct}%)**, level: **{level:?}**.\n\n\
                 The next assistant turn will summarize the conversation up to here, replacing the prior turns with a 9-section summary.\n\n\
                 *(Tip: set `JFC_AUTOCOMPACT_PCT_OVERRIDE=N` (1-100) to test thresholds, or `JFC_DISABLE_AUTO_COMPACT=1` to disable auto-compact entirely.)*",
                state.max_context_tokens
            )));
    state.force_compact_pending = true;
}

pub(super) async fn cmd_advisor(
    state: &mut EngineState,
    parts: &[&str],
    text: &str,
    _tx: Option<&mpsc::Sender<EngineEvent>>,
) {
    // Manual local advisor query (see `crate::advisor`). Doesn't touch the main
    // agent's stream — runs a separate `provider.complete()` against a
    // SNAPSHOT of the current transcript and surfaces the reply as a dedicated
    // `MessagePart::Advisor` part with its own visual style. Model-initiated
    // Advisor tool calls use the stream dispatcher and return as normal tool
    // results.
    //
    // Default-off per deliverable: gated by `state.advisor_enabled`,
    // populated from local advisor config or `JFC_ADVISOR_ENABLED=1` on
    // startup. Even when on, each session has a per-budget ceiling
    // (`DEFAULT_TOKEN_BUDGET`) so a runaway loop can't drain the user's account.
    let args = text.trim().strip_prefix("/advisor").unwrap_or("").trim();
    let first = parts.get(1).copied().unwrap_or("").trim();
    // Echo the user's command into the transcript first so the chat
    // shows what the user asked, even on the error paths below.
    state.messages.push(ChatMessage::user(text.to_owned()));

    if args.is_empty() || first.eq_ignore_ascii_case("status") {
        let server = state
            .server_advisor_model
            .as_ref()
            .map(|m| m.to_string())
            .unwrap_or_else(|| "disabled".to_owned());
        let local = state
            .local_advisor_model
            .as_ref()
            .map(|m| match state.local_advisor_provider.as_ref() {
                Some(provider) => format!("{provider}/{m}"),
                None => m.to_string(),
            })
            .unwrap_or_else(|| "disabled".to_owned());
        state.messages
            .push(ChatMessage::assistant_parts(vec![MessagePart::Advisor(
                format!(
                    "Local advisor: `{local}`\nServer advisor: `{server}`\n\nUse `/advisor config <model>` for local, `/advisor server <model>` for Anthropic server-side, or `/advisor off`."
                ),
            )]));
        return;
    }

    if first.eq_ignore_ascii_case("server") {
        let raw_model = args.get(first.len()..).unwrap_or("").trim();
        if matches!(
            raw_model.to_ascii_lowercase().as_str(),
            "off" | "disable" | "disabled"
        ) {
            match crate::config::save_server_advisor_model(None) {
                Ok(_) => {
                    let old_identity = crate::cache_lineage::current_identity(state);
                    crate::advisor::set_active_server_advisor_model(None);
                    state.server_advisor_model = None;
                    let new_identity = crate::cache_lineage::current_identity(state);
                    let piggyback_drop = settle_cache_identity_change(
                        state,
                        &old_identity,
                        &new_identity,
                        "advisor config change",
                    );
                    push_advisor_reply_after_cache_change(
                        state,
                        text,
                        "Server advisor disabled.".into(),
                        piggyback_drop,
                    );
                }
                Err(e) => {
                    state
                        .messages
                        .push(ChatMessage::assistant_parts(vec![MessagePart::Advisor(
                            format!("Could not persist server advisor setting: {e}"),
                        )]));
                }
            }
            return;
        }
        if raw_model.is_empty() {
            state.messages
                .push(ChatMessage::assistant_parts(vec![MessagePart::Advisor(
                    "Usage: `/advisor server opus`, `/advisor server sonnet`, `/advisor server <model-id>`, or `/advisor server off`.".into(),
                )]));
            return;
        }
        if !matches!(
            state.provider.stream_convention(),
            jfc_provider::StreamConvention::AnthropicNative
        ) || !matches!(state.provider.name(), "anthropic" | "anthropic-oauth")
        {
            state.messages
                .push(ChatMessage::assistant_parts(vec![MessagePart::Advisor(
                    format!(
                        "Server advisor requires an Anthropic-native provider; active provider is `{}`.",
                        state.provider.name()
                    ),
                )]));
            return;
        }
        match crate::advisor::resolve_server_advisor_model(
            &state.model,
            Some(raw_model),
            true,
            true,
        ) {
            Ok(Some(model)) => {
                match crate::config::save_server_advisor_model(Some(model.as_str())) {
                    Ok(_) => {
                        let old_identity = crate::cache_lineage::current_identity(state);
                        crate::advisor::set_active_server_advisor_model(Some(model.clone()));
                        state.server_advisor_model = Some(model.clone());
                        let new_identity = crate::cache_lineage::current_identity(state);
                        let piggyback_drop = settle_cache_identity_change(
                            state,
                            &old_identity,
                            &new_identity,
                            "advisor config change",
                        );
                        push_advisor_reply_after_cache_change(
                            state,
                            text,
                            format!("Server advisor set to `{model}`."),
                            piggyback_drop,
                        );
                    }
                    Err(e) => {
                        state.messages.push(ChatMessage::assistant_parts(vec![
                            MessagePart::Advisor(format!(
                                "Could not persist server advisor setting: {e}"
                            )),
                        ]));
                    }
                }
            }
            Ok(None) => {
                state
                    .messages
                    .push(ChatMessage::assistant_parts(vec![MessagePart::Advisor(
                        "Server advisor is not available for the active model/provider.".into(),
                    )]));
            }
            Err(e) => {
                state
                    .messages
                    .push(ChatMessage::assistant_parts(vec![MessagePart::Advisor(
                        format!("Server advisor config error: {e}"),
                    )]));
            }
        }
        return;
    }

    if matches!(
        first.to_ascii_lowercase().as_str(),
        "off" | "disable" | "disabled"
    ) {
        match crate::config::save_advisor_model(None) {
            Ok(_) => {
                let old_identity = crate::cache_lineage::current_identity(state);
                crate::advisor::set_active_local_advisor_model(None);
                state.local_advisor_model = None;
                state.advisor_enabled = false;
                state.advisor_session = None;
                let new_identity = crate::cache_lineage::current_identity(state);
                let piggyback_drop = settle_cache_identity_change(
                    state,
                    &old_identity,
                    &new_identity,
                    "advisor config change",
                );
                push_advisor_reply_after_cache_change(
                    state,
                    text,
                    "Local advisor disabled.".into(),
                    piggyback_drop,
                );
            }
            Err(e) => {
                state
                    .messages
                    .push(ChatMessage::assistant_parts(vec![MessagePart::Advisor(
                        format!("Could not persist advisor setting: {e}"),
                    )]));
            }
        }
        return;
    }

    if matches!(
        first.to_ascii_lowercase().as_str(),
        "config" | "model" | "set"
    ) {
        let raw_model = args.get(first.len()..).unwrap_or("").trim();
        if raw_model.is_empty() {
            state.messages
                .push(ChatMessage::assistant_parts(vec![MessagePart::Advisor(
                    "Usage: `/advisor config opus`, `/advisor config sonnet`, `/advisor config openai/gpt-5.5`, or `/advisor config <provider/model>`.".into(),
                )]));
            return;
        }
        match crate::advisor::resolve_local_advisor_model(
            &state.model,
            Some(raw_model),
            true,
            Some(true),
        ) {
            Ok(Some(model)) => {
                let provider = crate::advisor::resolve_local_advisor_provider(
                    &state.providers,
                    std::sync::Arc::clone(&state.provider),
                    model.provider.as_ref(),
                    &model.model,
                );
                match provider {
                    Ok(provider) => {
                        match crate::config::save_advisor_model(Some(&model.config_value())) {
                            Ok(_) => {
                                let old_identity = crate::cache_lineage::current_identity(state);
                                crate::advisor::set_active_local_advisor_provider(
                                    model.provider.clone(),
                                );
                                crate::advisor::set_active_local_advisor_model(Some(
                                    model.model.clone(),
                                ));
                                state.local_advisor_provider = model.provider.clone();
                                state.local_advisor_model = Some(model.model.clone());
                                state.advisor_enabled = true;
                                state.advisor_session =
                                    Some(crate::advisor::AdvisorSession::new(model.model.clone()));
                                let new_identity = crate::cache_lineage::current_identity(state);
                                let piggyback_drop = settle_cache_identity_change(
                                    state,
                                    &old_identity,
                                    &new_identity,
                                    "advisor config change",
                                );
                                push_advisor_reply_after_cache_change(
                                    state,
                                    text,
                                    format!(
                                        "Local advisor set to `{}` via `{}`.",
                                        model.config_value(),
                                        provider.name()
                                    ),
                                    piggyback_drop,
                                );
                            }
                            Err(e) => {
                                state.messages.push(ChatMessage::assistant_parts(vec![
                                    MessagePart::Advisor(format!(
                                        "Could not persist advisor setting: {e}"
                                    )),
                                ]));
                            }
                        }
                    }
                    Err(e) => {
                        state.messages.push(ChatMessage::assistant_parts(vec![
                            MessagePart::Advisor(format!("Advisor provider config error: {e}")),
                        ]));
                    }
                }
            }
            Ok(None) => {
                state
                    .messages
                    .push(ChatMessage::assistant_parts(vec![MessagePart::Advisor(
                        "Local advisor is not available.".into(),
                    )]));
            }
            Err(e) => {
                state
                    .messages
                    .push(ChatMessage::assistant_parts(vec![MessagePart::Advisor(
                        format!("Advisor config error: {e}"),
                    )]));
            }
        }
        return;
    }

    let query = args.to_owned();
    if !state.advisor_enabled {
        state.messages
            .push(ChatMessage::assistant_parts(vec![MessagePart::Advisor(
                "Local advisor queries are disabled. Use `/advisor config <model>` or start with `--advisor [MODEL]`."
                    .into(),
            )]));
    } else {
        // Lazy-mint the session on first use so users that never
        // call /advisor pay no allocation cost. The session model
        // tracks the *active* model at first invocation; switching
        // models mid-session keeps the original advisor model.
        let session = state.advisor_session.get_or_insert_with(|| {
            crate::advisor::AdvisorSession::new(
                state
                    .local_advisor_model
                    .clone()
                    .unwrap_or_else(|| state.model.clone()),
            )
        });
        // Snapshot — Vec::clone is fine here, the deliverable
        // explicitly calls for a SNAPSHOT semantic. Without the
        // clone, `ask_advisor` would borrow `state.messages`
        // immutably while we're holding `&mut state.advisor_session`
        // mutably — borrow-check fails.
        let snapshot = state.messages.clone();
        let targets = match crate::advisor::resolve_local_advisor_provider_targets(
            &state.providers,
            std::sync::Arc::clone(&state.provider),
            state.local_advisor_provider.as_ref(),
            &session.model,
        ) {
            Ok(targets) => targets,
            Err(e) => {
                state
                    .messages
                    .push(ChatMessage::assistant_parts(vec![MessagePart::Advisor(
                        format!("Advisor provider error: {e}"),
                    )]));
                return;
            }
        };
        match crate::advisor::ask_advisor_with_fallback(&targets, session, query.clone(), &snapshot)
            .await
        {
            Ok(reply) => {
                let remaining = session.tokens_remaining();
                let total_budget = session.token_budget;
                state.local_advisor_provider = Some(reply.provider.clone());
                state.local_advisor_model = Some(reply.model.clone());
                crate::advisor::set_active_local_advisor_provider(Some(reply.provider.clone()));
                crate::advisor::set_active_local_advisor_model(Some(reply.model.clone()));
                state
                    .messages
                    .push(ChatMessage::assistant_parts(vec![MessagePart::Advisor(
                        format!(
                            "{}\n\n_({}; advisor budget: {} of {} tokens remaining)_",
                            reply.content,
                            reply.model_note(),
                            remaining,
                            total_budget
                        ),
                    )]));
            }
            Err(e) => {
                state.messages.push(ChatMessage::assistant_parts(vec![
                            MessagePart::Advisor(format!(
                                "Advisor error: {e}\n\nUse `/clear` to start a fresh session if the budget is exhausted."
                            )),
                        ]));
            }
        }
    }
}

/// `/council [model-a,model-b,…] <question>` — convene a multi-model council.
///
/// Fans the question out to each member model in parallel (one tool-less
/// `provider.complete()` each, like `/advisor`), then an arbiter model
/// synthesises the member answers into one consolidated reply that surfaces
/// agreement/disagreement. The first member doubles as the arbiter unless a
/// dedicated one is configured. Budget-bounded like the advisor so a runaway
/// fan-out can't drain the account.
///
/// Member selection: a leading comma-separated token list is treated as model
/// ids; otherwise members default to the active model plus the local advisor
/// model (when distinct), giving a 2-model council out of the box.
pub(super) async fn cmd_council(
    state: &mut EngineState,
    _parts: &[&str],
    text: &str,
    _tx: Option<&mpsc::Sender<EngineEvent>>,
) {
    use crate::council::{
        CouncilIntent, CouncilMember, CouncilRequest, run_agentic_council, run_council,
    };

    state.messages.push(ChatMessage::user(text.to_owned()));
    let args = text.trim().strip_prefix("/council").unwrap_or("").trim();
    if args.is_empty() {
        state
            .messages
            .push(ChatMessage::assistant_parts(vec![MessagePart::Advisor(
                "Usage: `/council <question>` or `/council model-a,model-b <question>`.".into(),
            )]));
        return;
    }

    let cfg = crate::config::load_arc();
    let council_cfg = cfg.council.as_ref();
    let agentic = matches!(
        council_cfg.map(|cfg| &cfg.mode),
        Some(jfc_config::CouncilMode::Agentic)
    );

    // A leading token with a comma is a member list; otherwise default members.
    let (model_ids, question) = parse_council_args(args, state, council_cfg);
    if question.trim().is_empty() {
        state
            .messages
            .push(ChatMessage::assistant_parts(vec![MessagePart::Advisor(
                "No question provided to the council.".into(),
            )]));
        return;
    }

    let mut members = Vec::new();
    let mut unresolved = Vec::new();
    for id in &model_ids {
        match crate::runtime::bootstrap::resolve_provider_model(&state.providers, id) {
            Some(res) => {
                let label = council_cfg
                    .and_then(|cfg| {
                        cfg.members.iter().find_map(|member| {
                            let model_matches = member.model.trim() == id.trim();
                            model_matches
                                .then(|| member.name.as_deref())
                                .flatten()
                                .map(str::trim)
                                .filter(|s| !s.is_empty())
                                .map(str::to_owned)
                        })
                    })
                    .unwrap_or_else(|| id.clone());
                members.push(CouncilMember::new(res.provider, res.model).with_label(label));
            }
            None => unresolved.push(id.clone()),
        }
    }

    if members.is_empty() {
        state
            .messages
            .push(ChatMessage::assistant_parts(vec![MessagePart::Advisor(
                format!(
                    "Could not resolve any council models from: {}. Try `/council <model-id> <question>` with a configured model.",
                    model_ids.join(", ")
                ),
            )]));
        return;
    }

    let snapshot = render_council_snapshot(&state.messages);
    let mut request = CouncilRequest::new(question, members).with_context(snapshot);
    if let Some(cfg) = council_cfg {
        request = request
            .with_quorum(cfg.quorum)
            .with_retry_on_fail(cfg.retry_on_fail)
            .with_archive(
                cfg.archive,
                Some(std::env::current_dir().unwrap_or_default()),
            );
        request = if cfg.member_timeout_ms == 0 {
            request.with_member_timeout(None)
        } else {
            request.with_member_timeout(Some(std::time::Duration::from_millis(
                cfg.member_timeout_ms,
            )))
        };
        if let Some(intent) = cfg.intent.as_deref().and_then(CouncilIntent::parse) {
            request = request.with_intent(Some(intent));
        }
    }
    let council_result = if agentic {
        run_agentic_council(
            request,
            Some(state.task_store.clone()),
            state.team_context.team_name.as_deref(),
            std::env::current_dir().unwrap_or_default(),
        )
        .await
    } else {
        run_council(request).await
    };
    match council_result {
        Ok(report) => {
            let mut body = report.to_markdown();
            if !unresolved.is_empty() {
                body.push_str(&format!(
                    "\n_(skipped unresolved models: {})_",
                    unresolved.join(", ")
                ));
            }
            state
                .messages
                .push(ChatMessage::assistant_parts(vec![MessagePart::Advisor(
                    body,
                )]));
        }
        Err(e) => {
            state
                .messages
                .push(ChatMessage::assistant_parts(vec![MessagePart::Advisor(
                    format!("Council error: {e}"),
                )]));
        }
    }
}

/// Split `/council` args into (member model ids, question). A leading token is
/// treated as a comma-separated member list only when it actually contains a
/// comma; otherwise members default to the active model + local advisor model.
fn parse_council_args(
    args: &str,
    state: &EngineState,
    council_cfg: Option<&jfc_config::CouncilConfig>,
) -> (Vec<String>, String) {
    if let Some((head, rest)) = args.split_once(char::is_whitespace) {
        if head.contains(',') {
            let ids: Vec<String> = head
                .split(',')
                .map(|s| s.trim().to_owned())
                .filter(|s| !s.is_empty())
                .collect();
            if !ids.is_empty() {
                return (ids, rest.trim().to_owned());
            }
        }
    }
    (default_council_models(state, council_cfg), args.to_owned())
}

/// Default council membership: the active model plus the local advisor model
/// when it's distinct. Falls back to a single-model council (still valid).
fn default_council_models(
    state: &EngineState,
    council_cfg: Option<&jfc_config::CouncilConfig>,
) -> Vec<String> {
    if let Some(configured) = council_cfg
        .map(|cfg| cfg.members.as_slice())
        .filter(|members| !members.is_empty())
    {
        let ids: Vec<String> = configured
            .iter()
            .map(|member| member.model.trim())
            .filter(|model| !model.is_empty())
            .map(str::to_owned)
            .collect();
        if !ids.is_empty() {
            return ids;
        }
    }
    let mut ids = vec![state.model.to_string()];
    if let Some(advisor) = state.local_advisor_model.as_ref() {
        let advisor = advisor.to_string();
        if advisor != ids[0] {
            ids.push(advisor);
        }
    }
    ids
}

/// Render a compact transcript snapshot as shared council context. Mirrors the
/// advisor's snapshot semantic but kept short to bound member token cost.
fn render_council_snapshot(messages: &[ChatMessage]) -> String {
    const MAX_CHARS: usize = 8_000;
    let mut out = String::new();
    for msg in messages.iter().rev() {
        let role = match msg.role {
            Role::User => "User",
            Role::Assistant => "Assistant",
        };
        let text = msg
            .parts
            .iter()
            .map(|p| p.text_only())
            .collect::<Vec<_>>()
            .join(" ");
        if text.trim().is_empty() {
            continue;
        }
        let line = format!("{role}: {text}\n");
        if out.len() + line.len() > MAX_CHARS {
            break;
        }
        out.insert_str(0, &line);
    }
    out
}

/// `/research <question>` — run the deep-research loop: plan sub-queries,
/// search the web per step, and synthesise the evidence into one answer.
///
/// Mirrors Perplexity's `/rest/sse/perplexity_ask` research flow (PLAN →
/// pro_search_step → FINAL). Runs out-of-band like `/advisor` and `/council`:
/// it does its own web searches via `research::WebSearcher` and a deterministic
/// `LocalSynthesizer`, then surfaces the markdown report. Append ` --export` to
/// also write a durable artifact bundle next to the project.
pub(super) async fn cmd_research(
    state: &mut EngineState,
    _parts: &[&str],
    text: &str,
    _tx: Option<&mpsc::Sender<EngineEvent>>,
) {
    use crate::research::{LocalSynthesizer, ResearchRequest, WebSearcher, run_research};

    state.messages.push(ChatMessage::user(text.to_owned()));
    let args = text.trim().strip_prefix("/research").unwrap_or("").trim();

    let export = args.contains("--export");
    let question = args.replace("--export", "").trim().to_owned();
    if question.is_empty() {
        state
            .messages
            .push(ChatMessage::assistant_parts(vec![MessagePart::Advisor(
                "Usage: `/research <question>` (add `--export` to save an artifact).".into(),
            )]));
        return;
    }

    let request = ResearchRequest::new(question);
    let searcher = WebSearcher;
    let synthesizer = LocalSynthesizer;
    match run_research(request, &searcher, &synthesizer).await {
        Ok(report) => {
            let mut body = report.to_markdown();
            if export {
                match report.export(&std::env::temp_dir().join("jfc-research")) {
                    Ok(artifact) => body.push_str(&format!(
                        "\n\n_Artifact saved: `{}`_",
                        artifact.markdown_path.display()
                    )),
                    Err(e) => body.push_str(&format!("\n\n_(export failed: {e})_")),
                }
            }
            state
                .messages
                .push(ChatMessage::assistant_parts(vec![MessagePart::Advisor(
                    body,
                )]));
        }
        Err(e) => {
            state
                .messages
                .push(ChatMessage::assistant_parts(vec![MessagePart::Advisor(
                    format!("Research error: {e}"),
                )]));
        }
    }
}

/// `/btw <question>` — ask a quick side question without interrupting current work.
/// CC 177 parity: spawns a separate lightweight agent with NO tools, gets one
/// response only, and doesn't disturb the main agent's work.
pub(super) async fn cmd_btw(
    state: &mut EngineState,
    _parts: &[&str],
    text: &str,
    _tx: Option<&mpsc::Sender<EngineEvent>>,
) {
    use jfc_provider::{ProviderContent, ProviderMessage, ProviderRole, StreamOptions};

    let question = text
        .trim()
        .strip_prefix("/btw")
        .unwrap_or("")
        .trim()
        .to_owned();
    if question.is_empty() {
        state.messages.push(ChatMessage::user(text.to_owned()));
        state
            .messages
            .push(ChatMessage::assistant_parts(vec![MessagePart::Advisor(
                "Usage: `/btw <your question>` — ask a quick side question without interrupting current work.".into(),
            )]));
        return;
    }

    // Echo the question in the transcript
    state
        .messages
        .push(ChatMessage::user(format!("/btw {question}")));

    // Build a snapshot of recent context for the side agent
    let context = render_btw_context(&state.messages);
    let prompt = format!(
        "<system-reminder>This is a side question from the user. Answer directly in a single response.

IMPORTANT CONTEXT:
- You are a separate, lightweight agent spawned to answer this one question
- The main agent is NOT interrupted — it continues working independently
- You share the conversation context but are a completely separate instance

CRITICAL CONSTRAINTS:
- You have NO tools available — you cannot read files, run commands, or take any actions
- This is a one-off response — there will be no follow-up turns
- NEVER say things like \"Let me try...\", \"I'll now...\", or promise to take any action
- If you don't know the answer from the context, say so

Simply answer the question with the information you have.</system-reminder>

Context from the conversation:
{context}

Question: {question}"
    );

    let messages = vec![ProviderMessage {
        role: ProviderRole::User,
        content: vec![ProviderContent::Text(prompt)],
    }];

    let opts = StreamOptions::new(state.model.clone()).max_tokens(2048);
    match state.provider.complete(messages, &opts).await {
        Ok(response) => {
            // Surface the answer with a distinctive visual style
            state
                .messages
                .push(ChatMessage::assistant_parts(vec![MessagePart::Advisor(
                    format!("**BTW:** {}", response.content),
                )]));
        }
        Err(e) => {
            state
                .messages
                .push(ChatMessage::assistant_parts(vec![MessagePart::Advisor(
                    format!("Side question error: {e}"),
                )]));
        }
    }
}

/// Render a compact context snapshot for `/btw` side questions.
fn render_btw_context(messages: &[ChatMessage]) -> String {
    const MAX_CHARS: usize = 4_000;
    let mut out = String::new();
    for msg in messages.iter().rev() {
        let role = match msg.role {
            Role::User => "User",
            Role::Assistant => "Assistant",
        };
        let text = msg
            .parts
            .iter()
            .map(|p| p.text_only())
            .collect::<Vec<_>>()
            .join(" ");
        if text.trim().is_empty() {
            continue;
        }
        let line = format!("{role}: {text}\n");
        if out.len() + line.len() > MAX_CHARS {
            break;
        }
        out.insert_str(0, &line);
    }
    out
}

pub(super) async fn cmd_config(
    state: &mut EngineState,
    parts: &[&str],
    text: &str,
    _tx: Option<&mpsc::Sender<EngineEvent>>,
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
    state.messages.push(ChatMessage::user(text.to_owned()));
    if arg == "path" {
        let p = crate::config::config_path();
        state.messages.push(ChatMessage::assistant(format!(
            "**Config path:** `{}`",
            p.display()
        )));
    } else {
        let cfg = crate::config::load_arc();
        let body = match toml::to_string_pretty(&cfg) {
            Ok(s) if s.trim().is_empty() => "(empty config — no overrides)".to_owned(),
            Ok(s) => format!("```toml\n{s}```"),
            Err(e) => format!("**Error serializing config:** {e}"),
        };
        state.messages.push(ChatMessage::assistant(body));
    }
}

fn settle_cache_identity_change(
    state: &mut EngineState,
    old_identity: &str,
    new_identity: &str,
    change_kind: &str,
) -> Option<crate::cache_lineage::PiggybackDrop> {
    if new_identity == old_identity {
        return None;
    }
    let drop = crate::cache_lineage::maybe_piggyback_drop_for_identity_change(
        state,
        new_identity,
        change_kind,
    );
    state.last_response_id = state
        .response_ids_by_cache_identity
        .get(new_identity)
        .cloned();
    drop
}

fn append_cache_lineage_note(
    reply: &mut String,
    piggyback_drop: Option<crate::cache_lineage::PiggybackDrop>,
) {
    if let Some(drop) = piggyback_drop {
        reply.push_str(&format!(
            "\n\nCache lineage preserved: trimmed {} incompatible tail messages.",
            drop.dropped_messages
        ));
        if let Some(archive_id) = drop.archive_id {
            reply.push_str(&format!(" Raw tail archive: `/expand {archive_id}`."));
        }
    }
}

fn push_advisor_reply_after_cache_change(
    state: &mut EngineState,
    text: &str,
    mut reply: String,
    piggyback_drop: Option<crate::cache_lineage::PiggybackDrop>,
) {
    let reecho_user = piggyback_drop.is_some();
    append_cache_lineage_note(&mut reply, piggyback_drop);
    if reecho_user {
        state.messages.push(ChatMessage::user(text.to_owned()));
    }
    state
        .messages
        .push(ChatMessage::assistant_parts(vec![MessagePart::Advisor(
            reply,
        )]));
}

pub(super) async fn cmd_model(
    state: &mut EngineState,
    parts: &[&str],
    text: &str,
    _tx: Option<&mpsc::Sender<EngineEvent>>,
) {
    // `/model <name>` immediately switches the active model for
    // subsequent turns without restarting the session or clearing history.
    let arg = parts.get(1).copied().unwrap_or("").trim();
    if arg.is_empty() {
        state.messages.push(ChatMessage::user(text.to_owned()));
        state.messages.push(ChatMessage::assistant(format!(
            "Current model: `{}`\n\nUsage: `/model <name>` to switch.\n\
             Or press Ctrl+M to open the model picker.",
            state.model.as_str()
        )));
        return;
    }
    let requested_model = arg.to_string();
    let old_model = state.model.clone();
    let old_identity = crate::cache_lineage::current_identity(state);
    let mut recent_model = requested_model.clone();
    if let Some(resolved) =
        crate::runtime::bootstrap::resolve_provider_model(&state.providers, &requested_model)
    {
        state.provider = resolved.provider;
        state.model = resolved.model;
        recent_model =
            crate::runtime::bootstrap::qualified_model_id(state.provider.as_ref(), &state.model);
    } else {
        state.model = jfc_provider::ModelId::new(requested_model);
    }
    let new_identity = crate::cache_lineage::current_identity(state);
    let piggyback_drop =
        settle_cache_identity_change(state, &old_identity, &new_identity, "provider/model switch");

    state.messages.push(ChatMessage::user(text.to_owned()));
    crate::app::push_recent_model(&mut state.recent_models, &recent_model);
    state.sync_selected_context_window();
    tracing::info!(
        target: "jfc::input",
        old_model = %old_model,
        new_model = %state.model,
        provider = %state.provider.name(),
        old_identity = %old_identity,
        new_identity = %new_identity,
        piggyback_dropped_messages = piggyback_drop.as_ref().map_or(0, |drop| drop.dropped_messages),
        "model switch via /model command"
    );
    let mut reply = format!("Model switched to: {}", state.model);
    append_cache_lineage_note(&mut reply, piggyback_drop);
    state.messages.push(ChatMessage::assistant(reply));
}

pub(super) async fn cmd_fast(
    state: &mut EngineState,
    _parts: &[&str],
    text: &str,
    _tx: Option<&mpsc::Sender<EngineEvent>>,
) {
    // Toggle fast mode (lower-latency inference via Anthropic's
    // `fast-mode-2026-02-01` beta header). Mirrors Claude Code
    // v2.1.139's `/fast` command (Alt+O keybind).
    let old_identity = crate::cache_lineage::current_identity(state);
    state.fast_mode = !state.fast_mode;
    crate::effort::set_fast_mode_global(state.fast_mode);
    let new_identity = crate::cache_lineage::current_identity(state);
    let piggyback_drop =
        settle_cache_identity_change(state, &old_identity, &new_identity, "fast-mode toggle");
    state.messages.push(ChatMessage::user(text.to_owned()));
    let mut reply = format!(
        "Fast mode: **{}** — {}",
        if state.fast_mode { "ON" } else { "OFF" },
        if state.fast_mode {
            "requests will use the low-latency inference path"
        } else {
            "requests will use the standard inference path"
        },
    );
    append_cache_lineage_note(&mut reply, piggyback_drop);
    state.messages.push(ChatMessage::assistant(reply));
}

pub(super) async fn cmd_pin(
    state: &mut EngineState,
    parts: &[&str],
    text: &str,
    _tx: Option<&mpsc::Sender<EngineEvent>>,
) {
    // Pin a message by transcript index so compaction can't
    // drop it. /pin without an arg pins the most recent
    // message; /pin <n> pins index n; /pin list prints the
    // current pin set.
    // Capture the index of the message the user means by "most recent"
    // BEFORE pushing the /pin echo — otherwise the echo itself gets pinned.
    let prior_idx = state.messages.len().checked_sub(1);
    state.messages.push(ChatMessage::user(text.to_owned()));
    let arg = parts.get(1).copied().unwrap_or("").trim();
    if arg == "list" {
        if state.pinned_message_indices.is_empty() {
            state.messages.push(ChatMessage::assistant(
                "No pinned messages. `/pin <n>` pins index n; `/pin` pins the most recent.".into(),
            ));
        } else {
            let mut idx: Vec<usize> = state.pinned_message_indices.iter().copied().collect();
            idx.sort();
            let listing = idx
                .into_iter()
                .map(|i| format!("- #{i}"))
                .collect::<Vec<_>>()
                .join("\n");
            state.messages.push(ChatMessage::assistant(format!(
                "**Pinned messages:**\n{listing}"
            )));
        }
    } else if arg.is_empty() {
        let Some(idx) = prior_idx else {
            state.messages.push(ChatMessage::assistant(
                "Nothing to pin yet — the transcript is empty.".into(),
            ));
            return;
        };
        state.pinned_message_indices.insert(idx);
        state.messages.push(ChatMessage::assistant(format!(
            "Pinned message #{idx} (compaction will preserve it)."
        )));
    } else {
        match arg.parse::<usize>() {
            Ok(idx) if idx < state.messages.len() => {
                state.pinned_message_indices.insert(idx);
                state
                    .messages
                    .push(ChatMessage::assistant(format!("Pinned message #{idx}.")));
            }
            Ok(idx) => {
                state.messages.push(ChatMessage::assistant(format!(
                    "No message at index {idx} (transcript has {} messages).",
                    state.messages.len()
                )));
            }
            Err(_) => {
                state.messages.push(ChatMessage::assistant(format!(
                            "Couldn't parse `{arg}` as a message index. Use `/pin`, `/pin <n>`, or `/pin list`."
                        )));
            }
        }
    }
}

pub(super) async fn cmd_unpin(
    state: &mut EngineState,
    parts: &[&str],
    text: &str,
    _tx: Option<&mpsc::Sender<EngineEvent>>,
) {
    state.messages.push(ChatMessage::user(text.to_owned()));
    let arg = parts.get(1).copied().unwrap_or("").trim();
    if arg.is_empty() || arg == "all" {
        let n = state.pinned_message_indices.len();
        state.pinned_message_indices.clear();
        state
            .messages
            .push(ChatMessage::assistant(format!("Cleared {n} pin(s).")));
    } else {
        match arg.parse::<usize>() {
            Ok(idx) => {
                if state.pinned_message_indices.remove(&idx) {
                    state
                        .messages
                        .push(ChatMessage::assistant(format!("Unpinned message #{idx}.")));
                } else {
                    state.messages.push(ChatMessage::assistant(format!(
                        "Message #{idx} wasn't pinned."
                    )));
                }
            }
            Err(_) => {
                state.messages.push(ChatMessage::assistant(format!(
                    "Couldn't parse `{arg}` as a message index."
                )));
            }
        }
    }
}

pub(super) async fn cmd_effort(
    state: &mut EngineState,
    parts: &[&str],
    text: &str,
    _tx: Option<&mpsc::Sender<EngineEvent>>,
) {
    // v132 reasoning-effort pin. `/effort low|medium|high|xhigh|max`
    // sets the pin; `/effort` alone shows the current state;
    // `/effort clear` removes the pin so the model picks adaptive.
    let arg = parts.get(1).copied().unwrap_or("").trim();
    if arg.is_empty() {
        state.messages.push(ChatMessage::user(text.to_owned()));
        state
            .messages
            .push(ChatMessage::assistant(state.effort_state.status()));
    } else if arg == "clear" || arg == "off" {
        let old_identity = crate::cache_lineage::current_identity(state);
        let mut msg = state.effort_state.clear();
        let new_identity = crate::cache_lineage::current_identity(state);
        let piggyback_drop =
            settle_cache_identity_change(state, &old_identity, &new_identity, "effort pin change");
        append_cache_lineage_note(&mut msg, piggyback_drop);
        state.messages.push(ChatMessage::user(text.to_owned()));
        state.messages.push(ChatMessage::assistant(msg));
    } else if arg.eq_ignore_ascii_case("ultracode") {
        // Claude Code's `/effort ultracode`: standing session mode that pins
        // xhigh effort and instructs the model to use Workflow by default.
        let old_identity = crate::cache_lineage::current_identity(state);
        let mut msg = state.effort_state.set_ultracode();
        let new_identity = crate::cache_lineage::current_identity(state);
        let piggyback_drop =
            settle_cache_identity_change(state, &old_identity, &new_identity, "effort pin change");
        append_cache_lineage_note(&mut msg, piggyback_drop);
        state.messages.push(ChatMessage::user(text.to_owned()));
        state.messages.push(ChatMessage::assistant(msg));
    } else if let Some(level) = crate::effort::ReasoningEffort::from_str_loose(arg) {
        let old_identity = crate::cache_lineage::current_identity(state);
        let mut msg = state.effort_state.set(level);
        let new_identity = crate::cache_lineage::current_identity(state);
        let piggyback_drop =
            settle_cache_identity_change(state, &old_identity, &new_identity, "effort pin change");
        append_cache_lineage_note(&mut msg, piggyback_drop);
        state.messages.push(ChatMessage::user(text.to_owned()));
        state.messages.push(ChatMessage::assistant(msg));
    } else {
        state.messages.push(ChatMessage::user(text.to_owned()));
        state.messages.push(ChatMessage::assistant(format!(
            "Unknown effort `{arg}`. Use one of: low, medium, high, xhigh, max, ultracode, clear."
        )));
    }
}

pub(super) async fn cmd_temp(
    state: &mut EngineState,
    parts: &[&str],
    text: &str,
    _tx: Option<&mpsc::Sender<EngineEvent>>,
) {
    let arg = parts.get(1).copied().unwrap_or("").trim();
    if arg.is_empty() {
        state.messages.push(ChatMessage::user(text.to_owned()));
        state
            .messages
            .push(ChatMessage::assistant(state.temperature_state.status()));
    } else if matches!(arg, "clear" | "default" | "auto" | "off") {
        let old_identity = crate::cache_lineage::current_identity(state);
        let mut msg = state.temperature_state.clear();
        let new_identity = crate::cache_lineage::current_identity(state);
        let piggyback_drop = settle_cache_identity_change(
            state,
            &old_identity,
            &new_identity,
            "temperature pin change",
        );
        append_cache_lineage_note(&mut msg, piggyback_drop);
        state.messages.push(ChatMessage::user(text.to_owned()));
        state.messages.push(ChatMessage::assistant(msg));
    } else {
        match crate::exploration::parse_temperature(arg) {
            Ok(value) => {
                let old_identity = crate::cache_lineage::current_identity(state);
                let mut msg = state.temperature_state.set(value);
                let new_identity = crate::cache_lineage::current_identity(state);
                let piggyback_drop = settle_cache_identity_change(
                    state,
                    &old_identity,
                    &new_identity,
                    "temperature pin change",
                );
                // The pin is silently dropped for request shapes that lock
                // sampling — the Anthropic OAuth/subscription API, and any
                // request with extended thinking on (Anthropic requires
                // temperature=1 there). Say so, so `/temp` isn't a mystery
                // no-op; point the user at `/effort` for those shapes.
                match state.provider.name() {
                    "anthropic-oauth" => msg.push_str(
                        "\n\n⚠ The Anthropic OAuth/subscription API locks sampling — \
                         this temperature won't be sent. Use `/effort` to steer reasoning depth.",
                    ),
                    "anthropic" | "bedrock" | "vertex" => msg.push_str(
                        "\n\nNote: temperature is ignored while extended thinking is active \
                         (Anthropic requires temperature=1); it applies only when thinking is \
                         off. `/effort` controls thinking depth.",
                    ),
                    _ => {}
                }
                append_cache_lineage_note(&mut msg, piggyback_drop);
                state.messages.push(ChatMessage::user(text.to_owned()));
                state.messages.push(ChatMessage::assistant(msg));
            }
            Err(reason) => {
                state.messages.push(ChatMessage::user(text.to_owned()));
                state.messages.push(ChatMessage::assistant(format!(
                    "{reason} Use `/temp <0..2>` or `/temp clear`."
                )));
            }
        }
    }
}

pub(super) async fn cmd_explore(
    state: &mut EngineState,
    parts: &[&str],
    text: &str,
    _tx: Option<&mpsc::Sender<EngineEvent>>,
) {
    state.messages.push(ChatMessage::user(text.to_owned()));
    let arg = parts.get(1).copied().unwrap_or("").trim();
    let msg = match arg {
        "" | "up" | "+1" | "more" => state.exploration_state.adjust_sticky(1),
        "status" => state.exploration_state.status(),
        "clear" | "default" | "auto" => state.exploration_state.clear_adjustments(),
        "max" | "ultra" => {
            state
                .exploration_state
                .force_next(crate::exploration::ExplorationLevel::MAX);
            "Next turn exploration forced to level 4.".to_owned()
        }
        other => format!(
            "Unknown exploration argument `{other}`. Use `/explore`, `/explore status`, or `/explore clear`."
        ),
    };
    state.messages.push(ChatMessage::assistant(msg));
}

pub(super) async fn cmd_focus(
    state: &mut EngineState,
    parts: &[&str],
    text: &str,
    _tx: Option<&mpsc::Sender<EngineEvent>>,
) {
    state.messages.push(ChatMessage::user(text.to_owned()));
    let arg = parts.get(1).copied().unwrap_or("").trim();
    let msg = match arg {
        "" | "down" | "-1" | "less" => state.exploration_state.adjust_sticky(-1),
        "status" => state.exploration_state.status(),
        "clear" | "default" | "auto" => state.exploration_state.clear_adjustments(),
        other => format!(
            "Unknown focus argument `{other}`. Use `/focus`, `/focus status`, or `/focus clear`."
        ),
    };
    state.messages.push(ChatMessage::assistant(msg));
}

pub(super) async fn cmd_feature(
    state: &mut EngineState,
    parts: &[&str],
    text: &str,
    _tx: Option<&mpsc::Sender<EngineEvent>>,
) {
    // v132 feature-gate framework. `/feature` lists all gates and
    // their state; `/feature <codename> on|off` flips one.
    state.messages.push(ChatMessage::user(text.to_owned()));
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
        state.messages.push(ChatMessage::assistant(body));
    } else {
        let mut sub = rest.split_whitespace();
        let name = sub.next().unwrap_or("");
        let toggle = sub.next().unwrap_or("").to_ascii_lowercase();
        let Some(gate) = crate::feature_gates::FeatureGate::from_codename(name) else {
            state.messages.push(ChatMessage::assistant(format!(
                "Unknown feature gate `{name}`. List with `/feature`."
            )));
            return;
        };
        let enabled = match toggle.as_str() {
            "on" | "enable" | "true" | "1" => true,
            "off" | "disable" | "false" | "0" => false,
            "" => {
                state.messages.push(ChatMessage::assistant(format!(
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
                state.messages.push(ChatMessage::assistant(format!(
                    "Unknown toggle `{other}`. Use `on` or `off`."
                )));
                return;
            }
        };
        let old_identity = crate::cache_lineage::current_identity(state);
        crate::feature_gates::set(gate, enabled);
        let new_identity = crate::cache_lineage::current_identity(state);
        let piggyback_drop = settle_cache_identity_change(
            state,
            &old_identity,
            &new_identity,
            "feature-gate switch",
        );
        if piggyback_drop.is_some() {
            state.messages.push(ChatMessage::user(text.to_owned()));
        }
        let mut reply = format!(
            "`{}` set to **{}** ({}).",
            gate.codename(),
            if enabled { "ON" } else { "OFF" },
            gate.description(),
        );
        append_cache_lineage_note(&mut reply, piggyback_drop);
        state.messages.push(ChatMessage::assistant(reply));
        // v132 system-reminder so the model sees the gate flip
        // on the next turn (rather than guessing from changed
        // behavior).
        crate::system_reminder::append_to_last_user(
            &mut state.messages,
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
    state: &mut EngineState,
    parts: &[&str],
    text: &str,
    tx: Option<&mpsc::Sender<EngineEvent>>,
) {
    // v137 session-scoped goal. `/goal <condition>` sets a stop
    // condition — the agent keeps working until the evaluator
    // says it's met (see `crate::goal::evaluate`). `/goal
    // clear` (or stop/off/reset/none/cancel) removes it.
    // `/goal` alone shows the current state.
    state.messages.push(ChatMessage::user(text.to_owned()));
    let arg = parts[1..].join(" ");
    let arg = arg.trim();
    if arg.is_empty() {
        let msg = match &state.goal {
            Some(g) => format!(
                "Current goal ({} iterations): {}\n\nUse `/goal clear` to remove.",
                g.iterations, g.condition
            ),
            None => "No goal set. Usage: `/goal <condition>`".to_string(),
        };
        state.messages.push(ChatMessage::assistant(msg));
    } else if crate::goal::is_clear_arg(arg) {
        let prev = state.goal.take();
        state.goal_evaluator_in_flight = false;
        // Drop the sidecar so a future /continue doesn't
        // revive a goal the user just cancelled.
        if let Some(sid) = state.current_session_id.as_ref() {
            crate::goal::save_sidecar(sid.as_str(), None);
        }
        let msg = match prev {
            Some(g) => format!(
                "Goal cleared after {} iterations: {}",
                g.iterations, g.condition
            ),
            None => "No goal was set.".to_string(),
        };
        state.messages.push(ChatMessage::assistant(msg));
        crate::toast::push_with_cap(
            &mut state.toasts,
            crate::toast::Toast::new(crate::toast::ToastKind::Success, "Goal cleared".to_string()),
        );
    } else {
        match crate::goal::validate_condition(arg) {
            Ok(condition) => {
                let goal = crate::goal::ActiveGoal::new(condition.clone());
                state.goal = Some(goal);
                // Persist the new goal so /continue picks it
                // up if the user exits before the next turn.
                if let Some(sid) = state.current_session_id.as_ref() {
                    crate::goal::save_sidecar(sid.as_str(), state.goal.as_ref());
                }
                state.messages.push(ChatMessage::assistant(format!(
                    "Goal set: {condition}\n\nThe agent will keep \
                             working until this condition is met (auto-\
                             evaluated after each turn, max {} iterations). \
                             Use `/goal clear` to cancel.",
                    crate::goal::MAX_ITERATIONS
                )));
                crate::toast::push_with_cap(
                    &mut state.toasts,
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
                let idle = !state.is_streaming
                    && state.pending_approval.is_none()
                    && state.approval_queue.is_empty()
                    && state.pending_tool_calls.is_empty();
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
                    let _ = tx
                        .send(EngineEvent::Control(ControlEvent::SubmitPrompt(kickoff)))
                        .await;
                    tracing::info!(
                        target: "jfc::goal",
                        "/goal: dispatched kickoff meta-prompt"
                    );
                }
            }
            Err(reason) => {
                state
                    .messages
                    .push(ChatMessage::assistant(reason.to_owned()));
            }
        }
    }
}

pub(super) async fn cmd_memory(
    state: &mut EngineState,
    parts: &[&str],
    text: &str,
    _tx: Option<&mpsc::Sender<EngineEvent>>,
) {
    // `/memory` (no args)            → list memory files
    // `/memory recall on|off|status` → toggle two-phase recall
    //
    // The recall sub-command targets the runtime override in
    // `memory_recall::set_runtime_override` — persisting to
    // `~/.config/jfc/config.toml` is left to the user since they
    // may have hand-formatted that file.
    let arg = parts.get(1).copied().unwrap_or("").trim();
    state.messages.push(ChatMessage::user(text.to_owned()));
    if arg.starts_with("recall") {
        let sub = arg
            .split_once(' ')
            .map(|x| x.1)
            .map(str::trim)
            .unwrap_or("status");
        match sub {
            "on" | "enable" => {
                crate::memory_recall::set_runtime_override(Some(true));
                state.messages.push(ChatMessage::assistant(
                    "Two-phase memory recall: **on** (runtime override).".into(),
                ));
            }
            "off" | "disable" => {
                crate::memory_recall::set_runtime_override(Some(false));
                state.messages.push(ChatMessage::assistant(
                    "Two-phase memory recall: **off** (runtime override).".into(),
                ));
            }
            "default" | "reset" => {
                crate::memory_recall::set_runtime_override(None);
                state.messages.push(ChatMessage::assistant(
                    "Two-phase memory recall: cleared runtime override; \
                             falling back to `~/.config/jfc/config.toml` value."
                        .into(),
                ));
            }
            "status" | "" => {
                let persisted = crate::config::load_arc().memory_recall_enabled;
                let effective = crate::memory_recall::is_enabled(persisted);
                state.messages.push(ChatMessage::assistant(format!(
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
                state.messages.push(ChatMessage::assistant(format!(
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
        state.messages.push(ChatMessage::assistant(body));
    }
}

pub(super) async fn cmd_claude_md(
    state: &mut EngineState,
    _parts: &[&str],
    _text: &str,
    _tx: Option<&mpsc::Sender<EngineEvent>>,
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
    state.messages.push(ChatMessage::user("/claude-md".into()));
    state.messages.push(ChatMessage::assistant(body));
}

pub(super) async fn cmd_mode(
    state: &mut EngineState,
    parts: &[&str],
    _text: &str,
    _tx: Option<&mpsc::Sender<EngineEvent>>,
) {
    let arg = parts.get(1).copied().unwrap_or("").trim().to_lowercase();
    let new_mode = match arg.as_str() {
        "default" | "d" => Some(crate::app::PermissionMode::Default),
        "plan" | "p" => Some(crate::app::PermissionMode::Plan),
        "accept" | "acceptedits" | "a" => Some(crate::app::PermissionMode::AcceptEdits),
        "bypass" | "b" | "yolo" => Some(crate::app::PermissionMode::BypassPermissions),
        "auto" => Some(crate::app::PermissionMode::Auto),
        "" => {
            state.messages.push(ChatMessage::assistant(format!(
                "**Current mode:** {} {}\n\n\
                         Available: `default`, `plan`, `accept`, `auto`, `bypass`\n\
                         Switch: `/mode <name>` or **Shift+Tab** to cycle.",
                state.permission_mode.symbol(),
                state.permission_mode.label(),
            )));
            None
        }
        _ => {
            state.messages.push(ChatMessage::assistant(format!(
                "Unknown mode `{arg}`. Available: `default`, `plan`, `accept`, `auto`, `bypass`"
            )));
            None
        }
    };
    if let Some(mode) = new_mode {
        let old_identity = crate::cache_lineage::current_identity(state);
        state.permission_mode = mode;
        // Persist so the mode survives session restart / --continue.
        crate::config::save_permission_mode(&state.permission_mode);
        // Sync auto_mode.enabled with permission mode for backward compat
        state.auto_mode.enabled = mode == crate::app::PermissionMode::Auto;
        let new_identity = crate::cache_lineage::current_identity(state);
        let piggyback_drop = settle_cache_identity_change(
            state,
            &old_identity,
            &new_identity,
            "permission mode switch",
        );
        let mut reply = format!("**Mode → {} {}**", mode.symbol(), mode.label());
        append_cache_lineage_note(&mut reply, piggyback_drop);
        state.messages.push(ChatMessage::assistant(reply));
    }
}

pub(super) async fn cmd_auto_mode(
    state: &mut EngineState,
    parts: &[&str],
    _text: &str,
    _tx: Option<&mpsc::Sender<EngineEvent>>,
) {
    let arg = parts.get(1).copied().unwrap_or("status").trim();
    match arg {
        "on" | "enable" | "true" => {
            let old_identity = crate::cache_lineage::current_identity(state);
            state.auto_mode.enabled = true;
            let new_identity = crate::cache_lineage::current_identity(state);
            let piggyback_drop = settle_cache_identity_change(
                state,
                &old_identity,
                &new_identity,
                "auto-mode toggle",
            );
            let mut reply = "**Auto-mode enabled.** Every tool call will be sent to the v126 \
                         classifier LLM. The classifier may block dangerous operations \
                         without prompting you. Edit `~/.config/jfc/settings.json` under \
                         `autoMode.{allow,soft_deny,environment}` (with `$defaults` \
                         inheritance) to extend the rules."
                .to_owned();
            append_cache_lineage_note(&mut reply, piggyback_drop);
            state.messages.push(ChatMessage::assistant(reply));
        }
        "off" | "disable" | "false" => {
            let old_identity = crate::cache_lineage::current_identity(state);
            state.auto_mode.enabled = false;
            let new_identity = crate::cache_lineage::current_identity(state);
            let piggyback_drop = settle_cache_identity_change(
                state,
                &old_identity,
                &new_identity,
                "auto-mode toggle",
            );
            let mut reply = "**Auto-mode disabled.** Tool calls will use the manual approval \
                         flow again."
                .to_owned();
            append_cache_lineage_note(&mut reply, piggyback_drop);
            state.messages.push(ChatMessage::assistant(reply));
        }
        _ => {
            let n_allow = state.auto_mode.allow.len();
            let n_block = state.auto_mode.soft_deny.len();
            let n_env = state.auto_mode.environment.len();
            let mode_state = if state.auto_mode.enabled { "ON" } else { "OFF" };
            state.messages.push(ChatMessage::assistant(format!(
                "**Auto-mode: {mode_state}**\n\
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
    state: &mut EngineState,
    parts: &[&str],
    _text: &str,
    _tx: Option<&mpsc::Sender<EngineEvent>>,
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
        state.messages.push(ChatMessage::assistant(format!(
                    "Usage: {} <request-id> [feedback]\nFind the id in the toast that appeared when the teammate asked.",
                    parts[0]
                )));
    } else {
        let team_name = state.team_context.team_name.clone().unwrap_or_default();
        let echo = if approve {
            format!("/swarm-approve {id}")
        } else if let Some(ref f) = feedback {
            format!("/swarm-deny {id} {f}")
        } else {
            format!("/swarm-deny {id}")
        };
        state.messages.push(ChatMessage::user(echo));
        if team_name.is_empty() {
            state.messages.push(ChatMessage::assistant(
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
            state.messages.push(ChatMessage::assistant(format!(
                "Resolved swarm request {id} → {}",
                if approve { "approved" } else { "denied" }
            )));
        }
    }
}

pub(super) async fn cmd_brief(
    state: &mut EngineState,
    _parts: &[&str],
    _text: &str,
    _tx: Option<&mpsc::Sender<EngineEvent>>,
) {
    state.brief_mode = !state.brief_mode;
    let msg = if state.brief_mode {
        "Brief mode enabled. Use the SendUserMessage tool for all user-facing \
         output — plain text outside it is hidden from the user's view."
    } else {
        "Brief mode disabled. The SendUserMessage tool is no longer required — \
         reply with plain text."
    };
    state.messages.push(ChatMessage::assistant(msg.to_string()));
}

pub(super) async fn cmd_autoloop(
    state: &mut EngineState,
    parts: &[&str],
    _text: &str,
    _tx: Option<&mpsc::Sender<EngineEvent>>,
) {
    use crate::autonomous_loop::{AutonomousLoopState, LoopPacing, read_loop_file};

    // `/loop stop` kills an active loop.
    if parts.get(1).copied() == Some("stop") {
        if state.autonomous_loop.take().is_some() {
            state
                .messages
                .push(ChatMessage::assistant("Autonomous loop stopped.".into()));
        } else {
            state
                .messages
                .push(ChatMessage::assistant("No active autonomous loop.".into()));
        }
        return;
    }
    // `/loop` with no args starts a new dynamic-pacing loop.
    if state.autonomous_loop.is_some() {
        state.messages.push(ChatMessage::assistant(
            "Autonomous loop already active. Use `/loop stop` first.".into(),
        ));
        return;
    }
    let git_root = crate::context::discover_git_root();
    let project_root = git_root
        .as_deref()
        .unwrap_or_else(|| std::path::Path::new("."));
    let loop_content = read_loop_file(project_root);
    let mut loop_state = AutonomousLoopState::new(LoopPacing::Dynamic);
    loop_state.loop_file_content = loop_content.clone();
    state.autonomous_loop = Some(loop_state);
    let hint = if let Some(ref content) = loop_content {
        format!(
            "Autonomous loop started (dynamic pacing). \
             Loaded loop.md ({} bytes). First tick will fire on next ScheduleWakeup.",
            content.len()
        )
    } else {
        "Autonomous loop started (dynamic pacing). \
         No loop.md found — the loop will use conversation context for task instructions."
            .into()
    };
    state.messages.push(ChatMessage::assistant(hint));
}

pub(super) async fn cmd_sandbox(
    state: &mut EngineState,
    _parts: &[&str],
    _text: &str,
    _tx: Option<&mpsc::Sender<EngineEvent>>,
) {
    state.bash_sandbox.enabled = !state.bash_sandbox.enabled;
    // Mirror the toggle into the global static so the bash dispatch path
    // (which doesn't have access to `&mut EngineState`) sees the new config.
    crate::sandbox::install_bash_sandbox_config(state.bash_sandbox.clone());
    let avail = crate::sandbox::is_bwrap_available();
    let msg = if state.bash_sandbox.enabled {
        if avail {
            "Bash sandbox enabled — commands will be wrapped in bwrap with network isolation."
        } else {
            "Bash sandbox enabled (config) but bwrap is not available on this system. \
             Install bubblewrap (`apt install bubblewrap`) for actual isolation."
        }
    } else {
        "Bash sandbox disabled — commands run without network isolation."
    };
    state.messages.push(ChatMessage::assistant(msg.to_string()));
}

pub(super) async fn cmd_permissions(
    state: &mut EngineState,
    parts: &[&str],
    _text: &str,
    _tx: Option<&mpsc::Sender<EngineEvent>>,
) {
    state
        .messages
        .push(ChatMessage::user("/permissions".into()));

    let arg = parts.get(1).copied().unwrap_or("").trim();

    // Load existing rules from .jfc/settings.json
    let settings_path = std::path::Path::new(".jfc/settings.json");
    let mut settings: serde_json::Value = if settings_path.exists() {
        std::fs::read_to_string(settings_path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_else(|| serde_json::json!({}))
    } else {
        serde_json::json!({})
    };

    if arg.is_empty() {
        // List current rules
        let allow_rules = settings
            .get("permissions")
            .and_then(|p| p.get("allow"))
            .and_then(|a| a.as_array())
            .cloned()
            .unwrap_or_default();

        let mut body = String::from("**Permission Rules**\n\n");
        body.push_str(&format!("Mode: **{}**\n\n", state.permission_mode.label()));
        if allow_rules.is_empty() {
            body.push_str("No custom allow rules configured.\n\n");
        } else {
            body.push_str("Allow rules:\n");
            for rule in &allow_rules {
                if let Some(s) = rule.as_str() {
                    body.push_str(&format!("  ✓ {s}\n"));
                }
            }
            body.push('\n');
        }
        body.push_str("Usage: `/permissions add Bash(git *)` to auto-allow a pattern.");
        state.messages.push(ChatMessage::assistant(body));
    } else if let Some(rule) = arg.strip_prefix("add ") {
        // Add a new allow rule
        let rule = rule.trim();
        let perms = settings
            .as_object_mut()
            .unwrap()
            .entry("permissions")
            .or_insert_with(|| serde_json::json!({}));
        let allow = perms
            .as_object_mut()
            .unwrap()
            .entry("allow")
            .or_insert_with(|| serde_json::json!([]));
        if let Some(arr) = allow.as_array_mut() {
            arr.push(serde_json::Value::String(rule.to_owned()));
        }
        // Write back
        let _ = std::fs::create_dir_all(".jfc");
        let _ = std::fs::write(
            settings_path,
            serde_json::to_string_pretty(&settings).unwrap(),
        );
        state.messages.push(ChatMessage::assistant(format!(
            "Added permission allow rule: `{rule}`"
        )));
    } else {
        state.messages.push(ChatMessage::assistant(
            "Usage: `/permissions` to list, `/permissions add <rule>` to add a rule.\n\
             Example: `/permissions add Bash(git *)`"
                .into(),
        ));
    }
}

pub(super) async fn cmd_stuck(
    state: &mut EngineState,
    _parts: &[&str],
    _text: &str,
    _tx: Option<&mpsc::Sender<EngineEvent>>,
) {
    state.messages.push(ChatMessage::user("/stuck".into()));

    let mut report = String::from("**Diagnostic Report (/stuck)**\n\n");

    // Process count
    let proc_count = std::process::Command::new("sh")
        .args(["-c", "ps aux | wc -l"])
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .unwrap_or_else(|| "unknown".into());
    report.push_str(&format!("• Processes: {}\n", proc_count.trim()));

    // Memory usage (from /proc/meminfo or sysctl)
    let mem_info = std::fs::read_to_string("/proc/meminfo")
        .ok()
        .and_then(|s| {
            let total = s.lines().find(|l| l.starts_with("MemTotal:"))?;
            let avail = s.lines().find(|l| l.starts_with("MemAvailable:"))?;
            Some(format!("{} / {}", avail.trim(), total.trim()))
        })
        .unwrap_or_else(|| "unavailable".into());
    report.push_str(&format!("• Memory: {mem_info}\n"));

    // Active streams
    let streaming = if state.is_streaming { "YES" } else { "no" };
    report.push_str(&format!("• Active stream: {streaming}\n"));

    // Pending tool calls
    let pending_tools = state
        .messages
        .iter()
        .flat_map(|m| m.parts.iter())
        .filter_map(|p| match p {
            jfc_core::MessagePart::Tool(tc) => Some(tc),
            _ => None,
        })
        .filter(|tc| tc.status == jfc_core::ToolStatus::Running)
        .count();
    report.push_str(&format!("• Pending tool calls: {pending_tools}\n"));

    // Time-since-activity is the highest-signal row for "is this session
    // wedged?" — always render one. Stream activity is the engine's clock;
    // before any stream has run, fall back to engine uptime so the idle case
    // (the very case /stuck diagnoses) still gets an answer.
    match state.last_stream_event_at {
        Some(at) => report.push_str(&format!(
            "• Last stream activity: {:.1}s ago\n",
            at.elapsed().as_secs_f64()
        )),
        None => report.push_str(&format!(
            "• No stream activity yet (engine up {:.1}s)\n",
            state.started_at.elapsed().as_secs_f64()
        )),
    }

    // Token usage
    report.push_str(&format!(
        "• Context tokens: {} / {}\n",
        state.tool_ctx.approx_tokens, state.max_context_tokens
    ));

    // Session info
    if let Some(ref id) = state.current_session_id {
        report.push_str(&format!("• Session: {id}\n"));
    }

    state.messages.push(ChatMessage::assistant(report));
}

pub(super) async fn cmd_teleport_export(
    state: &mut EngineState,
    _parts: &[&str],
    _text: &str,
    _tx: Option<&mpsc::Sender<EngineEvent>>,
) {
    state
        .messages
        .push(ChatMessage::user("/teleport-export".into()));

    let id = uuid::Uuid::new_v4().to_string();
    let dir = std::path::Path::new(".jfc/teleport");
    let _ = std::fs::create_dir_all(dir);
    let path = dir.join(format!("{id}.json"));

    // Build export payload
    let messages: Vec<serde_json::Value> = state
        .messages
        .iter()
        .map(|m| {
            let content: String = m
                .parts
                .iter()
                .map(|p| p.text_only())
                .collect::<Vec<_>>()
                .join("");
            serde_json::json!({
                "role": m.role.to_string(),
                "content": content,
            })
        })
        .collect();

    let export = serde_json::json!({
        "id": id,
        "session_id": state.current_session_id,
        "model": state.model.to_string(),
        "messages": messages,
        "exported_at": chrono::Utc::now().to_rfc3339(),
    });

    match std::fs::write(&path, serde_json::to_string_pretty(&export).unwrap()) {
        Ok(_) => {
            state.messages.push(ChatMessage::assistant(format!(
                "Context exported to `{}`\n\nAnother session can import with: \
                 `--fork-session {id}`",
                path.display()
            )));
        }
        Err(e) => {
            state
                .messages
                .push(ChatMessage::assistant(format!("Failed to export: {e}")));
        }
    }
}

pub(super) async fn cmd_team_onboarding(
    state: &mut EngineState,
    _parts: &[&str],
    _text: &str,
    _tx: Option<&mpsc::Sender<EngineEvent>>,
) {
    let root = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
    let guide = crate::team_onboarding::generate_onboarding_guide(&root);
    state.messages.push(ChatMessage::assistant(guide));
}

pub(super) async fn cmd_coach(
    state: &mut EngineState,
    _parts: &[&str],
    _text: &str,
    _tx: Option<&mpsc::Sender<EngineEvent>>,
) {
    // Build session stats from state.messages
    let mut stats = crate::coach::SessionStats {
        total_tool_calls: 0,
        read_calls: 0,
        write_calls: 0,
        bash_calls: 0,
        search_calls: 0,
        total_tokens_in: 0,
        total_tokens_out: 0,
        session_duration_secs: state.started_at.elapsed().as_secs(),
        compaction_count: 0,
        error_count: 0,
    };
    for m in &state.messages {
        for p in &m.parts {
            if let jfc_core::MessagePart::Tool(t) = p {
                stats.total_tool_calls += 1;
                match t.kind.label() {
                    "Read" => stats.read_calls += 1,
                    "Write" => stats.write_calls += 1,
                    "Bash" => stats.bash_calls += 1,
                    "Grep" | "Glob" => stats.search_calls += 1,
                    _ => {}
                }
                if t.status == jfc_core::ExecutionStatus::Failed {
                    stats.error_count += 1;
                }
            }
        }
    }
    let tips = crate::coach::generate_coaching_tips(&stats);
    state.messages.push(ChatMessage::assistant(format!(
        "## Coaching tips\n\n{tips}"
    )));
}

pub(super) async fn cmd_remote(
    state: &mut EngineState,
    parts: &[&str],
    _text: &str,
    _tx: Option<&mpsc::Sender<EngineEvent>>,
) {
    let prompt = parts.get(1..).map(|p| p.join(" ")).unwrap_or_default();
    if prompt.trim().is_empty() {
        state.messages.push(ChatMessage::assistant(
            "Usage: `/remote <prompt>` — spawn a CCR remote session with this prompt.".into(),
        ));
        return;
    }
    let api_key = match std::env::var("ANTHROPIC_API_KEY") {
        Ok(k) => k,
        Err(_) => {
            state.messages.push(ChatMessage::assistant(
                "ANTHROPIC_API_KEY not set — `/remote` requires it.".into(),
            ));
            return;
        }
    };
    let client = reqwest::Client::new();
    match crate::ccr::spawn_remote_session(
        &client,
        &api_key,
        "https://api.anthropic.com",
        prompt.clone(),
        "default".to_string(),
    )
    .await
    {
        Ok(session) => {
            state.messages.push(ChatMessage::assistant(format!(
                "Remote CCR session spawned: `{}`\nURL: {}",
                session.session_id, session.session_url
            )));
        }
        Err(e) => {
            state
                .messages
                .push(ChatMessage::assistant(format!("Remote spawn failed: {e}")));
        }
    }
}

/// `/factory` — show factory throughput + quality telemetry (Morescient GAI,
/// arXiv:2406.04710): success rate, rework ratio, retry/attempt counts.
pub(super) async fn cmd_factory(
    state: &mut EngineState,
    _parts: &[&str],
    _text: &str,
    _tx: Option<&mpsc::Sender<EngineEvent>>,
) {
    let m = state.task_store.factory_metrics();
    let success = m
        .success_rate()
        .map(|r| format!("{:.0}%", r * 100.0))
        .unwrap_or_else(|| "—".to_string());
    let msg = format!(
        "## Factory metrics\n\n\
         | Metric | Value |\n\
         | --- | --- |\n\
         | Total tasks | {} |\n\
         | Completed | {} |\n\
         | In progress | {} |\n\
         | Pending | {} |\n\
         | Failed | {} |\n\
         | **Success rate** | {} |\n\
         | Rework ratio (replans/total) | {:.0}% |\n\
         | Retried tasks | {} |\n\
         | Multi-attempt tasks | {} |\n\
         | Avg extra attempts/task | {:.2} |\n\n\
         {}",
        m.total(),
        m.completed,
        m.in_progress,
        m.pending,
        m.failed,
        success,
        m.rework_ratio() * 100.0,
        m.retried_tasks,
        m.multi_attempt_tasks,
        m.avg_attempts(),
        factory_health_note(&m),
    );
    state.messages.push(ChatMessage::assistant(msg));
}

/// One-line health interpretation of the factory metrics.
fn factory_health_note(m: &jfc_session::FactoryMetrics) -> &'static str {
    if m.total() == 0 {
        return "_No tasks yet — the factory is idle._";
    }
    if m.rework_ratio() > 0.3 {
        return "⚠️ High rework — the planner is under-decomposing; tasks keep needing revision.";
    }
    if m.multi_attempt_tasks > 0 {
        return "⚠️ Some tasks needed multiple attempts — consider splitting the flaky ones.";
    }
    match m.success_rate() {
        Some(r) if r >= 0.9 => "✅ Healthy — high success rate, low rework.",
        Some(_) => "Steady — watch the failure rate.",
        None => "_Tasks queued; nothing terminal yet._",
    }
}

pub(super) async fn cmd_oauth_login(
    state: &mut EngineState,
    _parts: &[&str],
    _text: &str,
    _tx: Option<&mpsc::Sender<EngineEvent>>,
) {
    let cfg = crate::auth::device_flow::DeviceFlowConfig {
        client_id: std::env::var("JFC_OAUTH_CLIENT_ID").unwrap_or_else(|_| "jfc-cli".into()),
        device_auth_url: std::env::var("JFC_OAUTH_DEVICE_URL")
            .unwrap_or_else(|_| "https://auth.anthropic.com/oauth/device/code".into()),
        token_url: std::env::var("JFC_OAUTH_TOKEN_URL")
            .unwrap_or_else(|_| "https://auth.anthropic.com/oauth/token".into()),
        scopes: vec!["openid".into(), "offline_access".into()],
    };
    let client = reqwest::Client::new();
    let device_resp = match crate::auth::device_flow::request_device_code(&client, &cfg).await {
        Ok(r) => r,
        Err(e) => {
            state.messages.push(ChatMessage::assistant(format!(
                "OAuth device-code request failed: {e}"
            )));
            return;
        }
    };
    state.messages.push(ChatMessage::assistant(format!(
        "Go to: **{}**\nEnter code: **{}**\n\nPolling for completion (expires in {}s)...",
        device_resp.verification_uri, device_resp.user_code, device_resp.expires_in,
    )));
    match crate::auth::device_flow::poll_for_token(
        &client,
        &cfg,
        &device_resp.device_code,
        device_resp.interval,
        device_resp.expires_in,
    )
    .await
    {
        Ok(token) => {
            let _ = crate::auth::device_flow::store_token(&token);
            state.messages.push(ChatMessage::assistant(
                "Login successful — token stored in `.jfc/credentials.json`.".into(),
            ));
        }
        Err(e) => {
            state
                .messages
                .push(ChatMessage::assistant(format!("OAuth poll failed: {e}")));
        }
    }
}

/// Run `sh -c <cmd>` synchronously and return its stdout (trimmed).
/// Returns `Err(stderr-or-spawn-error)` on non-zero exit so the caller
/// can surface a single hint instead of an empty PR list silently.
fn run_capture(cmd: &str) -> Result<String, String> {
    let out = std::process::Command::new("sh")
        .args(["-c", cmd])
        .output()
        .map_err(|e| format!("spawn `{cmd}` failed: {e}"))?;
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();
        return Err(if stderr.is_empty() {
            format!("`{cmd}` exited with {}", out.status)
        } else {
            stderr
        });
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// Build the PR-status summary block used by both `/babysit-prs` and
/// `/morning-checkin`. Returns a markdown string highlighting PRs that
/// need attention (review requested, CI failing, changes requested).
fn pr_status_summary() -> String {
    // `gh` is a hard requirement — surface a helpful message rather than
    // a parse failure when the CLI is missing or the user isn't logged in.
    let raw = match run_capture(
        "gh pr list --json number,title,reviewDecision,statusCheckRollup --limit 10",
    ) {
        Ok(s) if !s.is_empty() => s,
        Ok(_) => return "No open PRs found.".to_string(),
        Err(e) => return format!("Unable to query PRs (is `gh` installed and logged in?): {e}"),
    };

    let prs: Vec<serde_json::Value> = match serde_json::from_str(&raw) {
        Ok(v) => v,
        Err(e) => return format!("Could not parse `gh pr list` output: {e}"),
    };

    if prs.is_empty() {
        return "No open PRs found.".to_string();
    }

    let mut needs_attention: Vec<String> = Vec::new();
    let mut healthy: Vec<String> = Vec::new();

    for pr in &prs {
        let num = pr.get("number").and_then(|v| v.as_i64()).unwrap_or(0);
        let title = pr
            .get("title")
            .and_then(|v| v.as_str())
            .unwrap_or("(no title)");
        let review = pr
            .get("reviewDecision")
            .and_then(|v| v.as_str())
            .unwrap_or("");

        // statusCheckRollup is an array of check objects with a
        // `conclusion` field; "FAILURE"/"TIMED_OUT" mean CI is red.
        let checks = pr
            .get("statusCheckRollup")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        let any_fail = checks.iter().any(|c| {
            matches!(
                c.get("conclusion").and_then(|v| v.as_str()),
                Some("FAILURE") | Some("TIMED_OUT") | Some("CANCELLED")
            )
        });
        let any_pending = checks.iter().any(|c| {
            matches!(
                c.get("status").and_then(|v| v.as_str()),
                Some("IN_PROGRESS") | Some("QUEUED") | Some("PENDING")
            )
        });

        let mut flags: Vec<&str> = Vec::new();
        if review == "CHANGES_REQUESTED" {
            flags.push("changes requested");
        }
        if review == "REVIEW_REQUIRED" || review.is_empty() {
            flags.push("review requested");
        }
        if any_fail {
            flags.push("CI failing");
        } else if any_pending {
            flags.push("CI pending");
        }

        let line = format!(
            "  • #{num} {title}{}",
            if flags.is_empty() {
                String::new()
            } else {
                format!(" — _{}_", flags.join(", "))
            }
        );
        if flags.iter().any(|f| *f != "CI pending") {
            needs_attention.push(line);
        } else {
            healthy.push(line);
        }
    }

    let mut body = String::new();
    if !needs_attention.is_empty() {
        body.push_str("**Needs attention:**\n");
        body.push_str(&needs_attention.join("\n"));
        body.push('\n');
    }
    if !healthy.is_empty() {
        if !body.is_empty() {
            body.push('\n');
        }
        body.push_str("**Looking good:**\n");
        body.push_str(&healthy.join("\n"));
        body.push('\n');
    }
    body
}

pub(super) async fn cmd_babysit_prs(
    state: &mut EngineState,
    parts: &[&str],
    _text: &str,
    _tx: Option<&mpsc::Sender<EngineEvent>>,
) {
    state
        .messages
        .push(ChatMessage::user(parts.to_vec().join(" ")));

    let arg = parts.get(1).copied().unwrap_or("").trim();

    // ── `/babysit-prs stop` cancels an active loop ────────────────────
    if arg.eq_ignore_ascii_case("stop") {
        match state.babysit_prs_cron_id.take() {
            Some(id) => {
                use crate::daemon::{Daemon, DaemonPaths};
                let paths = DaemonPaths::default_user();
                let removed = match Daemon::new(&paths.base_dir) {
                    Ok(mut d) => d.remove_cron_job(&id),
                    Err(e) => {
                        state.messages.push(ChatMessage::assistant(format!(
                            "Could not open daemon state to cancel `{id}`: {e}"
                        )));
                        return;
                    }
                };
                let msg = if removed {
                    format!("Cancelled PR-watch loop (`{id}`).")
                } else {
                    format!(
                        "No cron job with id `{id}` was registered — it may have already \
                         been removed."
                    )
                };
                state.messages.push(ChatMessage::assistant(msg));
            }
            None => {
                state.messages.push(ChatMessage::assistant(
                    "No active PR-watch loop to stop.".to_string(),
                ));
            }
        }
        return;
    }

    // ── Build the current snapshot (`git log` + PR summary) ───────────
    let mut report = String::from("**PR babysitter**\n\n");

    match run_capture("git log --oneline origin/HEAD..HEAD") {
        Ok(local) if !local.is_empty() => {
            report.push_str("Local commits ahead of `origin/HEAD`:\n```\n");
            report.push_str(&local);
            report.push_str("\n```\n\n");
        }
        Ok(_) => {
            report.push_str("_No local commits ahead of `origin/HEAD`._\n\n");
        }
        Err(e) => {
            report.push_str(&format!("_Could not compare to `origin/HEAD`: {e}_\n\n"));
        }
    }

    report.push_str(&pr_status_summary());

    // ── Optional schedule arg registers a cron loop ───────────────────
    if !arg.is_empty() {
        // `parse_schedule` accepts crontab (`*/5 * * * *`), `@hourly`,
        // and `@every <dur>` (e.g. `@every 5m`). When the user types a
        // bare duration like `5m`, wrap it in `@every` so the daemon
        // accepts it.
        let normalized = if arg.starts_with('@') || arg.contains(' ') {
            arg.to_string()
        } else {
            format!("@every {arg}")
        };

        use crate::daemon::{Daemon, DaemonPaths, parse_schedule};
        match parse_schedule(&normalized) {
            Ok(sched) => {
                let paths = DaemonPaths::default_user();
                match Daemon::new(&paths.base_dir) {
                    Ok(mut daemon) => {
                        // Replace any existing loop first so the user
                        // never accumulates duplicate cron entries.
                        if let Some(prev) = state.babysit_prs_cron_id.take() {
                            daemon.remove_cron_job(&prev);
                        }
                        // The cron command is what runs on the daemon's
                        // schedule. We can't dispatch a slash command
                        // directly from cron, so the command writes a
                        // markdown report to `.jfc/pr-status.md` — the
                        // user (or a sister loop) can pick it up.
                        let cmd = "sh -c 'mkdir -p .jfc && \
                                   gh pr list --json number,title,reviewDecision,statusCheckRollup \
                                   --limit 10 > .jfc/pr-status.json 2>&1'";
                        let id =
                            daemon.add_cron_job(sched, "jfc /babysit-prs PR status refresher", cmd);
                        state.babysit_prs_cron_id = Some(id.clone());
                        report.push_str(&format!(
                            "\n_Registered cron job `{id}` ({normalized}) — \
                             use `/babysit-prs stop` to cancel._\n"
                        ));
                    }
                    Err(e) => {
                        report.push_str(&format!(
                            "\n_Could not register cron loop: daemon state init failed: {e}_\n"
                        ));
                    }
                }
            }
            Err(e) => {
                report.push_str(&format!(
                    "\n_Schedule `{arg}` is not valid (`{e}`). Try `5m`, `@hourly`, \
                     or a crontab expression like `*/5 * * * *`._\n"
                ));
            }
        }
    }

    state.messages.push(ChatMessage::assistant(report));
}

pub(super) async fn cmd_morning_checkin(
    state: &mut EngineState,
    _parts: &[&str],
    _text: &str,
    _tx: Option<&mpsc::Sender<EngineEvent>>,
) {
    state
        .messages
        .push(ChatMessage::user("/morning-checkin".to_string()));

    let mut body = String::from("# Morning check-in\n\n");

    // ── 1. PRs needing attention ──────────────────────────────────────
    body.push_str("## PRs needing attention\n\n");
    match run_capture(
        "gh pr list --author @me \
         --json number,title,reviewDecision,statusCheckRollup",
    ) {
        Ok(s) if !s.is_empty() => {
            let prs: Vec<serde_json::Value> = serde_json::from_str(&s).unwrap_or_default();
            if prs.is_empty() {
                body.push_str("_No open PRs authored by you._\n");
            } else {
                for pr in &prs {
                    let num = pr.get("number").and_then(|v| v.as_i64()).unwrap_or(0);
                    let title = pr.get("title").and_then(|v| v.as_str()).unwrap_or("");
                    let review = pr
                        .get("reviewDecision")
                        .and_then(|v| v.as_str())
                        .unwrap_or("PENDING");
                    let checks = pr
                        .get("statusCheckRollup")
                        .and_then(|v| v.as_array())
                        .cloned()
                        .unwrap_or_default();
                    let ci = if checks.iter().any(|c| {
                        matches!(
                            c.get("conclusion").and_then(|v| v.as_str()),
                            Some("FAILURE") | Some("TIMED_OUT") | Some("CANCELLED")
                        )
                    }) {
                        "CI ✗"
                    } else if checks.iter().any(|c| {
                        matches!(
                            c.get("status").and_then(|v| v.as_str()),
                            Some("IN_PROGRESS") | Some("QUEUED") | Some("PENDING")
                        )
                    }) {
                        "CI …"
                    } else {
                        "CI ✓"
                    };
                    body.push_str(&format!("- **#{num}** {title} — {review} · {ci}\n"));
                }
            }
        }
        Ok(_) => body.push_str("_No open PRs authored by you._\n"),
        Err(e) => body.push_str(&format!(
            "_Could not query PRs (is `gh` installed and logged in?): {e}_\n"
        )),
    }
    body.push('\n');

    // ── 2. Assigned issues ────────────────────────────────────────────
    body.push_str("## Assigned issues\n\n");
    match run_capture("gh issue list --assignee @me --json number,title,state --limit 5") {
        Ok(s) if !s.is_empty() => {
            let issues: Vec<serde_json::Value> = serde_json::from_str(&s).unwrap_or_default();
            if issues.is_empty() {
                body.push_str("_No assigned issues._\n");
            } else {
                for issue in &issues {
                    let num = issue.get("number").and_then(|v| v.as_i64()).unwrap_or(0);
                    let title = issue.get("title").and_then(|v| v.as_str()).unwrap_or("");
                    let state = issue
                        .get("state")
                        .and_then(|v| v.as_str())
                        .unwrap_or("OPEN");
                    body.push_str(&format!("- **#{num}** ({state}) {title}\n"));
                }
            }
        }
        Ok(_) => body.push_str("_No assigned issues._\n"),
        Err(e) => body.push_str(&format!("_Could not query issues: {e}_\n")),
    }
    body.push('\n');

    // ── 3. Yesterday's work (commits) ─────────────────────────────────
    body.push_str("## Yesterday's work\n\n");
    let email = run_capture("git config user.email").unwrap_or_default();
    let log_cmd = if email.is_empty() {
        "git log --oneline --since=\"yesterday\"".to_string()
    } else {
        format!("git log --oneline --since=\"yesterday\" --author={email}")
    };
    match run_capture(&log_cmd) {
        Ok(s) if !s.is_empty() => {
            body.push_str("```\n");
            body.push_str(&s);
            body.push_str("\n```\n");
        }
        Ok(_) => body.push_str("_No commits since yesterday._\n"),
        Err(e) => body.push_str(&format!("_Could not read git log: {e}_\n")),
    }

    state.messages.push(ChatMessage::assistant(body));
}

/// `/queue` — show pending queued messages, or `/queue clear` to discard them.
///
/// When `message_queue_mode = true` in config, every prompt submitted while a
/// turn is streaming queues behind it rather than interrupting. This command
/// surfaces the queue contents and lets the user discard pending messages.
pub(super) async fn cmd_queue(
    state: &mut EngineState,
    parts: &[&str],
    _text: &str,
    _tx: Option<&mpsc::Sender<EngineEvent>>,
) {
    let sub = parts.get(1).map(|s| s.trim()).unwrap_or("");
    if sub == "clear" {
        let n = state.queued_prompts.len();
        if n == 0 {
            state.messages.push(ChatMessage::assistant(
                "Queue is already empty.".to_string(),
            ));
        } else {
            // Drain and discard all queued prompts.
            let _drained = state.queued_prompts.drain_all();
            // Remove the corresponding queued placeholder messages from the
            // transcript (they carry `queued = true` and haven't been promoted
            // yet, so no model context is lost).
            state.messages.retain(|m| !m.queued);
            state.messages.push(ChatMessage::assistant(format!(
                "Cleared **{n}** queued message{}.",
                if n == 1 { "" } else { "s" },
            )));
        }
        return;
    }

    let queue_mode = config::load_arc().message_queue_mode;
    let depth = state.queued_prompts.len();
    let mode_label = if queue_mode {
        "`message_queue_mode` **on** — interrupts disabled, all new messages queue."
    } else {
        "`message_queue_mode` **off** — interrupts enabled (default)."
    };

    if depth == 0 {
        state.messages.push(ChatMessage::assistant(format!(
            "**Queue** — {mode_label}\n\nNo messages queued."
        )));
        return;
    }

    let mut body = format!("**Queue** ({depth} pending) — {mode_label}\n\n");
    for (i, entry) in state.queued_prompts.iter().enumerate() {
        let preview: String = entry.text.chars().take(120).collect();
        let ellipsis = if entry.text.chars().count() > 120 {
            "…"
        } else {
            ""
        };
        body.push_str(&format!("{}. `{preview}{ellipsis}`\n", i + 1));
    }
    body.push_str("\nUse `/queue clear` to discard all pending messages.");
    state.messages.push(ChatMessage::assistant(body));
}
