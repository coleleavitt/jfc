//! One shared one-shot completion path.
//!
//! Advisor, council, and research each used to carry a near-verbatim copy of
//! the same "prefer `Provider::complete`, else drain a stream" logic plus the
//! same `StreamEvent` fold. This module is the single owner of that path so the
//! fallback detection and the token-usage fold cannot drift between callers.
//!
//! Scope boundary (deliberate): this module owns *transport* — turning a
//! tool-less request into a [`CompletionResponse`] with full [`TokenUsage`]. It
//! does **not** own accounting. Each caller keeps its own budget/usage sink
//! (`AdvisorSession`, `CouncilBudget`, the economy ledger) and its own
//! chars/4 estimate for providers that report no usage. That stays caller-side
//! until accounting is unified separately.
//!
//! The live-streaming chat path (`stream/`) and economy's UI-event loop are
//! intentionally *not* routed through here: they stream deltas to the UI rather
//! than collecting to completion, which is a different abstraction.

use anyhow::{Result, anyhow};
use futures::StreamExt;
use jfc_provider::{
    CompletionResponse, Provider, ProviderMessage, StreamEvent, StreamOptions, TokenUsage,
};

/// Run one tool-less completion.
///
/// Prefers the provider's native non-streaming [`Provider::complete`] and only
/// falls back to draining a stream when the provider does not implement it.
/// Any other `complete()` error propagates unchanged so real failures are not
/// masked by the fallback.
pub async fn complete_once(
    provider: &dyn Provider,
    messages: Vec<ProviderMessage>,
    opts: &StreamOptions,
) -> Result<CompletionResponse> {
    match provider.complete(messages.clone(), opts).await {
        Ok(resp) => Ok(resp),
        Err(e) => {
            if completion_unsupported(&e) {
                stream_to_completion(provider, messages, opts).await
            } else {
                Err(e)
            }
        }
    }
}

/// True when a `complete()` error means "this provider has no non-streaming
/// endpoint", so the caller should fall back to draining a stream.
///
/// Matches the string signal every caller used before unification: the default
/// `Provider::complete` impl bails with `"<name> does not support non-streaming
/// completion"`.
fn completion_unsupported(error: &anyhow::Error) -> bool {
    let lower = error.to_string().to_lowercase();
    lower.contains("not support") || lower.contains("unsupported")
}

/// Drain a provider stream into a single [`CompletionResponse`].
///
/// Collects text deltas and the full four-field [`TokenUsage`]. When no text
/// deltas arrive, falls back to the terminal `TextDone` block (some providers
/// only emit the final block, never incremental deltas). Token usage is left at
/// its zero default when the provider reports none — callers apply their own
/// estimate in that case.
pub async fn stream_to_completion(
    provider: &dyn Provider,
    messages: Vec<ProviderMessage>,
    opts: &StreamOptions,
) -> Result<CompletionResponse> {
    let mut stream = provider.stream(messages, opts).await?;
    let mut content = String::new();
    let mut usage = TokenUsage::default();
    while let Some(event) = stream.next().await {
        match event? {
            StreamEvent::TextDelta { delta, .. } => content.push_str(&delta),
            StreamEvent::TextDone { text, .. } => {
                if content.is_empty() {
                    content = text;
                }
            }
            StreamEvent::Usage {
                input_tokens,
                output_tokens,
                thinking_tokens,
                cache_read_tokens,
                cache_write_tokens,
            } => {
                usage.input_tokens = input_tokens as usize;
                usage.output_tokens = output_tokens as usize;
                usage.thinking_tokens = thinking_tokens.map(|tokens| tokens as usize);
                usage.cache_read_tokens = cache_read_tokens as usize;
                usage.cache_creation_tokens = cache_write_tokens as usize;
            }
            StreamEvent::Error { message } => return Err(anyhow!("{message}")),
            StreamEvent::Done { .. } => break,
            // Keepalive, thinking, tool-call, and metadata frames carry no
            // collectable text for a tool-less completion. Trace them rather
            // than dropping silently so an unexpected frame is still visible.
            other => tracing::trace!(
                target: "jfc::prompt_executor",
                event = ?other,
                "ignoring non-text frame in one-shot completion"
            ),
        }
    }
    Ok(CompletionResponse {
        content,
        usage,
        context_signals: None,
        reasoning: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use jfc_provider::{EventStream, ModelInfo};
    use std::sync::Mutex;

    /// Provider that drives one of: a native `complete()` result, or a scripted
    /// list of stream events. `complete_supported = false` makes `complete()`
    /// bail with the canonical "does not support" message so `complete_once`
    /// exercises the stream fallback — mirroring the real default impl.
    struct ScriptedProvider {
        complete_supported: bool,
        complete_result: Mutex<Option<Result<CompletionResponse>>>,
        events: Mutex<Option<Vec<StreamEvent>>>,
    }

    impl ScriptedProvider {
        fn streaming(events: Vec<StreamEvent>) -> Self {
            Self {
                complete_supported: false,
                complete_result: Mutex::new(None),
                events: Mutex::new(Some(events)),
            }
        }

        fn completing(resp: CompletionResponse) -> Self {
            Self {
                complete_supported: true,
                complete_result: Mutex::new(Some(Ok(resp))),
                events: Mutex::new(None),
            }
        }
    }

    impl jfc_provider::seal::Sealed for ScriptedProvider {}

    #[async_trait]
    impl Provider for ScriptedProvider {
        fn name(&self) -> &str {
            "scripted"
        }

        fn available_models(&self) -> Vec<ModelInfo> {
            Vec::new()
        }

        async fn stream(
            &self,
            _messages: Vec<ProviderMessage>,
            _options: &StreamOptions,
        ) -> anyhow::Result<EventStream> {
            let events = self
                .events
                .lock()
                .unwrap()
                .take()
                .expect("stream called more than once");
            let stream = futures::stream::iter(events.into_iter().map(Ok));
            Ok(Box::pin(stream))
        }

        async fn complete(
            &self,
            _messages: Vec<ProviderMessage>,
            _options: &StreamOptions,
        ) -> anyhow::Result<CompletionResponse> {
            if !self.complete_supported {
                anyhow::bail!("{} does not support non-streaming completion", self.name());
            }
            self.complete_result
                .lock()
                .unwrap()
                .take()
                .expect("complete called more than once")
        }
    }

    fn opts() -> StreamOptions {
        StreamOptions::new("test-model")
    }

    // Normal: a streaming provider's text deltas are concatenated and the full
    // four-field usage is captured verbatim (no estimation, no truncation).
    #[tokio::test]
    async fn stream_to_completion_collects_text_and_full_usage_normal() {
        let provider = ScriptedProvider::streaming(vec![
            StreamEvent::TextDelta {
                index: 0,
                delta: "Hello, ".into(),
            },
            StreamEvent::TextDelta {
                index: 0,
                delta: "world".into(),
            },
            StreamEvent::Usage {
                input_tokens: 11,
                output_tokens: 7,
                thinking_tokens: Some(3),
                cache_read_tokens: 3,
                cache_write_tokens: 5,
            },
            StreamEvent::Done {
                stop_reason: jfc_provider::StopReason::EndTurn,
            },
        ]);
        let resp = stream_to_completion(&provider, vec![], &opts())
            .await
            .unwrap();
        assert_eq!(resp.content, "Hello, world");
        assert_eq!(resp.usage.input_tokens, 11);
        assert_eq!(resp.usage.output_tokens, 7);
        assert_eq!(resp.usage.cache_read_tokens, 3);
        assert_eq!(resp.usage.cache_creation_tokens, 5);
    }

    // Robust: a provider that emits only a terminal TextDone (no deltas) still
    // yields the full text — the fallback path the per-caller copies lacked.
    #[tokio::test]
    async fn stream_to_completion_falls_back_to_text_done_robust() {
        let provider = ScriptedProvider::streaming(vec![
            StreamEvent::TextDone {
                index: 0,
                text: "final only".into(),
            },
            StreamEvent::Done {
                stop_reason: jfc_provider::StopReason::EndTurn,
            },
        ]);
        let resp = stream_to_completion(&provider, vec![], &opts())
            .await
            .unwrap();
        assert_eq!(resp.content, "final only");
        // No Usage event → zero usage; callers estimate from chars themselves.
        assert_eq!(resp.usage.input_tokens, 0);
        assert_eq!(resp.usage.output_tokens, 0);
    }

    // Robust: an Error frame mid-stream surfaces as an Err, not a truncated Ok.
    #[tokio::test]
    async fn stream_to_completion_surfaces_error_frame_robust() {
        let provider = ScriptedProvider::streaming(vec![
            StreamEvent::TextDelta {
                index: 0,
                delta: "partial".into(),
            },
            StreamEvent::Error {
                message: "boom".into(),
            },
        ]);
        let err = stream_to_completion(&provider, vec![], &opts())
            .await
            .unwrap_err();
        assert!(err.to_string().contains("boom"));
    }

    // Normal: when the provider supports complete(), complete_once returns it
    // directly without ever touching the stream path.
    #[tokio::test]
    async fn complete_once_prefers_native_complete_normal() {
        let provider = ScriptedProvider::completing(CompletionResponse {
            content: "native".into(),
            usage: TokenUsage {
                input_tokens: 4,
                output_tokens: 2,
                thinking_tokens: None,
                cache_read_tokens: 0,
                cache_creation_tokens: 0,
            },
            context_signals: None,
            reasoning: None,
        });
        let resp = complete_once(&provider, vec![], &opts()).await.unwrap();
        assert_eq!(resp.content, "native");
        assert_eq!(resp.usage.input_tokens, 4);
    }

    // Robust: a provider whose complete() bails "does not support" transparently
    // falls back to the stream path.
    #[tokio::test]
    async fn complete_once_falls_back_when_unsupported_robust() {
        let provider = ScriptedProvider::streaming(vec![
            StreamEvent::TextDelta {
                index: 0,
                delta: "streamed".into(),
            },
            StreamEvent::Done {
                stop_reason: jfc_provider::StopReason::EndTurn,
            },
        ]);
        let resp = complete_once(&provider, vec![], &opts()).await.unwrap();
        assert_eq!(resp.content, "streamed");
    }

    // Robust: the "unsupported" sniff is case-insensitive and matches both the
    // canonical "does not support" phrasing and a bare "unsupported".
    #[test]
    fn completion_unsupported_matches_known_signals_robust() {
        assert!(completion_unsupported(&anyhow!(
            "scripted does not support non-streaming completion"
        )));
        assert!(completion_unsupported(&anyhow!("UNSUPPORTED operation")));
        assert!(!completion_unsupported(&anyhow!("rate limit exceeded")));
    }
}
