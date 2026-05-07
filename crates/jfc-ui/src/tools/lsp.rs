use std::path::Path;

use super::ExecutionResult;

pub(super) async fn execute_lsp(
    kind: &str,
    file: &str,
    line: u32,
    column: u32,
    cwd: &Path,
) -> ExecutionResult {
    let kind_norm = kind.to_ascii_lowercase();
    if !matches!(kind_norm.as_str(), "hover" | "definition" | "references") {
        return ExecutionResult::failure(format!(
            "lsp: invalid kind '{kind}'. Must be one of: hover | definition | references"
        ));
    }

    let path = std::path::PathBuf::from(file);
    if !path.is_absolute() {
        return ExecutionResult::failure(format!("lsp: file path must be absolute (got '{file}')"));
    }
    if !path.exists() {
        return ExecutionResult::failure(format!("lsp: file does not exist: {file}"));
    }

    let Some((cmd, args)) = crate::lsp_client::detect_lsp_for_cwd(cwd) else {
        return ExecutionResult::failure(format!(
            "lsp: no language server detected for {} (looked for Cargo.toml, build.zig)",
            cwd.display()
        ));
    };

    // Spawn a discard channel for app events — this client is one-shot
    // and we don't need its publishDiagnostics notifications.
    let (tx, _rx) = tokio::sync::mpsc::channel::<crate::app::AppEvent>(16);
    let root_uri = format!("file://{}", cwd.display());
    let owned_args: Vec<&str> = args.to_vec();
    let Some(client) =
        crate::lsp_client::LspClient::spawn(cmd, &owned_args, &root_uri, tx).await
    else {
        return ExecutionResult::failure(format!(
            "lsp: failed to spawn '{cmd}' (binary not on PATH or handshake timed out)"
        ));
    };

    // The server only pushes useful answers once it has the file open.
    let language_id = match path.extension().and_then(|e| e.to_str()) {
        Some("rs") => "rust",
        Some("zig") => "zig",
        Some("ts") => "typescript",
        Some("tsx") => "typescriptreact",
        Some("js") => "javascript",
        Some("py") => "python",
        Some("go") => "go",
        _ => "plaintext",
    };
    let uri = format!("file://{}", path.display());
    if let Ok(text) = tokio::fs::read_to_string(&path).await {
        client.did_open(&uri, language_id, 1, &text);
        // Give rust-analyzer a moment to index the file before queries.
        // Without this nap, hover/definition often comes back empty.
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    }

    let result = match kind_norm.as_str() {
        "hover" => {
            let params = serde_json::json!({
                "textDocument": { "uri": uri },
                "position": {
                    "line": line.saturating_sub(1),
                    "character": column.saturating_sub(1),
                },
            });
            match client.send_request("textDocument/hover", params).await {
                Some(v) => format_lsp_hover(&v),
                None => "lsp: hover request timed out or returned nothing".to_owned(),
            }
        }
        "definition" => match client.goto_definition_async(&path, line, column).await {
            Some(loc) => format!(
                "{}:{}:{}",
                loc.file.display(),
                loc.line + 1,
                loc.col + 1,
            ),
            None => "lsp: definition not found".to_owned(),
        },
        "references" => {
            let locs = client.find_references_async(&path, line, column).await;
            if locs.is_empty() {
                "lsp: no references found".to_owned()
            } else {
                locs.iter()
                    .map(|loc| {
                        format!(
                            "{}:{}:{}",
                            loc.file.display(),
                            loc.line + 1,
                            loc.col + 1,
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("\n")
            }
        }
        _ => unreachable!("kind validated above"),
    };

    client.shutdown().await;
    ExecutionResult::success(result)
}

pub(super) fn format_lsp_hover(v: &serde_json::Value) -> String {
    // LSP hover responses come as one of:
    //   {"contents": "string"}                — legacy
    //   {"contents": {"kind":"markdown","value":"..."}}
    //   {"contents": [...]}                   — MarkedString[]
    if v.is_null() {
        return "lsp: no hover information".to_owned();
    }
    let contents = v.get("contents").unwrap_or(v);
    if let Some(s) = contents.as_str() {
        return s.to_owned();
    }
    if let Some(obj) = contents.as_object()
        && let Some(val) = obj.get("value").and_then(|v| v.as_str())
    {
        return val.to_owned();
    }
    if let Some(arr) = contents.as_array() {
        let parts: Vec<String> = arr
            .iter()
            .filter_map(|item| {
                if let Some(s) = item.as_str() {
                    Some(s.to_owned())
                } else {
                    item.get("value")
                        .and_then(|v| v.as_str())
                        .map(str::to_owned)
                }
            })
            .collect();
        if !parts.is_empty() {
            return parts.join("\n");
        }
    }
    contents.to_string()
}

