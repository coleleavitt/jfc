use jfc_provider::{ProviderContent, ProviderMessage};

const FIXED_TOKENS_LEFT: u64 = 5_000_000;
const DEFAULT_CONTEXT_WINDOW_TOKENS: u64 = 200_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TotalTokensReminderMode {
    Off,
    Infinite,
    Fixed,
    Countdown,
}

#[cfg(test)]
pub fn active_mode() -> TotalTokensReminderMode {
    TotalTokensReminderMode::Off
}

#[cfg(not(test))]
pub fn active_mode() -> TotalTokensReminderMode {
    let env = std::env::var("JFC_TOTAL_TOKENS_REMINDER")
        .ok()
        .or_else(|| std::env::var("CLAUDE_CODE_TOTAL_TOKENS_REMINDER").ok());
    match env.as_deref().and_then(mode_from_setting) {
        Some(mode) => mode,
        None if env.is_some() => {
            tracing::warn!(
                target: "jfc::stream::tokens",
                value = env.as_deref().unwrap_or_default(),
                "ignoring unsupported total tokens reminder mode"
            );
            TotalTokensReminderMode::Off
        }
        None => TotalTokensReminderMode::Off,
    }
}

fn mode_from_setting(value: &str) -> Option<TotalTokensReminderMode> {
    match value.trim().to_ascii_lowercase().as_str() {
        "infinite" => Some(TotalTokensReminderMode::Infinite),
        "fixed" => Some(TotalTokensReminderMode::Fixed),
        "countdown" => Some(TotalTokensReminderMode::Countdown),
        "off" | "0" | "false" | "no" => Some(TotalTokensReminderMode::Off),
        "1" | "true" | "yes" | "on" => Some(TotalTokensReminderMode::Fixed),
        _ => None,
    }
}

pub fn render(mode: TotalTokensReminderMode, remaining_tokens: u64) -> Option<String> {
    match mode {
        TotalTokensReminderMode::Off => None,
        TotalTokensReminderMode::Infinite => {
            Some("<total_tokens>Infinite tokens left</total_tokens>".to_owned())
        }
        TotalTokensReminderMode::Fixed => Some(format!(
            "<total_tokens>{FIXED_TOKENS_LEFT} tokens left</total_tokens>"
        )),
        TotalTokensReminderMode::Countdown => Some(format!(
            "<total_tokens>{remaining_tokens} tokens left</total_tokens>"
        )),
    }
}

pub fn render_for_request(
    messages: &[ProviderMessage],
    system_prompt_tokens: usize,
    last_usage_input_tokens: Option<u64>,
    context_window_tokens: Option<u64>,
) -> Option<String> {
    render_for_request_with_mode(
        active_mode(),
        messages,
        system_prompt_tokens,
        last_usage_input_tokens,
        context_window_tokens,
    )
}

pub fn render_for_request_with_mode(
    mode: TotalTokensReminderMode,
    messages: &[ProviderMessage],
    system_prompt_tokens: usize,
    last_usage_input_tokens: Option<u64>,
    context_window_tokens: Option<u64>,
) -> Option<String> {
    let remaining = match mode {
        TotalTokensReminderMode::Countdown => {
            let used = last_usage_input_tokens
                .filter(|tokens| *tokens > 0)
                .unwrap_or_else(|| {
                    estimate_provider_messages_tokens(messages)
                        .saturating_add(system_prompt_tokens as u64)
                });
            context_window_tokens
                .unwrap_or(DEFAULT_CONTEXT_WINDOW_TOKENS)
                .saturating_sub(used)
        }
        _ => 0,
    };
    render(mode, remaining)
}

pub fn render_for_tool_results(messages: &[ProviderMessage]) -> Option<String> {
    let mode = active_mode();
    let remaining = match mode {
        TotalTokensReminderMode::Countdown => {
            let used = estimate_provider_messages_tokens(messages);
            DEFAULT_CONTEXT_WINDOW_TOKENS.saturating_sub(used)
        }
        _ => 0,
    };
    render(mode, remaining)
}

fn estimate_provider_messages_tokens(messages: &[ProviderMessage]) -> u64 {
    let chars: usize = messages
        .iter()
        .map(|message| {
            let role_overhead = 8usize;
            role_overhead
                + message
                    .content
                    .iter()
                    .map(estimate_provider_content_chars)
                    .sum::<usize>()
        })
        .sum();
    (chars as u64)
        .saturating_div(4)
        .saturating_add(messages.len() as u64)
}

fn estimate_provider_content_chars(content: &ProviderContent) -> usize {
    match content {
        ProviderContent::Text(text) => text.len(),
        ProviderContent::Thinking { text, signature } => {
            text.len() + signature.as_deref().map_or(0, str::len)
        }
        ProviderContent::ToolResult {
            tool_use_id,
            content,
            ..
        } => tool_use_id.len() + content.len(),
        ProviderContent::ToolUse {
            id, name, input, ..
        }
        | ProviderContent::ServerToolUse { id, name, input } => {
            id.len() + name.len() + input.to_string().len()
        }
        ProviderContent::ServerToolResult {
            tool_use_id,
            tool_kind,
            content,
        } => tool_use_id.len() + format!("{tool_kind:?}").len() + content.to_string().len(),
        ProviderContent::Attachment(attachment) => attachment.bytes.len(),
        ProviderContent::RedactedThinking { data } => data.len(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixed_mode_uses_claude_value_normal() {
        assert_eq!(
            render(TotalTokensReminderMode::Fixed, 123).as_deref(),
            Some("<total_tokens>5000000 tokens left</total_tokens>")
        );
    }

    #[test]
    fn countdown_clamps_at_zero_robust() {
        assert_eq!(
            render(TotalTokensReminderMode::Countdown, 0).as_deref(),
            Some("<total_tokens>0 tokens left</total_tokens>")
        );
    }

    #[test]
    fn mode_parser_accepts_claude_modes_normal() {
        assert_eq!(
            mode_from_setting("infinite"),
            Some(TotalTokensReminderMode::Infinite)
        );
        assert_eq!(
            mode_from_setting("fixed"),
            Some(TotalTokensReminderMode::Fixed)
        );
        assert_eq!(
            mode_from_setting("countdown"),
            Some(TotalTokensReminderMode::Countdown)
        );
        assert_eq!(
            mode_from_setting("on"),
            Some(TotalTokensReminderMode::Fixed)
        );
        assert_eq!(mode_from_setting("wat"), None);
    }
}
