use std::path::PathBuf;

use clap::Args;

#[derive(Args, Debug)]
pub(super) struct RsiWorkerArgs {
    #[arg(long)]
    input: PathBuf,
    #[arg(long)]
    output: PathBuf,
}

pub(super) async fn run_rsi_worker_subcommand(args: RsiWorkerArgs) -> anyhow::Result<()> {
    jfc_learn::run_rsi_worker_file(&args.input, &args.output).await?;
    Ok(())
}
