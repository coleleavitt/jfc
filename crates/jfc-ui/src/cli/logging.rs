/// Initialize tracing so structured logs flow to `~/.config/jfc/logs/`.
/// Returns the `WorkerGuard` from `tracing-appender::non_blocking` — caller
/// must hold it until process exit so buffered logs flush.
///
/// File routing:
/// - **Interactive UI** (`is_short_lived_cli=false`): per-session file
///   `ses_YYYYMMDD_HHMMSS.log`, with `latest.log` symlink kept in sync.
///   Each UI session is its own file so a crash trace doesn't get mixed
///   with the next run.
/// - **CLI subcommand** (`is_short_lived_cli=true`): a single shared
///   `jfc-cli.log`. Subcommands like `daemon agents`/`status`/`fire`
///   exit in milliseconds; giving each its own file would leave a
///   per-invocation empty file behind (we used to ship hundreds of
///   them — see the cleanup pass below).
///
/// On startup, also unlinks empty `ses_*.log` files older than 1 hour
/// to garbage-collect leftovers from previous buggy runs.
pub(super) fn init_tracing(
    is_short_lived_cli: bool,
) -> tracing_appender::non_blocking::WorkerGuard {
    use tracing_subscriber::EnvFilter;

    let log_dir = dirs::config_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join("jfc")
        .join("logs");
    let _ = std::fs::create_dir_all(&log_dir);

    // Sweep empty `ses_*.log` files left behind by previous short-lived
    // CLI invocations or buggy launches. Only target files >1h old so a
    // live UI session that hasn't logged its first line yet stays put.
    cleanup_empty_session_logs(&log_dir);

    let log_path = if is_short_lived_cli {
        // Short-lived subcommand — share one file across all CLI calls.
        log_dir.join("jfc-cli.log")
    } else {
        // Interactive UI — own its session file.
        let now = chrono::Local::now();
        let log_filename = format!("ses_{}.log", now.format("%Y%m%d_%H%M%S"));
        log_dir.join(log_filename)
    };

    let file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)
        .unwrap_or_else(|_| {
            // Fallback to /dev/null equivalent
            std::fs::OpenOptions::new()
                .write(true)
                .open(if cfg!(unix) { "/dev/null" } else { "NUL" })
                .expect("cannot open null device")
        });

    if !is_short_lived_cli {
        // Update `latest.log` symlink only for interactive sessions —
        // CLI subcommands shouldn't redirect what "latest" means.
        let latest_link = log_dir.join("latest.log");
        let _ = std::fs::remove_file(&latest_link);
        #[cfg(unix)]
        {
            if let Some(name) = log_path.file_name() {
                let _ = std::os::unix::fs::symlink(name, &latest_link);
            }
        }
        #[cfg(not(unix))]
        {
            let _ = std::fs::copy(&log_path, &latest_link);
        }
    }

    let (writer, guard) = tracing_appender::non_blocking(file);

    // Filter resolution: if RUST_LOG is set externally (e.g. by a Wayland
    // compositor like niri that bakes its own `niri=debug,…` filter into
    // the environment), `try_from_default_env` honors it verbatim — and
    // any target it doesn't mention drops to the implicit "off" default.
    // That silently swallowed every `jfc::*` event for users with
    // unrelated RUST_LOG values, leaving `~/.config/jfc/logs/*.log` at
    // 0 bytes and making bugs invisible.
    //
    // Fix: when RUST_LOG is set but doesn't mention a `jfc` directive,
    // append `jfc=debug` so our targets stay visible. When the user
    // explicitly set a `jfc=…` directive, leave it alone — their
    // override wins.
    let env_rust_log = std::env::var("RUST_LOG").unwrap_or_default();
    let filter = if env_rust_log.is_empty() {
        EnvFilter::new("debug,reqwest=warn,hyper=warn,h2=warn")
    } else if env_rust_log
        .split(',')
        .any(|d| d.trim().starts_with("jfc"))
    {
        EnvFilter::try_from_default_env()
            .unwrap_or_else(|_| EnvFilter::new("debug,reqwest=warn,hyper=warn,h2=warn"))
    } else {
        // Append our default so jfc events survive an unrelated RUST_LOG.
        let combined = format!("{env_rust_log},jfc=debug");
        EnvFilter::try_new(&combined)
            .unwrap_or_else(|_| EnvFilter::new("debug,reqwest=warn,hyper=warn,h2=warn"))
    };

    if let Err(e) = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(writer)
        .with_ansi(false) // file output — no ANSI escapes
        .with_target(true)
        .with_file(false)
        .with_line_number(false)
        .with_thread_ids(false)
        .try_init()
    {
        // Subscriber already set (or failed). Don't silently swallow — write a
        // breadcrumb to the log dir so the user has *something* to look at when
        // logs come up empty.
        let _ = std::fs::write(
            log_dir.join("tracing-init-error.txt"),
            format!("tracing init failed: {e}\n"),
        );
    }

    tracing::info!(log_dir = %log_dir.display(), "tracing initialized");
    guard
}

/// Remove zero-byte `ses_*.log` files older than one hour.
///
/// We used to create a fresh `ses_YYYYMMDD_HHMMSS.log` file for every
/// process start, including each short-lived CLI subcommand. Most CLI
/// runs exited before writing a line, leaving the log directory full of
/// empty files (237 on one local box). This pass GC's that leftover
/// set on every startup. Best-effort: any IO error is silently ignored.
fn cleanup_empty_session_logs(log_dir: &std::path::Path) {
    let Ok(entries) = std::fs::read_dir(log_dir) else {
        return;
    };
    let cutoff = std::time::SystemTime::now()
        .checked_sub(std::time::Duration::from_secs(3600))
        .unwrap_or(std::time::SystemTime::UNIX_EPOCH);
    for entry in entries.flatten() {
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        if !name.starts_with("ses_") || !name.ends_with(".log") {
            continue;
        }
        let Ok(meta) = entry.metadata() else {
            continue;
        };
        if meta.len() != 0 {
            continue;
        }
        let Ok(modified) = meta.modified() else {
            continue;
        };
        if modified > cutoff {
            continue;
        }
        let _ = std::fs::remove_file(&path);
    }
}
