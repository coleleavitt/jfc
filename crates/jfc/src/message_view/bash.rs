use super::syntax::redact_quoted;
/// What kind of bash command produced this output, derived purely
/// from the command string. Drives renderer dispatch — each kind
/// has its own visual treatment.

#[derive(Debug, Clone)]
pub(super) enum BashCmdKind {
    /// `grep` / `rg` / `ack` results: `path:line:match` per line.
    Grep,
    /// `find` / `ls` / `tree` / `fd` etc. — flat path list.
    PathList,
    /// `git diff` / `git show` / raw `diff -u` — unified diff with
    /// `+`/`-`/`@@` lines that should be colored.
    GitDiff,
    /// `git log` — commit metadata + body.
    GitLog,
    /// `jq` — output is always JSON.
    Json,
    /// `cargo test` / `cargo check` / `make` — compiler/test output.
    CompilerOutput,
    /// `curl` — HTTP response (may be JSON/HTML/XML).
    HttpResponse,
    /// `xxd` / `hexyl` / `od` — hex dump (offset · bytes · ASCII).
    HexDump,
    /// `docker ps` / `docker images` / `kubectl get` — fixed-width
    /// table with a header row and aligned columns.
    TabularList,
    /// Plain command (default).
    Other,
}

/// Classify the *primary* command (first segment of `||` / `|`)
/// for output-rendering dispatch. Independent of the
/// `infer_lang_from_bash` path which is for cat-and-friends file
/// content; this one routes structured tools (grep, find, git).
pub(super) fn classify_bash_cmd(command: &str) -> BashCmdKind {
    // Pipeline / chain decomposition. We walk in this order:
    //   1. split on `||` (cat-with-fallback pattern),
    //   2. split on `|` (pipe to less etc.),
    //   3. split on `&&` (cd-and-then pattern: `cd X && grep …`).
    // For (3) we take the LAST segment because the chain semantically
    // ends with the meaningful command — `cd ~/dir && cat README.md`
    // is "the cat is what produces output", not the cd.
    let primary_alt = command
        .split("||")
        .next()
        .unwrap_or(command)
        .split('|')
        .next()
        .unwrap_or(command);
    let primary = primary_alt
        .split("&&")
        .filter(|s| !s.trim().is_empty())
        .last()
        .unwrap_or(primary_alt);
    let trimmed = primary.trim();
    // Reject only the *truly* fancy patterns now: command
    // substitution, backticks, sequential `;`, background `&` not
    // covered by `&&` (single-`&` daemonization). The earlier
    // version blanket-rejected `&` which broke `cd X && cmd` for
    // every structured tool.
    // Quote-aware meta-character check: `sed -n '1,$p' file` is a
    // benign call and shouldn't be rejected for its quoted `$`.
    let probe = redact_quoted(trimmed);
    if probe.contains('$') || probe.contains('`') || probe.contains(';') {
        return BashCmdKind::Other;
    }
    // Reject lone `&` (background) — but `&&` was already split
    // out above, so any `&` left here is the lone form.
    if probe.contains('&') {
        return BashCmdKind::Other;
    }
    let toks: Vec<&str> = trimmed
        .split_whitespace()
        .filter(|t| !t.starts_with("2>") && !t.starts_with(">"))
        .collect();
    let Some(verb) = toks.first() else {
        return BashCmdKind::Other;
    };
    // git subcommand routing — `git diff`, `git show`, `git log`
    // each get their own renderer.
    if *verb == "git" {
        if let Some(sub) = toks.get(1) {
            match *sub {
                "diff" | "show" => return BashCmdKind::GitDiff,
                "log" => return BashCmdKind::GitLog,
                _ => return BashCmdKind::Other,
            }
        }
        return BashCmdKind::Other;
    }
    match *verb {
        "grep" | "rg" | "ack" | "ag" => BashCmdKind::Grep,
        "find" | "ls" | "tree" | "fd" | "exa" | "eza" => BashCmdKind::PathList,
        "jq" | "yq" => BashCmdKind::Json,
        // Raw POSIX `diff` (with -u/--unified) emits the same +/-/@@
        // shape `git diff` does — share the renderer so coloring
        // works for ad-hoc `diff -u a b` invocations too.
        "diff" => BashCmdKind::GitDiff,
        "cargo" => {
            if let Some(sub) = toks.get(1) {
                match *sub {
                    "test" | "check" | "build" | "clippy" => BashCmdKind::CompilerOutput,
                    _ => BashCmdKind::Other,
                }
            } else {
                BashCmdKind::Other
            }
        }
        "make" | "cmake" | "gcc" | "g++" | "rustc" | "tsc" | "npm" | "yarn" | "pnpm" => {
            BashCmdKind::CompilerOutput
        }
        "curl" | "wget" | "httpie" | "http" => BashCmdKind::HttpResponse,
        "xxd" | "hexyl" | "od" => BashCmdKind::HexDump,
        // Container / k8s tools — `docker ps`, `docker images`,
        // `kubectl get …`, `podman ps` — output is always a header
        // row + fixed-width columns.
        "docker" | "podman" => match toks.get(1).copied() {
            Some("ps") | Some("images") | Some("image") | Some("container") | Some("network")
            | Some("volume") => BashCmdKind::TabularList,
            _ => BashCmdKind::Other,
        },
        "kubectl" | "k9s" | "oc" => match toks.get(1).copied() {
            Some("get") | Some("describe") | Some("top") => BashCmdKind::TabularList,
            _ => BashCmdKind::Other,
        },
        _ => BashCmdKind::Other,
    }
}
