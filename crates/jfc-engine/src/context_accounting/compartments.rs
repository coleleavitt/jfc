//! Owned compartment model derived from the live transcript's compaction
//! boundaries.
//!
//! Each `CompactBoundary`-bearing message is a folded-history unit the
//! compaction path produced. This turns those real boundaries into the owned
//! [`jfc_context::CompartmentSequence`] skeleton model, tiered by recency
//! (newest folded → Recent, oldest → Archived). When nothing has been compacted
//! there are no compartments (`None`), matching the sidebar's `0`.
//!
//! This is the honest available granularity: one compartment per compaction
//! summary. A per-raw-message historian with paraphrase tiers + deterministic
//! decay is the larger MC-2 work (it needs a historian producer) and is
//! intentionally NOT faked here.

use crate::types::{ChatMessage, MessagePart};
use jfc_context::{
    Compartment, CompartmentFingerprint, CompartmentRange, CompartmentSequence, CompartmentTier,
    HistoryEvent, HistoryEventIndex,
};

/// Build the owned compartment sequence from the transcript's compaction
/// boundaries. Returns `None` when nothing has been compacted, or if the
/// validated skeleton constructors reject the derived shape — graceful by
/// design: it never panics and never invents data.
pub fn build_compartment_sequence(messages: &[ChatMessage]) -> Option<CompartmentSequence> {
    let boundary_count = messages
        .iter()
        .filter(|message| message.is_compact_boundary())
        .count();
    if boundary_count == 0 {
        return None;
    }

    let mut compartments = Vec::with_capacity(boundary_count);
    for index in 0..boundary_count {
        let position = index as u64;
        let range = CompartmentRange::new(
            HistoryEventIndex::new(position),
            HistoryEventIndex::new(position + 1),
        )
        .ok()?;
        let event = HistoryEvent::new(
            HistoryEventIndex::new(position),
            CompartmentFingerprint::new(format!("compaction-event-{index}")).ok()?,
        );
        let compartment = Compartment::new(
            tier_for(index, boundary_count),
            range,
            CompartmentFingerprint::new(format!("compaction-{index}")).ok()?,
            vec![event],
        )
        .ok()?;
        compartments.push(compartment);
    }

    CompartmentSequence::new(compartments).ok()
}

/// Total tokens folded into compaction boundaries — the `pre_tokens` the
/// compaction path recorded for each summary.
pub fn compartment_total_tokens(messages: &[ChatMessage]) -> u64 {
    messages
        .iter()
        .flat_map(|message| &message.parts)
        .filter_map(|part| match part {
            MessagePart::CompactBoundary { pre_tokens } => Some(*pre_tokens as u64),
            _ => None,
        })
        .sum()
}

/// Recency-tier the `index`-th compartment of `count` (oldest first): the newest
/// folded history is `Recent`, the oldest is `Archived`.
fn tier_for(index: usize, count: usize) -> CompartmentTier {
    match count - 1 - index {
        0 => CompartmentTier::Recent,
        1 => CompartmentTier::Warm,
        2 => CompartmentTier::Cold,
        _ => CompartmentTier::Archived,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn boundary(pre_tokens: usize) -> ChatMessage {
        ChatMessage::compact_boundary("summary", pre_tokens)
    }

    #[test]
    fn no_boundaries_yields_none_normal() {
        assert!(build_compartment_sequence(&[]).is_none());
    }

    #[test]
    fn boundaries_become_recency_tiered_compartments_normal() {
        let messages = vec![boundary(1_000), boundary(2_000), boundary(3_000)];
        let sequence = build_compartment_sequence(&messages).expect("three boundaries");
        assert_eq!(sequence.compartments().len(), 3);
        // Newest is Recent, oldest is Cold (3 boundaries: Cold, Warm, Recent).
        assert_eq!(sequence.compartments()[2].tier(), CompartmentTier::Recent);
        assert_eq!(sequence.compartments()[0].tier(), CompartmentTier::Cold);
        assert_eq!(compartment_total_tokens(&messages), 6_000);
    }
}
