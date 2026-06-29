use crate::{ContextSkeletonError, trace};

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct HistoryAnchor(String);

impl HistoryAnchor {
    pub fn new(anchor: impl Into<String>) -> Result<Self, ContextSkeletonError> {
        let anchor = anchor.into();
        if anchor.trim().is_empty() {
            trace::record_status("context.history_anchor.new", "empty");
            return Err(ContextSkeletonError::EmptyHistoryAnchor);
        }

        trace::record_text_shape(trace::TextShape {
            label: "context.history_anchor.new",
            field: "anchor_bytes",
            bytes: anchor.len(),
        });
        Ok(Self(anchor))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct HistoryEventIndex(u64);

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CompartmentFingerprint(String);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompartmentTier {
    Recent,
    Warm,
    Cold,
    Archived,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CompartmentRange {
    start: HistoryEventIndex,
    end: HistoryEventIndex,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HistoryEvent {
    index: HistoryEventIndex,
    fingerprint: CompartmentFingerprint,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Compartment {
    tier: CompartmentTier,
    range: CompartmentRange,
    fingerprint: CompartmentFingerprint,
    events: Vec<HistoryEvent>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompartmentSequence {
    compartments: Vec<Compartment>,
}

impl HistoryEventIndex {
    pub const fn new(index: u64) -> Self {
        Self(index)
    }

    pub const fn get(self) -> u64 {
        self.0
    }
}

impl CompartmentFingerprint {
    pub fn new(fingerprint: impl Into<String>) -> Result<Self, ContextSkeletonError> {
        let fingerprint = fingerprint.into();
        if fingerprint.trim().is_empty() {
            trace::record_status("context.compartment_fingerprint.new", "empty");
            return Err(ContextSkeletonError::EmptyCompartmentFingerprint);
        }
        if fingerprint.chars().any(char::is_whitespace) {
            trace::record_status("context.compartment_fingerprint.new", "invalid");
            return Err(ContextSkeletonError::InvalidCompartmentFingerprint);
        }

        trace::record_text_shape(trace::TextShape {
            label: "context.compartment_fingerprint.new",
            field: "fingerprint_bytes",
            bytes: fingerprint.len(),
        });
        Ok(Self(fingerprint))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl CompartmentRange {
    pub const fn new(
        start: HistoryEventIndex,
        end: HistoryEventIndex,
    ) -> Result<Self, ContextSkeletonError> {
        if start.0 >= end.0 {
            return Err(ContextSkeletonError::EmptyCompartmentRange);
        }

        Ok(Self { start, end })
    }

    pub const fn start(self) -> HistoryEventIndex {
        self.start
    }

    pub const fn end(self) -> HistoryEventIndex {
        self.end
    }
}

impl HistoryEvent {
    pub fn new(index: HistoryEventIndex, fingerprint: CompartmentFingerprint) -> Self {
        linkscope::record_items("context.history_event.new", 1);
        if linkscope::trace_detail_enabled() {
            linkscope::detail_event_fields(
                "context.history_event.new",
                [linkscope::TraceField::count("index", index.get())],
            );
        }
        Self { index, fingerprint }
    }

    pub const fn index(&self) -> HistoryEventIndex {
        self.index
    }

    pub fn fingerprint(&self) -> &CompartmentFingerprint {
        &self.fingerprint
    }
}

impl Compartment {
    pub fn new(
        tier: CompartmentTier,
        range: CompartmentRange,
        fingerprint: CompartmentFingerprint,
        events: Vec<HistoryEvent>,
    ) -> Result<Self, ContextSkeletonError> {
        if !events_cover_range(&events, range) {
            trace::record_range_shape(trace::RangeShape {
                label: "context.compartment.new.incomplete_events",
                range,
                items: events.len(),
            });
            return Err(ContextSkeletonError::IncompleteCompartmentEvents);
        }

        trace::record_compartment("context.compartment.new", tier, range, events.len());
        Ok(Self {
            tier,
            range,
            fingerprint,
            events,
        })
    }

    pub const fn tier(&self) -> CompartmentTier {
        self.tier
    }

    pub const fn range(&self) -> CompartmentRange {
        self.range
    }

    pub fn fingerprint(&self) -> &CompartmentFingerprint {
        &self.fingerprint
    }

    pub fn events(&self) -> &[HistoryEvent] {
        &self.events
    }
}

impl CompartmentSequence {
    pub fn new(compartments: Vec<Compartment>) -> Result<Self, ContextSkeletonError> {
        let Some((first, rest)) = compartments.split_first() else {
            trace::record_status("context.compartment_sequence.new", "empty");
            return Err(ContextSkeletonError::EmptyCompartmentSequence);
        };
        let mut previous_end = first.range().end();

        for compartment in rest {
            let start = compartment.range().start();
            if start < previous_end {
                trace::record_status("context.compartment_sequence.new", "overlap");
                return Err(ContextSkeletonError::OverlappingCompartmentRange);
            }
            if start > previous_end {
                trace::record_status("context.compartment_sequence.new", "gap");
                return Err(ContextSkeletonError::GappedCompartmentRange);
            }
            previous_end = compartment.range().end();
        }

        trace::record_sequence(
            "context.compartment_sequence.new",
            compartments.len(),
            first.range().start().get(),
            previous_end.get(),
        );
        Ok(Self { compartments })
    }

    pub fn compartments(&self) -> &[Compartment] {
        &self.compartments
    }
}

fn events_cover_range(events: &[HistoryEvent], range: CompartmentRange) -> bool {
    let Ok(event_count) = u64::try_from(events.len()) else {
        return false;
    };
    if event_count != range.end().get() - range.start().get() {
        return false;
    }

    events
        .iter()
        .map(|event| event.index().get())
        .eq(range.start().get()..range.end().get())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fingerprint(value: &str) -> CompartmentFingerprint {
        CompartmentFingerprint::new(value).expect("valid fingerprint")
    }

    #[test]
    fn history_trace_records_shape_without_anchor_or_fingerprint_payload_normal() {
        linkscope::trace_detail_enable();
        let anchor = HistoryAnchor::new("private-history-anchor").expect("valid anchor");
        let fp = fingerprint("privatefingerprint");
        let range = CompartmentRange::new(HistoryEventIndex::new(0), HistoryEventIndex::new(2))
            .expect("valid range");
        let events = vec![
            HistoryEvent::new(HistoryEventIndex::new(0), fp.clone()),
            HistoryEvent::new(HistoryEventIndex::new(1), fp.clone()),
        ];
        let compartment =
            Compartment::new(CompartmentTier::Warm, range, fp, events).expect("valid compartment");
        let sequence = CompartmentSequence::new(vec![compartment]).expect("valid sequence");

        assert_eq!(anchor.as_str(), "private-history-anchor");
        assert_eq!(sequence.compartments().len(), 1);
        let rendered = format!("{:?}", linkscope::snapshot());
        assert!(rendered.contains("context.history_anchor.new"));
        assert!(rendered.contains("context.compartment_fingerprint.new"));
        assert!(rendered.contains("context.history_event.new"));
        assert!(rendered.contains("context.compartment.new"));
        assert!(rendered.contains("context.compartment_sequence.new"));
        assert!(rendered.contains("warm"));
        assert!(!rendered.contains("private-history-anchor"));
        assert!(!rendered.contains("privatefingerprint"));
    }
}
