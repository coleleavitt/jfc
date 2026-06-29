use std::error::Error;
use std::io::{self, BufRead, Write};

use jfc_plugin_sdk::{
    BridgeEnvelope, BridgeErrorDto, BridgePromptContextRefreshResult, BridgeRequest, BridgeResponse,
};

fn main() -> Result<(), Box<dyn Error>> {
    let stdin = io::stdin();
    let mut lines = stdin.lock().lines();
    let frame = read_frame(&mut lines)?;
    let id = frame.id().to_owned();
    let response = match frame {
        BridgeEnvelope::Request {
            request: BridgeRequest::PromptContextRefresh { refresh },
            ..
        } => {
            let count = refresh_count(refresh.state.as_ref());
            let cwd = refresh.cwd.as_deref().unwrap_or(".");
            BridgeResponse::PromptContextRefresh {
                result: BridgePromptContextRefreshResult::body(format!(
                    "Cached prompt context refresh #{count}\nProject root: {cwd}"
                ))
                .with_state(serde_json::json!({ "count": count })),
            }
        }
        _ => BridgeResponse::Error(BridgeErrorDto::new(
            "unsupported_request",
            "expected prompt_context_refresh request",
        )),
    };
    write_frame(&BridgeEnvelope::response(id, response))?;
    Ok(())
}

fn refresh_count(state: Option<&serde_json::Value>) -> u64 {
    state
        .and_then(|state| state.get("count"))
        .and_then(serde_json::Value::as_u64)
        .unwrap_or_default()
        .saturating_add(1)
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
