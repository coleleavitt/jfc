//! Background-agent registry: roster mutations, queries, and CLI follow.
//!
//! All `record_background_agent_*` functions and the `wait`/`attach` CLI
//! helpers live here. They share two invariants worth remembering:
//!
//! 1. `record_background_agent_started_at` is the only place that creates
//!    a new `BackgroundAgentInfo`. Worker spawn and worker entry both call
//!    through it so the schema/log_path init logic stays in one spot.
//! 2. The PID-clobber guard in `record_background_agent_started_at` is the
//!    last line of defense against the UI process stomping a detached
//!    worker's PID — see `daemon/pid.rs` for the matching `/proc` repair
//!    pass in `reconcile`.
//!
//! Every state mutation does `load → mutate → save` *without* file locking,
//! which is the largest remaining design smell in this module (callers
//! across UI + N workers can race on the JSON file). Treat this as the
//! current contract, not the intended end state.

use std::path::PathBuf;
use std::time::{Duration, Instant, SystemTime};

use super::logs::{append_chunk_raw, append_log_line, background_agent_log_path, read_last_lines};
use super::reconcile::reconcile_background_agents;
use super::state::{
    BackgroundAgentInfo, BackgroundAgentStatus, DaemonPaths, load_state, save_state,
    with_state_lock,
};

/// Persist that a background agent started. Safe to call repeatedly for the
/// same id; later calls refresh mutable metadata without dropping existing
/// logs or terminal fields.
///
/// The caller's PID is passed in: for in-process subagents that's the UI's
/// own PID (and reconcile uses it for liveness checks); for detached
/// agents the `_at` guard inside this module rejects writes from a PID
/// that doesn't own the launch spec, so it's safe to always pass it.
pub fn record_background_agent_started(
    id: &str,
    description: &str,
    model: Option<String>,
    worktree_path: Option<PathBuf>,
) {
    let paths = DaemonPaths::default_user();
    record_background_agent_started_at(
        &paths,
        id,
        description,
        None,
        model,
        worktree_path,
        Some(std::process::id()),
    );
}

pub(super) fn record_background_agent_started_at(
    paths: &DaemonPaths,
    id: &str,
    description: &str,
    parent_session_id: Option<String>,
    model: Option<String>,
    worktree_path: Option<PathBuf>,
    pid: Option<u32>,
) {
    let id = id.to_owned();
    let description_owned = description.to_owned();
    let (log_path, existed) = with_state_lock(paths, || {
        let mut state = load_state(paths).unwrap_or_default();
        let now = SystemTime::now();
        let log_path = state
            .background_agents
            .get(&id)
            .map(|a| a.log_path.clone())
            .unwrap_or_else(|| background_agent_log_path(paths, &id));
        let existed = state.background_agents.contains_key(&id);
        let entry = state
            .background_agents
            .entry(id.clone())
            .or_insert_with(|| BackgroundAgentInfo {
                id: id.clone(),
                description: description_owned.clone(),
                parent_session_id: parent_session_id.clone(),
                status: BackgroundAgentStatus::Running,
                started_at: now,
                updated_at: now,
                completed_at: None,
                pid,
                model: model.clone(),
                worktree_path: worktree_path.clone(),
                log_path: log_path.clone(),
                launch_path: None,
                cancel_requested: false,
                respawn_count: 0,
                summary: None,
                error: None,
                tool_use_count: 0,
                latest_input_tokens: 0,
                latest_cache_read_tokens: 0,
                latest_cache_write_tokens: 0,
                cumulative_output_tokens: 0,
                last_tool: None,
            });
        entry.description = description_owned.clone();
        if parent_session_id.is_some() {
            entry.parent_session_id = parent_session_id.clone();
        }
        entry.status = BackgroundAgentStatus::Running;
        entry.updated_at = now;
        entry.completed_at = None;
        // PID-write contract: each process writes only its own PID. A
        // detached worker (`launch_path.is_some()`) is owned by the worker
        // process — the UI must never overwrite that PID with its own.
        // The check below accepts a write iff (a) the caller IS the
        // owning process (self-write), or (b) no PID has been recorded
        // yet, or (c) this is an in-process task with no launch metadata.
        if let Some(pid) = pid {
            let is_self_write = pid == std::process::id();
            let is_first_write = entry.pid.is_none();
            let is_detached = entry.launch_path.is_some();
            if is_self_write || is_first_write || !is_detached {
                entry.pid = Some(pid);
            }
        }
        if model.is_some() {
            entry.model = model.clone();
        }
        if worktree_path.is_some() {
            entry.worktree_path = worktree_path.clone();
        }
        if !existed {
            entry.cancel_requested = false;
        }
        entry.summary = None;
        entry.error = None;
        let _ = save_state(paths, &state);
        (log_path, existed)
    });
    if !existed {
        append_log_line(&log_path, &format!("[started] {description}"));
    }
}

/// Append streamed worker output to the per-agent log file.
///
/// **Performance contract**: this is called once per text chunk, which
/// for a streaming agent means several times per second. Rewriting the
/// entire `DaemonState` JSON file here would create severe write
/// amplification (8 workers × N chunks/sec × ~30 KB rewrite per chunk).
///
/// Instead, this function only appends to the per-agent `.log` file —
/// nothing in `DaemonState` changes per chunk. The `updated_at`
/// timestamp is refreshed by `record_background_agent_progress`, which
/// fires once per API round-trip rather than once per stream chunk.
///
/// The agent record is *created* lazily here if it doesn't exist yet —
/// covers the edge case where the worker emits a chunk before its own
/// `record_background_agent_started_at` lands.
pub fn record_background_agent_log(id: &str, text: &str) {
    let paths = DaemonPaths::default_user();
    let log_path = with_state_lock(&paths, || {
        let mut state = load_state(&paths).unwrap_or_default();
        if let Some(agent) = state.background_agents.get(id) {
            return agent.log_path.clone();
        }
        // First chunk for an unknown id: seed the roster so subsequent
        // queries see the agent. After this initial create, no further
        // state writes happen for log chunks.
        let now = SystemTime::now();
        let log_path = background_agent_log_path(&paths, id);
        state.background_agents.insert(
            id.to_owned(),
            BackgroundAgentInfo {
                id: id.to_owned(),
                description: id.to_owned(),
                parent_session_id: None,
                status: BackgroundAgentStatus::Running,
                started_at: now,
                updated_at: now,
                completed_at: None,
                pid: Some(std::process::id()),
                model: None,
                worktree_path: None,
                log_path: log_path.clone(),
                launch_path: None,
                cancel_requested: false,
                respawn_count: 0,
                summary: None,
                error: None,
                tool_use_count: 0,
                latest_input_tokens: 0,
                latest_cache_read_tokens: 0,
                latest_cache_write_tokens: 0,
                cumulative_output_tokens: 0,
                last_tool: None,
            },
        );
        let _ = save_state(&paths, &state);
        log_path
    });
    // SSE text deltas arrive in arbitrary chunks ("Let me", " implement",
    // " the full SPIR-V lif", "ter with…"). Writing one `writeln!` per
    // chunk turned the rendered task view into a column of 1-3-word
    // fragments. Append raw so only the model's own `\n` bytes break lines.
    append_chunk_raw(&log_path, text);
}

pub fn record_background_agent_progress(
    id: &str,
    last_tool: Option<&str>,
    tool_use_count: Option<u32>,
    latest_input_tokens: Option<u64>,
    latest_cache_read_tokens: Option<u64>,
    latest_cache_write_tokens: Option<u64>,
    output_tokens_delta: Option<u64>,
) {
    let paths = DaemonPaths::default_user();
    let log_path = with_state_lock(&paths, || {
        let mut state = load_state(&paths).unwrap_or_default();
        let Some(agent) = state.background_agents.get_mut(id) else {
            return None;
        };
        agent.updated_at = SystemTime::now();
        if let Some(n) = tool_use_count {
            agent.tool_use_count = n;
        }
        if let Some(n) = latest_input_tokens {
            agent.latest_input_tokens = n;
        }
        if let Some(n) = latest_cache_read_tokens {
            agent.latest_cache_read_tokens = n;
        }
        if let Some(n) = latest_cache_write_tokens {
            agent.latest_cache_write_tokens = n;
        }
        if let Some(n) = output_tokens_delta {
            agent.cumulative_output_tokens = agent.cumulative_output_tokens.saturating_add(n);
        }
        if let Some(tool) = last_tool {
            agent.last_tool = Some(tool.to_owned());
        }
        let log_path = agent.log_path.clone();
        let _ = save_state(&paths, &state);
        Some(log_path)
    });
    if let (Some(log_path), Some(tool)) = (log_path, last_tool) {
        append_log_line(&log_path, &format!("[tool] {tool}"));
    }
}

pub fn record_background_agent_finished(
    id: &str,
    status: BackgroundAgentStatus,
    summary_or_error: &str,
) {
    let paths = DaemonPaths::default_user();
    let log_path = with_state_lock(&paths, || {
        let mut state = load_state(&paths).unwrap_or_default();
        let now = SystemTime::now();
        let Some(agent) = state.background_agents.get_mut(id) else {
            return None;
        };
        agent.status = status;
        agent.updated_at = now;
        agent.completed_at = Some(now);
        match status {
            BackgroundAgentStatus::Completed => agent.summary = Some(summary_or_error.to_owned()),
            BackgroundAgentStatus::Failed | BackgroundAgentStatus::Cancelled => {
                agent.error = Some(summary_or_error.to_owned())
            }
            BackgroundAgentStatus::Running => {}
        }
        let log_path = agent.log_path.clone();
        let _ = save_state(&paths, &state);
        Some(log_path)
    });
    if let Some(log_path) = log_path {
        append_log_line(
            &log_path,
            &format!("[{:?}] {}", status, summary_or_error.replace('\n', " ")),
        );
    }
}

pub fn background_agent_cancel_requested(id: &str) -> bool {
    let paths = DaemonPaths::default_user();
    load_state(&paths)
        .and_then(|state| state.background_agents.get(id).cloned())
        .map(|agent| agent.cancel_requested && !agent.status.is_terminal())
        .unwrap_or(false)
}

pub fn request_background_agent_cancel(paths: &DaemonPaths, id: &str) -> std::io::Result<()> {
    let result = with_state_lock(paths, || -> std::io::Result<Option<PathBuf>> {
        let mut state = load_state(paths).unwrap_or_default();
        let Some(agent) = state.background_agents.get_mut(id) else {
            return Err(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!("no background agent `{id}`"),
            ));
        };
        if agent.status.is_terminal() {
            return Ok(None);
        }
        agent.cancel_requested = true;
        agent.updated_at = SystemTime::now();
        let log_path = agent.log_path.clone();
        save_state(paths, &state)?;
        Ok(Some(log_path))
    })?;
    if let Some(log_path) = result {
        append_log_line(&log_path, "[cancel-requested]");
    }
    Ok(())
}

pub fn background_agents_string(paths: &DaemonPaths) -> String {
    let state = reconcile_background_agents(paths).unwrap_or_default();
    let mut agents: Vec<_> = state.background_agents.values().collect();
    agents.sort_by_key(|a| a.started_at);
    agents.reverse();
    let mut s = String::new();
    s.push_str("background agents:\n");
    if agents.is_empty() {
        s.push_str("  (none)\n");
        return s;
    }
    for a in agents {
        let age = SystemTime::now()
            .duration_since(a.started_at)
            .unwrap_or_default()
            .as_secs();
        let tokens = a
            .latest_input_tokens
            .saturating_add(a.cumulative_output_tokens);
        let cancel = if a.cancel_requested {
            " cancel-requested"
        } else {
            ""
        };
        s.push_str(&format!(
            "  {} [{:?}{}] age={}s tools={} tokens={} :: {}\n",
            a.id, a.status, cancel, age, a.tool_use_count, tokens, a.description
        ));
        if let Some(wt) = &a.worktree_path {
            s.push_str(&format!("    worktree: {}\n", wt.display()));
        }
        s.push_str(&format!("    log: {}\n", a.log_path.display()));
    }
    s
}

pub fn background_agent_logs_string(paths: &DaemonPaths, id: &str, lines: usize) -> String {
    let state = reconcile_background_agents(paths).unwrap_or_default();
    let Some(agent) = state.background_agents.get(id) else {
        return format!("no background agent `{id}`\n");
    };
    let mut s = format!(
        "{} [{:?}] :: {}\n",
        agent.id, agent.status, agent.description
    );
    for line in read_last_lines(&agent.log_path, lines) {
        s.push_str(&line);
        s.push('\n');
    }
    s
}

pub fn background_agents_for_restore(
    paths: &DaemonPaths,
    parent_session_id: Option<&str>,
    limit: usize,
) -> Vec<BackgroundAgentInfo> {
    let state = reconcile_background_agents(paths).unwrap_or_default();
    let mut agents: Vec<_> = state
        .background_agents
        .into_values()
        .filter(|agent| match parent_session_id {
            Some(session_id) => agent.parent_session_id.as_deref() == Some(session_id),
            None => false,
        })
        .collect();
    agents.sort_by_key(|a| a.started_at);
    agents.reverse();
    let (active, terminal): (Vec<_>, Vec<_>) =
        agents.into_iter().partition(|a| !a.status.is_terminal());
    active.into_iter().chain(terminal).take(limit).collect()
}

pub async fn wait_background_agent_cli(
    paths: &DaemonPaths,
    id: &str,
    timeout: Duration,
) -> std::io::Result<String> {
    let started = Instant::now();
    loop {
        let state = reconcile_background_agents(paths).unwrap_or_default();
        let agent = state.background_agents.get(id).ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!("no background agent `{id}`"),
            )
        })?;
        if agent.status.is_terminal() {
            return Ok(format!(
                "{} finished with {:?}: {}\n",
                agent.id,
                agent.status,
                agent
                    .summary
                    .as_deref()
                    .or(agent.error.as_deref())
                    .unwrap_or("")
            ));
        }
        if started.elapsed() >= timeout {
            return Ok(format!("{} still {:?}\n", agent.id, agent.status));
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
}

pub async fn attach_background_agent_cli(
    paths: &DaemonPaths,
    id: &str,
    lines: usize,
) -> std::io::Result<()> {
    use std::io::{Read, Seek, Write};

    let state = reconcile_background_agents(paths)?;
    let agent = state.background_agents.get(id).ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::NotFound,
            format!("no background agent `{id}`"),
        )
    })?;
    println!("{} [{:?}] :: {}", agent.id, agent.status, agent.description);
    for line in read_last_lines(&agent.log_path, lines) {
        println!("{line}");
    }
    std::io::stdout().flush()?;

    let mut offset = std::fs::metadata(&agent.log_path)
        .map(|m| m.len())
        .unwrap_or(0);
    if agent.status.is_terminal() {
        return Ok(());
    }

    loop {
        let state = reconcile_background_agents(paths)?;
        let agent = state.background_agents.get(id).ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!("no background agent `{id}`"),
            )
        })?;
        if let Ok(mut file) = std::fs::File::open(&agent.log_path) {
            let len = file.metadata().map(|m| m.len()).unwrap_or(0);
            if len < offset {
                offset = 0;
            }
            if len > offset {
                file.seek(std::io::SeekFrom::Start(offset))?;
                let mut buf = String::new();
                file.read_to_string(&mut buf)?;
                print!("{buf}");
                std::io::stdout().flush()?;
                offset = len;
            }
        }
        if agent.status.is_terminal() {
            println!("[{:?}]", agent.status);
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
}
