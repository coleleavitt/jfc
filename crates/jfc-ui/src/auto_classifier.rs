//! Auto-mode permission classifier.
//!
//! When the user opts into permission mode `auto`, the application
//! delegates each tool-use decision to this classifier. The classifier
//! is deliberately conservative — anything it can't prove safe falls
//! through to [`ClassifierDecision::Ask`] (or `Deny` when the input
//! matches a known-destructive pattern).
//!
//! Decision precedence, top-down:
//!
//! 1. **Read-only tools** (`Read`, `Glob`, `Grep`, `Search`,
//!    graph queries, …) → always `Allow`.
//! 2. **Tool kind on the allow-rules list** (`Bash`, `Edit`, etc.) →
//!    `Allow`. Rules accept either tool kinds (`"Edit"`, `"Bash"`) or
//!    Bash command prefixes (`"Bash(git status*)"`, `"Bash(cargo *)"`).
//! 3. **Bash safe-command pattern** (`git status`, `cargo check`,
//!    `ls`, `pwd`, …) → `Allow`.
//! 4. **Bash destructive-command pattern** (`rm -rf`, `git push
//!    --force`, `dd`, `sudo`, …) → `Deny`.
//! 5. Anything else → `Ask`.
//!
//! The classifier is intentionally `pub fn` (not a method) so it can
//! be wired from anywhere — `app::permissions`, headless mode, the
//! intent gate — without instantiating a struct.

use jfc_core::{ToolInput, ToolKind};

/// Outcome of classifying one tool call.
#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClassifierDecision {
    /// Auto-approve.
    Allow,
    /// Auto-deny with an explanation suitable for showing the user.
    Deny(String),
    /// Don't decide — fall back to interactive prompt.
    Ask,
}

/// Bash command prefixes considered safe enough to auto-allow.
///
/// Each entry is a *prefix* matched against the trimmed, lower-cased
/// command. Whitespace after the prefix is required (a literal end
/// of string or space char) so `cargo` doesn't accidentally match
/// `cargox`.
const SAFE_BASH_PREFIXES: &[&str] = &[
    "ls",
    "pwd",
    "echo",
    "cat",
    "head",
    "tail",
    "wc",
    "true",
    "false",
    "date",
    "uname",
    "whoami",
    "id",
    "env",
    "which",
    "type",
    "git status",
    "git diff",
    "git log",
    "git show",
    "git branch",
    "git remote",
    "git stash list",
    "git rev-parse",
    "cargo check",
    "cargo build",
    "cargo test",
    "cargo clippy",
    "cargo fmt --check",
    "cargo metadata",
    "cargo tree",
    "cargo doc",
    "rustc --version",
    "rustup show",
    "go vet",
    "go test",
    "go build",
    "npm test",
    "npm run lint",
    "npm run build",
    "python -V",
    "python --version",
    "python3 -V",
    "python3 --version",
];

/// Bash patterns that are *always* denied even if the allow-rules
/// list mentions them. These are the "you almost certainly don't
/// want this" cases.
const DESTRUCTIVE_BASH_PATTERNS: &[&str] = &[
    "rm -rf ",
    "rm -fr ",
    "rm --recursive --force",
    "rm -r /",
    "sudo ",
    "su -",
    "su root",
    "git push --force",
    "git push -f",
    "git push --force-with-lease",
    "git reset --hard origin",
    "git clean -fdx",
    "git clean -fdX",
    "dd if=",
    "dd of=/dev",
    "mkfs",
    " :(){:|:&};:",
    "chmod -R 777",
    "chown -R",
    "shutdown ",
    "reboot",
    "halt",
    "kill -9 1",
    "killall -9",
    "> /dev/sda",
];

/// Returns `true` if a bash command matches a known-destructive pattern
/// (rm -rf, git push --force, dd, sudo, etc.). Used by the destructive
/// command warning UI (`DestructiveWarn` feature gate) to surface a
/// ⚠ DESTRUCTIVE label in the approval prompt.
pub fn is_destructive_bash(command: &str) -> bool {
    let normalized = command.trim().to_lowercase();
    if normalized.is_empty() {
        return false;
    }
    DESTRUCTIVE_BASH_PATTERNS
        .iter()
        .any(|pat| normalized.contains(pat))
}

/// Top-level entry: classify one tool call.
pub fn classify_tool_use(
    kind: &ToolKind,
    input: &ToolInput,
    allow_rules: &[String],
) -> ClassifierDecision {
    if is_read_only(kind) {
        return ClassifierDecision::Allow;
    }
    if is_explicitly_allowed(kind, input, allow_rules) {
        return ClassifierDecision::Allow;
    }
    match input {
        ToolInput::Bash { command, .. } => classify_bash(command),
        _ => ClassifierDecision::Ask,
    }
}

/// Tools that never mutate state and are always safe to auto-allow.
fn is_read_only(kind: &ToolKind) -> bool {
    matches!(
        kind,
        ToolKind::Read
            | ToolKind::Glob
            | ToolKind::Grep
            | ToolKind::Search
            | ToolKind::GraphSearch
            | ToolKind::GraphQuery
            | ToolKind::GraphContext
            | ToolKind::GraphCallers
            | ToolKind::GraphCallees
            | ToolKind::GraphImpact
            | ToolKind::GraphNode
            | ToolKind::GraphExplore
            | ToolKind::GraphStatus
            | ToolKind::GraphFiles
            | ToolKind::CodeIndex
            | ToolKind::TaskList
            | ToolKind::TaskGet
            | ToolKind::TaskValidate
            | ToolKind::MarketStatus
            | ToolKind::PlanList
            | ToolKind::PlanShow
            | ToolKind::LearnStatus
            | ToolKind::LearnKeyFilesList
            | ToolKind::LearnUserProfileShow
    )
}

/// Check whether the user's allow-rule list explicitly permits this
/// tool. Rules look like `"Edit"`, `"Bash"`, or `"Bash(git status*)"`.
fn is_explicitly_allowed(kind: &ToolKind, input: &ToolInput, allow_rules: &[String]) -> bool {
    let kind_name = tool_kind_name(kind);
    for rule in allow_rules {
        let trimmed = rule.trim();
        if trimmed.is_empty() {
            continue;
        }
        // Bare tool name: `Bash`, `Edit`, ...
        if trimmed.eq_ignore_ascii_case(kind_name) {
            return true;
        }
        // Scoped Bash rule: `Bash(git status*)`.
        if let Some(rest) = trimmed
            .strip_prefix("Bash(")
            .and_then(|s| s.strip_suffix(')'))
            && matches!(kind, ToolKind::Bash)
            && let ToolInput::Bash { command, .. } = input
            && glob_prefix_match(rest, command.trim())
        {
            return true;
        }
    }
    false
}

/// Tiny prefix-with-trailing-`*` matcher. We don't pull in `globset`
/// for this — auto-classifier hot path runs on every tool call.
fn glob_prefix_match(pattern: &str, command: &str) -> bool {
    if let Some(prefix) = pattern.strip_suffix('*') {
        command.starts_with(prefix)
    } else {
        pattern == command
    }
}

fn classify_bash(command: &str) -> ClassifierDecision {
    let normalized = command.trim().to_lowercase();
    if normalized.is_empty() {
        return ClassifierDecision::Ask;
    }
    for pat in DESTRUCTIVE_BASH_PATTERNS {
        if normalized.contains(pat) {
            return ClassifierDecision::Deny(format!(
                "destructive pattern `{}` detected in command",
                pat.trim()
            ));
        }
    }
    for prefix in SAFE_BASH_PREFIXES {
        if matches_safe_prefix(&normalized, prefix) {
            return ClassifierDecision::Allow;
        }
    }
    ClassifierDecision::Ask
}

fn matches_safe_prefix(command: &str, prefix: &str) -> bool {
    if !command.starts_with(prefix) {
        return false;
    }
    match command.as_bytes().get(prefix.len()) {
        None => true,       // exact match
        Some(b' ') => true, // followed by args
        Some(b'\t') => true,
        _ => false,
    }
}

/// Human-readable name for a [`ToolKind`] used in allow-rule
/// matching. Matches the names users type into their settings file.
fn tool_kind_name(kind: &ToolKind) -> &'static str {
    match kind {
        ToolKind::Edit => "Edit",
        ToolKind::Write => "Write",
        ToolKind::Read => "Read",
        ToolKind::Bash => "Bash",
        ToolKind::Glob => "Glob",
        ToolKind::Grep => "Grep",
        ToolKind::Search => "Search",
        ToolKind::ApplyPatch => "ApplyPatch",
        ToolKind::MultiEdit => "MultiEdit",
        _ => "Other",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use jfc_core::ReplacementMode;

    fn bash(cmd: &str) -> ToolInput {
        ToolInput::Bash {
            command: cmd.to_string(),
            timeout: None,
            workdir: None,
        }
    }

    fn edit() -> ToolInput {
        ToolInput::Edit {
            file_path: "/tmp/x".into(),
            old_string: "a".into(),
            new_string: "b".into(),
            replacement: ReplacementMode::FirstOnly,
        }
    }

    #[test]
    fn read_tool_always_allowed() {
        let input = ToolInput::Read {
            file_path: "/etc/hosts".into(),
            offset: None,
            limit: None,
        };
        assert_eq!(
            classify_tool_use(&ToolKind::Read, &input, &[]),
            ClassifierDecision::Allow
        );
    }

    #[test]
    fn safe_bash_prefix_allowed() {
        assert_eq!(
            classify_tool_use(&ToolKind::Bash, &bash("git status -s"), &[]),
            ClassifierDecision::Allow
        );
        assert_eq!(
            classify_tool_use(&ToolKind::Bash, &bash("cargo check"), &[]),
            ClassifierDecision::Allow
        );
    }

    #[test]
    fn destructive_bash_denied() {
        match classify_tool_use(&ToolKind::Bash, &bash("rm -rf /tmp/foo"), &[]) {
            ClassifierDecision::Deny(reason) => assert!(reason.contains("rm -rf")),
            other => panic!("expected Deny, got {other:?}"),
        }
        assert!(matches!(
            classify_tool_use(&ToolKind::Bash, &bash("git push --force origin"), &[]),
            ClassifierDecision::Deny(_)
        ));
    }

    #[test]
    fn unknown_bash_asks() {
        assert_eq!(
            classify_tool_use(&ToolKind::Bash, &bash("./deploy.sh prod"), &[]),
            ClassifierDecision::Ask
        );
    }

    #[test]
    fn explicit_allow_rule_for_edit() {
        let rules = vec!["Edit".to_string()];
        assert_eq!(
            classify_tool_use(&ToolKind::Edit, &edit(), &rules),
            ClassifierDecision::Allow
        );
    }

    #[test]
    fn scoped_bash_rule_matches_prefix() {
        let rules = vec!["Bash(./deploy.sh*)".to_string()];
        assert_eq!(
            classify_tool_use(&ToolKind::Bash, &bash("./deploy.sh staging"), &rules),
            ClassifierDecision::Allow
        );
    }

    #[test]
    fn destructive_pattern_denies_even_when_unrelated_rule_present() {
        // Even with an Edit allow-rule, rm -rf is still denied.
        let rules = vec!["Edit".to_string()];
        assert!(matches!(
            classify_tool_use(&ToolKind::Bash, &bash("rm -rf /etc"), &rules),
            ClassifierDecision::Deny(_)
        ));
    }

    #[test]
    fn edit_without_rule_asks() {
        assert_eq!(
            classify_tool_use(&ToolKind::Edit, &edit(), &[]),
            ClassifierDecision::Ask
        );
    }
}
