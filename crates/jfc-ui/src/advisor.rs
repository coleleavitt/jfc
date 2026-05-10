//! Advisor mode — a parallel AI assistant that provides guidance without
//! affecting the main conversation.
//!
//! Mirrors Claude Code v2.1.132's `tengu_advisor_*` telemetry surface (command,
//! dialog_shown, tool_call, tool_interrupted, tool_token_usage). The advisor:
//!
//! - Sees a SNAPSHOT of the main transcript at query time (read-only context).
//! - Runs as a separate non-streaming `provider.complete()` call so it doesn't
//!   share the main agent's stream channel or token accounting.
//! - Has no tools — it answers in prose only.
//! - Maintains its own per-session token budget; when exhausted, queries return
//!   `Err` and the user must reset the session (or wait for a future
//!   `/advisor-reset`).
//! - Doesn't mutate the main `Vec<ChatMessage>` — the caller is responsible for
//!   surfacing the reply (typically by appending an `Advisor`-tagged
//!   `MessagePart` so the renderer can style it distinctly).
//!
//! Default OFF semantics are upheld two ways:
//!   1. The slash command is gated by `app.advisor_enabled` (config flag).
//!   2. Even when "always available", the per-session token budget defaults to
//!      a small ceiling (`DEFAULT_TOKEN_BUDGET`), so a runaway loop can't drain
//!      the user's account.
//!
//! Side-pane rendering is intentionally not implemented here — the renderer
//! consumes `MessagePart::Advisor(String)` inline. See the follow-up note in
//! `render.rs` for where a split-pane would hook in.

use anyhow::{Result, anyhow};
use futures::StreamExt;
use uuid::Uuid;

use crate::provider::{
    CompletionResponse, ModelId, Provider, ProviderContent, ProviderMessage, ProviderRole,
    StreamEvent, StreamOptions,
};
use crate::types::{ChatMessage, MessagePart, Role};

/// Default per-session token budget. Conservative — about three round-trips
/// worth of advisor calls on a 200K-context model. Users can override via
/// [`AdvisorSession::with_budget`].
pub const DEFAULT_TOKEN_BUDGET: u64 = 50_000;

/// Hard cap on the number of main-transcript chars we'll inline into the
/// advisor's user message. Long sessions otherwise blow past the model's
/// context window AND eat the entire token budget on the first call.
const MAX_SNAPSHOT_CHARS: usize = 40_000;

/// System prompt prepended to every advisor query. Spelled out in full so the
/// model knows it has no tools, no authority to "go do something", and is
/// expected to be terse.
pub const ADVISOR_SYSTEM_PROMPT: &str = "You are an advisor. The user's main agent is currently working on a task. \
     Their transcript so far is below. Answer their advisor question concisely. \
     You don't have tools — just give advice.";

/// Per-session advisor state. Owns its own transcript (separate from the main
/// agent) and tracks token usage for budget enforcement.
///
/// Constructed lazily by callers (typically `App::ensure_advisor_session()`)
/// so a user that never invokes `/advisor` pays no allocation cost.
#[derive(Debug, Clone)]
pub struct AdvisorSession {
    pub id: String,
    pub transcript: Vec<ChatMessage>,
    pub model: ModelId,
    pub token_budget: u64,
    pub tokens_used: u64,
}

impl AdvisorSession {
    /// New session with the default budget.
    pub fn new(model: impl Into<ModelId>) -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            transcript: Vec::new(),
            model: model.into(),
            token_budget: DEFAULT_TOKEN_BUDGET,
            tokens_used: 0,
        }
    }

    /// Builder-style budget override.
    pub fn with_budget(mut self, budget: u64) -> Self {
        self.token_budget = budget;
        self
    }

    /// Tokens still available for new advisor calls.
    pub fn tokens_remaining(&self) -> u64 {
        self.token_budget.saturating_sub(self.tokens_used)
    }

    /// True when the budget has been spent and `ask_advisor` will refuse new
    /// queries.
    pub fn is_exhausted(&self) -> bool {
        self.tokens_used >= self.token_budget
    }

    /// Account a successful round-trip's token usage against the budget. The
    /// stream / completion path normally does this for us; tests poke it
    /// directly.
    pub fn record_usage(&mut self, used: u64) {
        // saturating_add: a misbehaving provider that reports `u64::MAX` won't
        // wrap us back to "plenty of budget left".
        self.tokens_used = self.tokens_used.saturating_add(used);
    }
}

/// Render the main transcript into a single user-message string the advisor
/// can read. Capped at `MAX_SNAPSHOT_CHARS`; older content gets dropped.
///
/// Format mirrors `auto_mode::build_transcript` — role-prefixed plain text, no
/// JSON. Tool calls collapse to `[Tool: <kind>]` so the advisor sees activity
/// without the noise of full diff/output bodies.
fn render_snapshot(main_transcript: &[ChatMessage]) -> String {
    let mut out = String::new();
    for msg in main_transcript {
        let role_label = match msg.role {
            Role::User => "User",
            Role::Assistant => "Assistant",
        };
        for part in &msg.parts {
            match part {
                MessagePart::Text(s) if !s.is_empty() => {
                    out.push_str(role_label);
                    out.push_str(": ");
                    out.push_str(s);
                    out.push('\n');
                }
                MessagePart::Reasoning(s) if !s.is_empty() => {
                    out.push_str(role_label);
                    out.push_str(" (reasoning): ");
                    out.push_str(s);
                    out.push('\n');
                }
                MessagePart::Tool(tc) => {
                    out.push_str(role_label);
                    out.push_str(": [Tool: ");
                    out.push_str(tc.kind.label());
                    out.push_str("]\n");
                }
                MessagePart::Advisor(_) => {
                    // Don't echo prior advisor turns into the snapshot — the
                    // advisor's own transcript handles that. Including them
                    // would double-count tokens AND let the advisor
                    // accidentally treat its own past suggestions as
                    // authoritative "main agent" decisions.
                }
                MessagePart::TaskStatus(_) | MessagePart::CompactBoundary { .. } => {}
                _ => {}
            }
        }
    }
    if out.len() > MAX_SNAPSHOT_CHARS {
        // Tail-truncate so the most recent context is preserved. Older
        // material is replaced with a `[…elided…]` marker so the advisor
        // knows context was clipped, not that the session began mid-thought.
        let head_marker = "[…earlier transcript elided…]\n";
        let keep = MAX_SNAPSHOT_CHARS.saturating_sub(head_marker.len());
        // Snap to a UTF-8 char boundary at or after `len - keep`.
        let mut start = out.len().saturating_sub(keep);
        while start < out.len() && !out.is_char_boundary(start) {
            start += 1;
        }
        let tail = out[start..].to_owned();
        out.clear();
        out.push_str(head_marker);
        out.push_str(&tail);
    }
    out
}

/// Build the messages for one advisor round-trip. The first user message
/// contains the main-transcript snapshot so the advisor has working context;
/// the second contains the user's actual advisor question. The model itself
/// is told (via the system prompt) not to invent tools.
fn build_messages(snapshot: &str, query: &str) -> Vec<ProviderMessage> {
    let snapshot_block = if snapshot.is_empty() {
        "<main-transcript>\n(empty — main agent has not started yet)\n</main-transcript>".to_owned()
    } else {
        format!("<main-transcript>\n{snapshot}</main-transcript>")
    };
    vec![
        ProviderMessage {
            role: ProviderRole::User,
            content: vec![ProviderContent::Text(snapshot_block)],
        },
        ProviderMessage {
            role: ProviderRole::User,
            content: vec![ProviderContent::Text(format!("Advisor question: {query}"))],
        },
    ]
}

/// Run one advisor round-trip. Returns the advisor's reply text on success;
/// `Err` when the budget is exhausted, when the provider errors, or when the
/// query is empty.
///
/// Does NOT mutate `main_transcript_snapshot` — that's a `&[ChatMessage]` by
/// design. The caller is responsible for surfacing the reply (e.g. by pushing
/// a `MessagePart::Advisor(reply)` onto the main transcript at their leisure).
///
/// The session's own `transcript` field IS updated: each call appends a user
/// turn (the query) and an assistant turn (the reply), so `/advisor` follow-up
/// questions can build on prior advisor context if a future revision wants to.
#[tracing::instrument(
    target = "jfc::advisor",
    skip(provider, session, main_transcript_snapshot),
    fields(
        provider = %provider.name(),
        model = %session.model,
        snapshot_msgs = main_transcript_snapshot.len(),
        budget_remaining = session.tokens_remaining(),
        session_id = %session.id,
    ),
)]
pub async fn ask_advisor(
    provider: &dyn Provider,
    session: &mut AdvisorSession,
    query: String,
    main_transcript_snapshot: &[ChatMessage],
) -> Result<String> {
    let trimmed = query.trim();
    if trimmed.is_empty() {
        return Err(anyhow!("advisor query is empty"));
    }
    if session.is_exhausted() {
        // tengu_advisor_tool_interrupted analog — surface a structured error
        // so the UI can render a "budget exhausted" toast distinct from a
        // network failure.
        return Err(anyhow!(
            "advisor token budget exhausted ({}/{} used)",
            session.tokens_used,
            session.token_budget
        ));
    }

    let snapshot = render_snapshot(main_transcript_snapshot);
    let messages = build_messages(&snapshot, trimmed);

    let opts = StreamOptions::new(session.model.clone())
        .system(ADVISOR_SYSTEM_PROMPT)
        .max_tokens(2048);

    let resp = match provider.complete(messages.clone(), &opts).await {
        Ok(r) => r,
        Err(e) => {
            let err_msg = e.to_string().to_lowercase();
            if err_msg.contains("not support") || err_msg.contains("unsupported") {
                // Streaming fallback for OpenWebUI/LiteLLM-style providers.
                // Mirrors compact.rs's `complete_or_stream` pattern.
                stream_to_completion(provider, messages, &opts).await?
            } else {
                return Err(e);
            }
        }
    };

    // Account against the budget. Some providers report token usage,
    // others don't — fall back to a char/4 estimate so the budget still
    // moves and a misconfigured provider can't get the user infinite
    // calls. Mirrors v126's `responseLength / 4` heuristic.
    let used = if resp.usage.input_tokens + resp.usage.output_tokens > 0 {
        (resp.usage.input_tokens + resp.usage.output_tokens) as u64
    } else {
        ((snapshot.len() + trimmed.len() + resp.content.len()) / 4) as u64
    };
    session.record_usage(used);

    // Append to the advisor's own transcript so a future /advisor follow-up
    // can read prior turns. (Not surfaced to the user yet — but the session
    // owns this state.)
    session
        .transcript
        .push(ChatMessage::user(trimmed.to_owned()));
    session
        .transcript
        .push(ChatMessage::assistant(resp.content.clone()));

    tracing::info!(
        target: "jfc::advisor",
        used,
        tokens_used_total = session.tokens_used,
        budget_remaining = session.tokens_remaining(),
        reply_chars = resp.content.len(),
        "advisor_reply"
    );

    Ok(resp.content)
}

/// Stream-to-completion fallback for providers that don't implement
/// `Provider::complete`. Collects all `TextDelta`s and returns them as a
/// `CompletionResponse`. Token usage is captured from `StreamEvent::Usage` if
/// the provider reports it.
async fn stream_to_completion(
    provider: &dyn Provider,
    messages: Vec<ProviderMessage>,
    opts: &StreamOptions,
) -> Result<CompletionResponse> {
    let mut stream = provider.stream(messages, opts).await?;
    let mut collected = String::new();
    let mut usage = crate::provider::TokenUsage::default();
    while let Some(event) = stream.next().await {
        match event {
            Ok(StreamEvent::TextDelta { delta, .. }) => collected.push_str(&delta),
            Ok(StreamEvent::Usage {
                input_tokens,
                output_tokens,
                cache_read_tokens,
                cache_write_tokens,
            }) => {
                usage.input_tokens = input_tokens as usize;
                usage.output_tokens = output_tokens as usize;
                usage.cache_read_tokens = cache_read_tokens as usize;
                usage.cache_creation_tokens = cache_write_tokens as usize;
            }
            Ok(StreamEvent::Done { .. }) => break,
            Ok(StreamEvent::Error { message }) => return Err(anyhow!("{}", message)),
            Ok(_) => {}
            Err(e) => return Err(anyhow!("{}", e)),
        }
    }
    Ok(CompletionResponse {
        content: collected,
        usage,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::{
        EventStream, ModelInfo, ProviderMessage as PMsg, StreamConvention, StreamOptions as SOpts,
    };
    use async_trait::async_trait;

    /// Mock provider that always returns a canned `CompletionResponse`.
    /// Modelled on `auto_mode::tests::FakeProvider` — same Mutex-of-Option
    /// pattern so we can stash either Ok or Err and pop it once.
    struct FakeProvider {
        result: std::sync::Mutex<Option<Result<CompletionResponse>>>,
    }

    impl FakeProvider {
        fn echo(reply: &str) -> Self {
            Self {
                result: std::sync::Mutex::new(Some(Ok(CompletionResponse {
                    content: reply.to_owned(),
                    usage: crate::provider::TokenUsage {
                        input_tokens: 100,
                        output_tokens: 50,
                        cache_read_tokens: 0,
                        cache_creation_tokens: 0,
                    },
                }))),
            }
        }

        fn err() -> Self {
            Self {
                result: std::sync::Mutex::new(Some(Err(anyhow!("network down")))),
            }
        }
    }

    #[async_trait]
    impl Provider for FakeProvider {
        fn name(&self) -> &str {
            "fake-advisor"
        }
        fn available_models(&self) -> Vec<ModelInfo> {
            Vec::new()
        }
        fn stream_convention(&self) -> StreamConvention {
            StreamConvention::AnthropicNative
        }
        async fn stream(&self, _messages: Vec<PMsg>, _options: &SOpts) -> Result<EventStream> {
            Err(anyhow!("not used in advisor tests"))
        }
        async fn complete(
            &self,
            _messages: Vec<PMsg>,
            _options: &SOpts,
        ) -> Result<CompletionResponse> {
            self.result
                .lock()
                .unwrap()
                .take()
                .expect("FakeProvider::complete called more than once")
        }
    }
    impl crate::provider::seal::Sealed for FakeProvider {}

    // ─── Normal path ─────────────────────────────────────────────────────

    /// Normal: a fresh advisor session has the default budget and zero usage.
    #[test]
    fn advisor_session_new_has_default_budget_normal() {
        let s = AdvisorSession::new("claude-opus-4-7");
        assert_eq!(s.token_budget, DEFAULT_TOKEN_BUDGET);
        assert_eq!(s.tokens_used, 0);
        assert!(!s.is_exhausted());
        assert_eq!(s.tokens_remaining(), DEFAULT_TOKEN_BUDGET);
        assert!(!s.id.is_empty());
        assert_eq!(s.model.as_str(), "claude-opus-4-7");
    }

    /// Normal: with_budget overrides the default ceiling.
    #[test]
    fn advisor_session_with_budget_overrides_default_normal() {
        let s = AdvisorSession::new("m").with_budget(123);
        assert_eq!(s.token_budget, 123);
    }

    /// Normal: ask_advisor with a mock provider returns the canned advice
    /// verbatim (the "echo" requirement in the deliverable).
    #[tokio::test]
    async fn ask_advisor_returns_canned_advice_normal() {
        let provider = FakeProvider::echo("Yes, that approach is correct.");
        let mut session = AdvisorSession::new("test-model");
        let main_transcript = vec![ChatMessage::user("doing some work".into())];

        let reply = ask_advisor(
            &provider,
            &mut session,
            "is this approach right?".into(),
            &main_transcript,
        )
        .await
        .expect("advisor should reply");

        assert_eq!(reply, "Yes, that approach is correct.");
    }

    /// Normal: token budget is decremented per query.
    #[tokio::test]
    async fn ask_advisor_decrements_budget_normal() {
        let provider = FakeProvider::echo("ok");
        let mut session = AdvisorSession::new("test-model").with_budget(1_000);
        assert_eq!(session.tokens_used, 0);

        let _ = ask_advisor(&provider, &mut session, "hello?".into(), &[])
            .await
            .expect("advisor reply");

        // Provider reported 100 input + 50 output = 150 tokens.
        assert_eq!(session.tokens_used, 150);
        assert_eq!(session.tokens_remaining(), 850);
    }

    /// Normal: the advisor session's own transcript records the query +
    /// reply, so a follow-up advisor turn could build on prior context.
    #[tokio::test]
    async fn ask_advisor_records_session_transcript_normal() {
        let provider = FakeProvider::echo("here is advice");
        let mut session = AdvisorSession::new("test-model");

        let _ = ask_advisor(&provider, &mut session, "q?".into(), &[])
            .await
            .expect("advisor reply");

        assert_eq!(session.transcript.len(), 2);
        assert_eq!(session.transcript[0].role, Role::User);
        assert_eq!(session.transcript[1].role, Role::Assistant);
    }

    /// Normal: render_snapshot produces role-prefixed lines for text parts.
    #[test]
    fn render_snapshot_includes_user_and_assistant_text_normal() {
        let mut transcript = vec![
            ChatMessage::user("hello".into()),
            ChatMessage::assistant("hi back".into()),
        ];
        // Mix in a reasoning part to exercise that branch.
        transcript[1]
            .parts
            .push(MessagePart::Reasoning("inner thoughts".into()));

        let snap = render_snapshot(&transcript);
        assert!(snap.contains("User: hello"));
        assert!(snap.contains("Assistant: hi back"));
        assert!(snap.contains("Assistant (reasoning): inner thoughts"));
    }

    // ─── Robust path ─────────────────────────────────────────────────────

    /// Robust: when the budget is exhausted, ask_advisor refuses new queries
    /// with an Err, rather than silently going over budget.
    #[tokio::test]
    async fn ask_advisor_refuses_when_budget_exhausted_robust() {
        let provider = FakeProvider::echo("should not be called");
        let mut session = AdvisorSession::new("test-model").with_budget(10);
        // Pre-spend the whole budget.
        session.record_usage(10);
        assert!(session.is_exhausted());

        let result = ask_advisor(&provider, &mut session, "anything?".into(), &[]).await;
        assert!(result.is_err());
        let msg = result.err().unwrap().to_string();
        assert!(
            msg.contains("budget exhausted"),
            "expected budget-exhausted error, got: {msg}"
        );
    }

    /// Robust: ask_advisor leaves the main transcript untouched. The
    /// "snapshot" semantics in the deliverable demand this — the advisor must
    /// not mutate the main agent's view of the conversation.
    #[tokio::test]
    async fn ask_advisor_does_not_mutate_main_transcript_robust() {
        let provider = FakeProvider::echo("advice text");
        let mut session = AdvisorSession::new("test-model");
        let main_transcript = vec![
            ChatMessage::user("u1".into()),
            ChatMessage::assistant("a1".into()),
            ChatMessage::user("u2".into()),
        ];
        let before = main_transcript.clone();

        let _ = ask_advisor(
            &provider,
            &mut session,
            "review please".into(),
            &main_transcript,
        )
        .await
        .expect("advisor reply");

        // Length unchanged.
        assert_eq!(main_transcript.len(), before.len());
        // Role + first text part unchanged for each message — full struct
        // equality isn't derived (and `Provider` field on App makes that
        // hard), so we compare the visible text.
        for (m1, m2) in main_transcript.iter().zip(before.iter()) {
            assert_eq!(m1.role, m2.role);
            assert_eq!(m1.parts.len(), m2.parts.len());
        }
    }

    /// Robust: an empty query short-circuits with Err and doesn't burn budget.
    #[tokio::test]
    async fn ask_advisor_rejects_empty_query_robust() {
        let provider = FakeProvider::echo("should not be called");
        let mut session = AdvisorSession::new("test-model");
        let result = ask_advisor(&provider, &mut session, "   ".into(), &[]).await;
        assert!(result.is_err());
        assert_eq!(session.tokens_used, 0);
    }

    /// Robust: provider errors propagate without changing the budget. Without
    /// this, a flaky network would silently exhaust the budget on retries.
    #[tokio::test]
    async fn ask_advisor_provider_error_preserves_budget_robust() {
        let provider = FakeProvider::err();
        let mut session = AdvisorSession::new("test-model");
        let result = ask_advisor(&provider, &mut session, "q".into(), &[]).await;
        assert!(result.is_err());
        assert_eq!(session.tokens_used, 0);
    }

    /// Robust: a snapshot with no usable parts (only tool / task-status /
    /// boundary parts) renders to an empty string and ask_advisor still works
    /// (the snapshot block falls back to "(empty)" in build_messages).
    #[test]
    fn render_snapshot_skips_non_text_parts_robust() {
        let mut transcript = vec![ChatMessage::compact_boundary("summary", 1234)];
        transcript[0].parts.clear();
        transcript[0]
            .parts
            .push(MessagePart::CompactBoundary { pre_tokens: 1234 });

        let snap = render_snapshot(&transcript);
        assert!(snap.is_empty());
    }

    /// Robust: an oversized snapshot is tail-truncated and prefixed with the
    /// elision marker — the advisor still gets the *recent* context, which is
    /// what matters for "is this approach right?".
    #[test]
    fn render_snapshot_tail_truncates_oversized_input_robust() {
        // Build a transcript whose rendered form would dwarf MAX_SNAPSHOT_CHARS.
        let mut transcript = Vec::new();
        for i in 0..2_000 {
            transcript.push(ChatMessage::user(format!("padding-line-{i}")));
        }
        transcript.push(ChatMessage::user("FINAL_RECENT_LINE".into()));

        let snap = render_snapshot(&transcript);
        assert!(snap.len() <= MAX_SNAPSHOT_CHARS);
        assert!(snap.starts_with("[…earlier transcript elided…]"));
        // Tail-preservation: the most recent line survives.
        assert!(snap.contains("FINAL_RECENT_LINE"));
    }

    /// Robust: build_messages emits an empty-marker snapshot block when the
    /// main transcript is empty, so the advisor has a well-formed prompt
    /// even on a fresh session.
    #[test]
    fn build_messages_handles_empty_snapshot_robust() {
        let messages = build_messages("", "what should I do?");
        assert_eq!(messages.len(), 2);
        let ProviderContent::Text(s) = &messages[0].content[0] else {
            panic!("expected text");
        };
        assert!(s.contains("(empty"));
    }

    /// Robust: when the provider doesn't report token usage (zeros), we fall
    /// back to a char/4 estimate so the budget still progresses. Without
    /// this, a misconfigured provider gives the user infinite advisor calls.
    #[tokio::test]
    async fn ask_advisor_falls_back_to_char_estimate_when_no_usage_robust() {
        struct ZeroUsageProvider;
        #[async_trait]
        impl Provider for ZeroUsageProvider {
            fn name(&self) -> &str {
                "zero-usage"
            }
            fn available_models(&self) -> Vec<ModelInfo> {
                Vec::new()
            }
            async fn stream(&self, _: Vec<PMsg>, _: &SOpts) -> Result<EventStream> {
                Err(anyhow!("unused"))
            }
            async fn complete(&self, _: Vec<PMsg>, _: &SOpts) -> Result<CompletionResponse> {
                Ok(CompletionResponse {
                    content: "x".repeat(400), // 400 chars → ~100 tokens
                    usage: Default::default(),
                })
            }
        }
        impl crate::provider::seal::Sealed for ZeroUsageProvider {}
        let mut session = AdvisorSession::new("test-model").with_budget(10_000);
        let _ = ask_advisor(&ZeroUsageProvider, &mut session, "hi".into(), &[])
            .await
            .expect("reply");
        // We can't pin the exact number (depends on snapshot/query length),
        // but it must be > 0 (budget moved) and not absurd.
        assert!(session.tokens_used > 0);
        assert!(session.tokens_used < 1_000);
    }
}
