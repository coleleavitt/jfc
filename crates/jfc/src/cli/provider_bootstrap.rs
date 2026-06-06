use std::sync::Arc;

use crate::providers::{
    AnthropicOAuthProvider, AnthropicProvider, AntigravityOAuthProvider, BedrockProvider,
    CodexOAuthProvider, GeminiApiProvider, LiteLLMProvider, OpenAIProvider, OpenRouterProvider,
    OpenWebUIProvider, VertexProvider,
};
use jfc_provider::{ModelId, ModelSpec, Provider, ProviderId};

/// Result of `build_providers()`. We keep a typed `Arc<AnthropicOAuthProvider>` next
/// to the trait-object list so the OAuth-specific profile fetch can run without
/// needing `Any`-style downcasting through the `Provider` trait.
pub(crate) struct ProvidersInit {
    pub(crate) providers: Vec<Arc<dyn Provider>>,
    pub(crate) active_idx: usize,
    pub(crate) model: ModelId,
    pub(crate) oauth: Option<Arc<AnthropicOAuthProvider>>,
}

pub(crate) struct ProviderModelResolution {
    pub(crate) provider: Arc<dyn Provider>,
    pub(crate) model: ModelId,
}

/// Build every provider that has usable config in this environment, plus pick which one
/// should be active at startup.
///
/// Active selection mirrors the prior single-provider precedence: explicit `ANTHROPIC_API_KEY`
/// wins, then `OPENWEBUI_BASE_URL`, then OAuth.
pub(crate) fn build_providers() -> ProvidersInit {
    // Cascade for the startup model id:
    //   1. ANTHROPIC_MODEL / OPENWEBUI_MODEL env (explicit override for one run)
    //   2. ~/.config/jfc/config.toml `[default].model` (the user's persisted choice)
    //   3. recent_models[0] (last model the user picked from the UI)
    //   4. hardcoded `claude-opus-4-5` (last-resort fallback)
    //
    // The config value may be a qualified `ModelSpec` like `"openwebui/bedrock-claude-4-6-opus"`
    // or a bare model id like `"claude-opus-4-7"`. When qualified, the provider prefix
    // directly routes to the matching provider — no heuristic guessing needed.
    let env_model = std::env::var("ANTHROPIC_MODEL")
        .ok()
        .or_else(|| std::env::var("OPENWEBUI_MODEL").ok())
        .or_else(|| std::env::var("OPENROUTER_MODEL").ok())
        .or_else(|| std::env::var("JFC_LITELLM_MODEL").ok())
        .filter(|s| !s.is_empty());
    let cfg_model = crate::config::load_arc()
        .default
        .model
        .clone()
        .filter(|s| !s.is_empty());
    let recent_model = crate::app::load_recent_models()
        .into_iter()
        .next()
        .filter(|s| !s.is_empty());
    let resolved_raw = env_model
        .or(cfg_model)
        .or(recent_model)
        .unwrap_or_else(|| "claude-opus-4-5".to_owned());

    // Parse as ModelSpec: "provider/model" or bare "model". Lenient because
    // `resolved_raw` came from an env var / config / recent-models entry — a
    // user-typed value that might contain stray slashes we'd rather treat as
    // part of a bare id than reject. `resolved_raw` is filtered non-empty
    // above, so the only `Err` path here is the empty-string guard.
    let spec: ModelSpec = ModelSpec::parse_lenient(&resolved_raw)
        .unwrap_or_else(|_| ModelSpec::bare(resolved_raw.clone()));
    tracing::info!(
        target: "jfc::startup",
        spec = %spec,
        provider_prefix = ?spec.provider().map(|p| p.as_str()),
        model_id = %spec.model(),
        "resolved startup model spec"
    );
    let model = spec.model().clone();

    let mut providers: Vec<Arc<dyn Provider>> = Vec::new();
    let mut prefer: Option<&'static str> = None;

    // Explicit env wins: ANTHROPIC_API_KEY → API-key provider as default.
    if let Ok(api_key) = std::env::var("ANTHROPIC_API_KEY") {
        providers.push(Arc::new(AnthropicProvider::new(api_key)));
        prefer.get_or_insert("anthropic");
    }

    // OAuth before OpenWebUI: when both stores exist (e.g. user runs opencode for
    // both auths), OAuth is what the model ids in `anthropic_models` actually serve.
    // Defaulting to OpenWebUI here caused "Model not found" because the seeded
    // `claude-sonnet-4-20250514` id doesn't exist on most OpenWebUI instances.
    let oauth_inst = AnthropicOAuthProvider::new();
    let oauth_arc = if oauth_inst.has_usable_config() {
        let arc = Arc::new(oauth_inst);
        providers.push(Arc::clone(&arc) as Arc<dyn Provider>);
        prefer.get_or_insert("anthropic-oauth");
        Some(arc)
    } else {
        None
    };

    if let Some(openai) = OpenAIProvider::from_env() {
        providers.push(Arc::new(openai));
        prefer.get_or_insert("openai");
    }

    if let Some(openrouter) = OpenRouterProvider::from_env() {
        providers.push(Arc::new(openrouter));
        prefer.get_or_insert("openrouter");
    }

    if let Some(litellm) = LiteLLMProvider::from_env() {
        providers.push(Arc::new(litellm));
        prefer.get_or_insert("litellm");
    }

    let codex = CodexOAuthProvider::new();
    if codex.has_usable_config() {
        providers.push(Arc::new(codex));
        prefer.get_or_insert("codex");
    }

    // OpenWebUI is registered as a candidate so its models show up in the picker, but
    // it only becomes the *default* when the user explicitly opts in via OPENWEBUI_BASE_URL.
    let openwebui = OpenWebUIProvider::new();
    let has_openwebui_config = openwebui.has_usable_config();
    if has_openwebui_config {
        providers.push(Arc::new(openwebui));
        if std::env::var("OPENWEBUI_BASE_URL").is_ok() {
            prefer.get_or_insert("openwebui");
        }
    }

    // Bedrock + Vertex: register as candidates when the wizard config is on
    // disk *and* the relevant CLI is installed. Neither becomes the default
    // — the user opts in by picking a Bedrock/Vertex model from the picker
    // or by using a `bedrock/<id>` / `vertex/<id>` qualified ModelSpec.
    let bedrock = BedrockProvider::new();
    if bedrock.has_usable_config() {
        tracing::info!(
            target: "jfc::startup",
            "registering Bedrock provider (config + aws CLI present)"
        );
        providers.push(Arc::new(bedrock));
    }
    let vertex = VertexProvider::new();
    if vertex.has_usable_config() {
        tracing::info!(
            target: "jfc::startup",
            "registering Vertex provider (config + gcloud CLI present)"
        );
        providers.push(Arc::new(vertex));
    }

    // Antigravity OAuth: Google AI Pro / Gemini 3 + Claude via Code Assist API.
    let antigravity = AntigravityOAuthProvider::new();
    if antigravity.has_usable_config() {
        tracing::info!(
            target: "jfc::startup",
            "registering Antigravity provider (OAuth tokens present)"
        );
        providers.push(Arc::new(antigravity));
    }

    // Direct Gemini API key: simplest path for users with a Google AI Studio key.
    if let Some(gemini) = GeminiApiProvider::from_env() {
        tracing::info!(
            target: "jfc::startup",
            "registering Gemini API provider (GEMINI_API_KEY set)"
        );
        providers.push(Arc::new(gemini));
        prefer.get_or_insert("gemini");
    }

    if providers.is_empty() {
        // Last-resort fallback so we don't panic on empty list — OAuth provider will
        // surface a clean "no accounts" error on first stream.
        providers.push(Arc::new(AnthropicOAuthProvider::new()));
        prefer = Some("anthropic-oauth");
    }

    // Provider routing — three-tier:
    //
    // 1. **Explicit prefix** (from ModelSpec): `"openwebui/bedrock-claude-4-6-opus"`
    //    → directly look up provider named "openwebui". No guessing.
    //
    // 2. **Static catalogue match**: scan each provider's `available_models()` for
    //    an id matching the model portion. First match wins.
    //
    // 3. **Heuristic fallback**: if no static match AND OpenWebUI is configured AND
    //    the model id doesn't look Anthropic-native (`claude-…`), route to OpenWebUI
    //    as the catch-all proxy whose catalogue is populated at runtime.
    //
    // Without any of these matching, fall back to the env-var precedence (`prefer`).
    let model_str = model.as_str();

    let provider_for_model: Option<String> = if let Some(prefix) = spec.provider() {
        // Tier 1: explicit provider prefix from config
        tracing::info!(
            target: "jfc::startup",
            model = %model_str,
            explicit_provider = %prefix,
            "model spec has explicit provider prefix — routing directly"
        );
        Some(prefix.as_str().to_owned())
    } else {
        // Tier 2: static catalogue lookup
        let static_match: Option<String> = providers
            .iter()
            .find(|p| {
                p.available_models()
                    .iter()
                    .any(|m| m.id.as_str() == model_str)
            })
            .map(|p| p.name().to_owned());

        static_match.or_else(|| {
            // Tier 3: heuristic — OpenAI-looking ids route to OpenAI, then
            // non-`claude-` ids route to OpenWebUI proxy when configured.
            let has_codex_config = providers.iter().any(|p| p.name() == "codex");
            if has_codex_config && looks_codex_model(model_str) {
                tracing::info!(
                    target: "jfc::startup",
                    model = %model_str,
                    "no static match, model looks Codex-native → codex"
                );
                return Some("codex".to_owned());
            }

            let has_openai_config = providers.iter().any(|p| p.name() == "openai");
            if has_openai_config && looks_openai_model(model_str) {
                tracing::info!(
                    target: "jfc::startup",
                    model = %model_str,
                    "no static match, model looks OpenAI-native → openai"
                );
                return Some("openai".to_owned());
            }

            let has_openrouter_config = providers.iter().any(|p| p.name() == "openrouter");
            if has_openrouter_config && looks_openrouter_model(model_str) {
                tracing::info!(
                    target: "jfc::startup",
                    model = %model_str,
                    "no static match, model looks OpenRouter-native → openrouter"
                );
                return Some("openrouter".to_owned());
            }

            let has_litellm_config = providers.iter().any(|p| p.name() == "litellm");
            let looks_proxy_routed = !model_str.is_empty() && !model_str.starts_with("claude-");
            if has_litellm_config && looks_proxy_routed {
                tracing::info!(
                    target: "jfc::startup",
                    model = %model_str,
                    "no static match, model looks proxy-routed → litellm"
                );
                return Some("litellm".to_owned());
            }

            if has_openwebui_config && looks_proxy_routed {
                tracing::info!(
                    target: "jfc::startup",
                    model = %model_str,
                    "no static match, model looks proxy-routed → openwebui"
                );
                Some("openwebui".to_owned())
            } else {
                None
            }
        })
    };

    if let Some(name) = provider_for_model.as_deref() {
        tracing::info!(
            target: "jfc::startup",
            model = %model_str,
            matched_provider = %name,
            "routed startup model to its owning provider"
        );
    }

    let active_idx = provider_for_model
        .as_deref()
        .or(prefer)
        .and_then(|name| providers.iter().position(|p| p.name() == name))
        .unwrap_or(0);

    ProvidersInit {
        providers,
        active_idx,
        model,
        oauth: oauth_arc,
    }
}

/// Route a model id to the provider that owns it.
///
/// Accepts either a qualified `"provider/model"` spec or a bare `"model"` id.
/// When qualified, looks up the provider by name directly. Otherwise uses the
/// same three-tier logic as `build_providers`: static catalogue → heuristic.
///
/// Used by `--continue`/`--resume` to re-route when the saved session's model
/// belongs to a different provider than the env-var precedence picked.
pub(crate) fn resolve_provider_model(
    providers: &[Arc<dyn Provider>],
    model_id: &str,
) -> Option<ProviderModelResolution> {
    if model_id.is_empty() {
        return None;
    }
    // Try parsing as ModelSpec — if qualified, route directly by provider name
    // and strip the prefix before the model id is sent to the provider.
    if let Ok(spec) = model_id.parse::<ModelSpec>()
        && let Some(prefix) = spec.provider()
    {
        return providers
            .iter()
            .find(|p| p.name() == prefix.as_str())
            .cloned()
            .map(|provider| ProviderModelResolution {
                provider,
                model: spec.model().clone(),
            });
    }
    let model = ModelId::new(model_id);
    // Tier 2: static catalogue lookup
    if let Some(p) = providers.iter().find(|p| {
        p.available_models()
            .iter()
            .any(|m| m.id.as_str() == model_id)
    }) {
        return Some(ProviderModelResolution {
            provider: Arc::clone(p),
            model,
        });
    }
    // Tier 3: heuristic — OpenAI-looking ids route to OpenAI first, then
    // non-`claude-` ids route to OpenWebUI proxy.
    let has_codex = providers.iter().any(|p| p.name() == "codex");
    if has_codex && looks_codex_model(model_id) {
        return providers
            .iter()
            .find(|p| p.name() == "codex")
            .cloned()
            .map(|provider| ProviderModelResolution { provider, model });
    }

    let has_openai = providers.iter().any(|p| p.name() == "openai");
    if has_openai && looks_openai_model(model_id) {
        return providers
            .iter()
            .find(|p| p.name() == "openai")
            .cloned()
            .map(|provider| ProviderModelResolution { provider, model });
    }

    let has_openrouter = providers.iter().any(|p| p.name() == "openrouter");
    if has_openrouter && looks_openrouter_model(model_id) {
        return providers
            .iter()
            .find(|p| p.name() == "openrouter")
            .cloned()
            .map(|provider| ProviderModelResolution { provider, model });
    }

    let has_litellm = providers.iter().any(|p| p.name() == "litellm");
    if has_litellm && !model_id.starts_with("claude-") {
        return providers
            .iter()
            .find(|p| p.name() == "litellm")
            .cloned()
            .map(|provider| ProviderModelResolution { provider, model });
    }

    let has_openwebui = providers.iter().any(|p| p.name() == "openwebui");
    if has_openwebui && !model_id.starts_with("claude-") && !model_id.starts_with("gemini") {
        return providers
            .iter()
            .find(|p| p.name() == "openwebui")
            .cloned()
            .map(|provider| ProviderModelResolution { provider, model });
    }

    // Tier 3b: gemini-prefixed ids route to the gemini or antigravity provider.
    if model_id.starts_with("gemini") {
        if let Some(p) = providers.iter().find(|p| p.name() == "antigravity") {
            return Some(ProviderModelResolution {
                provider: Arc::clone(p),
                model,
            });
        }
        if let Some(p) = providers.iter().find(|p| p.name() == "gemini") {
            return Some(ProviderModelResolution {
                provider: Arc::clone(p),
                model,
            });
        }
    }
    None
}

pub(crate) fn provider_for_model(
    providers: &[Arc<dyn Provider>],
    model_id: &str,
) -> Option<Arc<dyn Provider>> {
    resolve_provider_model(providers, model_id).map(|r| r.provider)
}

pub(crate) fn qualified_model_id(provider: &dyn Provider, model: &ModelId) -> String {
    ModelSpec::qualified(ProviderId::new(provider.name()), model.clone()).to_string()
}

fn looks_openai_model(model_id: &str) -> bool {
    model_id.starts_with("gpt-")
        || model_id.starts_with("o1")
        || model_id.starts_with("o3")
        || model_id.starts_with("o4")
}

fn looks_codex_model(model_id: &str) -> bool {
    let id = model_id
        .rsplit('/')
        .next()
        .unwrap_or(model_id)
        .to_ascii_lowercase();
    id.contains("codex")
}

fn looks_openrouter_model(model_id: &str) -> bool {
    let Some((vendor, routed_model)) = model_id.split_once('/') else {
        return false;
    };
    !routed_model.is_empty()
        && matches!(
            vendor,
            "anthropic"
                | "openai"
                | "google"
                | "meta-llama"
                | "mistralai"
                | "qwen"
                | "deepseek"
                | "x-ai"
        )
}
