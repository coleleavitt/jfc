//! Proof-backed attention pruning and sink-preservation helpers.
//!
//! These are direct Rust counterparts of the pure models in
//! `rcoq-tests/theorems/AttentionPruning.v` and
//! `rcoq-tests/theorems/AttentionSink.v`. They intentionally make no provider
//! assumptions: callers supply scores, positions, and budgets.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScoredToken {
    pub token_id: u64,
    pub token_position: u64,
    pub attention_weight: u64,
}

pub fn weight_ge(left: &ScoredToken, right: &ScoredToken) -> bool {
    left.attention_weight >= right.attention_weight
}

pub fn is_sorted_by_weight(tokens: &[ScoredToken]) -> bool {
    tokens
        .windows(2)
        .all(|window| window[0].attention_weight >= window[1].attention_weight)
}

pub fn top_k(tokens: &[ScoredToken], k: usize) -> Vec<ScoredToken> {
    tokens.iter().take(k).copied().collect()
}

pub fn threshold_prune(tokens: &[ScoredToken], gamma: u64) -> Vec<ScoredToken> {
    tokens
        .iter()
        .copied()
        .filter(|token| token.attention_weight >= gamma)
        .collect()
}

pub fn strict_threshold_prune(tokens: &[ScoredToken], gamma: u64) -> Vec<ScoredToken> {
    tokens
        .iter()
        .copied()
        .filter(|token| token.attention_weight > gamma)
        .collect()
}

pub fn total_attention(tokens: &[ScoredToken]) -> u64 {
    tokens.iter().fold(0u64, |sum, token| {
        sum.saturating_add(token.attention_weight)
    })
}

pub fn attention_sample(tokens: &[ScoredToken], k: usize) -> Vec<ScoredToken> {
    top_k(tokens, k)
}

pub fn position_boost(max_pos: u64, pos: u64) -> u64 {
    pos.saturating_mul(100) / max_pos.saturating_add(1)
}

pub fn combined_score(token: &ScoredToken, max_pos: u64) -> u64 {
    token
        .attention_weight
        .saturating_add(position_boost(max_pos, token.token_position))
}

pub fn is_attention_sink(token: &ScoredToken, threshold: u64) -> bool {
    token.attention_weight >= threshold
}

pub fn attention_sinks(tokens: &[ScoredToken], threshold: u64) -> Vec<ScoredToken> {
    tokens
        .iter()
        .copied()
        .filter(|token| is_attention_sink(token, threshold))
        .collect()
}

pub fn in_attention_window(pos: u64, window_start: u64, window_size: u64) -> bool {
    pos >= window_start && pos < window_start.saturating_add(window_size)
}

pub fn windowed_tokens(
    tokens: &[ScoredToken],
    window_start: u64,
    window_size: u64,
) -> Vec<ScoredToken> {
    tokens
        .iter()
        .copied()
        .filter(|token| in_attention_window(token.token_position, window_start, window_size))
        .collect()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StreamingWindow {
    pub sink_tokens: u64,
    pub window_size: u64,
    pub total_seen: u64,
}

pub fn active_positions(window: StreamingWindow) -> Vec<u64> {
    let mut positions = Vec::new();
    for pos in 0..window.sink_tokens {
        positions.push(pos);
    }

    let recent_len = window.window_size.min(window.total_seen);
    let window_start = window.total_seen.saturating_sub(recent_len);
    for pos in window_start..window.total_seen {
        if !positions.contains(&pos) {
            positions.push(pos);
        }
    }
    positions
}

pub fn memory_tokens(window: StreamingWindow) -> u64 {
    window
        .sink_tokens
        .saturating_add(window.window_size.min(window.total_seen))
}

pub fn perplexity(context_quality: u64) -> u64 {
    1000 / context_quality.max(1)
}

pub fn naive_window_quality(window: StreamingWindow) -> u64 {
    if window.total_seen <= window.window_size {
        100
    } else {
        50
    }
}

pub fn streaming_llm_quality(window: StreamingWindow) -> u64 {
    if window.sink_tokens <= 4 { 95 } else { 100 }
}

pub fn sink_correct_compaction(before: &[u64], after: &[u64], sinks: &[u64]) -> bool {
    sinks
        .iter()
        .all(|sink| !before.contains(sink) || after.contains(sink))
}

pub fn preserved_sinks(before: &[u64], after: &[u64], sinks: &[u64]) -> usize {
    sinks
        .iter()
        .filter(|sink| before.contains(sink) && after.contains(sink))
        .count()
}

pub fn quality_of(preserved: usize) -> u64 {
    if preserved >= 4 { 95 } else { 50 }
}

pub fn position_bias(pos: u64, total: u64) -> u64 {
    total.saturating_sub(pos).saturating_mul(100) / total.max(1)
}

pub fn attended(weight: u64) -> bool {
    weight >= 50
}

pub fn attention_entropy(weights: &[u64]) -> usize {
    weights.iter().filter(|&&weight| attended(weight)).count()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn token(id: u64, pos: u64, weight: u64) -> ScoredToken {
        ScoredToken {
            token_id: id,
            token_position: pos,
            attention_weight: weight,
        }
    }

    #[test]
    fn threshold_prune_is_sound_complete_and_idempotent() {
        let tokens = vec![token(1, 0, 40), token(2, 1, 70), token(3, 2, 70)];
        let kept = threshold_prune(&tokens, 70);
        assert_eq!(kept, vec![token(2, 1, 70), token(3, 2, 70)]);
        assert!(kept.iter().all(|token| token.attention_weight >= 70));
        assert!(
            tokens
                .iter()
                .filter(|token| token.attention_weight >= 70)
                .all(|token| kept.contains(token))
        );
        assert_eq!(threshold_prune(&kept, 70), kept);
    }

    #[test]
    fn lowering_threshold_keeps_a_superset() {
        let tokens = vec![token(1, 0, 10), token(2, 1, 50), token(3, 2, 90)];
        let high = threshold_prune(&tokens, 60);
        let low = threshold_prune(&tokens, 40);
        assert!(high.iter().all(|token| low.contains(token)));
        assert!(low.len() >= high.len());
    }

    #[test]
    fn top_k_preserves_sorted_order_and_size_bounds() {
        let tokens = vec![token(1, 0, 90), token(2, 1, 70), token(3, 2, 20)];
        assert!(is_sorted_by_weight(&tokens));
        let kept = top_k(&tokens, 2);
        assert_eq!(kept.len(), 2);
        assert!(kept.len() <= tokens.len());
        assert!(
            kept.windows(2)
                .all(|window| weight_ge(&window[0], &window[1]))
        );
    }

    #[test]
    fn strict_threshold_uses_greater_than_cut() {
        let tokens = vec![token(1, 0, 50), token(2, 1, 51)];
        assert_eq!(strict_threshold_prune(&tokens, 50), vec![token(2, 1, 51)]);
    }

    #[test]
    fn position_score_never_lowers_attention() {
        let t = token(1, 8, 500);
        assert!(combined_score(&t, 10) >= t.attention_weight);
    }

    #[test]
    fn sinks_are_threshold_kept_and_windowed_tokens_are_bounded_with_distinct_positions() {
        let tokens = vec![
            token(1, 10, 100),
            token(2, 11, 20),
            token(3, 12, 90),
            token(4, 20, 80),
        ];
        let sinks = attention_sinks(&tokens, 80);
        let kept = threshold_prune(&tokens, 80);
        assert!(sinks.iter().all(|sink| kept.contains(sink)));

        let windowed = windowed_tokens(&tokens, 10, 3);
        assert_eq!(
            windowed,
            vec![token(1, 10, 100), token(2, 11, 20), token(3, 12, 90)]
        );
        assert!(windowed.len() <= 3);
    }

    #[test]
    fn streaming_window_preserves_sinks_and_recent_endpoint() {
        let window = StreamingWindow {
            sink_tokens: 2,
            window_size: 3,
            total_seen: 10,
        };
        let active = active_positions(window);
        assert!(active.contains(&0));
        assert!(active.contains(&1));
        assert!(active.contains(&9));
        assert!(memory_tokens(window) <= window.sink_tokens + window.window_size);
    }

    #[test]
    fn sink_correct_compaction_preserves_quality_level() {
        let before = vec![0, 1, 2, 3, 9, 10];
        let after = vec![0, 1, 2, 3, 10];
        let sinks = vec![0, 1, 2, 3];
        assert!(sink_correct_compaction(&before, &after, &sinks));
        assert_eq!(preserved_sinks(&before, &after, &sinks), sinks.len());
        assert_eq!(quality_of(preserved_sinks(&before, &after, &sinks)), 95);
    }

    #[test]
    fn streaming_quality_beats_naive_after_window_when_enough_sinks() {
        let window = StreamingWindow {
            sink_tokens: 4,
            window_size: 8,
            total_seen: 20,
        };
        assert!(
            perplexity(streaming_llm_quality(window)) < perplexity(naive_window_quality(window))
        );
    }

    #[test]
    fn sink_block_increases_entropy_by_attended_count() {
        let sinks = vec![50, 90, 100];
        let rest = vec![0, 10, 50];
        let mut combined = sinks.clone();
        combined.extend_from_slice(&rest);
        assert_eq!(
            attention_entropy(&combined),
            sinks.len() + attention_entropy(&rest)
        );
    }
}
