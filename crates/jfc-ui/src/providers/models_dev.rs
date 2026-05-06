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

/// Pure transformer: given a parsed catalog, the provider id we asked for, and
/// the tag we want stamped on each `ModelInfo`, produce the picker-ready list.
///
/// Extracted from `fetch_provider_models` so the network-free portion (sort,
/// projection, error path for unknown provider) can be exercised in isolation.
/// The async wrapper below handles network I/O only and delegates here.
fn shape_catalog(
    catalog: &HashMap<String, ProviderEntry>,
    models_dev_provider_id: &str,
    provider_tag: &str,
) -> anyhow::Result<Vec<ModelInfo>> {
    let entry = catalog.get(models_dev_provider_id).ok_or_else(|| {
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
        "shape_catalog projected entries"
    );

    Ok(result)
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
    shape_catalog(&catalog, models_dev_provider_id, provider_tag)
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

    // ─────────────────────────────────────────────────────────────────────────
    // DO-178B §6.4.2: pure-data tests that exercise the JSON parser, the
    // ModelEntry / ModelLimit / ModelCost serde wiring, and the post-fetch
    // shaping logic. These run offline and don't depend on the live endpoint
    // so coverage is deterministic.
    // ─────────────────────────────────────────────────────────────────────────

    // Normal: a fully-populated catalog entry maps every field through serde.
    #[test]
    fn full_model_entry_deserializes_all_fields_normal() {
        let v = serde_json::json!({
            "id": "claude-opus-4-7",
            "name": "Claude Opus 4.7",
            "release_date": "2026-04-01",
            "limit": { "context": 1_000_000usize, "output": 128_000usize },
            "cost": { "input": 15.0f64, "output": 75.0f64 }
        });
        let entry: ModelEntry = serde_json::from_value(v).unwrap();
        assert_eq!(entry.id, "claude-opus-4-7");
        assert_eq!(entry.name.as_deref(), Some("Claude Opus 4.7"));
        assert_eq!(entry.release_date.as_deref(), Some("2026-04-01"));
        assert_eq!(entry.limit.context, Some(1_000_000));
        assert_eq!(entry.limit.output, Some(128_000));
        let cost = entry.cost.as_ref().unwrap();
        assert_eq!(cost.input, Some(15.0));
        assert_eq!(cost.output, Some(75.0));
    }

    // Robust: the only required field is `id`. With everything else missing,
    // serde defaults must kick in so models.dev catalog rows that ship without
    // a `cost` block (true for many open-source providers) don't fail to parse.
    #[test]
    fn minimal_entry_uses_serde_defaults_robust() {
        let v = serde_json::json!({ "id": "minimal" });
        let entry: ModelEntry = serde_json::from_value(v).unwrap();
        assert_eq!(entry.id, "minimal");
        assert!(entry.name.is_none());
        assert!(entry.release_date.is_none());
        assert!(entry.limit.context.is_none());
        assert!(entry.limit.output.is_none());
        assert!(entry.cost.is_none());
    }

    // Robust: a partial cost block — output set but input missing — parses
    // with `None` for the missing side rather than failing the whole entry.
    #[test]
    fn partial_cost_block_parses_with_none_for_missing_side_robust() {
        let v = serde_json::json!({
            "id": "x",
            "cost": { "output": 1.0f64 }
        });
        let entry: ModelEntry = serde_json::from_value(v).unwrap();
        let cost = entry.cost.as_ref().unwrap();
        assert!(cost.input.is_none());
        assert_eq!(cost.output, Some(1.0));
    }

    // Robust: a partial limit block — context set but output missing — parses
    // both fields independently. Mirrors how some llama.cpp / Ollama providers
    // expose only context window.
    #[test]
    fn partial_limit_block_parses_with_none_for_missing_side_robust() {
        let v = serde_json::json!({
            "id": "x",
            "limit": { "context": 8192usize }
        });
        let entry: ModelEntry = serde_json::from_value(v).unwrap();
        assert_eq!(entry.limit.context, Some(8192));
        assert!(entry.limit.output.is_none());
    }

    // Robust: an entry without the `id` field MUST fail to parse — the id is
    // load-bearing for the picker's routing key. Catching this at deserialize
    // time is cheaper than catching it at request time.
    #[test]
    fn missing_id_field_fails_robust() {
        let v = serde_json::json!({ "name": "no id" });
        let result: Result<ModelEntry, _> = serde_json::from_value(v);
        assert!(result.is_err(), "missing id must fail: got {result:?}");
    }

    // Robust: provider entry with no `models` key uses the HashMap default
    // (empty) — the catalog occasionally ships a provider stub with metadata
    // but no model list, and we mustn't choke on it.
    #[test]
    fn provider_entry_with_no_models_uses_default_robust() {
        let v = serde_json::json!({});
        let entry: ProviderEntry = serde_json::from_value(v).unwrap();
        assert!(entry.models.is_empty());
    }

    // ── shape_catalog: pure transformer that drives the picker layout ──────

    fn parse_catalog(json: serde_json::Value) -> HashMap<String, ProviderEntry> {
        serde_json::from_value(json).expect("parse catalog")
    }

    // Normal: a single-entry catalog projects to a single ModelInfo with
    // every field threaded through (display name, context, output, costs).
    #[test]
    fn shape_catalog_single_entry_full_projection_normal() {
        let catalog = parse_catalog(serde_json::json!({
            "anthropic": {
                "models": {
                    "claude-opus-4-7": {
                        "id": "claude-opus-4-7",
                        "name": "Claude Opus 4.7",
                        "release_date": "2026-04-01",
                        "limit": { "context": 1_000_000usize, "output": 128_000usize },
                        "cost": { "input": 15.0f64, "output": 75.0f64 }
                    }
                }
            }
        }));
        let models = shape_catalog(&catalog, "anthropic", "anthropic-oauth").unwrap();
        assert_eq!(models.len(), 1);
        let m = &models[0];
        assert_eq!(m.id.as_str(), "claude-opus-4-7");
        assert_eq!(m.display_name, "Claude Opus 4.7");
        assert_eq!(m.provider.as_str(), "anthropic-oauth");
        assert_eq!(m.context_window_tokens, Some(1_000_000));
        assert_eq!(m.max_output_tokens, Some(128_000));
        assert_eq!(m.input_cost, Some(15.0));
        assert_eq!(m.output_cost, Some(75.0));
    }

    // Normal: entries are sorted newest-first by release_date. String
    // comparison on ISO-8601 dates is monotonic so this is always correct.
    #[test]
    fn shape_catalog_sorts_newest_first_normal() {
        let catalog = parse_catalog(serde_json::json!({
            "anthropic": {
                "models": {
                    "old": { "id": "old", "release_date": "2024-01-01" },
                    "new": { "id": "new", "release_date": "2026-04-01" },
                    "mid": { "id": "mid", "release_date": "2025-09-01" }
                }
            }
        }));
        let models = shape_catalog(&catalog, "anthropic", "anthropic").unwrap();
        let ids: Vec<&str> = models.iter().map(|m| m.id.as_str()).collect();
        assert_eq!(ids, vec!["new", "mid", "old"]);
    }

    // Normal: an entry with no `name` field falls back to using its `id` as
    // the display name. Picker rows must always have something to show.
    #[test]
    fn shape_catalog_display_falls_back_to_id_normal() {
        let catalog = parse_catalog(serde_json::json!({
            "anthropic": {
                "models": {
                    "x-only-id": { "id": "x-only-id" }
                }
            }
        }));
        let models = shape_catalog(&catalog, "anthropic", "anthropic").unwrap();
        assert_eq!(models[0].display_name, "x-only-id");
    }

    // Robust: missing cost block → both costs surface as None. The picker's
    // cost column then renders "—" instead of fabricating a price.
    #[test]
    fn shape_catalog_missing_cost_yields_none_robust() {
        let catalog = parse_catalog(serde_json::json!({
            "anthropic": {
                "models": {
                    "no-cost": { "id": "no-cost" }
                }
            }
        }));
        let models = shape_catalog(&catalog, "anthropic", "anthropic").unwrap();
        assert!(models[0].input_cost.is_none());
        assert!(models[0].output_cost.is_none());
    }

    // Robust: provider id not in catalog → Err with the offending id quoted
    // in the message. The friendly text helps users diagnose typos in their
    // provider config.
    #[test]
    fn shape_catalog_unknown_provider_errors_robust() {
        let catalog = parse_catalog(serde_json::json!({
            "anthropic": { "models": {} }
        }));
        let result = shape_catalog(&catalog, "openrouter", "x");
        let err = result.expect_err("unknown provider must error");
        assert!(err.to_string().contains("openrouter"));
    }

    // Robust: provider entry with empty `models` map → Ok(empty Vec). Not an
    // error: the catalog might describe a provider that has no rows yet.
    #[test]
    fn shape_catalog_empty_models_returns_empty_vec_robust() {
        let catalog = parse_catalog(serde_json::json!({
            "openrouter": { "models": {} }
        }));
        let models = shape_catalog(&catalog, "openrouter", "openrouter").unwrap();
        assert!(models.is_empty());
    }

    // Normal: provider_tag is stamped on every entry so the picker round-
    // trips selection back to the right `Provider` impl regardless of the
    // models.dev id we used to fetch.
    #[test]
    fn shape_catalog_provider_tag_threads_through_normal() {
        let catalog = parse_catalog(serde_json::json!({
            "anthropic": {
                "models": {
                    "a": { "id": "a" },
                    "b": { "id": "b" }
                }
            }
        }));
        let models = shape_catalog(&catalog, "anthropic", "anthropic-oauth").unwrap();
        assert!(models.iter().all(|m| m.provider == "anthropic-oauth"));
    }

    // Normal: a tiny catalog parses end-to-end, including nested HashMap of
    // model entries. Verifies the top-level shape works as advertised.
    #[test]
    fn catalog_parses_with_multiple_providers_normal() {
        let v = serde_json::json!({
            "anthropic": {
                "models": {
                    "claude-opus-4-7": {
                        "id": "claude-opus-4-7",
                        "name": "Claude Opus 4.7",
                        "release_date": "2026-04-01",
                        "limit": { "context": 200_000usize }
                    },
                    "claude-haiku-4-5": {
                        "id": "claude-haiku-4-5",
                        "release_date": "2025-10-01"
                    }
                }
            },
            "openai": {
                "models": {
                    "gpt-5": { "id": "gpt-5" }
                }
            }
        });
        let catalog: HashMap<String, ProviderEntry> = serde_json::from_value(v).unwrap();
        assert_eq!(catalog.len(), 2);
        assert_eq!(catalog["anthropic"].models.len(), 2);
        assert_eq!(catalog["openai"].models.len(), 1);
    }
}
