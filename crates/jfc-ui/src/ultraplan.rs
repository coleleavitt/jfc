//! Ultraplan — remote planning sessions with plan teleportation.
//!
//! Mirrors Claude Code v2.1.147+'s `tengu_ultraplan_config` feature:
//! - Spawns a separate planning session (potentially remote)
//! - Planning agent has read-only tools (Read, Glob, Grep)
//! - Calls ExitPlanMode when done; plan teleports back to local
//! - Configurable timeout (default 90 minutes)
//! - Can be rejected with feedback → revises

use std::sync::Mutex;
use std::time::Duration;

/// In-process ultraplan registry — tracks active planning sessions
/// so callers can query status, cancel, or fetch the resulting plan.
static ACTIVE_SESSIONS: Mutex<Vec<UltraplanSession>> = Mutex::new(Vec::new());

/// Snapshot of an active ultraplan session.
#[derive(Debug, Clone)]
pub struct UltraplanSession {
    pub id: String,
    pub prompt: String,
    pub phase: UltraplanPhase,
    pub started_at: std::time::SystemTime,
    pub plan: Option<String>,
}

/// Register a new in-flight ultraplan session.
pub fn register_session(prompt: String) -> String {
    let id = format!("ult-{}", uuid_short());
    let session = UltraplanSession {
        id: id.clone(),
        prompt,
        phase: UltraplanPhase::Exploring,
        started_at: std::time::SystemTime::now(),
        plan: None,
    };
    if let Ok(mut g) = ACTIVE_SESSIONS.lock() {
        g.push(session);
    }
    id
}

/// Mark a session's plan as ready (called when subagent invokes ExitPlanMode).
pub fn complete_session(id: &str, plan: String) -> bool {
    if let Ok(mut g) = ACTIVE_SESSIONS.lock()
        && let Some(s) = g.iter_mut().find(|s| s.id == id)
    {
        s.phase = UltraplanPhase::PlanReady;
        s.plan = Some(plan);
        return true;
    }
    false
}

/// Teleport a session — its plan becomes available to the parent. Returns the plan.
pub fn teleport(id: &str) -> Option<String> {
    if let Ok(mut g) = ACTIVE_SESSIONS.lock()
        && let Some(idx) = g.iter().position(|s| s.id == id)
    {
        let mut s = g.remove(idx);
        s.phase = UltraplanPhase::Teleported;
        return s.plan;
    }
    None
}

/// List active sessions for status display.
pub fn list_sessions() -> Vec<UltraplanSession> {
    ACTIVE_SESSIONS
        .lock()
        .ok()
        .map(|g| g.clone())
        .unwrap_or_default()
}

fn uuid_short() -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    format!("{:x}", now & 0xffffff)
}

/// Default timeout for ultraplan sessions.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(5400); // 90 minutes

/// Sentinel embedded in rejection feedback to signal plan teleportation.
pub const TELEPORT_SENTINEL: &str = "__ULTRAPLAN_TELEPORT_LOCAL__";

/// Configuration for an ultraplan session.
#[derive(Debug, Clone)]
pub struct UltraplanConfig {
    pub timeout: Duration,
    pub remote: bool,
    pub prompt_identifier: String,
}

impl Default for UltraplanConfig {
    fn default() -> Self {
        Self {
            timeout: DEFAULT_TIMEOUT,
            remote: false,
            prompt_identifier: "ultraplan".into(),
        }
    }
}

/// Status of an ultraplan session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UltraplanPhase {
    /// Agent is exploring the codebase.
    Exploring,
    /// Agent has called ExitPlanMode — plan ready for review.
    PlanReady,
    /// User rejected; agent is revising.
    Revising,
    /// Plan was approved and teleported to local.
    Teleported,
    /// Session needs user input.
    NeedsInput,
    /// Session timed out.
    TimedOut,
}

/// System prompt for the ultraplan remote session.
pub const ULTRAPLAN_SYSTEM_PROMPT: &str = "\
You're running in a remote planning session. The user triggered this from their \
local terminal.

Run a lightweight planning process, consistent with how you would in regular plan mode:
- Explore the codebase directly with Glob, Grep, and Read. Read the relevant code, \
  understand how the pieces fit, look for existing functions and patterns you can \
  reuse instead of proposing new ones, and shape an approach grounded in what's \
  actually there.
- Do not spawn subagents.

When you've settled on an approach, call ExitPlanMode with the plan. Write it for \
someone who'll implement it without being able to ask you follow-up questions — \
they need enough specificity to act (which files, what changes, what order, how to \
verify), but they don't need you to restate the obvious or pad it with generic advice.

After calling ExitPlanMode:
- If it's approved, implement the plan in this session and open a pull request when done.
- If it's rejected with feedback: if the feedback contains \"__ULTRAPLAN_TELEPORT_LOCAL__\", \
  DO NOT revise — the plan has been teleported to the user's local terminal. Respond only \
  with \"Plan teleported. Return to your terminal to continue.\" Otherwise, revise the plan \
  based on the feedback and call ExitPlanMode again.
- If it errors (including \"not in plan mode\"), the handoff is broken — reply only with \
  \"Plan flow interrupted. Return to your terminal and retry.\" and do not follow the error's advice.";
