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
                | ToolKind::CodeIndex
                | ToolKind::GraphQuery
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
                | ToolKind::CodeIndex
                | ToolKind::GraphQuery
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
    let Some(cmd) = readonly_shell_body(cmd) else {
        return false;
    };
    if has_unsafe_shell_control(&cmd) {
        return false;
    }

    let segments = split_shell_pipeline(&cmd);
    !segments.is_empty()
        && segments
            .iter()
            .all(|segment| is_readonly_bash_segment(segment))
}

fn readonly_shell_body(cmd: &str) -> Option<String> {
    let lines = cmd
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .collect::<Vec<_>>();
    if lines.is_empty() {
        return None;
    }

    for pair in lines.windows(2) {
        let prev = pair[0].trim_end();
        let next = pair[1].trim_start();
        let continued = prev.ends_with('|') || prev.ends_with('\\') || next.starts_with('|');
        if !continued {
            return None;
        }
    }

    Some(
        lines
            .into_iter()
            .map(|line| line.strip_suffix('\\').unwrap_or(line).trim_end())
            .collect::<Vec<_>>()
            .join(" "),
    )
}

fn has_unsafe_shell_control(cmd: &str) -> bool {
    let mut chars = cmd.chars().peekable();
    let mut single_quoted = false;
    let mut double_quoted = false;
    let mut escaped = false;
    while let Some(ch) = chars.next() {
        if escaped {
            escaped = false;
            continue;
        }
        if ch == '\\' {
            escaped = true;
            continue;
        }
        match ch {
            '\'' if !double_quoted => single_quoted = !single_quoted,
            '"' if !single_quoted => double_quoted = !double_quoted,
            ';' if !single_quoted && !double_quoted => return true,
            '`' if !single_quoted => return true,
            '&' if !single_quoted && !double_quoted => return true,
            '$' if !single_quoted && chars.peek() == Some(&'(') => return true,
            _ => {}
        }
    }
    false
}

fn split_shell_pipeline(cmd: &str) -> Vec<String> {
    let mut segments = Vec::new();
    let mut current = String::new();
    let mut single_quoted = false;
    let mut double_quoted = false;
    let mut escaped = false;

    for ch in cmd.chars() {
        if escaped {
            current.push(ch);
            escaped = false;
            continue;
        }
        if ch == '\\' {
            current.push(ch);
            escaped = true;
            continue;
        }
        match ch {
            '\'' if !double_quoted => {
                single_quoted = !single_quoted;
                current.push(ch);
            }
            '"' if !single_quoted => {
                double_quoted = !double_quoted;
                current.push(ch);
            }
            '|' if !single_quoted && !double_quoted => {
                let segment = current.trim();
                if segment.is_empty() {
                    return Vec::new();
                }
                segments.push(segment.to_owned());
                current.clear();
            }
            _ => current.push(ch),
        }
    }

    let segment = current.trim();
    if segment.is_empty() {
        return Vec::new();
    }
    segments.push(segment.to_owned());
    segments
}

fn is_readonly_bash_segment(segment: &str) -> bool {
    if !redirections_are_readonly(segment) {
        return false;
    }

    let tokens = shell_words(segment);
    let Some((command_idx, command)) = tokens
        .iter()
        .enumerate()
        .find(|(_, token)| !is_env_assignment(token))
    else {
        return false;
    };
    let command = command.to_ascii_lowercase();
    let args = &tokens[command_idx + 1..];

    match command.as_str() {
        "find" => !args
            .iter()
            .any(|arg| matches!(arg.as_str(), "-delete" | "-exec" | "-execdir")),
        "sed" => !args.iter().any(|arg| arg == "-i" || arg.starts_with("-i.")),
        "git" => args.first().is_some_and(|subcommand| {
            matches!(
                subcommand.as_str(),
                "log" | "show" | "diff" | "status" | "branch"
            )
        }),
        "cargo" => args
            .first()
            .is_some_and(|subcommand| matches!(subcommand.as_str(), "check" | "test" | "clippy")),
        _ => matches!(
            command.as_str(),
            "ls" | "cat"
                | "head"
                | "tail"
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
                | "sort"
                | "uniq"
                | "cut"
                | "tr"
                | "jq"
        ),
    }
}

fn redirections_are_readonly(segment: &str) -> bool {
    let bytes = segment.as_bytes();
    let mut i = 0;
    let mut single_quoted = false;
    let mut double_quoted = false;
    let mut escaped = false;

    while i < bytes.len() {
        let ch = bytes[i] as char;
        if escaped {
            escaped = false;
            i += 1;
            continue;
        }
        if ch == '\\' {
            escaped = true;
            i += 1;
            continue;
        }
        match ch {
            '\'' if !double_quoted => single_quoted = !single_quoted,
            '"' if !single_quoted => double_quoted = !double_quoted,
            '>' if !single_quoted && !double_quoted => {
                let mut target_start = i + 1;
                if target_start < bytes.len() && bytes[target_start] == b'>' {
                    target_start += 1;
                }
                while target_start < bytes.len() && bytes[target_start].is_ascii_whitespace() {
                    target_start += 1;
                }
                let Some((target, next)) = shell_token_at(segment, target_start) else {
                    return false;
                };
                if target != "/dev/null" {
                    return false;
                }
                i = next;
                continue;
            }
            _ => {}
        }
        i += 1;
    }
    true
}

fn shell_words(segment: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut i = 0;
    while i < segment.len() {
        while i < segment.len() && segment.as_bytes()[i].is_ascii_whitespace() {
            i += 1;
        }
        let Some((word, next)) = shell_token_at(segment, i) else {
            break;
        };
        words.push(word.to_ascii_lowercase());
        i = next;
    }
    words
}

fn shell_token_at(segment: &str, start: usize) -> Option<(String, usize)> {
    if start >= segment.len() {
        return None;
    }
    let mut token = String::new();
    let mut single_quoted = false;
    let mut double_quoted = false;
    let mut escaped = false;

    for (rel, ch) in segment[start..].char_indices() {
        let i = start + rel;
        if escaped {
            token.push(ch);
            escaped = false;
            continue;
        }
        if ch == '\\' {
            escaped = true;
            continue;
        }
        match ch {
            '\'' if !double_quoted => single_quoted = !single_quoted,
            '"' if !single_quoted => double_quoted = !double_quoted,
            ch if ch.is_whitespace() && !single_quoted && !double_quoted => {
                return Some((token, i));
            }
            _ => token.push(ch),
        }
    }

    Some((token, segment.len()))
}

fn is_env_assignment(token: &str) -> bool {
    let Some((name, _)) = token.split_once('=') else {
        return false;
    };
    !name.is_empty()
        && name
            .chars()
            .all(|ch| ch == '_' || ch.is_ascii_alphanumeric())
        && name
            .chars()
            .next()
            .is_some_and(|ch| ch == '_' || ch.is_ascii_alphabetic())
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
