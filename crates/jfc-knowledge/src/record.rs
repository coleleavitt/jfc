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

    /// Default salience by kind: durable, reusable knowledge (findings,
    /// conventions, skills, preferences) outranks one-off facts.
    pub fn default_importance(self) -> f64 {
        match self {
            Self::Finding | Self::Convention => 0.8,
            Self::Skill | Self::Preference => 0.7,
            Self::Fact => 0.5,
        }
    }
}

/// Typed edge between two knowledge records (Obsidian-style link-graph). Lets
/// recall traverse (a surfaced error pulls in its `FixedBy` lesson) and answers
/// backlink queries ("what depends on this lesson").
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RelKind {
    RelatesTo,
    Supersedes,
    CausedBy,
    FixedBy,
    Refines,
}

impl RelKind {
    pub fn slug(self) -> &'static str {
        match self {
            Self::RelatesTo => "relates-to",
            Self::Supersedes => "supersedes",
            Self::CausedBy => "caused-by",
            Self::FixedBy => "fixed-by",
            Self::Refines => "refines",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "relates-to" => Some(Self::RelatesTo),
            "supersedes" => Some(Self::Supersedes),
            "caused-by" => Some(Self::CausedBy),
            "fixed-by" => Some(Self::FixedBy),
            "refines" => Some(Self::Refines),
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

/// Whether a lesson's claim has been *verified*. The literature's #1 lever for a
/// compounding (vs. plateauing) self-improvement loop: an error-lesson is only
/// `Verified` when the evidence confirms the fix actually worked (e.g. a
/// failed→succeeded recovery in the same transcript). `Unverified` self-reports
/// rank far lower; `Refuted` lessons are contradicted by later evidence.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    #[default]
    Unverified,
    Verified,
    Refuted,
}

impl Outcome {
    pub fn slug(self) -> &'static str {
        match self {
            Self::Unverified => "unverified",
            Self::Verified => "verified",
            Self::Refuted => "refuted",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "unverified" => Some(Self::Unverified),
            "verified" => Some(Self::Verified),
            "refuted" => Some(Self::Refuted),
            _ => None,
        }
    }

    /// Ranking multiplier: verified lessons rank well above unverified ones on
    /// equal relevance; refuted lessons are suppressed.
    pub fn rank_boost(self) -> f64 {
        match self {
            Self::Verified => 2.0,
            Self::Unverified => 1.0,
            Self::Refuted => 0.1,
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
    /// Verification status (schema v2). Drives the verified rank boost.
    pub outcome: Outcome,
    /// Salience/importance in [0.0, 1.0] (schema v2). Generative-Agents-style:
    /// findings/conventions matter more than ephemeral facts. Multiplies rank.
    pub importance: f64,
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
            outcome: Outcome::Unverified,
            importance: kind.default_importance(),
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

    pub fn with_outcome(mut self, outcome: Outcome) -> Self {
        self.outcome = outcome;
        self
    }

    pub fn with_importance(mut self, importance: f64) -> Self {
        self.importance = importance.clamp(0.0, 1.0);
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
