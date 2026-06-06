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
    let valid_kinds = [
        "hover",
        "definition",
        "references",
        "implementation",
        "type_definition",
        "document_symbols",
        "workspace_symbols",
        "incoming_calls",
        "outgoing_calls",
    ];
    if !valid_kinds.contains(&kind_norm.as_str()) {
        return ExecutionResult::failure(format!(
            "lsp: invalid kind '{kind}'. Must be one of: {}",
            valid_kinds.join(" | ")
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
    let (tx, _rx) = tokio::sync::mpsc::channel::<crate::runtime::EngineEvent>(16);
    let root_uri = format!("file://{}", cwd.display());
    let owned_args: Vec<&str> = args.to_vec();
    let Some(client) =
        crate::lsp_client::LspClient::spawn(cmd, &owned_args, cwd, &root_uri, tx).await
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
            Some(loc) => format!("{}:{}:{}", loc.file.display(), loc.line + 1, loc.col + 1,),
            None => "lsp: definition not found".to_owned(),
        },
        "references" => {
            let locs = client.find_references_async(&path, line, column).await;
            if locs.is_empty() {
                "lsp: no references found".to_owned()
            } else {
                locs.iter()
                    .map(|loc| format!("{}:{}:{}", loc.file.display(), loc.line + 1, loc.col + 1,))
                    .collect::<Vec<_>>()
                    .join("\n")
            }
        }
        "implementation" | "type_definition" => {
            let method = if kind_norm == "implementation" {
                "textDocument/implementation"
            } else {
                "textDocument/typeDefinition"
            };
            let params = serde_json::json!({
                "textDocument": { "uri": uri },
                "position": {
                    "line": line.saturating_sub(1),
                    "character": column.saturating_sub(1),
                },
            });
            match client.send_request(method, params).await {
                Some(v) => format_location_response(&v),
                None => format!("lsp: {kind_norm} request returned nothing"),
            }
        }
        "document_symbols" => {
            let params = serde_json::json!({
                "textDocument": { "uri": uri },
            });
            match client
                .send_request("textDocument/documentSymbol", params)
                .await
            {
                Some(v) => format_symbols_response(&v),
                None => "lsp: document symbols request returned nothing".to_owned(),
            }
        }
        "workspace_symbols" => {
            let params = serde_json::json!({ "query": "" });
            match client.send_request("workspace/symbol", params).await {
                Some(v) => format_symbols_response(&v),
                None => "lsp: workspace symbols request returned nothing".to_owned(),
            }
        }
        "incoming_calls" | "outgoing_calls" => {
            // Call hierarchy requires a two-step: prepare, then calls
            let prep_params = serde_json::json!({
                "textDocument": { "uri": uri },
                "position": {
                    "line": line.saturating_sub(1),
                    "character": column.saturating_sub(1),
                },
            });
            let items = client
                .send_request("textDocument/prepareCallHierarchy", prep_params)
                .await;
            match items {
                Some(arr) if arr.is_array() && !arr.as_array().unwrap().is_empty() => {
                    let item = &arr.as_array().unwrap()[0];
                    let method = if kind_norm == "incoming_calls" {
                        "callHierarchy/incomingCalls"
                    } else {
                        "callHierarchy/outgoingCalls"
                    };
                    let params = serde_json::json!({ "item": item });
                    match client.send_request(method, params).await {
                        Some(v) => format_call_hierarchy(&v),
                        None => format!("lsp: {kind_norm} returned nothing"),
                    }
                }
                _ => format!("lsp: no call hierarchy item at {file}:{line}:{column}"),
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

fn format_location_response(v: &serde_json::Value) -> String {
    if v.is_null() {
        return "lsp: no locations found".to_owned();
    }
    // Can be a single Location or an array of Locations
    let locations = if v.is_array() {
        v.as_array().unwrap().clone()
    } else {
        vec![v.clone()]
    };
    if locations.is_empty() {
        return "lsp: no locations found".to_owned();
    }
    locations
        .iter()
        .filter_map(|loc| {
            let uri = loc.get("uri")?.as_str()?;
            let range = loc.get("range")?;
            let start = range.get("start")?;
            let line = start.get("line")?.as_u64()? + 1;
            let col = start.get("character")?.as_u64()? + 1;
            let path = uri.strip_prefix("file://")?;
            Some(format!("{path}:{line}:{col}"))
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn format_symbols_response(v: &serde_json::Value) -> String {
    if v.is_null() {
        return "lsp: no symbols found".to_owned();
    }
    let Some(arr) = v.as_array() else {
        return v.to_string();
    };
    if arr.is_empty() {
        return "lsp: no symbols found".to_owned();
    }
    arr.iter()
        .take(50)
        .filter_map(|sym| {
            let name = sym.get("name")?.as_str()?;
            let kind_num = sym.get("kind")?.as_u64().unwrap_or(0);
            let kind_label = lsp_symbol_kind(kind_num);
            // DocumentSymbol has range, SymbolInformation has location
            if let Some(loc) = sym.get("location") {
                let uri = loc.get("uri")?.as_str()?;
                let range = loc.get("range")?;
                let line = range.get("start")?.get("line")?.as_u64()? + 1;
                let path = uri.strip_prefix("file://").unwrap_or(uri);
                Some(format!("{kind_label} {name} — {path}:{line}"))
            } else if let Some(range) = sym.get("range") {
                let line = range.get("start")?.get("line")?.as_u64()? + 1;
                Some(format!("{kind_label} {name} — line {line}"))
            } else {
                Some(format!("{kind_label} {name}"))
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn format_call_hierarchy(v: &serde_json::Value) -> String {
    let Some(arr) = v.as_array() else {
        return "lsp: no calls found".to_owned();
    };
    if arr.is_empty() {
        return "lsp: no calls found".to_owned();
    }
    arr.iter()
        .take(30)
        .filter_map(|entry| {
            // IncomingCall has "from", OutgoingCall has "to"
            let item = entry.get("from").or_else(|| entry.get("to"))?;
            let name = item.get("name")?.as_str()?;
            let uri = item.get("uri")?.as_str()?;
            let range = item.get("range")?;
            let line = range.get("start")?.get("line")?.as_u64()? + 1;
            let path = uri.strip_prefix("file://").unwrap_or(uri);
            Some(format!("{name} — {path}:{line}"))
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn lsp_symbol_kind(kind: u64) -> &'static str {
    match kind {
        1 => "File",
        2 => "Module",
        3 => "Namespace",
        4 => "Package",
        5 => "Class",
        6 => "Method",
        7 => "Property",
        8 => "Field",
        9 => "Constructor",
        10 => "Enum",
        11 => "Interface",
        12 => "Function",
        13 => "Variable",
        14 => "Constant",
        15 => "String",
        16 => "Number",
        17 => "Boolean",
        18 => "Array",
        19 => "Object",
        22 => "Struct",
        23 => "Event",
        24 => "Operator",
        25 => "TypeParameter",
        _ => "Symbol",
    }
}
