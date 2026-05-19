//! Slash command parser and dispatcher.
//!
//! Handles user-typed slash commands like /compact, /model, /stats, etc.
//! Returns a response string to display to the user.

use std::fmt;

/// Recognized slash commands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SlashCommand {
    /// /compact — trigger context compaction
    Compact,
    /// /clear — reset conversation history
    Clear,
    /// /model [name] — show or switch model
    Model(Option<String>),
    /// /stats — show session statistics
    Stats,
    /// /effort [low|medium|high|xhigh|max] — set reasoning effort
    Effort(Option<String>),
    /// /resume [id] — resume a saved session
    Resume(Option<String>),
    /// /branch [name] — show or create git branch
    Branch(Option<String>),
    /// /permissions — show current permission mode
    Permissions,
    /// /memory [sub] — list memories or manage recall (e.g. `/memory recall on`)
    Memory(Option<String>),
    /// /hooks — list registered hooks
    Hooks,
    /// /sessions — list saved sessions
    Sessions,
    /// /help — show available commands
    Help,
    /// /exit or /quit — exit the session
    Exit,
    /// /worktree [sub] — worktree management (delegated)
    Worktree(Option<String>),
    /// /daemon [sub] — daemon management (delegated)
    Daemon(Option<String>),
    /// /mcp [list|restart <name>|logs <name>] — MCP server management
    Mcp(Option<String>),
    /// /login [provider] — sign in to a provider (anthropic, claudeai, bedrock,
    /// vertex, console). Subcommand selects which wizard to drive.
    Login(Option<String>),
    /// /logout [provider] — sign out of a provider, clearing stored credentials.
    /// Default (no arg) signs out of every provider that has tokens.
    Logout(Option<String>),
    /// /release-notes — show the changelog for the most recent jfc release.
    ReleaseNotes,
    /// /feedback — open the issue tracker / send feedback to the maintainer.
    Feedback,
    /// /upgrade — install the latest jfc release.
    Upgrade,
    /// /bug — file a bug report with the current session attached.
    Bug,
    /// /copy [all|last|n] — copy transcript text to the system clipboard.
    /// Default is the last assistant message. `all` copies the entire
    /// rendered transcript; `n` copies the last N messages.
    Copy(Option<String>),
    /// /fork [N] — snapshot the current session at message N (or the
    /// current end) as a brand-new session and switch into it. Used to
    /// branch a conversation without losing the original.
    Fork(Option<String>),
    /// /fast — toggle fast mode (lower-latency inference via beta header)
    Fast,
    /// /init — generate CLAUDE.md from codebase analysis
    Init,
    /// /commit — generate a conventional commit message from staged changes
    Commit,
    /// /review — ask the model to review current git changes for bugs/quality
    Review,
    /// /skills — list available skills (.claude/skills/*.md, ~/.config/jfc/skills/*.md)
    Skills,
    /// /status — show rich session status (model, provider, tokens, cost, MCP, etc.)
    Status,
    /// /dream [nightly] — consolidate session learnings into typed memory files
    Dream(Option<String>),
    /// /loop [interval] <prompt> — schedule a recurring cron prompt
    Loop(Option<String>),
    /// /schedule [list|cancel <id>] — manage cron schedules
    Schedule(Option<String>),
    /// /doctor — run diagnostic health check of the jfc installation
    Doctor,
    /// /plan — draft or update PLAN.md (Atlas-compatible task contract)
    Plan,
    /// /roadmap — draft or update ROADMAP.md (stable-id decimal phases)
    Roadmap,
    /// /parity — draft or update PARITY.md (lane checkpoints + evidence)
    Parity,
    /// /philosophy — draft or update PHILOSOPHY.md
    Philosophy,
    /// /usage — draft or update USAGE.md (operator-focused commands)
    Usage,
    /// Unknown command
    Unknown(String),
}

impl fmt::Display for SlashCommand {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Compact => write!(f, "/compact"),
            Self::Clear => write!(f, "/clear"),
            Self::Model(Some(m)) => write!(f, "/model {m}"),
            Self::Model(None) => write!(f, "/model"),
            Self::Stats => write!(f, "/stats"),
            Self::Effort(Some(e)) => write!(f, "/effort {e}"),
            Self::Effort(None) => write!(f, "/effort"),
            Self::Resume(Some(id)) => write!(f, "/resume {id}"),
            Self::Resume(None) => write!(f, "/resume"),
            Self::Branch(Some(b)) => write!(f, "/branch {b}"),
            Self::Branch(None) => write!(f, "/branch"),
            Self::Permissions => write!(f, "/permissions"),
            Self::Memory(Some(s)) => write!(f, "/memory {s}"),
            Self::Memory(None) => write!(f, "/memory"),
            Self::Hooks => write!(f, "/hooks"),
            Self::Sessions => write!(f, "/sessions"),
            Self::Help => write!(f, "/help"),
            Self::Exit => write!(f, "/exit"),
            Self::Worktree(Some(s)) => write!(f, "/worktree {s}"),
            Self::Worktree(None) => write!(f, "/worktree"),
            Self::Daemon(Some(s)) => write!(f, "/daemon {s}"),
            Self::Daemon(None) => write!(f, "/daemon"),
            Self::Mcp(Some(s)) => write!(f, "/mcp {s}"),
            Self::Mcp(None) => write!(f, "/mcp"),
            Self::Login(Some(p)) => write!(f, "/login {p}"),
            Self::Login(None) => write!(f, "/login"),
            Self::Logout(Some(p)) => write!(f, "/logout {p}"),
            Self::Logout(None) => write!(f, "/logout"),
            Self::ReleaseNotes => write!(f, "/release-notes"),
            Self::Feedback => write!(f, "/feedback"),
            Self::Upgrade => write!(f, "/upgrade"),
            Self::Bug => write!(f, "/bug"),
            Self::Copy(Some(a)) => write!(f, "/copy {a}"),
            Self::Copy(None) => write!(f, "/copy"),
            Self::Fork(Some(a)) => write!(f, "/fork {a}"),
            Self::Fork(None) => write!(f, "/fork"),
            Self::Fast => write!(f, "/fast"),
            Self::Init => write!(f, "/init"),
            Self::Commit => write!(f, "/commit"),
            Self::Review => write!(f, "/review"),
            Self::Skills => write!(f, "/skills"),
            Self::Status => write!(f, "/status"),
            Self::Dream(Some(a)) => write!(f, "/dream {a}"),
            Self::Dream(None) => write!(f, "/dream"),
            Self::Loop(Some(a)) => write!(f, "/loop {a}"),
            Self::Loop(None) => write!(f, "/loop"),
            Self::Schedule(Some(a)) => write!(f, "/schedule {a}"),
            Self::Schedule(None) => write!(f, "/schedule"),
            Self::Doctor => write!(f, "/doctor"),
            Self::Plan => write!(f, "/plan"),
            Self::Roadmap => write!(f, "/roadmap"),
            Self::Parity => write!(f, "/parity"),
            Self::Philosophy => write!(f, "/philosophy"),
            Self::Usage => write!(f, "/usage"),
            Self::Unknown(s) => write!(f, "/{s}"),
        }
    }
}

/// Parse user input into a slash command (if it starts with /).
pub fn parse_slash_command(input: &str) -> Option<SlashCommand> {
    let trimmed = input.trim();
    if !trimmed.starts_with('/') {
        return None;
    }

    let parts: Vec<&str> = trimmed[1..].splitn(2, ' ').collect();
    let cmd = parts[0].to_lowercase();
    let arg = parts.get(1).map(|s| s.trim().to_string());

    let slash = match cmd.as_str() {
        "compact" => SlashCommand::Compact,
        "clear" => SlashCommand::Clear,
        "model" | "m" => SlashCommand::Model(arg),
        "stats" => SlashCommand::Stats,
        "status" => SlashCommand::Status,
        "effort" | "e" => SlashCommand::Effort(arg),
        "resume" | "r" => SlashCommand::Resume(arg),
        "branch" | "br" => SlashCommand::Branch(arg),
        "permissions" | "perms" | "mode" => SlashCommand::Permissions,
        "memory" | "mem" => SlashCommand::Memory(arg),
        "hooks" => SlashCommand::Hooks,
        "sessions" => SlashCommand::Sessions,
        "help" | "h" | "?" => SlashCommand::Help,
        "exit" | "quit" | "q" => SlashCommand::Exit,
        "worktree" | "wt" => SlashCommand::Worktree(arg),
        "daemon" | "fleet" => SlashCommand::Daemon(arg),
        "mcp" => SlashCommand::Mcp(arg),
        "login" => SlashCommand::Login(arg),
        "logout" => SlashCommand::Logout(arg),
        "release-notes" | "releasenotes" | "changelog" => SlashCommand::ReleaseNotes,
        "feedback" => SlashCommand::Feedback,
        "upgrade" => SlashCommand::Upgrade,
        "bug" => SlashCommand::Bug,
        "copy" => SlashCommand::Copy(arg),
        "fork" => SlashCommand::Fork(arg),
        "fast" | "f" => SlashCommand::Fast,
        "init" => SlashCommand::Init,
        "commit" => SlashCommand::Commit,
        "review" => SlashCommand::Review,
        "skills" => SlashCommand::Skills,
        "dream" | "learn" => SlashCommand::Dream(arg),
        "loop" | "proactive" => SlashCommand::Loop(arg),
        "schedule" | "routines" => SlashCommand::Schedule(arg),
        "doctor" => SlashCommand::Doctor,
        "plan" => SlashCommand::Plan,
        "roadmap" => SlashCommand::Roadmap,
        "parity" => SlashCommand::Parity,
        "philosophy" => SlashCommand::Philosophy,
        "usage" => SlashCommand::Usage,
        other => SlashCommand::Unknown(other.to_string()),
    };

    Some(slash)
}

/// Format the help text for all available slash commands.
pub fn help_text() -> &'static str {
    "\
Available commands:
  /compact         Compact conversation history (free up context)
  /clear           Reset conversation (start fresh)
  /model [name]    Show current model or switch to <name>
  /effort [level]  Set reasoning effort: low, medium, high, xhigh, max
  /stats           Show session statistics (tokens, cost, turns)
  /resume [id]     Resume a previous session
  /sessions        List saved sessions
  /branch [name]   Show current branch or create <name>
  /permissions     Show/change permission mode
  /memory          List project memories
  /memory recall   Manage two-phase memory recall (on / off / status)
  /hooks           Show registered lifecycle hooks
  /worktree [cmd]  Worktree management (create/list/remove/switch)
  /mcp [cmd]       MCP server management (list/restart <name>/logs <name>)
  /daemon [cmd]    Daemon management (start/stop/status/run/cron)
  /fast             Toggle fast mode (lower-latency inference)
  /init            Generate CLAUDE.md with project documentation
  /commit          Generate a conventional commit message for staged changes
  /review          Ask the model to review current git changes for issues
  /skills          List available skills (.claude/skills/, ~/.config/jfc/skills/)
  /status          Show rich session status (model, tokens, cost, MCP servers)
  /dream [nightly] Consolidate session learnings into typed memory files
  /loop [int] <p>  Schedule a recurring prompt (e.g. /loop 10m check the deploy)
  /schedule [sub]  Manage cron schedules (list / cancel <id>)
  /doctor          Run diagnostic health check of the jfc installation
  /plan            Draft or update PLAN.md (Atlas-compatible task contract)
  /roadmap         Draft or update ROADMAP.md (stable-id decimal phases)
  /parity          Draft or update PARITY.md (lane checkpoints + evidence)
  /philosophy      Draft or update PHILOSOPHY.md (project rationale)
  /usage           Draft or update USAGE.md (operator-focused commands)
  /login [target]  Sign in: anthropic | claudeai | bedrock | vertex | console
  /logout [target] Sign out and forget stored credentials (all providers if no arg)
  /release-notes   Show the changelog for the most recent jfc release
  /feedback        Open the issue tracker / send feedback to the maintainer
  /upgrade         Install the latest jfc release
  /bug             File a bug report with the current session attached
  /copy [scope]    Copy transcript to clipboard: last (default), all, or <n> tail messages
  /fork [N]        Snapshot the current session at message N as a new session and switch in
  /help            Show this help
  /exit            Exit the session"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_basic_commands() {
        assert_eq!(parse_slash_command("/compact"), Some(SlashCommand::Compact));
        assert_eq!(parse_slash_command("/clear"), Some(SlashCommand::Clear));
        assert_eq!(parse_slash_command("/exit"), Some(SlashCommand::Exit));
        assert_eq!(parse_slash_command("/quit"), Some(SlashCommand::Exit));
        assert_eq!(parse_slash_command("/help"), Some(SlashCommand::Help));
        assert_eq!(parse_slash_command("/stats"), Some(SlashCommand::Stats));
        assert_eq!(parse_slash_command("/status"), Some(SlashCommand::Status));
        assert_eq!(parse_slash_command("/commit"), Some(SlashCommand::Commit));
        assert_eq!(parse_slash_command("/review"), Some(SlashCommand::Review));
        assert_eq!(parse_slash_command("/skills"), Some(SlashCommand::Skills));
    }

    #[test]
    fn parse_commands_with_args() {
        assert_eq!(
            parse_slash_command("/model claude-3-5-sonnet"),
            Some(SlashCommand::Model(Some("claude-3-5-sonnet".to_string())))
        );
        assert_eq!(
            parse_slash_command("/effort high"),
            Some(SlashCommand::Effort(Some("high".to_string())))
        );
        assert_eq!(
            parse_slash_command("/resume abc123"),
            Some(SlashCommand::Resume(Some("abc123".to_string())))
        );
    }

    #[test]
    fn parse_shortcuts() {
        assert_eq!(parse_slash_command("/m"), Some(SlashCommand::Model(None)));
        assert_eq!(
            parse_slash_command("/e high"),
            Some(SlashCommand::Effort(Some("high".to_string())))
        );
        assert_eq!(parse_slash_command("/q"), Some(SlashCommand::Exit));
        assert_eq!(parse_slash_command("/h"), Some(SlashCommand::Help));
    }

    #[test]
    fn parse_unknown() {
        assert_eq!(
            parse_slash_command("/foobar"),
            Some(SlashCommand::Unknown("foobar".to_string()))
        );
    }

    #[test]
    fn non_slash_returns_none() {
        assert_eq!(parse_slash_command("hello"), None);
        assert_eq!(parse_slash_command(""), None);
        assert_eq!(parse_slash_command("no slash"), None);
    }

    #[test]
    fn case_insensitive() {
        assert_eq!(parse_slash_command("/COMPACT"), Some(SlashCommand::Compact));
        assert_eq!(
            parse_slash_command("/Model foo"),
            Some(SlashCommand::Model(Some("foo".to_string())))
        );
    }

    #[test]
    fn parse_mcp_no_args_normal() {
        assert_eq!(parse_slash_command("/mcp"), Some(SlashCommand::Mcp(None)));
    }

    #[test]
    fn parse_mcp_with_subcommand_normal() {
        assert_eq!(
            parse_slash_command("/mcp list"),
            Some(SlashCommand::Mcp(Some("list".to_string())))
        );
        assert_eq!(
            parse_slash_command("/mcp restart filesystem"),
            Some(SlashCommand::Mcp(Some("restart filesystem".to_string())))
        );
        assert_eq!(
            parse_slash_command("/mcp logs git"),
            Some(SlashCommand::Mcp(Some("logs git".to_string())))
        );
    }

    // Normal: bare /login surfaces a None arg so the dispatcher can render
    // a provider chooser. The named variants route to the provider-specific
    // wizards (anthropic / claudeai / bedrock / vertex / console).
    #[test]
    fn parse_login_normal() {
        assert_eq!(
            parse_slash_command("/login"),
            Some(SlashCommand::Login(None))
        );
        assert_eq!(
            parse_slash_command("/login bedrock"),
            Some(SlashCommand::Login(Some("bedrock".to_string())))
        );
        assert_eq!(
            parse_slash_command("/login vertex"),
            Some(SlashCommand::Login(Some("vertex".to_string())))
        );
        assert_eq!(
            parse_slash_command("/login console"),
            Some(SlashCommand::Login(Some("console".to_string())))
        );
    }

    // Normal: the five project-doc verbs parse without arguments.
    #[test]
    fn parse_project_doc_commands_normal() {
        assert_eq!(parse_slash_command("/plan"), Some(SlashCommand::Plan));
        assert_eq!(parse_slash_command("/roadmap"), Some(SlashCommand::Roadmap));
        assert_eq!(parse_slash_command("/parity"), Some(SlashCommand::Parity));
        assert_eq!(
            parse_slash_command("/philosophy"),
            Some(SlashCommand::Philosophy)
        );
        assert_eq!(parse_slash_command("/usage"), Some(SlashCommand::Usage));
    }

    // Robust: case-insensitive parsing covers the doc verbs.
    #[test]
    fn parse_project_doc_case_insensitive_robust() {
        assert_eq!(parse_slash_command("/PLAN"), Some(SlashCommand::Plan));
        assert_eq!(parse_slash_command("/Roadmap"), Some(SlashCommand::Roadmap));
        assert_eq!(parse_slash_command("/PARITY"), Some(SlashCommand::Parity));
    }

    // Robust: /login with extra whitespace round-trips through trim.
    #[test]
    fn parse_login_trims_whitespace_robust() {
        assert_eq!(
            parse_slash_command("/login   bedrock  "),
            Some(SlashCommand::Login(Some("bedrock".to_string())))
        );
    }
}
