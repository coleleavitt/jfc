mod advisor;
mod agents;
mod app;

mod atomic_write;
mod attachments;
mod auto_mode;
mod bash_processes;
mod claude_status;
mod cli;
mod compact;
mod config;
mod context;
mod cost;
mod diagnostics;
mod diagnostics_producer;
mod document_formats;
mod effort;
mod env_context;

mod feature_gates;
mod file_watcher;
mod git_context;
mod github;
mod goal;
mod idle_prefetch;
mod ids;
mod inline_tools;
mod input;
mod keybindings;
mod keywords;
mod lsp_client;
mod lsp_rpc;
mod managed_session;
use jfc_markdown as markdown;
mod mcp;
mod memory;
mod memory_recall;
mod mentions;
mod message_view;
mod notifications;
mod output_style;
mod plan;
mod plan_dreamer;
mod plan_recall;
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
mod slate;
mod spinner;
mod sprint;
mod stream;
mod swarm;
mod system_reminder;
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

/// Returns `true` when the landlock sandbox feature is enabled AND the
/// sandbox was successfully initialized for this process. Used by the
/// permission system to auto-approve tool calls without prompting.
#[cfg(feature = "landlock-sandbox")]
pub(crate) fn is_sandbox_active() -> bool {
    sandbox::is_sandbox_active()
}

#[cfg(not(feature = "landlock-sandbox"))]
pub(crate) fn is_sandbox_active() -> bool {
    false
}

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
