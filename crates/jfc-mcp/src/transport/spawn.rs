use std::process::Stdio;
use std::sync::Arc;

use rmcp::ServiceExt;
use rmcp::transport::streamable_http_client::StreamableHttpClientTransportConfig;
use rmcp::transport::{StreamableHttpClientTransport, TokioChildProcess};
use tokio::process::Command;

use super::error::service_error_suggests_auth_rejection;
use super::handler::JfcClientHandler;
use super::http::header_map_from_config;
use super::stderr::{empty_ring, new_ring, spawn_stderr_drain};
use super::trace;
use super::{SpawnConfig, Transport, TransportInner, TransportKind};

impl Transport {
    pub async fn spawn(cfg: SpawnConfig) -> Option<Self> {
        let _linkscope_spawn = linkscope::phase("mcp.spawn");
        trace::spawn_start(&cfg);
        match cfg.kind {
            TransportKind::Stdio => Self::spawn_stdio(cfg).await,
            TransportKind::Http => Self::spawn_http(cfg).await,
        }
    }

    async fn spawn_stdio(cfg: SpawnConfig) -> Option<Self> {
        let _linkscope_stdio = linkscope::phase("mcp.spawn.stdio");
        trace::spawn_stdio_command(&cfg);
        let mut command = Command::new(&cfg.command);
        command.args(&cfg.args);
        for (key, value) in &cfg.env {
            command.env(key, value);
        }
        if let Some(cwd) = &cfg.cwd {
            command.current_dir(cwd);
        }

        let (proc, stderr) = match TokioChildProcess::builder(command)
            .stderr(Stdio::piped())
            .spawn()
        {
            Ok(pair) => pair,
            Err(error) => {
                linkscope::record_items("mcp.spawn.stdio.error", 1);
                trace::spawn_result(&cfg.server_name, cfg.kind, "spawn_error");
                tracing::info!(
                    target: "jfc::mcp",
                    server = %cfg.server_name,
                    command = %cfg.command,
                    error = %error,
                    "spawn failed (binary likely not on PATH)"
                );
                return None;
            }
        };

        let stderr_ring = new_ring();
        if let Some(stderr) = stderr {
            spawn_stderr_drain(cfg.server_name.clone(), stderr, Arc::clone(&stderr_ring));
        }

        let client = match JfcClientHandler::new(cfg.server_name.clone())
            .serve(proc)
            .await
        {
            Ok(client) => client,
            Err(error) => {
                linkscope::record_items("mcp.spawn.stdio.handshake_error", 1);
                trace::spawn_result(&cfg.server_name, cfg.kind, "handshake_error");
                tracing::warn!(
                    target: "jfc::mcp",
                    server = %cfg.server_name,
                    error = ?error,
                    "mcp initialize handshake failed"
                );
                return None;
            }
        };

        linkscope::record_items("mcp.spawn.stdio.ok", 1);
        trace::spawn_result(&cfg.server_name, cfg.kind, "ok");
        tracing::info!(
            target: "jfc::mcp",
            server = %cfg.server_name,
            command = %cfg.command,
            "mcp stdio transport ready"
        );
        Some(Self {
            inner: Arc::new(TransportInner {
                server_name: cfg.server_name,
                client,
                has_auth_header: false,
                stderr_ring,
            }),
        })
    }

    async fn spawn_http(cfg: SpawnConfig) -> Option<Self> {
        let _linkscope_http = linkscope::phase("mcp.spawn.http");
        let url = cfg.url.as_deref().unwrap_or_default();
        let (headers, has_auth_header) = match header_map_from_config(&cfg.headers) {
            Ok(headers) => (
                headers,
                cfg.headers
                    .keys()
                    .any(|key| key.eq_ignore_ascii_case("authorization")),
            ),
            Err(error) => {
                linkscope::record_items("mcp.spawn.http.header_error", 1);
                trace::spawn_result(&cfg.server_name, cfg.kind, "header_error");
                tracing::warn!(
                    target: "jfc::mcp",
                    server = %cfg.server_name,
                    error = %error,
                    "invalid MCP HTTP headers"
                );
                return None;
            }
        };
        trace::spawn_http_config(&cfg, has_auth_header);
        let transport = StreamableHttpClientTransport::from_config(
            StreamableHttpClientTransportConfig::with_uri(url).custom_headers(headers),
        );
        let client = match JfcClientHandler::new(cfg.server_name.clone())
            .serve(transport)
            .await
        {
            Ok(client) => client,
            Err(error) => {
                linkscope::record_items("mcp.spawn.http.handshake_error", 1);
                trace::spawn_result(&cfg.server_name, cfg.kind, "handshake_error");
                let rejected_auth =
                    has_auth_header && service_error_suggests_auth_rejection(&error);
                if rejected_auth {
                    tracing::warn!(
                        target: "jfc::mcp",
                        server = %cfg.server_name,
                        url = %url,
                        error = ?error,
                        "mcp http initialize rejected configured Authorization header"
                    );
                } else {
                    tracing::warn!(
                        target: "jfc::mcp",
                        server = %cfg.server_name,
                        url = %url,
                        error = ?error,
                        "mcp http initialize handshake failed"
                    );
                }
                return None;
            }
        };

        linkscope::record_items("mcp.spawn.http.ok", 1);
        trace::spawn_result(&cfg.server_name, cfg.kind, "ok");
        tracing::info!(
            target: "jfc::mcp",
            server = %cfg.server_name,
            url = %url,
            "mcp http transport ready"
        );
        Some(Self {
            inner: Arc::new(TransportInner {
                server_name: cfg.server_name,
                client,
                has_auth_header,
                stderr_ring: empty_ring(),
            }),
        })
    }
}
