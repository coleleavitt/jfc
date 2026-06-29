use std::ops::Range;

use jfc_provider::ModelId;
use sha2::{Digest, Sha256};

use crate::app::EngineState;
use crate::types::{ChatMessage, Role};

const CACHE_READ_DROP_THRESHOLD: u32 = 2_000;
const SUMMARY_RECENT_MESSAGES: usize = 8;
const SUMMARY_MESSAGE_CHARS: usize = 1_200;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExpectedCacheDrop {
    pub identity: String,
    pub reason: String,
    pub dropped_messages: usize,
    pub archive_id: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PiggybackDrop {
    pub target_identity: String,
    pub dropped_messages: usize,
    pub archive_id: Option<String>,
}

pub fn cache_identity(provider_name: &str, model: &ModelId) -> String {
    format!("{provider_name}/{}", model.as_str())
}

pub fn request_cache_identity(state: &EngineState, provider_name: &str, model: &ModelId) -> String {
    let base = cache_identity(provider_name, model);
    format!("{base}#{}", request_cache_fingerprint(state))
}

pub fn current_identity(state: &EngineState) -> String {
    request_cache_identity(state, state.provider.name(), &state.model)
}

pub fn stamp_assistant(messages: &mut [ChatMessage], assistant_idx: usize, identity: &str) {
    if let Some(msg) = messages.get_mut(assistant_idx)
        && msg.role == Role::Assistant
    {
        msg.model_name = Some(identity.to_owned());
    }
}

pub fn active_stream_identity(state: &EngineState) -> String {
    state
        .streaming_assistant_idx
        .and_then(|idx| state.messages.get(idx))
        .and_then(|msg| msg.model_name.as_deref())
        .map(str::to_owned)
        .unwrap_or_else(|| current_identity(state))
}

pub fn previous_response_id_for(
    state: &EngineState,
    provider_name: &str,
    model: &ModelId,
) -> Option<String> {
    let identity = request_cache_identity(state, provider_name, model);
    state
        .response_ids_by_cache_identity
        .get(&identity)
        .cloned()
        .or_else(|| {
            let legacy_identity = cache_identity(provider_name, model);
            state
                .response_ids_by_cache_identity
                .get(&legacy_identity)
                .cloned()
        })
}

pub fn record_response_id(state: &mut EngineState, id: String) {
    let identity = active_stream_identity(state);
    state
        .response_ids_by_cache_identity
        .insert(identity, id.clone());
    state.last_response_id = Some(id);
}

pub fn mark_expected_drop(
    state: &mut EngineState,
    identity: String,
    reason: impl Into<String>,
    dropped_messages: usize,
    archive_id: Option<String>,
) {
    state.prompt_cache_expected_drop = Some(ExpectedCacheDrop {
        identity,
        reason: reason.into(),
        dropped_messages,
        archive_id,
    });
}

pub fn observe_cache_usage(
    state: &mut EngineState,
    input_tokens: u32,
    cache_read_tokens: u32,
    cache_write_tokens: u32,
    partial_input_only: bool,
) -> bool {
    if partial_input_only {
        return false;
    }

    let identity = active_stream_identity(state);
    let previous_read = state
        .prompt_cache_reads_by_identity
        .insert(identity.clone(), cache_read_tokens);

    if state
        .prompt_cache_expected_drop
        .as_ref()
        .is_some_and(|drop| drop.identity == identity)
    {
        let drop = state.prompt_cache_expected_drop.take().expect("checked");
        tracing::info!(
            target: "jfc::cache_diagnosis",
            identity = %drop.identity,
            reason = %drop.reason,
            dropped_messages = drop.dropped_messages,
            archive_id = drop.archive_id.as_deref().unwrap_or(""),
            input_tokens,
            cache_read_tokens,
            cache_write_tokens,
            "prompt cache deletion applied (expected drop)"
        );
        return true;
    }

    if let Some(previous_read) = previous_read {
        let dropped = previous_read.saturating_sub(cache_read_tokens);
        if cache_read_tokens < previous_read.saturating_mul(95) / 100
            && dropped >= CACHE_READ_DROP_THRESHOLD
        {
            tracing::warn!(
                target: "jfc::cache_diagnosis",
                identity = %identity,
                previous_cache_read_tokens = previous_read,
                cache_read_tokens,
                dropped,
                input_tokens,
                cache_write_tokens,
                "prompt cache read dropped unexpectedly"
            );
        }
    }

    false
}

pub fn maybe_piggyback_drop_on_model_switch(
    state: &mut EngineState,
    target_identity: &str,
) -> Option<PiggybackDrop> {
    maybe_piggyback_drop_for_identity_change(state, target_identity, "provider/model switch")
}

pub fn maybe_piggyback_drop_for_identity_change(
    state: &mut EngineState,
    target_identity: &str,
    change_kind: &str,
) -> Option<PiggybackDrop> {
    let range = cold_tail_range(&state.messages, target_identity)?;
    let pre_tokens = state
        .tool_ctx
        .approx_tokens
        .max(crate::compact::estimate_tokens(&state.messages));
    let dropped_tail: Vec<ChatMessage> = state.messages[range.clone()].to_vec();
    let dropped_messages = dropped_tail.len();
    let summary_without_archive =
        build_cache_tail_summary(target_identity, change_kind, &dropped_tail, None);

    let archive_id = match crate::compact_archive::archive_current_project(
        &dropped_tail,
        pre_tokens,
        &summary_without_archive,
    ) {
        Ok(Some(meta)) => Some(meta.id),
        Ok(None) => None,
        Err(error) => {
            tracing::warn!(
                target: "jfc::cache_lineage",
                error = %error,
                dropped_messages,
                target_identity,
                "failed to archive cache-lineage tail before dropping it"
            );
            None
        }
    };

    let summary = build_cache_tail_summary(
        target_identity,
        change_kind,
        &dropped_tail,
        archive_id.as_deref(),
    );
    state.messages.truncate(range.start);
    state
        .messages
        .push(ChatMessage::compact_boundary(&summary, pre_tokens));

    let post_tokens = crate::compact::estimate_tokens(&state.messages)
        .saturating_add(state.last_system_prompt_len.unwrap_or(30_000));
    state.tool_ctx.approx_tokens = post_tokens;
    state.post_compact_token_ceiling = Some(
        post_tokens
            .saturating_mul(2)
            .max(post_tokens.saturating_add(50_000)),
    );
    mark_expected_drop(
        state,
        target_identity.to_owned(),
        format!("{change_kind} trimmed an incompatible prompt-cache tail"),
        dropped_messages,
        archive_id.clone(),
    );

    tracing::info!(
        target: "jfc::cache_lineage",
        target_identity,
        change_kind,
        dropped_messages,
        archive_id = archive_id.as_deref().unwrap_or(""),
        "piggybacked cache-lineage tail drop on identity switch"
    );

    Some(PiggybackDrop {
        target_identity: target_identity.to_owned(),
        dropped_messages,
        archive_id,
    })
}

fn request_cache_fingerprint(state: &EngineState) -> String {
    let mut entries = Vec::new();
    let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
    let context_hierarchy = crate::prompt_context_cache::context_hierarchy(&cwd, &state.extra_dirs);
    let active_style = crate::output_style::active();
    let output_style_suffix_hash = crate::output_style::active_suffix(&cwd)
        .map(|suffix| stable_hash(suffix.as_bytes()))
        .unwrap_or_else(|| "none".to_owned());

    push_entry(&mut entries, "context_window", state.max_context_tokens);
    push_entry(
        &mut entries,
        "max_output_tokens",
        format!("{:?}", state.max_output_tokens),
    );
    push_entry(&mut entries, "output_style", active_style.name());
    push_entry(
        &mut entries,
        "output_style_suffix",
        output_style_suffix_hash,
    );
    push_entry(
        &mut entries,
        "context_hierarchy",
        context_hierarchy
            .rendered
            .as_deref()
            .map(|rendered| stable_hash(rendered.as_bytes()))
            .unwrap_or_else(|| "none".to_owned()),
    );
    push_entry(
        &mut entries,
        "permission_mode",
        state.permission_mode.label(),
    );
    push_entry(&mut entries, "auto_mode_enabled", state.auto_mode.enabled);
    push_entry(&mut entries, "fast_mode", state.fast_mode);
    push_entry(&mut entries, "brief_mode", state.brief_mode);
    push_entry(
        &mut entries,
        "reasoning_effort",
        format!("{:?}", state.effort_state.current),
    );
    push_entry(&mut entries, "ultracode", state.effort_state.ultracode);
    push_entry(
        &mut entries,
        "temperature",
        format!("{:?}", state.temperature_state.current),
    );
    push_entry(
        &mut entries,
        "advisor_local_provider",
        if state.advisor_enabled && state.local_advisor_model.is_some() {
            state
                .local_advisor_provider
                .as_ref()
                .map(ToString::to_string)
                .unwrap_or_default()
        } else {
            String::new()
        },
    );
    push_entry(
        &mut entries,
        "advisor_local_model",
        if state.advisor_enabled {
            state
                .local_advisor_model
                .as_ref()
                .map(ToString::to_string)
                .unwrap_or_default()
        } else {
            String::new()
        },
    );
    push_entry(
        &mut entries,
        "advisor_server_model",
        state
            .server_advisor_model
            .as_ref()
            .map(ToString::to_string)
            .unwrap_or_default(),
    );
    push_entry(&mut entries, "advisor_enabled", state.advisor_enabled);
    push_entry(
        &mut entries,
        "council_verdict",
        state.council_verdict_enabled,
    );
    push_entry(
        &mut entries,
        "cli_system_prompt",
        state
            .cli_system_prompt
            .as_deref()
            .map(|prompt| stable_hash(prompt.as_bytes()))
            .unwrap_or_else(|| "none".to_owned()),
    );
    push_entry(
        &mut entries,
        "dangerously_skip_permissions",
        state.dangerously_skip_permissions,
    );
    push_entry(
        &mut entries,
        "max_thinking_tokens",
        format!("{:?}", state.cli_max_thinking_tokens),
    );
    push_entry(
        &mut entries,
        "thinking_display",
        state.cli_thinking_display.as_deref().unwrap_or(""),
    );
    push_entry(
        &mut entries,
        "task_budget",
        format!("{:?}", state.cli_task_budget),
    );
    push_entry(
        &mut entries,
        "fine_grained_tool_streaming",
        state.fine_grained_tool_streaming,
    );
    push_entry(
        &mut entries,
        "strict_tool_schemas",
        state.strict_tool_schemas,
    );
    push_entry(
        &mut entries,
        "mcp_config_path",
        state
            .mcp_config_path
            .as_ref()
            .map(|path| path.display().to_string())
            .unwrap_or_default(),
    );
    push_entry(&mut entries, "cwd", cwd.display());
    push_entry(&mut entries, "git_root", git_root_label(state));
    push_sorted(&mut entries, "custom_betas", state.custom_betas.iter());
    push_sorted(&mut entries, "allowed_tools", state.allowed_tools.iter());
    push_sorted(
        &mut entries,
        "disallowed_tools",
        state
            .disallowed_tools
            .iter()
            .chain(context_hierarchy.disallowed_tools.iter()),
    );
    push_sorted(
        &mut entries,
        "extra_dirs",
        state
            .extra_dirs
            .iter()
            .map(|path| path.display().to_string()),
    );
    push_sorted(
        &mut entries,
        "mcp_servers",
        state
            .mcp_servers
            .iter()
            .map(|server| format!("{}:{:?}", server.name, server.status)),
    );
    push_sorted(
        &mut entries,
        "feature_gates",
        crate::feature_gates::FeatureGate::ALL.iter().map(|gate| {
            format!(
                "{}:{}",
                gate.codename(),
                crate::feature_gates::is_enabled(*gate)
            )
        }),
    );

    entries.sort();
    stable_hash(entries.join("\n").as_bytes())
}

fn push_entry(value: &mut Vec<String>, key: &str, entry: impl ToString) {
    value.push(format!("{key}={}", entry.to_string()));
}

fn push_sorted<'a, I, T>(entries: &mut Vec<String>, key: &str, values: I)
where
    I: IntoIterator<Item = T>,
    T: ToString + 'a,
{
    let mut values = values
        .into_iter()
        .map(|value| value.to_string())
        .collect::<Vec<_>>();
    values.sort();
    push_entry(entries, key, values.join(","));
}

fn git_root_label(state: &EngineState) -> String {
    match &state.git_root {
        Some(Some(root)) => root.display().to_string(),
        Some(None) => "none".to_owned(),
        None => "unknown".to_owned(),
    }
}

fn stable_hash(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hex::encode(&hasher.finalize()[..8])
}

pub(crate) fn cold_tail_range(
    messages: &[ChatMessage],
    target_identity: &str,
) -> Option<Range<usize>> {
    let anchor = messages
        .iter()
        .enumerate()
        .rev()
        .find(|(_, msg)| {
            msg.role == Role::Assistant && msg.model_name.as_deref() == Some(target_identity)
        })?
        .0;
    let tail_start = anchor + 1;
    if tail_start >= messages.len() {
        return None;
    }

    let has_foreign_assistant = messages[tail_start..].iter().any(|msg| {
        msg.role == Role::Assistant
            && msg
                .model_name
                .as_deref()
                .is_some_and(|identity| identity != target_identity)
    });
    has_foreign_assistant.then_some(tail_start..messages.len())
}

fn build_cache_tail_summary(
    target_identity: &str,
    change_kind: &str,
    dropped_tail: &[ChatMessage],
    archive_id: Option<&str>,
) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "Summary: JFC trimmed a {change_kind} tail to preserve prompt-cache lineage.\n\n"
    ));
    out.push_str(&format!("Returning cache identity: `{target_identity}`\n"));
    out.push_str(&format!("Dropped tail messages: {}\n", dropped_tail.len()));
    if let Some(archive_id) = archive_id {
        out.push_str(&format!(
            "Raw cache-tail archive: `{archive_id}`. Use `/expand {archive_id}` to inspect the exact dropped transcript.\n"
        ));
    }
    out.push_str(
        "\nWhy this exists: those later messages were generated under another cache identity, so keeping them in front of the returning request would break the provider prefix cache. Continue using the handoff below as prior work.\n\n",
    );
    out.push_str("Recent dropped tail:\n");

    let start = dropped_tail.len().saturating_sub(SUMMARY_RECENT_MESSAGES);
    for (offset, msg) in dropped_tail[start..].iter().enumerate() {
        let idx = start + offset;
        let role = match msg.role {
            Role::User => "user",
            Role::Assistant => "assistant",
        };
        let identity = msg.model_name.as_deref().unwrap_or("local");
        let text = message_text(msg);
        if text.trim().is_empty() {
            continue;
        }
        out.push_str(&format!(
            "- #{idx} {role} [{identity}]: {}\n",
            truncate_chars(text.trim(), SUMMARY_MESSAGE_CHARS)
        ));
    }

    out
}

fn message_text(msg: &ChatMessage) -> String {
    msg.parts
        .iter()
        .map(|part| part.text_only())
        .filter(|text| !text.trim().is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

fn truncate_chars(text: &str, max_chars: usize) -> String {
    let mut chars = text.chars();
    let mut out: String = chars.by_ref().take(max_chars).collect();
    if chars.next().is_some() {
        out.push_str("...");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assistant(identity: &str) -> ChatMessage {
        let mut msg = ChatMessage::assistant(format!("reply from {identity}"));
        msg.model_name = Some(identity.to_owned());
        msg
    }

    #[test]
    fn cold_tail_range_detects_foreign_tail_after_return_anchor_normal() {
        let messages = vec![
            ChatMessage::user("one".into()),
            assistant("anthropic/claude"),
            ChatMessage::user("two".into()),
            assistant("openai/gpt"),
            ChatMessage::user("three".into()),
        ];

        assert_eq!(cold_tail_range(&messages, "anthropic/claude"), Some(2..5));
    }

    #[test]
    fn cold_tail_range_ignores_same_identity_tail_robust() {
        let messages = vec![
            ChatMessage::user("one".into()),
            assistant("anthropic/claude"),
            ChatMessage::user("two".into()),
            assistant("anthropic/claude"),
        ];

        assert_eq!(cold_tail_range(&messages, "anthropic/claude"), None);
    }

    #[test]
    fn stamp_assistant_sets_model_name_only_on_assistant_normal() {
        let mut messages = vec![
            ChatMessage::user("one".into()),
            ChatMessage::assistant(String::new()),
        ];

        stamp_assistant(&mut messages, 0, "anthropic/claude");
        stamp_assistant(&mut messages, 1, "anthropic/claude");

        assert_eq!(messages[0].model_name, None);
        assert_eq!(messages[1].model_name.as_deref(), Some("anthropic/claude"));
    }
}
