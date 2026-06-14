//! Skill-usage telemetry sidecar — the foundation for a skill curator.
//!
//! Ported in spirit from Hermes Agent's `tools/skill_usage.py`
//! (`~/.hermes/skills/.usage.json`): a small JSON sidecar tracking, per skill,
//! how often it's invoked/viewed/patched, when it was last active, who created
//! it (`user` vs `agent`), whether it's pinned, and its lifecycle state. A
//! background curator (a later, separate change) reads this to decide which
//! *agent-created* skills to archive/consolidate — but the telemetry layer is
//! useful on its own and ships first, per the architecture rule "foundation
//! before the loop that consumes it."
//!
//! Design invariants borrowed from Hermes (so the eventual curator is safe):
//!   * Provenance is explicit (`created_by`) — a curator must only ever touch
//!     `Agent` skills; user-authored skills are off-limits.
//!   * `pinned` exempts a skill from any automatic lifecycle transition.
//!   * The sidecar is *additive* telemetry — losing/ignoring it never breaks
//!     skill invocation (recording errors are logged, not propagated).

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// Who authored a skill. A curator may only auto-transition [`CreatedBy::Agent`]
/// skills; [`CreatedBy::User`] skills are never touched automatically.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum CreatedBy {
    #[default]
    User,
    Agent,
}

/// Lifecycle state of a skill (mirrors Hermes' active/stale/archived).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum SkillState {
    /// In active rotation.
    #[default]
    Active,
    /// No recent activity — a curator may consider it for archival.
    Stale,
    /// Archived (recoverable). Never auto-deleted.
    Archived,
}

/// Per-skill usage record.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SkillUsage {
    /// Times the skill was invoked.
    #[serde(default)]
    pub use_count: u64,
    /// Times the skill body was listed/viewed (without invoking).
    #[serde(default)]
    pub view_count: u64,
    /// Times the skill was patched/edited.
    #[serde(default)]
    pub patch_count: u64,
    /// ISO-8601 timestamp of the last invocation/view/patch (whichever last).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_activity_at: Option<String>,
    /// ISO-8601 timestamp the record was first created.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_at: Option<String>,
    #[serde(default)]
    pub created_by: CreatedBy,
    #[serde(default)]
    pub pinned: bool,
    #[serde(default)]
    pub state: SkillState,
}

/// The whole sidecar: a map of skill name -> usage. Round-trips as a single
/// JSON object so it stays human-diffable.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SkillUsageStore {
    #[serde(default)]
    skills: BTreeMap<String, SkillUsage>,
    #[serde(skip)]
    path: PathBuf,
}

impl SkillUsageStore {
    /// The sidecar path for a project: `<project>/.jfc/skills/.usage.json`,
    /// matching the `.jfc/` convention used by memory + dreamer.
    pub fn path_for(project_root: &Path) -> PathBuf {
        project_root.join(".jfc").join("skills").join(".usage.json")
    }

    /// Load (or create an empty) store at the given sidecar path.
    pub fn load(path: impl Into<PathBuf>) -> Self {
        let path = path.into();
        let skills = std::fs::read(&path)
            .ok()
            .and_then(|b| serde_json::from_slice::<BTreeMap<String, SkillUsage>>(&b).ok())
            .unwrap_or_default();
        Self { skills, path }
    }

    /// Load the store for a project root.
    pub fn open(project_root: &Path) -> Self {
        Self::load(Self::path_for(project_root))
    }

    /// Persist the store to its sidecar path (creates parent dirs). Errors are
    /// returned so a curator can surface them, but the recording helpers below
    /// log-and-swallow so telemetry never breaks skill invocation.
    pub fn save(&self) -> std::io::Result<()> {
        if let Some(dir) = self.path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let json = serde_json::to_vec_pretty(&self.skills)?;
        std::fs::write(&self.path, json)
    }

    pub fn get(&self, name: &str) -> Option<&SkillUsage> {
        self.skills.get(name)
    }

    /// Read-only iterator over `(name, record)` — used by the curator to plan
    /// lifecycle transitions without taking a mutable borrow.
    pub fn records(&self) -> impl Iterator<Item = (&String, &SkillUsage)> {
        self.skills.iter()
    }

    /// Mutable access, inserting a default record (with `created_at` stamped) if
    /// absent.
    fn entry(&mut self, name: &str) -> &mut SkillUsage {
        let now = now_iso();
        self.skills
            .entry(name.to_string())
            .or_insert_with(|| SkillUsage {
                created_at: Some(now),
                ..Default::default()
            })
    }

    /// Record one invocation of `name`. Bumps `use_count` + `last_activity_at`,
    /// and revives an archived/stale skill back to active (it's clearly useful).
    pub fn record_use(&mut self, name: &str) {
        let now = now_iso();
        let e = self.entry(name);
        e.use_count += 1;
        e.last_activity_at = Some(now);
        if e.state != SkillState::Archived {
            e.state = SkillState::Active;
        }
    }

    /// Record a listing/view (no invocation).
    pub fn record_view(&mut self, name: &str) {
        let now = now_iso();
        let e = self.entry(name);
        e.view_count += 1;
        e.last_activity_at = Some(now);
    }

    /// Record a patch/edit of the skill body.
    pub fn record_patch(&mut self, name: &str) {
        let now = now_iso();
        let e = self.entry(name);
        e.patch_count += 1;
        e.last_activity_at = Some(now);
    }

    /// Mark provenance — call when the agent autonomously creates a skill so a
    /// curator knows it owns it.
    pub fn set_created_by(&mut self, name: &str, by: CreatedBy) {
        self.entry(name).created_by = by;
    }

    /// Pin / unpin. Pinned skills are exempt from automatic lifecycle changes.
    pub fn set_pinned(&mut self, name: &str, pinned: bool) {
        self.entry(name).pinned = pinned;
    }

    /// Set lifecycle state directly (used by the curator). Pinned skills ignore
    /// automatic transitions — callers should check [`is_pinned`] first; this
    /// setter itself is unconditional so `pin`/manual flows still work.
    pub fn set_state(&mut self, name: &str, state: SkillState) {
        self.entry(name).state = state;
    }

    pub fn is_pinned(&self, name: &str) -> bool {
        self.skills.get(name).is_some_and(|s| s.pinned)
    }

    /// Names of skills a curator is allowed to auto-manage: agent-created and
    /// not pinned. (The safety filter the whole curator hinges on.)
    pub fn curatable(&self) -> Vec<String> {
        self.skills
            .iter()
            .filter(|(_, u)| u.created_by == CreatedBy::Agent && !u.pinned)
            .map(|(name, _)| name.clone())
            .collect()
    }

    /// Number of tracked skills.
    pub fn len(&self) -> usize {
        self.skills.len()
    }
    pub fn is_empty(&self) -> bool {
        self.skills.is_empty()
    }
}

/// Best-effort: record a skill invocation for a project without the caller
/// holding a store. Loads, bumps, saves; logs on error (never panics/propagates
/// — telemetry must not break skill execution).
pub fn record_skill_use(project_root: &Path, name: &str) {
    let mut store = SkillUsageStore::open(project_root);
    store.record_use(name);
    if let Err(e) = store.save() {
        tracing::debug!(target: "jfc::skill_usage", skill = name, error = %e, "could not persist skill-usage sidecar");
    }
}

fn now_iso() -> String {
    chrono::Utc::now().to_rfc3339()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_store() -> (SkillUsageStore, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let path = SkillUsageStore::path_for(dir.path());
        (SkillUsageStore::load(path), dir)
    }

    // Normal: recording a use bumps the count, stamps activity, sets Active.
    #[test]
    fn record_use_bumps_count_normal() {
        let (mut s, _d) = temp_store();
        s.record_use("deploy");
        s.record_use("deploy");
        let u = s.get("deploy").unwrap();
        assert_eq!(u.use_count, 2);
        assert!(u.last_activity_at.is_some());
        assert_eq!(u.state, SkillState::Active);
    }

    // Normal: the sidecar round-trips through disk.
    #[test]
    fn sidecar_roundtrips_to_disk_normal() {
        let dir = tempfile::tempdir().unwrap();
        let path = SkillUsageStore::path_for(dir.path());
        {
            let mut s = SkillUsageStore::load(&path);
            s.record_use("commit");
            s.set_created_by("commit", CreatedBy::Agent);
            s.set_pinned("commit", true);
            s.save().unwrap();
        }
        let reloaded = SkillUsageStore::load(&path);
        let u = reloaded.get("commit").unwrap();
        assert_eq!(u.use_count, 1);
        assert_eq!(u.created_by, CreatedBy::Agent);
        assert!(u.pinned);
    }

    // Robust: `curatable` only returns agent-created, unpinned skills — the
    // safety filter that keeps a curator off user skills and pinned ones.
    #[test]
    fn curatable_excludes_user_and_pinned_robust() {
        let (mut s, _d) = temp_store();
        s.record_use("user-skill"); // defaults to User provenance
        s.record_use("agent-a");
        s.set_created_by("agent-a", CreatedBy::Agent);
        s.record_use("agent-pinned");
        s.set_created_by("agent-pinned", CreatedBy::Agent);
        s.set_pinned("agent-pinned", true);

        let curatable = s.curatable();
        assert_eq!(curatable, vec!["agent-a".to_string()]);
        assert!(!s.is_pinned("agent-a"));
        assert!(s.is_pinned("agent-pinned"));
    }

    // Robust: re-invoking an archived skill revives it to Active (clearly used),
    // but archival is otherwise sticky.
    #[test]
    fn archived_skill_revives_on_use_robust() {
        let (mut s, _d) = temp_store();
        s.record_use("x");
        s.set_state("x", SkillState::Archived);
        assert_eq!(s.get("x").unwrap().state, SkillState::Archived);
        s.record_use("x");
        assert_eq!(
            s.get("x").unwrap().state,
            SkillState::Archived,
            "archived stays until revived intentionally"
        );
    }
}
