//! Core types for the swarm / team orchestration system.

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Instant;

use serde::{Deserialize, Serialize};
use tokio::sync::watch;

use crate::app::PermissionMode;

// ─── Team File ───────────────────────────────────────────────────────────────

/// The team configuration file stored at `~/.claude/teams/{name}/config.json`.
/// Contains the roster of all team members and metadata about the team.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TeamFile {
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    pub created_at: u64,
    pub lead_agent_id: String,
    #[serde(default)]
    pub lead_session_id: Option<String>,
    pub members: Vec<TeamMember>,
}

/// One member in the team roster.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TeamMember {
    pub agent_id: String,
    pub name: String,
    #[serde(default)]
    pub agent_type: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub color: Option<String>,
    #[serde(default)]
    pub plan_mode_required: Option<bool>,
    pub joined_at: u64,
    #[serde(default)]
    pub cwd: Option<String>,
    #[serde(default)]
    pub worktree_path: Option<String>,
    #[serde(default)]
    pub backend_type: Option<BackendType>,
    #[serde(default)]
    pub is_active: Option<bool>,
    #[serde(default)]
    pub mode: Option<String>,
}

/// Backend type for how a teammate is executed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum BackendType {
    InProcess,
    Tmux,
    Iterm,
}

// ─── Teammate Identity ───────────────────────────────────────────────────────

/// Identity information for a spawned teammate. Passed to the runner and used
/// for message routing, task ownership, and display.
#[derive(Debug, Clone)]
pub struct TeammateIdentity {
    pub agent_id: String,
    pub agent_name: String,
    pub team_name: String,
    pub color: Option<String>,
    pub plan_mode_required: bool,
    pub parent_session_id: String,
}

// ─── Mailbox Messages ────────────────────────────────────────────────────────

/// A single message in a teammate's inbox file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MailboxMessage {
    pub from: String,
    pub text: String,
    pub timestamp: String,
    #[serde(default)]
    pub color: Option<String>,
    #[serde(default)]
    pub summary: Option<String>,
    #[serde(default)]
    pub read: bool,
}

/// Idle notification sent by a teammate when it finishes a turn and goes idle.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IdleNotification {
    #[serde(rename = "type")]
    pub msg_type: String, // always "idle_notification"
    pub from: String,
    pub timestamp: String,
    #[serde(default)]
    pub idle_reason: Option<String>,
    #[serde(default)]
    pub summary: Option<String>,
    #[serde(default)]
    pub completed_task_id: Option<String>,
    #[serde(default)]
    pub completed_status: Option<String>,
    #[serde(default)]
    pub failure_reason: Option<String>,
}

/// Shutdown request message (JSON structured message).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ShutdownRequest {
    #[serde(rename = "type")]
    pub msg_type: String, // "shutdown_request"
    pub request_id: String,
    pub from: String,
    #[serde(default)]
    pub reason: Option<String>,
}

/// Shutdown response message.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ShutdownResponse {
    #[serde(rename = "type")]
    pub msg_type: String, // "shutdown_response"
    pub request_id: String,
    pub from: String,
    pub approve: bool,
    #[serde(default)]
    pub reason: Option<String>,
}

// ─── Permission Sync ─────────────────────────────────────────────────────────

/// A permission request from a worker to the team leader.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SwarmPermissionRequest {
    pub id: String,
    pub worker_id: String,
    pub worker_name: String,
    #[serde(default)]
    pub worker_color: Option<String>,
    pub team_name: String,
    pub tool_name: String,
    pub tool_use_id: String,
    pub description: String,
    pub input: serde_json::Value,
    #[serde(default)]
    pub permission_suggestions: Vec<serde_json::Value>,
    pub status: PermissionRequestStatus,
    #[serde(default)]
    pub resolved_by: Option<String>,
    #[serde(default)]
    pub resolved_at: Option<u64>,
    #[serde(default)]
    pub feedback: Option<String>,
    #[serde(default)]
    pub updated_input: Option<serde_json::Value>,
    #[serde(default)]
    pub permission_updates: Vec<serde_json::Value>,
    pub created_at: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PermissionRequestStatus {
    Pending,
    Approved,
    Rejected,
}

/// Resolution data returned when leader/worker resolves a permission request.
#[derive(Debug, Clone)]
pub struct PermissionResolution {
    pub decision: PermissionDecision,
    pub resolved_by: String,
    pub feedback: Option<String>,
    pub updated_input: Option<serde_json::Value>,
    pub permission_updates: Vec<serde_json::Value>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PermissionDecision {
    Approved,
    Rejected,
}

// ─── In-Process Teammate Task State ──────────────────────────────────────────

/// Status of an in-process teammate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TeammateStatus {
    Running,
    Idle,
    Completed,
    Failed,
    Killed,
}

/// Progress tracking for a running teammate.
#[derive(Debug, Clone, Default)]
pub struct TeammateProgress {
    pub token_count: u64,
    pub tool_use_count: u64,
    pub recent_activities: Vec<TeammateActivity>,
}

/// A single tool activity entry for the progress display.
#[derive(Debug, Clone)]
pub struct TeammateActivity {
    pub tool_name: String,
    pub input_summary: String,
}

/// Full state for an in-process teammate task. This is the Rust equivalent
/// of v126's `InProcessTeammateTaskState`.
#[derive(Debug)]
pub struct InProcessTeammateState {
    pub identity: TeammateIdentity,
    pub task_id: String,
    pub status: TeammateStatus,
    pub prompt: String,
    pub description: String,
    pub model: Option<String>,
    pub progress: TeammateProgress,
    pub is_idle: bool,
    pub shutdown_requested: bool,
    pub permission_mode: PermissionMode,
    pub started_at: Instant,
    pub ended_at: Option<Instant>,
    pub error: Option<String>,
    /// Pending messages from the user/leader injected while the teammate is running.
    pub pending_user_messages: Vec<String>,
    /// Abort handle to kill this teammate.
    pub abort_tx: watch::Sender<bool>,
}

// ─── Team Context (in-memory state for the leader) ───────────────────────────

/// In-memory team context maintained by the team leader. Tracks the current
/// team's state including all spawned teammates.
#[derive(Debug, Clone, Default)]
pub struct TeamContext {
    pub team_name: Option<String>,
    pub team_file_path: Option<PathBuf>,
    pub lead_agent_id: Option<String>,
    pub teammates: HashMap<String, TeammateInfo>,
}

impl TeamContext {
    pub fn is_active(&self) -> bool {
        self.team_name.is_some()
    }

    pub fn teammate_names(&self) -> Vec<&str> {
        self.teammates.values().map(|t| t.name.as_str()).collect()
    }
}

/// Summary info about a spawned teammate (kept in the leader's TeamContext).
///
/// `abort_tx` holds the `watch::Sender<bool>` returned by
/// `swarm::runner::start_teammate`. The receiver inside the teammate's
/// run loop short-circuits to `Aborted` as soon as the sender is
/// dropped (because `watch::Receiver::changed()` immediately resolves
/// `Err(RecvError)` when there are no senders left). Storing it here
/// keeps the channel alive for the teammate's lifetime — without this,
/// the previous spawn site at `stream.rs:1962` dropped the sender on
/// the next line and every teammate was marked "Done" before doing any
/// real work.
#[derive(Debug, Clone)]
pub struct TeammateInfo {
    pub name: String,
    pub agent_type: Option<String>,
    pub color: Option<String>,
    pub cwd: String,
    pub spawned_at: Instant,
    pub backend: BackendType,
    /// Abort handle. Only `Some` when the runtime owns a live in-process
    /// teammate; daemon-backed entries are `None`. Cloning a watch sender
    /// is cheap and keeps the channel alive.
    pub abort_tx: Option<watch::Sender<bool>>,
}

// ─── Spawn Parameters ────────────────────────────────────────────────────────

/// Parameters for spawning a new teammate.
#[derive(Debug, Clone)]
pub struct SpawnTeammateParams {
    pub name: String,
    pub team_name: String,
    pub prompt: String,
    pub description: String,
    pub agent_type: Option<String>,
    pub model: Option<String>,
    pub plan_mode_required: bool,
    pub color: Option<String>,
}

/// Result of a successful teammate spawn.
#[derive(Debug, Clone)]
pub struct SpawnResult {
    pub agent_id: String,
    pub task_id: String,
    pub name: String,
    pub team_name: String,
    pub color: Option<String>,
}

// ─── Message Formatting ──────────────────────────────────────────────────────

/// Format a message as a `<teammate-message>` XML wrapper for delivery to the model.
///
/// ```xml
/// <teammate-message teammate_id="researcher" color="#ff0000" summary="task done">
/// message text here
/// </teammate-message>
/// ```
pub fn format_teammate_message(
    from: &str,
    text: &str,
    color: Option<&str>,
    summary: Option<&str>,
) -> String {
    let color_attr = color.map(|c| format!(" color=\"{c}\"")).unwrap_or_default();
    let summary_attr = summary
        .map(|s| format!(" summary=\"{s}\""))
        .unwrap_or_default();
    format!(
        "<{tag} teammate_id=\"{from}\"{color_attr}{summary_attr}>\n\
         {text}\n\
         </{tag}>",
        tag = super::TEAMMATE_MESSAGE_TAG
    )
}

/// Generate a deterministic agent ID from name and team name.
/// Format: `{name}@{team_name}`
pub fn make_agent_id(name: &str, team_name: &str) -> String {
    format!("{name}@{team_name}")
}

/// Sanitize a name for use in file paths and agent IDs.
pub fn sanitize_name(name: &str) -> String {
    name.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '-'
            }
        })
        .collect::<String>()
        .to_lowercase()
}

// ─── Color Utilities ─────────────────────────────────────────────────────────

/// Parse a hex color string (e.g., "#4FC3F7") into a ratatui `Color::Rgb`.
/// Returns `Color::White` if parsing fails.
pub fn hex_to_color(hex: &str) -> ratatui::style::Color {
    let hex = hex.trim_start_matches('#');
    if hex.len() != 6 {
        return ratatui::style::Color::White;
    }
    let r = u8::from_str_radix(&hex[0..2], 16).unwrap_or(255);
    let g = u8::from_str_radix(&hex[2..4], 16).unwrap_or(255);
    let b = u8::from_str_radix(&hex[4..6], 16).unwrap_or(255);
    ratatui::style::Color::Rgb(r, g, b)
}

/// Get a ratatui Color for a teammate, falling back to White if no color set.
pub fn teammate_color(color: Option<&str>) -> ratatui::style::Color {
    match color {
        Some(hex) => hex_to_color(hex),
        None => ratatui::style::Color::White,
    }
}

// ─── Plan Approval ───────────────────────────────────────────────────────────

/// Plan approval request sent by a teammate in plan mode.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanApprovalRequest {
    #[serde(rename = "type")]
    pub msg_type: String, // "plan_approval_request"
    pub request_id: String,
    pub from: String,
    pub plan: String,
    pub file_path: Option<String>,
}

/// Plan approval response from the leader.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanApprovalResponse {
    #[serde(rename = "type")]
    pub msg_type: String, // "plan_approval_response"
    pub request_id: String,
    pub approved: bool,
    #[serde(default)]
    pub feedback: Option<String>,
    pub timestamp: String,
    /// Permission mode to grant if approved.
    #[serde(default)]
    pub permission_mode: Option<String>,
}

impl PlanApprovalRequest {
    pub fn new(from: &str, plan: &str) -> Self {
        Self {
            msg_type: "plan_approval_request".to_owned(),
            request_id: format!("plan-{}", uuid::Uuid::new_v4()),
            from: from.to_owned(),
            plan: plan.to_owned(),
            file_path: None,
        }
    }
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::style::Color;

    #[test]
    fn make_agent_id_combines_name_and_team_normal() {
        assert_eq!(make_agent_id("alice", "alpha"), "alice@alpha");
    }

    #[test]
    fn sanitize_name_keeps_alphanumeric_and_dashes_normal() {
        assert_eq!(sanitize_name("alice-bob_42"), "alice-bob_42");
    }

    #[test]
    fn sanitize_name_lowercases_and_replaces_invalid_robust() {
        // Spaces, slashes, and unicode are folded to dashes; ASCII letters
        // are lowercased. Names land in file paths so we must keep them
        // path-safe.
        assert_eq!(sanitize_name("Alice Bob"), "alice-bob");
        assert_eq!(sanitize_name("a/b"), "a-b");
        assert_eq!(sanitize_name("FOO!"), "foo-");
        assert_eq!(sanitize_name(""), "");
        assert_eq!(sanitize_name("café"), "caf-");
    }

    #[test]
    fn format_teammate_message_no_color_no_summary_normal() {
        let formatted = format_teammate_message("alice", "hello", None, None);
        assert!(formatted.contains("teammate_id=\"alice\""));
        assert!(formatted.contains("hello"));
        assert!(!formatted.contains("color="));
        assert!(!formatted.contains("summary="));
    }

    #[test]
    fn format_teammate_message_with_color_and_summary_normal() {
        let formatted = format_teammate_message("bob", "report", Some("#123abc"), Some("done"));
        assert!(formatted.contains("teammate_id=\"bob\""));
        assert!(formatted.contains("color=\"#123abc\""));
        assert!(formatted.contains("summary=\"done\""));
        assert!(formatted.contains("report"));
    }

    #[test]
    fn hex_to_color_parses_six_digit_hex_normal() {
        assert_eq!(hex_to_color("#FF0000"), Color::Rgb(255, 0, 0));
        assert_eq!(hex_to_color("00FF00"), Color::Rgb(0, 255, 0));
        assert_eq!(hex_to_color("#0000ff"), Color::Rgb(0, 0, 255));
    }

    #[test]
    fn hex_to_color_returns_white_on_bad_input_robust() {
        // Wrong length / not hex → fall back to White instead of panicking.
        assert_eq!(hex_to_color("abc"), Color::White);
        assert_eq!(hex_to_color("#1234567"), Color::White);
        assert_eq!(hex_to_color(""), Color::White);
    }

    #[test]
    fn teammate_color_falls_back_to_white_normal() {
        assert_eq!(teammate_color(None), Color::White);
        assert_eq!(teammate_color(Some("#FFFFFF")), Color::Rgb(255, 255, 255));
    }

    #[test]
    fn team_context_is_active_only_when_team_set_normal() {
        let mut ctx = TeamContext::default();
        assert!(!ctx.is_active());
        ctx.team_name = Some("alpha".into());
        assert!(ctx.is_active());
    }

    #[test]
    fn team_context_teammate_names_returns_empty_for_default_normal() {
        let ctx = TeamContext::default();
        assert!(ctx.teammate_names().is_empty());
    }

    #[test]
    fn plan_approval_request_new_generates_unique_id_normal() {
        let r1 = PlanApprovalRequest::new("alice", "do x");
        let r2 = PlanApprovalRequest::new("alice", "do x");
        assert_eq!(r1.from, "alice");
        assert_eq!(r1.plan, "do x");
        assert_eq!(r1.msg_type, "plan_approval_request");
        assert!(r1.request_id.starts_with("plan-"));
        assert_ne!(r1.request_id, r2.request_id, "uuids must differ");
    }

    #[test]
    fn backend_type_serde_uses_kebab_case_normal() {
        let json = serde_json::to_string(&BackendType::InProcess).unwrap();
        assert_eq!(json, "\"in-process\"");
        let parsed: BackendType = serde_json::from_str("\"tmux\"").unwrap();
        assert_eq!(parsed, BackendType::Tmux);
    }

    #[test]
    fn permission_request_status_serde_lowercase_normal() {
        let s = serde_json::to_string(&PermissionRequestStatus::Pending).unwrap();
        assert_eq!(s, "\"pending\"");
        let parsed: PermissionRequestStatus = serde_json::from_str("\"approved\"").unwrap();
        assert_eq!(parsed, PermissionRequestStatus::Approved);
    }

    #[test]
    fn team_file_round_trips_through_json_normal() {
        let now = 1234u64;
        let tf = TeamFile {
            name: "alpha".into(),
            description: Some("test".into()),
            created_at: now,
            lead_agent_id: "lead@alpha".into(),
            lead_session_id: None,
            members: vec![TeamMember {
                agent_id: "lead@alpha".into(),
                name: "team-lead".into(),
                agent_type: Some("team-lead".into()),
                model: None,
                color: None,
                plan_mode_required: None,
                joined_at: now,
                cwd: Some("/tmp".into()),
                worktree_path: None,
                backend_type: Some(BackendType::InProcess),
                is_active: Some(true),
                mode: None,
            }],
        };
        let json = serde_json::to_string(&tf).unwrap();
        let parsed: TeamFile = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.name, "alpha");
        assert_eq!(parsed.members.len(), 1);
        assert_eq!(parsed.members[0].name, "team-lead");
        assert_eq!(parsed.members[0].backend_type, Some(BackendType::InProcess));
    }

    #[test]
    fn mailbox_message_defaults_normal() {
        // Required fields only — `read`, `color`, `summary` default to false/None.
        let json = r#"{"from":"x","text":"y","timestamp":"t"}"#;
        let parsed: MailboxMessage = serde_json::from_str(json).unwrap();
        assert!(!parsed.read);
        assert!(parsed.color.is_none());
        assert!(parsed.summary.is_none());
    }

    #[test]
    fn idle_notification_serde_uses_camel_case_normal() {
        let n = IdleNotification {
            msg_type: "idle_notification".into(),
            from: "alice".into(),
            timestamp: "t".into(),
            idle_reason: Some("done".into()),
            summary: None,
            completed_task_id: Some("task-1".into()),
            completed_status: Some("ok".into()),
            failure_reason: None,
        };
        let json = serde_json::to_value(&n).unwrap();
        assert_eq!(json["type"], "idle_notification");
        assert_eq!(json["idleReason"], "done");
        assert_eq!(json["completedTaskId"], "task-1");
    }
}
