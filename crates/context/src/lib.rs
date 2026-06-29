mod error;
mod trace;

pub mod contributors;
pub mod ctx_reduce;
pub mod doctor;
pub mod health;
pub mod history;
pub mod layout;
pub mod memory;
pub mod reduce;
pub mod search;

pub use contributors::{ContextAccount, ContextContributor, ContributorId};
pub use ctx_reduce::{
    ContextDropSpec, ContextReduceOptions, ContextTag, ContextTagId, ContextTagKind,
    ContextTagStatus, PlannedContextDrops, dropped_tag_marker,
};
pub use doctor::{ContextDoctorReport, ContextHealthDoctorEvent, ContextHealthDoctorSummary};
pub use error::ContextSkeletonError;
pub use health::{
    ContextHealth, ContextHealthEvent, ContextHealthEventKind, ContextHealthService,
    ContextHealthStatus, ContextHealthUpdate, InMemoryContextHealthService,
};
pub use history::{
    Compartment, CompartmentFingerprint, CompartmentRange, CompartmentSequence, CompartmentTier,
    HistoryAnchor, HistoryEvent, HistoryEventIndex,
};
pub use layout::{ContextLayout, ContextModule};
pub use memory::MemoryAnchor;
pub use reduce::{
    ContextDropRange, ContextDropReplayMode, ContextReductionQueue, ProviderToolPair,
    QueuedContextDrop, ReducePlan,
};
pub use search::{
    ContextSearchFacade, ContextSearchSource, SearchCandidate, SearchHit, SearchQuery,
    SearchResponse, SearchSourceKind, SearchSourceOutput, SearchSourceReport, SearchSourceSlot,
    SearchSourceStatus, SearchStatus,
};
