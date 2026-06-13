//! Centralized Anthropic request error-recovery controller.
//!
//! Mirrors Claude Code 2.1.177's `onError` retry classifier (cli.js ~635540):
//! a single place that, given a failed request's HTTP status + body, decides
//! how to *mutate the request and retry* rather than failing the turn. Each
//! classifier latches a "done" flag so the same recovery is never attempted
//! twice in one turn (preventing infinite retry loops), and the classifiers
//! run in a fixed priority order.
//!
//! The controller is pure and synchronous: it inspects an error and a JSON
//! request body, returns a [`RecoveryAction`], and (for the mutating actions)
//! edits the body in place. The async rotation loop in `anthropic_oauth.rs`
//! owns the actual re-send; this module owns the *decision*.
//!
//! Coverage (priority order, matching 2.1.177):
//! 1. media-block strip — 400 pointing at an unprocessable image/document block
//! 2. cache-diagnosis beta reject — 400 rejecting the cache-diagnosis beta
//! 3. thinking-type toggle — 400 rejecting `thinking.type` (enabled↔adaptive)
//! 4. thinking-signature strip — 400 rejecting a thinking block signature
//! 5. mid-conversation system fallback — 400 rejecting a `role:"system"` message
//! 6. context-hint cleanup — 422/424 (handled by the orchestrator's compaction)
//!
//! Model/Opus fallback and account rotation stay in `anthropic_oauth.rs`
//! because they depend on account-manager state; this module owns the
//! body-mutating recoveries that are pure functions of (error, body).

use serde_json::Value;

/// A sticky latch set: each recovery may fire at most once per turn. The
/// rotation loop owns one of these for the lifetime of a single `stream()`
/// call and threads it through every [`classify_and_recover`] call.
#[derive(Debug, Default, Clone)]
pub struct RecoveryLatches {
    pub media_strip_done: bool,
    pub cache_diagnosis_dropped: bool,
    pub thinking_type_toggled: bool,
    pub thinking_signature_stripped: bool,
    pub mid_conv_system_fallback_done: bool,
    pub fallback_credit_stripped: bool,
    /// Number of media blocks stripped so far — bounds the strip-latest path so
    /// a persistently rejected request can't loop forever. Mirrors `N$ < p$`.
    pub media_blocks_stripped: u32,
}

/// Upper bound on un-targeted media-block strips per turn. Mirrors 2.1.177's
/// `p$` carrier cap.
pub const MAX_MEDIA_BLOCK_STRIPS: u32 = 4;

/// What the rotation loop should do after the controller inspects an error.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RecoveryAction {
    /// The body was mutated in place; re-send it. The payload is a short,
    /// stable directive label mirroring 2.1.177's `"retry:..."` strings — used
    /// for telemetry and tests.
    Retry(RetryKind),
    /// No recovery applies; the caller should surface the error.
    Surface,
}

/// Stable labels for the recovery that fired. Mirror 2.1.177's `retry:*`
/// directive strings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RetryKind {
    MediaStrip,
    CacheDiagnosisBeta,
    ThinkingType,
    ThinkingSignatureStrip,
    MidConvSystem,
    /// The server rejected the `fallback_credit_token` (expired, wrong org,
    /// malformed, invalid for model, or forbidden); strip it and retry without
    /// the credit. Mirrors 2.1.177's `tengu_fallback_credit_*` strip path.
    FallbackCreditStrip,
}

impl RetryKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::MediaStrip => "retry:media-strip",
            Self::CacheDiagnosisBeta => "retry:cache-diagnosis-beta",
            Self::ThinkingType => "retry:thinking-type",
            Self::ThinkingSignatureStrip => "retry:thinking-signature-strip",
            Self::MidConvSystem => "retry:mid-conv-system",
            Self::FallbackCreditStrip => "retry:fallback-credit-strip",
        }
    }
}

/// Classify a `fallback_credit_token` rejection from a 400 body, mirroring
/// 2.1.177's `credit_*` outcome labels (cli.js ~511847).
pub fn classify_fallback_credit_reject(body: &str) -> Option<&'static str> {
    if body.contains("fallback_credit_token: invalid or malformed") {
        Some("credit_malformed")
    } else if body.contains("fallback_credit_token: does not belong to this organization") {
        Some("credit_wrong_org")
    } else if body.contains("fallback_credit_token: has expired") {
        Some("credit_expired")
    } else if body.contains("fallback_credit_token: is not valid for model") {
        Some("credit_invalid_model")
    } else if body.contains("Extra inputs are not permitted")
        && body.contains("fallback_credit_token")
    {
        Some("credit_extra_forbidden")
    } else if body.contains("fallback_credit_token") {
        Some("credit_other")
    } else {
        None
    }
}

/// True when the error is an HTTP 400 (the status every body-mutating recovery
/// keys off). `status` is the parsed HTTP status code, `body` the raw response.
fn is_400(status: u16) -> bool {
    status == 400
}

/// Classify a failed Anthropic request and, if a recovery applies, mutate
/// `body` in place and return [`RecoveryAction::Retry`]. `latches` carries the
/// per-turn sticky flags so each recovery fires at most once.
///
/// `status` is the HTTP status code; `body` the raw error response text;
/// `request_body` the JSON request that was sent (mutated in place on retry).
pub fn classify_and_recover(
    status: u16,
    body: &str,
    request_body: &mut Value,
    latches: &mut RecoveryLatches,
) -> RecoveryAction {
    if !is_400(status) {
        return RecoveryAction::Surface;
    }

    // 0. Fallback-credit-token reject — strip the poisoned credit token and
    //    retry without it. Highest priority: a bad token fails every retry.
    if !latches.fallback_credit_stripped
        && let Some(reason) = classify_fallback_credit_reject(body)
        && strip_fallback_credit_token(request_body)
    {
        latches.fallback_credit_stripped = true;
        tracing::warn!(
            target: "jfc::provider::anthropic_recovery",
            reason,
            "tengu_fallback_credit_forfeited: server rejected fallback_credit_token — stripping and retrying"
        );
        return RecoveryAction::Retry(RetryKind::FallbackCreditStrip);
    }

    // 1. Media-block strip — a 400 pointing at an unprocessable image/document.
    if !latches.media_strip_done
        && latches.media_blocks_stripped < MAX_MEDIA_BLOCK_STRIPS
        && let Some(kind) = detect_unprocessable_media(body)
        && strip_first_media_block(request_body, kind)
    {
        latches.media_blocks_stripped += 1;
        if latches.media_blocks_stripped >= MAX_MEDIA_BLOCK_STRIPS {
            latches.media_strip_done = true;
        }
        return RecoveryAction::Retry(RetryKind::MediaStrip);
    }

    // 2. Cache-diagnosis beta reject — drop the diagnostics field + latch.
    if !latches.cache_diagnosis_dropped
        && is_cache_diagnosis_beta_reject(body)
        && drop_cache_diagnosis(request_body)
    {
        latches.cache_diagnosis_dropped = true;
        return RecoveryAction::Retry(RetryKind::CacheDiagnosisBeta);
    }

    // 3. Thinking-type toggle — flip enabled↔adaptive.
    if !latches.thinking_type_toggled
        && is_thinking_type_reject(body)
        && toggle_thinking_type(request_body)
    {
        latches.thinking_type_toggled = true;
        return RecoveryAction::Retry(RetryKind::ThinkingType);
    }

    // 4. Thinking-signature strip — drop ALL thinking blocks.
    if !latches.thinking_signature_stripped
        && is_thinking_signature_reject(body)
        && strip_thinking_blocks(request_body)
    {
        latches.thinking_signature_stripped = true;
        return RecoveryAction::Retry(RetryKind::ThinkingSignatureStrip);
    }

    // 5. Mid-conversation system fallback — server rejected a `role:"system"`
    //    message; fold it into a `<system-reminder>` user block.
    if !latches.mid_conv_system_fallback_done
        && is_mid_conv_system_reject(body)
        && fold_system_role_messages(request_body)
    {
        latches.mid_conv_system_fallback_done = true;
        return RecoveryAction::Retry(RetryKind::MidConvSystem);
    }

    RecoveryAction::Surface
}

// ─── Detection ──────────────────────────────────────────────────────────────

/// The media kind an Anthropic 400 says it could not process.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaKind {
    Image,
    Document,
}

impl MediaKind {
    fn wire_type(self) -> &'static str {
        match self {
            Self::Image => "image",
            Self::Document => "document",
        }
    }
}

/// Detect a 400 that names an unprocessable media block. Anthropic returns
/// messages like `messages.3.content.1: Could not process image` or mentions a
/// `document`/`image` block that failed validation.
pub fn detect_unprocessable_media(body: &str) -> Option<MediaKind> {
    let lower = body.to_ascii_lowercase();
    let unprocessable = lower.contains("could not process")
        || lower.contains("unable to process")
        || lower.contains("failed to process")
        || lower.contains("invalid image")
        || lower.contains("invalid document")
        || lower.contains("unsupported image")
        || lower.contains("could not be processed");
    if !unprocessable {
        return None;
    }
    if lower.contains("document") || lower.contains("pdf") {
        Some(MediaKind::Document)
    } else if lower.contains("image") {
        Some(MediaKind::Image)
    } else {
        None
    }
}

/// 400 rejecting the cache-diagnosis beta / `diagnostics.previous_message_id`.
pub fn is_cache_diagnosis_beta_reject(body: &str) -> bool {
    let lower = body.to_ascii_lowercase();
    (lower.contains("diagnostics") || lower.contains("previous_message_id"))
        && (lower.contains("extra inputs are not permitted")
            || lower.contains("unexpected")
            || lower.contains("not permitted")
            || lower.contains("not allowed"))
}

/// 400 rejecting `thinking.type` (e.g. adaptive rejected by this model).
pub fn is_thinking_type_reject(body: &str) -> bool {
    let lower = body.to_ascii_lowercase();
    lower.contains("thinking")
        && (lower.contains("thinking.type")
            || lower.contains("adaptive")
            || lower.contains("thinking type"))
        && (lower.contains("not support")
            || lower.contains("invalid")
            || lower.contains("not allowed")
            || lower.contains("unexpected")
            || lower.contains("rejected"))
}

/// 400 rejecting a thinking block / its signature (re-validation failed).
pub fn is_thinking_signature_reject(body: &str) -> bool {
    let lower = body.to_ascii_lowercase();
    lower.contains("thinking")
        && (lower.contains("signature")
            || lower.contains("redacted_thinking")
            || (lower.contains("thinking") && lower.contains("invalid")))
        && !is_thinking_type_reject(body)
}

/// 400 rejecting a `role:"system"` message mid-conversation.
pub fn is_mid_conv_system_reject(body: &str) -> bool {
    let lower = body.to_ascii_lowercase();
    lower.contains("role")
        && lower.contains("system")
        && (lower.contains("not support")
            || lower.contains("invalid")
            || lower.contains("unexpected")
            || lower.contains("not permitted")
            || lower.contains("rejected"))
}

// ─── Mutation ─────────────────────────────────────────────────────────────

/// Remove the first content block matching `kind` from the request body's
/// messages. Returns whether a block was removed.
fn strip_first_media_block(body: &mut Value, kind: MediaKind) -> bool {
    let Some(messages) = body.get_mut("messages").and_then(|m| m.as_array_mut()) else {
        return false;
    };
    let wire = kind.wire_type();
    for message in messages.iter_mut() {
        let Some(content) = message.get_mut("content").and_then(|c| c.as_array_mut()) else {
            continue;
        };
        if let Some(pos) = content
            .iter()
            .position(|block| block.get("type").and_then(|t| t.as_str()) == Some(wire))
        {
            content.remove(pos);
            // Don't leave an empty content array — the API rejects it.
            if content.is_empty() {
                content.push(serde_json::json!({
                    "type": "text",
                    "text": "[unprocessable media removed]"
                }));
            }
            return true;
        }
    }
    false
}

/// Remove the `fallback_credit_token` field from the request body. Returns
/// whether it was present. Mirrors 2.1.177's `delete v32.fallback_credit_token`
/// strip (cli.js 634654).
fn strip_fallback_credit_token(body: &mut Value) -> bool {
    if let Some(obj) = body.as_object_mut() {
        return obj.remove("fallback_credit_token").is_some();
    }
    false
}

/// Drop the `diagnostics` request field (and thus the cache-diagnosis beta).
fn drop_cache_diagnosis(body: &mut Value) -> bool {
    if let Some(obj) = body.as_object_mut() {
        return obj.remove("diagnostics").is_some();
    }
    false
}

/// Flip `thinking.type` between `"enabled"` and `"adaptive"`. When toggling to
/// `"enabled"` we must supply a `budget_tokens` (the API requires it); when
/// toggling to `"adaptive"` we drop it.
fn toggle_thinking_type(body: &mut Value) -> bool {
    let Some(thinking) = body.get_mut("thinking").and_then(|t| t.as_object_mut()) else {
        return false;
    };
    match thinking.get("type").and_then(|t| t.as_str()) {
        Some("adaptive") => {
            thinking.insert("type".into(), Value::String("enabled".into()));
            thinking
                .entry("budget_tokens")
                .or_insert(serde_json::json!(16000));
            true
        }
        Some("enabled") => {
            thinking.insert("type".into(), Value::String("adaptive".into()));
            thinking.remove("budget_tokens");
            true
        }
        _ => false,
    }
}

/// Remove every `thinking` / `redacted_thinking` block from all messages.
/// Returns whether anything was removed.
fn strip_thinking_blocks(body: &mut Value) -> bool {
    let Some(messages) = body.get_mut("messages").and_then(|m| m.as_array_mut()) else {
        return false;
    };
    let mut removed = false;
    for message in messages.iter_mut() {
        let Some(content) = message.get_mut("content").and_then(|c| c.as_array_mut()) else {
            continue;
        };
        let before = content.len();
        content.retain(|block| {
            !matches!(
                block.get("type").and_then(|t| t.as_str()),
                Some("thinking") | Some("redacted_thinking")
            )
        });
        if content.len() != before {
            removed = true;
        }
        if content.is_empty() {
            content.push(serde_json::json!({ "type": "text", "text": "" }));
        }
    }
    removed
}

/// Fold any `role:"system"` message into a `<system-reminder>` user text block,
/// mirroring 2.1.177's mid-conversation system fallback. Returns whether a
/// system-role message was rewritten.
fn fold_system_role_messages(body: &mut Value) -> bool {
    let Some(messages) = body.get_mut("messages").and_then(|m| m.as_array_mut()) else {
        return false;
    };
    let mut changed = false;
    for message in messages.iter_mut() {
        if message.get("role").and_then(|r| r.as_str()) != Some("system") {
            continue;
        }
        let text = extract_message_text(message);
        let wrapped = format!("<system-reminder>{text}</system-reminder>");
        *message = serde_json::json!({
            "role": "user",
            "content": [{ "type": "text", "text": wrapped }],
        });
        changed = true;
    }
    changed
}

/// Concatenate the text of a message's content blocks (string or array shape).
fn extract_message_text(message: &Value) -> String {
    match message.get("content") {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Array(blocks)) => blocks
            .iter()
            .filter_map(|b| b.get("text").and_then(|t| t.as_str()))
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn body_with_thinking(kind: &str) -> Value {
        json!({
            "model": "claude-opus-4-6",
            "messages": [
                { "role": "user", "content": [{ "type": "text", "text": "hi" }] },
                { "role": "assistant", "content": [
                    { "type": "thinking", "thinking": "...", "signature": "sig" },
                    { "type": "text", "text": "answer" }
                ]}
            ],
            "thinking": { "type": kind, "budget_tokens": 16000 }
        })
    }

    // Normal: a media-strip 400 removes the offending image block and latches.
    #[test]
    fn media_strip_removes_block_and_counts_normal() {
        let mut body = json!({
            "messages": [
                { "role": "user", "content": [
                    { "type": "image", "source": { "data": "AAAA" } },
                    { "type": "text", "text": "describe" }
                ]}
            ]
        });
        let mut latches = RecoveryLatches::default();
        let action = classify_and_recover(
            400,
            "messages.0.content.0: Could not process image",
            &mut body,
            &mut latches,
        );
        assert_eq!(action, RecoveryAction::Retry(RetryKind::MediaStrip));
        assert_eq!(latches.media_blocks_stripped, 1);
        let content = body["messages"][0]["content"].as_array().unwrap();
        assert!(
            content
                .iter()
                .all(|b| b["type"].as_str() != Some("image")),
            "image block must be stripped"
        );
    }

    // Robust: a non-400 never triggers recovery.
    #[test]
    fn non_400_surfaces_robust() {
        let mut body = json!({ "messages": [] });
        let mut latches = RecoveryLatches::default();
        let action = classify_and_recover(529, "overloaded", &mut body, &mut latches);
        assert_eq!(action, RecoveryAction::Surface);
    }

    // Normal: thinking-type toggle flips adaptive→enabled and adds a budget.
    #[test]
    fn thinking_type_toggle_adaptive_to_enabled_normal() {
        let mut body = body_with_thinking("adaptive");
        body["thinking"].as_object_mut().unwrap().remove("budget_tokens");
        let mut latches = RecoveryLatches::default();
        let action = classify_and_recover(
            400,
            "thinking.type adaptive: API rejected this value for the model",
            &mut body,
            &mut latches,
        );
        assert_eq!(action, RecoveryAction::Retry(RetryKind::ThinkingType));
        assert_eq!(body["thinking"]["type"].as_str(), Some("enabled"));
        assert!(body["thinking"]["budget_tokens"].is_number());
        assert!(latches.thinking_type_toggled);
    }

    // Normal: a thinking-signature reject strips all thinking blocks.
    #[test]
    fn thinking_signature_strip_removes_blocks_normal() {
        let mut body = body_with_thinking("enabled");
        let mut latches = RecoveryLatches::default();
        let action = classify_and_recover(
            400,
            "invalid signature on thinking block",
            &mut body,
            &mut latches,
        );
        assert_eq!(action, RecoveryAction::Retry(RetryKind::ThinkingSignatureStrip));
        let assistant = &body["messages"][1]["content"].as_array().unwrap();
        assert!(
            assistant
                .iter()
                .all(|b| b["type"].as_str() != Some("thinking")),
            "thinking blocks must be stripped"
        );
    }

    // Normal: cache-diagnosis reject drops the diagnostics field once.
    #[test]
    fn cache_diagnosis_reject_drops_field_normal() {
        let mut body = json!({
            "messages": [],
            "diagnostics": { "previous_message_id": "msg_1" }
        });
        let mut latches = RecoveryLatches::default();
        let action = classify_and_recover(
            400,
            "diagnostics.previous_message_id: Extra inputs are not permitted",
            &mut body,
            &mut latches,
        );
        assert_eq!(action, RecoveryAction::Retry(RetryKind::CacheDiagnosisBeta));
        assert!(body.get("diagnostics").is_none());
        assert!(latches.cache_diagnosis_dropped);
    }

    // Normal: mid-conv system reject folds the system message into a reminder.
    #[test]
    fn mid_conv_system_fallback_folds_message_normal() {
        let mut body = json!({
            "messages": [
                { "role": "system", "content": [{ "type": "text", "text": "be brief" }] },
                { "role": "user", "content": [{ "type": "text", "text": "hi" }] }
            ]
        });
        let mut latches = RecoveryLatches::default();
        let action = classify_and_recover(
            400,
            "messages.0: role \"system\" rejected in this position",
            &mut body,
            &mut latches,
        );
        assert_eq!(action, RecoveryAction::Retry(RetryKind::MidConvSystem));
        assert_eq!(body["messages"][0]["role"].as_str(), Some("user"));
        let text = body["messages"][0]["content"][0]["text"].as_str().unwrap();
        assert!(text.contains("<system-reminder>"));
        assert!(text.contains("be brief"));
    }

    // Robust: a latched recovery does not fire a second time — the same error
    // surfaces instead of looping.
    #[test]
    fn latched_recovery_does_not_repeat_robust() {
        let mut body = json!({
            "messages": [],
            "diagnostics": { "previous_message_id": "msg_1" }
        });
        let mut latches = RecoveryLatches::default();
        let err = "diagnostics: Extra inputs are not permitted";
        let first = classify_and_recover(400, err, &mut body, &mut latches);
        assert_eq!(first, RecoveryAction::Retry(RetryKind::CacheDiagnosisBeta));
        // Re-send still fails the same way (field already gone) — must surface.
        let second = classify_and_recover(400, err, &mut body, &mut latches);
        assert_eq!(second, RecoveryAction::Surface);
    }

    fn image_body() -> Value {
        json!({
            "messages": [
                { "role": "user", "content": [
                    { "type": "image", "source": {} },
                    { "type": "text", "text": "x" }
                ]}
            ]
        })
    }

    // Robust: media strips are bounded so a persistently rejected request can't
    // loop forever.
    #[test]
    fn media_strip_is_bounded_robust() {
        let mut latches = RecoveryLatches::default();
        for _ in 0..MAX_MEDIA_BLOCK_STRIPS {
            let mut body = image_body();
            let action =
                classify_and_recover(400, "could not process image", &mut body, &mut latches);
            assert_eq!(action, RecoveryAction::Retry(RetryKind::MediaStrip));
        }
        assert!(latches.media_strip_done);
        let mut body = image_body();
        let action =
            classify_and_recover(400, "could not process image", &mut body, &mut latches);
        assert_eq!(action, RecoveryAction::Surface);
    }

    // Normal: an expired credit token is stripped and retried with a labeled
    // credit reason.
    #[test]
    fn fallback_credit_expired_strips_token_normal() {
        let mut body = json!({
            "messages": [],
            "fallback_credit_token": "fct_abc"
        });
        let mut latches = RecoveryLatches::default();
        let action = classify_and_recover(
            400,
            "fallback_credit_token: has expired",
            &mut body,
            &mut latches,
        );
        assert_eq!(action, RecoveryAction::Retry(RetryKind::FallbackCreditStrip));
        assert!(body.get("fallback_credit_token").is_none());
        assert!(latches.fallback_credit_stripped);
    }

    // Robust: every documented credit-reject phrasing maps to a label, and an
    // unrelated body yields None.
    #[test]
    fn classify_fallback_credit_reject_covers_all_reasons_robust() {
        assert_eq!(
            classify_fallback_credit_reject("fallback_credit_token: has expired"),
            Some("credit_expired")
        );
        assert_eq!(
            classify_fallback_credit_reject("fallback_credit_token: invalid or malformed"),
            Some("credit_malformed")
        );
        assert_eq!(
            classify_fallback_credit_reject(
                "fallback_credit_token: does not belong to this organization"
            ),
            Some("credit_wrong_org")
        );
        assert_eq!(
            classify_fallback_credit_reject("fallback_credit_token: is not valid for model"),
            Some("credit_invalid_model")
        );
        assert_eq!(
            classify_fallback_credit_reject(
                "fallback_credit_token: Extra inputs are not permitted"
            ),
            Some("credit_extra_forbidden")
        );
        assert_eq!(
            classify_fallback_credit_reject("fallback_credit_token: weird new error"),
            Some("credit_other")
        );
        assert_eq!(classify_fallback_credit_reject("some other 400"), None);
    }

    #[test]
    fn retry_kind_labels_match_upstream_strings_normal() {
        assert_eq!(RetryKind::MediaStrip.as_str(), "retry:media-strip");
        assert_eq!(
            RetryKind::CacheDiagnosisBeta.as_str(),
            "retry:cache-diagnosis-beta"
        );
        assert_eq!(RetryKind::ThinkingType.as_str(), "retry:thinking-type");
        assert_eq!(
            RetryKind::ThinkingSignatureStrip.as_str(),
            "retry:thinking-signature-strip"
        );
        assert_eq!(RetryKind::MidConvSystem.as_str(), "retry:mid-conv-system");
        assert_eq!(
            RetryKind::FallbackCreditStrip.as_str(),
            "retry:fallback-credit-strip"
        );
    }
}
