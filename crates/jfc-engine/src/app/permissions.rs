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
        self.decide_parts(&tool.kind, &tool.input)
    }

    /// Permission decision from a tool's kind + input, without a full
    /// [`ToolCall`]. Used by non-interactive executors (e.g. background
    /// subagents) that need the same gate but don't have a `ToolCall` in hand.
    pub fn decide_parts(self, kind: &ToolKind, input: &ToolInput) -> PermissionDecision {
        if let Some(policy) = crate::tools::external_tool_policy(kind) {
            return self.decide_external_tool(policy.approval_policy);
        }
        // Unknown tools are denied in every permission mode (including
        // BypassPermissions) — we don't dispatch a name we don't know,
        // because the input schema is unknown and `execute_tool` would
        // route the call to a "not yet implemented" failure anyway.
        // The whole point of the UnknownTool variant is to make the
        // refusal explicit instead of silently hitting that default.
        if matches!(kind, ToolKind::UnknownTool { .. }) {
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
            && let ToolKind::Bash = kind
            && let ToolInput::Bash { command, .. } = input
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
            Self::Default => {
                // CC 177 parity: auto-approve read-only bash commands even in
                // Default mode. This covers ls, cat, git status, etc. — the
                // commands that never prompt in CC 177 regardless of mode.
                // Non-bash tools still need explicit approval in Default mode.
                if let ToolKind::Bash = kind {
                    if let ToolInput::Bash { command, .. } = input {
                        if super::shell_safety::is_readonly_bash(command) {
                            return PermissionDecision::Approved;
                        }
                    }
                }
                PermissionDecision::NeedsPrompt
            }
            Self::Plan => match kind {
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
                | ToolKind::MarketStatus
                | ToolKind::CronList
                | ToolKind::TeamCreate
                | ToolKind::TeamDelete
                | ToolKind::SendMessage
                | ToolKind::ScratchpadRead
                | ToolKind::ScratchpadWrite
                | ToolKind::AskUserQuestion
                | ToolKind::EnterPlanMode
                | ToolKind::DesignProjectList
                | ToolKind::DesignListFiles
                | ToolKind::DesignReadFile
                | ToolKind::DesignCapabilities
                // ExitPlanMode is the *only* way the agent can leave
                // plan mode programmatically. Auto-approving it lets
                // the model surface a plan whenever it's ready —
                // mirrors v132's `ExitPlanMode` contract.
                | ToolKind::ExitPlanMode => PermissionDecision::Approved,
                ToolKind::Mcp(name) if is_plan_safe_mcp_tool(name) => {
                    PermissionDecision::Approved
                }
                ToolKind::Bash => {
                    let ToolInput::Bash { command, .. } = input else {
                        return PermissionDecision::Denied("Plan mode: malformed bash input");
                    };
                    match super::shell_safety::classify_readonly_bash(command) {
                        Ok(()) => PermissionDecision::Approved,
                        Err(reason) => PermissionDecision::Denied(reason),
                    }
                }
                _ => PermissionDecision::Denied("Plan mode: write operations blocked"),
            },
            Self::AcceptEdits => match kind {
                ToolKind::Write
                | ToolKind::Edit
                | ToolKind::MultiEdit
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
                | ToolKind::ExitWorktree
                | ToolKind::DesignProjectCreate
                | ToolKind::DesignProjectList
                | ToolKind::DesignProjectSetMeta
                | ToolKind::DesignListFiles
                | ToolKind::DesignReadFile
                | ToolKind::DesignWriteFile
                | ToolKind::DesignDeleteFile
                | ToolKind::DesignCopyFile
                | ToolKind::DesignRegisterAsset
                | ToolKind::DesignUnregisterAsset
                | ToolKind::DesignBundleHtml
                | ToolKind::DesignHandoff
                | ToolKind::DesignCheckSystem
                | ToolKind::DesignCapabilities
                | ToolKind::DesignServe => PermissionDecision::Approved,
                _ => PermissionDecision::NeedsPrompt,
            },
            Self::BypassPermissions => PermissionDecision::Approved,
            Self::Auto => PermissionDecision::NeedsClassifier,
        }
    }

    fn decide_external_tool(
        self,
        approval_policy: jfc_plugin_sdk::ToolApprovalPolicy,
    ) -> PermissionDecision {
        match self {
            Self::Default => {
                if approval_policy.needs_interactive_approval() {
                    PermissionDecision::NeedsPrompt
                } else {
                    PermissionDecision::Approved
                }
            }
            Self::Plan => {
                if approval_policy.plan_mode_allowed() {
                    PermissionDecision::Approved
                } else {
                    PermissionDecision::Denied("Plan mode: plugin mutating tool blocked")
                }
            }
            Self::AcceptEdits => {
                if approval_policy.mutates_user_state() {
                    PermissionDecision::NeedsPrompt
                } else {
                    PermissionDecision::Approved
                }
            }
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

/// Process-default [`crate::runtime::RuntimePolicy`] backed by
/// [`PermissionMode`] gating. Stateless service struct in the same shape as
/// [`crate::diagnostics::GlobalDiagnosticsService`] and
/// `BuiltinToolRuntime`: it fronts the existing pure decision logic so the
/// runtime can read tool gating through the policy service boundary instead of
/// reaching into the mode enum directly.
pub struct BuiltinRuntimePolicy;

impl crate::runtime::RuntimeService for BuiltinRuntimePolicy {
    fn service_name(&self) -> &'static str {
        "builtin-runtime-policy"
    }
}

impl crate::runtime::RuntimePolicy for BuiltinRuntimePolicy {
    fn tool_decision(
        &self,
        mode: PermissionMode,
        kind: &ToolKind,
        input: &ToolInput,
    ) -> PermissionDecision {
        mode.decide_parts(kind, input)
    }
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

/// One question within an `AskUserQuestion` prompt, plus the user's
/// in-progress selection for it. The auto-injected "Other" row is handled
/// positionally as the row just past `options.len()` (Claude Code's
/// `__other__` sentinel).
pub struct QuestionItem {
    /// The question prose (ends with `?`).
    pub question: String,
    /// Short chip label (≤12 chars in the contract). Empty if omitted.
    pub header: String,
    /// Model-supplied options, in order. Excludes the auto "Other" row.
    pub options: Vec<QuestionOption>,
    /// When true the user may pick more than one option.
    pub multi_select: bool,
    /// Cursor over `[options…, Other]`. `selected == options.len()` is "Other".
    pub selected: usize,
    /// Multi-select: chosen option indices; the "Other" row is `options.len()`.
    pub chosen: std::collections::BTreeSet<usize>,
    /// Free text typed into the "Other" row.
    pub other_text: String,
    /// The committed answer, set when the user confirms this question. `None`
    /// while still pending.
    pub answer: Option<String>,
}

impl QuestionItem {
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

    /// The current selection as an answer string (not yet committed).
    ///
    /// Single-select: the focused option's label, or the typed "Other" text.
    /// Multi-select: every chosen option label in order, plus the "Other" text
    /// when the "Other" row is chosen, comma-joined (matches Claude Code's
    /// comma-separated multi-select answers).
    pub fn current_selection(&self) -> String {
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

    /// Whether the current selection is committable (non-empty).
    pub fn can_commit(&self) -> bool {
        !self.current_selection().trim().is_empty()
    }
}

/// A pending `AskUserQuestion` modal awaiting the user's answers.
///
/// Mirrors [`PendingApproval`] structurally, but the semantics differ: an
/// approval *gates a tool dispatch*, whereas a question *collects answers that
/// become the tool_result*. The event loop blocks the agentic continuation
/// while this is `Some`; once every question is committed, `combined_result()`
/// is turned into a `ToolEvent::Result` for `tool_id` and the loop resumes
/// (see `input/question.rs`). Questions are presented one at a time; `current`
/// is the focused one and the nav bar shows progress.
pub struct PendingQuestion {
    /// The `AskUserQuestion` tool_use this modal answers.
    pub tool_id: crate::ids::ToolId,
    /// The 1-4 questions, in order. Always non-empty.
    pub items: Vec<QuestionItem>,
    /// Index of the focused question.
    pub current: usize,
    /// Whether the focused question's "Other" free-text input has focus.
    pub editing_other: bool,
}

impl PendingQuestion {
    /// The focused question.
    pub fn cur(&self) -> &QuestionItem {
        &self.items[self.current]
    }

    /// The focused question, mutably.
    pub fn cur_mut(&mut self) -> &mut QuestionItem {
        &mut self.items[self.current]
    }

    /// True once every question has a committed answer. (Exercised by tests;
    /// the runtime path uses `advance_to_next_unanswered`'s return instead.)
    pub fn all_committed(&self) -> bool {
        self.items.iter().all(|i| i.answer.is_some())
    }

    /// Move `current` to the next question lacking a committed answer (wrapping
    /// from the end). Returns false when every question is already committed.
    pub fn advance_to_next_unanswered(&mut self) -> bool {
        let n = self.items.len();
        for step in 1..=n {
            let idx = (self.current + step) % n;
            if self.items[idx].answer.is_none() {
                self.current = idx;
                self.editing_other = false;
                return true;
            }
        }
        false
    }

    /// The combined tool_result: `"Q1"="A1", "Q2"="A2"` (Claude Code's format).
    /// Uses each committed answer, falling back to the live selection.
    pub fn combined_result(&self) -> String {
        self.items
            .iter()
            .map(|i| {
                let a = i.answer.clone().unwrap_or_else(|| i.current_selection());
                format!("\"{}\"=\"{}\"", i.question, a)
            })
            .collect::<Vec<_>>()
            .join(", ")
    }
}
