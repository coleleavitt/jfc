use crate::app::App;
use crate::types::ChatMessage;

/// Dispatch `/mcp ...` subcommands.
///
/// - `/mcp` or `/mcp list` shows every configured MCP server, its connection
///   status, and the count of tools it exposes.
/// - `/mcp restart <name>` kills the running server, if any, and re-spawns it
///   from cached config.
/// - `/mcp logs <name>` prints recent stderr lines from the server transport's
///   ring buffer.
pub(super) async fn handle_mcp_command(app: &mut App, args: &str) {
    let mut it = args.split_whitespace();
    let sub = it.next().unwrap_or("list");
    let arg = it.next().unwrap_or("");

    let raw = if args.is_empty() {
        "/mcp".to_owned()
    } else {
        format!("/mcp {args}")
    };

    let Some(registry) = crate::tools::snapshot_mcp_registry() else {
        app.messages.push(ChatMessage::user(raw));
        app.messages.push(ChatMessage::assistant(
            "MCP registry not initialized. Add `[mcp.<name>]` blocks to \
             `~/.config/jfc/config.toml` and restart jfc."
                .to_owned(),
        ));
        return;
    };

    match sub {
        "" | "list" => {
            let servers = registry.list().await;
            let body = if servers.is_empty() {
                "No MCP servers configured. Add `[mcp.<name>]` blocks to \
                 `~/.config/jfc/config.toml`."
                    .to_owned()
            } else {
                let mut s = format!("**{} MCP server(s):**\n\n", servers.len());
                for srv in &servers {
                    s.push_str(&format!(
                        "- `{}` — *{}* — {} tool{}\n",
                        srv.name,
                        srv.status.label(),
                        srv.tools.len(),
                        if srv.tools.len() == 1 { "" } else { "s" }
                    ));
                }
                s
            };
            app.messages.push(ChatMessage::user(raw));
            app.messages.push(ChatMessage::assistant(body));
        }
        "restart" => {
            if arg.is_empty() {
                app.messages.push(ChatMessage::user(raw));
                app.messages.push(ChatMessage::assistant(
                    "Usage: `/mcp restart <name>`.".to_owned(),
                ));
                return;
            }
            app.messages.push(ChatMessage::user(raw));
            let body = match crate::mcp::restart_server(&registry, arg).await {
                Some(true) => format!("MCP server `{arg}` restarted and reconnected."),
                Some(false) => format!(
                    "MCP server `{arg}` was restarted but failed to reconnect. \
                     See `/mcp logs {arg}` for stderr."
                ),
                None => format!("MCP server `{arg}` is not configured."),
            };
            app.messages.push(ChatMessage::assistant(body));
        }
        "logs" => {
            if arg.is_empty() {
                app.messages.push(ChatMessage::user(raw));
                app.messages.push(ChatMessage::assistant(
                    "Usage: `/mcp logs <name>`.".to_owned(),
                ));
                return;
            }
            let body = match registry.get(arg).await {
                None => format!("MCP server `{arg}` is not configured."),
                Some(server) => match server.transport.as_ref() {
                    None => format!(
                        "MCP server `{arg}` has no live transport (status: {}).",
                        server.status.label()
                    ),
                    Some(transport) => {
                        let lines = transport.recent_stderr().await;
                        if lines.is_empty() {
                            format!("MCP server `{arg}` — no stderr captured yet.")
                        } else {
                            let recent: Vec<&String> = lines.iter().rev().take(50).collect();
                            let mut body = format!(
                                "**`{arg}` stderr (last {} line{}):**\n\n```\n",
                                recent.len(),
                                if recent.len() == 1 { "" } else { "s" }
                            );
                            for l in recent.iter().rev() {
                                body.push_str(l);
                                body.push('\n');
                            }
                            body.push_str("```\n");
                            body
                        }
                    }
                },
            };
            app.messages.push(ChatMessage::user(raw));
            app.messages.push(ChatMessage::assistant(body));
        }
        other => {
            app.messages.push(ChatMessage::user(raw));
            app.messages.push(ChatMessage::assistant(format!(
                "Unknown subcommand `{other}`. Try `/mcp list`, `/mcp restart <name>`, or `/mcp logs <name>`."
            )));
        }
    }
}
