use clap::Subcommand;
use std::path::PathBuf;

#[derive(Subcommand, Debug)]
pub(super) enum DaemonSubcommand {
    /// Run the daemon in the foreground (cron + wakeup poll loop).
    /// Use `&` or `nohup` from the shell to background it.
    Start,
    /// Stop the running daemon (SIGTERM the PID file).
    Stop,
    /// Print daemon health + session/cron/wakeup counts.
    Status,
    /// List registered cron jobs and pending wakeups.
    List,
    /// Manually fire a cron job by ID once.
    Fire {
        /// Cron job ID returned by `daemon list` (e.g. `cron-abcd1234`).
        id: String,
    },
    /// List persistent background-agent roster.
    Agents,
    /// Print recent log lines for one background agent.
    Logs {
        /// Background agent / Task id.
        id: String,
        /// Number of log lines to show.
        #[arg(long, default_value_t = 80)]
        lines: usize,
    },
    /// Follow a background agent log until it reaches a terminal state.
    Attach {
        /// Background agent / Task id.
        id: String,
        /// Number of existing log lines to print before following.
        #[arg(long, default_value_t = 80)]
        lines: usize,
    },
    /// Wait until a background agent reaches a terminal state.
    Wait {
        /// Background agent / Task id.
        id: String,
        /// Maximum seconds to wait before returning current status.
        #[arg(long, default_value_t = 300)]
        timeout_secs: u64,
    },
    /// Request cancellation for a background agent.
    Kill {
        /// Background agent / Task id.
        id: String,
    },
    /// Internal worker entrypoint for durable background agents.
    #[command(hide = true)]
    Worker {
        /// Launch spec written by the parent process.
        #[arg(long)]
        launch: PathBuf,
    },
}

pub(super) async fn run_daemon_subcommand(sub: DaemonSubcommand) -> anyhow::Result<()> {
    use crate::daemon::{
        DaemonPaths, attach_background_agent_cli, background_agent_logs_string,
        background_agents_string, fire_cron_cli, list_string, request_background_agent_cancel,
        run_daemon, status_string, stop_daemon, wait_background_agent_cli,
    };

    let paths = DaemonPaths::default_user();

    match sub {
        DaemonSubcommand::Start => {
            run_daemon(paths)
                .await
                .map_err(|e| anyhow::anyhow!("daemon start failed: {e}"))?;
            Ok(())
        }
        DaemonSubcommand::Stop => {
            stop_daemon(&paths).map_err(|e| anyhow::anyhow!("daemon stop failed: {e}"))?;
            println!("daemon stopped");
            Ok(())
        }
        DaemonSubcommand::Status => {
            print!("{}", status_string(&paths));
            Ok(())
        }
        DaemonSubcommand::List => {
            print!("{}", list_string(&paths));
            Ok(())
        }
        DaemonSubcommand::Fire { id } => {
            let msg = fire_cron_cli(&paths, &id)
                .await
                .map_err(|e| anyhow::anyhow!("fire failed: {e}"))?;
            println!("{msg}");
            Ok(())
        }
        DaemonSubcommand::Agents => {
            print!("{}", background_agents_string(&paths));
            Ok(())
        }
        DaemonSubcommand::Logs { id, lines } => {
            print!("{}", background_agent_logs_string(&paths, &id, lines));
            Ok(())
        }
        DaemonSubcommand::Attach { id, lines } => {
            attach_background_agent_cli(&paths, &id, lines)
                .await
                .map_err(|e| anyhow::anyhow!("attach failed: {e}"))?;
            Ok(())
        }
        DaemonSubcommand::Wait { id, timeout_secs } => {
            let msg = wait_background_agent_cli(
                &paths,
                &id,
                std::time::Duration::from_secs(timeout_secs),
            )
            .await
            .map_err(|e| anyhow::anyhow!("wait failed: {e}"))?;
            print!("{msg}");
            Ok(())
        }
        DaemonSubcommand::Kill { id } => {
            request_background_agent_cancel(&paths, &id)
                .map_err(|e| anyhow::anyhow!("kill failed: {e}"))?;
            println!("cancel requested for {id}");
            Ok(())
        }
        DaemonSubcommand::Worker { launch } => crate::daemon::run_background_agent_worker(launch)
            .await
            .map_err(|e| anyhow::anyhow!("worker failed: {e}")),
    }
}

pub(super) fn compact_terminal_agents_on_startup() {
    let paths = crate::daemon::DaemonPaths::default_user();
    let Some(mut state) = crate::daemon::load_state(&paths) else {
        return;
    };
    let dropped = crate::daemon::compact_background_agents(
        &mut state,
        std::time::SystemTime::now(),
        crate::daemon::TERMINAL_AGENT_RETENTION,
        crate::daemon::TERMINAL_AGENT_CAP,
    );
    if dropped == 0 {
        return;
    }
    if let Err(err) = crate::daemon::save_state(&paths, &state) {
        tracing::warn!(
            target: "jfc::daemon",
            error = %err,
            dropped,
            "compact_background_agents: save_state failed"
        );
    } else {
        tracing::info!(
            target: "jfc::daemon",
            dropped,
            retained = state.background_agents.len(),
            "compacted terminal background-agent records on startup"
        );
    }
}
