use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::{Task, TaskRisk};

pub const TASK_EXECUTION_METADATA_KEY: &str = "execution";
pub const LEGACY_MARKET_METADATA_KEY: &str = "market";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskExecutionMode {
    Solo,
    Assisted,
    Bounty,
}

impl TaskExecutionMode {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Solo => "solo",
            Self::Assisted => "assisted",
            Self::Bounty => "bounty",
        }
    }

    fn from_label(value: &str) -> Option<Self> {
        match value {
            "solo" => Some(Self::Solo),
            "assisted" => Some(Self::Assisted),
            "bounty" => Some(Self::Bounty),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskBountyRef {
    pub bounty_id: String,
    pub budget: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_solvers: Option<u8>,
    pub auto_dispatch: bool,
}

impl TaskBountyRef {
    pub fn new(
        bounty_id: impl Into<String>,
        budget: u64,
        max_solvers: Option<u8>,
        auto_dispatch: bool,
    ) -> Self {
        Self {
            bounty_id: bounty_id.into(),
            budget,
            max_solvers,
            auto_dispatch,
        }
    }

    fn from_value(value: &Value) -> Option<Self> {
        let bounty_id = value.get("bounty_id")?.as_str()?.to_owned();
        let budget = value.get("budget").and_then(Value::as_u64).unwrap_or(0);
        let max_solvers = value
            .get("max_solvers")
            .and_then(Value::as_u64)
            .and_then(|value| u8::try_from(value).ok());
        let auto_dispatch = value
            .get("auto_dispatch")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        Some(Self {
            bounty_id,
            budget,
            max_solvers,
            auto_dispatch,
        })
    }

    fn to_value(&self) -> Value {
        let mut out = Map::new();
        out.insert(
            "bounty_id".to_owned(),
            Value::String(self.bounty_id.clone()),
        );
        out.insert(
            "budget".to_owned(),
            Value::Number(serde_json::Number::from(self.budget)),
        );
        if let Some(max_solvers) = self.max_solvers {
            out.insert(
                "max_solvers".to_owned(),
                Value::Number(serde_json::Number::from(max_solvers)),
            );
        } else {
            out.insert("max_solvers".to_owned(), Value::Null);
        }
        out.insert("auto_dispatch".to_owned(), Value::Bool(self.auto_dispatch));
        Value::Object(out)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskExecutionMetadata {
    pub mode: TaskExecutionMode,
    pub reason: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bounty: Option<TaskBountyRef>,
}

impl TaskExecutionMetadata {
    pub fn solo(reason: impl Into<String>) -> Self {
        Self {
            mode: TaskExecutionMode::Solo,
            reason: reason.into(),
            bounty: None,
        }
    }

    pub fn assisted(reason: impl Into<String>) -> Self {
        Self {
            mode: TaskExecutionMode::Assisted,
            reason: reason.into(),
            bounty: None,
        }
    }

    pub fn bounty(reason: impl Into<String>, bounty: Option<TaskBountyRef>) -> Self {
        Self {
            mode: TaskExecutionMode::Bounty,
            reason: reason.into(),
            bounty,
        }
    }

    pub fn recommended_for_fields(
        subject: &str,
        description: &str,
        risk: Option<TaskRisk>,
        tags: &[String],
        attempt_count: u32,
    ) -> Self {
        if tags.iter().any(|tag| tag == "bounty" || tag == "market") {
            return Self::bounty("task is already linked to the bounty market", None);
        }
        if attempt_count > 0 {
            return Self::bounty("task already needed a retry", None);
        }
        if matches!(risk, Some(TaskRisk::High)) {
            return Self::bounty(
                "high-risk task should use solver/validator competition",
                None,
            );
        }
        if contains_any_signal(subject, description, tags, BOUNTY_SIGNALS) {
            return Self::bounty(
                "task has security, migration, or correctness-risk signals",
                None,
            );
        }
        if matches!(risk, Some(TaskRisk::Medium))
            || contains_any_signal(subject, description, tags, ASSISTED_SIGNALS)
        {
            return Self::assisted("task may benefit from a second focused agent");
        }
        Self::solo("default execution for ordinary scoped work")
    }

    pub fn from_task(task: &Task) -> Option<Self> {
        Self::from_metadata(task.metadata.as_ref())
    }

    pub fn from_metadata(metadata: Option<&Value>) -> Option<Self> {
        let metadata = metadata?;
        if let Some(execution) = metadata.get(TASK_EXECUTION_METADATA_KEY)
            && let Some(parsed) = Self::from_execution_value(execution)
        {
            return Some(parsed);
        }
        metadata
            .get(LEGACY_MARKET_METADATA_KEY)
            .and_then(Self::from_legacy_market_value)
    }

    pub fn to_task_metadata(&self, existing: Option<&Value>) -> Value {
        let mut root = existing
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();
        root.insert(TASK_EXECUTION_METADATA_KEY.to_owned(), self.to_value());
        if let Some(bounty) = &self.bounty {
            let mut legacy = bounty.to_value();
            if let Value::Object(ref mut object) = legacy {
                object.insert("kind".to_owned(), Value::String("bounty".to_owned()));
            }
            root.insert(LEGACY_MARKET_METADATA_KEY.to_owned(), legacy);
        }
        Value::Object(root)
    }

    fn from_execution_value(value: &Value) -> Option<Self> {
        let mode = value
            .get("mode")
            .and_then(Value::as_str)
            .and_then(TaskExecutionMode::from_label)?;
        let reason = value
            .get("reason")
            .and_then(Value::as_str)
            .unwrap_or("stored execution strategy")
            .to_owned();
        let bounty = value.get("bounty").and_then(TaskBountyRef::from_value);
        Some(Self {
            mode,
            reason,
            bounty,
        })
    }

    fn from_legacy_market_value(value: &Value) -> Option<Self> {
        if value.get("kind").and_then(Value::as_str) != Some("bounty") {
            return None;
        }
        Some(Self::bounty(
            "legacy market metadata links this task to a bounty",
            TaskBountyRef::from_value(value),
        ))
    }

    fn to_value(&self) -> Value {
        let mut out = Map::new();
        out.insert(
            "mode".to_owned(),
            Value::String(self.mode.label().to_owned()),
        );
        out.insert("reason".to_owned(), Value::String(self.reason.clone()));
        if let Some(bounty) = &self.bounty {
            out.insert("bounty".to_owned(), bounty.to_value());
        }
        Value::Object(out)
    }
}

const BOUNTY_SIGNALS: &[&str] = &[
    "audit",
    "bounty",
    "critical",
    "exploit",
    "migration",
    "race",
    "security",
    "unsafe",
    "vulnerability",
];

const ASSISTED_SIGNALS: &[&str] = &[
    "architecture",
    "design",
    "plan",
    "refactor",
    "research",
    "review",
];

fn contains_any_signal(
    subject: &str,
    description: &str,
    tags: &[String],
    signals: &[&str],
) -> bool {
    let subject = subject.to_ascii_lowercase();
    let description = description.to_ascii_lowercase();
    signals.iter().any(|signal| {
        subject.contains(signal)
            || description.contains(signal)
            || tags.iter().any(|tag| tag.eq_ignore_ascii_case(signal))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn high_risk_task_recommends_bounty_normal() {
        let metadata = TaskExecutionMetadata::recommended_for_fields(
            "fix verifier",
            "high impact correctness issue",
            Some(TaskRisk::High),
            &[],
            0,
        );

        assert_eq!(metadata.mode, TaskExecutionMode::Bounty);
        assert!(metadata.reason.contains("high-risk"));
    }

    #[test]
    fn ordinary_task_defaults_to_solo_normal() {
        let metadata = TaskExecutionMetadata::recommended_for_fields(
            "rename label",
            "small cleanup",
            None,
            &[],
            0,
        );

        assert_eq!(metadata.mode, TaskExecutionMode::Solo);
    }

    #[test]
    fn bounty_metadata_round_trips_and_preserves_legacy_market_normal() {
        let metadata = TaskExecutionMetadata::bounty(
            "posted bounty",
            Some(TaskBountyRef::new("bounty_1", 42, Some(3), true)),
        );
        let value = metadata.to_task_metadata(None);
        let parsed = TaskExecutionMetadata::from_metadata(Some(&value)).expect("metadata");

        assert_eq!(parsed.mode, TaskExecutionMode::Bounty);
        assert_eq!(
            parsed
                .bounty
                .as_ref()
                .map(|bounty| bounty.bounty_id.as_str()),
            Some("bounty_1")
        );
        assert_eq!(
            value
                .get("market")
                .and_then(|market| market.get("bounty_id"))
                .and_then(Value::as_str),
            Some("bounty_1")
        );
    }

    #[test]
    fn legacy_market_metadata_parses_as_bounty_normal() {
        let value = serde_json::json!({
            "market": {
                "kind": "bounty",
                "bounty_id": "bounty_legacy",
                "budget": 12,
                "max_solvers": 2,
                "auto_dispatch": false
            }
        });

        let parsed = TaskExecutionMetadata::from_metadata(Some(&value)).expect("legacy metadata");

        assert_eq!(parsed.mode, TaskExecutionMode::Bounty);
        assert_eq!(
            parsed
                .bounty
                .as_ref()
                .map(|bounty| bounty.bounty_id.as_str()),
            Some("bounty_legacy")
        );
    }
}
