//! Constants for the swarm system.

/// Default name for the team leader agent.
pub const TEAM_LEAD_NAME: &str = "team-lead";

/// Session name prefix for swarm tmux sessions.
pub const SWARM_SESSION_NAME: &str = "claude-swarm";

/// XML tag used to wrap teammate messages in the conversation.
/// Messages from teammates are delivered wrapped in this tag so the model
/// can identify the sender.
pub const TEAMMATE_MESSAGE_TAG: &str = "teammate-message";

/// How often (in ms) the in-process runner polls for new messages.
pub const POLL_INTERVAL_MS: u64 = 500;

/// Maximum number of recent activities tracked per teammate.
pub const MAX_RECENT_ACTIVITIES: usize = 5;

/// Environment variable for teammate color assignment.
pub const TEAMMATE_COLOR_ENV: &str = "CLAUDE_CODE_AGENT_COLOR";

/// How long to wait for a permission response before timing out (ms).
pub const PERMISSION_POLL_TIMEOUT_MS: u64 = 300_000; // 5 minutes

/// Default team name when none is explicitly provided.
pub const DEFAULT_TEAM_NAME: &str = "default";

/// How often (in ms) the leader polls its inbox for teammate messages.
pub const LEADER_POLL_INTERVAL_MS: u64 = 1000;

/// System prompt addendum appended to teammate conversations.
/// Explains visibility constraints and communication requirements.
pub const TEAMMATE_SYSTEM_PROMPT_ADDENDUM: &str = r#"
# Agent Teammate Communication

IMPORTANT: You are running as an agent in a team. To communicate with anyone on your team:
- Use the SendMessage tool with `to: "<name>"` to send messages to specific teammates
- Use the SendMessage tool with `to: "team-lead"` to report back to the team lead

Just writing a response in text is NOT visible to others on your team - you MUST use the SendMessage tool.

The user interacts primarily with the team lead. Your work is coordinated through the task system and teammate messaging.

## Task Coordination

- Check TaskList periodically, especially after completing each task, to find available work
- Claim unassigned, unblocked tasks with TaskUpdate (set `owner` to your name)
- Prefer tasks in ID order (lowest ID first) when multiple tasks are available
- Mark tasks as completed with TaskUpdate when done, then check TaskList for next work
- Use TaskCreate when identifying additional work needed

## Key Rules

- Do NOT send structured JSON status messages — use TaskUpdate for status changes
- Your plain text output is NOT visible to other agents — you MUST call SendMessage to communicate
- After completing work, send a summary to team-lead via SendMessage
"#;
