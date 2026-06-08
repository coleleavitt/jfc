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
    /// List durable worker control-plane records.
    Controls,
    /// Resolve and mark a spare worker binary ready.
    Spare {
        /// Optional worker executable to pin for future respawns.
        #[arg(long)]
        worker_exe: Option<PathBuf>,
    },
    /// Take over a stale background worker from its launch spec.
    Takeover {
        /// Background agent / Task id.
        id: String,
        /// Take over even if the recorded owner PID still appears live.
        #[arg(long)]
        force: bool,
        /// Audit reason recorded in daemon state and the worker log.
        #[arg(long)]
        reason: Option<String>,
    },
    /// Request daemon restart/binary handoff on the next daemon tick.
    BinaryTakeover {
        /// Optional replacement worker executable.
        #[arg(long)]
        worker_exe: Option<PathBuf>,
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
    use jfc_engine::daemon::{
        DaemonPaths, WorkerControlKind, WorkerControlRequest, attach_background_agent_cli,
        background_agent_logs_string, background_agents_string, fire_cron_cli, list_string,
        request_background_agent_cancel, request_worker_control, run_daemon, status_string,
        stop_daemon, wait_background_agent_cli, worker_controls_string,
    };

    let paths = DaemonPaths::default_user();

    match sub {
        DaemonSubcommand::Start => {
            // Spawn the overnight dreamer scheduler before the cron loop.
            // It runs PlanDreamer + jfc-learn Dreamer on the configured
            // interval (JFC_PLAN_DREAMER_INTERVAL, default 1h). Both
            // dreamers own their own lease + circuit breaker, so the
            // scheduler is fire-and-forget; the handle is kept alive for
            // the daemon's lifetime so the spawned task isn't dropped.
            let project_root =
                std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
            let _dreamer_handle = jfc_engine::dreamer_scheduler::spawn_from_env(project_root);

            run_daemon(paths)
                .await
                .map_err(|e| anyhow::anyhow!("daemon start failed: {e}"))?;

            drop(_dreamer_handle);
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
        DaemonSubcommand::Controls => {
            print!("{}", worker_controls_string(&paths));
            Ok(())
        }
        DaemonSubcommand::Spare { worker_exe } => {
            let id = request_worker_control(
                &paths,
                WorkerControlRequest {
                    kind: WorkerControlKind::PrepareSpare,
                    agent_id: None,
                    target_pid: None,
                    worker_exe,
                    force: false,
                    reason: Some("manual spare preparation".to_owned()),
                },
            )
            .map_err(|e| anyhow::anyhow!("spare request failed: {e}"))?;
            println!("worker spare request queued: {id}");
            Ok(())
        }
        DaemonSubcommand::Takeover { id, force, reason } => {
            let control_id = request_worker_control(
                &paths,
                WorkerControlRequest {
                    kind: WorkerControlKind::Takeover,
                    agent_id: Some(id.clone()),
                    target_pid: None,
                    worker_exe: None,
                    force,
                    reason: Some(reason.unwrap_or_else(|| "manual takeover".to_owned())),
                },
            )
            .map_err(|e| anyhow::anyhow!("takeover request failed: {e}"))?;
            println!("worker takeover queued for {id}: {control_id}");
            Ok(())
        }
        DaemonSubcommand::BinaryTakeover { worker_exe } => {
            let id = request_worker_control(
                &paths,
                WorkerControlRequest {
                    kind: WorkerControlKind::BinaryTakeover,
                    agent_id: None,
                    target_pid: None,
                    worker_exe,
                    force: false,
                    reason: Some("manual binary takeover".to_owned()),
                },
            )
            .map_err(|e| anyhow::anyhow!("binary takeover request failed: {e}"))?;
            println!("binary takeover queued: {id}");
            Ok(())
        }
        DaemonSubcommand::Worker { launch } => {
            jfc_engine::daemon::run_background_agent_worker(launch)
                .await
                .map_err(|e| anyhow::anyhow!("worker failed: {e}"))
        }
    }
}

pub(super) fn compact_terminal_agents_on_startup() {
    let paths = jfc_engine::daemon::DaemonPaths::default_user();
    let Some(mut state) = jfc_engine::daemon::load_state(&paths) else {
        return;
    };
    let dropped = jfc_engine::daemon::compact_background_agents(
        &mut state,
        std::time::SystemTime::now(),
        jfc_engine::daemon::TERMINAL_AGENT_RETENTION,
        jfc_engine::daemon::TERMINAL_AGENTS_PER_SESSION,
        jfc_engine::daemon::TERMINAL_AGENT_GLOBAL_CAP,
    );
    if dropped == 0 {
        return;
    }
    if let Err(err) = jfc_engine::daemon::save_state(&paths, &state) {
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
