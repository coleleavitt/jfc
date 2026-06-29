//! Shared per-turn stream drain for agent loops.
//!
//! The subagent (`tools::execute_task`) and teammate (`swarm::executor::
//! run_single_turn`) turn loops each inlined the same ~100-line drain:
//! pull `StreamEvent`s, accumulate text / tool uses / stop reason, fold
//! usage via a baseline so cumulative `output_tokens` snapshots become
//! per-turn deltas, classify retryable provider errors, and honour
//! cancellation. The copies had already drifted (the teammate ignored the
//! terminal `TextDone` fallback; the subagent didn't track estimated
//! tokens). This module is the single driver; each caller keeps its own
//! identity via a sink callback (live UI deltas) and a cancellation source.

use futures::StreamExt;
use futures::future::BoxFuture;
use jfc_provider::{StopReason, StreamEvent};

/// The caller's incremental-event sink: takes an owned [`AgentDrainEvent`]
/// and returns a `'static` boxed future (clone channel senders into it). The
/// boxed-`'static` shape sidesteps the higher-ranked-lifetime `Send` traps an
/// `AsyncFnMut(&...)` sink hits when the drain runs inside `tokio::spawn`.
pub type DrainSink<'a> = dyn FnMut(AgentDrainEvent) -> BoxFuture<'static, ()> + Send + 'a;

/// One tool call the model requested this turn, in raw wire form. Parsing
/// into `ToolInput` stays with the caller (the teammate validates shape at
/// drain time, the subagent at dispatch time) — both operate on this data.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentToolUse {
    pub id: String,
    pub name: String,
    pub input_json: String,
    /// Gemini 3.x thought signature, round-tripped on the next turn to keep
    /// multi-turn agentic tool calls coherent.
    pub thought_signature: Option<String>,
}

/// Everything one drained turn produced.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AgentTurn {
    pub text: String,
    pub tool_uses: Vec<AgentToolUse>,
    pub stop_reason: Option<StopReason>,
    /// Whether the provider reported usage this turn. When `false`, callers
    /// fall back to `estimated_tokens`.
    pub saw_usage: bool,
    /// `len/4` text-delta estimate, accumulated only while no usage event has
    /// arrived (the teammate's spinner heuristic).
    pub estimated_tokens: u64,
    pub input_tokens: u64,
    pub cache_read_tokens: u64,
    pub cache_write_tokens: u64,
    /// Per-turn output tokens: providers snapshot *cumulative* output in each
    /// usage event, so the driver folds them through a baseline into deltas.
    pub output_tokens: u64,
}

/// Incremental events surfaced to the caller's sink while draining — the
/// hook each loop uses to keep its UI live (task-panel chunks, token
/// counters, last-tool labels) without owning the drain. Owned data so the
/// sink can be async (forwarding over a bounded channel applies backpressure
/// to the stream instead of dropping deltas — the no-loss semantics both
/// original loops had). The delta clone matches what both inlined drains did.
pub enum AgentDrainEvent {
    TextDelta(String),
    /// A usage report, with `output_delta` already baseline-folded.
    Usage {
        input_tokens: u64,
        cache_read_tokens: u64,
        cache_write_tokens: u64,
        output_delta: u64,
    },
    ToolUse {
        name: String,
        input_json: String,
    },
}

/// How the caller wants cancellation observed.
pub enum DrainCancel<'a> {
    /// Poll a flag between events (the subagent's daemon cancel marker).
    /// `Sync` so the drain future stays `Send` across spawn boundaries.
    Poll(&'a (dyn Fn() -> bool + Sync)),
    /// Wake on a watch-channel flip even mid-await (the teammate's abort).
    Watch(&'a mut tokio::sync::watch::Receiver<bool>),
}

/// Why the drain stopped.
#[derive(Debug)]
pub enum AgentDrainOutcome {
    /// Stream completed (or ended) normally.
    Completed(AgentTurn),
    /// A provider error the retry policy classifies as retryable — the caller
    /// re-opens the stream after its own backoff/logging.
    Retryable(String),
    /// A non-retryable stream error.
    Fatal(String),
    /// The caller's cancellation source fired.
    Cancelled,
}

/// Drain one provider stream into an [`AgentTurn`], forwarding incremental
/// events to `sink`. Shared by the subagent and teammate turn loops.
pub async fn drain_agent_stream<S>(
    stream: S,
    cancel: DrainCancel<'_>,
    sink: &mut DrainSink<'_>,
) -> AgentDrainOutcome
where
    S: futures::Stream<Item = anyhow::Result<StreamEvent>>,
{
    futures::pin_mut!(stream);
    let mut turn = AgentTurn::default();
    let mut usage_output_baseline = 0u32;
    let mut cancel = cancel;

    loop {
        // Cancellation, in the caller's preferred style.
        let event_result = match &mut cancel {
            DrainCancel::Poll(cancelled) => {
                if cancelled() {
                    return AgentDrainOutcome::Cancelled;
                }
                stream.next().await
            }
            DrainCancel::Watch(rx) => {
                if *rx.borrow() {
                    return AgentDrainOutcome::Cancelled;
                }
                tokio::select! {
                    biased;
                    changed = rx.changed() => {
                        if changed.is_err() || *rx.borrow() {
                            return AgentDrainOutcome::Cancelled;
                        }
                        continue;
                    }
                    event_result = stream.next() => event_result,
                }
            }
        };

        let Some(event_result) = event_result else {
            break;
        };
        let event = match event_result {
            Ok(e) => e,
            Err(e) => {
                let message = e.to_string();
                return if jfc_provider::retry::retryable_stream_error(&message).is_some() {
                    AgentDrainOutcome::Retryable(message)
                } else {
                    AgentDrainOutcome::Fatal(message)
                };
            }
        };

        if let Some(outcome) = apply_event(event, &mut turn, &mut usage_output_baseline, sink).await
        {
            return outcome;
        }
    }

    AgentDrainOutcome::Completed(turn)
}

/// Fold one stream event into the turn (and the caller's sink). Returns
/// `Some(outcome)` to stop the drain (stream error), `None` to keep going.
async fn apply_event(
    event: StreamEvent,
    turn: &mut AgentTurn,
    usage_output_baseline: &mut u32,
    sink: &mut DrainSink<'_>,
) -> Option<AgentDrainOutcome> {
    match event {
        StreamEvent::TextDelta { delta, .. } => {
            if !turn.saw_usage {
                turn.estimated_tokens += (delta.len() / 4) as u64;
            }
            sink(AgentDrainEvent::TextDelta(delta.clone())).await;
            turn.text.push_str(&delta);
        }
        // Terminal text block: fall back to it when no deltas arrived (some
        // providers emit only the final block). Previously only the subagent
        // path had this; the teammate silently lost such turns.
        StreamEvent::TextDone { text, .. } => {
            if turn.text.is_empty() {
                sink(AgentDrainEvent::TextDelta(text.clone())).await;
                turn.text = text;
            }
        }
        StreamEvent::ToolDone {
            tool_name,
            tool_use_id,
            input_json,
            thought_signature,
            ..
        } => {
            sink(AgentDrainEvent::ToolUse {
                name: tool_name.clone(),
                input_json: input_json.clone(),
            })
            .await;
            turn.tool_uses.push(AgentToolUse {
                id: tool_use_id,
                name: tool_name,
                input_json,
                thought_signature,
            });
        }
        StreamEvent::Usage {
            input_tokens,
            output_tokens,
            thinking_tokens: _,
            cache_read_tokens,
            cache_write_tokens,
        } => {
            let output_delta = output_tokens.saturating_sub(*usage_output_baseline) as u64;
            *usage_output_baseline = output_tokens;
            turn.saw_usage = true;
            turn.input_tokens = input_tokens as u64;
            turn.cache_read_tokens = cache_read_tokens as u64;
            turn.cache_write_tokens = cache_write_tokens as u64;
            turn.output_tokens = turn.output_tokens.saturating_add(output_delta);
            sink(AgentDrainEvent::Usage {
                input_tokens: turn.input_tokens,
                cache_read_tokens: turn.cache_read_tokens,
                cache_write_tokens: turn.cache_write_tokens,
                output_delta,
            })
            .await;
        }
        StreamEvent::Done { stop_reason } => {
            turn.stop_reason = Some(stop_reason);
        }
        StreamEvent::Error { message } => {
            return Some(
                if jfc_provider::retry::retryable_stream_error(&message).is_some() {
                    AgentDrainOutcome::Retryable(message)
                } else {
                    AgentDrainOutcome::Fatal(message)
                },
            );
        }
        // Thinking / metadata / keepalive frames carry nothing the agent
        // loops collect; trace rather than drop silently so an unexpected
        // frame is still diagnosable.
        other => tracing::trace!(
            target: "jfc::stream::agent_drain",
            category = ?other.category(),
            "agent drain ignoring non-collected frame"
        ),
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scripted(
        events: Vec<StreamEvent>,
    ) -> impl futures::Stream<Item = anyhow::Result<StreamEvent>> {
        futures::stream::iter(events.into_iter().map(Ok))
    }

    fn no_cancel() -> bool {
        false
    }

    fn noop_sink() -> impl FnMut(AgentDrainEvent) -> futures::future::BoxFuture<'static, ()> + Send
    {
        |_| Box::pin(async {})
    }

    // The driver accumulates text, tool uses, stop reason, and per-turn usage
    // deltas exactly like both inlined drains did.
    #[tokio::test(flavor = "current_thread")]
    async fn drain_accumulates_turn_normal() {
        let events = vec![
            StreamEvent::TextDelta {
                index: 0,
                delta: "hello ".into(),
            },
            StreamEvent::TextDelta {
                index: 0,
                delta: "world".into(),
            },
            StreamEvent::ToolDone {
                index: 1,
                tool_name: "Bash".into(),
                tool_use_id: "t1".into(),
                input_json: "{\"command\":\"ls\"}".into(),
                thought_signature: None,
            },
            // Two cumulative usage snapshots → output deltas 10 then 5.
            StreamEvent::Usage {
                input_tokens: 100,
                output_tokens: 10,
                thinking_tokens: None,
                cache_read_tokens: 7,
                cache_write_tokens: 3,
            },
            StreamEvent::Usage {
                input_tokens: 100,
                output_tokens: 15,
                thinking_tokens: None,
                cache_read_tokens: 7,
                cache_write_tokens: 3,
            },
            StreamEvent::Done {
                stop_reason: StopReason::ToolUse,
            },
        ];
        let deltas = std::sync::Arc::new(std::sync::Mutex::new(String::new()));
        let last_tool = std::sync::Arc::new(std::sync::Mutex::new(None::<String>));
        let last_tool_input = std::sync::Arc::new(std::sync::Mutex::new(None::<String>));
        let (d2, t2, i2) = (deltas.clone(), last_tool.clone(), last_tool_input.clone());
        let mut sink = move |ev: AgentDrainEvent| -> futures::future::BoxFuture<'static, ()> {
            match ev {
                AgentDrainEvent::TextDelta(d) => d2.lock().unwrap().push_str(&d),
                AgentDrainEvent::ToolUse { name, input_json } => {
                    *t2.lock().unwrap() = Some(name);
                    *i2.lock().unwrap() = Some(input_json);
                }
                AgentDrainEvent::Usage { .. } => {}
            }
            Box::pin(async {})
        };
        let outcome =
            drain_agent_stream(scripted(events), DrainCancel::Poll(&no_cancel), &mut sink).await;

        let AgentDrainOutcome::Completed(turn) = outcome else {
            panic!("expected Completed, got {outcome:?}");
        };
        assert_eq!(turn.text, "hello world");
        assert_eq!(*deltas.lock().unwrap(), "hello world");
        assert_eq!(turn.tool_uses.len(), 1);
        assert_eq!(turn.tool_uses[0].name, "Bash");
        assert_eq!(last_tool.lock().unwrap().as_deref(), Some("Bash"));
        assert_eq!(
            last_tool_input.lock().unwrap().as_deref(),
            Some(r#"{"command":"ls"}"#)
        );
        assert_eq!(turn.stop_reason, Some(StopReason::ToolUse));
        assert!(turn.saw_usage);
        assert_eq!(turn.input_tokens, 100);
        // Cumulative 10 then 15 folds into a per-turn total of 15.
        assert_eq!(turn.output_tokens, 15);
    }

    // A provider that emits only a terminal TextDone (no deltas) still yields
    // the text — the fallback both loops now share.
    #[tokio::test(flavor = "current_thread")]
    async fn drain_textdone_fallback_robust() {
        let events = vec![
            StreamEvent::TextDone {
                index: 0,
                text: "only terminal".into(),
            },
            StreamEvent::Done {
                stop_reason: StopReason::EndTurn,
            },
        ];
        let deltas = std::sync::Arc::new(std::sync::Mutex::new(String::new()));
        let d2 = deltas.clone();
        let mut sink = move |ev: AgentDrainEvent| -> futures::future::BoxFuture<'static, ()> {
            if let AgentDrainEvent::TextDelta(d) = ev {
                d2.lock().unwrap().push_str(&d);
            }
            Box::pin(async {})
        };
        let outcome =
            drain_agent_stream(scripted(events), DrainCancel::Poll(&no_cancel), &mut sink).await;
        let AgentDrainOutcome::Completed(turn) = outcome else {
            panic!("expected Completed, got {outcome:?}");
        };
        assert_eq!(turn.text, "only terminal");
        assert_eq!(*deltas.lock().unwrap(), "only terminal");
    }

    // A retryable in-stream error surfaces as Retryable; a plain one as Fatal.
    #[tokio::test(flavor = "current_thread")]
    async fn drain_classifies_errors_robust() {
        let retry = drain_agent_stream(
            scripted(vec![StreamEvent::Error {
                message: "overloaded_error: please retry".into(),
            }]),
            DrainCancel::Poll(&no_cancel),
            &mut noop_sink(),
        )
        .await;
        assert!(
            matches!(retry, AgentDrainOutcome::Retryable(_)),
            "got {retry:?}"
        );

        let fatal = drain_agent_stream(
            scripted(vec![StreamEvent::Error {
                message: "invalid_request: bad tool schema".into(),
            }]),
            DrainCancel::Poll(&no_cancel),
            &mut noop_sink(),
        )
        .await;
        assert!(
            matches!(fatal, AgentDrainOutcome::Fatal(_)),
            "got {fatal:?}"
        );
    }

    // The watch-channel cancel aborts even while events keep flowing.
    #[tokio::test(flavor = "current_thread")]
    async fn drain_watch_cancel_robust() {
        let (tx, mut rx) = tokio::sync::watch::channel(true); // already aborted
        let _ = &tx;
        let outcome = drain_agent_stream(
            scripted(vec![StreamEvent::TextDelta {
                index: 0,
                delta: "never seen".into(),
            }]),
            DrainCancel::Watch(&mut rx),
            &mut noop_sink(),
        )
        .await;
        assert!(matches!(outcome, AgentDrainOutcome::Cancelled));
    }

    // Without a usage event, the len/4 text estimate accumulates.
    #[tokio::test(flavor = "current_thread")]
    async fn drain_estimates_tokens_without_usage_normal() {
        let events = vec![
            StreamEvent::TextDelta {
                index: 0,
                delta: "x".repeat(40),
            },
            StreamEvent::Done {
                stop_reason: StopReason::EndTurn,
            },
        ];
        let mut sink = noop_sink();
        let outcome =
            drain_agent_stream(scripted(events), DrainCancel::Poll(&no_cancel), &mut sink).await;
        let AgentDrainOutcome::Completed(turn) = outcome else {
            panic!("expected Completed, got {outcome:?}");
        };
        assert!(!turn.saw_usage);
        assert_eq!(turn.estimated_tokens, 10);
    }
}
