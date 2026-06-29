use std::sync::Arc;

use crate::providers::{
    AnthropicOAuthProvider, AnthropicProvider, AntigravityOAuthProvider, CodexOAuthProvider,
    GeminiApiProvider, LiteLLMProvider, OpenAIProvider, OpenRouterProvider, OpenWebUIProvider,
    VertexProvider,
};
use crate::runtime::{
    ProviderModelResolution, ProviderRegistry, ProviderRegistryError, RuntimeService,
};
use jfc_provider::{
    ModelId, ModelResolutionReason, ModelSpec, Provider, ProviderId, ResolvedModel,
};

/// Result of `build_providers()`. We keep a typed `Arc<AnthropicOAuthProvider>` next
/// to the trait-object list so the OAuth-specific profile fetch can run without
/// needing `Any`-style downcasting through the `Provider` trait.
pub struct ProvidersInit {
    pub providers: Vec<Arc<dyn Provider>>,
    pub active_idx: usize,
    pub model: ModelId,
    pub oauth: Option<Arc<AnthropicOAuthProvider>>,
    pub provider_plugin_runtime: jfc_plugin_host::PluginRuntime,
    pub provider_plugin_diagnostics: jfc_plugin_host::PluginHostDiagnostics,
}

/// Fire-and-forget TLS/DNS preconnect warmup for the active provider.
///
/// Must be called from an async context (a live Tokio runtime). Spawns a
/// detached task that issues a cheap HEAD request to the active provider's
/// base origin so DNS + TCP + TLS are complete before the user's first turn.
/// Errors are silently swallowed — warmup must never affect startup.
///
/// Skipped when:
/// - `JFC_DISABLE_CONNECT_WARMUP=1` is set (env-var opt-out)
/// - The active provider returns `None` from `warmup_url()` / `http_client()`
///   (Bedrock shells out; no-pool providers like the last-resort OAuth fallback)
pub fn spawn_active_provider_warmup(init: &ProvidersInit) {
    let provider = &*init.providers[init.active_idx];
    jfc_provider::http::spawn_connect_warmup(provider);
}

pub struct BuiltInProviderRegistry {
    providers: Vec<Arc<dyn Provider>>,
}

impl BuiltInProviderRegistry {
    pub fn new(providers: Vec<Arc<dyn Provider>>) -> Self {
        Self { providers }
    }
}

impl RuntimeService for BuiltInProviderRegistry {
    fn service_name(&self) -> &'static str {
        "built-in-provider-registry"
    }
}

impl ProviderRegistry for BuiltInProviderRegistry {
    fn resolve_provider_model(
        &self,
        model_id: &str,
    ) -> Result<ProviderModelResolution, ProviderRegistryError> {
        resolve_provider_model_from_slice(&self.providers, model_id)
    }
}

pub fn register_first_party_provider_pack(
    host: &mut jfc_plugin_host::PluginHost,
) -> Result<(), jfc_plugin_host::PluginHostError> {
    let plugin_id = jfc_plugin_sdk::PluginId::new(jfc_providers::BUILTIN_PROVIDER_PACK_ID);
    host.register_internal(
        jfc_plugin_host::PluginRegistration::new(jfc_plugin_sdk::PluginManifest::new(
            plugin_id,
            jfc_plugin_sdk::PluginVersion::new(env!("CARGO_PKG_VERSION")),
            jfc_plugin_sdk::PluginSource::built_in("first-party-providers"),
        ))
        .with_provider_descriptors(jfc_providers::first_party_provider_descriptors()),
    )
}

pub fn first_party_provider_plugin_host()
-> Result<jfc_plugin_host::PluginHost, jfc_plugin_host::PluginHostError> {
    let mut host = jfc_plugin_host::PluginHost::new();
    register_first_party_provider_pack(&mut host)?;
    host.activate_all()?;
    Ok(host)
}

/// Build every provider that has usable config in this environment, plus pick which one
/// should be active at startup.
///
/// Active selection mirrors the prior single-provider precedence: explicit `ANTHROPIC_API_KEY`
/// wins, then `OPENWEBUI_BASE_URL`, then OAuth.
pub fn build_providers() -> ProvidersInit {
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
    let provider_plugin_host = first_party_provider_plugin_host()
        .expect("first-party provider pack registration is static and must activate");
    let provider_plugin_diagnostics = provider_plugin_host.diagnostics();
    let provider_plugin_runtime = jfc_plugin_host::PluginRuntime::from_host(&provider_plugin_host)
        .expect("first-party provider pack descriptors must be unique");

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

    // Vertex registers when configured. Bedrock stays hidden until its
    // streaming path is implemented; otherwise the picker advertises models
    // that fail on the first request.
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

    let project_root = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
    let plugin_provider_count = super::provider_descriptors::append_discovered_provider_plugins(
        &mut providers,
        &project_root,
    );
    if plugin_provider_count > 0 {
        tracing::info!(
            target: "jfc::startup",
            count = plugin_provider_count,
            "registered plugin provider descriptors"
        );
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
        provider_plugin_runtime,
        provider_plugin_diagnostics,
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
pub fn resolve_provider_model(
    providers: &[Arc<dyn Provider>],
    model_id: &str,
) -> Option<ProviderModelResolution> {
    BuiltInProviderRegistry::new(providers.to_vec())
        .resolve_provider_model(model_id)
        .ok()
}

fn resolve_provider_model_from_slice(
    providers: &[Arc<dyn Provider>],
    model_id: &str,
) -> Result<ProviderModelResolution, ProviderRegistryError> {
    if model_id.is_empty() {
        return Err(ProviderRegistryError::MissingProvider {
            model: model_id.to_owned(),
        });
    }
    // Try parsing as ModelSpec — if qualified, route directly by provider name
    // and strip the prefix before the model id is sent to the provider.
    if let Ok(spec) = model_id.parse::<ModelSpec>()
        && let Some(prefix) = spec.provider()
    {
        return providers
            .iter()
            .find(|p| p.name() == prefix.as_str())
            .or_else(|| {
                providers
                    .iter()
                    .find(|p| provider_name_matches_request(p.name(), prefix.as_str()))
            })
            .cloned()
            .map(|provider| {
                provider_resolution(
                    provider,
                    spec.clone(),
                    spec.model().clone(),
                    ModelResolutionReason::ExplicitProvider,
                )
            })
            .ok_or_else(|| ProviderRegistryError::MissingProvider {
                model: model_id.to_owned(),
            });
    }
    let model = ModelId::new(model_id);
    // Tier 2: static catalogue lookup
    for p in providers {
        let models = p.available_models();
        if models.iter().any(|m| m.id.as_str() == model_id) {
            return Ok(provider_resolution(
                Arc::clone(p),
                ModelSpec::bare(model_id),
                model,
                ModelResolutionReason::CatalogMatch,
            ));
        }
    }
    // Tier 3: heuristic — OpenAI-looking ids route to OpenAI first, then
    // non-`claude-` ids route to OpenWebUI proxy.
    let has_codex = providers.iter().any(|p| p.name() == "codex");
    if has_codex && looks_codex_model(model_id) {
        return providers
            .iter()
            .find(|p| p.name() == "codex")
            .cloned()
            .map(|provider| {
                provider_resolution(
                    provider,
                    ModelSpec::bare(model_id),
                    model.clone(),
                    ModelResolutionReason::Heuristic {
                        rule: "codex-looking model id".to_owned(),
                    },
                )
            })
            .ok_or_else(|| ProviderRegistryError::MissingProvider {
                model: model_id.to_owned(),
            });
    }

    let has_openai = providers.iter().any(|p| p.name() == "openai");
    if has_openai && looks_openai_model(model_id) {
        return providers
            .iter()
            .find(|p| p.name() == "openai")
            .cloned()
            .map(|provider| {
                provider_resolution(
                    provider,
                    ModelSpec::bare(model_id),
                    model.clone(),
                    ModelResolutionReason::Heuristic {
                        rule: "openai-looking model id".to_owned(),
                    },
                )
            })
            .ok_or_else(|| ProviderRegistryError::MissingProvider {
                model: model_id.to_owned(),
            });
    }

    let has_openrouter = providers.iter().any(|p| p.name() == "openrouter");
    if has_openrouter && looks_openrouter_model(model_id) {
        return providers
            .iter()
            .find(|p| p.name() == "openrouter")
            .cloned()
            .map(|provider| {
                provider_resolution(
                    provider,
                    ModelSpec::bare(model_id),
                    model.clone(),
                    ModelResolutionReason::Heuristic {
                        rule: "openrouter vendor/model id".to_owned(),
                    },
                )
            })
            .ok_or_else(|| ProviderRegistryError::MissingProvider {
                model: model_id.to_owned(),
            });
    }

    let has_litellm = providers.iter().any(|p| p.name() == "litellm");
    if has_litellm && !model_id.starts_with("claude-") {
        return providers
            .iter()
            .find(|p| p.name() == "litellm")
            .cloned()
            .map(|provider| {
                provider_resolution(
                    provider,
                    ModelSpec::bare(model_id),
                    model.clone(),
                    ModelResolutionReason::Heuristic {
                        rule: "proxy-routed non-claude model id".to_owned(),
                    },
                )
            })
            .ok_or_else(|| ProviderRegistryError::MissingProvider {
                model: model_id.to_owned(),
            });
    }

    let has_openwebui = providers.iter().any(|p| p.name() == "openwebui");
    if has_openwebui && !model_id.starts_with("claude-") && !model_id.starts_with("gemini") {
        return providers
            .iter()
            .find(|p| p.name() == "openwebui")
            .cloned()
            .map(|provider| {
                provider_resolution(
                    provider,
                    ModelSpec::bare(model_id),
                    model.clone(),
                    ModelResolutionReason::Heuristic {
                        rule: "openwebui catch-all proxy model id".to_owned(),
                    },
                )
            })
            .ok_or_else(|| ProviderRegistryError::MissingProvider {
                model: model_id.to_owned(),
            });
    }

    // Tier 3b: gemini-prefixed ids route to the gemini or antigravity provider.
    if model_id.starts_with("gemini") {
        if let Some(p) = providers.iter().find(|p| p.name() == "antigravity") {
            return Ok(provider_resolution(
                Arc::clone(p),
                ModelSpec::bare(model_id),
                model,
                ModelResolutionReason::Heuristic {
                    rule: "gemini-prefixed id routed to antigravity".to_owned(),
                },
            ));
        }
        if let Some(p) = providers.iter().find(|p| p.name() == "gemini") {
            return Ok(provider_resolution(
                Arc::clone(p),
                ModelSpec::bare(model_id),
                model,
                ModelResolutionReason::Heuristic {
                    rule: "gemini-prefixed id".to_owned(),
                },
            ));
        }
    }
    Err(ProviderRegistryError::MissingProvider {
        model: model_id.to_owned(),
    })
}

pub fn provider_name_matches_request(provider_name: &str, requested_provider: &str) -> bool {
    provider_name == requested_provider
        || matches!(
            (provider_name, requested_provider),
            ("anthropic", "anthropic-oauth") | ("anthropic-oauth", "anthropic")
        )
}

fn provider_resolution(
    provider: Arc<dyn Provider>,
    requested: ModelSpec,
    model: ModelId,
    reason: ModelResolutionReason,
) -> ProviderModelResolution {
    let info = provider
        .available_models()
        .into_iter()
        .find(|info| info.id == model);
    let effective = ModelSpec::qualified(ProviderId::new(provider.name()), model.clone());
    let resolved_model = ResolvedModel::new(requested, effective, reason, info.as_ref());
    ProviderModelResolution {
        provider,
        model,
        resolved_model,
    }
}

pub fn provider_for_model(
    providers: &[Arc<dyn Provider>],
    model_id: &str,
) -> Option<Arc<dyn Provider>> {
    resolve_provider_model(providers, model_id).map(|r| r.provider)
}

pub fn qualified_model_id(provider: &dyn Provider, model: &ModelId) -> String {
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

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use jfc_provider::{EventStream, ModelInfo, ProviderMessage, StreamOptions};

    struct NamedProvider(&'static str);

    struct CatalogProvider {
        name: &'static str,
        model: &'static str,
    }

    #[async_trait::async_trait]
    impl Provider for NamedProvider {
        fn name(&self) -> &str {
            self.0
        }

        fn available_models(&self) -> Vec<ModelInfo> {
            Vec::new()
        }

        async fn stream(
            &self,
            _messages: Vec<ProviderMessage>,
            _options: &StreamOptions,
        ) -> anyhow::Result<EventStream> {
            anyhow::bail!("unused")
        }
    }

    impl jfc_provider::seal::Sealed for NamedProvider {}

    #[async_trait::async_trait]
    impl Provider for CatalogProvider {
        fn name(&self) -> &str {
            self.name
        }

        fn available_models(&self) -> Vec<ModelInfo> {
            vec![ModelInfo::new(self.model, "Catalog Model", self.name)]
        }

        async fn stream(
            &self,
            _messages: Vec<ProviderMessage>,
            _options: &StreamOptions,
        ) -> anyhow::Result<EventStream> {
            anyhow::bail!("unused")
        }
    }

    impl jfc_provider::seal::Sealed for CatalogProvider {}

    #[test]
    fn resolve_provider_model_routes_anthropic_prefix_to_oauth_regression() {
        let providers: Vec<Arc<dyn Provider>> = vec![Arc::new(NamedProvider("anthropic-oauth"))];

        let resolved = resolve_provider_model(&providers, "anthropic/claude-opus-4-6")
            .expect("anthropic prefix should route to OAuth Anthropic provider");

        assert_eq!(resolved.provider.name(), "anthropic-oauth");
        assert_eq!(resolved.model.as_str(), "claude-opus-4-6");
    }

    #[test]
    fn resolve_provider_model_prefers_exact_anthropic_provider_normal() {
        let providers: Vec<Arc<dyn Provider>> = vec![
            Arc::new(NamedProvider("anthropic-oauth")),
            Arc::new(NamedProvider("anthropic")),
        ];

        let resolved = resolve_provider_model(&providers, "anthropic/claude-opus-4-6")
            .expect("exact Anthropic provider should resolve");

        assert_eq!(resolved.provider.name(), "anthropic");
        assert_eq!(resolved.model.as_str(), "claude-opus-4-6");
    }

    #[test]
    fn built_in_provider_registry_selection_uses_current_provider_catalog_normal() {
        let registry = BuiltInProviderRegistry::new(vec![Arc::new(CatalogProvider {
            name: "anthropic",
            model: "claude-opus-4-8",
        })]);

        let resolved = registry
            .resolve_provider_model("claude-opus-4-8")
            .expect("catalog model should resolve through current provider registry");

        assert_eq!(resolved.provider.name(), "anthropic");
        assert_eq!(resolved.model.as_str(), "claude-opus-4-8");
    }

    #[test]
    fn built_in_provider_registry_missing_provider_returns_typed_error_robust() {
        let registry = BuiltInProviderRegistry::new(vec![Arc::new(NamedProvider("anthropic"))]);

        let error = registry
            .resolve_provider_model("missing-model")
            .expect_err("unknown model should not resolve silently");

        assert_eq!(
            error,
            ProviderRegistryError::MissingProvider {
                model: "missing-model".to_owned()
            }
        );
    }

    #[test]
    fn built_in_provider_registry_selects_openai_family_catalog_normal() {
        let registry = BuiltInProviderRegistry::new(vec![
            Arc::new(NamedProvider("anthropic")),
            Arc::new(CatalogProvider {
                name: "openai",
                model: "gpt-5.1",
            }),
        ]);

        let resolved = registry
            .resolve_provider_model("gpt-5.1")
            .expect("openai catalog model should resolve");

        assert_eq!(resolved.provider.name(), "openai");
        assert_eq!(resolved.model.as_str(), "gpt-5.1");
        assert!(matches!(
            resolved.resolved_model.reason,
            ModelResolutionReason::CatalogMatch
        ));
    }

    #[test]
    fn built_in_provider_registry_does_not_cross_route_missing_openai_family_prefix_robust() {
        let registry = BuiltInProviderRegistry::new(vec![Arc::new(CatalogProvider {
            name: "openai",
            model: "gpt-5.1",
        })]);

        let error = registry
            .resolve_provider_model("openrouter/openai/gpt-5.1")
            .expect_err("missing explicit OpenRouter provider should not fall back to OpenAI");

        assert_eq!(
            error,
            ProviderRegistryError::MissingProvider {
                model: "openrouter/openai/gpt-5.1".to_owned()
            }
        );
    }

    #[test]
    fn first_party_provider_pack_reaches_runtime_maps_and_status_normal() {
        let mut host = jfc_plugin_host::PluginHost::new();
        register_first_party_provider_pack(&mut host).expect("provider pack registers");
        host.activate_all().expect("provider pack activates");

        let runtime = jfc_plugin_host::PluginRuntime::from_host(&host).expect("runtime builds");
        let descriptor = runtime
            .providers()
            .get("anthropic")
            .expect("anthropic provider descriptor is mapped")
            .descriptor();

        assert_eq!(
            descriptor.plugin_id.as_str(),
            jfc_providers::BUILTIN_PROVIDER_PACK_ID
        );
        assert_eq!(descriptor.provider, "anthropic");
        assert!(
            descriptor
                .models
                .iter()
                .any(|model| model.id == "claude-opus-4-8")
        );

        let diagnostics = host.diagnostics();
        let openai_descriptor = runtime
            .providers()
            .get("openai")
            .expect("openai provider descriptor is mapped")
            .descriptor();
        assert_eq!(openai_descriptor.provider, "openai");
        assert!(
            openai_descriptor
                .models
                .iter()
                .any(|model| model.id == "gpt-5.1")
        );

        assert!(runtime.providers().contains_key("openrouter"));
        assert!(runtime.providers().contains_key("litellm"));
        assert_eq!(diagnostics.counts.providers, 4);
        assert!(
            diagnostics
                .active_plugins
                .iter()
                .any(|plugin_id| plugin_id.as_str() == jfc_providers::BUILTIN_PROVIDER_PACK_ID)
        );
    }

    #[test]
    fn disabled_first_party_provider_pack_is_hidden_from_runtime_maps_and_status_robust() {
        let mut host = jfc_plugin_host::PluginHost::new();
        register_first_party_provider_pack(&mut host).expect("provider pack registers");
        let plugin_id = jfc_plugin_sdk::PluginId::new(jfc_providers::BUILTIN_PROVIDER_PACK_ID);
        host.disable_plugin(&plugin_id)
            .expect("provider pack disables");
        host.activate_all()
            .expect("provider pack activation skips disabled");

        let runtime = jfc_plugin_host::PluginRuntime::from_host(&host).expect("runtime builds");

        assert!(runtime.providers().is_empty());
        assert_eq!(host.diagnostics().counts.providers, 0);
    }
}
