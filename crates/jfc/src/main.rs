pub mod advisor;
pub mod agents;
pub mod app;

pub mod atomic_write;
pub mod attachments;
pub mod auth;
pub mod auto_classifier;
pub mod auto_mode;
pub mod autonomous_loop;
pub mod bash_processes;
pub mod bridge_attestation;
pub mod ccr;
pub mod changeset;
pub mod claude_status;
pub mod cli;
pub mod coach;
pub mod command_spec;
pub mod compact;
pub mod config;
pub mod context;
pub mod cost;
pub mod diagnostics;
pub mod diagnostics_producer;
pub mod document_formats;
pub mod effort;
pub mod env_context;
pub mod exploration;

pub mod feature_gates;
pub mod file_checkpoint;
pub mod file_watcher;
pub mod git_context;
pub mod github;
pub mod goal;
pub mod headless;
pub mod idle_prefetch;
pub mod ids;
pub mod inline_tools;
pub mod input;
pub mod keybindings;
pub mod keywords;
pub mod learn_lifecycle;
pub mod lsp_client;
pub mod lsp_rpc;
pub mod managed_session;
use jfc_markdown as markdown;
pub mod dreamer_scheduler;
pub mod glyphs;
pub mod mcp;
pub mod memory;
pub mod memory_recall;
pub mod mentions;
pub mod message_view;
pub mod notifications;
pub mod output_style;
pub mod plan;
pub mod plan_dreamer;
pub mod plan_recall;
pub mod providers;
pub mod push_notifications;
pub mod query;
pub mod remote_host;
pub mod render;
pub mod render_cache;
pub mod runtime;
pub mod scaffold_detector;
pub mod scheduler;
pub mod sdk_bridge;
pub mod session;
pub mod session_naming;
pub mod slate;
pub mod speculation;
pub mod spinner;
pub mod sprint;
pub mod stream;
pub mod swarm;
pub mod system_reminder;
pub mod team_onboarding;
pub mod theme;
pub mod toast;
pub mod tools;
pub mod types;
pub mod ultraplan;
pub mod web_cache;
pub mod web_search;
pub mod workflows;
pub mod worktrees;

#[cfg(feature = "background-agents")]
pub mod background;
pub mod daemon;
#[cfg(feature = "hashline")]
pub mod hashline;
#[cfg(feature = "hooks")]
pub mod hooks;
/// No-op hooks facade for `--no-default-features` builds: `fire`/`fire_async`
/// short-circuit to `Continue` so call sites need no cfg-gating. (The gated
/// real module is API-identical; this keeps every feature combination
/// compiling — the ungated `hooks::fire` calls in stream/ops previously broke
/// feature-off builds.)
#[cfg(not(feature = "hooks"))]
pub mod hooks {
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum HookPoint {
        OnUserPromptSubmit,
        BeforeStream,
        AfterStream,
        OnHeartbeat,
    }

    pub struct HookContext;

    impl HookContext {
        pub fn for_session(_session_id: &str) -> Self {
            Self
        }
        #[must_use]
        pub fn with_extra(self, _key: &str, _value: String) -> Self {
            self
        }
    }

    pub enum HookAction {
        Continue,
        Abort(String),
    }

    pub fn fire(_point: HookPoint, _ctx: &HookContext) -> HookAction {
        HookAction::Continue
    }

    pub fn fire_async(_point: HookPoint, _ctx: &HookContext) {}

    pub struct HookRegistry;

    pub fn default_registry() -> HookRegistry {
        HookRegistry
    }

    pub fn init_global(_registry: HookRegistry) {}
}
#[cfg(feature = "intent-gate")]
pub mod intent;
#[cfg(feature = "permission-automation")]
pub mod permissions;
// Sandbox module: contains both landlock (feature-gated) and bwrap
// (always-on) sandbox configuration. The BashSandboxConfig type is
// referenced from app state regardless of platform/feature.
pub mod sandbox;
pub mod session_recap;
pub mod slop_guard;

pub(crate) use cli::{
    CliRuntimeConfig, StartupSession, build_providers, provider_for_model, qualified_model_id,
    resolve_provider_model,
};

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
