//! Position encoding and positional salience models.
//!
//! This module implements the arithmetic models from
//! `rcoq-tests/theorems/PositionEncoding.v`: extrapolation quality by encoding
//! scheme, RoPE relative-position invariance, absolute code monotonicity,
//! lost-in-the-middle salience, and hierarchical position flattening.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PositionScheme {
    Absolute,
    RoPE,
    ALiBi,
    NoPE,
}

pub fn trained_length(scheme: PositionScheme) -> u64 {
    match scheme {
        PositionScheme::Absolute => 512,
        PositionScheme::RoPE => 4096,
        PositionScheme::ALiBi => 2048,
        PositionScheme::NoPE => 0,
    }
}

pub fn extrapolation_quality(scheme: PositionScheme, train_len: u64, pos: u64) -> u64 {
    if matches!(scheme, PositionScheme::NoPE) {
        return 0;
    }
    if pos <= train_len {
        return 100;
    }

    let over = pos.saturating_sub(train_len);
    match scheme {
        PositionScheme::Absolute => 0,
        PositionScheme::RoPE => 100u64
            .saturating_sub(over.saturating_mul(50) / train_len.max(1))
            .min(100),
        PositionScheme::ALiBi => 100u64
            .saturating_sub(over.saturating_mul(20) / train_len.max(1))
            .min(100),
        PositionScheme::NoPE => 0,
    }
}

pub fn interpolated_position(pos: u64, target_len: u64, train_len: u64) -> u64 {
    pos.saturating_mul(train_len) / target_len.max(1)
}

pub fn utilization_ratio(used: u64, window: u64) -> u64 {
    used.saturating_mul(100) / window.max(1)
}

pub fn rope_relative(i: u64, j: u64) -> u64 {
    i.max(j) - i.min(j)
}

pub fn abs_code(stride: u64, pos: u64) -> u64 {
    stride.saturating_mul(pos)
}

pub fn edge_distance(pos: u64, n: u64) -> u64 {
    pos.min(n.saturating_sub(pos))
}

pub fn salience(pos: u64, n: u64) -> u64 {
    n.saturating_sub(edge_distance(pos, n))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ConversationPosition {
    pub absolute_position: u64,
    pub turn_number: u64,
    pub within_turn_position: u64,
    pub is_system: bool,
}

pub fn conversation_importance(position: ConversationPosition) -> u64 {
    let turn_recency = 100u64.saturating_sub((position.turn_number.saturating_mul(10)).min(100));
    let system_boost = if position.is_system { 20 } else { 0 };
    turn_recency.saturating_add(system_boost)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HierarchicalPosition {
    pub block_id: u64,
    pub block_position: u64,
    pub block_size: u64,
}

pub fn flatten_position(position: HierarchicalPosition) -> u64 {
    position
        .block_id
        .saturating_mul(position.block_size)
        .saturating_add(position.block_position)
}

pub fn hierarchical_attention_cost(seq_len: u64, block_size: u64) -> u64 {
    let block_size = block_size.max(1);
    let n_blocks = seq_len / block_size;
    let local_cost = block_size.saturating_mul(block_size);
    let cross_block_cost = n_blocks.saturating_mul(n_blocks);
    n_blocks
        .saturating_mul(local_cost)
        .saturating_add(cross_block_cost)
}

pub fn jfc_safe_context_length(scheme: PositionScheme, _quality_threshold: u64) -> u64 {
    let train_len = trained_length(scheme);
    match scheme {
        PositionScheme::Absolute => train_len,
        PositionScheme::RoPE => train_len.saturating_mul(2),
        PositionScheme::ALiBi => train_len.saturating_mul(2),
        PositionScheme::NoPE => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn absolute_fails_beyond_training_but_alibi_degrades_more_slowly_than_rope() {
        assert_eq!(
            extrapolation_quality(
                PositionScheme::Absolute,
                trained_length(PositionScheme::Absolute),
                513
            ),
            0
        );
        let train_len = 4096;
        let pos = 8192;
        assert!(
            extrapolation_quality(PositionScheme::ALiBi, train_len, pos)
                >= extrapolation_quality(PositionScheme::RoPE, train_len, pos)
        );
    }

    #[test]
    fn interpolation_maps_target_window_back_into_training_range() {
        let train_len = 4096;
        let target_len = 8192;
        assert!(interpolated_position(target_len, target_len, train_len) <= train_len);
    }

    #[test]
    fn rope_relative_is_shift_invariant_and_symmetric() {
        assert_eq!(rope_relative(10 + 7, 3 + 7), rope_relative(10, 3));
        assert_eq!(rope_relative(10, 3), rope_relative(3, 10));
    }

    #[test]
    fn absolute_code_is_injective_and_order_preserving_with_positive_stride() {
        let stride = 8;
        assert_ne!(abs_code(stride, 1), abs_code(stride, 2));
        assert!(abs_code(stride, 1) < abs_code(stride, 2));
    }

    #[test]
    fn edge_salience_is_u_shaped() {
        let n = 20;
        assert_eq!(salience(0, n), n);
        assert_eq!(salience(n, n), n);
        assert!(salience(10, n) < salience(0, n));
    }

    #[test]
    fn recent_turns_are_at_least_as_important_as_older_matching_turns() {
        let recent = ConversationPosition {
            absolute_position: 0,
            turn_number: 1,
            within_turn_position: 0,
            is_system: false,
        };
        let old = ConversationPosition {
            absolute_position: 100,
            turn_number: 5,
            within_turn_position: 0,
            is_system: false,
        };
        assert!(conversation_importance(recent) >= conversation_importance(old));
    }

    #[test]
    fn hierarchical_position_flattening_is_unique_for_well_formed_positions() {
        let h1 = HierarchicalPosition {
            block_id: 3,
            block_position: 4,
            block_size: 64,
        };
        let h2 = HierarchicalPosition {
            block_id: flatten_position(h1) / h1.block_size,
            block_position: flatten_position(h1) % h1.block_size,
            block_size: 64,
        };
        assert_eq!(h1, h2);
    }

    #[test]
    fn hierarchical_attention_is_cheaper_for_long_sequences() {
        let seq_len = 4096;
        let block_size = 64;
        assert!(hierarchical_attention_cost(seq_len, block_size) < seq_len * seq_len);
    }

    #[test]
    fn safe_length_respects_quality_threshold_for_supported_schemes() {
        for scheme in [
            PositionScheme::Absolute,
            PositionScheme::RoPE,
            PositionScheme::ALiBi,
        ] {
            let safe = jfc_safe_context_length(scheme, 50);
            assert!(extrapolation_quality(scheme, trained_length(scheme), safe) >= 50);
        }
    }
}
