//! The knowledge record and its enums.
//!
//! A [`KnowledgeRecord`] is one durable, cross-project lesson: a fact,
//! preference, induced skill, verification finding, or convention. Records are
//! *immutable* — to revise one, insert a replacement and mark the old row
//! superseded (see [`crate::query`]). This mirrors the immutable `.md` memory
//! model in `jfc-memory`, but in a queryable store.

use serde::{Deserialize, Serialize};

/// What kind of knowledge a record holds. Stored as a lowercase slug.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    /// A durable fact about a project, tool, or environment.
    Fact,
    /// A user preference (style, workflow, tone).
    Preference,
    /// An induced skill / repeatable procedure.
    Skill,
    /// A verification finding (a repeatable failure or blind spot).
    Finding,
    /// A project/code convention worth applying consistently.
    Convention,
}

impl Kind {
    pub fn slug(self) -> &'static str {
        match self {
            Self::Fact => "fact",
            Self::Preference => "preference",
            Self::Skill => "skill",
            Self::Finding => "finding",
            Self::Convention => "convention",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "fact" => Some(Self::Fact),
            "preference" => Some(Self::Preference),
            "skill" => Some(Self::Skill),
            "finding" => Some(Self::Finding),
            "convention" => Some(Self::Convention),
            _ => None,
        }
    }
}

/// The visibility scope of a record. This is the core safety axis: only
/// [`Scope::Global`] records leak across projects, and a record reaches
/// `Global` **only via explicit human promotion** (never autonomously at
/// runtime). See `PLAN.md` §2.4.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Scope {
    /// Personal, follows the user across projects (preferences).
    User,
    /// Scoped to one project (keyed by `project_key`). The default.
    Project,
    /// Promoted, applies to every project. Human-gated only.
    Global,
}

impl Scope {
    pub fn slug(self) -> &'static str {
        match self {
            Self::User => "user",
            Self::Project => "project",
            Self::Global => "global",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "user" => Some(Self::User),
            "project" => Some(Self::Project),
            "global" => Some(Self::Global),
            _ => None,
        }
    }
}

/// One knowledge record. Field semantics match the `knowledge` table in
/// [`crate::schema`].
#[derive(Debug, Clone, PartialEq)]
pub struct KnowledgeRecord {
    pub id: String,
    pub kind: Kind,
    pub scope: Scope,
    /// `None` for `Global`/`User`; the stable project id for `Project`.
    pub project_key: Option<String>,
    pub title: String,
    pub body: String,
    /// Comma-separated tags (also FTS-indexed).
    pub tags: String,
    pub source: Option<String>,
    /// Confidence in [0.0, 1.0].
    pub confidence: f64,
    pub created_at_ms: i64,
    pub last_used_ms: Option<i64>,
    pub use_count: i64,
    /// `Some(id)` of the record that replaced this one; `None` if live.
    pub superseded_by: Option<String>,
    /// True only after explicit human promotion to global scope.
    pub promoted: bool,
}

impl KnowledgeRecord {
    /// Build a fresh, live record with a generated id and `created_at` = now.
    /// `confidence` is clamped to [0.0, 1.0].
    pub fn new(
        kind: Kind,
        scope: Scope,
        project_key: Option<String>,
        title: impl Into<String>,
        body: impl Into<String>,
    ) -> Self {
        Self {
            id: uuid::Uuid::new_v4().simple().to_string(),
            kind,
            scope,
            project_key,
            title: title.into(),
            body: body.into(),
            tags: String::new(),
            source: None,
            confidence: 0.5,
            created_at_ms: now_ms(),
            last_used_ms: None,
            use_count: 0,
            superseded_by: None,
            promoted: false,
        }
    }

    pub fn with_tags(mut self, tags: impl Into<String>) -> Self {
        self.tags = tags.into();
        self
    }

    pub fn with_source(mut self, source: impl Into<String>) -> Self {
        self.source = Some(source.into());
        self
    }

    pub fn with_confidence(mut self, confidence: f64) -> Self {
        self.confidence = confidence.clamp(0.0, 1.0);
        self
    }

    /// A record is live when nothing has superseded it.
    pub fn is_live(&self) -> bool {
        self.superseded_by.is_none()
    }
}

pub(crate) fn now_ms() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}
