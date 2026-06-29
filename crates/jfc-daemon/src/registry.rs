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
    BackgroundAgentInfo, BackgroundAgentStatus, DaemonPaths, load_state, load_state_for_update,
    save_state, with_state_lock,
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

pub fn record_background_agent_started_at(
    paths: &DaemonPaths,
    id: &str,
    description: &str,
    parent_session_id: Option<String>,
    model: Option<String>,
    worktree_path: Option<PathBuf>,
    pid: Option<u32>,
) {
    let _linkscope_started = linkscope::phase("daemon.agent.started");
    linkscope::event_fields(
        "daemon.agent.started.start",
        [
            linkscope::TraceField::text("id", id.to_owned()),
            linkscope::TraceField::count("has_parent", u64::from(parent_session_id.is_some())),
            linkscope::TraceField::count("has_model", u64::from(model.is_some())),
            linkscope::TraceField::count("has_worktree", u64::from(worktree_path.is_some())),
            linkscope::TraceField::count("pid", pid.map(u64::from).unwrap_or(0)),
            linkscope::TraceField::bytes(
                "description_bytes",
                usize_to_u64_saturating(description.len()),
            ),
        ],
    );
    let id = id.to_owned();
    let description_owned = description.to_owned();
    let result = with_state_lock(paths, || {
        let mut state = match load_state_for_update(paths) {
            Ok(s) => s,
            Err(e) => {
                tracing::error!(target: "jfc::daemon", error = %e, "refusing to overwrite corrupt daemon state in record_background_agent_started_at");
                return None;
            }
        };
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
                worker_epoch: 0,
                last_heartbeat_at: pid.map(|_| now),
                takeover_count: 0,
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
                last_tool_info: None,
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
                entry.last_heartbeat_at = Some(now);
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
        Some((log_path, existed))
    });
    if let Some((log_path, existed)) = result.as_ref() {
        linkscope::event_fields(
            "daemon.agent.started.result",
            [
                linkscope::TraceField::text("status", "ok"),
                linkscope::TraceField::count("existed", u64::from(*existed)),
                linkscope::TraceField::text("log_path", log_path.display().to_string()),
            ],
        );
    } else {
        linkscope::event_fields(
            "daemon.agent.started.result",
            [
                linkscope::TraceField::text("status", "state_error"),
                linkscope::TraceField::text("id", id),
            ],
        );
    }
    if let Some((log_path, false)) = result.as_ref() {
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
fn worker_epoch_matches(agent: &BackgroundAgentInfo, worker_epoch: u64) -> bool {
    if worker_epoch == 0 {
        return agent.worker_epoch == 0 || agent.launch_path.is_none();
    }
    agent.worker_epoch == worker_epoch
}

pub fn record_background_agent_log(id: &str, text: &str) {
    let _ = record_background_agent_log_at_epoch(id, 0, text);
}

pub fn record_background_agent_log_at_epoch(id: &str, worker_epoch: u64, text: &str) -> bool {
    let _linkscope_log = linkscope::phase("daemon.agent.log_chunk");
    linkscope::record_bytes(
        "daemon.agent.log_chunk.input",
        usize_to_u64_saturating(text.len()),
    );
    linkscope::detail_event_fields(
        "daemon.agent.log_chunk.start",
        [
            linkscope::TraceField::text("id", id.to_owned()),
            linkscope::TraceField::count("worker_epoch", worker_epoch),
            linkscope::TraceField::bytes("bytes", usize_to_u64_saturating(text.len())),
        ],
    );
    let paths = DaemonPaths::default_user();
    let log_path = with_state_lock(&paths, || {
        let mut state = match load_state_for_update(&paths) {
            Ok(s) => s,
            Err(e) => {
                tracing::error!(target: "jfc::daemon", error = %e, "refusing to overwrite corrupt daemon state in record_background_agent_log");
                return None;
            }
        };
        if let Some(agent) = state.background_agents.get(id) {
            return worker_epoch_matches(agent, worker_epoch).then(|| agent.log_path.clone());
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
                worker_epoch,
                last_heartbeat_at: Some(now),
                takeover_count: 0,
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
                last_tool_info: None,
            },
        );
        let _ = save_state(&paths, &state);
        Some(log_path)
    });
    // SSE text deltas arrive in arbitrary chunks ("Let me", " implement",
    // " the full SPIR-V lif", "ter with…"). Writing one `writeln!` per
    // chunk turned the rendered task view into a column of 1-3-word
    // fragments. Append raw so only the model's own `\n` bytes break lines.
    let ok = log_path.is_some();
    linkscope::detail_event_fields(
        "daemon.agent.log_chunk.result",
        [
            linkscope::TraceField::text("id", id.to_owned()),
            linkscope::TraceField::count("accepted", u64::from(ok)),
        ],
    );
    if let Some(log_path) = log_path {
        append_chunk_raw(&log_path, text);
        true
    } else {
        false
    }
}

pub fn record_background_agent_progress(
    id: &str,
    last_tool: Option<&str>,
    last_tool_info: Option<&str>,
    tool_use_count: Option<u32>,
    latest_input_tokens: Option<u64>,
    latest_cache_read_tokens: Option<u64>,
    latest_cache_write_tokens: Option<u64>,
    output_tokens_delta: Option<u64>,
) {
    let _ = record_background_agent_progress_at_epoch(
        id,
        0,
        last_tool,
        last_tool_info,
        tool_use_count,
        latest_input_tokens,
        latest_cache_read_tokens,
        latest_cache_write_tokens,
        output_tokens_delta,
    );
}

pub fn record_background_agent_progress_at_epoch(
    id: &str,
    worker_epoch: u64,
    last_tool: Option<&str>,
    last_tool_info: Option<&str>,
    tool_use_count: Option<u32>,
    latest_input_tokens: Option<u64>,
    latest_cache_read_tokens: Option<u64>,
    latest_cache_write_tokens: Option<u64>,
    output_tokens_delta: Option<u64>,
) -> bool {
    let paths = DaemonPaths::default_user();
    record_background_agent_progress_at_epoch_with_paths(
        &paths,
        id,
        worker_epoch,
        last_tool,
        last_tool_info,
        tool_use_count,
        latest_input_tokens,
        latest_cache_read_tokens,
        latest_cache_write_tokens,
        output_tokens_delta,
    )
    .unwrap_or(false)
}

pub fn record_background_agent_progress_at_epoch_with_paths(
    paths: &DaemonPaths,
    id: &str,
    worker_epoch: u64,
    last_tool: Option<&str>,
    last_tool_info: Option<&str>,
    tool_use_count: Option<u32>,
    latest_input_tokens: Option<u64>,
    latest_cache_read_tokens: Option<u64>,
    latest_cache_write_tokens: Option<u64>,
    output_tokens_delta: Option<u64>,
) -> std::io::Result<bool> {
    let _linkscope_progress = linkscope::phase("daemon.agent.progress");
    linkscope::event_fields(
        "daemon.agent.progress.start",
        [
            linkscope::TraceField::text("id", id.to_owned()),
            linkscope::TraceField::count("worker_epoch", worker_epoch),
            linkscope::TraceField::count("has_last_tool", u64::from(last_tool.is_some())),
            linkscope::TraceField::count("has_tool_info", u64::from(last_tool_info.is_some())),
            linkscope::TraceField::count(
                "has_output_delta",
                u64::from(output_tokens_delta.is_some()),
            ),
        ],
    );
    let log_path = with_state_lock(paths, || {
        let mut state = match load_state_for_update(paths) {
            Ok(s) => s,
            Err(e) => {
                tracing::error!(target: "jfc::daemon", error = %e, "refusing to overwrite corrupt daemon state in record_background_agent_progress");
                return None;
            }
        };
        let agent = state.background_agents.get_mut(id)?;
        if !worker_epoch_matches(agent, worker_epoch) {
            return None;
        }
        let now = SystemTime::now();
        agent.updated_at = now;
        agent.last_heartbeat_at = Some(now);
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
        if let Some(info) = last_tool_info {
            agent.last_tool_info = Some(info.to_owned());
        }
        let log_path = agent.log_path.clone();
        let _ = save_state(paths, &state);
        Some(log_path)
    });
    let ok = log_path.is_some();
    linkscope::event_fields(
        "daemon.agent.progress.result",
        [
            linkscope::TraceField::text("id", id.to_owned()),
            linkscope::TraceField::count("accepted", u64::from(ok)),
            linkscope::TraceField::count(
                "tool_use_count",
                tool_use_count.map(u64::from).unwrap_or(0),
            ),
            linkscope::TraceField::count("input_tokens", latest_input_tokens.unwrap_or(0)),
            linkscope::TraceField::count("output_delta", output_tokens_delta.unwrap_or(0)),
        ],
    );
    if let (Some(log_path), Some(tool)) = (log_path.as_ref(), last_tool_info.or(last_tool)) {
        append_log_line(log_path, &format!("[tool] {tool}"));
    }
    Ok(ok)
}

pub fn record_background_agent_finished(
    id: &str,
    status: BackgroundAgentStatus,
    summary_or_error: &str,
) {
    let _ = record_background_agent_finished_at_epoch(id, 0, status, summary_or_error);
}

pub fn record_background_agent_finished_at_epoch(
    id: &str,
    worker_epoch: u64,
    status: BackgroundAgentStatus,
    summary_or_error: &str,
) -> bool {
    let paths = DaemonPaths::default_user();
    record_background_agent_finished_at_epoch_with_paths(
        &paths,
        id,
        worker_epoch,
        status,
        summary_or_error,
    )
    .unwrap_or(false)
}

pub fn record_background_agent_finished_at_epoch_with_paths(
    paths: &DaemonPaths,
    id: &str,
    worker_epoch: u64,
    status: BackgroundAgentStatus,
    summary_or_error: &str,
) -> std::io::Result<bool> {
    let _linkscope_finished = linkscope::phase("daemon.agent.finished");
    linkscope::event_fields(
        "daemon.agent.finished.start",
        [
            linkscope::TraceField::text("id", id.to_owned()),
            linkscope::TraceField::count("worker_epoch", worker_epoch),
            linkscope::TraceField::text("status", format!("{status:?}")),
            linkscope::TraceField::bytes(
                "summary_or_error_bytes",
                usize_to_u64_saturating(summary_or_error.len()),
            ),
        ],
    );
    let log_path = with_state_lock(paths, || {
        let mut state = match load_state_for_update(paths) {
            Ok(s) => s,
            Err(e) => {
                tracing::error!(target: "jfc::daemon", error = %e, "refusing to overwrite corrupt daemon state in record_background_agent_finished");
                return None;
            }
        };
        let now = SystemTime::now();
        let agent = state.background_agents.get_mut(id)?;
        if !worker_epoch_matches(agent, worker_epoch) {
            return None;
        }
        agent.status = status;
        agent.updated_at = now;
        agent.last_heartbeat_at = Some(now);
        agent.completed_at = Some(now);
        match status {
            BackgroundAgentStatus::Completed => agent.summary = Some(summary_or_error.to_owned()),
            BackgroundAgentStatus::Failed | BackgroundAgentStatus::Cancelled => {
                agent.error = Some(summary_or_error.to_owned())
            }
            BackgroundAgentStatus::Running => {}
        }
        let log_path = agent.log_path.clone();
        let _ = save_state(paths, &state);
        Some(log_path)
    });
    let ok = log_path.is_some();
    linkscope::event_fields(
        "daemon.agent.finished.result",
        [
            linkscope::TraceField::text("id", id.to_owned()),
            linkscope::TraceField::text("status", format!("{status:?}")),
            linkscope::TraceField::count("accepted", u64::from(ok)),
        ],
    );
    if let Some(log_path) = log_path.as_ref() {
        append_log_line(
            log_path,
            &format!("[{:?}] {}", status, summary_or_error.replace('\n', " ")),
        );
    }
    Ok(ok)
}

pub fn record_background_agent_heartbeat(id: &str, worker_epoch: u64) -> bool {
    let paths = DaemonPaths::default_user();
    record_background_agent_heartbeat_at(&paths, id, worker_epoch, std::process::id())
        .unwrap_or(false)
}

pub fn record_background_agent_heartbeat_at(
    paths: &DaemonPaths,
    id: &str,
    worker_epoch: u64,
    pid: u32,
) -> std::io::Result<bool> {
    let _linkscope_heartbeat = linkscope::phase("daemon.agent.heartbeat");
    linkscope::detail_event_fields(
        "daemon.agent.heartbeat.start",
        [
            linkscope::TraceField::text("id", id.to_owned()),
            linkscope::TraceField::count("worker_epoch", worker_epoch),
            linkscope::TraceField::count("pid", u64::from(pid)),
        ],
    );
    let result = with_state_lock(paths, || -> std::io::Result<bool> {
        let mut state = load_state_for_update(paths)?;
        let Some(agent) = state.background_agents.get_mut(id) else {
            return Ok(false);
        };
        if !worker_epoch_matches(agent, worker_epoch) {
            return Ok(false);
        }
        let now = SystemTime::now();
        agent.pid = Some(pid);
        agent.status = BackgroundAgentStatus::Running;
        agent.updated_at = now;
        agent.last_heartbeat_at = Some(now);
        save_state(paths, &state)?;
        Ok(true)
    });
    linkscope::detail_event_fields(
        "daemon.agent.heartbeat.result",
        [
            linkscope::TraceField::text("id", id.to_owned()),
            linkscope::TraceField::count(
                "accepted",
                u64::from(result.as_ref().copied().unwrap_or(false)),
            ),
        ],
    );
    result
}

pub fn background_agent_cancel_requested(id: &str) -> bool {
    let paths = DaemonPaths::default_user();
    with_state_lock(&paths, || {
        load_state(&paths)
            .and_then(|state| state.background_agents.get(id).cloned())
            .map(|agent| agent.cancel_requested && !agent.status.is_terminal())
            .unwrap_or(false)
    })
}

pub fn request_background_agent_cancel(paths: &DaemonPaths, id: &str) -> std::io::Result<()> {
    let _linkscope_cancel = linkscope::phase("daemon.agent.cancel");
    linkscope::event_fields(
        "daemon.agent.cancel.start",
        [linkscope::TraceField::text("id", id.to_owned())],
    );
    let result = with_state_lock(paths, || -> std::io::Result<Option<PathBuf>> {
        let mut state = load_state_for_update(paths)?;
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
        linkscope::event_fields(
            "daemon.agent.cancel.result",
            [
                linkscope::TraceField::text("status", "requested"),
                linkscope::TraceField::text("log_path", log_path.display().to_string()),
            ],
        );
        append_log_line(&log_path, "[cancel-requested]");
    } else {
        linkscope::event_fields(
            "daemon.agent.cancel.result",
            [linkscope::TraceField::text("status", "terminal")],
        );
    }
    Ok(())
}

pub fn background_agents_string(paths: &DaemonPaths) -> String {
    let _linkscope_list = linkscope::phase("daemon.agent.list_string");
    let state = reconcile_background_agents(paths).unwrap_or_default();
    let mut agents: Vec<_> = state.background_agents.values().collect();
    agents.sort_by_key(|a| a.started_at);
    agents.reverse();
    linkscope::event_fields(
        "daemon.agent.list_string.state",
        [
            linkscope::TraceField::count("agents", usize_to_u64_saturating(agents.len())),
            linkscope::TraceField::count(
                "running",
                usize_to_u64_saturating(
                    agents
                        .iter()
                        .filter(|agent| agent.status == BackgroundAgentStatus::Running)
                        .count(),
                ),
            ),
        ],
    );
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
        if a.worker_epoch > 0 || a.takeover_count > 0 || a.last_heartbeat_at.is_some() {
            let heartbeat_age = a
                .last_heartbeat_at
                .and_then(|ts| SystemTime::now().duration_since(ts).ok())
                .map(|d| format!("{}s ago", d.as_secs()))
                .unwrap_or_else(|| "never".to_owned());
            s.push_str(&format!(
                "    worker: pid={} epoch={} takeovers={} heartbeat={}\n",
                a.pid
                    .map(|pid| pid.to_string())
                    .unwrap_or_else(|| "-".to_owned()),
                a.worker_epoch,
                a.takeover_count,
                heartbeat_age
            ));
        }
        s.push_str(&format!("    log: {}\n", a.log_path.display()));
    }
    s
}

pub fn background_agent_logs_string(paths: &DaemonPaths, id: &str, lines: usize) -> String {
    let _linkscope_logs = linkscope::phase("daemon.agent.logs_string");
    linkscope::event_fields(
        "daemon.agent.logs_string.start",
        [
            linkscope::TraceField::text("id", id.to_owned()),
            linkscope::TraceField::count("lines", usize_to_u64_saturating(lines)),
        ],
    );
    let state = reconcile_background_agents(paths).unwrap_or_default();
    let Some(agent) = state.background_agents.get(id) else {
        linkscope::event_fields(
            "daemon.agent.logs_string.result",
            [
                linkscope::TraceField::text("id", id.to_owned()),
                linkscope::TraceField::text("status", "missing"),
            ],
        );
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
    linkscope::event_fields(
        "daemon.agent.logs_string.result",
        [
            linkscope::TraceField::text("id", id.to_owned()),
            linkscope::TraceField::text("status", format!("{:?}", agent.status)),
            linkscope::TraceField::text("log_path", agent.log_path.display().to_string()),
        ],
    );
    s
}

pub fn background_agents_for_restore(
    paths: &DaemonPaths,
    parent_session_id: Option<&str>,
    limit: usize,
) -> Vec<BackgroundAgentInfo> {
    let _linkscope_restore = linkscope::phase("daemon.agent.restore_candidates");
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
    let restored = active
        .into_iter()
        .chain(terminal)
        .take(limit)
        .collect::<Vec<_>>();
    linkscope::event_fields(
        "daemon.agent.restore_candidates.result",
        [
            linkscope::TraceField::count("limit", usize_to_u64_saturating(limit)),
            linkscope::TraceField::count("returned", usize_to_u64_saturating(restored.len())),
            linkscope::TraceField::count("has_parent", u64::from(parent_session_id.is_some())),
        ],
    );
    restored
}

pub async fn wait_background_agent_cli(
    paths: &DaemonPaths,
    id: &str,
    timeout: Duration,
) -> std::io::Result<String> {
    let _linkscope_wait = linkscope::phase("daemon.agent.wait_cli");
    linkscope::event_fields(
        "daemon.agent.wait_cli.start",
        [
            linkscope::TraceField::text("id", id.to_owned()),
            linkscope::TraceField::count("timeout_ms", duration_millis_u64(timeout)),
        ],
    );
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
            linkscope::event_fields(
                "daemon.agent.wait_cli.result",
                [
                    linkscope::TraceField::text("id", id.to_owned()),
                    linkscope::TraceField::text("status", format!("{:?}", agent.status)),
                    linkscope::TraceField::count(
                        "elapsed_ms",
                        duration_millis_u64(started.elapsed()),
                    ),
                ],
            );
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
            linkscope::event_fields(
                "daemon.agent.wait_cli.result",
                [
                    linkscope::TraceField::text("id", id.to_owned()),
                    linkscope::TraceField::text("status", format!("{:?}", agent.status)),
                    linkscope::TraceField::text("outcome", "timeout"),
                    linkscope::TraceField::count(
                        "elapsed_ms",
                        duration_millis_u64(started.elapsed()),
                    ),
                ],
            );
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
    let _linkscope_attach = linkscope::phase("daemon.agent.attach_cli");
    linkscope::event_fields(
        "daemon.agent.attach_cli.start",
        [
            linkscope::TraceField::text("id", id.to_owned()),
            linkscope::TraceField::count("lines", usize_to_u64_saturating(lines)),
        ],
    );
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
    let stall_after = attach_stall_after();
    let mut last_progress = Instant::now();
    let mut stall_reported = false;
    if agent.status.is_terminal() {
        linkscope::event_fields(
            "daemon.agent.attach_cli.result",
            [
                linkscope::TraceField::text("id", id.to_owned()),
                linkscope::TraceField::text("status", format!("{:?}", agent.status)),
                linkscope::TraceField::text("outcome", "already_terminal"),
            ],
        );
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
                linkscope::event_fields(
                    "daemon.agent.attach_cli.chunk",
                    [
                        linkscope::TraceField::text("id", id.to_owned()),
                        linkscope::TraceField::bytes("bytes", usize_to_u64_saturating(buf.len())),
                        linkscope::TraceField::count("offset", offset),
                        linkscope::TraceField::count("len", len),
                    ],
                );
                print!("{buf}");
                std::io::stdout().flush()?;
                offset = len;
                last_progress = Instant::now();
                stall_reported = false;
            }
        }
        if agent.status.is_terminal() {
            linkscope::event_fields(
                "daemon.agent.attach_cli.result",
                [
                    linkscope::TraceField::text("id", id.to_owned()),
                    linkscope::TraceField::text("status", format!("{:?}", agent.status)),
                    linkscope::TraceField::text("outcome", "terminal"),
                ],
            );
            println!("[{:?}]", agent.status);
            return Ok(());
        }
        if let Some(stall_after) = stall_after
            && !stall_reported
            && last_progress.elapsed() >= stall_after
        {
            println!(
                "[attach-stall] no new log output for {}ms; worker is still {:?}",
                stall_after.as_millis(),
                agent.status
            );
            linkscope::event_fields(
                "daemon.agent.attach_cli.stall",
                [
                    linkscope::TraceField::text("id", id.to_owned()),
                    linkscope::TraceField::text("status", format!("{:?}", agent.status)),
                    linkscope::TraceField::count("stall_ms", duration_millis_u64(stall_after)),
                ],
            );
            std::io::stdout().flush()?;
            stall_reported = true;
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
}

fn attach_stall_after() -> Option<Duration> {
    std::env::var("JFC_DAEMON_ATTACH_STALL_MS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .filter(|ms| *ms > 0)
        .map(Duration::from_millis)
}

fn duration_millis_u64(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}

fn usize_to_u64_saturating(value: usize) -> u64 {
    u64::try_from(value).unwrap_or(u64::MAX)
}
