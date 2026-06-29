//! Proof-backed context-management policy used by the runtime.
//!
//! This is the small integration layer that composes theorem primitives into
//! decisions the engine can consume directly. The richer modules
//! (`attention`, `semantic_hash`, `kv_cache`, etc.) stay pure until the runtime
//! has real attention scores, embeddings, or KV metadata to feed them.

use crate::{
    TurnCost, attention, information_bottleneck, kv_cache, position_encoding, select_retained,
};

pub const RECENCY_PRESERVE_FRACTION_NUM: usize = 30;
pub const RECENCY_PRESERVE_FRACTION_DEN: usize = 100;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContextItemKind {
    SystemSink,
    CompactSummary,
    Memory,
    UserText,
    AssistantText,
    ToolInput,
    ToolOutput,
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ContextItem {
    pub id: u64,
    /// Monotone position in the retained context, oldest first.
    pub position: u64,
    pub token_estimate: u64,
    pub kind: ContextItemKind,
    /// Number of known exact/similar repeats before this item.
    pub duplicate_count: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScoredContextItem {
    pub item: ContextItem,
    /// Synthetic salience score on the same 0-1000 scale as the theorem
    /// attention model. This is not provider attention; it is the runtime
    /// fallback when attention/KV tensors are unavailable.
    pub score: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ContextSignals {
    /// True provider attention/importance scores, when a backend exposes them.
    /// `token_id` is the context item id used by the caller.
    pub attention_tokens: Vec<attention::ScoredToken>,
    /// True provider KV metadata, when a backend exposes cache internals.
    /// `position` is the context item id used by the caller.
    pub kv_entries: Vec<kv_cache::KVEntry>,
}

impl ContextSignals {
    pub fn is_empty(&self) -> bool {
        self.attention_tokens.is_empty() && self.kv_entries.is_empty()
    }
}

const SYNTHETIC_SINK_SCORE: u64 = 900;

fn kind_base_score(kind: ContextItemKind) -> u64 {
    match kind {
        ContextItemKind::SystemSink => 950,
        ContextItemKind::CompactSummary => 920,
        ContextItemKind::Memory => 860,
        ContextItemKind::UserText => 760,
        ContextItemKind::ToolInput => 720,
        ContextItemKind::AssistantText => 640,
        ContextItemKind::ToolOutput => 520,
        ContextItemKind::Other => 500,
    }
}

fn is_semantic_sink(kind: ContextItemKind) -> bool {
    matches!(
        kind,
        ContextItemKind::SystemSink | ContextItemKind::CompactSummary | ContextItemKind::Memory
    )
}

fn usize_to_u64(value: usize) -> u64 {
    value.try_into().unwrap_or(u64::MAX)
}

/// Score runtime context when provider-side attention/KV metadata is absent.
///
/// The score composes the proof-backed pieces we do have:
/// - role/type priors model semantic sinks;
/// - `position_boost` gives recency;
/// - lost-in-the-middle edge salience protects the beginning and end;
/// - the information-bottleneck compressibility proxy penalizes repeated/noisy
///   tool output;
/// - scores are then consumable by threshold pruning or synthetic KV eviction.
pub fn score_context_item(item: ContextItem, total_items: usize) -> ScoredContextItem {
    let last_pos = usize_to_u64(total_items.saturating_sub(1)).max(1);
    let pos = item.position.min(last_pos);
    let recency = attention::position_boost(last_pos, pos);
    let edge = position_encoding::salience(pos, last_pos).saturating_mul(100) / last_pos.max(1);
    let duplicate_penalty = item.duplicate_count.saturating_mul(120).min(360);
    let noise = match item.kind {
        ContextItemKind::ToolOutput => 120,
        ContextItemKind::Other => 60,
        _ => 0,
    };
    let compressibility =
        information_bottleneck::compressibility(information_bottleneck::ConversationContext {
            total_tokens: item.token_estimate.max(1),
            semantic_content: kind_base_score(item.kind).min(item.token_estimate.max(1)),
            redundancy: item.duplicate_count.saturating_mul(50),
            noise,
        })
        .min(250);

    let mut score = kind_base_score(item.kind)
        .saturating_add(recency)
        .saturating_add(edge / 2)
        .saturating_sub(duplicate_penalty)
        .saturating_sub(compressibility);

    if is_semantic_sink(item.kind) {
        score = score.max(SYNTHETIC_SINK_SCORE);
    }

    ScoredContextItem {
        item,
        score: score.min(1000),
    }
}

pub fn score_context_items(items: &[ContextItem]) -> Vec<ScoredContextItem> {
    items
        .iter()
        .copied()
        .map(|item| score_context_item(item, items.len()))
        .collect()
}

pub fn synthetic_attention_tokens(items: &[ContextItem]) -> Vec<attention::ScoredToken> {
    score_context_items(items)
        .into_iter()
        .map(|scored| attention::ScoredToken {
            token_id: scored.item.id,
            token_position: scored.item.position,
            attention_weight: scored.score,
        })
        .collect()
}

pub fn threshold_prune_context(items: &[ContextItem], gamma: u64) -> Vec<u64> {
    attention::threshold_prune(&synthetic_attention_tokens(items), gamma)
        .into_iter()
        .map(|token| token.token_id)
        .collect()
}

pub fn synthetic_kv_entries(items: &[ContextItem]) -> Vec<kv_cache::KVEntry> {
    score_context_items(items)
        .into_iter()
        .map(|scored| kv_cache::KVEntry {
            // Use the context item id as the synthetic KV position so callers
            // can recover the selected item without a side table.
            position: scored.item.id,
            layer: 0,
            attention_score: scored.score,
            recent_access: attention::position_boost(
                usize_to_u64(items.len().saturating_sub(1)).max(1),
                scored.item.position,
            ),
            size_bytes: scored.item.token_estimate.max(1).saturating_mul(4),
        })
        .collect()
}

pub fn select_context_items_by_count(items: &[ContextItem], budget: usize) -> Vec<u64> {
    let mut kv = synthetic_kv_entries(items);
    kv.sort_by_key(|entry| {
        (
            std::cmp::Reverse(kv_cache::combined_score(entry)),
            entry.position,
        )
    });
    kv_cache::evict_by_attention_sorted(&kv, budget)
        .into_iter()
        .map(|entry| entry.position)
        .collect()
}

pub fn select_context_items_by_token_budget(items: &[ContextItem], token_budget: u64) -> Vec<u64> {
    let mut scored = score_context_items(items);
    scored.sort_by_key(|entry| (std::cmp::Reverse(entry.score), entry.item.position));

    let mut selected = Vec::new();
    let mut spent = 0u64;
    for entry in scored {
        let cost = entry.item.token_estimate.max(1);
        if spent.saturating_add(cost) <= token_budget {
            spent = spent.saturating_add(cost);
            selected.push(entry.item.id);
        }
    }
    selected
}

pub fn select_context_items_with_signals(
    items: &[ContextItem],
    token_budget: u64,
    signals: Option<&ContextSignals>,
) -> Vec<u64> {
    let Some(signals) = signals.filter(|signals| !signals.is_empty()) else {
        return select_context_items_by_token_budget(items, token_budget);
    };

    if !signals.kv_entries.is_empty() {
        let mut scored = signals.kv_entries.clone();
        scored.sort_by_key(|entry| {
            (
                std::cmp::Reverse(kv_cache::combined_score(entry)),
                entry.position,
            )
        });
        return select_ids_by_order(
            items,
            token_budget,
            scored.into_iter().map(|entry| entry.position),
        );
    }

    let mut scored = signals.attention_tokens.clone();
    scored.sort_by_key(|token| {
        (
            std::cmp::Reverse(token.attention_weight),
            token.token_position,
        )
    });
    select_ids_by_order(
        items,
        token_budget,
        scored.into_iter().map(|token| token.token_id),
    )
}

fn select_ids_by_order<I>(items: &[ContextItem], token_budget: u64, ids: I) -> Vec<u64>
where
    I: IntoIterator<Item = u64>,
{
    let mut selected = Vec::new();
    let mut spent = 0u64;
    for id in ids {
        if selected.contains(&id) {
            continue;
        }
        let Some(item) = items.iter().find(|item| item.id == id) else {
            continue;
        };
        let cost = item.token_estimate.max(1);
        if spent.saturating_add(cost) <= token_budget {
            spent = spent.saturating_add(cost);
            selected.push(id);
        }
    }
    selected
}

/// Prune very long text with the StreamingLLM sink+recent-window fallback.
///
/// This is the explicit no-attention-metadata path: preserve a prefix sink and
/// a recent suffix, and mark the omitted middle. The output is always no longer
/// than the original plus the short omission marker, and short text is left
/// untouched.
pub fn prune_text_with_sink_window(text: &str, max_chars: usize) -> String {
    let char_count = text.chars().count();
    if max_chars == 0 || char_count <= max_chars {
        return text.to_owned();
    }

    let sink_chars = (max_chars / 4).max(1).min(char_count);
    let recent_chars = max_chars.saturating_sub(sink_chars).max(1);
    let window = attention::StreamingWindow {
        sink_tokens: usize_to_u64(sink_chars),
        window_size: usize_to_u64(recent_chars),
        total_seen: usize_to_u64(char_count),
    };
    let active = attention::active_positions(window);
    let prefix_len = active
        .iter()
        .take_while(|&&pos| pos < usize_to_u64(sink_chars))
        .count();
    let suffix_len = active.len().saturating_sub(prefix_len);

    if prefix_len.saturating_add(suffix_len) >= char_count {
        return text.to_owned();
    }

    let prefix: String = text.chars().take(prefix_len).collect();
    let suffix: String = text
        .chars()
        .skip(char_count.saturating_sub(suffix_len))
        .collect();
    let omitted = char_count.saturating_sub(prefix_len.saturating_add(suffix_len));
    format!(
        "{prefix}\n[... omitted {omitted} middle chars by context salience window ...]\n{suffix}"
    )
}

/// Token budget assigned to the verbatim newest-context floor.
///
/// This is the integer model from `RecencyRetention.v`:
/// `(window * 30) / 100`.
pub fn recency_budget(window: usize) -> usize {
    window.saturating_mul(RECENCY_PRESERVE_FRACTION_NUM) / RECENCY_PRESERVE_FRACTION_DEN
}

/// Number of newest groups to preserve verbatim on the first compaction pass.
///
/// The input is oldest-first group token counts. The result is clamped to
/// `[1, total - 1]` for multi-group transcripts so the runtime preserves some
/// recent context while still leaving at least one group to summarize.
pub fn recency_preserve_floor(group_tokens: &[usize], window: usize) -> usize {
    let total = group_tokens.len();
    if total <= 1 {
        return 1;
    }

    let budget = usize_to_u64(recency_budget(window));
    let turns: Vec<TurnCost> = group_tokens
        .iter()
        .enumerate()
        .map(|(i, &tokens)| TurnCost {
            id: usize_to_u64(i),
            tokens: usize_to_u64(tokens),
        })
        .collect();
    let retention = select_retained(&turns, budget);
    retention.kept.len().clamp(1, total - 1)
}

/// Smart retry step from `CompressionBounds.v`.
///
/// With an API-reported token gap, walk backward from `current_split`,
/// accumulating group tokens until the gap is covered. Without gap data, fall
/// back to halving the summarization side. The result is always at least one.
pub fn token_gap_step(
    token_gap: Option<usize>,
    group_tokens: &[usize],
    current_split: usize,
) -> usize {
    let Some(gap) = token_gap else {
        return (current_split / 2).max(1);
    };

    let mut freed: usize = 0;
    let mut step: usize = 0;
    for i in (0..current_split).rev() {
        if freed >= gap {
            break;
        }
        freed = freed.saturating_add(group_tokens.get(i).copied().unwrap_or(0));
        step += 1;
    }
    step.max(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recency_preserve_floor_keeps_recent_groups_inside_fraction_budget() {
        let groups = vec![100, 100, 100, 100, 100];
        let floor = recency_preserve_floor(&groups, 1000);
        assert_eq!(floor, 3);
    }

    #[test]
    fn recency_preserve_floor_clamps_to_leave_summary_work() {
        assert_eq!(recency_preserve_floor(&[10], 1000), 1);
        assert_eq!(recency_preserve_floor(&[10, 10], 1000), 1);
    }

    #[test]
    fn token_gap_step_falls_back_to_halving() {
        assert_eq!(token_gap_step(None, &[10, 20, 30, 40], 4), 2);
        assert_eq!(token_gap_step(None, &[10], 1), 1);
    }

    #[test]
    fn token_gap_step_covers_gap_when_possible() {
        let groups = vec![50, 100, 200, 300];
        let split = 4;
        let gap = 450;
        let step = token_gap_step(Some(gap), &groups, split);
        let freed: usize = groups[..split].iter().rev().take(step).sum();
        assert!(freed >= gap);
    }

    #[test]
    fn token_gap_step_saturates_pathological_counts() {
        let groups = vec![usize::MAX, usize::MAX];
        assert_eq!(token_gap_step(Some(usize::MAX), &groups, 2), 1);
    }

    #[test]
    fn token_gap_step_is_always_positive() {
        assert!(token_gap_step(Some(0), &[], 0) >= 1);
        assert!(token_gap_step(None, &[], 0) >= 1);
    }

    #[test]
    fn proxy_context_scores_preserve_semantic_sinks() {
        let items = vec![
            ContextItem {
                id: 1,
                position: 0,
                token_estimate: 500,
                kind: ContextItemKind::CompactSummary,
                duplicate_count: 0,
            },
            ContextItem {
                id: 2,
                position: 1,
                token_estimate: 500,
                kind: ContextItemKind::ToolOutput,
                duplicate_count: 3,
            },
        ];
        let scored = score_context_items(&items);
        assert!(scored[0].score >= SYNTHETIC_SINK_SCORE);
        assert!(threshold_prune_context(&items, SYNTHETIC_SINK_SCORE).contains(&1));
        assert!(scored[0].score > scored[1].score);
    }

    #[test]
    fn synthetic_kv_selection_respects_count_budget() {
        let items = vec![
            ContextItem {
                id: 10,
                position: 0,
                token_estimate: 100,
                kind: ContextItemKind::ToolOutput,
                duplicate_count: 2,
            },
            ContextItem {
                id: 11,
                position: 1,
                token_estimate: 100,
                kind: ContextItemKind::Memory,
                duplicate_count: 0,
            },
            ContextItem {
                id: 12,
                position: 2,
                token_estimate: 100,
                kind: ContextItemKind::UserText,
                duplicate_count: 0,
            },
        ];
        let selected = select_context_items_by_count(&items, 2);
        assert_eq!(selected.len(), 2);
        assert!(selected.contains(&11));
    }

    #[test]
    fn token_budget_selection_never_overspends() {
        let items = vec![
            ContextItem {
                id: 1,
                position: 0,
                token_estimate: 75,
                kind: ContextItemKind::UserText,
                duplicate_count: 0,
            },
            ContextItem {
                id: 2,
                position: 1,
                token_estimate: 50,
                kind: ContextItemKind::AssistantText,
                duplicate_count: 0,
            },
            ContextItem {
                id: 3,
                position: 2,
                token_estimate: 40,
                kind: ContextItemKind::ToolOutput,
                duplicate_count: 0,
            },
        ];
        let selected = select_context_items_by_token_budget(&items, 100);
        let spent: u64 = items
            .iter()
            .filter(|item| selected.contains(&item.id))
            .map(|item| item.token_estimate.max(1))
            .sum();
        assert!(spent <= 100);
    }

    #[test]
    fn true_attention_signals_override_proxy_order() {
        let items = vec![
            ContextItem {
                id: 1,
                position: 0,
                token_estimate: 50,
                kind: ContextItemKind::ToolOutput,
                duplicate_count: 3,
            },
            ContextItem {
                id: 2,
                position: 1,
                token_estimate: 50,
                kind: ContextItemKind::Memory,
                duplicate_count: 0,
            },
        ];
        let signals = ContextSignals {
            attention_tokens: vec![
                attention::ScoredToken {
                    token_id: 1,
                    token_position: 0,
                    attention_weight: 1000,
                },
                attention::ScoredToken {
                    token_id: 2,
                    token_position: 1,
                    attention_weight: 1,
                },
            ],
            kv_entries: Vec::new(),
        };
        assert_eq!(
            select_context_items_with_signals(&items, 50, Some(&signals)),
            vec![1]
        );
    }

    #[test]
    fn true_kv_signals_override_attention_when_present() {
        let items = vec![
            ContextItem {
                id: 1,
                position: 0,
                token_estimate: 50,
                kind: ContextItemKind::UserText,
                duplicate_count: 0,
            },
            ContextItem {
                id: 2,
                position: 1,
                token_estimate: 50,
                kind: ContextItemKind::UserText,
                duplicate_count: 0,
            },
        ];
        let signals = ContextSignals {
            attention_tokens: vec![attention::ScoredToken {
                token_id: 1,
                token_position: 0,
                attention_weight: 1000,
            }],
            kv_entries: vec![kv_cache::KVEntry {
                position: 2,
                layer: 0,
                attention_score: 800,
                recent_access: 400,
                size_bytes: 200,
            }],
        };
        assert_eq!(
            select_context_items_with_signals(&items, 50, Some(&signals)),
            vec![2]
        );
    }

    #[test]
    fn sink_window_text_pruning_keeps_edges() {
        let text = format!("{}{}{}", "A".repeat(100), "M".repeat(1000), "Z".repeat(100));
        let pruned = prune_text_with_sink_window(&text, 200);
        assert!(pruned.starts_with('A'));
        assert!(pruned.ends_with('Z'));
        assert!(pruned.contains("omitted"));
        assert_eq!(prune_text_with_sink_window("short", 200), "short");
    }
}
