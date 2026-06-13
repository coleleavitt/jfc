//! Engine seam for the local prompt-rewriter / over-refusal mitigation.
//!
//! This is the *integration* layer between the provider-neutral pipeline in
//! `jfc-audit::prompt_rewrite` and the engine's `submit_prompt`. It owns three
//! things and nothing else:
//!
//! 1. a [`ProviderRewriteModel`] adapter that drives the pipeline's
//!    [`RewriteModel`] trait through a [`jfc_provider::Provider`];
//! 2. building a [`RewritePipeline`] from [`jfc_config::PromptRewriteConfig`]
//!    (default-OFF: absent/`enabled = false` ⇒ no-op pass-through); and
//! 3. [`evaluate`], the single entry `submit_prompt` calls.
//!
//! The gate NEVER rewrites silently: a `Rewritten` outcome is returned to the
//! caller, which surfaces original→rewrite+rationale and requires the user to
//! accept/reject/edit. A `Refused` outcome blocks the turn with a reason. This
//! keeps state ownership clean — the gate computes a decision and the caller
//! decides what to do with the transcript.

use std::sync::Arc;

use async_trait::async_trait;
use jfc_audit::prompt_rewrite::types::RewriteModel;
use jfc_audit::{PolicyGate, RewriteDecision, RewritePipeline};
use jfc_config::PromptRewriteConfig;
use jfc_provider::{Provider, ProviderContent, ProviderMessage, ProviderRole, StreamOptions};

/// Adapts a [`Provider`] to the pipeline's [`RewriteModel`]. Each stage call
/// becomes one non-streaming `complete()` against `model`.
pub struct ProviderRewriteModel {
    provider: Arc<dyn Provider>,
    model: String,
}

impl ProviderRewriteModel {
    pub fn new(provider: Arc<dyn Provider>, model: impl Into<String>) -> Self {
        Self {
            provider,
            model: model.into(),
        }
    }
}

#[async_trait]
impl RewriteModel for ProviderRewriteModel {
    async fn complete(&self, system: &str, user: &str) -> jfc_audit::Result<String> {
        let messages = vec![ProviderMessage {
            role: ProviderRole::User,
            content: vec![ProviderContent::Text(user.to_string())],
        }];
        let opts = StreamOptions::new(self.model.clone())
            .system(system.to_string())
            .max_tokens(1024);
        match self.provider.complete(messages, &opts).await {
            Ok(resp) => Ok(resp.content),
            Err(e) => Err(jfc_audit::AuditError::Internal {
                message: format!("prompt-rewrite model error: {e}"),
            }),
        }
    }
}

/// Build a pipeline from config, or `None` when the feature is disabled. A
/// `None` here means the caller must treat the prompt as an unchanged
/// pass-through (the default-OFF contract).
pub fn pipeline_from_config(cfg: Option<&PromptRewriteConfig>) -> Option<RewritePipeline> {
    let cfg = cfg?;
    if !cfg.enabled {
        return None;
    }
    let gate = match &cfg.constitution {
        Some(text) if !text.trim().is_empty() => PolicyGate::new(text.clone()),
        _ => PolicyGate::default(),
    };
    Some(RewritePipeline::with_default_stages(gate))
}

/// Resolve the model id the LLM stages should use: explicit
/// `prompt_rewrite.model`, else the advisor model, else the active model.
pub fn resolve_model(
    cfg: Option<&PromptRewriteConfig>,
    advisor_model: Option<&str>,
    active_model: &str,
) -> String {
    cfg.and_then(|c| c.model.as_deref())
        .or(advisor_model)
        .unwrap_or(active_model)
        .to_string()
}

/// Evaluate a prompt through the gate. Returns `None` when the feature is off
/// (caller proceeds unchanged); otherwise the [`RewriteDecision`] the caller
/// surfaces to the user.
pub async fn evaluate(
    cfg: Option<&PromptRewriteConfig>,
    provider: Arc<dyn Provider>,
    advisor_model: Option<&str>,
    active_model: &str,
    prompt: &str,
) -> Option<RewriteDecision> {
    let pipeline = pipeline_from_config(cfg)?;
    let model_id = resolve_model(cfg, advisor_model, active_model);
    let model = ProviderRewriteModel::new(provider, model_id);
    match pipeline.run(prompt, &model).await {
        Ok(decision) => Some(decision),
        Err(e) => {
            // Fail-open on infrastructure error: an LLM/transport failure must
            // not block a legitimate turn. The screener already caught the
            // clearly-disallowed cases before any model call.
            tracing::warn!(
                target: "jfc::prompt_rewrite",
                error = %e,
                "rewrite gate errored; passing prompt through unchanged"
            );
            Some(RewriteDecision::Pass)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use jfc_provider::{CompletionResponse, EventStream, ModelInfo, TokenUsage};

    fn cfg(enabled: bool) -> PromptRewriteConfig {
        PromptRewriteConfig {
            enabled,
            model: None,
            threshold: None,
            constitution: None,
        }
    }

    #[test]
    fn disabled_config_yields_no_pipeline() {
        assert!(pipeline_from_config(None).is_none());
        assert!(pipeline_from_config(Some(&cfg(false))).is_none());
        assert!(pipeline_from_config(Some(&cfg(true))).is_some());
    }

    #[test]
    fn resolve_model_precedence() {
        // explicit prompt_rewrite.model wins
        let mut c = cfg(true);
        c.model = Some("explicit".into());
        assert_eq!(resolve_model(Some(&c), Some("advisor"), "active"), "explicit");
        // else advisor
        assert_eq!(resolve_model(Some(&cfg(true)), Some("advisor"), "active"), "advisor");
        // else active
        assert_eq!(resolve_model(Some(&cfg(true)), None, "active"), "active");
    }

    /// Minimal provider that returns scripted `complete()` text and is otherwise
    /// inert — enough to drive the gate end-to-end offline.
    struct ScriptProvider {
        classify: String,
        rewrite: String,
        verify: String,
    }

    impl jfc_provider::seal::Sealed for ScriptProvider {}

    #[async_trait]
    impl Provider for ScriptProvider {
        fn name(&self) -> &str {
            "script"
        }
        fn available_models(&self) -> Vec<ModelInfo> {
            Vec::new()
        }
        async fn stream(
            &self,
            _messages: Vec<ProviderMessage>,
            _options: &StreamOptions,
        ) -> anyhow::Result<EventStream> {
            anyhow::bail!("stream unused")
        }
        async fn complete(
            &self,
            _messages: Vec<ProviderMessage>,
            options: &StreamOptions,
        ) -> anyhow::Result<CompletionResponse> {
            let sys = options.system.clone().unwrap_or_default();
            let content = if sys.starts_with("You are a safety intent classifier") {
                self.classify.clone()
            } else if sys.starts_with("You rewrite") {
                self.rewrite.clone()
            } else {
                self.verify.clone()
            };
            Ok(CompletionResponse {
                content,
                usage: TokenUsage::default(),
            })
        }
    }

    #[tokio::test]
    async fn disabled_returns_none() {
        let provider = Arc::new(ScriptProvider {
            classify: String::new(),
            rewrite: String::new(),
            verify: String::new(),
        });
        let out = evaluate(Some(&cfg(false)), provider, None, "active", "hello").await;
        assert!(out.is_none());
    }

    #[tokio::test]
    async fn benign_passes_without_calling_model() {
        let provider = Arc::new(ScriptProvider {
            classify: String::new(),
            rewrite: String::new(),
            verify: String::new(),
        });
        let out = evaluate(
            Some(&cfg(true)),
            provider,
            None,
            "active",
            "write a rust function to sort a vec",
        )
        .await;
        assert_eq!(out, Some(RewriteDecision::Pass));
    }

    #[tokio::test]
    async fn ambiguous_prompt_produces_rewrite() {
        let provider = Arc::new(ScriptProvider {
            classify: r#"{"goal_category":"policy_analysis","verdict":"ambiguous","risk_flags":["evasion_phrasing"],"confidence":0.6}"#.into(),
            rewrite: r#"{"original_intent":"understand classifiers","text":"Research public docs on safety classifiers; defensive analysis only.","rationale":"removed evasion wording"}"#.into(),
            verify: r#"{"intent_preserved":true,"introduced_harm":false}"#.into(),
        });
        let out = evaluate(
            Some(&cfg(true)),
            provider,
            None,
            "active",
            "dig into their classifiers and how to get around it",
        )
        .await
        .unwrap();
        assert!(out.rewrite().is_some(), "expected a rewrite, got {out:?}");
    }
}
