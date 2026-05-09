//! Canonical Anthropic model catalog, mirrored from
//! `claude-code/src/utils/model/configs.ts` (`ALL_MODEL_CONFIGS`) and cross-checked
//! against the public models.dev catalog (https://models.dev/api.json :: `anthropic`).
//!
//! Both `AnthropicProvider` (API key) and `AnthropicOAuthProvider` (Claude Code OAuth)
//! talk to `api.anthropic.com/v1/messages`, so they share the same model set. Keep this
//! file in sync when Anthropic ships a new model — search the codebase for `[MODEL LAUNCH]`.
//!
//! Listing order is most-capable-first within each family (Opus → Sonnet → Haiku) and
//! newest-first within a family, which is what the picker renders top-to-bottom.

use crate::provider::ModelInfo;

/// Canonical "latest" alias targets — mirrors v126 cli.js alias resolution.
///
/// In v126 the picker leads with three rows (`sonnet` / `haiku` / `opus`) that
/// resolve to these full ids before hitting `/v1/messages`. Pinning the latest
/// firstParty id here gives users a stable "always pick the newest" option
/// without having to read release dates.
pub const ALIAS_SONNET: &str = "claude-sonnet-4-6";
pub const ALIAS_HAIKU: &str = "claude-haiku-4-5-20251001";
pub const ALIAS_OPUS: &str = "claude-opus-4-7";

/// Build the canonical first-party Anthropic model list.
///
/// `provider_tag` is stamped on each `ModelInfo` so the picker can swap `app.provider`
/// when the user picks a row. The list is alias rows first (mirrors v126's picker
/// layout), then specific dated/versioned ids, then any custom model injected via
/// `ANTHROPIC_CUSTOM_MODEL_OPTION` (matches v126's custom-model injection point).
pub fn anthropic_first_party_models(provider_tag: &str) -> Vec<ModelInfo> {
    let mut out: Vec<ModelInfo> = [
        // Aliases — top of picker so "just pick the latest" is one keystroke away.
        // Display name is prefixed with "↗" so the user knows this is an alias that
        // resolves to the dated id printed in the second column.
        (ALIAS_OPUS, "↗ Opus (latest)"),
        (ALIAS_SONNET, "↗ Sonnet (latest)"),
        (ALIAS_HAIKU, "↗ Haiku (latest)"),
        // Preview / experimental
        ("claude-mythos-preview", "Claude Mythos (preview)"),
        // Opus — flagship, dated/specific
        ("claude-opus-4-7", "Claude Opus 4.7"),
        ("claude-opus-4-6", "Claude Opus 4.6"),
        ("claude-opus-4-5-20251101", "Claude Opus 4.5"),
        ("claude-opus-4-1-20250805", "Claude Opus 4.1"),
        ("claude-opus-4-20250514", "Claude Opus 4"),
        // Sonnet
        ("claude-sonnet-4-6", "Claude Sonnet 4.6"),
        ("claude-sonnet-4-5-20250929", "Claude Sonnet 4.5"),
        ("claude-sonnet-4-20250514", "Claude Sonnet 4"),
        ("claude-3-7-sonnet-20250219", "Claude Sonnet 3.7"),
        // Haiku — fast/cheap
        ("claude-haiku-4-5-20251001", "Claude Haiku 4.5"),
        ("claude-3-5-haiku-20241022", "Claude Haiku 3.5"),
    ]
    .into_iter()
    .map(|(id, display)| {
        ModelInfo::new(id, display, provider_tag).with_context_window_tokens(200_000)
    })
    .collect();

    // ANTHROPIC_CUSTOM_MODEL_OPTION — v126 reads this env var and surfaces the value
    // as an extra picker row so power users can target preview/internal model ids.
    if let Ok(custom) = std::env::var("ANTHROPIC_CUSTOM_MODEL_OPTION") {
        let custom = custom.trim();
        if !custom.is_empty() && !out.iter().any(|m| m.id == custom) {
            out.push(ModelInfo::new(
                custom.to_owned(),
                format!("✱ {custom} (custom)"),
                provider_tag,
            ));
        }
    }

    tracing::debug!(
        target: "jfc::provider::anthropic_models",
        model_count = out.len(),
        provider_tag,
        "anthropic_first_party_models"
    );
    out
}

/// Apply the v126 `XwH()` seat-tier gate to a model list.
///
/// Rules (matched from v126 cli.js):
/// - `None` or unrecognized → no filter, return list as-is.
/// - `"opus"`, `"opus[1m]"`, `"opusplan"` → no filter (Opus access granted).
/// - A specific firstParty id like `"claude-opus-4-6"` or `"claude-opus-4-6[1m]"`
///   → strip `[1m]` suffix, then drop every Opus row whose id doesn't match.
///   Sonnet/Haiku rows are always kept.
///
/// The filter only operates on Anthropic models (any tag starting with `anthropic`)
/// — third-party providers (OpenWebUI) are pass-through.
pub fn apply_seat_tier_filter(models: Vec<ModelInfo>, seat_tier: Option<&str>) -> Vec<ModelInfo> {
    let input_count = models.len();
    let Some(tier) = seat_tier.map(str::trim).filter(|s| !s.is_empty()) else {
        tracing::debug!(
            target: "jfc::provider::anthropic_models",
            input_count,
            tier = "none",
            output_count = input_count,
            "apply_seat_tier_filter: no tier, pass-through"
        );
        return models;
    };
    // No restriction tiers — Anthropic granted broad Opus access.
    if matches!(tier, "opus" | "opus[1m]" | "opusplan") {
        tracing::debug!(
            target: "jfc::provider::anthropic_models",
            input_count,
            tier,
            output_count = input_count,
            "apply_seat_tier_filter: unrestricted tier"
        );
        return models;
    }
    // Specific-id tier: e.g. "claude-opus-4-6" or "claude-opus-4-6[1m]". Anything
    // else (subscription names like "max", "pro", or unknown values) → don't filter,
    // let the API's 404 path surface the truth at request time.
    let pinned = tier.strip_suffix("[1m]").unwrap_or(tier);
    if !pinned.starts_with("claude-opus-") {
        tracing::debug!(
            target: "jfc::provider::anthropic_models",
            input_count,
            tier,
            output_count = input_count,
            "apply_seat_tier_filter: unrecognized tier, pass-through"
        );
        return models;
    }
    let result: Vec<ModelInfo> = models
        .into_iter()
        .filter(|m| {
            // Pass through non-Anthropic providers and non-opus rows unchanged.
            if !m.provider.starts_with("anthropic") {
                return true;
            }
            if !m.id.contains("opus") && m.id != ALIAS_OPUS {
                return true;
            }
            // Keep the alias if it points at the pinned id, plus the pinned id itself.
            m.id == pinned || (m.id == ALIAS_OPUS && ALIAS_OPUS == pinned)
        })
        .collect();
    tracing::debug!(
        target: "jfc::provider::anthropic_models",
        input_count,
        tier,
        output_count = result.len(),
        "apply_seat_tier_filter: pinned opus filter applied"
    );
    result
}

/// Whether a model supports `thinking.type = "adaptive"` (Claude decides when
/// and how much to think). Mirrors v137's `FH8()` function.
///
/// Adaptive thinking is the preferred mode for 4.6+ models; older models must
/// use `thinking.type = "enabled"` with an explicit `budget_tokens`.
pub fn supports_adaptive_thinking(model_id: &str) -> bool {
    let id = model_id.to_lowercase();
    // Opus 4.6+ and Sonnet 4.6+ support adaptive
    id.contains("opus-4-6")
        || id.contains("opus-4-7")
        || id.contains("sonnet-4-6")
        || id.contains("mythos")
}

#[cfg(test)]
mod tests {
    use super::*;

    // Normal: every entry has a non-empty id, display, and the requested provider tag.
    #[test]
    fn all_entries_well_formed_normal() {
        let models = anthropic_first_party_models("anthropic");
        assert!(!models.is_empty());
        for m in &models {
            assert!(!m.id.is_empty(), "empty id in {m:?}");
            assert!(!m.display_name.is_empty(), "empty display in {m:?}");
            assert_eq!(m.provider, "anthropic");
        }
    }

    // Normal: provider tag is propagated verbatim — picker uses it to look up the active
    // provider when the user selects a row.
    #[test]
    fn provider_tag_is_threaded_through_normal() {
        let models = anthropic_first_party_models("anthropic-oauth");
        assert!(models.iter().all(|m| m.provider == "anthropic-oauth"));
    }

    // Normal: the canonical catalog includes the current flagship, mid, and fast tiers
    // so the user can always reach them without typing a custom id.
    #[test]
    fn current_flagship_models_present_normal() {
        let models = anthropic_first_party_models("x");
        for required in [
            "claude-opus-4-7",
            "claude-sonnet-4-6",
            "claude-haiku-4-5-20251001",
        ] {
            assert!(
                models.iter().any(|m| m.id == required),
                "missing canonical model {required}"
            );
        }
    }

    // Robust: ids may repeat across alias and dated rows. Aliases share their id with
    // the dated row they resolve to (Opus 4.7 alias → claude-opus-4-7 dated id), but
    // each (id, display_name) pair must be unique so the picker doesn't have two
    // visually identical rows.
    #[test]
    fn rows_are_distinct_robust() {
        let models = anthropic_first_party_models("x");
        let mut pairs: Vec<(&str, &str)> = models
            .iter()
            .map(|m| (m.id.as_str(), m.display_name.as_str()))
            .collect();
        pairs.sort();
        let before = pairs.len();
        pairs.dedup();
        assert_eq!(before, pairs.len(), "duplicate (id, display_name) pairs");
    }

    // ── Seat-tier filter (v126 XwH() equivalent) ────────────────────────────

    fn catalog() -> Vec<ModelInfo> {
        anthropic_first_party_models("anthropic-oauth")
    }

    fn ids(models: &[ModelInfo]) -> Vec<&str> {
        models.iter().map(|m| m.id.as_str()).collect()
    }

    // Normal: tier=None → return list verbatim.
    #[test]
    fn seat_tier_none_passes_through_normal() {
        let before = catalog();
        let after = apply_seat_tier_filter(before.clone(), None);
        assert_eq!(ids(&before), ids(&after));
    }

    // Normal: tier="opus" / "opusplan" / "opus[1m]" → all opus rows kept.
    #[test]
    fn seat_tier_broad_opus_keeps_everything_normal() {
        for tier in ["opus", "opusplan", "opus[1m]"] {
            let after = apply_seat_tier_filter(catalog(), Some(tier));
            assert!(
                ids(&after).iter().any(|id| id.contains("opus")),
                "tier {tier} dropped opus rows"
            );
        }
    }

    // Normal: pinned-id tier "claude-opus-4-6" → only that opus + sonnet/haiku.
    #[test]
    fn seat_tier_pinned_opus_id_filters_other_opus_normal() {
        let after = apply_seat_tier_filter(catalog(), Some("claude-opus-4-6"));
        let opus_ids: Vec<&str> = ids(&after)
            .into_iter()
            .filter(|id| id.contains("opus"))
            .collect();
        assert_eq!(
            opus_ids,
            vec!["claude-opus-4-6"],
            "expected only claude-opus-4-6, got {opus_ids:?}"
        );
        // Sonnet/Haiku rows must survive.
        assert!(ids(&after).iter().any(|id| id.contains("sonnet")));
        assert!(ids(&after).iter().any(|id| id.contains("haiku")));
    }

    // Normal: "[1m]" suffix is stripped before id comparison.
    #[test]
    fn seat_tier_one_m_suffix_stripped_normal() {
        let after = apply_seat_tier_filter(catalog(), Some("claude-opus-4-5-20251101[1m]"));
        let opus_ids: Vec<&str> = ids(&after)
            .into_iter()
            .filter(|id| id.contains("opus"))
            .collect();
        assert_eq!(opus_ids, vec!["claude-opus-4-5-20251101"]);
    }

    // Robust: empty / whitespace tier behaves like None.
    #[test]
    fn seat_tier_empty_string_passes_through_robust() {
        let before = catalog();
        let after = apply_seat_tier_filter(before.clone(), Some("  "));
        assert_eq!(ids(&before), ids(&after));
    }

    // Robust: unknown tier ("max", "code_pro", random string) → no filter, let the
    // API's 404 path be the source of truth.
    #[test]
    fn seat_tier_unknown_value_passes_through_robust() {
        let before = catalog();
        let after = apply_seat_tier_filter(before.clone(), Some("max"));
        assert_eq!(ids(&before), ids(&after));
    }

    // Robust: filter never touches non-Anthropic providers (OpenWebUI rows pass).
    #[test]
    fn seat_tier_leaves_other_providers_alone_robust() {
        let mut mixed = catalog();
        mixed.push(ModelInfo::new("local-llm", "Local", "openwebui"));
        let after = apply_seat_tier_filter(mixed, Some("claude-opus-4-6"));
        assert!(after.iter().any(|m| m.provider == "openwebui"));
    }

    // Normal: ANTHROPIC_CUSTOM_MODEL_OPTION env var injects a custom row at the
    // bottom of the catalog. Tested by setting the env in-process; safe because
    // the function reads it on each call.
    //
    // NB: rust tests share an env, so unset on exit to avoid polluting siblings.
    #[test]
    #[cfg_attr(any(target_env = "msvc"), ignore)]
    fn custom_model_env_var_appends_row_normal() {
        unsafe { std::env::set_var("ANTHROPIC_CUSTOM_MODEL_OPTION", "claude-test-foo-bar") };
        let models = anthropic_first_party_models("anthropic-oauth");
        unsafe { std::env::remove_var("ANTHROPIC_CUSTOM_MODEL_OPTION") };
        assert!(
            models.iter().any(|m| m.id == "claude-test-foo-bar"),
            "custom model env var was not injected"
        );
    }

    // ── Adaptive thinking ──────────────────────────────────────────────────

    #[test]
    fn adaptive_thinking_supported_for_4_6_plus_normal() {
        assert!(supports_adaptive_thinking("claude-opus-4-6"));
        assert!(supports_adaptive_thinking("claude-opus-4-7"));
        assert!(supports_adaptive_thinking("claude-sonnet-4-6"));
        assert!(supports_adaptive_thinking("claude-mythos-preview"));
    }

    #[test]
    fn adaptive_thinking_not_supported_for_older_models_normal() {
        assert!(!supports_adaptive_thinking("claude-opus-4-5-20251101"));
        assert!(!supports_adaptive_thinking("claude-opus-4-20250514"));
        assert!(!supports_adaptive_thinking("claude-sonnet-4-5-20250929"));
        assert!(!supports_adaptive_thinking("claude-sonnet-4-20250514"));
        assert!(!supports_adaptive_thinking("claude-3-7-sonnet-20250219"));
        assert!(!supports_adaptive_thinking("claude-haiku-4-5-20251001"));
        assert!(!supports_adaptive_thinking("claude-3-5-haiku-20241022"));
    }

    #[test]
    fn mythos_preview_in_catalog_normal() {
        let models = anthropic_first_party_models("anthropic-oauth");
        assert!(
            models.iter().any(|m| m.id == "claude-mythos-preview"),
            "claude-mythos-preview should be in the catalog"
        );
    }
}
