use crate::tools;

fn truncate_chars(text: &str, max_chars: usize) -> String {
    let mut iter = text.chars();
    let mut out: String = iter.by_ref().take(max_chars).collect();
    if iter.next().is_some() {
        out.push_str("\n...[truncated]");
    }
    out
}

pub(super) async fn mcp_server_instructions_section() -> String {
    const MAX_SERVERS: usize = 8;
    const MAX_CHARS_PER_SERVER: usize = 6_000;
    const MAX_TOTAL_CHARS: usize = 18_000;

    let Some(registry) = tools::snapshot_mcp_registry() else {
        return String::new();
    };
    let entries = registry.all_server_instructions().await;
    if entries.is_empty() {
        return String::new();
    }

    let mut out = String::from(
        "## MCP Server Instructions\n\n\
         Connected MCP servers provided these usage instructions during the \
         `initialize` handshake. Follow the instructions for a server when \
         using tools from that server.\n",
    );
    let mut used = out.chars().count();
    let mut included = 0usize;
    for (name, instructions) in entries.into_iter().take(MAX_SERVERS) {
        let body = truncate_chars(&instructions, MAX_CHARS_PER_SERVER);
        let block = format!("\n### {name}\n{body}\n");
        let block_chars = block.chars().count();
        if used + block_chars > MAX_TOTAL_CHARS {
            out.push_str("\n...[additional MCP instructions omitted]\n");
            break;
        }
        out.push_str(&block);
        used += block_chars;
        included += 1;
    }

    if included == 0 { String::new() } else { out }
}

/// Render the connected MCP servers' behavior-affecting tool metadata
/// (annotation hints + titles) into a prompt section. Delegates the rendering
/// to the registry, which owns the metadata; this just snapshots the registry
/// and caps total size.
pub(super) async fn mcp_tool_metadata_section() -> String {
    const MAX_TOTAL_CHARS: usize = 8_000;
    let Some(registry) = tools::snapshot_mcp_registry() else {
        return String::new();
    };
    let section = registry.tool_metadata_prompt_section().await;
    truncate_chars(&section, MAX_TOTAL_CHARS)
}
