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
    /// /fast — toggle fast mode (lower-latency inference via beta header)
    Fast,
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
            Self::Fast => write!(f, "/fast"),
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
        "stats" | "status" => SlashCommand::Stats,
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
        "fast" | "f" => SlashCommand::Fast,
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
  /login [target]  Sign in: anthropic | claudeai | bedrock | vertex | console
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

    // Robust: /login with extra whitespace round-trips through trim.
    #[test]
    fn parse_login_trims_whitespace_robust() {
        assert_eq!(
            parse_slash_command("/login   bedrock  "),
            Some(SlashCommand::Login(Some("bedrock".to_string())))
        );
    }
}
