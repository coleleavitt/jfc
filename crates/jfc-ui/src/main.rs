#![allow(dead_code, unused_imports, unused_mut, unused_variables)]
#![allow(clippy::all)]

mod advisor;
mod agents;
mod app;

mod attachments;
mod auto_mode;
mod bash_processes;
mod claude_status;
mod cli;
mod compact;
mod config;
mod context;
mod cost;
mod credential_vault;
mod diagnostics;
mod diagnostics_producer;
mod document_formats;
mod effort;
mod env_context;
mod event_loop;
mod feature_gates;
mod file_watcher;
mod fleet_view;
mod git_context;
mod github;
mod goal;
mod idle_prefetch;
mod ids;
mod inline_tools;
mod input;
mod keybindings;
mod lsp_client;
mod lsp_rpc;
mod managed_session;
mod markdown;
mod mcp;
mod memory;
mod memory_recall;
mod mentions;
mod message_view;
mod notifications;
mod output_style;
mod plan_cache;
mod provider;
mod providers;
mod push_notifications;
mod query;
mod render;
mod render_cache;
mod runtime;
mod scheduler;
mod sdk_bridge;
mod session;
mod session_naming;
mod slash_commands;
mod slate;
mod speculation;
mod spinner;
mod stream;
mod swarm;
mod system_reminder;
mod tasks;
mod telemetry;
mod theme;
mod toast;
mod tools;
mod types;
mod web_cache;
mod web_search;
mod workflows;
mod worktrees;

#[cfg(feature = "background-agents")]
mod background;
mod daemon;
mod hallucination_guard;
#[cfg(feature = "hashline")]
mod hashline;
#[cfg(feature = "hooks")]
mod hooks;
#[cfg(feature = "intent-gate")]
mod intent;
#[cfg(feature = "permission-automation")]
mod permissions;
#[cfg(feature = "landlock-sandbox")]
mod sandbox;
mod slop_guard;

pub(crate) use cli::{StartupSession, build_providers, provider_for_model};

use clap::Parser;

#[cfg(feature = "dhat-heap")]
#[global_allocator]
static ALLOC: dhat::Alloc = dhat::Alloc;

#[tokio::main(worker_threads = 4)]
async fn main() -> anyhow::Result<()> {
    #[cfg(feature = "dhat-heap")]
    let _dhat_profiler = dhat::Profiler::new_heap();

    let cli = cli::Cli::parse();
    cli::run(cli).await
}
