//! Daemon submodule tests, moved verbatim from the pre-split `daemon.rs`.

#![cfg(test)]

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use tempfile::TempDir;

use super::cron::{CronField, CronSchedule, parse_schedule, should_fire_cron};
use super::logs::{background_agent_launch_path, background_agent_log_path, read_last_lines};
use super::reconcile::reconcile_background_agents;
use super::registry::{
    background_agents_for_restore, background_agents_string, record_background_agent_started_at,
    request_background_agent_cancel,
};
use super::runtime::Daemon;
use super::state::{
    BackgroundAgentInfo, BackgroundAgentLaunch, BackgroundAgentStatus, DaemonPaths, DaemonState,
    SessionStatus, load_state, save_state,
};
use super::worker::{
    spawn_background_agent_worker_with_paths, validate_worker_spawn_inputs,
    worker_exe_workspace_candidates,
};

fn test_daemon() -> (Daemon, TempDir) {
    let tmp = TempDir::new().unwrap();
    let daemon = Daemon::new(tmp.path()).unwrap();
    (daemon, tmp)
}

fn test_task_input() -> crate::types::TaskInput {
    crate::types::TaskInput {
        description: "inspect".to_owned(),
        prompt: "inspect".to_owned(),
        subagent_type: Some("explore".to_owned()),
        category: None,
        run_in_background: true,
        model: None,
        name: None,
        team_name: None,
        mode: None,
        isolation: None,
        parent_task_id: None,
    }
}

fn test_launch(cwd: PathBuf) -> BackgroundAgentLaunch {
    BackgroundAgentLaunch {
        task_id: "task-worker".to_owned(),
        task_input: test_task_input(),
        parent_session_id: Some("ses-owner".to_owned()),
        model: crate::provider::ModelId::new("claude-sonnet-4-5"),
        provider_name: Some("anthropic".to_owned()),
        agent_def: None,
        cwd,
        worker_exe: Some(std::env::current_exe().unwrap()),
        active_team_name: None,
        created_at: SystemTime::now(),
    }
}

fn test_background_info(id: &str, parent_session_id: Option<&str>) -> BackgroundAgentInfo {
    let tmp_log = std::env::temp_dir().join(format!("{id}.log"));
    BackgroundAgentInfo {
        id: id.to_owned(),
        description: id.to_owned(),
        parent_session_id: parent_session_id.map(str::to_owned),
        status: BackgroundAgentStatus::Running,
        started_at: SystemTime::now(),
        updated_at: SystemTime::now(),
        completed_at: None,
        pid: Some(std::process::id()),
        model: None,
        worktree_path: None,
        log_path: tmp_log,
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
    }
}

#[test]
fn worker_spawn_validation_reports_missing_executable_robust() {
    let tmp = TempDir::new().unwrap();
    let missing_exe = tmp.path().join("missing-jfc");
    let err = validate_worker_spawn_inputs(&missing_exe, tmp.path()).unwrap_err();
    assert_eq!(err.kind(), std::io::ErrorKind::NotFound);
    assert!(
        err.to_string().contains("worker executable"),
        "unexpected error: {err}"
    );
    assert!(
        err.to_string().contains(missing_exe.to_str().unwrap()),
        "missing path not included: {err}"
    );
}

#[test]
fn worker_spawn_validation_reports_missing_cwd_robust() {
    let tmp = TempDir::new().unwrap();
    let missing_cwd = tmp.path().join("removed-worktree");
    let exe = std::env::current_exe().unwrap();
    let err = validate_worker_spawn_inputs(&exe, &missing_cwd).unwrap_err();
    assert_eq!(err.kind(), std::io::ErrorKind::NotFound);
    assert!(
        err.to_string().contains("worker cwd"),
        "unexpected error: {err}"
    );
    assert!(
        err.to_string().contains(missing_cwd.to_str().unwrap()),
        "missing cwd not included: {err}"
    );
}

#[test]
fn worker_exe_workspace_candidates_include_dev_and_release_bins_robust() {
    let candidates = worker_exe_workspace_candidates();
    assert!(
        candidates
            .iter()
            .any(|path| path.ends_with("target/release/jfc")),
        "release worker candidate missing: {candidates:?}"
    );
    assert!(
        candidates
            .iter()
            .any(|path| path.ends_with("target/debug/jfc")),
        "debug worker candidate missing: {candidates:?}"
    );
}

#[test]
fn background_agent_launch_deserializes_old_records_without_worker_exe_robust() {
    let json = serde_json::json!({
        "task_id": "task-old",
        "task_input": test_task_input(),
        "model": "claude-sonnet-4-5",
        "provider_name": "anthropic",
        "agent_def": null,
        "cwd": "/tmp",
        "active_team_name": null,
        "created_at": std::time::SystemTime::UNIX_EPOCH,
    });
    let launch: BackgroundAgentLaunch = serde_json::from_value(json).unwrap();
    assert!(launch.worker_exe.is_none());
    assert!(launch.parent_session_id.is_none());
}

#[test]
fn background_agents_for_restore_filters_by_parent_session_robust() {
    let tmp = TempDir::new().unwrap();
    let paths = DaemonPaths::new(tmp.path());
    paths.ensure_dirs().unwrap();
    let mut state = DaemonState::default();
    state.background_agents.insert(
        "owned".to_owned(),
        test_background_info("owned", Some("ses-a")),
    );
    state.background_agents.insert(
        "foreign".to_owned(),
        test_background_info("foreign", Some("ses-b")),
    );
    state
        .background_agents
        .insert("legacy".to_owned(), test_background_info("legacy", None));
    save_state(&paths, &state).unwrap();

    let restored = background_agents_for_restore(&paths, Some("ses-a"), 20);
    assert_eq!(restored.len(), 1);
    assert_eq!(restored[0].id, "owned");
    assert!(background_agents_for_restore(&paths, None, 20).is_empty());
}

#[test]
fn ui_metadata_refresh_does_not_clobber_detached_worker_pid_robust() {
    let tmp = TempDir::new().unwrap();
    let paths = DaemonPaths::new(tmp.path());
    paths.ensure_dirs().unwrap();
    let mut state = DaemonState::default();
    let mut agent = test_background_info("task-detached", Some("ses-a"));
    agent.pid = Some(4242);
    agent.launch_path = Some(background_agent_launch_path(&paths, "task-detached"));
    state
        .background_agents
        .insert("task-detached".to_owned(), agent);
    save_state(&paths, &state).unwrap();

    record_background_agent_started_at(
        &paths,
        "task-detached",
        "refreshed description",
        None,
        Some("claude-sonnet-4-5".to_owned()),
        None,
        Some(7777),
    );

    let state = load_state(&paths).unwrap();
    let agent = &state.background_agents["task-detached"];
    assert_eq!(agent.pid, Some(4242));
    assert_eq!(agent.description, "refreshed description");
}

#[test]
fn spawn_background_worker_missing_cwd_persists_failed_roster_robust() {
    let tmp = TempDir::new().unwrap();
    let paths = DaemonPaths::new(tmp.path());
    let missing_cwd = tmp.path().join("removed-worktree");
    let launch = test_launch(missing_cwd.clone());

    let err = spawn_background_agent_worker_with_paths(&paths, launch).unwrap_err();
    assert!(
        err.to_string().contains("worker cwd"),
        "unexpected error: {err}"
    );

    let state = load_state(&paths).unwrap();
    let agent = &state.background_agents["task-worker"];
    assert_eq!(agent.status, BackgroundAgentStatus::Failed);
    assert!(
        agent
            .error
            .as_deref()
            .unwrap_or_default()
            .contains(missing_cwd.to_str().unwrap()),
        "missing cwd not persisted in error: {:?}",
        agent.error
    );
    assert!(
        read_last_lines(&agent.log_path, 4)
            .iter()
            .any(|line| line.contains("[Failed] background worker cwd")),
        "failure log missing cwd error"
    );
}

// ─── schedule parsing (DO-178B _normal) ─────────────────────────────

#[test]
fn parse_schedule_crontab_normal() {
    let s = parse_schedule("* * * * *").unwrap();
    match s {
        CronSchedule::Crontab {
            minute,
            hour,
            day,
            month,
            weekday,
        } => {
            assert!(matches!(minute, CronField::Any));
            assert!(matches!(hour, CronField::Any));
            assert!(matches!(day, CronField::Any));
            assert!(matches!(month, CronField::Any));
            assert!(matches!(weekday, CronField::Any));
        }
        _ => panic!("expected Crontab"),
    }
}

#[test]
fn parse_schedule_daily_normal() {
    let s = parse_schedule("@daily").unwrap();
    match s {
        CronSchedule::Crontab { minute, hour, .. } => {
            assert_eq!(minute, CronField::Exact(0));
            assert_eq!(hour, CronField::Exact(0));
        }
        _ => panic!("expected Crontab"),
    }
}

#[test]
fn parse_schedule_hourly_normal() {
    let s = parse_schedule("@hourly").unwrap();
    match s {
        CronSchedule::Crontab { minute, hour, .. } => {
            assert_eq!(minute, CronField::Exact(0));
            assert_eq!(hour, CronField::Any);
        }
        _ => panic!("expected Crontab"),
    }
}

#[test]
fn parse_schedule_every_5m_normal() {
    let s = parse_schedule("@every 5m").unwrap();
    assert_eq!(
        s,
        CronSchedule::Every {
            period: Duration::from_secs(300)
        }
    );
}

#[test]
fn parse_schedule_every_complex_normal() {
    let s = parse_schedule("@every 1h30m").unwrap();
    assert_eq!(
        s,
        CronSchedule::Every {
            period: Duration::from_secs(5400)
        }
    );
}

#[test]
fn parse_schedule_step_normal() {
    let s = parse_schedule("*/15 * * * *").unwrap();
    match s {
        CronSchedule::Crontab { minute, .. } => {
            assert_eq!(minute, CronField::Step(15));
        }
        _ => panic!("expected Crontab"),
    }
}

// ─── schedule parsing (DO-178B _robust) ─────────────────────────────

#[test]
fn parse_schedule_empty_robust() {
    assert!(parse_schedule("").is_err());
    assert!(parse_schedule("   ").is_err());
}

#[test]
fn parse_schedule_short_robust() {
    assert!(parse_schedule("* * *").is_err());
}

#[test]
fn parse_schedule_garbage_field_robust() {
    assert!(parse_schedule("foo * * * *").is_err());
}

#[test]
fn parse_schedule_zero_step_robust() {
    assert!(parse_schedule("*/0 * * * *").is_err());
}

#[test]
fn parse_schedule_zero_every_robust() {
    assert!(parse_schedule("@every 0s").is_err());
}

#[test]
fn parse_schedule_unknown_alias_robust() {
    // @yearly isn't supported; should fail rather than silently misparse.
    assert!(parse_schedule("@yearly").is_err());
}

// ─── should_fire_cron boundary conditions (DO-178B _normal/_robust) ─

fn cron_job(schedule: CronSchedule, last_run: Option<SystemTime>) -> super::cron::CronJob {
    super::cron::CronJob {
        id: "cron-test".into(),
        schedule,
        description: "test".into(),
        command: "true".into(),
        enabled: true,
        last_run,
        created_at: SystemTime::now(),
    }
}

#[test]
fn should_fire_every_first_run_normal() {
    let job = cron_job(
        CronSchedule::Every {
            period: Duration::from_secs(60),
        },
        None,
    );
    assert!(should_fire_cron(&job, SystemTime::now()));
}

#[test]
fn should_fire_every_just_after_normal() {
    let now = SystemTime::now();
    let job = cron_job(
        CronSchedule::Every {
            period: Duration::from_secs(60),
        },
        Some(now - Duration::from_secs(61)),
    );
    assert!(should_fire_cron(&job, now));
}

#[test]
fn should_fire_every_just_before_robust() {
    let now = SystemTime::now();
    let job = cron_job(
        CronSchedule::Every {
            period: Duration::from_secs(60),
        },
        Some(now - Duration::from_secs(59)),
    );
    assert!(!should_fire_cron(&job, now));
}

#[test]
fn should_fire_every_exactly_at_boundary_normal() {
    let now = SystemTime::now();
    let job = cron_job(
        CronSchedule::Every {
            period: Duration::from_secs(60),
        },
        Some(now - Duration::from_secs(60)),
    );
    // At exactly the boundary the contract is `>=`, so it fires.
    assert!(should_fire_cron(&job, now));
}

#[test]
fn should_fire_crontab_minute_match_normal() {
    // Build a `*/1 * * * *` (every minute) schedule and a "now"
    // value; the job should fire on the first poll.
    let s = parse_schedule("*/1 * * * *").unwrap();
    let job = cron_job(s, None);
    assert!(should_fire_cron(&job, SystemTime::now()));
}

#[test]
fn should_fire_crontab_no_double_fire_within_minute_robust() {
    let s = parse_schedule("* * * * *").unwrap();
    let now = SystemTime::now();
    let job = cron_job(s, Some(now));
    // Same `now` ⇒ same minute ⇒ must not fire twice.
    assert!(!should_fire_cron(&job, now));
}

// ─── state save/load round-trip (DO-178B _normal) ───────────────────

#[test]
fn state_roundtrip_normal() {
    let tmp = TempDir::new().unwrap();
    {
        let mut d = Daemon::new(tmp.path()).unwrap();
        d.add_cron_job(
            parse_schedule("@daily").unwrap(),
            "nightly housekeeping",
            "echo hi",
        );
        d.schedule_wakeup(Duration::from_secs(60), "ping me", "test");
    }
    let d2 = Daemon::new(tmp.path()).unwrap();
    assert_eq!(d2.state.cron_jobs.len(), 1);
    assert_eq!(d2.state.cron_jobs[0].command, "echo hi");
    assert_eq!(d2.state.wakeups.len(), 1);
    assert_eq!(d2.state.wakeups[0].reason, "test");
}

#[test]
fn state_roundtrip_empty_state_robust() {
    let tmp = TempDir::new().unwrap();
    // Reading from a fresh dir should yield default state.
    let d = Daemon::new(tmp.path()).unwrap();
    assert!(d.state.cron_jobs.is_empty());
    assert!(d.state.wakeups.is_empty());
}

#[test]
fn state_roundtrip_corrupt_file_robust() {
    let tmp = TempDir::new().unwrap();
    let paths = DaemonPaths::new(tmp.path());
    paths.ensure_dirs().unwrap();
    std::fs::write(&paths.state_file, "not-json {{ ").unwrap();
    // Should not panic — `Daemon::new` falls back to default state.
    let d = Daemon::new(tmp.path()).unwrap();
    assert!(d.state.cron_jobs.is_empty());
}

#[test]
fn background_agent_state_roundtrip_normal() {
    let tmp = TempDir::new().unwrap();
    let paths = DaemonPaths::new(tmp.path());
    paths.ensure_dirs().unwrap();
    let log_path = background_agent_log_path(&paths, "task-1");
    let mut state = DaemonState::default();
    state.background_agents.insert(
        "task-1".to_owned(),
        BackgroundAgentInfo {
            id: "task-1".to_owned(),
            description: "inspect repo".to_owned(),
            parent_session_id: Some("ses-owner".to_owned()),
            status: BackgroundAgentStatus::Running,
            started_at: SystemTime::now(),
            updated_at: SystemTime::now(),
            completed_at: None,
            pid: Some(std::process::id()),
            model: Some("claude-sonnet-4-5".to_owned()),
            worktree_path: Some(tmp.path().join("wt")),
            log_path: log_path.clone(),
            launch_path: None,
            cancel_requested: false,
            respawn_count: 0,
            summary: None,
            error: None,
            tool_use_count: 2,
            latest_input_tokens: 100,
            latest_cache_read_tokens: 0,
            latest_cache_write_tokens: 0,
            cumulative_output_tokens: 25,
            last_tool: None,
        },
    );
    save_state(&paths, &state).unwrap();

    let out = background_agents_string(&paths);
    assert!(out.contains("task-1"));
    assert!(out.contains("tokens=125"));
    assert!(out.contains("worktree:"));
}

#[test]
fn background_agent_cancel_request_normal() {
    let tmp = TempDir::new().unwrap();
    let paths = DaemonPaths::new(tmp.path());
    paths.ensure_dirs().unwrap();
    let log_path = background_agent_log_path(&paths, "task-cancel");
    let mut state = DaemonState::default();
    state.background_agents.insert(
        "task-cancel".to_owned(),
        BackgroundAgentInfo {
            id: "task-cancel".to_owned(),
            description: "long run".to_owned(),
            parent_session_id: Some("ses-owner".to_owned()),
            status: BackgroundAgentStatus::Running,
            started_at: SystemTime::now(),
            updated_at: SystemTime::now(),
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
    save_state(&paths, &state).unwrap();

    request_background_agent_cancel(&paths, "task-cancel").unwrap();
    let state = load_state(&paths).unwrap();
    assert!(state.background_agents["task-cancel"].cancel_requested);
    assert!(read_last_lines(&log_path, 1)[0].contains("[cancel-requested]"));
}

#[test]
fn background_agent_stale_owner_marked_failed_robust() {
    let tmp = TempDir::new().unwrap();
    let paths = DaemonPaths::new(tmp.path());
    paths.ensure_dirs().unwrap();
    let log_path = background_agent_log_path(&paths, "task-stale");
    let mut state = DaemonState::default();
    state.background_agents.insert(
        "task-stale".to_owned(),
        BackgroundAgentInfo {
            id: "task-stale".to_owned(),
            description: "lost run".to_owned(),
            parent_session_id: Some("ses-owner".to_owned()),
            status: BackgroundAgentStatus::Running,
            started_at: SystemTime::now(),
            updated_at: SystemTime::now(),
            completed_at: None,
            pid: Some(0),
            model: None,
            worktree_path: None,
            log_path: log_path.clone(),
            launch_path: None,
            cancel_requested: true,
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
    save_state(&paths, &state).unwrap();

    let state = reconcile_background_agents(&paths).unwrap();
    let agent = &state.background_agents["task-stale"];
    assert_eq!(agent.status, BackgroundAgentStatus::Failed);
    assert!(!agent.cancel_requested);
    assert!(agent.error.as_deref().unwrap_or_default().contains("stale"));
    assert!(read_last_lines(&log_path, 1)[0].contains("[Failed] stale"));
}

// ─── ScheduleWakeup persistence (DO-178B _normal/_robust) ───────────

#[test]
fn schedule_wakeup_persistence_normal() {
    let tmp = TempDir::new().unwrap();
    let id;
    {
        let mut d = Daemon::new(tmp.path()).unwrap();
        id = d.schedule_wakeup(
            Duration::from_secs(120),
            "check the deploy",
            "user said `/loop check`",
        );
    }
    let d2 = Daemon::new(tmp.path()).unwrap();
    assert_eq!(d2.state.wakeups.len(), 1);
    assert_eq!(d2.state.wakeups[0].id, id);
    assert_eq!(d2.state.wakeups[0].prompt, "check the deploy");
}

#[test]
fn schedule_wakeup_drain_due_normal() {
    let (mut d, _tmp) = test_daemon();
    d.schedule_wakeup(Duration::from_secs(0), "fire me", "now");
    d.schedule_wakeup(Duration::from_secs(3600), "later", "much later");
    let due = d.drain_due_wakeups(SystemTime::now() + Duration::from_secs(1));
    assert_eq!(due.len(), 1);
    assert_eq!(due[0].prompt, "fire me");
    assert_eq!(d.state.wakeups.len(), 1, "future wakeup must remain");
    assert_eq!(d.state.fired_wakeups.len(), 1);
}

#[test]
fn schedule_wakeup_drain_due_replays_after_restart_robust() {
    let tmp = TempDir::new().unwrap();
    {
        let mut d = Daemon::new(tmp.path()).unwrap();
        d.schedule_wakeup(Duration::from_secs(0), "p1", "r1");
    }
    let mut d2 = Daemon::new(tmp.path()).unwrap();
    let due = d2.drain_due_wakeups(SystemTime::now() + Duration::from_secs(1));
    assert_eq!(due.len(), 1);
    assert_eq!(due[0].reason, "r1");
}

#[test]
fn schedule_wakeup_no_due_returns_empty_robust() {
    let (mut d, _tmp) = test_daemon();
    d.schedule_wakeup(Duration::from_secs(3600), "later", "much later");
    let due = d.drain_due_wakeups(SystemTime::now());
    assert!(due.is_empty());
    assert_eq!(d.state.wakeups.len(), 1);
}

// ─── existing surface-area tests ────────────────────────────────────

#[test]
fn add_remove_cron_normal() {
    let (mut daemon, _tmp) = test_daemon();
    let id = daemon.add_cron_job(
        CronSchedule::Every {
            period: Duration::from_secs(1800),
        },
        "periodic check",
        "true",
    );
    assert_eq!(daemon.state.cron_jobs.len(), 1);
    assert!(daemon.remove_cron_job(&id));
    assert_eq!(daemon.state.cron_jobs.len(), 0);
}

#[test]
fn remove_unknown_cron_robust() {
    let (mut daemon, _tmp) = test_daemon();
    assert!(!daemon.remove_cron_job("no-such-id"));
}

#[test]
fn fire_cron_advances_last_run_normal() {
    let (mut daemon, _tmp) = test_daemon();
    let id = daemon.add_cron_job(
        CronSchedule::Every {
            period: Duration::from_secs(60),
        },
        "x",
        "true",
    );
    let now = SystemTime::now();
    let snapshot = daemon.fire_cron(&id, now).unwrap();
    assert_eq!(snapshot.last_run, Some(now));
    // Re-run should not fire — period not elapsed yet.
    let fired = daemon.tick_cron(now);
    assert!(fired.is_empty());
}

#[test]
fn cleanup_old_sessions_normal() {
    let (mut daemon, _tmp) = test_daemon();
    let id = daemon.start_session("old", None, Path::new("/tmp"));
    daemon.update_session_status(&id, SessionStatus::Completed);
    daemon.cleanup_old_sessions(Duration::from_secs(0));
    assert!(daemon.state.sessions.is_empty());
}

#[test]
fn paths_default_user_uses_jfc_subdir_normal() {
    let p = DaemonPaths::default_user();
    assert!(p.state_file.ends_with("daemon-state.json"));
    assert!(p.pid_file.ends_with("daemon.pid"));
}
