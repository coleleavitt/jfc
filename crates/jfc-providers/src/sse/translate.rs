use jfc_provider::{StopReason, StreamEvent};

use super::{SseEvent, apply_content_delta, start_content_block, stop_content_block};

pub fn parse_stop_reason(s: Option<&str>) -> StopReason {
    let result = match s {
        Some("end_turn") => StopReason::EndTurn,
        Some("tool_use") => StopReason::ToolUse,
        // Server-side sampling loop hit its iteration cap. The runtime
        // must re-send the conversation without injecting a synthetic
        // user message; the server resumes the loop where it left off.
        // See StopReason::PauseTurn docs and cli.js v142:622686.
        Some("pause_turn") => StopReason::PauseTurn,
        Some("refusal") => StopReason::Refusal,
        Some("max_tokens") => StopReason::MaxTokens,
        Some("stop_sequence") => StopReason::StopSequence,
        Some(other) => {
            // Unknown stop_reason string. Surface loudly — every
            // historical "stream silently ends" bug has eventually
            // traced back to a new server stop_reason being bucketed
            // into Other(...) and falling through event_loop's
            // dispatch ladder. The warn gives us a one-grep way to
            // catch the next variant (e.g. "container_*")
            // before users notice.
            tracing::warn!(
                target: "jfc::provider::sse",
                stop_reason = other,
                "parse_stop_reason: unknown stop_reason string — bucketing into Other(...) \
                 (event_loop will fall into the 'model said its piece' branch); \
                 check cli.js v142 for a new variant we need to map"
            );
            StopReason::Other(other.to_owned())
        }
        None => {
            // Missing stop_reason field. Anthropic sometimes omits it
            // on truncated streams or context_hint short-circuits. The
            // EndTurn default is most-conservative for back-compat
            // (closes the streaming slot cleanly) but the silent fall-
            // through is exactly the class of bug that hid pause_turn
            // for months. Warn loudly so future occurrences are
            // diagnosable from the trace log alone.
            tracing::warn!(
                target: "jfc::provider::sse",
                "parse_stop_reason: missing stop_reason field — defaulting to EndTurn \
                 (this is back-compat; if you see this paired with a stalled stream, \
                 the upstream omitted a real stop_reason we should be handling)"
            );
            StopReason::EndTurn
        }
    };
    tracing::trace!(
        target: "jfc::provider::sse",
        input = ?s,
        result = ?result,
        "parse_stop_reason"
    );
    result
}

pub fn translate(
    event: SseEvent,
    blocks: &mut Vec<Option<super::BlockState>>,
    stop_reason: &mut Option<StopReason>,
) -> Option<StreamEvent> {
    match event {
        SseEvent::ContentBlockStart {
            index,
            content_block,
        } => start_content_block(index, content_block, blocks),
        SseEvent::ContentBlockDelta { index, delta } => apply_content_delta(index, delta, blocks),
        SseEvent::ContentBlockStop { index } => stop_content_block(index, blocks),
        SseEvent::MessageDelta {
            delta,
            usage,
            context_management,
        } => {
            // Log server-side context management metadata when present.
            if let Some(ref cm) = context_management {
                tracing::debug!(
                    target: "jfc::stream",
                    context_management = ?cm,
                    "server-side context management active"
                );
                if cm.compacted {
                    tracing::info!(
                        target: "jfc::stream",
                        removed_tokens = ?cm.removed_tokens,
                        "server compacted context (context_management.compacted=true)"
                    );
                }
            }
            if let Some(reason) = delta.stop_reason.as_deref() {
                *stop_reason = Some(parse_stop_reason(Some(reason)));
            }
            usage.map(|usage| StreamEvent::Usage {
                input_tokens: usage.input_tokens(),
                output_tokens: usage.output_total(),
                thinking_tokens: usage.thinking_tokens(),
                cache_read_tokens: usage.cache_read_input_tokens.unwrap_or_default(),
                cache_write_tokens: usage.cache_creation_input_tokens.unwrap_or_default(),
            })
        }
        SseEvent::MessageStop => {
            // Same silent-default trap as parse_stop_reason(None):
            // message_stop without a preceding message_delta means the
            // upstream forgot to tell us why the turn ended. Default to
            // EndTurn for back-compat but log so a stalled stream is
            // diagnosable. Mirrors the warn in parse_stop_reason.
            let reason = match stop_reason.take() {
                Some(r) => r,
                None => {
                    tracing::warn!(
                        target: "jfc::provider::sse",
                        "message_stop arrived without a preceding message_delta \
                         (no stop_reason was set) — defaulting to EndTurn; if the \
                         turn looks truncated, check the raw SSE log for the missing \
                         delta event"
                    );
                    StopReason::EndTurn
                }
            };
            Some(StreamEvent::Done {
                stop_reason: reason,
            })
        }
        SseEvent::Error { error } => {
            let message = match error.kind.as_deref() {
                Some("overloaded_error" | "rate_limit_error" | "api_error") => {
                    format!("{}{}", crate::anthropic::AUTO_RETRY_SENTINEL, error.message)
                }
                _ => error.message,
            };
            Some(StreamEvent::Error { message })
        }
        SseEvent::MessageStart { message } => Some(StreamEvent::ResponseMetadata {
            response_id: message.id,
            input_tokens: message
                .usage
                .as_ref()
                .and_then(|u| u.input_tokens)
                .map(|t| t as u64),
        }),
        SseEvent::Ping => None,
        SseEvent::Unknown { kind, raw } => {
            tracing::warn!(
                target: "jfc::provider::anthropic_sse",
                kind = %kind,
                raw_preview = %raw.to_string().chars().take(200).collect::<String>(),
                "unknown SSE event type ignored"
            );
            None
        }
    }
}
