pub mod app;

pub mod attachments;
pub mod cli;

pub mod file_watcher;
pub mod input;
pub mod keybindings;
use jfc_markdown as markdown;
pub mod glyphs;
pub mod mentions;
pub mod message_view;
pub mod query;
pub mod render;
pub mod render_cache;
pub mod runtime;
pub mod spinner;
pub mod theme;
pub mod voice;

pub(crate) use cli::{
    CliRuntimeConfig, StartupSession, qualified_model_id, resolve_provider_model,
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
