mod advisor;
mod agents;
mod app;

mod atomic_write;
mod attachments;
#[allow(dead_code)]
mod auth;
#[allow(dead_code)]
mod auto_classifier;
mod auto_mode;
#[allow(dead_code)]
mod autonomous_loop;
mod bash_processes;
#[allow(dead_code)]
mod bridge_attestation;
#[allow(dead_code)]
mod ccr;
mod claude_status;
mod cli;
#[allow(dead_code)]
mod coach;
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
#[allow(dead_code)]
mod file_checkpoint;
mod file_watcher;
mod git_context;
mod github;
mod goal;
#[allow(dead_code)]
mod headless;
mod idle_prefetch;
mod ids;
mod inline_tools;
mod input;
mod keybindings;
mod keywords;
mod learn_lifecycle;
mod lsp_client;
mod lsp_rpc;
mod managed_session;
use jfc_markdown as markdown;
mod dreamer_scheduler;
mod mcp;
mod memory;
mod memory_recall;
mod mentions;
mod message_view;
mod notifications;
mod output_style;
#[allow(dead_code)] // Public PlanStore surface — full integration pending streaming/recall wiring.
mod plan;
mod plan_dreamer;
#[allow(dead_code)] // Wired by future request-builder integration task.
mod plan_recall;
mod providers;
mod push_notifications;
mod query;
mod remote_host;
mod render;
mod render_cache;
mod runtime;
mod scaffold_detector;
mod scheduler;
mod sdk_bridge;
mod session;
mod session_naming;
mod slate;
#[allow(dead_code)]
mod speculation;
mod spinner;
mod sprint;
mod stream;
mod swarm;
mod system_reminder;
#[allow(dead_code)]
mod team_onboarding;
mod theme;
mod toast;
mod tools;
mod types;
#[allow(dead_code)]
mod ultraplan;
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
// Sandbox module: contains both landlock (feature-gated) and bwrap
// (always-on) sandbox configuration. The BashSandboxConfig type is
// referenced from app state regardless of platform/feature.
#[allow(dead_code)]
mod sandbox;
#[allow(dead_code)]
mod session_recap;
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

pub(crate) use cli::{CliRuntimeConfig, StartupSession, build_providers, provider_for_model};

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
