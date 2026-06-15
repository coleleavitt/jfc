//! Test-only convenience wrappers for model policy lookups.
//!
//! The canonical model policy types and trait implementations live in
//! `jfc-provider::model_policy`. These wrappers exist solely to give
//! jfc-engine's regression tests a terse API without re-exporting the
//! provider types into the public interface.

#[cfg(test)]
use jfc_provider::{
    DEFAULT_MAX_OUTPUT_TOKENS, ModelRequestPolicy, ModelRequestProfile, ModelThinkingMode,
};

#[cfg(test)]
pub type ThinkingMode = ModelThinkingMode;

#[cfg(test)]
pub fn model_profile_for(provider: &str, model: &str) -> ModelRequestProfile {
    ModelRequestProfile::from_provider_model(provider, model, None, None)
}

#[cfg(test)]
pub fn thinking_mode_for(provider: &str, model: &str) -> ThinkingMode {
    model_profile_for(provider, model).thinking_mode()
}

#[cfg(test)]
pub fn max_output_tokens_for(provider: &str, model: &str) -> u32 {
    model_profile_for(provider, model)
        .max_output_tokens()
        .unwrap_or(DEFAULT_MAX_OUTPUT_TOKENS)
}

#[cfg(test)]
mod tests {
    use super::*;
    use jfc_provider::LEGACY_ANTHROPIC_THINKING_BUDGET_TOKENS;

    #[test]
    fn sonnet_45_output_cap_is_64k_regression() {
        assert_eq!(
            max_output_tokens_for("anthropic-oauth", "claude-sonnet-4-5-20250929"),
            64_000
        );
    }

    #[test]
    fn sonnet_46_output_cap_is_128k_normal() {
        assert_eq!(
            max_output_tokens_for("anthropic-oauth", "claude-sonnet-4-6"),
            128_000
        );
    }

    #[test]
    fn anthropic_native_thinking_uses_model_kind_normal() {
        assert_eq!(
            thinking_mode_for("anthropic-oauth", "claude-sonnet-4-5"),
            ThinkingMode::AnthropicLegacyBudget {
                budget_tokens: LEGACY_ANTHROPIC_THINKING_BUDGET_TOKENS,
            }
        );
        assert_eq!(
            thinking_mode_for("anthropic-oauth", "claude-sonnet-4-6"),
            ThinkingMode::AnthropicAdaptive
        );
        assert_eq!(
            thinking_mode_for("anthropic-oauth", "claude-haiku-4-5"),
            ThinkingMode::Off
        );
    }

    #[test]
    fn openai_compatible_claude_skips_anthropic_thinking_robust() {
        assert_eq!(
            thinking_mode_for("openwebui", "claude-sonnet-4-6"),
            ThinkingMode::Off
        );
    }

    #[test]
    fn unknown_model_keeps_conservative_default_robust() {
        assert_eq!(
            max_output_tokens_for("custom-provider", "totally-new-model"),
            DEFAULT_MAX_OUTPUT_TOKENS
        );
    }
}
