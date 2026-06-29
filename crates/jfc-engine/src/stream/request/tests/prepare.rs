use std::sync::Arc;

use jfc_provider::{ModelId, Provider, StreamConvention};

use crate::exploration::ExplorationLevel;

use super::super::prepare_stream_request;
use super::{TestProvider, user_text};

struct TemperatureGlobalGuard;

impl TemperatureGlobalGuard {
    fn set(value: f64) -> Self {
        crate::exploration::set_temperature_global(Some(value));
        Self
    }
}

impl Drop for TemperatureGlobalGuard {
    fn drop(&mut self) {
        crate::exploration::set_temperature_global(None);
        crate::exploration::set_exploration_level_global(None);
        crate::effort::set_turn_effort(None);
        crate::effort::EffortState::new().publish_global();
    }
}

struct ExplorationGlobalGuard;

impl ExplorationGlobalGuard {
    fn set(level: ExplorationLevel) -> Self {
        crate::exploration::set_temperature_global(None);
        crate::exploration::set_exploration_level_global(Some(level));
        crate::effort::set_turn_effort(None);
        crate::effort::EffortState::new().publish_global();
        Self
    }
}

impl Drop for ExplorationGlobalGuard {
    fn drop(&mut self) {
        crate::exploration::set_temperature_global(None);
        crate::exploration::set_exploration_level_global(None);
        crate::effort::set_turn_effort(None);
        crate::effort::EffortState::new().publish_global();
    }
}

#[tokio::test]
async fn prepare_preserves_discovery_tools_for_plain_question_regression() {
    let provider: Arc<dyn Provider> = Arc::new(TestProvider {
        name: "openai-test",
        convention: StreamConvention::OpenAiNative,
    });
    let request = prepare_stream_request(
        provider,
        &[user_text("what is ownership in rust?")],
        &ModelId::new("test-model"),
        Default::default(),
    )
    .await;

    let tool_names = request
        .opts
        .tools
        .iter()
        .map(|tool| tool.name.as_str())
        .collect::<Vec<_>>();
    assert!(tool_names.contains(&"ToolSearch"));
    assert!(tool_names.contains(&"ToolSuggest"));
    assert!(tool_names.contains(&"Task"));
    assert!(tool_names.contains(&"Research"));
    assert!(tool_names.contains(&"Council"));
    assert!(tool_names.contains(&"AskModel"));
    assert!(!tool_names.contains(&"Bash"));
    assert!(!tool_names.contains(&"Read"));
    assert!(!tool_names.contains(&"TeamCreate"));
}

#[tokio::test]
async fn prepare_advertises_team_tools_for_delegation_prompt_regression() {
    let provider: Arc<dyn Provider> = Arc::new(TestProvider {
        name: "openai-test",
        convention: StreamConvention::OpenAiNative,
    });
    let request = prepare_stream_request(
        provider,
        &[user_text(
            "why do I have to nudge it to fire off subagents or a team?",
        )],
        &ModelId::new("test-model"),
        Default::default(),
    )
    .await;

    let tool_names = request
        .opts
        .tools
        .iter()
        .map(|tool| tool.name.as_str())
        .collect::<Vec<_>>();
    for expected in [
        "Task",
        "TeamCreate",
        "SendMessage",
        "TeamMemberMode",
        "ToolSearch",
        "ToolSuggest",
        "Research",
        "Council",
        "AskModel",
    ] {
        assert!(tool_names.contains(&expected), "missing {expected}");
    }

    let system = request.opts.system.as_deref().unwrap_or_default();
    assert!(system.contains("Task for subagents"), "{system}");
    assert!(system.contains("TeamCreate and SendMessage"), "{system}");
    assert!(
        system.contains("Subagents have isolated context"),
        "{system}"
    );
    assert!(
        system.contains("emit multiple `Task` calls in one response"),
        "{system}"
    );
    assert!(system.contains("MCP `isError` results"), "{system}");
    assert!(system.contains("Prefer MCP resources"), "{system}");
    assert!(system.contains("Preserve provenance"), "{system}");
    assert!(system.contains("cross-file integration pass"), "{system}");
    assert!(
        system.contains("Use direct execution for clear"),
        "{system}"
    );
}

#[tokio::test]
async fn prepare_does_not_advertise_commit_message_tool_for_commit_action_regression() {
    let provider: Arc<dyn Provider> = Arc::new(TestProvider {
        name: "openai-test",
        convention: StreamConvention::OpenAiNative,
    });
    let request = prepare_stream_request(
        provider,
        &[user_text("can you git commit and push please")],
        &ModelId::new("test-model"),
        Default::default(),
    )
    .await;

    let tool_names = request
        .opts
        .tools
        .iter()
        .map(|tool| tool.name.as_str())
        .collect::<Vec<_>>();
    assert!(tool_names.contains(&"Bash"));
    assert!(!tool_names.contains(&"SuggestCommitMessage"));

    let system = request.opts.system.as_deref().unwrap_or_default();
    assert!(!system.contains("### Commit messages"), "{system}");
}

#[tokio::test]
#[serial_test::serial]
async fn prepare_injects_total_tokens_reminder_countdown_normal() {
    let provider: Arc<dyn Provider> = Arc::new(TestProvider {
        name: "openai-test",
        convention: StreamConvention::OpenAiNative,
    });
    let overrides = crate::runtime::StreamRequestOverrides {
        last_usage_input_tokens: Some(150),
        context_window_tokens: Some(200),
        total_tokens_reminder_mode: Some(
            crate::total_tokens_reminder::TotalTokensReminderMode::Countdown,
        ),
        ..Default::default()
    };
    let request = prepare_stream_request(
        provider,
        &[user_text("write a small function")],
        &ModelId::new("test-model"),
        overrides,
    )
    .await;

    assert!(
        request
            .opts
            .system
            .as_deref()
            .unwrap_or_default()
            .contains("<total_tokens>50 tokens left</total_tokens>")
    );
}

#[tokio::test]
async fn prepare_uses_smaller_runtime_context_window_regression() {
    let provider: Arc<dyn Provider> = Arc::new(TestProvider {
        name: "anthropic",
        convention: StreamConvention::AnthropicNative,
    });
    let overrides = crate::runtime::StreamRequestOverrides {
        context_window_tokens: Some(200_000),
        ..Default::default()
    };
    let request = prepare_stream_request(
        provider,
        &[user_text("write a small function")],
        &ModelId::new("claude-opus-4-8"),
        overrides,
    )
    .await;

    assert_eq!(request.context_pressure.window_tokens, Some(200_000));
}

#[tokio::test]
#[serial_test::serial]
async fn prepare_applies_temperature_when_thinking_absent_normal() {
    let _guard = TemperatureGlobalGuard::set(0.8);
    let provider: Arc<dyn Provider> = Arc::new(TestProvider {
        name: "openai-test",
        convention: StreamConvention::OpenAiNative,
    });
    let request = prepare_stream_request(
        provider,
        &[user_text("write a small function")],
        &ModelId::new("test-model"),
        Default::default(),
    )
    .await;

    assert_eq!(request.opts.temperature, Some(0.8));
}

#[tokio::test]
#[serial_test::serial]
async fn prepare_skips_temperature_for_anthropic_thinking_regression() {
    let _guard = TemperatureGlobalGuard::set(0.8);
    let provider: Arc<dyn Provider> = Arc::new(TestProvider {
        name: "anthropic",
        convention: StreamConvention::AnthropicNative,
    });
    let request = prepare_stream_request(
        provider,
        &[user_text("write a small function")],
        &ModelId::new("claude-opus-4-8"),
        Default::default(),
    )
    .await;

    assert!(request.opts.adaptive_thinking);
    assert_eq!(request.opts.temperature, None);
}

#[tokio::test]
#[serial_test::serial]
async fn prepare_resolves_adaptive_exploration_to_anthropic_oauth_effort_normal() {
    let _guard = ExplorationGlobalGuard::set(ExplorationLevel::new(3));
    let provider: Arc<dyn Provider> = Arc::new(TestProvider {
        name: "anthropic-oauth",
        convention: StreamConvention::AnthropicNative,
    });
    let request = prepare_stream_request(
        provider,
        &[user_text("solve this hard algorithm problem")],
        &ModelId::new("claude-opus-4-8"),
        Default::default(),
    )
    .await;

    assert!(request.opts.adaptive_thinking);
    assert_eq!(request.opts.reasoning_effort.as_deref(), Some("xhigh"));
    assert_eq!(request.opts.temperature, None);
}
