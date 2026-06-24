use std::path::Path;

use super::ExecutionResult;

pub async fn execute_lsp(
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
        // Newly supported kinds
        "diagnostics",
        "code_action",
        "rename",
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

    // Fast-path diagnostics: if we already have a cached publishDiagnostics snapshot,
    // surface that without requiring a running language server.
    if kind_norm == "diagnostics" {
        let mut lines: Vec<String> = Vec::new();
        let entries = crate::diagnostics::global_snapshot();
        let file_str = path.display().to_string();
        let mut matched: Vec<&crate::diagnostics::DiagnosticEntry> =
            entries.iter().filter(|e| e.file == file_str).collect();
        if matched.is_empty() {
            // Try matching by basename as a fallback (different absolute paths across OS/users).
            if let Some(name) = path.file_name().and_then(|s| s.to_str()) {
                matched = entries.iter().filter(|e| e.file.ends_with(name)).collect();
            }
        }
        if !matched.is_empty() {
            lines.push(format!("Diagnostics for {}:", file_str));
            for e in matched {
                lines.push(crate::diagnostics::format_entry(e));
            }
            return ExecutionResult::success(lines.join("\n"));
        }
        // No cache available for this file — we'll try a pull request if a server exists.
    }

    let Some((cmd, args)) = crate::lsp_client::detect_lsp_for_cwd(cwd) else {
        if kind_norm == "diagnostics" {
            return ExecutionResult::success(format!(
                "lsp: no cached diagnostics for {file} and no language server detected for {}",
                cwd.display()
            ));
        }
        return ExecutionResult::failure(format!(
            "lsp: no language server detected for {} (looked for Cargo.toml, build.zig)",
            cwd.display()
        ));
    };

    // Spawn a discard channel for app events — this client is one-shot
    // and we don't need its publishDiagnostics notifications.
    let (tx, _rx) = tokio::sync::mpsc::channel::<crate::runtime::EngineEvent>(16);
    let Some(root_uri) = crate::lsp_client::path_to_file_uri(cwd) else {
        return ExecutionResult::failure(format!("lsp: invalid workspace path {}", cwd.display()));
    };
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
    let Some(uri) = crate::lsp_client::path_to_file_uri(&path) else {
        return ExecutionResult::failure(format!("lsp: invalid file path {}", path.display()));
    };
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
            Some(loc) => format!("{}:{}:{}", loc.file.display(), loc.line, loc.col,),
            None => "lsp: definition not found".to_owned(),
        },
        "references" => {
            let locs = client.find_references_async(&path, line, column).await;
            if locs.is_empty() {
                "lsp: no references found".to_owned()
            } else {
                locs.iter()
                    .map(|loc| format!("{}:{}:{}", loc.file.display(), loc.line, loc.col,))
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
        "diagnostics" => {
            // Pull-based diagnostics — not widely supported. Best-effort.
            let params = serde_json::json!({
                "textDocument": { "uri": uri },
                // Identifier per spec; arbitrary stable string.
                "identifier": { "value": "jfc" }
            });
            match client.send_request("textDocument/diagnostic", params).await {
                Some(v) if !v.is_null() => format!("{}", v),
                _ => format!(
                    "lsp: pull-diagnostics not supported by this server (no cached diagnostics for {file})"
                ),
            }
        }
        "code_action" => {
            let params = serde_json::json!({
                "textDocument": { "uri": uri },
                "range": {
                    "start": {"line": line.saturating_sub(1), "character": column.saturating_sub(1)},
                    "end": {"line": line.saturating_sub(1), "character": column.saturating_sub(1)}
                },
                "context": {"diagnostics": []}
            });
            match client.send_request("textDocument/codeAction", params).await {
                Some(v) => format_code_actions(&v),
                None => "lsp: codeAction request returned nothing".to_owned(),
            }
        }
        "rename" => {
            // Safe preview: only probe whether rename is available and show the range/placeholder.
            let params = serde_json::json!({
                "textDocument": { "uri": uri },
                "position": {
                    "line": line.saturating_sub(1),
                    "character": column.saturating_sub(1),
                },
            });
            match client
                .send_request("textDocument/prepareRename", params)
                .await
            {
                Some(v) if v.is_null() => "lsp: rename not available at this location".to_owned(),
                Some(v) => format_prepare_rename(&v),
                None => "lsp: prepareRename returned nothing".to_owned(),
            }
        }
        _ => unreachable!("kind validated above"),
    };

    client.shutdown().await;
    ExecutionResult::success(result)
}

fn format_code_actions(v: &serde_json::Value) -> String {
    // textDocument/codeAction returns an array of CodeAction | Command.
    let Some(arr) = v.as_array() else {
        return v.to_string();
    };
    if arr.is_empty() {
        return "lsp: no code actions available".to_owned();
    }
    let mut lines = Vec::new();
    for (i, item) in arr.iter().enumerate() {
        if let Some(title) = item.get("title").and_then(|v| v.as_str()) {
            let kind = item.get("kind").and_then(|v| v.as_str()).unwrap_or("");
            let suffix = if kind.is_empty() {
                String::new()
            } else {
                format!(" ({kind})")
            };
            lines.push(format!("{}. {}{}", i + 1, title, suffix));
        } else if let Some(command) = item.get("command").and_then(|v| v.as_str()) {
            lines.push(format!("{}. Command: {}", i + 1, command));
        }
    }
    if lines.is_empty() {
        "lsp: no code actions available".to_owned()
    } else {
        lines.join("\n")
    }
}

fn format_prepare_rename(v: &serde_json::Value) -> String {
    // prepareRename result may be Range or { range, placeholder } or null/not supported
    if v.is_null() {
        return "lsp: rename not available at this location".to_owned();
    }
    if let Some(obj) = v.as_object() {
        let placeholder = obj
            .get("placeholder")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        if let Some(range) = obj.get("range") {
            let start = range.get("start").and_then(|s| {
                Some((
                    s.get("line")?.as_u64()? + 1,
                    s.get("character")?.as_u64()? + 1,
                ))
            });
            let end = range.get("end").and_then(|s| {
                Some((
                    s.get("line")?.as_u64()? + 1,
                    s.get("character")?.as_u64()? + 1,
                ))
            });
            if let (Some((sl, sc)), Some((el, ec))) = (start, end) {
                if placeholder.is_empty() {
                    return format!("rename available — selection {}:{}..{}:{}", sl, sc, el, ec);
                } else {
                    return format!(
                        "rename available — `{}` at {}:{}..{}:{}",
                        placeholder, sl, sc, el, ec
                    );
                }
            }
        }
        if !placeholder.is_empty() {
            return format!("rename available — `{}`", placeholder);
        }
        return "rename available".to_owned();
    }
    // Range-only form
    if let Some(range) = v.get("range") {
        let start = range.get("start").and_then(|s| {
            Some((
                s.get("line")?.as_u64()? + 1,
                s.get("character")?.as_u64()? + 1,
            ))
        });
        let end = range.get("end").and_then(|s| {
            Some((
                s.get("line")?.as_u64()? + 1,
                s.get("character")?.as_u64()? + 1,
            ))
        });
        if let (Some((sl, sc)), Some((el, ec))) = (start, end) {
            return format!("rename available — selection {}:{}..{}:{}", sl, sc, el, ec);
        }
    }
    v.to_string()
}

pub fn format_lsp_hover(v: &serde_json::Value) -> String {
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
            let path = format_uri_path(uri);
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
                let path = format_uri_path(uri);
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
            let path = format_uri_path(uri);
            Some(format!("{name} — {path}:{line}"))
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn format_uri_path(uri: &str) -> String {
    crate::lsp_client::file_uri_to_path(uri)
        .map(|path| path.display().to_string())
        .unwrap_or_else(|| uri.to_owned())
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn format_location_response_decodes_file_uri_regression() {
        let out = format_location_response(&json!({
            "uri": "file:///tmp/jfc%20lsp/%23file%25.rs",
            "range": {
                "start": { "line": 0, "character": 2 },
                "end": { "line": 0, "character": 5 }
            }
        }));

        assert_eq!(out, "/tmp/jfc lsp/#file%.rs:1:3");
    }

    #[test]
    fn format_uri_path_leaves_non_file_uri_visible_robust() {
        assert_eq!(format_uri_path("untitled:scratch"), "untitled:scratch");
    }
}
