mod plan;
mod spec;
mod tag;

pub use plan::{ContextReduceOptions, PlannedContextDrops};
pub use spec::ContextDropSpec;
pub use tag::{ContextTag, ContextTagId, ContextTagKind, ContextTagStatus, dropped_tag_marker};

pub(super) const MAX_DROP_TAGS: usize = 50_000;
