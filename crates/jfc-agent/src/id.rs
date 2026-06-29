//! Unified agent identity.
//!
//! Replaces three pre-existing identifiers that all meant "an agent":
//! - `AgentId(u64)` in the engine's in-memory background manager,
//! - `AgentId(String)` in `jfc-economy` (prefixed, e.g. `solver-0`),
//! - a bare `agent_id: String` field on the swarm's teammate identity.
//!
//! A single [`AgentId`] newtype over a UUID is used everywhere. It carries an
//! optional human-readable `display_name` so the UI can show "researcher"
//! instead of a raw UUID while the stable identity stays collision-free.

use std::fmt;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Stable, process-and-disk portable identity for an agent.
///
/// The `uuid` field is the canonical key (used for equality, hashing, and map
/// lookups). `display_name` is presentation-only and never participates in
/// identity — two `AgentId`s with the same UUID but different names are the
/// same agent.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentId {
    uuid: Uuid,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    display_name: Option<String>,
}

impl AgentId {
    /// Mint a fresh random identity (UUID v4).
    pub fn new() -> Self {
        let _linkscope_id = linkscope::phase("agent.id.new");
        let id = Self {
            uuid: Uuid::new_v4(),
            display_name: None,
        };
        trace_id_shape("agent.id.new.detail", &id, "random", 0);
        id
    }

    /// Mint a fresh identity with a human-readable name attached.
    pub fn named(name: impl Into<String>) -> Self {
        let _linkscope_id = linkscope::phase("agent.id.named");
        let name = name.into();
        let name_bytes = name.len();
        let id = Self {
            uuid: Uuid::new_v4(),
            display_name: Some(name),
        };
        trace_id_shape("agent.id.named.detail", &id, "named", name_bytes);
        id
    }

    /// Reconstruct from an existing UUID (e.g. when loading persisted state).
    pub fn from_uuid(uuid: Uuid) -> Self {
        let _linkscope_id = linkscope::phase("agent.id.from_uuid");
        let id = Self {
            uuid,
            display_name: None,
        };
        trace_id_shape("agent.id.from_uuid.detail", &id, "uuid", 0);
        id
    }

    /// Derive a *stable* identity from a role prefix and an index.
    ///
    /// This replaces `jfc-economy`'s `AgentId::new_stable("solver", i)`: the
    /// same `(prefix, index)` pair always yields the same UUID (UUID v5 over a
    /// fixed namespace), so solver/validator identities are reproducible across
    /// a bounty cycle without a shared counter. The prefix is also kept as the
    /// display name.
    pub fn stable(prefix: &str, index: u64) -> Self {
        let _linkscope_id = linkscope::phase("agent.id.stable");
        let seed = format!("{prefix}-{index}");
        let seed_bytes = seed.len();
        let id = Self {
            uuid: Uuid::new_v5(&Uuid::NAMESPACE_OID, seed.as_bytes()),
            display_name: Some(seed),
        };
        trace_stable_id(prefix.len(), index);
        trace_id_shape("agent.id.stable.detail", &id, "stable", seed_bytes);
        id
    }

    /// Derive a *deterministic* identity from a single string label.
    ///
    /// Two `from_label` calls with the same string yield the same identity
    /// (UUID v5 over the label), and the label is retained as the display name.
    /// This is the bridge for systems that previously used the string itself as
    /// the identity key (e.g. the economy's `AgentId(String)`): a fixed label
    /// like `"solver_a"` keeps stable equality and hashing after the type swap.
    pub fn from_label(label: impl Into<String>) -> Self {
        let _linkscope_id = linkscope::phase("agent.id.from_label");
        let label = label.into();
        let label_bytes = label.len();
        let id = Self {
            uuid: Uuid::new_v5(&Uuid::NAMESPACE_OID, label.as_bytes()),
            display_name: Some(label),
        };
        trace_id_shape("agent.id.from_label.detail", &id, "label", label_bytes);
        id
    }

    /// The canonical UUID.
    pub fn uuid(&self) -> Uuid {
        self.uuid
    }

    /// The optional presentation name.
    pub fn display_name(&self) -> Option<&str> {
        self.display_name.as_deref()
    }

    /// A stable string label for this id: its display name, or the empty string
    /// if it was minted without one. Callers that need a guaranteed-non-empty
    /// key should construct ids via [`AgentId::named`], [`AgentId::stable`], or
    /// [`AgentId::from_label`], all of which set a display name.
    pub fn label(&self) -> &str {
        self.display_name.as_deref().unwrap_or_default()
    }

    /// Attach (or replace) the presentation name.
    pub fn with_name(mut self, name: impl Into<String>) -> Self {
        let _linkscope_id = linkscope::phase("agent.id.with_name");
        let name = name.into();
        let name_bytes = name.len();
        self.display_name = Some(name);
        trace_id_shape("agent.id.with_name.detail", &self, "with_name", name_bytes);
        self
    }
}

fn trace_id_shape(label: &'static str, id: &AgentId, source: &'static str, name_bytes: usize) {
    if !linkscope::trace_detail_enabled() {
        return;
    }
    linkscope::detail_event_fields(
        label,
        [
            linkscope::TraceField::text("source", source),
            linkscope::TraceField::text("uuid", id.uuid.to_string()),
            linkscope::TraceField::count("has_name", u64::from(id.display_name.is_some())),
            linkscope::TraceField::bytes("name_bytes", usize_to_u64_saturating(name_bytes)),
        ],
    );
}

fn trace_stable_id(prefix_bytes: usize, index: u64) {
    if !linkscope::trace_detail_enabled() {
        return;
    }
    linkscope::detail_event_fields(
        "agent.id.stable.seed",
        [
            linkscope::TraceField::bytes("prefix_bytes", usize_to_u64_saturating(prefix_bytes)),
            linkscope::TraceField::count("index", index),
        ],
    );
}

fn usize_to_u64_saturating(value: usize) -> u64 {
    u64::try_from(value).unwrap_or(u64::MAX)
}

impl Default for AgentId {
    fn default() -> Self {
        Self::new()
    }
}

/// Identity is the UUID only — names are presentation.
impl PartialEq for AgentId {
    fn eq(&self, other: &Self) -> bool {
        self.uuid == other.uuid
    }
}

impl Eq for AgentId {}

impl std::hash::Hash for AgentId {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.uuid.hash(state);
    }
}

impl PartialOrd for AgentId {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for AgentId {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.uuid.cmp(&other.uuid)
    }
}

impl fmt::Display for AgentId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.display_name {
            Some(name) => write!(f, "{name} ({})", self.uuid),
            None => write!(f, "{}", self.uuid),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_ids_are_unique_normal() {
        let a = AgentId::new();
        let b = AgentId::new();
        assert_ne!(a, b);
    }

    #[test]
    fn stable_ids_are_reproducible_normal() {
        let a = AgentId::stable("solver", 0);
        let b = AgentId::stable("solver", 0);
        assert_eq!(a, b);
        assert_eq!(a.display_name(), Some("solver-0"));
    }

    #[test]
    fn from_label_is_deterministic_normal() {
        let a = AgentId::from_label("solver_a");
        let b = AgentId::from_label("solver_a");
        assert_eq!(a, b);
        assert_eq!(a.label(), "solver_a");
        assert_ne!(a, AgentId::from_label("solver_b"));
    }

    #[test]
    fn label_falls_back_to_empty_robust() {
        // An id minted without a name has an empty label, not a panic.
        assert_eq!(AgentId::from_uuid(Uuid::new_v4()).label(), "");
    }

    #[test]
    fn stable_ids_differ_by_index_normal() {
        assert_ne!(AgentId::stable("solver", 0), AgentId::stable("solver", 1));
    }

    #[test]
    fn display_name_does_not_affect_identity_robust() {
        let uuid = Uuid::new_v4();
        let a = AgentId::from_uuid(uuid).with_name("alice");
        let b = AgentId::from_uuid(uuid).with_name("bob");
        // Same UUID → same agent, regardless of display name.
        assert_eq!(a, b);
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};
        let mut ha = DefaultHasher::new();
        let mut hb = DefaultHasher::new();
        a.hash(&mut ha);
        b.hash(&mut hb);
        assert_eq!(ha.finish(), hb.finish());
    }

    #[test]
    fn serde_roundtrip_normal() {
        let id = AgentId::named("researcher");
        let json = serde_json::to_string(&id).unwrap();
        let back: AgentId = serde_json::from_str(&json).unwrap();
        assert_eq!(id, back);
        assert_eq!(back.display_name(), Some("researcher"));
    }

    #[test]
    fn id_trace_records_shape_without_label_payload_normal() {
        linkscope::trace_detail_enable();
        let id = AgentId::from_label("private researcher label").with_name("private display name");
        assert_eq!(id.display_name(), Some("private display name"));

        let snapshot = linkscope::snapshot();
        let rendered = format!("{snapshot:?}");
        assert!(rendered.contains("agent.id.from_label.detail"));
        assert!(rendered.contains("agent.id.with_name.detail"));
        assert!(rendered.contains("name_bytes"));
        assert!(!rendered.contains("private researcher label"));
        assert!(!rendered.contains("private display name"));
    }
}
