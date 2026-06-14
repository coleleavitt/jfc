use jfc_provider::StreamOptions;

const LEGACY_THINKING_BUDGET_TOKENS: u32 = 16_384;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThinkingMode {
    Off,
    LegacyBudget { budget_tokens: u32 },
    Adaptive,
}

impl ThinkingMode {
    pub fn has_thinking_support(self) -> bool {
        !matches!(self, Self::Off)
    }

    pub fn supports_adaptive(self) -> bool {
        matches!(self, Self::Adaptive)
    }

    pub fn apply_to(self, opts: StreamOptions) -> StreamOptions {
        match self {
            Self::Off => opts,
            Self::LegacyBudget { budget_tokens } => opts.thinking(budget_tokens),
            Self::Adaptive => opts.adaptive(),
        }
    }
}

pub fn thinking_mode_for(model: &str) -> ThinkingMode {
    if model_supports_adaptive_thinking(model) {
        ThinkingMode::Adaptive
    } else if model_supports_thinking(model) {
        ThinkingMode::LegacyBudget {
            budget_tokens: LEGACY_THINKING_BUDGET_TOKENS,
        }
    } else {
        ThinkingMode::Off
    }
}

/// Returns the max output tokens for `model`. Mirrors the
/// `getMaxOutputTokens` helper in opencode-anthropic-auth's
/// `plugin/constants.ts:195` and v126's MODEL_MAX_OUTPUT table.
///
/// Defaults are conservative; Opus/Sonnet 4.x family supports 128k
/// extended output (with the `output-128k-2025-02-19` beta header
/// already in our `ANTHROPIC_BETA` constant). Pre-4.x and Haiku get
/// 16k. Opus 4.0 dated releases are capped at 8k when not streaming
/// (we always stream so this is moot, but the constant stays as a
/// reference).
pub fn max_output_tokens_for(model: &str) -> u32 {
    let m = model.to_lowercase();
    // Opus/Sonnet 4.x family + the 2.1.170 fable-5/mythos-5 family --
    // extended-output 128k support.
    let extended_4x = m.contains("opus-4")
        || m.contains("sonnet-4")
        || m.contains("opus-5")
        || m.contains("sonnet-5")
        || m.contains("fable")
        || m.contains("mythos");
    if extended_4x {
        return 128_000;
    }
    // Haiku 4.5 caps at 16k.
    if m.contains("haiku-4-5") {
        return 16_384;
    }
    // Older Opus/Sonnet (3.x, 3.5, 3.7).
    if m.contains("opus") || m.contains("sonnet") {
        return 8_192;
    }
    // Unknown / proxy-routed: keep the safe v126 default.
    16_384
}

/// True only for proxy-routed model IDs (Bedrock through LiteLLM/OWUI,
/// Vertex, etc.). The Anthropic-native `thinking` field is rejected by
/// these proxies even when the underlying model is Claude -- Bedrock
/// uses its own `additionalModelRequestFields` schema for extended
/// thinking, and the OWUI/LiteLLM passthrough doesn't translate it. Mirrors
/// v126's provider-aware thinking gate (`shouldSendThinking` in cli.js).
fn is_proxy_routed_model(model: &str) -> bool {
    let m = model.to_lowercase();
    m.starts_with("bedrock-")
        || m.starts_with("aws-")
        || m.starts_with("vertex-")
        || m.starts_with("litellm-")
        || m.starts_with("openrouter-")
        || m.starts_with("openwebui-")
}

/// Returns true for models that require `{"type": "adaptive"}` thinking and
/// reject the legacy `budget_tokens` parameter. Matches v126's
/// `modelSupportsAdaptiveThinking` (claude.ts:1602). Proxy-routed
/// equivalents (bedrock-*, vertex-*) are excluded -- adaptive thinking is
/// an Anthropic-native parameter the proxies haven't adopted.
fn model_supports_adaptive_thinking(model: &str) -> bool {
    if is_proxy_routed_model(model) {
        return false;
    }
    let m = model.to_lowercase();
    // Opus 4.6, Opus 4.7, Sonnet 4.6 -- all reject budget_tokens.
    // Future models (5.x) will also use adaptive, so default to adaptive
    // for any model whose version segment is >= 4.6.
    m.contains("opus-4-6")
        || m.contains("opus-4-7")
        || m.contains("opus-4-8")
        || m.contains("opus-4-9")
        || m.contains("opus-5")
        || m.contains("sonnet-4-6")
        || m.contains("sonnet-4-7")
        || m.contains("sonnet-4-8")
        || m.contains("sonnet-4-9")
        || m.contains("sonnet-5")
        // Claude Code 2.1.170 fable-5/mythos-5: adaptive thinking required
        // (grouped with opus-4-6/7/8 + sonnet-4-6 in the cli's adaptive set).
        || m.contains("fable")
        || m.contains("mythos")
}

/// Returns true if the model supports thinking at all. Haiku 4.5 does NOT
/// support the thinking parameter -- sending it causes a 400. Opus 4.x and
/// Sonnet 4.5+ do support thinking. Proxy-routed model IDs (`bedrock-*`,
/// `aws-*`, `vertex-*`, `litellm-*`, `openrouter-*`) default to NOT
/// thinking even when the underlying model is Claude -- proxies frequently
/// reject the field with `400 invalid_request_error: adaptive thinking is
/// not supported on this model`. The user must explicitly opt back in via
/// config if a specific deployment supports it.
fn model_supports_thinking(model: &str) -> bool {
    if is_proxy_routed_model(model) {
        tracing::debug!(
            target: "jfc::stream",
            model,
            "model_supports_thinking: false (proxy-routed)"
        );
        return false;
    }
    let m = model.to_lowercase();
    // Opus 4.5 returns 400 "adaptive thinking is not supported on this
    // model" for both adaptive AND legacy budget_tokens -- the API
    // rejects the entire `thinking` field for that release. Other Opus
    // versions (4.6+) need adaptive thinking and are routed by the
    // `model_supports_adaptive_thinking` predicate first, so reaching
    // this branch with `opus-4-5` means we'd otherwise send the legacy
    // form and get a 400. Mark it as no-thinking so the request goes
    // through cleanly.
    if m.contains("opus-4-5") {
        return false;
    }
    // Known thinking-capable Anthropic-native families
    let supports = m.contains("opus")
        || m.contains("sonnet-4-5")
        || m.contains("sonnet-4-6")
        || m.contains("sonnet-4-7")
        || m.contains("sonnet-4-8")
        || m.contains("sonnet-4-9")
        || m.contains("sonnet-5")
        // Claude Code 2.1.170 fable-5/mythos-5 are thinking-capable.
        || m.contains("fable")
        || m.contains("mythos");
    tracing::debug!(
        target: "jfc::stream",
        model, supports,
        "model_supports_thinking"
    );
    supports
}

#[cfg(test)]
mod tests {
    use super::*;

    // Bedrock-routed Claude rejects `thinking` even though the underlying
    // model is Claude. Regression for the user's screenshot showing
    // `Anthropic API error 400: adaptive thinking is not supported on
    // this model` for `bedrock-claude-4-6-opus`.
    #[test]
    fn bedrock_routed_models_skip_thinking_robust() {
        assert!(!model_supports_thinking("bedrock-claude-4-6-opus"));
        assert!(!model_supports_thinking("bedrock-claude-3-5-sonnet"));
        assert!(!model_supports_adaptive_thinking("bedrock-claude-4-6-opus"));
        assert_eq!(
            thinking_mode_for("bedrock-claude-4-6-opus"),
            ThinkingMode::Off
        );
    }

    // Other proxy prefixes also default off -- none of them reliably
    // pass the thinking field through.
    #[test]
    fn other_proxy_prefixes_skip_thinking_robust() {
        assert!(!model_supports_thinking("vertex-claude-4-6-opus"));
        assert!(!model_supports_thinking("aws-claude-4-6-opus"));
        assert!(!model_supports_thinking("litellm-claude-4-6-opus"));
        assert!(!model_supports_thinking("openrouter-claude-4-6-opus"));
    }

    // Anthropic-native model IDs unchanged: they keep getting adaptive
    // thinking when version >= 4.6, legacy budget_tokens otherwise.
    #[test]
    fn anthropic_native_models_keep_thinking_normal() {
        assert!(model_supports_adaptive_thinking("claude-opus-4-6"));
        assert!(model_supports_adaptive_thinking("claude-opus-4-7"));
        assert!(model_supports_adaptive_thinking("claude-sonnet-4-6"));
        // Opus 4.5 rejects the entire `thinking` field -- see
        // `model_supports_thinking` for context. Excluded explicitly so
        // a regression doesn't silently put the request back into the
        // 400-loop.
        assert!(!model_supports_thinking("claude-opus-4-5"));
        assert!(model_supports_thinking("claude-opus-4-6"));
        assert_eq!(thinking_mode_for("claude-opus-4-6"), ThinkingMode::Adaptive);
    }

    // Haiku 4.5 doesn't support thinking at all on either path.
    #[test]
    fn haiku_excluded_robust() {
        assert!(!model_supports_thinking("claude-haiku-4-5"));
        assert!(!model_supports_adaptive_thinking("claude-haiku-4-5"));
    }

    // Normal: every documented proxy prefix returns true. The renderer +
    // stream pipeline decide whether to send `thinking` based on this gate,
    // so adding a new proxy means adding a row here.
    #[test]
    fn is_proxy_routed_recognizes_all_prefixes_normal() {
        for id in [
            "bedrock-claude-4-6-opus",
            "aws-claude-4-6-opus",
            "vertex-claude-4-6-opus",
            "litellm-claude-4-6-opus",
            "openrouter-claude-4-6-opus",
            "openwebui-claude-4-6-opus",
        ] {
            assert!(is_proxy_routed_model(id), "expected proxy match for {id}");
        }
    }

    // Robust: case-insensitive matching -- uppercase variants must still hit
    // the proxy rules. v126's gate normalizes via lowercase before checking.
    #[test]
    fn is_proxy_routed_is_case_insensitive_robust() {
        assert!(is_proxy_routed_model("BEDROCK-CLAUDE-4-6-OPUS"));
        assert!(is_proxy_routed_model("Vertex-Claude"));
    }

    // Robust: an Anthropic-native id (no proxy prefix) is NOT classified as
    // proxy-routed even though it contains substrings that look prefix-like.
    #[test]
    fn is_proxy_routed_native_anthropic_negative_robust() {
        assert!(!is_proxy_routed_model("claude-opus-4-7"));
        assert!(!is_proxy_routed_model("claude-sonnet-4-6"));
        assert!(!is_proxy_routed_model("claude-haiku-4-5"));
    }

    // Robust: empty string defaults to false -- the unknown-model code paths
    // shouldn't be tricked into the proxy branch by garbage inputs.
    #[test]
    fn is_proxy_routed_empty_returns_false_robust() {
        assert!(!is_proxy_routed_model(""));
    }

    // Normal: 4.x extended-output Opus / Sonnet -> 128k. The single test
    // validates each variant arm of the lowercase contains() chain.
    #[test]
    fn max_output_4x_extended_normal() {
        assert_eq!(max_output_tokens_for("claude-opus-4-7"), 128_000);
        assert_eq!(max_output_tokens_for("claude-opus-4-6"), 128_000);
        assert_eq!(max_output_tokens_for("claude-sonnet-4-6"), 128_000);
        assert_eq!(max_output_tokens_for("claude-sonnet-4-5"), 128_000);
        // Future-proofing: 5.x lands in the same bucket.
        assert_eq!(max_output_tokens_for("claude-opus-5-0"), 128_000);
        assert_eq!(max_output_tokens_for("claude-sonnet-5-0"), 128_000);
    }

    // Normal (CC 2.1.170): fable-5 / mythos-5 get the 128k extended-output
    // bucket and full thinking support (adaptive required, not legacy).
    #[test]
    fn fable_and_mythos_5_policy_normal() {
        for id in ["claude-fable-5", "claude-mythos-5"] {
            assert_eq!(max_output_tokens_for(id), 128_000, "{id} max output");
            assert!(model_supports_thinking(id), "{id} thinking");
            assert!(model_supports_adaptive_thinking(id), "{id} adaptive");
        }
        // Proxy-routed deployments still skip the native thinking field.
        assert!(!model_supports_adaptive_thinking("bedrock-claude-fable-5"));
    }

    // Normal: Haiku 4.5 caps at 16k -- distinct from Opus/Sonnet 4.x even
    // though both share the "4.5" version segment.
    #[test]
    fn max_output_haiku_4_5_normal() {
        assert_eq!(max_output_tokens_for("claude-haiku-4-5"), 16_384);
        assert_eq!(max_output_tokens_for("claude-haiku-4-5-20251001"), 16_384);
    }

    // Normal: 3.x families get 8k. Distinct from the 4.x branch above.
    #[test]
    fn max_output_legacy_opus_sonnet_normal() {
        assert_eq!(max_output_tokens_for("claude-3-7-sonnet-20250219"), 8_192);
        assert_eq!(max_output_tokens_for("claude-opus-3-5"), 8_192);
    }

    // Robust: an unknown / proxy-routed id falls through to the safe v126
    // default of 16k. Matches the comment-documented contract.
    #[test]
    fn max_output_unknown_falls_back_robust() {
        assert_eq!(max_output_tokens_for("bedrock-claude-mystery"), 16_384);
        assert_eq!(max_output_tokens_for("totally-new-model"), 16_384);
        assert_eq!(max_output_tokens_for(""), 16_384);
    }

    // Robust: case-insensitive -- the helper lowercases internally so
    // PascalCase or all-caps ids resolve correctly.
    #[test]
    fn max_output_case_insensitive_robust() {
        assert_eq!(max_output_tokens_for("CLAUDE-OPUS-4-7"), 128_000);
        assert_eq!(max_output_tokens_for("Claude-Haiku-4-5"), 16_384);
    }

    // Robust: Sonnet 4.4 (a hypothetical or pre-release) should NOT light up
    // adaptive thinking -- only 4.5+ Sonnet families do. Catches off-by-one
    // version bumps.
    #[test]
    fn sonnet_below_4_5_is_not_adaptive_robust() {
        assert!(!model_supports_adaptive_thinking("claude-sonnet-4-0"));
        assert!(!model_supports_adaptive_thinking("claude-sonnet-3-7"));
    }

    // Normal: legacy-thinking sonnet 4.5 returns true on the budget branch
    // (used as the second arm in stream_response after adaptive).
    #[test]
    fn sonnet_4_5_supports_thinking_normal() {
        assert!(model_supports_thinking("claude-sonnet-4-5"));
        assert!(model_supports_thinking("claude-sonnet-4-5-20250929"));
        assert_eq!(
            thinking_mode_for("claude-sonnet-4-5"),
            ThinkingMode::LegacyBudget {
                budget_tokens: LEGACY_THINKING_BUDGET_TOKENS,
            }
        );
    }
}
