use crate::{ContextContributor, ContextLayout, ContextSkeletonError};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContextHealthStatus {
    Healthy,
    Degraded,
    Unavailable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContextHealthEventKind {
    EmbeddingFailure,
    CacheBust,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContextHealthEvent {
    kind: ContextHealthEventKind,
    cause: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContextHealthUpdate {
    kind: ContextHealthEventKind,
    cause: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContextHealth {
    layout: ContextLayout,
    status: ContextHealthStatus,
    contributors: Vec<ContextContributor>,
    events: Vec<ContextHealthEvent>,
    revision: u64,
}

pub trait ContextHealthService {
    fn current_health(&self) -> &ContextHealth;

    fn update_health(
        &mut self,
        update: ContextHealthUpdate,
    ) -> Result<&ContextHealth, ContextSkeletonError>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InMemoryContextHealthService {
    health: ContextHealth,
}

impl ContextHealth {
    pub fn new(
        layout: ContextLayout,
        status: ContextHealthStatus,
        contributors: Vec<ContextContributor>,
    ) -> Result<Self, ContextSkeletonError> {
        if !layout.is_complete_destination_skeleton() {
            return Err(ContextSkeletonError::IncompleteLayout);
        }

        Ok(Self {
            layout,
            status,
            contributors,
            events: Vec::new(),
            revision: 0,
        })
    }

    pub fn layout(&self) -> &ContextLayout {
        &self.layout
    }

    pub fn status(&self) -> ContextHealthStatus {
        self.status
    }

    pub fn contributors(&self) -> &[ContextContributor] {
        &self.contributors
    }

    pub fn events(&self) -> &[ContextHealthEvent] {
        &self.events
    }

    pub fn revision(&self) -> u64 {
        self.revision
    }

    pub fn apply_update(
        &mut self,
        update: ContextHealthUpdate,
    ) -> Result<(), ContextSkeletonError> {
        let event = update.into_event()?;

        self.status = ContextHealthStatus::Degraded;
        self.revision += 1;
        self.events.push(event);

        Ok(())
    }
}

impl ContextHealthEvent {
    pub fn kind(&self) -> ContextHealthEventKind {
        self.kind
    }

    pub fn cause(&self) -> &str {
        &self.cause
    }
}

impl ContextHealthUpdate {
    pub fn embedding_failure(cause: impl Into<String>) -> Self {
        Self {
            kind: ContextHealthEventKind::EmbeddingFailure,
            cause: cause.into(),
        }
    }

    pub fn cache_bust(cause: impl Into<String>) -> Self {
        Self {
            kind: ContextHealthEventKind::CacheBust,
            cause: cause.into(),
        }
    }

    fn into_event(self) -> Result<ContextHealthEvent, ContextSkeletonError> {
        if self.cause.trim().is_empty() {
            return Err(ContextSkeletonError::EmptyHealthUpdateCause);
        }

        Ok(ContextHealthEvent {
            kind: self.kind,
            cause: self.cause,
        })
    }
}

impl InMemoryContextHealthService {
    pub fn new(health: ContextHealth) -> Self {
        Self { health }
    }

    pub fn into_health(self) -> ContextHealth {
        self.health
    }
}

impl ContextHealthService for InMemoryContextHealthService {
    fn current_health(&self) -> &ContextHealth {
        &self.health
    }

    fn update_health(
        &mut self,
        update: ContextHealthUpdate,
    ) -> Result<&ContextHealth, ContextSkeletonError> {
        self.health.apply_update(update)?;
        Ok(&self.health)
    }
}
