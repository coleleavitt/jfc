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
        // Catastrophic-command backstop. A tiny denylist of effectively
        // unrecoverable, whole-system / whole-history operations
        // (`rm -rf /home`, `dd of=/dev/sdX`, `mkfs`, force-push over master,
        // `rm -rf .git`, fork bomb) forces a confirmation prompt **even in
        // BypassPermissions / Auto** — the two modes that otherwise
        // auto-approve a detached/swarm agent's bash. Without it, a single
        // hallucinated path in an unattended run could wipe the box with no
        // human in the loop. Narrow by design (a 305-session audit found zero
        // real triggers) and overridable via `JFC_ALLOW_CATASTROPHIC_BASH=1`.
        // Default / AcceptEdits already prompt for Bash, so this only changes
        // behaviour where it must.
        if matches!(self, Self::BypassPermissions | Self::Auto)
            && let ToolKind::Bash = tool.kind
            && let ToolInput::Bash { command, .. } = &tool.input
            && let Some(reason) = super::shell_safety::catastrophic_bash_reason(command)
        {
            tracing::warn!(
                target: "jfc::permissions",
                mode = self.label(),
                reason,
                "catastrophic bash command — forcing approval prompt despite auto-approve mode"
            );
            return PermissionDecision::NeedsPrompt;
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
                | ToolKind::Advisor
                | ToolKind::ServerWebSearch
                | ToolKind::ServerAdvisor
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
                | ToolKind::Advisor
                | ToolKind::ServerWebSearch
                | ToolKind::ServerAdvisor
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

/// One selectable option in an `AskUserQuestion` prompt. The auto-injected
/// "Other" row is NOT stored here — it's rendered/handled positionally as the
/// row just past `options.len()`, mirroring Claude Code's `__other__` sentinel.
#[derive(Clone, Debug)]
pub struct QuestionOption {
    pub label: String,
    pub description: String,
    pub preview: Option<String>,
}

/// A pending `AskUserQuestion` modal awaiting the user's selection.
///
/// Mirrors [`PendingApproval`] structurally, but the semantics differ: an
/// approval *gates a tool dispatch*, whereas a question *collects an answer
/// that becomes the tool_result*. The event loop blocks the agentic
/// continuation while this is `Some`; on submit, `answer()` is turned into a
/// `ToolEvent::Result` for `tool_id` and the loop resumes (see
/// `input/question.rs`).
pub struct PendingQuestion {
    /// The `AskUserQuestion` tool_use this modal answers. The synthesized
    /// result is recorded against this id.
    pub tool_id: crate::ids::ToolId,
    /// The question prose (ends with `?`).
    pub question: String,
    /// Short chip label (≤12 chars in the contract). Empty if the model
    /// omitted it.
    pub header: String,
    /// The model-supplied options, in order. Excludes the auto "Other" row.
    pub options: Vec<QuestionOption>,
    /// When true the user may pick more than one option.
    pub multi_select: bool,
    /// Cursor over `[options…, Other]`. `selected == options.len()` is the
    /// "Other" free-text row.
    pub selected: usize,
    /// Multi-select: chosen option indices (into `options`); the "Other" row,
    /// when chosen, is represented by `options.len()`.
    pub chosen: std::collections::BTreeSet<usize>,
    /// Whether the "Other" free-text input currently has focus (keys append to
    /// `other_text` instead of moving the cursor).
    pub editing_other: bool,
    /// Free text typed into the "Other" row.
    pub other_text: String,
}

impl PendingQuestion {
    /// Index of the synthetic "Other" row in the cursor space.
    pub fn other_row(&self) -> usize {
        self.options.len()
    }

    /// True when the cursor is on the "Other" row.
    pub fn on_other(&self) -> bool {
        self.selected == self.other_row()
    }

    /// Total navigable rows: every option plus the "Other" row.
    pub fn row_count(&self) -> usize {
        self.options.len() + 1
    }

    /// Build the answer string sent back to the model.
    ///
    /// Single-select: the focused option's label, or the typed "Other" text.
    /// Multi-select: every chosen option label in order, plus the "Other" text
    /// when the "Other" row is chosen, comma-joined (matches Claude Code's
    /// comma-separated multi-select answers).
    pub fn answer(&self) -> String {
        let other = self.other_text.trim();
        if self.multi_select {
            let mut parts: Vec<String> = self
                .options
                .iter()
                .enumerate()
                .filter(|(i, _)| self.chosen.contains(i))
                .map(|(_, opt)| opt.label.clone())
                .collect();
            if self.chosen.contains(&self.other_row()) && !other.is_empty() {
                parts.push(other.to_owned());
            }
            parts.join(", ")
        } else if self.on_other() {
            other.to_owned()
        } else {
            self.options
                .get(self.selected)
                .map(|opt| opt.label.clone())
                .unwrap_or_default()
        }
    }

    /// Whether the current selection is submittable. Prevents committing an
    /// empty answer (e.g. "Other" focused with no text, or multi-select with
    /// nothing checked).
    pub fn can_submit(&self) -> bool {
        !self.answer().trim().is_empty()
    }
}
