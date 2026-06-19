use jfc_provider::StreamOptions;

use super::super::thinking::enforce_thinking_budget_fits_max_tokens;

#[test]
fn request_guard_clamps_legacy_thinking_budget_below_max_tokens_regression() {
    let mut opts = StreamOptions::new("claude-sonnet-4-5-20250929").max_tokens(8192);
    opts.thinking_budget = Some(16_384);

    enforce_thinking_budget_fits_max_tokens(&mut opts);

    assert_eq!(opts.max_tokens, 8192);
    assert_eq!(opts.thinking_budget, Some(8191));
}
