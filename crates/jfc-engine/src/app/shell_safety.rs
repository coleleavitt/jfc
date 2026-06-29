//! Read-only bash-command classifier — Plan-mode safety parser.
//!
//! Extracted from app/permissions.rs to separate two cohesive but unrelated
//! concerns: this module is a self-contained command-line classifier that
//! decides whether a given bash invocation is read-only (so Plan mode can
//! auto-approve it). The actual permission-mode policy lives in
//! `super::permissions` and calls `classify_readonly_bash` from here.
//!
//! Hardened against the bypass classes documented at CVE-2025-54795
//! (Cymulate `echo` quote-escape), CVE-2025-66032 (Flatt 8-bypass chain),
//! and the broader CWE-78 surface. Layered:
//!
//!   1. **Raw-byte deny list** — pattern strings no safe command should
//!      contain (`/dev/tcp/`, `${IFS}`, `@P}`, `<<<`, dangerous
//!      env-prefixes, …). Matched on unescaped bytes.
//!   2. **Shell-control reject** — backticks, `$(…)`, bare `&`.
//!   3. **Segment splitter** — `;`/`&&`/`||`/`|` separate independent
//!      commands; each segment is classified independently.
//!   4. **Per-segment classifier** — head command must be in the positive
//!      allowlist; flags must satisfy the per-tool guards (`find`
//!      write-actions, `sed -i`, `git -c`, etc.).
pub fn is_readonly_bash(cmd: &str) -> bool {
    classify_readonly_bash(cmd).is_ok()
}

/// Like [`is_readonly_bash`] but returns the specific deny reason so
/// the UI can surface *which* layer rejected the command. Each `Err`
/// variant is a `&'static str` suitable for display in toasts /
/// status badges / approval dialogs.
pub fn classify_readonly_bash(cmd: &str) -> Result<(), &'static str> {
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
        classify_readonly_segment(segment)?;
    }
    Ok(())
}

// ─── Reason constants ─────────────────────────────────────────────
// Surfaced verbatim in the UI denial badge so the user can see the
// exact reason a command was rejected (mirrors v126's denial-reason
// surfacing). All `&'static str` so `PermissionDecision::Denied`
// keeps its zero-allocation contract.

pub const REASON_MULTILINE: &str =
    "Plan mode: multi-line command without continuation (use `|` or `\\`)";
pub const REASON_SHELL_CONTROL: &str =
    "Plan mode: command substitution / backgrounding not allowed";
pub const REASON_EMPTY_SEGMENT: &str = "Plan mode: empty command segment";
pub const REASON_DEV_TCP: &str =
    "Plan mode: /dev/tcp /dev/udp pseudo-devices (network exfiltration)";
pub const REASON_ENV_MUTATE: &str =
    "Plan mode: dangerous env-prefix (LD_PRELOAD / BASH_ENV / PROMPT_COMMAND / …)";
pub const REASON_PARAM_MUTATE: &str =
    "Plan mode: parameter-expansion mutation (${var:=…} / ${var@P})";
pub const REASON_HEREDOC: &str =
    "Plan mode: heredoc / herestring (body may contain command substitution)";
pub const REASON_PROCESS_SUB: &str = "Plan mode: process substitution <(…) / >(…)";
pub const REASON_LONG_OPT_RCE: &str =
    "Plan mode: long-option RCE vector (--pre / --checkpoint-action / --html=…)";
pub const REASON_HEAD_BLOCKED: &str =
    "Plan mode: shell wrapper / REPL-from-args head (bash -c / eval / xargs / make / …)";
pub const REASON_NOT_ALLOWLISTED: &str = "Plan mode: command not in read-only allowlist";
pub const REASON_FIND_WRITE: &str =
    "Plan mode: find with write action (-delete / -exec / -fprint / -fls)";
pub const REASON_SED_EXEC: &str = "Plan mode: sed with `e` modifier / `w` write / `-i` in-place";
pub const REASON_AWK_EXEC: &str = "Plan mode: awk script with system() / getline / print-to-file";
pub const REASON_GIT_HOOK_RCE: &str = "Plan mode: `git -c` flag (pager/editor/sshCommand RCE)";
pub const REASON_GIT_SUBCOMMAND: &str = "Plan mode: git subcommand not in read-only set";
pub const REASON_CURL_WRITE: &str =
    "Plan mode: curl with write flag (-X / -d / --data / -T / -o / -F)";
pub const REASON_WGET_WRITE: &str = "Plan mode: wget without --spider (writes to disk)";
pub const REASON_SSH_FORWARD: &str = "Plan mode: ssh with port-forward / agent-forward flag";
pub const REASON_SSH_INTERACTIVE: &str = "Plan mode: ssh without an explicit remote command";
pub const REASON_SUDO_BARE: &str = "Plan mode: sudo / doas without a command to elevate";
pub const REASON_REDIRECT: &str = "Plan mode: redirect target is not /dev/null or another FD";
pub const REASON_FIND_NO_ACTION: &str =
    "Plan mode: find without any allowlisted action (or with unknown flag)";

/// First-match raw-byte deny scan; returns the reason constant for
/// whichever class triggered, or `None` if the bytes are clean.
fn first_dangerous_reason(cmd: &str) -> Option<&'static str> {
    if cmd.contains("/dev/tcp/") || cmd.contains("/dev/udp/") {
        return Some(REASON_DEV_TCP);
    }
    if cmd.contains("${IFS")
        || cmd.contains("${!")
        || cmd.contains(":=")
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
        "--html=",
        "--pager=",
        "--compress-program=",
        "--use-compress-program=",
        "--preprocessor=",
        "--pre=",
        "--checkpoint-action=",
        "--unzip-command=",
        "--rsh=",
        "--upload-pack=",
        "--receive-pack=",
        "--exec-path=",
    ] {
        if cmd.contains(needle) {
            return Some(REASON_LONG_OPT_RCE);
        }
    }
    for needle in [
        "LD_PRELOAD=",
        "LD_AUDIT=",
        "LD_LIBRARY_PATH=",
        "BASH_ENV=",
        "ENV=",
        "PROMPT_COMMAND=",
        "GIT_EXTERNAL_DIFF=",
        "GIT_PAGER=",
        "GIT_SSH_COMMAND=",
        "PAGER=",
        "MANPAGER=",
        "LESS=",
        "PATH=",
        "IFS=",
        "SHELL=",
    ] {
        if cmd.contains(needle) {
            return Some(REASON_ENV_MUTATE);
        }
    }
    None
}

// ─── Catastrophic-command backstop ────────────────────────────────────────
//
// `classify_readonly_bash` above answers "is this safe to AUTO-RUN in Plan
// mode?" — a strict allowlist. This is the opposite end: a tiny *denylist* of
// commands so destructive that we want a confirmation prompt **even in
// BypassPermissions / Auto**, where everything is normally auto-approved.
//
// Scope is deliberately narrow — only genuinely unrecoverable, whole-system
// or whole-history loss. A 305-session forensic audit found ZERO of these
// fired in practice (every `rm -rf` targeted /tmp, build artifacts, or
// worktree dirs; every `reset --hard` was `HEAD` inside a merge-abort script;
// every force-push was a solo-repo `--amend` iteration). So this backstop is
// pure insurance: it should essentially never trip on real usage, and when it
// does, the operation really is the catastrophic kind. Legit swarm cleanup
// (`worktree remove --force`, `branch -D`, `reset --hard HEAD`,
// `--force-with-lease`, `/tmp` deletes) is explicitly NOT catastrophic, so
// headless/background agents never deadlock waiting on an approval nobody can
// give. Override with `JFC_ALLOW_CATASTROPHIC_BASH=1` for unattended runs that
// genuinely need it.

/// Human-readable reason a command was flagged catastrophic. Surfaced in the
/// approval prompt so the user sees *why* this bypassed the bypass.
pub const REASON_CATASTROPHIC_RM: &str =
    "destructive: recursive delete of a root / home / system path";
pub const REASON_CATASTROPHIC_DISK: &str =
    "destructive: raw disk write / filesystem format (dd / mkfs / shred of a device)";
pub const REASON_CATASTROPHIC_FORKBOMB: &str = "destructive: fork bomb";
pub const REASON_CATASTROPHIC_FORCE_PUSH: &str =
    "destructive: force-push over master/main (use --force-with-lease)";
pub const REASON_CATASTROPHIC_GIT_WIPE: &str =
    "destructive: removing .git (deletes repository history)";

/// Returns `Some(reason)` if `cmd` contains a catastrophic, effectively
/// unrecoverable operation that should prompt even under BypassPermissions.
/// `None` for everything else (including ordinary destructive ops like a
/// project-local `rm -rf target` or `git reset --hard HEAD`, which are the
/// caller's normal Default-mode prompt territory, not this backstop's).
pub fn catastrophic_bash_reason(cmd: &str) -> Option<&'static str> {
    if std::env::var("JFC_ALLOW_CATASTROPHIC_BASH")
        .ok()
        .is_some_and(|v| !v.is_empty() && v != "0" && !v.eq_ignore_ascii_case("false"))
    {
        return None;
    }
    // Normalize whitespace runs to single spaces so `rm   -rf` matches.
    let norm: String = cmd.split_whitespace().collect::<Vec<_>>().join(" ");

    if let Some(r) = catastrophic_rm_reason(&norm) {
        return Some(r);
    }
    // Raw-disk / format. `dd ... of=/dev/sdX`, `mkfs`, `shred /dev/...`.
    if (norm.contains("dd ") && norm.contains("of=/dev/"))
        || norm.contains("mkfs")
        || (norm.contains("shred ") && norm.contains("/dev/"))
    {
        return Some(REASON_CATASTROPHIC_DISK);
    }
    // Classic fork bomb `:(){ :|:& };:` (and whitespace variants).
    let despaced: String = cmd.chars().filter(|c| !c.is_whitespace()).collect();
    if despaced.contains(":(){:|:") || despaced.contains(":(){:|:&};:") {
        return Some(REASON_CATASTROPHIC_FORKBOMB);
    }
    // Force-push over the primary branch. `--force-with-lease` is the SAFE
    // variant (refuses if the remote moved) — never flagged. Accept both the
    // long `--force` and the short `-f` flag (`git push -f origin main`).
    let has_force_flag = (norm.contains("--force") && !norm.contains("--force-with-lease"))
        || norm.split(' ').any(|t| t == "-f");
    if norm.contains("git push") && has_force_flag {
        let targets_primary = norm.contains(" master")
            || norm.contains(" main")
            || norm.contains(":master")
            || norm.contains(":main")
            || norm.contains("HEAD:master")
            || norm.contains("HEAD:main");
        // A bare `git push --force` (no refspec) pushes the current branch —
        // catastrophic only when that's the primary branch, which we can't
        // know statically; treat an explicit master/main mention as the
        // trigger and leave bare force-push to the normal Default prompt.
        if targets_primary {
            return Some(REASON_CATASTROPHIC_FORCE_PUSH);
        }
    }
    // `rm -rf .git` (or `.git/`) — wipes repository history. The forensic
    // tulip case (flattening a fresh scaffold) is rare; prompting once is a
    // fair price for protecting real history.
    if is_recursive_rm(&norm)
        && (norm.contains(" .git ")
            || norm.contains(" .git/")
            || norm.ends_with(" .git")
            || norm.contains("/.git "))
    {
        return Some(REASON_CATASTROPHIC_GIT_WIPE);
    }
    None
}

/// True if `norm` (whitespace-collapsed) contains a recursive+force `rm`.
fn is_recursive_rm(norm: &str) -> bool {
    // Match `rm -rf`, `rm -fr`, `rm -r -f`, `rm --recursive --force`, etc.
    let mut tokens = norm.split(' ').peekable();
    while let Some(t) = tokens.next() {
        if t != "rm" {
            continue;
        }
        let mut recursive = false;
        let mut force = false;
        for a in tokens.clone() {
            if !a.starts_with('-') {
                break;
            }
            if a == "--recursive" {
                recursive = true;
            }
            if a == "--force" {
                force = true;
            }
            if a.starts_with('-') && !a.starts_with("--") {
                if a.contains('r') || a.contains('R') {
                    recursive = true;
                }
                if a.contains('f') {
                    force = true;
                }
            }
        }
        if recursive && force {
            return true;
        }
    }
    false
}

/// Reason if a recursive `rm` targets a root / home / system path or a bare
/// glob. Project-relative and `/tmp` targets return `None`.
fn catastrophic_rm_reason(norm: &str) -> Option<&'static str> {
    if !is_recursive_rm(norm) {
        return None;
    }
    // Pull the argument tokens after `rm`'s flags and test each target. Use the
    // shell tokenizer so quoted `$HOME` forms normalize the same way as bare
    // arguments.
    let tokens = shell_words(norm);
    let mut tokens = tokens.iter().map(String::as_str).peekable();
    while let Some(t) = tokens.next() {
        if t != "rm" {
            continue;
        }
        for arg in tokens.by_ref() {
            if arg.starts_with('-') {
                continue; // flag
            }
            if is_catastrophic_rm_target(arg) {
                return Some(REASON_CATASTROPHIC_RM);
            }
        }
        break;
    }
    None
}

/// A single `rm` target path is catastrophic if it's a filesystem root, the
/// home dir, a top-level system dir, or a bare `*` / `.` / `~`. `/tmp/...`
/// and project-relative paths are safe.
fn is_catastrophic_rm_target(arg: &str) -> bool {
    // Strip a trailing slash for comparison.
    let p = arg.trim_end_matches('/');
    let lower = p.to_ascii_lowercase();
    // Bare glob / cwd / home with nothing after it.
    if matches!(arg, "*" | "." | ".." | "~" | "/" | "/*" | "~/*") {
        return true;
    }
    // Absolute system roots and their globs.
    const SYSTEM_ROOTS: &[&str] = &[
        "/bin", "/boot", "/dev", "/etc", "/home", "/lib", "/lib64", "/opt", "/proc", "/root",
        "/run", "/sbin", "/srv", "/sys", "/usr", "/var",
    ];
    for root in SYSTEM_ROOTS {
        // `/etc`, `/etc/`, `/etc/*`, `/home` exactly — but NOT a deep,
        // specific path like `/home/cole/RustProjects/x/target` which is a
        // legitimate targeted delete.
        if p == *root || arg.strip_prefix(root).is_some_and(|rest| rest == "/*") {
            return true;
        }
        // `/home/<user>` with no further path component is whole-home.
        if *root == "/home"
            && let Some(rest) = p.strip_prefix("/home/")
            && !rest.is_empty()
            && (!rest.contains('/')
                || rest
                    .strip_suffix("/*")
                    .is_some_and(|user| !user.is_empty() && !user.contains('/')))
        {
            return true; // /home/<user> or /home/<user>/*
        }
    }
    // `$HOME` / `~` with no sub-path.
    if matches!(
        lower.as_str(),
        "$home" | "${home}" | "~" | "$home/*" | "${home}/*" | "~/*"
    ) {
        return true;
    }
    false
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
        let has_inspection_flag = args.iter().any(|a| {
            matches!(
                a.as_str(),
                "-n" | "--noexec" | "-V" | "--version" | "-h" | "--help"
            )
        });
        let no_exec_flag = !args
            .iter()
            .any(|a| matches!(a.as_str(), "-c" | "-i" | "-l" | "--login" | "-s"));
        if !args.is_empty() && has_inspection_flag && no_exec_flag {
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
        "bash"
            | "sh"
            | "dash"
            | "zsh"
            | "ksh"
            | "busybox"
            | "ash"
            | "fish"
            | "eval"
            | "exec"
            | "source"
            | "."
            | "command"
            | "builtin"
            | "enable"
            | "trap"
            | "alias"
            | "unalias"
            | "export"
            | "declare"
            | "typeset"
            | "local"
            | "readonly"
            | "unset"
            | "set"
            | "shopt"
            | "ulimit"
            | "umask"
            | "fc"
            | "history"
            | "bind"
            | "nice"
            | "nohup"
            | "setsid"
            | "timeout"
            | "time"
            | "coproc"
            | "xargs"
            | "parallel"
            | "make"
            | "ninja"
            | "just"
            | "cmake"
            | "msbuild"
            | "ant"
            | "gradle"
            | "python"
            | "python3"
            | "perl"
            | "ruby"
            | "node"
            | "lua"
            | "php"
            | "deno"
            | "bun"
            | "tar"
            | "zip"
            | "unzip"
            | "gzip"
            | "gunzip"
            | "bzip2"
            | "bunzip2"
            | "xz"
            | "unxz"
            | "7z"
            | "rar"
            | "unrar"
            | "tee"
            | "dd"
            | "rsync"
            | "scp"
            | "sftp"
            | "man"
            | "apt"
            | "apt-get"
            | "yum"
            | "dnf"
            | "pacman"
            | "zypper"
            | "brew"
            | "pip"
            | "pip3"
            | "npm"
            | "yarn"
            | "pnpm"
            | "cargo-install"
    ) {
        return Err(REASON_HEAD_BLOCKED);
    }
    // Per-command guards. Each arm returns the specific reason
    // constant on rejection so the UI can show *why* the command was
    // denied (e.g. "git -c flag" vs "git subcommand not in read-only set").
    match command.as_str() {
        "cd" | "pushd" | "popd" => {
            if is_readonly_cd(args) {
                Ok(())
            } else {
                Err(REASON_NOT_ALLOWLISTED)
            }
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
            if has_write {
                Err(REASON_FIND_WRITE)
            } else {
                Ok(())
            }
        }
        _x if {
            // Sentinel: every remaining arm in the original match runs
            // through `match_remaining_segment` below — the match is
            // unfortunately too long to inline cleanly twice. The
            // sentinel + always-true guard short-circuits this arm so
            // it never matches and the real dispatch follows.
            false
        } =>
        {
            unreachable!()
        }
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
            if args
                .iter()
                .any(|a| a == "--file" || a.starts_with("--file="))
            {
                return Err(REASON_SED_EXEC);
            }
            // Only inspect actual script expressions, not file paths.
            // sed syntax: sed [flags] [-e script]... [script] [file...]
            // If no -e was given, the first non-flag arg is the script.
            let mut scripts: Vec<&str> = Vec::new();
            let mut i = 0;
            let mut explicit_script = false;
            while i < args.len() {
                let a = &args[i];
                if a == "-e" || a == "-E" || a == "--expression" {
                    explicit_script = true;
                    if i + 1 < args.len() {
                        scripts.push(&args[i + 1]);
                        i += 2;
                    } else {
                        i += 1;
                    }
                } else if let Some(expr) = a.strip_prefix("--expression=") {
                    explicit_script = true;
                    scripts.push(expr);
                    i += 1;
                } else if a == "-n"
                    || a == "--quiet"
                    || a == "--silent"
                    || a == "-l"
                    || a.starts_with("-n") && a.len() <= 3
                {
                    i += 1;
                } else if a.starts_with('-') {
                    // Other flags (e.g. -E for extended regex)
                    i += 1;
                } else {
                    // First non-flag arg: it's a script only if no -e was given
                    if !explicit_script && scripts.is_empty() {
                        scripts.push(a);
                    }
                    // Remaining args are file paths — don't inspect them
                    break;
                }
            }
            for s in &scripts {
                if sed_script_has_e_modifier(s) {
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
            if is_readonly_git(args) {
                Ok(())
            } else {
                Err(REASON_GIT_SUBCOMMAND)
            }
        }
        "cargo" => {
            let sub_ok = args
                .first()
                .is_some_and(|subcommand| match subcommand.as_str() {
                    "check" | "test" | "clippy" | "tree" | "metadata" | "search" | "doc" => true,
                    "fmt" => args.iter().skip(1).any(|arg| arg == "--check"),
                    _ => false,
                });
            if sub_ok {
                Ok(())
            } else {
                Err(REASON_NOT_ALLOWLISTED)
            }
        }
        "kubectl" => {
            let sub_ok = args.first().is_some_and(|sub| {
                matches!(
                    sub.as_str(),
                    "get"
                        | "describe"
                        | "logs"
                        | "version"
                        | "cluster-info"
                        | "explain"
                        | "top"
                        | "api-resources"
                        | "api-versions"
                        | "config"
                )
            });
            if sub_ok {
                Ok(())
            } else {
                Err(REASON_NOT_ALLOWLISTED)
            }
        }
        "docker" | "podman" => {
            let sub_ok = args.first().is_some_and(|sub| {
                matches!(
                    sub.as_str(),
                    "ps" | "inspect"
                        | "logs"
                        | "version"
                        | "info"
                        | "images"
                        | "image"
                        | "history"
                        | "stats"
                        | "top"
                        | "diff"
                        | "port"
                        | "search"
                )
            });
            if sub_ok {
                Ok(())
            } else {
                Err(REASON_NOT_ALLOWLISTED)
            }
        }
        "helm" => {
            let sub_ok = args.first().is_some_and(|sub| {
                matches!(
                    sub.as_str(),
                    "list"
                        | "ls"
                        | "get"
                        | "show"
                        | "version"
                        | "history"
                        | "status"
                        | "search"
                        | "env"
                )
            });
            if sub_ok {
                Ok(())
            } else {
                Err(REASON_NOT_ALLOWLISTED)
            }
        }
        "terraform" | "tofu" => {
            let sub_ok = args.first().is_some_and(|sub| match sub.as_str() {
                "plan" | "show" | "output" | "version" | "validate" | "providers" | "graph" => true,
                "fmt" => args
                    .iter()
                    .skip(1)
                    .any(|arg| matches!(arg.as_str(), "-check" | "--check")),
                _ => false,
            });
            if sub_ok {
                Ok(())
            } else {
                Err(REASON_NOT_ALLOWLISTED)
            }
        }
        "systemctl" => {
            let sub_ok = args.first().is_some_and(|sub| {
                matches!(
                    sub.as_str(),
                    "status"
                        | "show"
                        | "is-active"
                        | "is-enabled"
                        | "is-failed"
                        | "list-units"
                        | "list-unit-files"
                        | "list-timers"
                        | "list-dependencies"
                        | "list-jobs"
                        | "list-machines"
                        | "list-sockets"
                        | "list-paths"
                        | "cat"
                )
            });
            if sub_ok {
                Ok(())
            } else {
                Err(REASON_NOT_ALLOWLISTED)
            }
        }
        "journalctl" => {
            let has_write = args.iter().any(|arg| {
                matches!(
                    arg.as_str(),
                    "--vacuum-size"
                        | "--vacuum-time"
                        | "--vacuum-files"
                        | "--rotate"
                        | "--flush"
                        | "--sync"
                        | "--relinquish-var"
                )
            });
            if has_write {
                Err(REASON_NOT_ALLOWLISTED)
            } else {
                Ok(())
            }
        }
        "curl" => {
            let has_write = args.iter().any(|a| {
                let lower = a.to_ascii_lowercase();
                lower.starts_with("-d")
                    || lower == "--data"
                    || lower.starts_with("--data-")
                    || lower.starts_with("-t")
                    || lower == "--upload-file"
                    || lower.starts_with("--upload-file=")
                    || lower == "-c"
                    || lower.starts_with("-f")
                    || lower == "--form"
                    || lower.starts_with("-o")
                    || lower == "--output"
                    || lower.starts_with("--output=")
                    || lower == "--cookie-jar"
                    || lower.starts_with("--cookie-jar=")
                    || lower == "-d"
                    || lower == "--dump-header"
                    || lower.starts_with("--dump-header=")
                    || lower.starts_with("-x")
                    || lower == "--request"
                    || lower.starts_with("--request=")
            });
            if has_write {
                Err(REASON_CURL_WRITE)
            } else {
                Ok(())
            }
        }
        "wget" => {
            let ok = args.iter().any(|a| a == "--spider")
                || args.windows(2).any(|w| w[0] == "-O" && w[1] == "-");
            if ok { Ok(()) } else { Err(REASON_WGET_WRITE) }
        }
        "ssh" => is_readonly_ssh_reasoned(args),
        "sudo" | "doas" => is_readonly_sudo_reasoned(args),
        "dig" | "host" | "nslookup" | "getent" | "whois" | "ping" | "ping6" | "traceroute"
        | "traceroute6" | "tracepath" | "mtr" | "ip" | "ifconfig" | "ss" | "netstat" | "ip6"
        | "arp" | "route" => Ok(()),
        "nc" | "ncat" | "socat" => {
            let ok = args.iter().any(|a| a == "-z" || a.starts_with("-z"));
            if ok {
                Ok(())
            } else {
                Err(REASON_NOT_ALLOWLISTED)
            }
        }
        // System inspection — explicitly read-only across the board.
        "top" | "htop" | "btop" | "iotop" | "iostat" | "vmstat" | "mpstat" | "sar" | "lsmod"
        | "lspci" | "lsusb" | "lsblk" | "lscpu" | "lshw" | "dmidecode" | "uptime" | "w"
        | "users" | "last" | "lastlog" | "groups" | "uname" | "hostname" | "id" | "whoami"
        | "pwd" | "tty" | "stty" | "env" | "printenv" | "locale" | "ps" | "pgrep" | "pidof"
        | "free" | "df" | "du" | "lsof" | "fuser" | "ldd" | "objdump" | "nm" | "readelf"
        | "size" | "strings" | "file" | "hexdump" | "xxd" | "od" | "base64" => Ok(()),
        // Plain inspection / piping.
        "ls" | "cat" | "tac" | "head" | "tail" | "less" | "more" | "bat" | "grep" | "egrep"
        | "fgrep" | "rg" | "ack" | "fd" | "wc" | "stat" | "which" | "type" | "command"
        | "whereis" | "echo" | "printf" | "yes" | "true" | "false" | "date" | "cal" | "bc"
        | "expr" | "tree" | "diff" | "cmp" | "comm" | "sort" | "uniq" | "cut" | "paste"
        | "join" | "tr" | "rev" | "fold" | "expand" | "unexpand" | "jq" | "yq" | "tomlq" | "xq"
        | "dasel" | "miller" | "mlr" | "csvkit" => Ok(()),
        _ => Err(REASON_NOT_ALLOWLISTED),
    }
}

fn is_readonly_git(args: &[String]) -> bool {
    if args.iter().any(|a| a == "-c" || a.starts_with("-c=")) {
        return false;
    }

    let Some((subcommand_idx, subcommand)) = git_subcommand(args) else {
        return false;
    };
    let rest = &args[subcommand_idx + 1..];
    if git_has_write_output_flag(rest) {
        return false;
    }

    match subcommand {
        "log" | "show" | "diff" | "status" | "ls-files" | "ls-tree" | "rev-parse" | "blame"
        | "describe" | "shortlog" | "for-each-ref" | "cat-file" => true,
        "branch" => git_branch_is_readonly(rest),
        "tag" => git_tag_is_readonly(rest),
        "remote" => git_remote_is_readonly(rest),
        "submodule" => git_submodule_is_readonly(rest),
        "reflog" => git_reflog_is_readonly(rest),
        _ => false,
    }
}

fn git_subcommand(args: &[String]) -> Option<(usize, &str)> {
    let mut i = 0;
    while i < args.len() {
        let arg = &args[i];
        if arg == "--" {
            return None;
        }
        if git_global_option_takes_value(arg) {
            i += if arg.contains('=') { 1 } else { 2 };
            continue;
        }
        if arg.starts_with('-') {
            i += 1;
            continue;
        }
        return Some((i, arg.as_str()));
    }
    None
}

fn git_global_option_takes_value(arg: &str) -> bool {
    matches!(
        arg,
        "--git-dir"
            | "--work-tree"
            | "--namespace"
            | "--super-prefix"
            | "--config-env"
            | "--exec-path"
            | "--html-path"
            | "--man-path"
            | "--info-path"
    ) || arg.starts_with("--git-dir=")
        || arg.starts_with("--work-tree=")
        || arg.starts_with("--namespace=")
        || arg.starts_with("--super-prefix=")
        || arg.starts_with("--config-env=")
        || arg.starts_with("--exec-path=")
        || arg.starts_with("--html-path=")
        || arg.starts_with("--man-path=")
        || arg.starts_with("--info-path=")
}

fn git_has_write_output_flag(args: &[String]) -> bool {
    args.iter().any(|arg| {
        matches!(arg.as_str(), "--output" | "-o" | "--ext-diff")
            || arg.starts_with("--output=")
            || arg.starts_with("-o")
    })
}

fn git_branch_is_readonly(args: &[String]) -> bool {
    git_args_are_inspection_only(
        args,
        &[
            "--delete",
            "--move",
            "--copy",
            "--force",
            "--set-upstream-to",
            "--unset-upstream",
            "--edit-description",
            "--track",
            "--no-track",
            "--create-reflog",
        ],
        &['d', 'm', 'c', 'f'],
        &[
            "--contains",
            "--no-contains",
            "--merged",
            "--no-merged",
            "--points-at",
            "--format",
            "--sort",
            "--color",
            "--column",
            "--abbrev",
        ],
    )
}

fn git_tag_is_readonly(args: &[String]) -> bool {
    git_args_are_inspection_only(
        args,
        &[
            "--delete",
            "--annotate",
            "--sign",
            "--local-user",
            "--force",
            "--message",
            "--file",
            "--edit",
            "--cleanup",
        ],
        &['a', 's', 'u', 'f', 'd', 'm'],
        &[
            "--list",
            "--contains",
            "--no-contains",
            "--merged",
            "--no-merged",
            "--points-at",
            "--format",
            "--sort",
            "--color",
            "--column",
        ],
    )
}

fn git_args_are_inspection_only(
    args: &[String],
    mutating_long: &[&str],
    mutating_short_chars: &[char],
    value_flags: &[&str],
) -> bool {
    let mut i = 0;
    while i < args.len() {
        let arg = args[i].as_str();
        if arg == "--" {
            return false;
        }
        if mutating_long
            .iter()
            .any(|flag| arg_matches_long_flag_or_value(arg, flag))
        {
            return false;
        }
        if arg.starts_with('-') && !arg.starts_with("--") {
            if arg
                .chars()
                .skip(1)
                .any(|ch| mutating_short_chars.contains(&ch))
            {
                return false;
            }
            i += 1;
            continue;
        }
        if let Some(flag) = value_flags
            .iter()
            .find(|flag| arg_matches_long_flag_or_value(arg, flag))
        {
            i += if arg == *flag { 2 } else { 1 };
            continue;
        }
        if arg.starts_with('-') {
            i += 1;
            continue;
        }
        return false;
    }
    true
}

fn arg_matches_long_flag_or_value(arg: &str, flag: &str) -> bool {
    arg == flag
        || arg
            .strip_prefix(flag)
            .is_some_and(|rest| rest.starts_with('='))
}

fn git_remote_is_readonly(args: &[String]) -> bool {
    let Some((idx, subcommand)) = first_non_flag_arg(args) else {
        return true;
    };
    match subcommand {
        "show" | "get-url" => args[idx + 1..].iter().all(|arg| arg != "--add"),
        _ => false,
    }
}

fn git_submodule_is_readonly(args: &[String]) -> bool {
    let Some((_, subcommand)) = first_non_flag_arg(args) else {
        return true;
    };
    matches!(subcommand, "status" | "summary")
}

fn git_reflog_is_readonly(args: &[String]) -> bool {
    let Some((_, subcommand)) = first_non_flag_arg(args) else {
        return true;
    };
    matches!(subcommand, "show" | "exists")
}

fn first_non_flag_arg(args: &[String]) -> Option<(usize, &str)> {
    args.iter()
        .enumerate()
        .find(|(_, arg)| !arg.starts_with('-'))
        .map(|(idx, arg)| (idx, arg.as_str()))
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
            if matches!(
                a.as_str(),
                "-i" | "-p" | "-o" | "-F" | "-c" | "-J" | "-b" | "-B"
            ) {
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
        "LD_PRELOAD",
        "LD_AUDIT",
        "LD_LIBRARY_PATH",
        "BASH_ENV",
        "ENV",
        "PROMPT_COMMAND",
        "PS0",
        "PS1",
        "PS2",
        "PS3",
        "PS4",
        "IFS",
        "PATH",
        "SHELL",
        "BASH",
        "GIT_EXTERNAL_DIFF",
        "GIT_PAGER",
        "GIT_SSH_COMMAND",
        "GIT_DIR",
        "PAGER",
        "MANPAGER",
        "LESS",
        "MANROFFSEQ",
    ];
    !DENY_ENV.contains(&name)
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
        let Some(&sep) = chars.peek() else {
            return false;
        };
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
        "system(",
        "getline",
        "print>",
        "print >",
        "print|",
        "print |",
        "printf>",
        "printf >",
        "printf|",
        "printf |",
        "| getline",
        "|getline",
    ] {
        if lower.contains(needle) {
            return true;
        }
    }
    false
}

/// Serializes every test that mutates `JFC_ALLOW_CATASTROPHIC_BASH`, across
/// modules, so cargo's parallel test threads can't interleave a `set_var` in
/// one test with a `catastrophic_bash_reason` read in another (process-global
/// env is shared state). Tests lock this for the full set→assert→clear span.
/// Test-support only — downstream crates' suites need it across the crate
/// boundary, so it cannot be `#[cfg(test)]`-gated here.
#[doc(hidden)]
pub static CATASTROPHIC_ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[cfg(test)]
mod readonly_tests {
    use super::*;

    #[test]
    fn git_mutating_subcommands_are_not_readonly_regression() {
        for command in [
            "git branch -D feature/old",
            "git tag -d v1.0.0",
            "git remote add origin git@example.com:repo.git",
            "git remote set-url origin git@example.com:repo.git",
            "git submodule update --init",
        ] {
            assert_eq!(
                classify_readonly_bash(command),
                Err(REASON_GIT_SUBCOMMAND),
                "must reject mutating git command: {command}"
            );
        }
    }

    #[test]
    fn git_inspection_subcommands_remain_readonly_normal() {
        for command in [
            "git status --short",
            "git diff --stat",
            "git branch --show-current",
            "git tag --list v*",
            "git remote show origin",
            "git remote get-url origin",
            "git submodule status",
            "git reflog show --date=iso",
        ] {
            assert!(
                is_readonly_bash(command),
                "must allow read-only git command: {command:?} -> {:?}",
                classify_readonly_bash(command)
            );
        }
    }

    #[test]
    fn curl_attached_write_flags_are_not_readonly_regression() {
        for command in [
            "curl -o/tmp/out https://example.com",
            "curl --output=/tmp/out https://example.com",
            "curl --cookie-jar=/tmp/cookies https://example.com",
            "curl --upload-file=/tmp/payload https://example.com",
            "curl -XPOST https://example.com",
            "curl --request=POST https://example.com",
        ] {
            assert_eq!(
                classify_readonly_bash(command),
                Err(REASON_CURL_WRITE),
                "must reject curl write form: {command}"
            );
        }
    }

    #[test]
    fn formatters_require_check_mode_regression() {
        for command in ["cargo fmt", "terraform fmt", "tofu fmt"] {
            assert!(
                classify_readonly_bash(command).is_err(),
                "must reject writing formatter: {command}"
            );
        }
        for command in [
            "cargo fmt --check",
            "terraform fmt -check",
            "tofu fmt --check",
        ] {
            assert!(
                is_readonly_bash(command),
                "must allow formatter check mode: {command:?} -> {:?}",
                classify_readonly_bash(command)
            );
        }
    }
}

#[cfg(test)]
mod catastrophic_tests {
    use super::*;

    /// Take the cross-module env lock and clear the override. Returned guard
    /// must be held for the test body so no parallel test sets the var.
    fn guard_off() -> std::sync::MutexGuard<'static, ()> {
        let g = super::CATASTROPHIC_ENV_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        unsafe { std::env::remove_var("JFC_ALLOW_CATASTROPHIC_BASH") };
        g
    }

    // Normal: each catastrophic class is flagged.
    #[test]
    fn flags_catastrophic_classes_normal() {
        let _g = guard_off();
        let cases = [
            "rm -rf /",
            "rm -rf /*",
            "rm -rf ~",
            "rm -rf ~/*",
            "rm -rf /home/cole",
            "rm -rf /home/cole/*",
            "rm -rf $HOME/*",
            "rm -rf \"$HOME\"/*",
            "rm -rf \"${HOME}\"/*",
            "rm -rf /etc",
            "sudo rm -rf /usr",
            "dd if=/dev/zero of=/dev/sda bs=1M",
            "mkfs.ext4 /dev/sdb1",
            "shred -u /dev/sda",
            "git push --force origin master",
            "git push -f origin main",
            "rm -rf .git",
            "cd /repo && rm -rf .git/",
        ];
        for c in cases {
            assert!(
                catastrophic_bash_reason(c).is_some(),
                "should be catastrophic: {c:?}"
            );
        }
    }

    // Robust: ordinary destructive-but-recoverable ops are NOT flagged — they
    // are the caller's normal Default-mode prompt territory, and gating them
    // in Bypass would deadlock headless/swarm agents.
    #[test]
    fn does_not_flag_safe_destructive_robust() {
        let _g = guard_off();
        let safe = [
            "rm -rf target",
            "rm -rf node_modules",
            "rm -rf /tmp/whatever",
            "rm -rf .jfc-worktrees/t1",
            "rm -rf .claude/worktrees/t1",
            "rm -rf /home/cole/RustProjects/active/jfc/target", // deep targeted
            "git reset --hard HEAD",
            "git reset --hard origin/master",
            "git branch -D feature/old",
            "git worktree remove --force .jfc-worktrees/x",
            "git worktree remove --force .claude/worktrees/x",
            "git push --force-with-lease origin master", // SAFE variant
            "git stash drop",
            "git clean -fd",
            "dd if=in.img of=out.img", // file→file, not a device
            "echo hi > file.txt",
        ];
        for c in safe {
            assert!(
                catastrophic_bash_reason(c).is_none(),
                "should NOT be catastrophic: {c:?} → {:?}",
                catastrophic_bash_reason(c)
            );
        }
    }

    // Robust: force-with-lease is never flagged even targeting master (it's
    // the collision-safe push and the recommended replacement).
    #[test]
    fn force_with_lease_is_safe_robust() {
        let _g = guard_off();
        assert!(catastrophic_bash_reason("git push --force-with-lease origin master").is_none());
    }

    // Robust: env override suppresses all flagging.
    #[test]
    fn override_env_suppresses_robust() {
        let _g = super::CATASTROPHIC_ENV_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        // SAFETY: lock serializes all env mutation across test threads.
        unsafe { std::env::set_var("JFC_ALLOW_CATASTROPHIC_BASH", "1") };
        let r = catastrophic_bash_reason("rm -rf /home/cole");
        unsafe { std::env::remove_var("JFC_ALLOW_CATASTROPHIC_BASH") };
        assert!(r.is_none(), "override must suppress catastrophic flagging");
    }

    // Robust: a deep, specific path under /home is a targeted delete, not a
    // whole-home wipe — must not flag (false-positive guard).
    #[test]
    fn deep_home_path_is_not_whole_home_robust() {
        let _g = guard_off();
        assert!(catastrophic_bash_reason("rm -rf /home/cole/project/build").is_none());
        assert!(catastrophic_bash_reason("rm -rf /home/cole/project/*").is_none());
        // but the bare home dir IS catastrophic
        assert!(catastrophic_bash_reason("rm -rf /home/cole").is_some());
        assert!(catastrophic_bash_reason("rm -rf /home/cole/*").is_some());
    }
}

#[cfg(test)]
mod _review_probe {
    use super::is_readonly_bash;
    #[test]
    fn probe_secret_and_network_reads() {
        for c in [
            "cat /etc/shadow",
            "sudo cat /etc/shadow",
            "cat /home/cole/.ssh/id_rsa",
            "cat ~/.aws/credentials",
            "base64 /home/cole/.ssh/id_ed25519",
            "curl https://attacker.example/beacon",
            "curl https://example.com/p/abcdef",
            "ssh user@host cat /etc/passwd",
            "cat .env",
        ] {
            println!("READONLY {} => {}", is_readonly_bash(c), c);
        }
        // force visible output
        assert!(true);
    }
}
