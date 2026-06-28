use std::fmt::{Display, Formatter};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContextSkeletonError {
    EmptyLayout,
    IncompleteLayout,
    EmptyContributorId,
    EmptyContributorLabel,
    EmptyMemoryAnchor,
    EmptyHistoryAnchor,
    EmptyCompartmentFingerprint,
    InvalidCompartmentFingerprint,
    EmptyCompartmentRange,
    EmptyCompartmentSequence,
    OverlappingCompartmentRange,
    GappedCompartmentRange,
    IncompleteCompartmentEvents,
    EmptyReducePlan,
    InvalidContextDropRange,
    ProtectedTailStartRequired,
    InvalidProtectedTailStart,
    UnexpectedProtectedTailStart,
    InvalidProviderToolPair,
    EmptySearchQuery,
    EmptyHealthUpdateCause,
}

impl Display for ContextSkeletonError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::EmptyLayout => "context layout must contain at least one module",
            Self::IncompleteLayout => "context layout is missing destination modules",
            Self::EmptyContributorId => "context contributor id cannot be empty",
            Self::EmptyContributorLabel => "context contributor label cannot be empty",
            Self::EmptyMemoryAnchor => "memory anchor cannot be empty",
            Self::EmptyHistoryAnchor => "history anchor cannot be empty",
            Self::EmptyCompartmentFingerprint => "compartment fingerprint cannot be empty",
            Self::InvalidCompartmentFingerprint => {
                "compartment fingerprint cannot contain whitespace"
            }
            Self::EmptyCompartmentRange => "compartment range must include at least one event",
            Self::EmptyCompartmentSequence => "compartment sequence cannot be empty",
            Self::OverlappingCompartmentRange => "compartment ranges overlap",
            Self::GappedCompartmentRange => "compartment ranges must be contiguous",
            Self::IncompleteCompartmentEvents => "compartment events must exactly cover the range",
            Self::EmptyReducePlan => "reduce plan cannot be empty",
            Self::InvalidContextDropRange => "context drop range is invalid",
            Self::ProtectedTailStartRequired => {
                "protected tail skip requires a protected tail start"
            }
            Self::InvalidProtectedTailStart => "protected tail start is invalid",
            Self::UnexpectedProtectedTailStart => {
                "protected tail start is only valid for protected-tail skip"
            }
            Self::InvalidProviderToolPair => "provider tool pair must be adjacent and non-empty",
            Self::EmptySearchQuery => "search query cannot be empty",
            Self::EmptyHealthUpdateCause => "context health update cause cannot be empty",
        })
    }
}

impl std::error::Error for ContextSkeletonError {}
