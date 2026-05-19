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
                    let ToolInput::Bash { command, .. } = &tool.input else {
                        return PermissionDecision::Denied("Plan mode: malformed bash input");
                    };
                    match classify_readonly_bash(command) {
                        Ok(()) => PermissionDecision::Approved,
                        Err(reason) => PermissionDecision::Denied(reason),
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
///
/// Hardened against the bypass classes documented at CVE-2025-54795
/// (Cymulate `echo` quote-escape), CVE-2025-66032 (Flatt 8-bypass
/// chain), and the broader CWE-78 surface. The classifier is layered:
///
///   1. **Raw-byte deny list** — pattern strings that no safe command
///      should ever contain (`/dev/tcp/`, `${IFS}`, `@P}`, `<<<`,
///      `--checkpoint-action=`, dangerous env-prefixes, …). Matched on
///      the unescaped bytes so `\c\a\t` reassembles back to `cat`
///      *after* this layer's check.
///   2. **Shell-control reject** — backticks, `$(…)`, bare `&`.
///   3. **Segment splitter** — `;`/`&&`/`||`/`|` separate independent
///      commands; each segment is classified independently.
///   4. **Per-segment classifier** — head command must be in the
///      positive allowlist; flags must satisfy the per-tool guards
///      (`find` write-actions, `sed -i`, `git -c`, etc.).
pub(super) fn is_readonly_bash(cmd: &str) -> bool {
    classify_readonly_bash(cmd).is_ok()
}

/// Like [`is_readonly_bash`] but returns the specific deny reason so
/// the UI can surface *which* layer rejected the command. Each `Err`
/// variant is a `&'static str` suitable for display in toasts /
/// status badges / approval dialogs.
pub(super) fn classify_readonly_bash(cmd: &str) -> Result<(), &'static str> {
    let cmd = readonly_shell_body(cmd).ok_or(REASON_MULTILINE)?;
    if let Some(reason) = first_dangerous_reason(&cmd) {
        return Err(reason);
    }
    if has_unsafe_shell_control(&cmd) {
        return Err(REASON_SHELL_CONTROL);
    }
    let segments = split_readonly_shell_segments(&cmd);
    if segments.is_empty() {
        return Err(REASON_EMPTY_SEGMENT);
    }
    for segment in &segments {
        if let Err(reason) = classify_readonly_segment(segment) {
            return Err(reason);
        }
    }
    Ok(())
}

// ─── Reason constants ─────────────────────────────────────────────
// Surfaced verbatim in the UI denial badge so the user can see the
// exact reason a command was rejected (mirrors v126's denial-reason
// surfacing). All `&'static str` so `PermissionDecision::Denied`
// keeps its zero-allocation contract.

pub(super) const REASON_MULTILINE: &str =
    "Plan mode: multi-line command without continuation (use `|` or `\\`)";
pub(super) const REASON_SHELL_CONTROL: &str =
    "Plan mode: command substitution / backgrounding not allowed";
pub(super) const REASON_EMPTY_SEGMENT: &str =
    "Plan mode: empty command segment";
pub(super) const REASON_DEV_TCP: &str =
    "Plan mode: /dev/tcp /dev/udp pseudo-devices (network exfiltration)";
pub(super) const REASON_ENV_MUTATE: &str =
    "Plan mode: dangerous env-prefix (LD_PRELOAD / BASH_ENV / PROMPT_COMMAND / …)";
pub(super) const REASON_PARAM_MUTATE: &str =
    "Plan mode: parameter-expansion mutation (${var:=…} / ${var@P})";
pub(super) const REASON_HEREDOC: &str =
    "Plan mode: heredoc / herestring (body may contain command substitution)";
pub(super) const REASON_PROCESS_SUB: &str =
    "Plan mode: process substitution <(…) / >(…)";
pub(super) const REASON_LONG_OPT_RCE: &str =
    "Plan mode: long-option RCE vector (--pre / --checkpoint-action / --html=…)";
pub(super) const REASON_HEAD_BLOCKED: &str =
    "Plan mode: shell wrapper / REPL-from-args head (bash -c / eval / xargs / make / …)";
pub(super) const REASON_NOT_ALLOWLISTED: &str =
    "Plan mode: command not in read-only allowlist";
pub(super) const REASON_FIND_WRITE: &str =
    "Plan mode: find with write action (-delete / -exec / -fprint / -fls)";
pub(super) const REASON_SED_EXEC: &str =
    "Plan mode: sed with `e` modifier / `w` write / `-i` in-place";
pub(super) const REASON_AWK_EXEC: &str =
    "Plan mode: awk script with system() / getline / print-to-file";
pub(super) const REASON_GIT_HOOK_RCE: &str =
    "Plan mode: `git -c` flag (pager/editor/sshCommand RCE)";
pub(super) const REASON_GIT_SUBCOMMAND: &str =
    "Plan mode: git subcommand not in read-only set";
pub(super) const REASON_CURL_WRITE: &str =
    "Plan mode: curl with write flag (-X / -d / --data / -T / -o / -F)";
pub(super) const REASON_WGET_WRITE: &str =
    "Plan mode: wget without --spider (writes to disk)";
pub(super) const REASON_SSH_FORWARD: &str =
    "Plan mode: ssh with port-forward / agent-forward flag";
pub(super) const REASON_SSH_INTERACTIVE: &str =
    "Plan mode: ssh without an explicit remote command";
pub(super) const REASON_SUDO_BARE: &str =
    "Plan mode: sudo / doas without a command to elevate";
pub(super) const REASON_REDIRECT: &str =
    "Plan mode: redirect target is not /dev/null or another FD";
pub(super) const REASON_FIND_NO_ACTION: &str =
    "Plan mode: find without any allowlisted action (or with unknown flag)";

/// First-match raw-byte deny scan; returns the reason constant for
/// whichever class triggered, or `None` if the bytes are clean.
fn first_dangerous_reason(cmd: &str) -> Option<&'static str> {
    if cmd.contains("/dev/tcp/") || cmd.contains("/dev/udp/") {
        return Some(REASON_DEV_TCP);
    }
    if cmd.contains("${IFS")
        || cmd.contains("${!")
        || cmd.contains("@P}")
        || cmd.contains("@E}")
        || cmd.contains("@A}")
    {
        return Some(REASON_PARAM_MUTATE);
    }
    if cmd.contains("<<<") || cmd.contains("<<-") || cmd.contains("<<EOF") {
        return Some(REASON_HEREDOC);
    }
    if cmd.contains("<(") || cmd.contains(">(") {
        return Some(REASON_PROCESS_SUB);
    }
    for needle in [
        "--html=", "--pager=", "--compress-program=", "--use-compress-program=",
        "--preprocessor=", "--pre=", "--checkpoint-action=", "--unzip-command=",
        "--rsh=", "--upload-pack=", "--receive-pack=", "--exec-path=",
    ] {
        if cmd.contains(needle) {
            return Some(REASON_LONG_OPT_RCE);
        }
    }
    for needle in [
        "LD_PRELOAD=", "LD_AUDIT=", "LD_LIBRARY_PATH=",
        "BASH_ENV=", "ENV=", "PROMPT_COMMAND=",
        "GIT_EXTERNAL_DIFF=", "GIT_PAGER=", "GIT_SSH_COMMAND=",
        "PAGER=", "MANPAGER=", "LESS=",
    ] {
        if cmd.contains(needle) {
            return Some(REASON_ENV_MUTATE);
        }
    }
    None
}

// `contains_dangerous_token` was superseded by `first_dangerous_reason`,
// which returns the specific reason constant for UI surfacing.

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
    // Unsafe = command substitution (``backticks``, $(…)) or bare `&`
    // (backgrounding — non-deterministic side effects). Everything
    // else (`;` `&&` `||` `|`) is a sequence operator that we handle
    // at the segment level — each subcommand gets its own read-only
    // check, so the compound is safe iff every subcommand is.
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
            '`' if !single_quoted => return true,
            '&' if !single_quoted && !double_quoted => {
                if chars.peek() == Some(&'&') {
                    let _ = chars.next();
                } else {
                    return true;
                }
            }
            '$' if !single_quoted && chars.peek() == Some(&'(') => return true,
            _ => {}
        }
    }
    false
}

fn split_readonly_shell_segments(cmd: &str) -> Vec<String> {
    // Split on every sequence operator we treat as safe: `;`, `&&`,
    // `||`, `|`. Each resulting segment is a single command that gets
    // its own read-only check. Quote-aware so a `;` inside a quoted
    // string (e.g. `ssh host "cat foo; cat bar"`) stays attached to
    // the ssh segment for later recursive checking.
    let mut segments = Vec::new();
    let mut current = String::new();
    let mut single_quoted = false;
    let mut double_quoted = false;
    let mut escaped = false;
    let mut chars = cmd.chars().peekable();

    let push_seg = |segments: &mut Vec<String>, current: &mut String| -> bool {
        let segment = current.trim().to_owned();
        if segment.is_empty() {
            return false;
        }
        segments.push(segment);
        current.clear();
        true
    };

    while let Some(ch) = chars.next() {
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
            '&' if !single_quoted && !double_quoted && chars.peek() == Some(&'&') => {
                let _ = chars.next();
                if !push_seg(&mut segments, &mut current) {
                    return Vec::new();
                }
            }
            '|' if !single_quoted && !double_quoted => {
                // `||` and bare `|` are both treated as sequence
                // operators here. The user-facing semantics differ
                // (||=fallback, |=pipe), but for read-only
                // classification it doesn't matter: every connected
                // subcommand must be read-only.
                if chars.peek() == Some(&'|') {
                    let _ = chars.next();
                }
                if !push_seg(&mut segments, &mut current) {
                    return Vec::new();
                }
            }
            ';' if !single_quoted && !double_quoted => {
                if !push_seg(&mut segments, &mut current) {
                    return Vec::new();
                }
            }
            _ => current.push(ch),
        }
    }

    let segment = current.trim().to_owned();
    if segment.is_empty() {
        return Vec::new();
    }
    segments.push(segment);
    segments
}

fn is_readonly_bash_segment(segment: &str) -> bool {
    classify_readonly_segment(segment).is_ok()
}

fn classify_readonly_segment(segment: &str) -> Result<(), &'static str> {
    if !redirections_are_readonly(segment) {
        return Err(REASON_REDIRECT);
    }

    let tokens = shell_words(segment);
    let Some((command_idx, command)) = tokens
        .iter()
        .enumerate()
        .find(|(_, token)| !is_env_assignment(token))
    else {
        return Err(REASON_EMPTY_SEGMENT);
    };
    // If the first non-assignment token still failed `is_env_assignment`
    // because the env var name was in the deny list, we want to surface
    // that specifically. (Currently the iterator just stops at the first
    // non-assignment; tokens before it that LOOKED like `KEY=val` but had
    // a dangerous KEY would have been rejected by the raw-byte deny scan.
    // Defense-in-depth: re-check the skipped tokens.)
    for skipped in &tokens[..command_idx] {
        if !is_env_assignment(skipped) {
            // Token parsed as `name=val` but name was dangerous.
            return Err(REASON_ENV_MUTATE);
        }
    }
    let command = command.to_ascii_lowercase();
    let args = &tokens[command_idx + 1..];

    // Explicit narrow allow for `bash -n` syntax-check and `bash -V` /
    // `--version`. These are the only `bash` invocations that don't
    // execute the script body. Mirrors the GitHub Copilot CLI's
    // approach: shell wrappers are denied by default but the
    // syntax-only subset is whitelisted.
    if command.as_str() == "bash" || command.as_str() == "sh" {
        let only_inspection_flags = args.iter().all(|a| {
            matches!(a.as_str(), "-n" | "--noexec" | "-V" | "--version" | "-h" | "--help")
        });
        if !args.is_empty() && only_inspection_flags {
            return Ok(());
        }
    }

    // Hard reject: command-wrapper / REPL-from-args heads. Each of
    // these takes a string and executes it, defeating any
    // classification of the *outer* command line. Some have read-only
    // subsets (`bash -n` syntax-check, `env` with no args = print env)
    // but we deny across the board — these are the most-abused vectors
    // in published bypass chains (Flatt #3-5, GTFOBins).
    if matches!(
        command.as_str(),
        "bash" | "sh" | "dash" | "zsh" | "ksh" | "busybox" | "ash" | "fish"
        | "eval" | "exec" | "source" | "."
        | "command" | "builtin" | "enable" | "trap" | "alias" | "unalias"
        | "export" | "declare" | "typeset" | "local" | "readonly" | "unset"
        | "set" | "shopt" | "ulimit" | "umask" | "fc" | "history" | "bind"
        | "nice" | "nohup" | "setsid" | "timeout" | "time" | "coproc"
        | "xargs" | "parallel"
        | "make" | "ninja" | "just" | "cmake" | "msbuild" | "ant" | "gradle"
        | "python" | "python3" | "perl" | "ruby" | "node" | "lua" | "php" | "deno" | "bun"
        | "tar" | "zip" | "unzip" | "gzip" | "gunzip" | "bzip2" | "bunzip2" | "xz" | "unxz"
        | "7z" | "rar" | "unrar"
        | "tee" | "dd" | "rsync" | "scp" | "sftp"
        | "man"
        | "apt" | "apt-get" | "yum" | "dnf" | "pacman" | "zypper" | "brew" | "pip" | "pip3"
        | "npm" | "yarn" | "pnpm" | "cargo-install"
    ) {
        return Err(REASON_HEAD_BLOCKED);
    }
    // Per-command guards. Each arm returns the specific reason
    // constant on rejection so the UI can show *why* the command was
    // denied (e.g. "git -c flag" vs "git subcommand not in read-only set").
    match command.as_str() {
        "cd" | "pushd" | "popd" => {
            if is_readonly_cd(args) { Ok(()) } else { Err(REASON_NOT_ALLOWLISTED) }
        }
        "find" => {
            let has_write = args.iter().any(|arg| {
                // The write-side find actions: deletion, exec wrappers
                // (`-exec`/`-execdir`/`-ok`/`-okdir`), and the file-
                // writing actions documented in `find(1)` (`-fprint`,
                // `-fprintf`, `-fls` — *not* `-print`/`-printf`, which
                // only write to stdout).
                let a = arg.as_str();
                matches!(a, "-delete" | "-exec" | "-execdir" | "-ok" | "-okdir")
                    || a.starts_with("-fprint")
                    || a == "-fls"
            });
            if has_write { Err(REASON_FIND_WRITE) } else { Ok(()) }
        }
        _x if {
            // Sentinel: every remaining arm in the original match runs
            // through `match_remaining_segment` below — the match is
            // unfortunately too long to inline cleanly twice. The
            // sentinel + always-true guard short-circuits this arm so
            // it never matches and the real dispatch follows.
            false
        } => unreachable!(),
        _ => match_remaining_segment(&command, args),
    }
}

/// Continuation of the per-command guards for `classify_readonly_segment`.
/// Split out because the original arm count blew past 50 cases and
/// inline early-returns made the match unreadable.
fn match_remaining_segment(command: &str, args: &[String]) -> Result<(), &'static str> {
    match command {
        "sed" => {
            if args.iter().any(|a| a == "-i" || a.starts_with("-i.")) {
                return Err(REASON_SED_EXEC);
            }
            if args.iter().any(|a| a == "--file" || a.starts_with("--file=")) {
                return Err(REASON_SED_EXEC);
            }
            for a in args {
                if sed_script_has_e_modifier(a) {
                    return Err(REASON_SED_EXEC);
                }
            }
            Ok(())
        }
        "awk" | "gawk" | "mawk" | "nawk" => {
            if args.iter().any(|a| a == "-i" || a == "-i.") {
                return Err(REASON_AWK_EXEC);
            }
            for a in args {
                if awk_script_has_dangerous(a) {
                    return Err(REASON_AWK_EXEC);
                }
            }
            Ok(())
        }
        "git" => {
            if args.iter().any(|a| a == "-c" || a.starts_with("-c=")) {
                return Err(REASON_GIT_HOOK_RCE);
            }
            let sub_ok = args
                .iter()
                .find(|a| !a.starts_with('-'))
                .is_some_and(|subcommand| {
                    matches!(
                        subcommand.as_str(),
                        "log" | "show" | "diff" | "status" | "branch"
                        | "ls-files" | "ls-tree" | "rev-parse" | "blame"
                        | "describe" | "tag" | "remote" | "shortlog"
                        | "reflog" | "submodule" | "for-each-ref" | "cat-file"
                    )
                });
            if sub_ok { Ok(()) } else { Err(REASON_GIT_SUBCOMMAND) }
        }
        "cargo" => {
            let sub_ok = args.first().is_some_and(|subcommand| {
                matches!(
                    subcommand.as_str(),
                    "check" | "test" | "clippy" | "tree" | "metadata" | "search" | "doc" | "fmt"
                )
            });
            if sub_ok { Ok(()) } else { Err(REASON_NOT_ALLOWLISTED) }
        }
        "kubectl" => {
            let sub_ok = args.first().is_some_and(|sub| {
                matches!(
                    sub.as_str(),
                    "get" | "describe" | "logs" | "version" | "cluster-info"
                    | "explain" | "top" | "api-resources" | "api-versions" | "config"
                )
            });
            if sub_ok { Ok(()) } else { Err(REASON_NOT_ALLOWLISTED) }
        }
        "docker" | "podman" => {
            let sub_ok = args.first().is_some_and(|sub| {
                matches!(
                    sub.as_str(),
                    "ps" | "inspect" | "logs" | "version" | "info" | "images"
                    | "image" | "history" | "stats" | "top" | "diff" | "port" | "search"
                )
            });
            if sub_ok { Ok(()) } else { Err(REASON_NOT_ALLOWLISTED) }
        }
        "helm" => {
            let sub_ok = args.first().is_some_and(|sub| {
                matches!(
                    sub.as_str(),
                    "list" | "ls" | "get" | "show" | "version" | "history" | "status" | "search" | "env"
                )
            });
            if sub_ok { Ok(()) } else { Err(REASON_NOT_ALLOWLISTED) }
        }
        "terraform" | "tofu" => {
            let sub_ok = args.first().is_some_and(|sub| {
                matches!(
                    sub.as_str(),
                    "plan" | "show" | "output" | "version" | "validate" | "fmt" | "providers" | "graph"
                )
            });
            if sub_ok { Ok(()) } else { Err(REASON_NOT_ALLOWLISTED) }
        }
        "systemctl" => {
            let sub_ok = args.first().is_some_and(|sub| {
                matches!(
                    sub.as_str(),
                    "status" | "show" | "is-active" | "is-enabled" | "is-failed"
                    | "list-units" | "list-unit-files" | "list-timers" | "list-dependencies"
                    | "list-jobs" | "list-machines" | "list-sockets" | "list-paths" | "cat"
                )
            });
            if sub_ok { Ok(()) } else { Err(REASON_NOT_ALLOWLISTED) }
        }
        "journalctl" => {
            let has_write = args.iter().any(|arg| {
                matches!(
                    arg.as_str(),
                    "--vacuum-size" | "--vacuum-time" | "--vacuum-files"
                    | "--rotate" | "--flush" | "--sync" | "--relinquish-var"
                )
            });
            if has_write { Err(REASON_NOT_ALLOWLISTED) } else { Ok(()) }
        }
        "curl" => {
            let has_write = args.iter().any(|a| {
                let lower = a.to_ascii_lowercase();
                lower.starts_with("-d")
                    || lower == "--data" || lower.starts_with("--data-")
                    || lower == "-t" || lower == "--upload-file"
                    || lower == "-f" || lower == "--form"
                    || lower == "-o" || lower == "--output"
                    || lower == "--cookie-jar"
                    || (lower == "-x" || lower == "--request")
            });
            if has_write { Err(REASON_CURL_WRITE) } else { Ok(()) }
        }
        "wget" => {
            let ok = args.iter().any(|a| a == "--spider")
                || args.windows(2).any(|w| w[0] == "-O" && w[1] == "-");
            if ok { Ok(()) } else { Err(REASON_WGET_WRITE) }
        }
        "ssh" => is_readonly_ssh_reasoned(args),
        "sudo" | "doas" => is_readonly_sudo_reasoned(args),
        "dig" | "host" | "nslookup" | "getent" | "whois"
        | "ping" | "ping6" | "traceroute" | "traceroute6" | "tracepath" | "mtr"
        | "ip" | "ifconfig" | "ss" | "netstat" | "ip6" | "arp" | "route" => Ok(()),
        "nc" | "ncat" | "socat" => {
            let ok = args.iter().any(|a| a == "-z" || a.starts_with("-z"));
            if ok { Ok(()) } else { Err(REASON_NOT_ALLOWLISTED) }
        }
        // System inspection — explicitly read-only across the board.
        "top" | "htop" | "btop" | "iotop" | "iostat" | "vmstat" | "mpstat" | "sar"
        | "lsmod" | "lspci" | "lsusb" | "lsblk" | "lscpu" | "lshw" | "dmidecode"
        | "uptime" | "w" | "users" | "last" | "lastlog" | "groups"
        | "uname" | "hostname" | "id" | "whoami" | "pwd" | "tty" | "stty"
        | "env" | "printenv" | "locale"
        | "ps" | "pgrep" | "pidof"
        | "free" | "df" | "du"
        | "lsof" | "fuser"
        | "ldd" | "objdump" | "nm" | "readelf" | "size" | "strings" | "file"
        | "hexdump" | "xxd" | "od" | "base64" => Ok(()),
        // Plain inspection / piping.
        "ls" | "cat" | "tac" | "head" | "tail" | "less" | "more" | "bat"
        | "grep" | "egrep" | "fgrep" | "rg" | "ack" | "fd"
        | "wc" | "stat" | "which" | "type" | "command" | "whereis"
        | "echo" | "printf" | "yes" | "true" | "false"
        | "date" | "cal" | "bc" | "expr"
        | "tree" | "diff" | "cmp" | "comm"
        | "sort" | "uniq" | "cut" | "paste" | "join" | "tr" | "rev" | "fold" | "expand" | "unexpand"
        | "jq" | "yq" | "tomlq" | "xq" | "dasel" | "miller" | "mlr" | "csvkit" => Ok(()),
        _ => Err(REASON_NOT_ALLOWLISTED),
    }
}

/// Result-returning twin of `is_readonly_ssh`.
fn is_readonly_ssh_reasoned(args: &[String]) -> Result<(), &'static str> {
    for a in args {
        let lower = a.to_ascii_lowercase();
        if matches!(
            lower.as_str(),
            "-l" | "-r" | "-d" | "-w" | "-a" | "-x" | "-y" | "-m" | "-n" | "-q" | "-tt"
        ) || lower.starts_with("-l")
            || lower.starts_with("-r")
            || lower.starts_with("-d")
            || lower.starts_with("-w=")
        {
            return Err(REASON_SSH_FORWARD);
        }
    }
    let mut iter = args.iter().peekable();
    let mut host: Option<&str> = None;
    while let Some(a) = iter.next() {
        if a.starts_with('-') {
            if matches!(a.as_str(), "-i" | "-p" | "-o" | "-F" | "-c" | "-J" | "-b" | "-B") {
                let _ = iter.next();
            }
            continue;
        }
        host = Some(a.as_str());
        break;
    }
    if host.is_none() {
        return Err(REASON_SSH_INTERACTIVE);
    }
    let remote: String = iter.map(|s| s.as_str()).collect::<Vec<_>>().join(" ");
    if remote.trim().is_empty() {
        return Err(REASON_SSH_INTERACTIVE);
    }
    classify_readonly_bash(&remote)
}

/// Result-returning twin of `is_readonly_sudo`.
fn is_readonly_sudo_reasoned(args: &[String]) -> Result<(), &'static str> {
    let mut iter = args.iter().peekable();
    while let Some(a) = iter.peek() {
        if matches!(a.as_str(), "-u" | "-g" | "-h" | "-U") {
            let _ = iter.next();
            let _ = iter.next();
            continue;
        }
        if a.starts_with('-') {
            let _ = iter.next();
            continue;
        }
        break;
    }
    let elevated: String = iter.map(|s| s.as_str()).collect::<Vec<_>>().join(" ");
    if elevated.trim().is_empty() {
        return Err(REASON_SUDO_BARE);
    }
    classify_readonly_bash(&elevated)
}

/// Is `ssh <args>` a read-only invocation? Allowed when args resolve
/// to `host` + a single quoted command string AND that command itself
/// classifies as read-only via the same pipeline. Anything more
/// complex (port-forward setup `-L`/`-R`/`-D`, agent forwarding `-A`,
/// `scp`-like or file-transfer wrappers) is denied — those have
/// write-side effects (open listening sockets / mutate ssh-agent
/// state / copy files).
fn is_readonly_ssh(args: &[String]) -> bool {
    // Reject if any port-forwarding or write-mode flag is present.
    for a in args {
        let lower = a.to_ascii_lowercase();
        if matches!(
            lower.as_str(),
            "-l" | "-r" | "-d" | "-w" | "-a" | "-x" | "-y" | "-m" | "-n" | "-q" | "-tt"
        ) || lower.starts_with("-l")
            || lower.starts_with("-r")
            || lower.starts_with("-d")
            || lower.starts_with("-w=")
        {
            return false;
        }
    }
    // Find the remote command — the first arg that's not a flag and not
    // the host. Skip flag args (-i KEYFILE, -p PORT, -o OPT take a value
    // — we accept those silently because they don't change classification).
    let mut iter = args.iter().peekable();
    let mut host: Option<&str> = None;
    while let Some(a) = iter.next() {
        if a.starts_with('-') {
            // Skip flags that take a value.
            if matches!(a.as_str(), "-i" | "-p" | "-o" | "-F" | "-c" | "-J" | "-b" | "-B") {
                let _ = iter.next();
            }
            continue;
        }
        host = Some(a.as_str());
        break;
    }
    if host.is_none() {
        return false;
    }
    // Anything left is the remote command. Join + recursively classify.
    let remote: String = iter.map(|s| s.as_str()).collect::<Vec<_>>().join(" ");
    if remote.trim().is_empty() {
        // Bare `ssh host` opens an interactive shell — not classifiable.
        return false;
    }
    is_readonly_bash(&remote)
}

/// Is `sudo <args>` a read-only invocation? Recursively classifies the
/// command after stripping `-u USER`, `-E`, `-n`, `-S` flags.
fn is_readonly_sudo(args: &[String]) -> bool {
    let mut iter = args.iter().peekable();
    while let Some(a) = iter.peek() {
        if matches!(a.as_str(), "-u" | "-g" | "-h" | "-U") {
            let _ = iter.next();
            let _ = iter.next(); // value
            continue;
        }
        if a.starts_with('-') {
            let _ = iter.next();
            continue;
        }
        break;
    }
    let elevated: String = iter.map(|s| s.as_str()).collect::<Vec<_>>().join(" ");
    if elevated.trim().is_empty() {
        return false;
    }
    is_readonly_bash(&elevated)
}

fn is_readonly_cd(args: &[String]) -> bool {
    let mut positional = 0;
    let mut after_double_dash = false;

    for arg in args {
        if !after_double_dash && arg == "--" {
            after_double_dash = true;
            continue;
        }
        if !after_double_dash && arg.starts_with('-') && arg != "-" {
            return false;
        }
        positional += 1;
        if positional > 1 {
            return false;
        }
    }

    true
}

fn redirections_are_readonly(segment: &str) -> bool {
    // A redirect is read-only if its target is `/dev/null` (discard)
    // OR a file-descriptor reference (`&1`, `&2`, `&-`) — that's just
    // re-routing the existing streams, no new file is opened for
    // writing. Heredocs (`<<`) and process substitutions (`<()`/`>()`)
    // are blocked separately (the latter via `has_unsafe_shell_control`'s
    // bare-`&` check).
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
                // `>>` (append) is just as readonly-safe as `>` —
                // the safety check is on the target, not the mode.
                if target_start < bytes.len() && bytes[target_start] == b'>' {
                    target_start += 1;
                }
                // `>&N` form: ampersand-fd shorthand, no whitespace.
                if target_start < bytes.len() && bytes[target_start] == b'&' {
                    target_start += 1;
                    let Some((target, next)) = shell_token_at(segment, target_start) else {
                        return false;
                    };
                    if !is_fd_target(&target) {
                        return false;
                    }
                    i = next;
                    continue;
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

/// Is a redirect target a file-descriptor reference? `1`, `2`, `-`
/// (close), or any digit string. `2>&1` parses with target_start
/// pointing past the `&`, so the token here is just the digit.
fn is_fd_target(t: &str) -> bool {
    if t == "-" {
        return true;
    }
    !t.is_empty() && t.chars().all(|c| c.is_ascii_digit())
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
    if name.is_empty()
        || !name
            .chars()
            .all(|ch| ch == '_' || ch.is_ascii_alphanumeric())
        || !name
            .chars()
            .next()
            .is_some_and(|ch| ch == '_' || ch.is_ascii_alphabetic())
    {
        return false;
    }
    // Reject the env-prefix form for variables that mutate child-
    // process behavior in dangerous ways. `contains_dangerous_token`
    // catches most of these via raw-byte match, but this enforces it
    // at the per-token level too as defense-in-depth (the `LD_PRELOAD=`
    // entry there is `LD_PRELOAD=` literally, and only matches when
    // followed by something — empty `LD_PRELOAD=` somehow ends up not
    // hitting the substring, this still rejects it).
    const DENY_ENV: &[&str] = &[
        "LD_PRELOAD", "LD_AUDIT", "LD_LIBRARY_PATH",
        "BASH_ENV", "ENV", "PROMPT_COMMAND",
        "PS0", "PS1", "PS2", "PS3", "PS4",
        "IFS", "PATH", "SHELL", "BASH",
        "GIT_EXTERNAL_DIFF", "GIT_PAGER", "GIT_SSH_COMMAND", "GIT_DIR",
        "PAGER", "MANPAGER", "LESS", "MANROFFSEQ",
    ];
    !DENY_ENV.iter().any(|d| name == *d)
}

/// Does a `sed` script argument use the `/e` modifier (which executes
/// the substitution as a shell command — Flatt #5)? Walks the script
/// looking for `s/.../.../[gIimMe]+` where `e` appears in the
/// modifier set.
fn sed_script_has_e_modifier(script: &str) -> bool {
    // Strip a leading `-e`/`-E`/`--expression=` if present.
    let body = script
        .strip_prefix("--expression=")
        .or_else(|| {
            if script == "-e" || script == "-E" {
                Some("")
            } else {
                None
            }
        })
        .unwrap_or(script);
    // Quick reject: no `s` and no `e` flag possible.
    if !body.contains("s/") && !body.contains("s|") {
        return false;
    }
    // Naive scan: any `s<sep>…<sep>…<sep>…e…` pattern with `e` in the
    // modifier tail. Conservative: if we can't parse it, reject.
    let mut chars = body.chars().peekable();
    while let Some(c) = chars.next() {
        if c != 's' {
            continue;
        }
        let Some(&sep) = chars.peek() else { return false; };
        if !"/|#@!,".contains(sep) {
            continue;
        }
        // Past `s<sep>`, scan to find third separator → flags.
        let _ = chars.next(); // consume sep
        let mut seps_seen = 1;
        let mut escape = false;
        while let Some(c2) = chars.next() {
            if escape {
                escape = false;
                continue;
            }
            if c2 == '\\' {
                escape = true;
                continue;
            }
            if c2 == sep {
                seps_seen += 1;
                if seps_seen == 3 {
                    // Read flag chars until end of script or whitespace.
                    let mut flags = String::new();
                    while let Some(&fc) = chars.peek() {
                        if fc.is_whitespace() || fc == ';' || fc == '\n' {
                            break;
                        }
                        flags.push(fc);
                        let _ = chars.next();
                    }
                    if flags.contains('e') || flags.contains('w') {
                        // `w` writes to a file — also a write side-effect.
                        return true;
                    }
                    break;
                }
            }
        }
    }
    false
}

/// Does an `awk` script use `system(`, `getline … | cmd`, `print >`,
/// `print |`, or `getline … <` against `/dev/tcp/`-like targets? Any
/// of those make awk a shell.
fn awk_script_has_dangerous(script: &str) -> bool {
    // Match on the raw script bytes — quote stripping already
    // happened at tokenization, so what we see is the program text.
    let lower = script.to_ascii_lowercase();
    for needle in [
        "system(", "getline",
        "print>", "print >", "print|", "print |",
        "printf>", "printf >", "printf|", "printf |",
        "| getline", "|getline",
    ] {
        if lower.contains(needle) {
            return true;
        }
    }
    false
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
