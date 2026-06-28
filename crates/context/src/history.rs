use crate::ContextSkeletonError;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct HistoryAnchor(String);

impl HistoryAnchor {
    pub fn new(anchor: impl Into<String>) -> Result<Self, ContextSkeletonError> {
        let anchor = anchor.into();
        if anchor.trim().is_empty() {
            return Err(ContextSkeletonError::EmptyHistoryAnchor);
        }

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
            return Err(ContextSkeletonError::EmptyCompartmentFingerprint);
        }
        if fingerprint.chars().any(char::is_whitespace) {
            return Err(ContextSkeletonError::InvalidCompartmentFingerprint);
        }

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
            return Err(ContextSkeletonError::IncompleteCompartmentEvents);
        }

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
            return Err(ContextSkeletonError::EmptyCompartmentSequence);
        };
        let mut previous_end = first.range().end();

        for compartment in rest {
            let start = compartment.range().start();
            if start < previous_end {
                return Err(ContextSkeletonError::OverlappingCompartmentRange);
            }
            if start > previous_end {
                return Err(ContextSkeletonError::GappedCompartmentRange);
            }
            previous_end = compartment.range().end();
        }

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
