//! Slash-command `/login` dispatcher.
//!
//! Maps a sub-target string (`anthropic`, `claudeai`, `bedrock`, `vertex`,
//! `console`) to the appropriate wizard or instructional response. Keeps the
//! fan-out logic out of `main.rs` so adding a new auth backend is a single
//! match arm here.
//!
//! The wizards themselves are stateful (see [`super::bedrock_wizard::BedrockWizard`]
//! and [`super::vertex_wizard::VertexWizard`]); this module only chooses *which*
//! one to start. The TUI / slash-command host owns the per-step interaction.

use std::fmt;

/// Outcome of dispatching `/login [arg]`. The TUI host turns each variant into
/// the appropriate side effect: render a chooser, instantiate a wizard, or
/// print a one-shot message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoginDispatch {
    /// `/login` with no arg — show a list of available providers and ask the
    /// user to pick one. The string is the human-readable chooser body.
    ShowChooser(String),
    /// `/login anthropic` — point at the API-key env var. We don't run a wizard
    /// here because API-key auth is already covered by `ANTHROPIC_API_KEY`.
    AnthropicApiKey(String),
    /// `/login claudeai` — point at the OAuth account store managed by the
    /// existing `opencode auth` flow (or a future jfc-side wizard).
    ClaudeAiOAuth(String),
    /// `/login codex` — point at OpenAI Codex / ChatGPT OAuth commands.
    CodexOAuth(String),
    /// `/login bedrock` — kick off [`super::bedrock_wizard::BedrockWizard`].
    StartBedrockWizard,
    /// `/login vertex` — kick off [`super::vertex_wizard::VertexWizard`].
    StartVertexWizard,
    /// `/login litellm` — point at the LiteLLM proxy credential file.
    LiteLlm(String),
    /// `/login console` — treat the same as `anthropic` (Anthropic Console
    /// issues plain API keys that route through `api.anthropic.com`).
    ConsoleApiKey(String),
    /// Unknown sub-target. Body lists the recognized targets so the user can
    /// retry without re-reading the help.
    Unknown(String),
}

impl fmt::Display for LoginDispatch {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ShowChooser(s)
            | Self::AnthropicApiKey(s)
            | Self::ClaudeAiOAuth(s)
            | Self::CodexOAuth(s)
            | Self::LiteLlm(s)
            | Self::ConsoleApiKey(s)
            | Self::Unknown(s) => f.write_str(s),
            Self::StartBedrockWizard => f.write_str("Starting Bedrock wizard…"),
            Self::StartVertexWizard => f.write_str("Starting Vertex wizard…"),
        }
    }
}

const CHOOSER_BODY: &str = "\
Choose an authentication target:
  /login anthropic   API key (ANTHROPIC_API_KEY)
  /login claudeai    Claude Code OAuth (Pro / Max / Team)
  /login codex       OpenAI Codex OAuth (ChatGPT Pro / Plus)
  /login litellm     LiteLLM proxy (base URL + API key)
  /login bedrock     AWS Bedrock (requires `aws` CLI)
  /login vertex      GCP Vertex (requires `gcloud` CLI)
  /login console     Anthropic Console API key";

const ANTHROPIC_API_KEY_BODY: &str = "\
Anthropic API key sign-in:
  1. Visit https://console.anthropic.com/settings/keys to mint a key.
  2. Export it: `export ANTHROPIC_API_KEY=sk-ant-…`
  3. Restart jfc; the API-key provider is registered automatically.";

const CONSOLE_API_KEY_BODY: &str = "\
Anthropic Console sign-in:
  Anthropic Console issues plain API keys that hit api.anthropic.com.
  Run `/login anthropic` for the same setup, or paste the key into
  ANTHROPIC_API_KEY in your shell profile.";

const CLAUDEAI_OAUTH_BODY: &str = "\
Claude Code OAuth (claudeai) sign-in:
  Run from another terminal:
    jfc auth anthropic login
    jfc auth anthropic login --manual   # fallback paste flow

  Other commands:
    jfc auth anthropic list           # show all accounts + tier + status
    jfc auth anthropic switch <name>  # set the preferred account
    jfc auth anthropic disable <name> # skip an account in rotation
    jfc auth anthropic remove <name>  # delete an account from the store

  Multi-account rotation: when an account hits 429 / invalid_grant, jfc
  silently switches to the next-best account (tier-ranked, cooldown-aware).
  The store is at ~/.config/jfc-anthropic-accounts.json. Set
  JFC_ANTHROPIC_ACCOUNTS_PATH to use a custom path (e.g. opencode's store
  at ~/.config/opencode/anthropic-accounts.json to share rotation state).";

const LITELLM_BODY: &str = "\
LiteLLM proxy sign-in:
  Create or edit ~/.config/jfc/litellm.toml with:

    api_key  = \"sk-…\"
    base_url = \"http://localhost:4000\"

  Or run interactively:
    jfc auth litellm login

  Once configured, models are fetched dynamically from the proxy's
  /v1/models endpoint. Use `litellm/<model>` as the model selector.";

const CODEX_OAUTH_BODY: &str = "\
OpenAI Codex OAuth sign-in:
  Browser flow:
    jfc auth codex login

  Headless/device flow:
    jfc auth codex device

  Other commands:
    jfc auth codex status
    jfc auth codex logout

  Codex OAuth tokens are stored in ~/.local/share/jfc/auth.json (or the
  platform data directory) and route `codex/...` models to ChatGPT's Codex
  backend with subscription pricing treated as $0.";

/// Dispatch a `/login [arg]` invocation. `arg` is `Some` when the user typed
/// a sub-target, `None` for the bare `/login`. Sub-target matching is
/// case-insensitive and trim-aware.
pub fn dispatch(arg: Option<&str>) -> LoginDispatch {
    match arg.map(str::trim).filter(|s| !s.is_empty()) {
        None => LoginDispatch::ShowChooser(CHOOSER_BODY.to_owned()),
        Some(target) => match target.to_ascii_lowercase().as_str() {
            "anthropic" | "api" | "apikey" | "api-key" => {
                LoginDispatch::AnthropicApiKey(ANTHROPIC_API_KEY_BODY.to_owned())
            }
            "claudeai" | "oauth" | "claude" => {
                LoginDispatch::ClaudeAiOAuth(CLAUDEAI_OAUTH_BODY.to_owned())
            }
            "codex" | "chatgpt" | "openai-oauth" => {
                LoginDispatch::CodexOAuth(CODEX_OAUTH_BODY.to_owned())
            }
            "litellm" | "lite-llm" | "lite_llm" => LoginDispatch::LiteLlm(LITELLM_BODY.to_owned()),
            "bedrock" | "aws" => LoginDispatch::StartBedrockWizard,
            "vertex" | "gcp" | "gcloud" => LoginDispatch::StartVertexWizard,
            "console" => LoginDispatch::ConsoleApiKey(CONSOLE_API_KEY_BODY.to_owned()),
            other => LoginDispatch::Unknown(format!(
                "Unknown login target {:?}. Try one of: anthropic, claudeai, codex, litellm, bedrock, vertex, console.",
                other
            )),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Normal: bare /login surfaces the chooser body so the user can pick.
    #[test]
    fn dispatch_no_arg_shows_chooser_normal() {
        match dispatch(None) {
            LoginDispatch::ShowChooser(body) => {
                assert!(body.contains("anthropic"));
                assert!(body.contains("bedrock"));
                assert!(body.contains("vertex"));
                assert!(body.contains("claudeai"));
                assert!(body.contains("codex"));
                assert!(body.contains("litellm"));
                assert!(body.contains("console"));
            }
            other => panic!("expected ShowChooser, got {other:?}"),
        }
    }

    // Normal: every documented sub-target routes to its expected variant.
    #[test]
    fn dispatch_known_targets_normal() {
        assert!(matches!(
            dispatch(Some("anthropic")),
            LoginDispatch::AnthropicApiKey(_)
        ));
        assert!(matches!(
            dispatch(Some("claudeai")),
            LoginDispatch::ClaudeAiOAuth(_)
        ));
        assert!(matches!(
            dispatch(Some("codex")),
            LoginDispatch::CodexOAuth(_)
        ));
        assert!(matches!(
            dispatch(Some("litellm")),
            LoginDispatch::LiteLlm(_)
        ));
        assert!(matches!(
            dispatch(Some("bedrock")),
            LoginDispatch::StartBedrockWizard
        ));
        assert!(matches!(
            dispatch(Some("vertex")),
            LoginDispatch::StartVertexWizard
        ));
        assert!(matches!(
            dispatch(Some("console")),
            LoginDispatch::ConsoleApiKey(_)
        ));
    }

    // Normal: case-insensitive matching so `/login Bedrock` works.
    #[test]
    fn dispatch_case_insensitive_normal() {
        assert!(matches!(
            dispatch(Some("BEDROCK")),
            LoginDispatch::StartBedrockWizard
        ));
        assert!(matches!(
            dispatch(Some("Vertex")),
            LoginDispatch::StartVertexWizard
        ));
    }

    // Normal: friendly aliases still route correctly (api → anthropic, etc.).
    #[test]
    fn dispatch_aliases_normal() {
        assert!(matches!(
            dispatch(Some("api")),
            LoginDispatch::AnthropicApiKey(_)
        ));
        assert!(matches!(
            dispatch(Some("aws")),
            LoginDispatch::StartBedrockWizard
        ));
        assert!(matches!(
            dispatch(Some("gcp")),
            LoginDispatch::StartVertexWizard
        ));
        assert!(matches!(
            dispatch(Some("chatgpt")),
            LoginDispatch::CodexOAuth(_)
        ));
        assert!(matches!(
            dispatch(Some("lite-llm")),
            LoginDispatch::LiteLlm(_)
        ));
    }

    // Robust: empty / whitespace-only argument behaves like no argument.
    #[test]
    fn dispatch_empty_arg_treated_as_none_robust() {
        assert!(matches!(dispatch(Some("")), LoginDispatch::ShowChooser(_)));
        assert!(matches!(
            dispatch(Some("   ")),
            LoginDispatch::ShowChooser(_)
        ));
    }

    // Robust: unknown target produces an Unknown variant naming the typo and
    // listing the recognized targets, so the user can self-correct.
    #[test]
    fn dispatch_unknown_target_robust() {
        match dispatch(Some("snowflake")) {
            LoginDispatch::Unknown(msg) => {
                assert!(msg.contains("snowflake"));
                assert!(msg.contains("bedrock"));
                assert!(msg.contains("codex"));
                assert!(msg.contains("vertex"));
            }
            other => panic!("expected Unknown, got {other:?}"),
        }
    }
}
