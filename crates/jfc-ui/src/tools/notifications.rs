
use super::ExecutionResult;

pub(super) fn execute_push_notification(message: &str, title: Option<&str>) -> ExecutionResult {
    if message.is_empty() {
        return ExecutionResult::failure("push_notification: message is required");
    }
    let title = title.filter(|s| !s.is_empty()).unwrap_or("jfc");
    crate::notifications::notify(title, message);
    // TODO(remote-control): once the remote-control transport (websocket
    // bridge to the user's mobile companion) is wired in, also push the
    // notification through it. For now we only hit the local desktop
    // daemon and surface the gap in the success message so the user
    // knows we haven't silently dropped the remote leg.
    ExecutionResult::success(format!(
        "Desktop notification posted: {title} — {message} (remote-control push not yet implemented)"
    ))
}

// ─── RemoteTrigger tool ────────────────────────────────────────────────────
pub(super) async fn execute_remote_trigger(
    trigger_id: &str,
    payload: Option<&serde_json::Value>,
) -> ExecutionResult {
    if trigger_id.is_empty() {
        return ExecutionResult::failure("remote_trigger: trigger_id is required");
    }

    let triggers_path = match remote_trigger_config_path() {
        Some(p) => p,
        None => {
            return ExecutionResult::failure(
                "remote_trigger: cannot resolve ~/.config/jfc/triggers.toml (no HOME)",
            );
        }
    };
    let triggers_text = match tokio::fs::read_to_string(&triggers_path).await {
        Ok(s) => s,
        Err(e) => {
            return ExecutionResult::failure(format!(
                "remote_trigger: cannot read {} ({e}). \
                 Create the file with `[<trigger_id>] url = \"https://...\"` entries.",
                triggers_path.display()
            ));
        }
    };
    let url = match parse_trigger_url(&triggers_text, trigger_id) {
        Ok(u) => u,
        Err(e) => return ExecutionResult::failure(format!("remote_trigger: {e}")),
    };

    let client = match reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
    {
        Ok(c) => c,
        Err(e) => {
            return ExecutionResult::failure(format!(
                "remote_trigger: failed to build http client: {e}"
            ));
        }
    };
    let body = payload.cloned().unwrap_or(serde_json::json!({}));
    let resp = match client.post(&url).json(&body).send().await {
        Ok(r) => r,
        Err(e) => {
            return ExecutionResult::failure(format!(
                "remote_trigger: POST to {url} failed: {e}"
            ));
        }
    };
    let status = resp.status();
    let resp_text = resp.text().await.unwrap_or_default();
    if !status.is_success() {
        return ExecutionResult::failure(format!(
            "remote_trigger: {trigger_id} → HTTP {} from {url}: {}",
            status.as_u16(),
            resp_text.chars().take(200).collect::<String>(),
        ));
    }
    ExecutionResult::success(format!(
        "Triggered {trigger_id} (HTTP {}): {}",
        status.as_u16(),
        resp_text.chars().take(500).collect::<String>(),
    ))
}

pub(super) fn remote_trigger_config_path() -> Option<std::path::PathBuf> {
    dirs::config_dir().map(|d| d.join("jfc").join("triggers.toml"))
}

/// Parse a triggers.toml document and resolve `trigger_id` to its URL.
/// Format: each trigger is a table keyed by id with at minimum a `url`
/// string field. Example:
///   [deploy]
///   url = "https://ci.example.com/hook/deploy"
///
/// Errors:
///   - parse error  → "invalid TOML"
///   - missing id   → "trigger '<id>' not found"
///   - missing url  → "trigger '<id>' has no `url` field"
pub(crate) fn parse_trigger_url(toml_text: &str, trigger_id: &str) -> Result<String, String> {
    let parsed: toml::Value = toml::from_str(toml_text)
        .map_err(|e| format!("invalid triggers.toml: {e}"))?;
    let table = parsed
        .as_table()
        .ok_or_else(|| "triggers.toml must be a TOML table at the top level".to_owned())?;
    let entry = table
        .get(trigger_id)
        .ok_or_else(|| format!("trigger '{trigger_id}' not found in triggers.toml"))?;
    let url = entry
        .get("url")
        .and_then(|v| v.as_str())
        .ok_or_else(|| format!("trigger '{trigger_id}' has no `url` field"))?;
    Ok(url.to_owned())
}

