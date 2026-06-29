//! Small bounded Server-Sent Events parser used by streaming providers.
//!
//! This intentionally parses bytes directly instead of first converting each
//! transport chunk into `String`: UTF-8 code points, CRLF pairs, and SSE lines
//! can all be split across network chunks.

use std::str;

use anyhow::{Context, Result, bail};

mod stream;
#[cfg(test)]
mod tests;
mod trace;

pub use stream::{byte_stream_events, response_event_stream};
use trace::{ptr_to_u64, sse_field_label, trace_parser_input, trace_parser_state, trace_sse_line};

const PRODUCTION_MAX_LINE_BYTES: usize = 8 * 1024 * 1024;
const PRODUCTION_MAX_EVENT_DATA_BYTES: usize = 64 * 1024 * 1024;
const TEST_MAX_LINE_BYTES: usize = 64;
const TEST_MAX_EVENT_DATA_BYTES: usize = 128;
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
        linkscope::record_items("sdk.sse.parser.new", 1);
        Self::default()
    }

    pub fn push(&mut self, bytes: &[u8]) -> Result<Vec<SseFrame>> {
        let _linkscope_push = linkscope::phase("sdk.sse.parser.push");
        trace_parser_input("sdk.sse.parser.push.input", self, bytes);
        if bytes.is_empty() {
            return Ok(Vec::new());
        }
        self.buffer.extend_from_slice(bytes);
        let frames = self.parse_available(false)?;
        trace_parser_state(
            "sdk.sse.parser.push.output",
            self,
            Some(usize_to_u64_saturating(frames.len())),
        );
        Ok(frames)
    }

    pub fn finish(&mut self) -> Result<Vec<SseFrame>> {
        let _linkscope_finish = linkscope::phase("sdk.sse.parser.finish");
        trace_parser_state("sdk.sse.parser.finish.input", self, None);
        let mut frames = self.parse_available(true)?;
        if let Some(frame) = self.dispatch() {
            frames.push(frame);
        }
        trace_parser_state(
            "sdk.sse.parser.finish.output",
            self,
            Some(usize_to_u64_saturating(frames.len())),
        );
        Ok(frames)
    }

    fn parse_available(&mut self, eof: bool) -> Result<Vec<SseFrame>> {
        let _linkscope_parse = linkscope::phase("sdk.sse.parser.parse_available");
        if linkscope::trace_detail_enabled() {
            linkscope::detail_event_fields(
                "sdk.sse.parser.parse.start",
                [
                    linkscope::TraceField::count("eof", u64::from(eof)),
                    linkscope::TraceField::bytes(
                        "buffer_bytes",
                        usize_to_u64_saturating(self.buffer.len()),
                    ),
                    linkscope::TraceField::addr("buffer_addr", ptr_to_u64(self.buffer.as_ptr())),
                ],
            );
        }
        let mut frames = Vec::new();
        let mut consumed = 0usize;
        let mut lines = 0u64;

        while let Some((line_len, terminator_len)) = next_line_bounds(&self.buffer[consumed..], eof)
        {
            let line_start = consumed;
            let line_end = consumed + line_len;
            let mut line = self.buffer[line_start..line_end].to_vec();
            consumed = line_end + terminator_len;
            lines = lines.saturating_add(1);
            linkscope::record_bytes("sdk.sse.parser.line", usize_to_u64_saturating(line_len));
            linkscope::record_items("sdk.sse.parser.line", 1);

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
            linkscope::record_items("sdk.sse.parser.line_too_long", 1);
            bail!(
                "SSE line exceeded {} bytes without a line terminator",
                max_line_bytes()
            );
        }

        if linkscope::trace_detail_enabled() {
            linkscope::detail_event_fields(
                "sdk.sse.parser.parse.done",
                [
                    linkscope::TraceField::count("eof", u64::from(eof)),
                    linkscope::TraceField::count("lines", lines),
                    linkscope::TraceField::bytes("consumed", usize_to_u64_saturating(consumed)),
                    linkscope::TraceField::bytes(
                        "buffer_remaining",
                        usize_to_u64_saturating(self.buffer.len()),
                    ),
                    linkscope::TraceField::count("frames", usize_to_u64_saturating(frames.len())),
                ],
            );
        }
        Ok(frames)
    }

    fn process_line(&mut self, line: &[u8]) -> Result<Option<SseFrame>> {
        let _linkscope_line = linkscope::phase("sdk.sse.parser.process_line");
        if line.is_empty() {
            trace_sse_line("blank", line.len(), 0);
            return Ok(self.dispatch());
        }
        if line[0] == b':' {
            trace_sse_line("comment", line.len(), line.len().saturating_sub(1));
            return Ok(None);
        }

        let (field, mut value) = match line.iter().position(|b| *b == b':') {
            Some(idx) => (&line[..idx], &line[idx + 1..]),
            None => (line, &[][..]),
        };
        if value.first() == Some(&b' ') {
            value = &value[1..];
        }
        trace_sse_line(sse_field_label(field), line.len(), value.len());

        match field {
            b"data" => {
                let value = str::from_utf8(value).context("SSE data field was not valid UTF-8")?;
                let projected = self
                    .data
                    .len()
                    .saturating_add(value.len())
                    .saturating_add(1);
                if projected > max_event_data_bytes() {
                    linkscope::record_items("sdk.sse.parser.data_too_large", 1);
                    bail!(
                        "SSE event data exceeded {} bytes before dispatch",
                        max_event_data_bytes()
                    );
                }
                linkscope::record_bytes(
                    "sdk.sse.parser.data",
                    usize_to_u64_saturating(value.len()),
                );
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
                    linkscope::record_items("sdk.sse.parser.event_field", 1);
                }
            }
            b"id" | b"retry" => {}
            _ => {}
        }

        Ok(None)
    }

    fn dispatch(&mut self) -> Option<SseFrame> {
        let _linkscope_dispatch = linkscope::phase("sdk.sse.parser.dispatch");
        let event = self.event.take().unwrap_or_else(|| "message".to_owned());
        if self.data.is_empty() {
            if linkscope::trace_detail_enabled() {
                linkscope::detail_event_fields(
                    "sdk.sse.parser.dispatch.empty",
                    [linkscope::TraceField::text("event", event)],
                );
            }
            return None;
        }

        let data_bytes = self.data.len();
        if self.data.ends_with('\n') {
            self.data.pop();
        }
        let frame = SseFrame {
            event,
            data: std::mem::take(&mut self.data),
        };
        if linkscope::trace_detail_enabled() {
            linkscope::detail_event_fields(
                "sdk.sse.parser.dispatch.frame",
                [
                    linkscope::TraceField::text("event", frame.event.clone()),
                    linkscope::TraceField::bytes("data_bytes", usize_to_u64_saturating(data_bytes)),
                    linkscope::TraceField::addr("data_addr", ptr_to_u64(frame.data.as_ptr())),
                ],
            );
        }
        Some(frame)
    }
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

fn usize_to_u64_saturating(value: usize) -> u64 {
    u64::try_from(value).unwrap_or(u64::MAX)
}
