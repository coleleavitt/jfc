use jfc_provider::{ModelRequestPolicy, ModelRequestProfile, StreamConvention, StreamOptions};

use super::{active_exploration_level, active_temperature};

pub fn apply_to_stream_options(
    mut opts: StreamOptions,
    model_profile: &ModelRequestProfile,
    provider_name: &str,
    convention: StreamConvention,
) -> StreamOptions {
    let has_anthropic_thinking = matches!(convention, StreamConvention::AnthropicNative)
        && (opts.adaptive_thinking || opts.thinking_budget.is_some());
    let oauth_locked_temperature = provider_name == "anthropic-oauth";
    let manual_effort = crate::effort::resolve_effort_for_request();
    let manual_temperature = active_temperature();

    if let Some(effort) = manual_effort {
        opts = opts.reasoning_effort(effort);
    }
    if let Some(temperature) = manual_temperature {
        if has_anthropic_thinking || oauth_locked_temperature {
            tracing::debug!(
                target: "jfc::exploration",
                temperature,
                has_anthropic_thinking,
                oauth_locked_temperature,
                "temperature pin not applied for this request shape"
            );
        } else {
            opts = opts.temperature(temperature);
        }
    }

    if opts.reasoning_effort.is_some() || opts.temperature.is_some() {
        return opts;
    }
    let Some(level) = active_exploration_level() else {
        return opts;
    };
    let requested_effort = level.to_effort().api_value();
    if let Some(effort) = model_profile.normalized_reasoning_effort(requested_effort) {
        opts = opts.reasoning_effort(effort.into_owned());
        tracing::debug!(
            target: "jfc::exploration",
            level = level.as_u8(),
            effort = %level.to_effort(),
            "adaptive exploration resolved to reasoning_effort"
        );
    } else if !has_anthropic_thinking && !oauth_locked_temperature {
        let temperature = level.to_temperature();
        opts = opts.temperature(temperature);
        tracing::debug!(
            target: "jfc::exploration",
            level = level.as_u8(),
            temperature,
            "adaptive exploration resolved to temperature"
        );
    }
    opts
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::exploration::{
        ExplorationLevel, set_exploration_level_global, set_temperature_global,
    };

    struct ExplorationGlobalGuard;

    impl ExplorationGlobalGuard {
        fn new() -> Self {
            set_temperature_global(None);
            set_exploration_level_global(None);
            crate::effort::set_turn_effort(None);
            crate::effort::EffortState::new().publish_global();
            Self
        }
    }

    impl Drop for ExplorationGlobalGuard {
        fn drop(&mut self) {
            set_temperature_global(None);
            set_exploration_level_global(None);
            crate::effort::set_turn_effort(None);
            crate::effort::EffortState::new().publish_global();
        }
    }

    fn profile(provider: &str, model: &str) -> ModelRequestProfile {
        ModelRequestProfile::from_provider_model(provider, model, None, None)
    }

    #[test]
    #[serial_test::serial]
    fn adaptive_resolves_to_effort_for_anthropic_thinking_regression() {
        let _guard = ExplorationGlobalGuard::new();
        set_exploration_level_global(Some(ExplorationLevel::new(2)));
        let opts = StreamOptions::new("claude-opus-4-8").adaptive();
        let profile = profile("anthropic", "claude-opus-4-8");

        let opts = apply_to_stream_options(
            opts,
            &profile,
            "anthropic",
            StreamConvention::AnthropicNative,
        );

        assert_eq!(opts.reasoning_effort.as_deref(), Some("high"));
        assert_eq!(opts.temperature, None);
    }

    #[test]
    #[serial_test::serial]
    fn adaptive_resolves_to_temperature_without_thinking_normal() {
        let _guard = ExplorationGlobalGuard::new();
        set_exploration_level_global(Some(ExplorationLevel::new(3)));
        let opts = StreamOptions::new("claude-haiku-4-5");
        let profile = profile("anthropic", "claude-haiku-4-5");

        let opts = apply_to_stream_options(
            opts,
            &profile,
            "anthropic",
            StreamConvention::AnthropicNative,
        );

        assert_eq!(opts.reasoning_effort, None);
        assert_eq!(opts.temperature, Some(0.75));
    }

    #[test]
    #[serial_test::serial]
    fn adaptive_resolves_to_effort_for_anthropic_oauth_regression() {
        let _guard = ExplorationGlobalGuard::new();
        set_exploration_level_global(Some(ExplorationLevel::new(3)));
        let opts = StreamOptions::new("claude-opus-4-8").adaptive();
        let profile = profile("anthropic-oauth", "claude-opus-4-8");

        let opts = apply_to_stream_options(
            opts,
            &profile,
            "anthropic-oauth",
            StreamConvention::AnthropicNative,
        );

        assert_eq!(opts.reasoning_effort.as_deref(), Some("xhigh"));
        assert_eq!(opts.temperature, None);
    }
}
