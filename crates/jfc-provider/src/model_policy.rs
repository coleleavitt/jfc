use std::borrow::Cow;

use crate::StreamOptions;

pub const DEFAULT_MAX_OUTPUT_TOKENS: u32 = 16_384;
pub const LEGACY_ANTHROPIC_THINKING_BUDGET_TOKENS: u32 = 16_384;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderFamily {
    AnthropicNative,
    AnthropicProxy,
    OpenAiNative,
    OpenAiCompatible,
    Gemini,
    Unknown,
}

impl ProviderFamily {
    pub fn from_provider_name(provider: &str) -> Self {
        match provider.trim().to_ascii_lowercase().as_str() {
            "anthropic" | "anthropic-oauth" => Self::AnthropicNative,
            "bedrock" | "vertex" => Self::AnthropicProxy,
            "openai" | "codex" => Self::OpenAiNative,
            "openwebui" | "litellm" | "openrouter" => Self::OpenAiCompatible,
            "gemini" | "antigravity" => Self::Gemini,
            _ => Self::Unknown,
        }
    }

    pub fn can_send_anthropic_thinking(self) -> bool {
        matches!(self, Self::AnthropicNative)
    }

    pub fn can_send_openai_reasoning(self) -> bool {
        matches!(self, Self::OpenAiNative | Self::OpenAiCompatible)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelThinkingMode {
    Off,
    AnthropicLegacyBudget { budget_tokens: u32 },
    AnthropicAdaptive,
}

impl ModelThinkingMode {
    pub fn has_thinking_support(self) -> bool {
        !matches!(self, Self::Off)
    }

    pub fn supports_adaptive(self) -> bool {
        matches!(self, Self::AnthropicAdaptive)
    }

    pub fn apply_to(self, opts: StreamOptions) -> StreamOptions {
        match self {
            Self::Off => opts,
            Self::AnthropicLegacyBudget { budget_tokens } => opts.thinking(budget_tokens),
            Self::AnthropicAdaptive => opts.adaptive(),
        }
    }
}

pub trait ModelRequestPolicy {
    fn context_window_tokens(&self) -> Option<usize>;
    fn max_output_tokens(&self) -> Option<u32>;
    fn thinking_mode(&self) -> ModelThinkingMode;
    fn normalized_reasoning_effort<'a>(&self, requested: &'a str) -> Option<Cow<'a, str>>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnthropicModelKind {
    Fable5,
    Mythos5,
    Opus48,
    Opus47,
    Opus46,
    Opus45,
    Opus41,
    Opus4,
    Sonnet46,
    Sonnet45,
    Sonnet4,
    Sonnet37,
    Haiku45,
    Haiku35,
    Unknown,
}

impl AnthropicModelKind {
    pub fn from_model_id(model: &str) -> Self {
        let id = bare_model_id(model).to_ascii_lowercase();
        if id.contains("fable") {
            Self::Fable5
        } else if id.contains("mythos") {
            Self::Mythos5
        } else if id.contains("opus-4-8") {
            Self::Opus48
        } else if id.contains("opus-4-7") {
            Self::Opus47
        } else if id.contains("opus-4-6") {
            Self::Opus46
        } else if id.contains("opus-4-5") {
            Self::Opus45
        } else if id.contains("opus-4-1") {
            Self::Opus41
        } else if id.contains("opus-4") {
            Self::Opus4
        } else if id.contains("sonnet-4-6") {
            Self::Sonnet46
        } else if id.contains("sonnet-4-5") {
            Self::Sonnet45
        } else if id.contains("sonnet-4") {
            Self::Sonnet4
        } else if id.contains("3-7-sonnet") {
            Self::Sonnet37
        } else if id.contains("haiku-4-5") {
            Self::Haiku45
        } else if id.contains("3-5-haiku") {
            Self::Haiku35
        } else {
            Self::Unknown
        }
    }

    pub fn supports_effort(self) -> bool {
        matches!(
            self,
            Self::Fable5
                | Self::Mythos5
                | Self::Opus48
                | Self::Opus47
                | Self::Opus46
                | Self::Opus45
                | Self::Sonnet46
        )
    }

    /// Whether this model supports effort="max" (Opus 4.6+, Sonnet 4.6, Fable5, Mythos5).
    /// Other effort-capable models clamp max→high.
    pub fn supports_max_effort(self) -> bool {
        matches!(
            self,
            Self::Fable5
                | Self::Mythos5
                | Self::Opus48
                | Self::Opus47
                | Self::Opus46
                | Self::Sonnet46
        )
    }

    /// Whether this model supports effort="xhigh" (Opus 4.7+, Fable5, Mythos5).
    /// xhigh is a Claude Code internal value, not an official API value.
    pub fn supports_xhigh_effort(self) -> bool {
        matches!(
            self,
            Self::Fable5 | Self::Mythos5 | Self::Opus48 | Self::Opus47
        )
    }

    pub fn normalized_effort(self, requested: &str) -> Option<&str> {
        if std::env::var("CLAUDE_CODE_ALWAYS_ENABLE_EFFORT")
            .ok()
            .is_some_and(|v| !v.is_empty() && v != "0" && !v.eq_ignore_ascii_case("false"))
        {
            // Pass through verbatim when explicitly overridden.
            return Some(requested);
        }
        if !self.supports_effort() {
            return None;
        }
        // Anthropic API accepts: low, medium, high, max (model-dependent).
        // xhigh is Claude Code internal — always clamp to high before sending.
        // max is only supported on Opus 4.6+, Sonnet 4.6, Fable5, Mythos5.
        match requested {
            "xhigh" => {
                if self.supports_xhigh_effort() {
                    // xhigh maps to max for models that support it
                    Some("max")
                } else {
                    Some("high")
                }
            }
            "max" => {
                if self.supports_max_effort() {
                    Some("max")
                } else {
                    Some("high")
                }
            }
            _ => Some(requested),
        }
    }
}

impl ModelRequestPolicy for AnthropicModelKind {
    fn context_window_tokens(&self) -> Option<usize> {
        Some(match self {
            // 1M context: Fable5, Mythos5, Opus 4.6+, Sonnet 4.6
            Self::Fable5
            | Self::Mythos5
            | Self::Opus48
            | Self::Opus47
            | Self::Opus46
            | Self::Sonnet46 => 1_000_000,
            // 200K context: everything else
            Self::Opus45
            | Self::Opus41
            | Self::Opus4
            | Self::Sonnet45
            | Self::Sonnet4
            | Self::Sonnet37
            | Self::Haiku45
            | Self::Haiku35
            | Self::Unknown => 200_000,
        })
    }

    fn max_output_tokens(&self) -> Option<u32> {
        Some(match self {
            Self::Fable5
            | Self::Mythos5
            | Self::Opus48
            | Self::Opus47
            | Self::Opus46
            | Self::Sonnet46 => 128_000,
            Self::Opus45 | Self::Sonnet45 | Self::Sonnet4 | Self::Sonnet37 => 64_000,
            Self::Opus41 | Self::Opus4 | Self::Haiku45 => 32_000,
            Self::Haiku35 => 8_192,
            Self::Unknown => return None,
        })
    }

    fn thinking_mode(&self) -> ModelThinkingMode {
        match self {
            Self::Fable5
            | Self::Mythos5
            | Self::Opus48
            | Self::Opus47
            | Self::Opus46
            | Self::Sonnet46 => ModelThinkingMode::AnthropicAdaptive,
            Self::Sonnet45 | Self::Sonnet4 | Self::Sonnet37 => {
                ModelThinkingMode::AnthropicLegacyBudget {
                    budget_tokens: LEGACY_ANTHROPIC_THINKING_BUDGET_TOKENS,
                }
            }
            Self::Opus45
            | Self::Opus41
            | Self::Opus4
            | Self::Haiku45
            | Self::Haiku35
            | Self::Unknown => ModelThinkingMode::Off,
        }
    }

    fn normalized_reasoning_effort<'a>(&self, requested: &'a str) -> Option<Cow<'a, str>> {
        self.normalized_effort(requested).map(Cow::Borrowed)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpenAiModelKind {
    Gpt5,
    Gpt41,
    O1,
    O3,
    O4Mini,
    Unknown,
}

impl OpenAiModelKind {
    pub fn from_model_id(model: &str) -> Self {
        let id = bare_model_id(model).to_ascii_lowercase();
        if id.starts_with("gpt-5") {
            Self::Gpt5
        } else if id.starts_with("gpt-4.1") {
            Self::Gpt41
        } else if id.starts_with("o1") {
            Self::O1
        } else if id.starts_with("o3") {
            Self::O3
        } else if id.starts_with("o4-mini") || id.starts_with("o4") {
            Self::O4Mini
        } else {
            Self::Unknown
        }
    }

    pub fn supports_reasoning_effort(self) -> bool {
        matches!(self, Self::Gpt5 | Self::O1 | Self::O3 | Self::O4Mini)
    }
}

impl ModelRequestPolicy for OpenAiModelKind {
    fn context_window_tokens(&self) -> Option<usize> {
        Some(match self {
            Self::Gpt5 => 400_000,
            Self::Gpt41 => 1_000_000,
            Self::O1 | Self::O3 | Self::O4Mini => 200_000,
            Self::Unknown => return None,
        })
    }

    fn max_output_tokens(&self) -> Option<u32> {
        Some(match self {
            Self::Gpt5 => 128_000,
            Self::Gpt41 => 32_768,
            Self::O1 | Self::O3 | Self::O4Mini => 100_000,
            Self::Unknown => return None,
        })
    }

    fn thinking_mode(&self) -> ModelThinkingMode {
        ModelThinkingMode::Off
    }

    fn normalized_reasoning_effort<'a>(&self, requested: &'a str) -> Option<Cow<'a, str>> {
        if !self.supports_reasoning_effort() {
            return None;
        }
        match requested.trim().to_ascii_lowercase().as_str() {
            "max" => Some(Cow::Borrowed("xhigh")),
            _ => Some(Cow::Borrowed(requested)),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KnownModel {
    Anthropic(AnthropicModelKind),
    OpenAi(OpenAiModelKind),
    Unknown,
}

impl KnownModel {
    pub fn from_provider_model(provider: ProviderFamily, model: &str) -> Self {
        match provider {
            ProviderFamily::AnthropicNative | ProviderFamily::AnthropicProxy => {
                Self::Anthropic(AnthropicModelKind::from_model_id(model))
            }
            ProviderFamily::OpenAiNative | ProviderFamily::OpenAiCompatible => {
                let openai = OpenAiModelKind::from_model_id(model);
                if !matches!(openai, OpenAiModelKind::Unknown) {
                    Self::OpenAi(openai)
                } else if looks_like_claude(model) {
                    Self::Anthropic(AnthropicModelKind::from_model_id(model))
                } else {
                    Self::Unknown
                }
            }
            ProviderFamily::Gemini | ProviderFamily::Unknown => {
                if looks_like_claude(model) {
                    Self::Anthropic(AnthropicModelKind::from_model_id(model))
                } else {
                    Self::Unknown
                }
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelRequestProfile {
    pub provider: ProviderFamily,
    pub model: KnownModel,
    catalog_context_window_tokens: Option<usize>,
    catalog_max_output_tokens: Option<u32>,
}

impl ModelRequestProfile {
    pub fn from_provider_model(
        provider: &str,
        model: &str,
        catalog_context_window_tokens: Option<usize>,
        catalog_max_output_tokens: Option<usize>,
    ) -> Self {
        let provider = ProviderFamily::from_provider_name(provider);
        Self {
            provider,
            model: KnownModel::from_provider_model(provider, model),
            catalog_context_window_tokens,
            catalog_max_output_tokens: catalog_max_output_tokens
                .and_then(|v| u32::try_from(v).ok()),
        }
    }

    pub fn clamp_options(&self, mut opts: StreamOptions) -> StreamOptions {
        if let Some(max) = self.max_output_tokens() {
            opts.max_tokens = opts.max_tokens.min(max);
        }
        if let Some(requested) = opts.reasoning_effort.take() {
            opts.reasoning_effort = self
                .normalized_reasoning_effort(&requested)
                .map(|effort| effort.into_owned());
        }
        opts
    }
}

impl ModelRequestPolicy for ModelRequestProfile {
    fn context_window_tokens(&self) -> Option<usize> {
        self.catalog_context_window_tokens
            .or_else(|| match self.model {
                KnownModel::Anthropic(model) => model.context_window_tokens(),
                KnownModel::OpenAi(model) => model.context_window_tokens(),
                KnownModel::Unknown => None,
            })
    }

    fn max_output_tokens(&self) -> Option<u32> {
        self.catalog_max_output_tokens.or_else(|| match self.model {
            KnownModel::Anthropic(model) => model.max_output_tokens(),
            KnownModel::OpenAi(model) => model.max_output_tokens(),
            KnownModel::Unknown => Some(DEFAULT_MAX_OUTPUT_TOKENS),
        })
    }

    fn thinking_mode(&self) -> ModelThinkingMode {
        if !self.provider.can_send_anthropic_thinking() {
            return ModelThinkingMode::Off;
        }
        match self.model {
            KnownModel::Anthropic(model) => model.thinking_mode(),
            KnownModel::OpenAi(_) | KnownModel::Unknown => ModelThinkingMode::Off,
        }
    }

    fn normalized_reasoning_effort<'a>(&self, requested: &'a str) -> Option<Cow<'a, str>> {
        match self.model {
            KnownModel::Anthropic(model)
                if matches!(self.provider, ProviderFamily::AnthropicNative) =>
            {
                model.normalized_reasoning_effort(requested)
            }
            KnownModel::OpenAi(model) if self.provider.can_send_openai_reasoning() => {
                model.normalized_reasoning_effort(requested)
            }
            _ => None,
        }
    }
}

fn bare_model_id(model: &str) -> &str {
    model.rsplit('/').next().unwrap_or(model).trim()
}

fn looks_like_claude(model: &str) -> bool {
    let id = bare_model_id(model).to_ascii_lowercase();
    id.contains("claude")
        || id.contains("opus")
        || id.contains("sonnet")
        || id.contains("haiku")
        || id.contains("fable")
        || id.contains("mythos")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn anthropic_sonnet_45_uses_64k_output_regression() {
        let profile = ModelRequestProfile::from_provider_model(
            "anthropic-oauth",
            "claude-sonnet-4-5-20250929",
            None,
            None,
        );
        assert_eq!(profile.context_window_tokens(), Some(200_000));
        assert_eq!(profile.max_output_tokens(), Some(64_000));
        assert_eq!(
            profile.thinking_mode(),
            ModelThinkingMode::AnthropicLegacyBudget {
                budget_tokens: LEGACY_ANTHROPIC_THINKING_BUDGET_TOKENS,
            }
        );
        assert_eq!(profile.normalized_reasoning_effort("max"), None);
    }

    #[test]
    fn clamp_options_caps_sonnet_45_and_drops_effort_regression() {
        let profile = ModelRequestProfile::from_provider_model(
            "anthropic-oauth",
            "claude-sonnet-4-5-20250929",
            None,
            None,
        );
        let opts = profile.clamp_options(
            StreamOptions::new("claude-sonnet-4-5-20250929")
                .max_tokens(128_000)
                .reasoning_effort("max"),
        );
        assert_eq!(opts.max_tokens, 64_000);
        assert_eq!(opts.reasoning_effort, None);
    }

    #[test]
    fn anthropic_sonnet_46_uses_adaptive_and_128k_normal() {
        let profile =
            ModelRequestProfile::from_provider_model("anthropic", "claude-sonnet-4-6", None, None);
        assert_eq!(profile.max_output_tokens(), Some(128_000));
        assert_eq!(
            profile.thinking_mode(),
            ModelThinkingMode::AnthropicAdaptive
        );
        // Sonnet 4.6 supports max effort (per CC 177 ohH function)
        assert_eq!(
            profile.normalized_reasoning_effort("max").as_deref(),
            Some("max")
        );
        // But xhigh clamps to high (only Opus 4.7+ supports xhigh)
        assert_eq!(
            profile.normalized_reasoning_effort("xhigh").as_deref(),
            Some("high")
        );
    }

    #[test]
    fn catalog_limit_overrides_static_policy_normal() {
        let profile = ModelRequestProfile::from_provider_model(
            "anthropic",
            "claude-sonnet-4-6",
            Some(123_000),
            Some(7_000),
        );
        assert_eq!(profile.context_window_tokens(), Some(123_000));
        assert_eq!(profile.max_output_tokens(), Some(7_000));
    }

    #[test]
    fn openai_gpt5_effort_and_limits_normal() {
        let profile = ModelRequestProfile::from_provider_model("openai", "gpt-5.1", None, None);
        assert_eq!(profile.context_window_tokens(), Some(400_000));
        assert_eq!(profile.max_output_tokens(), Some(128_000));
        assert_eq!(
            profile.normalized_reasoning_effort("max").as_deref(),
            Some("xhigh")
        );
        assert_eq!(
            profile.normalized_reasoning_effort("high").as_deref(),
            Some("high")
        );
    }

    #[test]
    fn openai_non_reasoning_model_drops_effort_robust() {
        let profile = ModelRequestProfile::from_provider_model("openai", "gpt-4.1", None, None);
        assert_eq!(profile.max_output_tokens(), Some(32_768));
        assert_eq!(profile.normalized_reasoning_effort("high"), None);
    }

    #[test]
    fn openai_compatible_claude_does_not_send_anthropic_thinking_robust() {
        let profile =
            ModelRequestProfile::from_provider_model("openwebui", "claude-sonnet-4-5", None, None);
        assert_eq!(profile.max_output_tokens(), Some(64_000));
        assert_eq!(profile.thinking_mode(), ModelThinkingMode::Off);
        assert_eq!(profile.normalized_reasoning_effort("high"), None);
    }
}
