use std::{collections::VecDeque, pin::Pin, time::Duration};

use anyhow::{Result, anyhow};
use futures::{Stream, StreamExt};

use super::{SseFrame, SseParser, usize_to_u64_saturating};
use crate::sse::trace::{
    trace_chunk_received, trace_emitted_frame, trace_stream_error, trace_stream_timeout,
    trace_stream_wait,
};

const DEFAULT_BYTE_STREAM_IDLE_TIMEOUT: Duration = Duration::from_secs(600);
pub(super) const MIN_BYTE_STREAM_IDLE_TIMEOUT: Duration = Duration::from_secs(10);
pub(super) const MAX_BYTE_STREAM_IDLE_TIMEOUT: Duration = Duration::from_secs(30 * 60);

pub(super) struct ByteStreamState<S> {
    pub(super) stream: Pin<Box<S>>,
    pub(super) parser: SseParser,
    pub(super) pending: VecDeque<Result<SseFrame>>,
    pub(super) finished: bool,
    pub(super) chunk_index: u64,
    pub(super) frame_index: u64,
}

pub fn response_event_stream(
    resp: reqwest::Response,
) -> Pin<Box<dyn Stream<Item = Result<SseFrame>> + Send>> {
    byte_stream_events(resp.bytes_stream())
}

pub fn byte_stream_events<S, B, E>(
    stream: S,
) -> Pin<Box<dyn Stream<Item = Result<SseFrame>> + Send>>
where
    S: Stream<Item = std::result::Result<B, E>> + Send + 'static,
    B: AsRef<[u8]> + Send + 'static,
    E: std::error::Error + Send + Sync + 'static,
{
    let state = ByteStreamState {
        stream: Box::pin(stream),
        parser: SseParser::new(),
        pending: VecDeque::new(),
        finished: false,
        chunk_index: 0,
        frame_index: 0,
    };

    Box::pin(futures::stream::unfold(state, |mut state| async move {
        loop {
            let _linkscope_poll = linkscope::phase("sdk.sse.poll");
            if let Some(next) = state.pending.pop_front() {
                linkscope::record_items("sdk.sse.frame.emit", 1);
                trace_emitted_frame(state.frame_index, &next);
                state.frame_index = state.frame_index.saturating_add(1);
                return Some((next, state));
            }

            if state.finished {
                linkscope::record_items("sdk.sse.finished", 1);
                return None;
            }

            let idle_timeout = byte_stream_idle_timeout();
            trace_stream_wait(&state, idle_timeout);
            let next_chunk = tokio::time::timeout(idle_timeout, state.stream.next()).await;
            match next_chunk {
                Err(_) => {
                    linkscope::record_items("sdk.sse.idle_timeout", 1);
                    trace_stream_timeout(&state, idle_timeout);
                    state.finished = true;
                    return Some((
                        Err(anyhow!(
                            "SSE byte stream was idle for {}ms",
                            idle_timeout.as_millis()
                        )),
                        state,
                    ));
                }
                Ok(Some(Ok(chunk))) => match state.parser.push(chunk.as_ref()) {
                    Ok(frames) => {
                        let bytes = chunk.as_ref().len();
                        trace_chunk_received(&state, chunk.as_ref(), frames.len());
                        linkscope::record_items("sdk.sse.chunk", 1);
                        linkscope::record_bytes("sdk.sse.chunk", usize_to_u64_saturating(bytes));
                        linkscope::record_items(
                            "sdk.sse.frame.parsed",
                            usize_to_u64_saturating(frames.len()),
                        );
                        state.pending.extend(frames.into_iter().map(Ok));
                        state.chunk_index = state.chunk_index.saturating_add(1);
                    }
                    Err(e) => {
                        linkscope::record_items("sdk.sse.parse_error", 1);
                        trace_stream_error("sdk.sse.parse_error.detail", &state);
                        state.finished = true;
                        return Some((Err(e), state));
                    }
                },
                Ok(Some(Err(e))) => {
                    linkscope::record_items("sdk.sse.upstream_error", 1);
                    trace_stream_error("sdk.sse.upstream_error.detail", &state);
                    state.finished = true;
                    return Some((Err(anyhow!(e)), state));
                }
                Ok(None) => {
                    linkscope::record_items("sdk.sse.eof", 1);
                    trace_stream_error("sdk.sse.eof.detail", &state);
                    state.finished = true;
                    match state.parser.finish() {
                        Ok(frames) => {
                            linkscope::record_items(
                                "sdk.sse.frame.finish",
                                usize_to_u64_saturating(frames.len()),
                            );
                            state.pending.extend(frames.into_iter().map(Ok));
                        }
                        Err(e) => {
                            linkscope::record_items("sdk.sse.finish_error", 1);
                            trace_stream_error("sdk.sse.finish_error.detail", &state);
                            return Some((Err(e), state));
                        }
                    }
                }
            }
        }
    }))
}

fn byte_stream_idle_timeout() -> Duration {
    let configured = [
        "JFC_BYTE_STREAM_IDLE_TIMEOUT_MS",
        "CLAUDE_BYTE_STREAM_IDLE_TIMEOUT_MS",
        "JFC_STREAM_IDLE_TIMEOUT_MS",
        "CLAUDE_STREAM_IDLE_TIMEOUT_MS",
    ]
    .iter()
    .filter_map(|key| std::env::var(key).ok())
    .find_map(|value| parse_timeout_ms(Some(value.as_str())));
    clamp_byte_stream_timeout(configured.unwrap_or(DEFAULT_BYTE_STREAM_IDLE_TIMEOUT))
}

pub(super) fn parse_timeout_ms(value: Option<&str>) -> Option<Duration> {
    let millis = value?.trim().parse::<u64>().ok()?;
    (millis > 0).then(|| Duration::from_millis(millis))
}

pub(super) fn clamp_byte_stream_timeout(timeout: Duration) -> Duration {
    timeout.clamp(MIN_BYTE_STREAM_IDLE_TIMEOUT, MAX_BYTE_STREAM_IDLE_TIMEOUT)
}
