//! Constants for the swarm system.

/// Default name for the team leader agent.
pub const TEAM_LEAD_NAME: &str = "team-lead";

/// XML tag used to wrap teammate messages in the conversation.
pub const TEAMMATE_MESSAGE_TAG: &str = "teammate-message";

/// How often (in ms) the in-process runner polls for new messages.
pub const POLL_INTERVAL_MS: u64 = 500;

/// Default team name when none is explicitly provided.
pub const DEFAULT_TEAM_NAME: &str = "default";

/// How often (in ms) the leader polls its inbox for teammate messages.
#[allow(dead_code)]
pub const LEADER_POLL_INTERVAL_MS: u64 = 1000;

/// System prompt addendum appended to teammate conversations.
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
