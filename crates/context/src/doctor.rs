use crate::{ContextHealth, ContextHealthEventKind, ContextHealthStatus};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContextDoctorReport {
    context_health: ContextHealthDoctorSummary,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContextHealthDoctorSummary {
    status: ContextHealthStatus,
    revision: u64,
    contributors: usize,
    events: usize,
    latest_event: Option<ContextHealthDoctorEvent>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContextHealthDoctorEvent {
    kind: ContextHealthEventKind,
    cause: String,
}

impl ContextDoctorReport {
    pub fn from_health(health: &ContextHealth) -> Self {
        Self {
            context_health: ContextHealthDoctorSummary::from_health(health),
        }
    }

    pub fn context_health(&self) -> &ContextHealthDoctorSummary {
        &self.context_health
    }
}

impl ContextHealthDoctorSummary {
    fn from_health(health: &ContextHealth) -> Self {
        Self {
            status: health.status(),
            revision: health.revision(),
            contributors: health.contributors().len(),
            events: health.events().len(),
            latest_event: health
                .events()
                .last()
                .map(ContextHealthDoctorEvent::from_event),
        }
    }

    pub fn status(&self) -> ContextHealthStatus {
        self.status
    }

    pub fn revision(&self) -> u64 {
        self.revision
    }

    pub fn contributors(&self) -> usize {
        self.contributors
    }

    pub fn events(&self) -> usize {
        self.events
    }

    pub fn latest_event(&self) -> Option<&ContextHealthDoctorEvent> {
        self.latest_event.as_ref()
    }
}

impl ContextHealthDoctorEvent {
    fn from_event(event: &crate::ContextHealthEvent) -> Self {
        Self {
            kind: event.kind(),
            cause: event.cause().to_owned(),
        }
    }

    pub fn kind(&self) -> ContextHealthEventKind {
        self.kind
    }

    pub fn cause(&self) -> &str {
        &self.cause
    }
}
