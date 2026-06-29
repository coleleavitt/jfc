use std::error::Error;
use std::io::{self, BufRead, Write};

use jfc_plugin_sdk::{
    BridgeEnvelope, BridgeMailboxPollRequest, BridgeMailboxSendRequest, BridgeRequest,
    BridgeResponse, BridgeTeammateEvent, BridgeTeammateReady,
};

fn main() -> Result<(), Box<dyn Error>> {
    let stdin = io::stdin();
    let mut lines = stdin.lock().lines();
    let launch_id = read_frame(&mut lines)?.id().to_owned();

    write_frame(&BridgeEnvelope::request(
        "mailbox-1",
        BridgeRequest::TeammateMailboxPoll {
            request: BridgeMailboxPollRequest::unread().mark_read(true),
        },
    ))?;
    let mailbox_count = mailbox_message_count(read_frame(&mut lines)?);
    write_frame(&BridgeEnvelope::response(
        &launch_id,
        BridgeResponse::TeammateEvent {
            event: BridgeTeammateEvent::TextDelta {
                delta: format!("read {mailbox_count} mailbox message(s)"),
            },
        },
    ))?;

    write_frame(&BridgeEnvelope::request(
        "mailbox-2",
        BridgeRequest::TeammateMailboxSend {
            request: BridgeMailboxSendRequest::new("team-lead", "plugin helper is ready")
                .with_summary("ready"),
        },
    ))?;
    let _send_ack = read_frame(&mut lines)?;

    write_frame(&BridgeEnvelope::request(
        "ready-1",
        BridgeRequest::TeammateReady {
            ready: BridgeTeammateReady::new()
                .with_reason("waiting")
                .with_summary("ready for more work"),
        },
    ))?;
    let _ready_ack = read_frame(&mut lines)?;

    write_frame(&BridgeEnvelope::response(
        launch_id,
        BridgeResponse::TeammateEvent {
            event: BridgeTeammateEvent::Completed,
        },
    ))?;
    Ok(())
}

fn read_frame<I>(lines: &mut I) -> Result<BridgeEnvelope, Box<dyn Error>>
where
    I: Iterator<Item = io::Result<String>>,
{
    let Some(line) = lines.next() else {
        return Err("bridge stdin closed".into());
    };
    Ok(serde_json::from_str(&line?)?)
}

fn write_frame(frame: &BridgeEnvelope) -> Result<(), Box<dyn Error>> {
    let mut stdout = io::stdout().lock();
    serde_json::to_writer(&mut stdout, frame)?;
    stdout.write_all(b"\n")?;
    stdout.flush()?;
    Ok(())
}

fn mailbox_message_count(frame: BridgeEnvelope) -> usize {
    match frame {
        BridgeEnvelope::Response {
            response: BridgeResponse::TeammateMailboxMessages { messages },
            ..
        } => messages.len(),
        _ => 0,
    }
}
