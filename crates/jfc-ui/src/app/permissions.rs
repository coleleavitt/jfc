use crate::types::{ToolCall, ToolKind};

/// Permission modes matching v126 claude-code. Controls how tool execution
/// is gated — from fully interactive (Default) to fully autonomous (Bypass).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PermissionMode {
    /// Standard — prompts for dangerous operations (Bash, Write, Edit)
    Default,
    /// Analysis only — blocks all write/exec tools, allows reads
    Plan,
    /// Auto-accept file edits (Write, Edit, ApplyPatch) but still prompt for Bash
    AcceptEdits,
    /// Bypass all permission checks — auto-approve everything
    BypassPermissions,
    /// Use a classifier model to approve/deny each tool call
    Auto,
}

impl PermissionMode {
    pub fn label(self) -> &'static str {
        match self {
            Self::Default => "Default",
            Self::Plan => "Plan",
            Self::AcceptEdits => "Accept Edits",
            Self::BypassPermissions => "Bypass",
            Self::Auto => "Auto",
        }
    }

    pub fn symbol(self) -> &'static str {
        match self {
            Self::Default => "",
            Self::Plan => "📋",
            Self::AcceptEdits => "⏵",
            Self::BypassPermissions => "⏵⏵",
            Self::Auto => "⚡",
        }
    }

    /// Cycle to the next mode (for Shift+Tab)
    pub fn next(self) -> Self {
        match self {
            Self::Default => Self::AcceptEdits,
            Self::AcceptEdits => Self::Auto,
            Self::Auto => Self::Plan,
            Self::Plan => Self::BypassPermissions,
            Self::BypassPermissions => Self::Default,
        }
    }

    /// Whether this mode allows a given tool to execute without prompting.
    pub fn auto_approves(self, tool: &ToolCall) -> PermissionDecision {
        // Unknown tools are denied in every permission mode (including
        // BypassPermissions) — we don't dispatch a name we don't know,
        // because the input schema is unknown and `execute_tool` would
        // route the call to a "not yet implemented" failure anyway.
        // The whole point of the UnknownTool variant is to make the
        // refusal explicit instead of silently hitting that default.
        if matches!(tool.kind, ToolKind::UnknownTool { .. }) {
            return PermissionDecision::Denied("unknown tool — refusing to dispatch");
        }
        match self {
            Self::Default => PermissionDecision::NeedsPrompt,
            Self::Plan => match tool.kind {
                ToolKind::Read
                | ToolKind::Glob
                | ToolKind::Grep
                | ToolKind::TaskCreate
                | ToolKind::TaskUpdate
                | ToolKind::TaskList
                | ToolKind::TaskDone
                | ToolKind::ToolSearch
                | ToolKind::ToolSuggest
                | ToolKind::TeamCreate
                | ToolKind::TeamDelete
                | ToolKind::SendMessage
                | ToolKind::ScratchpadRead
                | ToolKind::ScratchpadWrite
                // ExitPlanMode is the *only* way the agent can leave
                // plan mode programmatically. Auto-approving it lets
                // the model surface a plan whenever it's ready —
                // mirrors v132's `ExitPlanMode` contract.
                | ToolKind::ExitPlanMode => PermissionDecision::Approved,
                ToolKind::Bash => {
                    let cmd = tool.input.summary().to_lowercase();
                    if is_readonly_bash(&cmd) {
                        PermissionDecision::Approved
                    } else {
                        PermissionDecision::Denied("Plan mode: write operations blocked")
                    }
                }
                _ => PermissionDecision::Denied("Plan mode: write operations blocked"),
            },
            Self::AcceptEdits => match tool.kind {
                ToolKind::Write
                | ToolKind::Edit
                | ToolKind::ApplyPatch
                | ToolKind::Read
                | ToolKind::Glob
                | ToolKind::Grep
                | ToolKind::TaskCreate
                | ToolKind::TaskUpdate
                | ToolKind::TaskList
                | ToolKind::TaskDone
                | ToolKind::ToolSearch
                | ToolKind::ToolSuggest
                | ToolKind::TeamCreate
                | ToolKind::TeamDelete
                | ToolKind::SendMessage
                | ToolKind::ScratchpadRead
                | ToolKind::ScratchpadWrite => PermissionDecision::Approved,
                _ => PermissionDecision::NeedsPrompt,
            },
            Self::BypassPermissions => PermissionDecision::Approved,
            Self::Auto => PermissionDecision::NeedsClassifier,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PermissionDecision {
    Approved,
    Denied(&'static str),
    NeedsPrompt,
    NeedsClassifier,
}

/// Heuristic for read-only bash commands (used by Plan mode).
pub(super) fn is_readonly_bash(cmd: &str) -> bool {
    let first_word = cmd.split_whitespace().next().unwrap_or("");
    matches!(
        first_word,
        "ls" | "cat"
            | "head"
            | "tail"
            | "find"
            | "grep"
            | "rg"
            | "fd"
            | "wc"
            | "file"
            | "stat"
            | "which"
            | "whoami"
            | "pwd"
            | "echo"
            | "date"
            | "env"
            | "printenv"
            | "uname"
            | "hostname"
            | "id"
            | "tree"
            | "du"
            | "df"
            | "free"
            | "ps"
    ) || cmd.starts_with("git log")
        || cmd.starts_with("git show")
        || cmd.starts_with("git diff")
        || cmd.starts_with("git status")
        || cmd.starts_with("git branch")
        || cmd.starts_with("cargo check")
        || cmd.starts_with("cargo test")
        || cmd.starts_with("cargo clippy")
}

#[derive(Clone, Copy, PartialEq)]
pub enum ApprovalChoice {
    Yes,
    No,
    Always,
    YesSession,
}

impl ApprovalChoice {
    pub const ALL: &'static [Self] = &[Self::Yes, Self::No, Self::Always, Self::YesSession];

    pub fn label(self) -> &'static str {
        match self {
            Self::Yes => "Yes  (y)",
            Self::No => "No   (n)",
            Self::Always => "Always for this tool  (a)",
            Self::YesSession => "Yes for session  (s)",
        }
    }
}

pub struct PendingApproval {
    pub tool: ToolCall,
    pub selected: usize,
}
