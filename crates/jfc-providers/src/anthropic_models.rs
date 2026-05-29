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

use jfc_provider::ModelInfo;

fn limits_for_anthropic_model(id: &str) -> (usize, Option<usize>) {
    let id = id.to_ascii_lowercase();
    if id.contains("mythos")
        || id.contains("opus-4-8")
        || id.contains("opus-4-7")
        || id.contains("opus-4-6")
        || id.contains("sonnet-4-6")
    {
        (1_000_000, Some(128_000))
    } else if id.contains("opus-4-5") {
        (1_000_000, Some(64_000))
    } else if id.contains("opus-4-1") || id.contains("opus-4-") {
        (200_000, Some(32_000))
    } else if id.contains("sonnet-4-5") || id.contains("3-7-sonnet") || id.contains("sonnet-4-") {
        (200_000, Some(64_000))
    } else if id.contains("haiku-4-5") {
        (200_000, Some(32_000))
    } else if id.contains("3-5-haiku") {
        (200_000, Some(8_192))
    } else {
        (200_000, None)
    }
}

/// Canonical "latest" alias targets — mirrors v126 cli.js alias resolution.
///
/// In v126 the picker leads with three rows (`sonnet` / `haiku` / `opus`) that
/// resolve to these full ids before hitting `/v1/messages`. Pinning the latest
/// firstParty id here gives users a stable "always pick the newest" option
/// without having to read release dates.
pub const ALIAS_SONNET: &str = "claude-sonnet-4-6";
pub const ALIAS_HAIKU: &str = "claude-haiku-4-5-20251001";
/// Tracks Claude Code 2.1.154's first-party default
/// (`he()` returns `Yz().opus48` for firstParty backends).
pub const ALIAS_OPUS: &str = "claude-opus-4-8";

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
        ("claude-opus-4-8", "Claude Opus 4.8"),
        ("claude-opus-4-7", "Claude Opus 4.7"),
        ("claude-opus-4-6", "Claude Opus 4.6"),
        ("claude-opus-4-5-20251101", "Claude Opus 4.5"),
        ("claude-opus-4-1-20250805", "Claude Opus 4.1"),
        ("claude-opus-4-20250514", "Claude Opus 4"),
        // Sonnet
        ("claude-sonnet-4-6", "Claude Sonnet 4.6"),
        (
            "claude-sonnet-4-6-20251114",
            "Claude Sonnet 4.6 (2025-11-14)",
        ),
        ("claude-sonnet-4-5-20250929", "Claude Sonnet 4.5"),
        ("claude-sonnet-4-20250514", "Claude Sonnet 4"),
        ("claude-3-7-sonnet-20250219", "Claude Sonnet 3.7"),
        // Haiku — fast/cheap
        ("claude-haiku-4-5-20251001", "Claude Haiku 4.5"),
        ("claude-3-5-haiku-20241022", "Claude Haiku 3.5"),
    ]
    .into_iter()
    .map(|(id, display)| {
        let (context, max_output) = limits_for_anthropic_model(id);
        ModelInfo::new(id, display, provider_tag)
            .with_context_window_tokens(context)
            .with_max_output_tokens(max_output)
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

/// Merge a live models.dev catalog into the curated canonical list.
///
/// ## Why this exists
///
/// `anthropic_first_party_models` is a hand-maintained list mirrored from Claude
/// Code's `ALL_MODEL_CONFIGS`. It owns the picker's *layout* — the three `↗ …
/// (latest)` alias rows up top, the curated display names ("Claude Opus 4.7"),
/// and the most-capable-first ordering. We never want a network fetch to clobber
/// that. But the canonical list also goes stale the moment Anthropic ships a new
/// revision: e.g. models.dev surfaced `claude-opus-4-8` the day it launched while
/// our hardcoded list still topped out at `claude-opus-4-7`.
///
/// This merge gives us both: the canonical rows are emitted **verbatim and
/// first**, then any live id that the canonical list doesn't already cover is
/// appended (newest-first, as models.dev sorts them). A freshly-launched model
/// thus appears at the bottom of the picker with no code change, while the
/// curated alias/order/naming stay authoritative.
///
/// Dedup is by model id. Live rows are re-stamped with `canonical`'s provider tag
/// so picker selection still routes back to the right `Provider` impl regardless
/// of the tag models.dev was fetched under. If `live` is empty (offline / fetch
/// failed) the result is exactly `canonical`.
pub fn merge_live_into_canonical(
    canonical: Vec<ModelInfo>,
    live: Vec<ModelInfo>,
) -> Vec<ModelInfo> {
    use std::collections::HashSet;

    let provider_tag: String = canonical
        .first()
        .map(|m| m.provider.to_string())
        .unwrap_or_default();
    let known: HashSet<String> = canonical.iter().map(|m| m.id.to_string()).collect();

    let canonical_count = canonical.len();
    let mut out = canonical;
    let mut appended = 0usize;
    for m in live {
        if known.contains(m.id.as_str()) {
            continue;
        }
        // Re-stamp the provider tag so the merged row routes to the same
        // Provider impl as the canonical rows (models.dev may have been fetched
        // under a different tag, e.g. "anthropic" vs "anthropic-oauth").
        let restamped = ModelInfo::new(m.id.clone(), m.display_name.clone(), provider_tag.as_str())
            .with_context_window_tokens(m.context_window_tokens)
            .with_max_output_tokens(m.max_output_tokens)
            .with_costs(m.input_cost, m.output_cost);
        out.push(restamped);
        appended += 1;
    }

    tracing::debug!(
        target: "jfc::provider::anthropic_models",
        canonical = canonical_count,
        appended,
        total = out.len(),
        "merge_live_into_canonical"
    );
    out
}

/// Whether a model supports `thinking.type = "adaptive"` (Claude decides when
/// and how much to think). Mirrors v137's `FH8()` function.
///
/// Adaptive thinking is the preferred mode for 4.6+ models; older models must
/// use `thinking.type = "enabled"` with an explicit `budget_tokens`.
#[allow(dead_code)]
pub fn supports_adaptive_thinking(model_id: &str) -> bool {
    let id = model_id.to_lowercase();
    // Opus 4.6+ and Sonnet 4.6+ support adaptive
    id.contains("opus-4-6")
        || id.contains("opus-4-7")
        || id.contains("opus-4-8")
        || id.contains("sonnet-4-6")
        || id.contains("mythos")
}

/// Whether `output_config.effort` may be sent to `model_id`.
///
/// Mirrors Claude Code 2.1.156's `A2(model)` gate (cli.js:180153): an
/// explicit allowlist of effort-capable models, with everything older
/// denied. Sending `effort` to a model that doesn't support it returns
/// `400 invalid_request_error: "This model does not support the effort
/// parameter."` — exactly the failure seen when a subagent inherited the
/// session's global `effort=max` and was dispatched to haiku.
///
/// CC's `A2` semantics, reproduced:
///   - explicit DENY: `claude-3-*`, `opus-4-0`, `opus-4-1`, `sonnet-4-0`,
///     `sonnet-4-5`, `haiku-4-5` (and haiku generally) → false
///   - explicit ALLOW: `opus-4-6`, `opus-4-7`, `opus-4-8`, `sonnet-4-6`,
///     plus `opus-4-5` (the skill doc lists it; the live `oR` fallback
///     covers it) and `mythos` (our preview) → true
///   - `CLAUDE_CODE_ALWAYS_ENABLE_EFFORT` env force-on → true
///   - unknown models → false (deny by default; an unknown model that
///     can't take `effort` would 400, and the cost of a missing `effort`
///     on a model that *could* take it is just default depth)
pub fn model_supports_effort(model_id: &str) -> bool {
    if std::env::var("CLAUDE_CODE_ALWAYS_ENABLE_EFFORT")
        .ok()
        .is_some_and(|v| !v.is_empty() && v != "0" && !v.eq_ignore_ascii_case("false"))
    {
        return true;
    }
    let id = model_id.to_ascii_lowercase();
    // Explicit deny — pre-effort families (mirrors CC's A2 deny list).
    if id.contains("claude-3-")
        || id.contains("opus-4-0")
        || id.contains("opus-4-1")
        || id.contains("sonnet-4-0")
        || id.contains("sonnet-4-5")
        || id.contains("haiku")
    {
        return false;
    }
    // Explicit allow — effort-capable families.
    id.contains("opus-4-5")
        || id.contains("opus-4-6")
        || id.contains("opus-4-7")
        || id.contains("opus-4-8")
        || id.contains("sonnet-4-6")
        || id.contains("mythos")
}

/// Whether the `"max"` / `"xhigh"` effort tiers are valid for `model_id`.
/// Both are Opus-tier only (Opus 4.6+ for `max`; Opus 4.7+ added `xhigh`).
/// Sonnet 4.6 supports effort but caps at `high` — sending `max`/`xhigh`
/// there 400s. This lets callers clamp rather than drop. Mirrors the skill
/// doc: "`max` is Opus-tier only … will error on Sonnet/Haiku".
pub fn model_supports_high_effort_tier(model_id: &str) -> bool {
    let id = model_id.to_ascii_lowercase();
    id.contains("opus-4-5")
        || id.contains("opus-4-6")
        || id.contains("opus-4-7")
        || id.contains("opus-4-8")
        || id.contains("mythos")
}

/// Resolve the effort value actually safe to send for `model_id`, given the
/// caller's `requested` effort. Returns `None` when effort must be omitted
/// entirely (model doesn't support the parameter — CC's `delete $.effort`),
/// or `Some(clamped)` where `max`/`xhigh` are clamped to `high` on
/// effort-capable-but-not-Opus models (e.g. Sonnet 4.6).
pub fn effort_for_model<'a>(model_id: &str, requested: &'a str) -> Option<&'a str> {
    if !model_supports_effort(model_id) {
        return None;
    }
    if matches!(requested, "max" | "xhigh") && !model_supports_high_effort_tier(model_id) {
        // Effort-capable but Sonnet-tier: clamp the Opus-only tiers to high.
        return Some("high");
    }
    Some(requested)
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── effort gating (CC 2.1.156 A2/NLz parity) ────────────────────────────

    // Normal: effort-capable models are allowed; pre-effort families denied.
    #[test]
    fn model_supports_effort_allowlist_normal() {
        for ok in [
            "claude-opus-4-8",
            "claude-opus-4-7",
            "claude-opus-4-6",
            "claude-opus-4-5",
            "claude-sonnet-4-6",
            "claude-mythos-preview",
        ] {
            assert!(model_supports_effort(ok), "{ok} should support effort");
        }
        for deny in [
            "claude-haiku-4-5",
            "claude-haiku-4-5-20251001",
            "claude-sonnet-4-5",
            "claude-sonnet-4-0",
            "claude-opus-4-1",
            "claude-opus-4-0",
            "claude-3-7-sonnet-20250219",
            "some-unknown-model",
        ] {
            assert!(
                !model_supports_effort(deny),
                "{deny} must NOT support effort"
            );
        }
    }

    // Normal — REGRESSION (the haiku 400): effort_for_model returns None for
    // haiku so the param is dropped entirely.
    #[test]
    fn effort_dropped_on_haiku_regression() {
        assert_eq!(effort_for_model("claude-haiku-4-5", "max"), None);
        assert_eq!(effort_for_model("claude-haiku-4-5", "high"), None);
    }

    // Normal: effort-capable Opus keeps the requested value verbatim,
    // including the Opus-only max/xhigh tiers.
    #[test]
    fn effort_passthrough_on_opus_normal() {
        assert_eq!(effort_for_model("claude-opus-4-8", "max"), Some("max"));
        assert_eq!(effort_for_model("claude-opus-4-7", "xhigh"), Some("xhigh"));
        assert_eq!(effort_for_model("claude-opus-4-6", "high"), Some("high"));
    }

    // Robust: Sonnet 4.6 supports effort but not the Opus-only max/xhigh
    // tiers — those clamp to high; low/medium/high pass through.
    #[test]
    fn effort_clamped_on_sonnet_robust() {
        assert_eq!(effort_for_model("claude-sonnet-4-6", "max"), Some("high"));
        assert_eq!(effort_for_model("claude-sonnet-4-6", "xhigh"), Some("high"));
        assert_eq!(effort_for_model("claude-sonnet-4-6", "low"), Some("low"));
        assert_eq!(
            effort_for_model("claude-sonnet-4-6", "medium"),
            Some("medium")
        );
    }

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
    // so the user can always reach them without typing a custom id. The flagship id
    // tracks Claude Code's `he()` first-party default (`Yz().opus48` in CC 2.1.154+).
    #[test]
    fn current_flagship_models_present_normal() {
        let models = anthropic_first_party_models("x");
        for required in [
            "claude-opus-4-8",
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
        assert!(supports_adaptive_thinking("claude-opus-4-8"));
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

    // ── merge_live_into_canonical (models.dev union) ────────────────────────

    fn live_row(id: &str, display: &str, tag: &str) -> ModelInfo {
        ModelInfo::new(id, display, tag).with_context_window_tokens(200_000usize)
    }

    // Normal: canonical rows appear verbatim, in order, before any live row.
    // Picker layout (alias rows up top, curated ordering) must never be
    // reshuffled by a live fetch.
    #[test]
    fn merge_preserves_canonical_order_normal() {
        let canonical = anthropic_first_party_models("anthropic-oauth");
        let canonical_ids: Vec<String> = canonical.iter().map(|m| m.id.to_string()).collect();
        let live = vec![live_row("claude-opus-4-8", "Claude Opus 4.8", "anthropic")];
        let merged = merge_live_into_canonical(canonical, live);
        let merged_ids: Vec<&str> = merged.iter().map(|m| m.id.as_str()).collect();
        // First N entries are the canonical list verbatim.
        for (i, want) in canonical_ids.iter().enumerate() {
            assert_eq!(merged_ids[i], want.as_str(), "canonical row {i} reshuffled");
        }
    }

    // Normal: a brand-new live id (the canonical list always lags Anthropic's
    // next release) is appended to the merged result. This is the whole point
    // of the merge. Uses a synthetic id so the test doesn't false-pass once
    // the canonical list catches up.
    #[test]
    fn merge_appends_new_live_id_normal() {
        let canonical = anthropic_first_party_models("anthropic-oauth");
        let live = vec![live_row(
            "claude-opus-4-9",
            "Claude Opus 4.9",
            "anthropic-oauth",
        )];
        let merged = merge_live_into_canonical(canonical, live);
        assert!(
            merged.iter().any(|m| m.id == "claude-opus-4-9"),
            "new live id was not appended"
        );
    }

    // Robust: merge_restamps_provider_tag uses a synthetic future id so the
    // assertion exercises the restamp path, not a canonical row.
    #[test]
    fn merge_restamps_provider_tag_synthetic_id_robust() {
        let canonical = anthropic_first_party_models("anthropic-oauth");
        let live = vec![live_row("claude-opus-4-9", "Claude Opus 4.9", "anthropic")];
        let merged = merge_live_into_canonical(canonical, live);
        let row = merged
            .iter()
            .find(|m| m.id == "claude-opus-4-9")
            .expect("merge dropped the new id");
        assert_eq!(
            row.provider, "anthropic-oauth",
            "live row was not re-stamped with canonical provider tag"
        );
    }

    // Robust: a live row whose id collides with a canonical row is DROPPED.
    // The canonical row's display names ("↗ Opus (latest)" and "Claude Opus 4.7")
    // survive intact — models.dev never gets to rename our curated rows.
    //
    // Note: `claude-opus-4-7` legitimately appears twice in the canonical list
    // (once as the `↗ Opus (latest)` alias row, once as the dated row). The
    // merge must add ZERO more entries, not collapse them.
    #[test]
    fn merge_drops_live_rows_colliding_with_canonical_robust() {
        let canonical = anthropic_first_party_models("anthropic-oauth");
        let canonical_47_count = canonical
            .iter()
            .filter(|m| m.id == "claude-opus-4-7")
            .count();
        let live = vec![live_row(
            "claude-opus-4-7",
            "Anthropic Opus 4.7 (different name)",
            "anthropic-oauth",
        )];
        let merged = merge_live_into_canonical(canonical, live);
        let merged_47_count = merged.iter().filter(|m| m.id == "claude-opus-4-7").count();
        assert_eq!(
            merged_47_count, canonical_47_count,
            "live row was not deduped against canonical"
        );
        assert!(
            !merged
                .iter()
                .any(|m| m.display_name == "Anthropic Opus 4.7 (different name)"),
            "live display name leaked into merged list"
        );
    }

    // Robust: empty live (offline / fetch failure) → merged equals canonical
    // exactly. This is the cold-path safety net — the picker must always
    // have rows to show even when models.dev is unreachable.
    #[test]
    fn merge_with_empty_live_returns_canonical_robust() {
        let canonical = anthropic_first_party_models("anthropic-oauth");
        let canonical_ids: Vec<String> = canonical.iter().map(|m| m.id.to_string()).collect();
        let merged = merge_live_into_canonical(canonical, vec![]);
        let merged_ids: Vec<String> = merged.iter().map(|m| m.id.to_string()).collect();
        assert_eq!(merged_ids, canonical_ids);
    }

    // Robust: multiple new live ids are all appended; their relative order
    // (newest-first as models.dev sorts them) is preserved.
    #[test]
    fn merge_preserves_live_relative_order_robust() {
        let canonical = anthropic_first_party_models("anthropic-oauth");
        let live = vec![
            live_row("claude-opus-5-0", "Claude Opus 5", "anthropic"),
            live_row("claude-opus-4-9", "Claude Opus 4.9", "anthropic"),
        ];
        let merged = merge_live_into_canonical(canonical.clone(), live);
        // Skip past the canonical prefix, then check the tail order.
        let tail: Vec<&str> = merged
            .iter()
            .skip(canonical.len())
            .map(|m| m.id.as_str())
            .collect();
        assert_eq!(tail, vec!["claude-opus-5-0", "claude-opus-4-9"]);
    }

    // Robust: cost + context window fields survive the re-stamp. Picker's
    // cost column would otherwise render "—" for merged rows.
    #[test]
    fn merge_preserves_live_costs_and_limits_robust() {
        let canonical = anthropic_first_party_models("anthropic-oauth");
        let live_one = ModelInfo::new("claude-opus-4-9", "Claude Opus 4.9", "anthropic")
            .with_context_window_tokens(1_000_000usize)
            .with_max_output_tokens(128_000usize)
            .with_costs(Some(15.0), Some(75.0));
        let merged = merge_live_into_canonical(canonical, vec![live_one]);
        let row = merged
            .iter()
            .find(|m| m.id == "claude-opus-4-9")
            .expect("merge dropped the new id");
        assert_eq!(row.context_window_tokens, Some(1_000_000));
        assert_eq!(row.max_output_tokens, Some(128_000));
        assert_eq!(row.input_cost, Some(15.0));
        assert_eq!(row.output_cost, Some(75.0));
    }
}
