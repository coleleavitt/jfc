use std::error::Error;
use std::io::{self, BufRead, Write};

use jfc_plugin_sdk::{
    BridgeEnvelope, BridgeErrorDto, BridgeRequest, BridgeResponse, BridgeUiPanelRefreshResult,
    BridgeUiWidgetRefreshResult,
};

fn main() -> Result<(), Box<dyn Error>> {
    let stdin = io::stdin();
    let mut lines = stdin.lock().lines();
    let frame = read_frame(&mut lines)?;
    let id = frame.id().to_owned();
    let response = match frame {
        BridgeEnvelope::Request {
            request: BridgeRequest::UiWidgetRefresh { refresh },
            ..
        } => {
            let count = refresh
                .state
                .as_ref()
                .and_then(|state| state.get("count"))
                .and_then(serde_json::Value::as_u64)
                .unwrap_or_default()
                .saturating_add(1);
            BridgeResponse::UiWidgetRefresh {
                result: BridgeUiWidgetRefreshResult::body(format!("refresh #{count}"))
                    .with_state(serde_json::json!({ "count": count })),
            }
        }
        BridgeEnvelope::Request {
            request: BridgeRequest::UiPanelRefresh { refresh },
            ..
        } => {
            let count = refresh
                .state
                .as_ref()
                .and_then(|state| state.get("count"))
                .and_then(serde_json::Value::as_u64)
                .unwrap_or_default()
                .saturating_add(1);
            BridgeResponse::UiPanelRefresh {
                result: BridgeUiPanelRefreshResult::body(format!(
                    "Panel refresh #{count}\nProcess bridge is healthy."
                ))
                .with_state(serde_json::json!({ "count": count })),
            }
        }
        _ => BridgeResponse::Error(BridgeErrorDto::new(
            "unsupported_request",
            "expected ui_widget_refresh or ui_panel_refresh request",
        )),
    };
    write_frame(&BridgeEnvelope::response(id, response))?;
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
