//! Small bounded Server-Sent Events parser used by streaming providers.
//!
//! This intentionally parses bytes directly instead of first converting each
//! transport chunk into `String`: UTF-8 code points, CRLF pairs, and SSE lines
//! can all be split across network chunks.

use std::{collections::VecDeque, pin::Pin, str, time::Duration};

use anyhow::{Context, Result, anyhow, bail};
use futures::{Stream, StreamExt};

const PRODUCTION_MAX_LINE_BYTES: usize = 8 * 1024 * 1024;
const PRODUCTION_MAX_EVENT_DATA_BYTES: usize = 64 * 1024 * 1024;
const TEST_MAX_LINE_BYTES: usize = 64;
const TEST_MAX_EVENT_DATA_BYTES: usize = 128;
const DEFAULT_BYTE_STREAM_IDLE_TIMEOUT: Duration = Duration::from_secs(600);
const MIN_BYTE_STREAM_IDLE_TIMEOUT: Duration = Duration::from_secs(10);
const MAX_BYTE_STREAM_IDLE_TIMEOUT: Duration = Duration::from_secs(30 * 60);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SseFrame {
    pub event: String,
    pub data: String,
}

#[derive(Debug, Default)]
pub struct SseParser {
    buffer: Vec<u8>,
    event: Option<String>,
    data: String,
    saw_first_line: bool,
}

impl SseParser {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&mut self, bytes: &[u8]) -> Result<Vec<SseFrame>> {
        if bytes.is_empty() {
            return Ok(Vec::new());
        }
        self.buffer.extend_from_slice(bytes);
        self.parse_available(false)
    }

    pub fn finish(&mut self) -> Result<Vec<SseFrame>> {
        let mut frames = self.parse_available(true)?;
        if let Some(frame) = self.dispatch() {
            frames.push(frame);
        }
        Ok(frames)
    }

    fn parse_available(&mut self, eof: bool) -> Result<Vec<SseFrame>> {
        let mut frames = Vec::new();
        let mut consumed = 0usize;

        while let Some((line_len, terminator_len)) = next_line_bounds(&self.buffer[consumed..], eof)
        {
            let line_start = consumed;
            let line_end = consumed + line_len;
            let mut line = self.buffer[line_start..line_end].to_vec();
            consumed = line_end + terminator_len;

            if !self.saw_first_line {
                self.saw_first_line = true;
                if line.starts_with(&[0xEF, 0xBB, 0xBF]) {
                    line.drain(..3);
                }
            }

            if let Some(frame) = self.process_line(&line)? {
                frames.push(frame);
            }
        }

        if consumed != 0 {
            self.buffer.drain(..consumed);
        }
        if self.buffer.len() > max_line_bytes() {
            bail!(
                "SSE line exceeded {} bytes without a line terminator",
                max_line_bytes()
            );
        }

        Ok(frames)
    }

    fn process_line(&mut self, line: &[u8]) -> Result<Option<SseFrame>> {
        if line.is_empty() {
            return Ok(self.dispatch());
        }
        if line[0] == b':' {
            return Ok(None);
        }

        let (field, mut value) = match line.iter().position(|b| *b == b':') {
            Some(idx) => (&line[..idx], &line[idx + 1..]),
            None => (line, &[][..]),
        };
        if value.first() == Some(&b' ') {
            value = &value[1..];
        }

        match field {
            b"data" => {
                let value = str::from_utf8(value).context("SSE data field was not valid UTF-8")?;
                let projected = self
                    .data
                    .len()
                    .saturating_add(value.len())
                    .saturating_add(1);
                if projected > max_event_data_bytes() {
                    bail!(
                        "SSE event data exceeded {} bytes before dispatch",
                        max_event_data_bytes()
                    );
                }
                self.data.push_str(value);
                self.data.push('\n');
            }
            b"event" => {
                if !value.contains(&0) {
                    self.event = Some(
                        str::from_utf8(value)
                            .context("SSE event field was not valid UTF-8")?
                            .to_owned(),
                    );
                }
            }
            b"id" | b"retry" => {}
            _ => {}
        }

        Ok(None)
    }

    fn dispatch(&mut self) -> Option<SseFrame> {
        let event = self.event.take().unwrap_or_else(|| "message".to_owned());
        if self.data.is_empty() {
            return None;
        }

        if self.data.ends_with('\n') {
            self.data.pop();
        }
        Some(SseFrame {
            event,
            data: std::mem::take(&mut self.data),
        })
    }
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
    struct State<S> {
        stream: Pin<Box<S>>,
        parser: SseParser,
        pending: VecDeque<Result<SseFrame>>,
        finished: bool,
    }

    let state = State {
        stream: Box::pin(stream),
        parser: SseParser::new(),
        pending: VecDeque::new(),
        finished: false,
    };

    Box::pin(futures::stream::unfold(state, |mut state| async move {
        loop {
            if let Some(next) = state.pending.pop_front() {
                return Some((next, state));
            }

            if state.finished {
                return None;
            }

            let idle_timeout = byte_stream_idle_timeout();
            let next_chunk = tokio::time::timeout(idle_timeout, state.stream.next()).await;
            match next_chunk {
                Err(_) => {
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
                        state.pending.extend(frames.into_iter().map(Ok));
                    }
                    Err(e) => {
                        state.finished = true;
                        return Some((Err(e), state));
                    }
                },
                Ok(Some(Err(e))) => {
                    state.finished = true;
                    return Some((Err(anyhow!(e)), state));
                }
                Ok(None) => {
                    state.finished = true;
                    match state.parser.finish() {
                        Ok(frames) => {
                            state.pending.extend(frames.into_iter().map(Ok));
                        }
                        Err(e) => return Some((Err(e), state)),
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

fn parse_timeout_ms(value: Option<&str>) -> Option<Duration> {
    let millis = value?.trim().parse::<u64>().ok()?;
    (millis > 0).then(|| Duration::from_millis(millis))
}

fn clamp_byte_stream_timeout(timeout: Duration) -> Duration {
    timeout.clamp(MIN_BYTE_STREAM_IDLE_TIMEOUT, MAX_BYTE_STREAM_IDLE_TIMEOUT)
}

fn next_line_bounds(buffer: &[u8], eof: bool) -> Option<(usize, usize)> {
    if buffer.is_empty() {
        return None;
    }

    for (idx, byte) in buffer.iter().enumerate() {
        match *byte {
            b'\n' => return Some((idx, 1)),
            b'\r' => {
                if buffer.get(idx + 1) == Some(&b'\n') {
                    return Some((idx, 2));
                }
                if idx + 1 == buffer.len() && !eof {
                    return None;
                }
                return Some((idx, 1));
            }
            _ => {}
        }
    }

    eof.then_some((buffer.len(), 0))
}

fn max_line_bytes() -> usize {
    if cfg!(test) {
        TEST_MAX_LINE_BYTES
    } else {
        PRODUCTION_MAX_LINE_BYTES
    }
}

fn max_event_data_bytes() -> usize {
    if cfg!(test) {
        TEST_MAX_EVENT_DATA_BYTES
    } else {
        PRODUCTION_MAX_EVENT_DATA_BYTES
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::StreamExt;

    #[test]
    fn parser_handles_split_utf8_and_multiline_data_normal() {
        let mut parser = SseParser::new();
        assert!(
            parser
                .push("event: update\ndata: caf".as_bytes())
                .unwrap()
                .is_empty()
        );
        let frames = parser.push("é\ndata: ok\n\n".as_bytes()).unwrap();
        assert_eq!(
            frames,
            vec![SseFrame {
                event: "update".to_owned(),
                data: "café\nok".to_owned(),
            }]
        );
    }

    #[test]
    fn parser_handles_crlf_split_across_chunks_robust() {
        let mut parser = SseParser::new();
        assert!(parser.push(b"data: hello\r").unwrap().is_empty());
        let frames = parser.push(b"\n\r\n").unwrap();
        assert_eq!(
            frames,
            vec![SseFrame {
                event: "message".to_owned(),
                data: "hello".to_owned(),
            }]
        );
    }

    #[test]
    fn parser_ignores_comments_and_strips_bom_normal() {
        let mut parser = SseParser::new();
        let frames = parser
            .push(b"\xEF\xBB\xBF: comment\nevent: ping\ndata: {}\n\n")
            .unwrap();
        assert_eq!(
            frames,
            vec![SseFrame {
                event: "ping".to_owned(),
                data: "{}".to_owned(),
            }]
        );
    }

    #[test]
    fn parser_emits_final_unterminated_event_robust() {
        let mut parser = SseParser::new();
        assert!(parser.push(b"data: [DONE]").unwrap().is_empty());
        let frames = parser.finish().unwrap();
        assert_eq!(
            frames,
            vec![SseFrame {
                event: "message".to_owned(),
                data: "[DONE]".to_owned(),
            }]
        );
    }

    #[test]
    fn parser_rejects_invalid_utf8_robust() {
        let mut parser = SseParser::new();
        let err = parser.push(b"data: \xFF\n\n").unwrap_err();
        assert!(err.to_string().contains("valid UTF-8"));
    }

    #[test]
    fn parser_rejects_unbounded_line_robust() {
        let mut parser = SseParser::new();
        let line = vec![b'a'; TEST_MAX_LINE_BYTES + 1];
        let err = parser.push(&line).unwrap_err();
        assert!(err.to_string().contains("exceeded"));
    }

    #[test]
    fn byte_stream_timeout_parser_and_clamp_normal() {
        assert_eq!(parse_timeout_ms(None), None);
        assert_eq!(parse_timeout_ms(Some("0")), None);
        assert_eq!(
            parse_timeout_ms(Some("15000")),
            Some(Duration::from_secs(15))
        );
        assert_eq!(
            clamp_byte_stream_timeout(Duration::from_millis(1)),
            MIN_BYTE_STREAM_IDLE_TIMEOUT
        );
        assert_eq!(
            clamp_byte_stream_timeout(Duration::from_secs(3600)),
            MAX_BYTE_STREAM_IDLE_TIMEOUT
        );
    }

    #[tokio::test]
    async fn byte_stream_events_preserves_frame_order_normal() {
        let chunks = futures::stream::iter([
            Ok::<_, std::io::Error>(b"data: one\n\n".to_vec()),
            Ok::<_, std::io::Error>(b"event: two\ndata: 2\n\n".to_vec()),
        ]);
        let frames = byte_stream_events(chunks)
            .collect::<Vec<_>>()
            .await
            .into_iter()
            .collect::<Result<Vec<_>>>()
            .unwrap();
        assert_eq!(frames[0].data, "one");
        assert_eq!(frames[1].event, "two");
    }
}
