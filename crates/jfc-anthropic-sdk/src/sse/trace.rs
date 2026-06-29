use std::time::Duration;

use anyhow::Result;

use super::stream::ByteStreamState;
use super::{SseFrame, SseParser, usize_to_u64_saturating};

pub(super) fn trace_parser_input(label: &'static str, parser: &SseParser, bytes: &[u8]) {
    if !linkscope::trace_detail_enabled() {
        return;
    }
    linkscope::detail_event_fields(
        label,
        [
            linkscope::TraceField::addr("parser_addr", ptr_to_u64(parser)),
            linkscope::TraceField::addr("input_addr", ptr_to_u64(bytes.as_ptr())),
            linkscope::TraceField::bytes("input_bytes", usize_to_u64_saturating(bytes.len())),
            linkscope::TraceField::bytes(
                "buffer_before",
                usize_to_u64_saturating(parser.buffer.len()),
            ),
            linkscope::TraceField::bytes("data_before", usize_to_u64_saturating(parser.data.len())),
            linkscope::TraceField::count("has_event", u64::from(parser.event.is_some())),
        ],
    );
}

pub(super) fn trace_parser_state(label: &'static str, parser: &SseParser, frames: Option<u64>) {
    if !linkscope::trace_detail_enabled() {
        return;
    }
    linkscope::detail_event_fields(
        label,
        [
            linkscope::TraceField::addr("parser_addr", ptr_to_u64(parser)),
            linkscope::TraceField::addr("buffer_addr", ptr_to_u64(parser.buffer.as_ptr())),
            linkscope::TraceField::addr("data_addr", ptr_to_u64(parser.data.as_ptr())),
            linkscope::TraceField::bytes(
                "buffer_bytes",
                usize_to_u64_saturating(parser.buffer.len()),
            ),
            linkscope::TraceField::bytes("data_bytes", usize_to_u64_saturating(parser.data.len())),
            linkscope::TraceField::count("has_event", u64::from(parser.event.is_some())),
            linkscope::TraceField::count("frames", frames.unwrap_or(0)),
        ],
    );
}

pub(super) fn trace_sse_line(field: &'static str, line_len: usize, value_len: usize) {
    if !linkscope::trace_detail_enabled() {
        return;
    }
    linkscope::detail_event_fields(
        "sdk.sse.parser.line.detail",
        [
            linkscope::TraceField::text("field", field),
            linkscope::TraceField::bytes("line_bytes", usize_to_u64_saturating(line_len)),
            linkscope::TraceField::bytes("value_bytes", usize_to_u64_saturating(value_len)),
        ],
    );
}

pub(super) fn sse_field_label(field: &[u8]) -> &'static str {
    match field {
        b"data" => "data",
        b"event" => "event",
        b"id" => "id",
        b"retry" => "retry",
        _ => "other",
    }
}

pub(super) fn trace_emitted_frame(frame_index: u64, result: &Result<SseFrame>) {
    if !linkscope::trace_detail_enabled() {
        return;
    }
    match result {
        Ok(frame) => linkscope::detail_event_fields(
            "sdk.sse.frame.emit.detail",
            [
                linkscope::TraceField::count("frame_index", frame_index),
                linkscope::TraceField::text("event", frame.event.clone()),
                linkscope::TraceField::bytes(
                    "data_bytes",
                    usize_to_u64_saturating(frame.data.len()),
                ),
                linkscope::TraceField::addr("data_addr", ptr_to_u64(frame.data.as_ptr())),
            ],
        ),
        Err(_) => linkscope::detail_event_fields(
            "sdk.sse.frame.emit.error",
            [linkscope::TraceField::count("frame_index", frame_index)],
        ),
    }
}

pub(super) fn trace_stream_wait<S>(state: &ByteStreamState<S>, timeout: Duration) {
    if !linkscope::trace_detail_enabled() {
        return;
    }
    linkscope::detail_event_fields(
        "sdk.sse.await_next_chunk",
        [
            linkscope::TraceField::addr("parser_addr", ptr_to_u64(&state.parser)),
            linkscope::TraceField::count("chunk_index", state.chunk_index),
            linkscope::TraceField::count("frame_index", state.frame_index),
            linkscope::TraceField::count("pending", usize_to_u64_saturating(state.pending.len())),
            linkscope::TraceField::count("idle_timeout_ms", duration_ms_u64(timeout)),
            linkscope::TraceField::bytes(
                "parser_buffer",
                usize_to_u64_saturating(state.parser.buffer.len()),
            ),
            linkscope::TraceField::bytes(
                "parser_data",
                usize_to_u64_saturating(state.parser.data.len()),
            ),
            linkscope::TraceField::count(
                "parser_has_event",
                u64::from(state.parser.event.is_some()),
            ),
        ],
    );
}

pub(super) fn trace_chunk_received<S>(state: &ByteStreamState<S>, chunk: &[u8], frames: usize) {
    if !linkscope::trace_detail_enabled() {
        return;
    }
    linkscope::detail_event_fields(
        "sdk.sse.chunk.received",
        [
            linkscope::TraceField::count("chunk_index", state.chunk_index),
            linkscope::TraceField::addr("chunk_addr", ptr_to_u64(chunk.as_ptr())),
            linkscope::TraceField::bytes("chunk_bytes", usize_to_u64_saturating(chunk.len())),
            linkscope::TraceField::count("frames", usize_to_u64_saturating(frames)),
            linkscope::TraceField::bytes(
                "parser_buffer",
                usize_to_u64_saturating(state.parser.buffer.len()),
            ),
            linkscope::TraceField::bytes(
                "parser_data",
                usize_to_u64_saturating(state.parser.data.len()),
            ),
            linkscope::TraceField::count("pending", usize_to_u64_saturating(state.pending.len())),
        ],
    );
}

pub(super) fn trace_stream_timeout<S>(state: &ByteStreamState<S>, timeout: Duration) {
    if !linkscope::trace_detail_enabled() {
        return;
    }
    linkscope::detail_event_fields(
        "sdk.sse.idle_timeout.detail",
        [
            linkscope::TraceField::count("chunk_index", state.chunk_index),
            linkscope::TraceField::count("frame_index", state.frame_index),
            linkscope::TraceField::count("idle_timeout_ms", duration_ms_u64(timeout)),
            linkscope::TraceField::bytes(
                "parser_buffer",
                usize_to_u64_saturating(state.parser.buffer.len()),
            ),
            linkscope::TraceField::bytes(
                "parser_data",
                usize_to_u64_saturating(state.parser.data.len()),
            ),
        ],
    );
}

pub(super) fn trace_stream_error<S>(label: &'static str, state: &ByteStreamState<S>) {
    if !linkscope::trace_detail_enabled() {
        return;
    }
    linkscope::detail_event_fields(
        label,
        [
            linkscope::TraceField::count("chunk_index", state.chunk_index),
            linkscope::TraceField::count("frame_index", state.frame_index),
            linkscope::TraceField::count("pending", usize_to_u64_saturating(state.pending.len())),
            linkscope::TraceField::bytes(
                "parser_buffer",
                usize_to_u64_saturating(state.parser.buffer.len()),
            ),
            linkscope::TraceField::bytes(
                "parser_data",
                usize_to_u64_saturating(state.parser.data.len()),
            ),
            linkscope::TraceField::count(
                "parser_has_event",
                u64::from(state.parser.event.is_some()),
            ),
        ],
    );
}

pub(super) fn ptr_to_u64<T>(value: *const T) -> u64 {
    u64::try_from(value.cast::<()>() as usize).unwrap_or(u64::MAX)
}

fn duration_ms_u64(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}
