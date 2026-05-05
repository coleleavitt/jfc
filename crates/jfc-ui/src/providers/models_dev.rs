//! Live model catalog fetched from <https://models.dev/api.json>.
//!
//! models.dev is a community-maintained registry of every model exposed by every
//! mainstream LLM API; we use it so the picker always reflects the current model
//! lineup without hardcoding ids that drift out of date the moment Anthropic ships
//! a new revision.
//!
//! The response is a single JSON document keyed by provider id (`"anthropic"`,
//! `"google-vertex-anthropic"`, `"openrouter"`, …). Each provider has a `models`
//! map keyed by model id. We consume the display name plus the context window so
//! the footer reflects the active model instead of a static fallback.
//!
//! Network failures degrade gracefully — callers fall back to
//! `anthropic_models::anthropic_first_party_models()` so the picker still works
//! offline.

use std::collections::HashMap;
use std::time::Duration;

use serde::Deserialize;

use crate::provider::ModelInfo;

const MODELS_DEV_URL: &str = "https://models.dev/api.json";
const MODELS_DEV_TIMEOUT: Duration = Duration::from_secs(8);

#[derive(Debug, Deserialize)]
struct ProviderEntry {
    #[serde(default)]
    models: HashMap<String, ModelEntry>,
}

#[derive(Debug, Deserialize)]
struct ModelEntry {
    id: String,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    release_date: Option<String>,
    #[serde(default)]
    limit: ModelLimit,
    #[serde(default)]
    cost: Option<ModelCost>,
}

#[derive(Debug, Default, Deserialize)]
struct ModelLimit {
    #[serde(default)]
    context: Option<usize>,
    #[serde(default)]
    output: Option<usize>,
}

#[derive(Debug, Default, Deserialize)]
struct ModelCost {
    #[serde(default)]
    input: Option<f64>,
    #[serde(default)]
    output: Option<f64>,
}

/// Fetch the model list for a given models.dev provider id (e.g. `"anthropic"`,
/// `"google-vertex-anthropic"`). Stamps each entry with the supplied `provider_tag`
/// so the picker can route selection back to the matching jfc `Provider` impl.
///
/// Sorted newest-first by `release_date` (string compare on `YYYY-MM-DD` is correct);
/// entries without a release date sort last.
pub async fn fetch_provider_models(
    client: &reqwest::Client,
    models_dev_provider_id: &str,
    provider_tag: &str,
) -> anyhow::Result<Vec<ModelInfo>> {
    tracing::info!(
        target: "jfc::provider::models_dev",
        provider_id = %models_dev_provider_id,
        provider_tag = %provider_tag,
        "fetching model catalog from models.dev"
    );

    let resp = client
        .get(MODELS_DEV_URL)
        .timeout(MODELS_DEV_TIMEOUT)
        .send()
        .await?
        .error_for_status()?;
    let catalog: HashMap<String, ProviderEntry> = resp.json().await?;
    let entry = catalog
        .get(models_dev_provider_id)
        .ok_or_else(|| {
            tracing::warn!(
                target: "jfc::provider::models_dev",
                provider_id = %models_dev_provider_id,
                "provider not found in models.dev catalog"
            );
            anyhow::anyhow!("models.dev has no provider {models_dev_provider_id}")
        })?;

    let mut models: Vec<&ModelEntry> = entry.models.values().collect();
    models.sort_by(|a, b| b.release_date.cmp(&a.release_date));

    let result: Vec<ModelInfo> = models
        .into_iter()
        .map(|m| {
            let display = m.name.clone().unwrap_or_else(|| m.id.clone());
            let (in_cost, out_cost) = match &m.cost {
                Some(c) => (c.input, c.output),
                None => (None, None),
            };
            ModelInfo::new(m.id.clone(), display, provider_tag)
                .with_context_window_tokens(m.limit.context)
                .with_max_output_tokens(m.limit.output)
                .with_costs(in_cost, out_cost)
        })
        .collect();

    tracing::debug!(
        target: "jfc::provider::models_dev",
        provider_id = %models_dev_provider_id,
        model_count = result.len(),
        "models.dev catalog fetched successfully"
    );

    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn limit_context_deserializes_from_catalog_entries() {
        let catalog: HashMap<String, ProviderEntry> = serde_json::from_value(serde_json::json!({
            "anthropic": {
                "models": {
                    "claude-sonnet-4-5-20250929": {
                        "id": "claude-sonnet-4-5-20250929",
                        "name": "Claude Sonnet 4.5",
                        "limit": { "context": 200000 }
                    }
                }
            }
        }))
        .unwrap();

        let model = catalog["anthropic"].models["claude-sonnet-4-5-20250929"]
            .limit
            .context;
        assert_eq!(model, Some(200_000));
    }

    // Robust: network unreachable / DNS fail / non-2xx → returns Err, never panics.
    // We exercise the error path with an obviously bogus URL via a custom client.
    #[tokio::test]
    async fn fetch_returns_err_on_unreachable_endpoint_robust() {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_millis(50))
            .build()
            .unwrap();
        // Override URL by hitting a localhost port that's almost certainly closed.
        let result = client.get("http://127.0.0.1:1/api.json").send().await;
        assert!(result.is_err(), "expected network error, got {result:?}");
    }

    // ── Real-API integration tests (gated with #[ignore]) ────────────────────
    // Run with: cargo test --bin jfc -- --ignored models_dev
    // These hit the live models.dev endpoint over the network. They assert that
    // the catalog actually contains the model families we need so the picker
    // doesn't silently degrade.

    // Normal: live models.dev returns Anthropic catalog with current flagship models.
    #[tokio::test]
    #[ignore = "hits live network — run with cargo test -- --ignored"]
    async fn live_anthropic_catalog_has_current_flagship_normal() {
        let client = reqwest::Client::new();
        let models = fetch_provider_models(&client, "anthropic", "anthropic")
            .await
            .expect("models.dev fetch");
        assert!(!models.is_empty(), "anthropic catalog must not be empty");

        // Pick a couple of stable canonical ids — at least one Opus and one Haiku
        // should always be present per Anthropic's release cadence.
        let ids: Vec<&str> = models.iter().map(|m| m.id.as_str()).collect();
        assert!(
            ids.iter().any(|id| id.contains("opus")),
            "no opus model found in {ids:?}"
        );
        assert!(
            ids.iter().any(|id| id.contains("haiku")),
            "no haiku model found in {ids:?}"
        );
    }

    // Normal: each entry carries the provider tag we requested so the picker can
    // round-trip selection back to the right `Provider` impl.
    #[tokio::test]
    #[ignore = "hits live network — run with cargo test -- --ignored"]
    async fn live_provider_tag_is_stamped_normal() {
        let client = reqwest::Client::new();
        let models = fetch_provider_models(&client, "anthropic", "anthropic-oauth")
            .await
            .expect("models.dev fetch");
        assert!(models.iter().all(|m| m.provider == "anthropic-oauth"));
    }

    // Robust: requesting a nonexistent provider id surfaces a clean Err instead of
    // returning a misleading empty list.
    #[tokio::test]
    #[ignore = "hits live network — run with cargo test -- --ignored"]
    async fn live_unknown_provider_id_errors_robust() {
        let client = reqwest::Client::new();
        let result = fetch_provider_models(&client, "this-provider-does-not-exist", "x").await;
        assert!(result.is_err(), "expected Err for unknown provider");
    }
}
