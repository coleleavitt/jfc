use crate::types::{ToolCall, ToolInput, ToolKind};

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
                | ToolKind::Search
                | ToolKind::Lsp
                | ToolKind::WebFetch
                | ToolKind::WebSearch
                | ToolKind::ServerWebSearch
                | ToolKind::NotebookRead
                | ToolKind::TaskCreate
                | ToolKind::TaskUpdate
                | ToolKind::TaskList
                | ToolKind::TaskDone
                | ToolKind::TaskStop
                | ToolKind::TaskGet
                | ToolKind::TaskValidate
                | ToolKind::ToolSearch
                | ToolKind::ToolSuggest
                | ToolKind::CodeIndex
                | ToolKind::GraphQuery
                | ToolKind::GraphSearch
                | ToolKind::GraphContext
                | ToolKind::GraphNode
                | ToolKind::GraphExplore
                | ToolKind::GraphCallers
                | ToolKind::GraphCallees
                | ToolKind::GraphImpact
                | ToolKind::GraphOutline
                | ToolKind::GraphGrep
                | ToolKind::GraphStatus
                | ToolKind::GraphFiles
                | ToolKind::RunCoverage
                | ToolKind::MarketStatus
                | ToolKind::CronList
                | ToolKind::TeamCreate
                | ToolKind::TeamDelete
                | ToolKind::SendMessage
                | ToolKind::ScratchpadRead
                | ToolKind::ScratchpadWrite
                | ToolKind::AskUserQuestion
                | ToolKind::EnterPlanMode
                // ExitPlanMode is the *only* way the agent can leave
                // plan mode programmatically. Auto-approving it lets
                // the model surface a plan whenever it's ready —
                // mirrors v132's `ExitPlanMode` contract.
                | ToolKind::ExitPlanMode => PermissionDecision::Approved,
                ToolKind::Mcp(ref name) if is_plan_safe_mcp_tool(name) => {
                    PermissionDecision::Approved
                }
                ToolKind::Bash => {
                    let ToolInput::Bash { command, .. } = &tool.input else {
                        return PermissionDecision::Denied("Plan mode: malformed bash input");
                    };
                    match super::shell_safety::classify_readonly_bash(command) {
                        Ok(()) => PermissionDecision::Approved,
                        Err(reason) => PermissionDecision::Denied(reason),
                    }
                }
                _ => PermissionDecision::Denied("Plan mode: write operations blocked"),
            },
            Self::AcceptEdits => match tool.kind {
                ToolKind::Write
                | ToolKind::Edit
                | ToolKind::MultiEdit
                | ToolKind::SymbolEdit
                | ToolKind::ApplyPatch
                | ToolKind::Read
                | ToolKind::Glob
                | ToolKind::Grep
                | ToolKind::Search
                | ToolKind::Lsp
                | ToolKind::WebFetch
                | ToolKind::WebSearch
                | ToolKind::ServerWebSearch
                | ToolKind::NotebookRead
                | ToolKind::NotebookEdit
                | ToolKind::TaskCreate
                | ToolKind::TaskUpdate
                | ToolKind::TaskList
                | ToolKind::TaskDone
                | ToolKind::TaskStop
                | ToolKind::TaskGet
                | ToolKind::TaskValidate
                | ToolKind::ToolSearch
                | ToolKind::ToolSuggest
                | ToolKind::CodeIndex
                | ToolKind::GraphQuery
                | ToolKind::GraphSearch
                | ToolKind::GraphContext
                | ToolKind::GraphNode
                | ToolKind::GraphExplore
                | ToolKind::GraphCallers
                | ToolKind::GraphCallees
                | ToolKind::GraphImpact
                | ToolKind::GraphOutline
                | ToolKind::GraphGrep
                | ToolKind::GraphStatus
                | ToolKind::GraphFiles
                | ToolKind::RunCoverage
                | ToolKind::MarketStatus
                | ToolKind::CronList
                | ToolKind::TeamCreate
                | ToolKind::TeamDelete
                | ToolKind::SendMessage
                | ToolKind::ScratchpadRead
                | ToolKind::ScratchpadWrite
                | ToolKind::AskUserQuestion
                | ToolKind::EnterPlanMode
                | ToolKind::ExitPlanMode
                | ToolKind::EnterWorktree
                | ToolKind::ExitWorktree => PermissionDecision::Approved,
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

fn is_plan_safe_mcp_tool(name: &str) -> bool {
    let Some((server, tool)) = crate::mcp::protocol::split_advertised(name) else {
        return false;
    };
    // CodeGraph tools are structural read/query operations backed by the
    // pre-built project index. Keep this narrow: arbitrary MCP servers may
    // expose write-capable tools with read-looking names.
    server == "codegraph" && tool.starts_with("codegraph_")
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
