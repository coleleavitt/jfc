use std::fmt::{Display, Formatter};

use crate::ContextSkeletonError;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ContextTagId(u32);

impl ContextTagId {
    pub const fn new(id: u32) -> Result<Self, ContextSkeletonError> {
        if id == 0 {
            return Err(ContextSkeletonError::InvalidContextTagId);
        }

        Ok(Self(id))
    }

    pub const fn get(self) -> u32 {
        self.0
    }
}

impl Display for ContextTagId {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "§{}§", self.0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ContextTagKind {
    Message,
    Tool,
    File,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ContextTagStatus {
    Active,
    PendingDrop,
    Dropped,
    Compacted,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ContextTag {
    id: ContextTagId,
    kind: ContextTagKind,
    status: ContextTagStatus,
}

impl ContextTag {
    pub fn new(
        id: u32,
        kind: ContextTagKind,
        status: ContextTagStatus,
    ) -> Result<Self, ContextSkeletonError> {
        let _linkscope_tag = linkscope::phase("context.tag.new");
        linkscope::detail_event_fields(
            "context.tag.new",
            [
                linkscope::TraceField::count("id", u64::from(id)),
                linkscope::TraceField::text("kind", format!("{kind:?}")),
                linkscope::TraceField::text("status", format!("{status:?}")),
            ],
        );
        Ok(Self {
            id: ContextTagId::new(id)?,
            kind,
            status,
        })
    }

    pub fn active(id: u32, kind: ContextTagKind) -> Result<Self, ContextSkeletonError> {
        Self::new(id, kind, ContextTagStatus::Active)
    }

    pub const fn id(self) -> ContextTagId {
        self.id
    }

    pub const fn kind(self) -> ContextTagKind {
        self.kind
    }

    pub const fn status(self) -> ContextTagStatus {
        self.status
    }
}

pub fn dropped_tag_marker(id: ContextTagId) -> String {
    let _linkscope_marker = linkscope::phase("context.tag.dropped_marker");
    linkscope::detail_event_fields(
        "context.tag.dropped_marker",
        [linkscope::TraceField::count("id", u64::from(id.get()))],
    );
    format!("[dropped {id}]")
}
