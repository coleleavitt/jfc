//! Semantic hashing, chunking, and deduplication primitives.
//!
//! This module mirrors the deterministic core of
//! `rcoq-tests/theorems/SemanticHashing.v`: fixed-width hashes are bounded and
//! deterministic, hash mismatches prove inequality, LSH lookup is a pure
//! first-match scan, deduplication never increases retained token count, and
//! chunking is lossless for positive chunk sizes.

pub type SemanticHash = u64;
pub type ChunkId = u64;
pub type DedupeIndex = Vec<(SemanticHash, ChunkId)>;

pub const HASH_BITS: u32 = 64;
pub const LSH_THRESHOLD: u64 = 8;
pub const NUM_BANDS: usize = 8;
pub const ROWS_PER_BAND: usize = 8;

pub fn content_hash(value: u64) -> SemanticHash {
    value
}

pub fn content_hash_u128(value: u128) -> SemanticHash {
    value as u64
}

pub fn content_hash_bytes(bytes: &[u8]) -> SemanticHash {
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    for &byte in bytes {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

pub fn hamming_distance(left: SemanticHash, right: SemanticHash) -> u64 {
    left.max(right) - left.min(right)
}

pub fn semantically_similar(left: SemanticHash, right: SemanticHash) -> bool {
    hamming_distance(left, right) <= LSH_THRESHOLD
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContentChunk {
    pub chunk_id: ChunkId,
    pub chunk_token_count: u64,
    pub chunk_hash: SemanticHash,
    pub chunk_embedding: u64,
}

pub fn find_similar(hash: SemanticHash, index: &[(SemanticHash, ChunkId)]) -> Option<ChunkId> {
    index
        .iter()
        .find(|(stored_hash, _)| semantically_similar(hash, *stored_hash))
        .map(|(_, chunk_id)| *chunk_id)
}

pub fn insert_index(hash: SemanticHash, id: ChunkId, index: &mut DedupeIndex) {
    index.insert(0, (hash, id));
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DedupeResult {
    pub unique_chunks: Vec<ContentChunk>,
    pub references: Vec<(usize, ChunkId)>,
    pub tokens_saved: u64,
}

pub fn deduplicate(chunks: &[ContentChunk]) -> DedupeResult {
    let mut index = DedupeIndex::new();
    let mut unique_chunks = Vec::new();
    let mut references = Vec::new();
    let mut tokens_saved = 0u64;

    for (position, chunk) in chunks.iter().enumerate() {
        if let Some(ref_id) = find_similar(chunk.chunk_hash, &index) {
            references.push((position, ref_id));
            tokens_saved = tokens_saved.saturating_add(chunk.chunk_token_count);
        } else {
            insert_index(chunk.chunk_hash, chunk.chunk_id, &mut index);
            unique_chunks.push(chunk.clone());
        }
    }

    DedupeResult {
        unique_chunks,
        references,
        tokens_saved,
    }
}

pub fn chunks_tokens(chunks: &[ContentChunk]) -> u64 {
    chunks.iter().fold(0u64, |sum, chunk| {
        sum.saturating_add(chunk.chunk_token_count)
    })
}

pub fn band_hash(hash: SemanticHash, band: usize) -> u8 {
    if band >= NUM_BANDS {
        0
    } else {
        ((hash >> (band * ROWS_PER_BAND)) & 0xff) as u8
    }
}

pub fn band_collision(left: SemanticHash, right: SemanticHash, band: usize) -> bool {
    band_hash(left, band) == band_hash(right, band)
}

pub fn any_band_collision(left: SemanticHash, right: SemanticHash) -> bool {
    (0..NUM_BANDS).any(|band| band_collision(left, right, band))
}

pub fn simhash(signs: &[bool]) -> SemanticHash {
    signs.iter().fold(0u64, |acc, sign| {
        acc.wrapping_mul(2).wrapping_add(u64::from(*sign))
    })
}

pub fn chunk_stream<T: Clone>(tokens: &[T], size: usize) -> Vec<Vec<T>> {
    if size == 0 {
        return Vec::new();
    }
    tokens.chunks(size).map(|chunk| chunk.to_vec()).collect()
}

pub fn expected_savings(total_chunks: u64, duplicate_rate: u64) -> u64 {
    total_chunks.saturating_mul(duplicate_rate) / 100
}

pub fn incremental_dedupe(
    new_chunk: &ContentChunk,
    index: &DedupeIndex,
) -> (Option<ChunkId>, DedupeIndex) {
    match find_similar(new_chunk.chunk_hash, index) {
        Some(ref_id) => (Some(ref_id), index.clone()),
        None => {
            let mut next = index.clone();
            insert_index(new_chunk.chunk_hash, new_chunk.chunk_id, &mut next);
            (None, next)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chunk(id: ChunkId, tokens: u64, hash: SemanticHash) -> ContentChunk {
        ContentChunk {
            chunk_id: id,
            chunk_token_count: tokens,
            chunk_hash: hash,
            chunk_embedding: hash,
        }
    }

    #[test]
    fn content_hash_is_deterministic_bounded_and_not_injective() {
        assert_eq!(content_hash(42), content_hash(42));
        assert_ne!(content_hash(41), content_hash(42));
        assert_eq!(content_hash_u128(0), content_hash_u128(1u128 << HASH_BITS));
    }

    #[test]
    fn hash_mismatch_is_sound_inequality_oracle() {
        let a = 10u64;
        let b = 20u64;
        assert_ne!(content_hash(a), content_hash(b));
        assert_ne!(a, b);
    }

    #[test]
    fn similar_lookup_returns_indexed_chunk() {
        let index = vec![(100, 1), (500, 2)];
        assert_eq!(find_similar(104, &index), Some(1));
        assert_eq!(find_similar(200, &index), None);
    }

    #[test]
    fn dedupe_reduces_tokens_and_references_are_valid() {
        let chunks = vec![chunk(1, 10, 100), chunk(2, 15, 105), chunk(3, 7, 500)];
        let result = deduplicate(&chunks);
        assert!(chunks_tokens(&result.unique_chunks) <= chunks_tokens(&chunks));
        assert_eq!(result.tokens_saved, 15);
        for (_, ref_id) in &result.references {
            assert!(
                result
                    .unique_chunks
                    .iter()
                    .any(|chunk| chunk.chunk_id == *ref_id)
            );
        }
    }

    #[test]
    fn dissimilar_chunk_is_kept() {
        let chunks = vec![chunk(1, 10, 100), chunk(2, 10, 200)];
        let result = deduplicate(&chunks);
        assert_eq!(result.unique_chunks, chunks);
        assert!(result.references.is_empty());
    }

    #[test]
    fn identical_hashes_collide_in_a_band() {
        assert!(any_band_collision(0xabcd, 0xabcd));
        assert_eq!(band_hash(0x1234, 0), 0x34);
        assert_eq!(band_hash(0x1234, 1), 0x12);
    }

    #[test]
    fn simhash_is_deterministic_and_singleton_matches_bit() {
        assert_eq!(simhash(&[true, false, true]), simhash(&[true, false, true]));
        assert_eq!(simhash(&[true]), 1);
        assert_eq!(simhash(&[false]), 0);
    }

    #[test]
    fn chunking_is_lossless_for_positive_size() {
        let tokens = vec![1, 2, 3, 4, 5];
        let chunks = chunk_stream(&tokens, 2);
        let flattened: Vec<_> = chunks.into_iter().flatten().collect();
        assert_eq!(flattened, tokens);
    }

    #[test]
    fn typical_savings_are_significant() {
        assert!(expected_savings(100, 30) >= 25);
    }

    #[test]
    fn incremental_dedupe_matches_lookup_decision() {
        let index = vec![(100, 1)];
        let duplicate = chunk(2, 10, 105);
        let unique = chunk(3, 10, 300);

        let (decision, unchanged) = incremental_dedupe(&duplicate, &index);
        assert_eq!(decision, find_similar(duplicate.chunk_hash, &index));
        assert_eq!(unchanged, index);

        let (decision, grown) = incremental_dedupe(&unique, &index);
        assert_eq!(decision, None);
        assert_eq!(
            find_similar(unique.chunk_hash, &grown),
            Some(unique.chunk_id)
        );
    }
}
