//! KV-cache eviction and compression helpers.
//!
//! The functions here implement the algorithmic surface modeled in
//! `rcoq-tests/theorems/KVCacheEviction.v`: heavy-hitter retention by prefix,
//! FIFO/LRU budget cuts, attention score updates, spatial smoothing, and
//! quantization-size accounting.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KVEntry {
    pub position: u64,
    pub layer: u64,
    pub attention_score: u64,
    pub recent_access: u64,
    pub size_bytes: u64,
}

pub fn update_attention(cache: &[KVEntry], new_scores: &[u64]) -> Vec<KVEntry> {
    cache
        .iter()
        .zip(new_scores.iter())
        .map(|(entry, score)| KVEntry {
            attention_score: entry.attention_score.saturating_add(*score) / 2,
            ..*entry
        })
        .collect()
}

pub fn heavy_hitter_sorted(cache: &[KVEntry]) -> bool {
    cache
        .windows(2)
        .all(|window| window[0].attention_score >= window[1].attention_score)
}

pub fn evict_by_attention_sorted(cache: &[KVEntry], budget: usize) -> Vec<KVEntry> {
    cache.iter().take(budget).copied().collect()
}

pub fn evicted_by_attention_sorted(cache: &[KVEntry], budget: usize) -> Vec<KVEntry> {
    cache.iter().skip(budget).copied().collect()
}

pub fn evict_by_attention(cache: &[KVEntry], budget: usize) -> Vec<KVEntry> {
    let mut sorted = cache.to_vec();
    sorted.sort_by_key(|entry| std::cmp::Reverse(entry.attention_score));
    evict_by_attention_sorted(&sorted, budget)
}

pub fn evict_fifo(cache: &[KVEntry], budget: usize) -> Vec<KVEntry> {
    let start = cache.len().saturating_sub(budget);
    cache[start..].to_vec()
}

pub fn evict_lru_sorted(cache: &[KVEntry], budget: usize) -> Vec<KVEntry> {
    cache.iter().take(budget).copied().collect()
}

pub fn combined_score(entry: &KVEntry) -> u64 {
    entry.attention_score.saturating_add(entry.recent_access)
}

pub fn evict_combined_sorted(cache: &[KVEntry], budget: usize) -> Vec<KVEntry> {
    cache.iter().take(budget).copied().collect()
}

pub fn insert_entry(cache: &[KVEntry], entry: KVEntry) -> Vec<KVEntry> {
    let mut next = Vec::with_capacity(cache.len() + 1);
    next.push(entry);
    next.extend_from_slice(cache);
    next
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SharedPool {
    pub compression_ratio: u64,
    pub agent_count: u64,
}

pub fn shared_pool_memory(entries: &[KVEntry]) -> u64 {
    entries
        .iter()
        .fold(0u64, |sum, entry| sum.saturating_add(entry.size_bytes))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LayerReconstruction {
    pub layer: u64,
    pub error: u64,
    pub compression: u64,
}

pub fn optimal_compression(layers: &[LayerReconstruction], error_threshold: u64) -> u64 {
    layers
        .iter()
        .filter(|layer| layer.error <= error_threshold)
        .fold(0u64, |sum, layer| sum.saturating_add(layer.compression))
}

pub fn spatial_smooth(scores: &[u64], window: usize) -> Vec<u64> {
    (0..scores.len())
        .map(|i| {
            let start = i.saturating_sub(window / 2);
            let end = start.saturating_add(window).min(scores.len());
            let neighbors = &scores[start..end];
            let sum = neighbors
                .iter()
                .fold(0u64, |sum, score| sum.saturating_add(*score));
            sum / (neighbors.len() as u64).max(1)
        })
        .collect()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CompressedEntry {
    pub original_size: u64,
    pub compressed_size: u64,
    pub quantization_bits: u64,
}

pub fn compress_entry(entry: KVEntry, bits: u64) -> CompressedEntry {
    CompressedEntry {
        original_size: entry.size_bytes,
        compressed_size: entry.size_bytes.saturating_mul(bits) / 32,
        quantization_bits: bits,
    }
}

pub fn joint_eviction(cache: &[KVEntry], memory_budget: u64) -> Vec<KVEntry> {
    let total_compressed = cache
        .iter()
        .fold(0u64, |sum, entry| sum.saturating_add(entry.size_bytes / 2));
    if total_compressed <= memory_budget {
        return cache.to_vec();
    }

    let entry_size = cache
        .first()
        .map(|entry| entry.size_bytes.max(1))
        .unwrap_or(1);
    let budget = memory_budget.saturating_mul(2) / entry_size;
    evict_by_attention_sorted(cache, budget as usize)
}

pub fn context_to_kv(message_tokens: &[u64]) -> Vec<KVEntry> {
    let len = message_tokens.len() as u64;
    message_tokens
        .iter()
        .enumerate()
        .map(|(i, _)| KVEntry {
            position: i as u64,
            layer: 0,
            attention_score: 500,
            recent_access: len.saturating_sub(i as u64),
            size_bytes: 4,
        })
        .collect()
}

pub fn jfc_context_eviction(context: &[u64], budget: usize) -> Vec<u64> {
    let kv = context_to_kv(context);
    evict_by_attention_sorted(&kv, budget)
        .into_iter()
        .map(|entry| entry.position)
        .collect()
}

pub fn kv_memory(num_layers: u64, seq_len: u64, hidden_dim: u64) -> u64 {
    2u64.saturating_mul(num_layers)
        .saturating_mul(seq_len)
        .saturating_mul(hidden_dim)
        .saturating_mul(2)
}

pub fn compressed_kv_memory(
    num_layers: u64,
    seq_len: u64,
    hidden_dim: u64,
    compression_ratio: u64,
) -> u64 {
    kv_memory(num_layers, seq_len, hidden_dim).saturating_mul(compression_ratio) / 100
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(position: u64, score: u64, recent: u64, size: u64) -> KVEntry {
        KVEntry {
            position,
            layer: 0,
            attention_score: score,
            recent_access: recent,
            size_bytes: size,
        }
    }

    #[test]
    fn update_attention_zips_and_averages_scores() {
        let cache = vec![entry(0, 100, 9, 32), entry(1, 300, 8, 32)];
        let updated = update_attention(&cache, &[500]);
        assert_eq!(updated, vec![entry(0, 300, 9, 32)]);
    }

    #[test]
    fn attention_eviction_respects_budget_and_preserves_heavy_hitters() {
        let cache = vec![
            entry(0, 900, 1, 32),
            entry(1, 700, 1, 32),
            entry(2, 200, 1, 32),
        ];
        assert!(heavy_hitter_sorted(&cache));
        let retained = evict_by_attention_sorted(&cache, 2);
        let evicted = evicted_by_attention_sorted(&cache, 2);
        assert!(retained.len() <= 2);
        assert!(retained.iter().all(|kept| {
            evicted
                .iter()
                .all(|dropped| kept.attention_score >= dropped.attention_score)
        }));
    }

    #[test]
    fn eviction_budget_monotonicity_retains_superset() {
        let cache = vec![
            entry(0, 900, 1, 32),
            entry(1, 700, 1, 32),
            entry(2, 200, 1, 32),
        ];
        let small = evict_by_attention_sorted(&cache, 1);
        let large = evict_by_attention_sorted(&cache, 2);
        assert!(small.iter().all(|entry| large.contains(entry)));
    }

    #[test]
    fn fifo_keeps_newest_suffix_and_lru_keeps_prefix() {
        let cache = vec![entry(0, 1, 1, 32), entry(1, 1, 2, 32), entry(2, 1, 3, 32)];
        assert_eq!(
            evict_fifo(&cache, 2),
            vec![entry(1, 1, 2, 32), entry(2, 1, 3, 32)]
        );
        assert_eq!(
            evict_lru_sorted(&cache, 2),
            vec![entry(0, 1, 1, 32), entry(1, 1, 2, 32)]
        );
    }

    #[test]
    fn insert_then_evict_full_cache_returns_budget_length() {
        let cache = vec![entry(0, 900, 1, 32), entry(1, 700, 1, 32)];
        let inserted = insert_entry(&cache, entry(2, 1000, 1, 32));
        assert_eq!(
            evict_by_attention_sorted(&inserted, cache.len()).len(),
            cache.len()
        );
    }

    #[test]
    fn optimal_compression_is_monotone_in_error_threshold() {
        let layers = vec![
            LayerReconstruction {
                layer: 0,
                error: 5,
                compression: 10,
            },
            LayerReconstruction {
                layer: 1,
                error: 20,
                compression: 30,
            },
        ];
        assert!(optimal_compression(&layers, 5) <= optimal_compression(&layers, 20));
    }

    #[test]
    fn smoothing_preserves_length() {
        let scores = vec![10, 20, 30, 40];
        assert_eq!(spatial_smooth(&scores, 3).len(), scores.len());
    }

    #[test]
    fn compression_bits_at_or_below_32_do_not_grow_entry() {
        let compressed = compress_entry(entry(0, 0, 0, 128), 8);
        assert!(compressed.compressed_size <= compressed.original_size);
    }

    #[test]
    fn joint_eviction_keeps_all_when_compressed_cache_fits() {
        let cache = vec![entry(0, 900, 1, 32), entry(1, 700, 1, 32)];
        assert_eq!(joint_eviction(&cache, 32), cache);
    }

    #[test]
    fn jfc_context_eviction_is_bounded() {
        let context = vec![10, 20, 30, 40];
        let retained = jfc_context_eviction(&context, 2);
        assert_eq!(retained.len(), 2);
        assert!(retained.len() <= 2);
    }

    #[test]
    fn compressed_kv_memory_does_not_exceed_raw_at_ratio_100_or_less() {
        assert!(compressed_kv_memory(2, 10, 64, 50) <= kv_memory(2, 10, 64));
    }
}
