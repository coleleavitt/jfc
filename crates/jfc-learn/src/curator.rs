//! Skill curator — lifecycle maintenance for agent-created skills.
//!
//! Built on the [`crate::skill_usage`] telemetry sidecar. Mirrors Hermes Agent's
//! `agent/curator.py`: a periodic, *non-destructive* maintenance pass that moves
//! agent-created skills through `Active → Stale → Archived` based on how long
//! they've been idle. Archive is the strongest action (recoverable); there is no
//! delete. The decision logic here is **pure** (no I/O, no model call) so it's
//! fully unit-testable — a caller loads the [`SkillUsageStore`], runs
//! [`plan_transitions`], applies the plan, and saves.
//!
//! Invariants (enforced by [`SkillUsage::created_by`] + `pinned`):
//!   * Only `CreatedBy::Agent` skills are ever transitioned.
//!   * Pinned skills are exempt from every transition.
//!   * Transitions only ever advance toward archival on *inactivity*; any use
//!     revives a skill (handled in `record_use`), so the curator never fights an
//!     actively-used skill.

use crate::skill_usage::{CreatedBy, SkillState, SkillUsage, SkillUsageStore};

/// Curator thresholds. Defaults mirror Hermes (`stale_after_days: 14`,
/// `archive_after_days: 30`).
#[derive(Debug, Clone, Copy)]
pub struct CuratorConfig {
    /// Idle days after which an Active agent-skill becomes Stale.
    pub stale_after_days: u64,
    /// Idle days after which a Stale agent-skill becomes Archived.
    pub archive_after_days: u64,
}

impl Default for CuratorConfig {
    fn default() -> Self {
        Self {
            stale_after_days: 14,
            archive_after_days: 30,
        }
    }
}

/// A single planned lifecycle transition.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillTransition {
    pub skill: String,
    pub from: SkillState,
    pub to: SkillState,
}

/// The outcome of a curator planning pass.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CuratorPlan {
    pub transitions: Vec<SkillTransition>,
}

impl CuratorPlan {
    pub fn is_empty(&self) -> bool {
        self.transitions.is_empty()
    }
    pub fn len(&self) -> usize {
        self.transitions.len()
    }
}

/// Compute idle days for a record relative to `now_ms` epoch millis. A record
/// with no `last_activity_at` falls back to `created_at`; if neither parses, it
/// is treated as freshly-active (0 idle) so we never archive on a parse failure.
fn idle_days(rec: &SkillUsage, now_ms: i64) -> u64 {
    let anchor = rec
        .last_activity_at
        .as_deref()
        .or(rec.created_at.as_deref());
    let Some(ts) = anchor else {
        return 0;
    };
    let Ok(parsed) = chrono::DateTime::parse_from_rfc3339(ts) else {
        return 0;
    };
    let then_ms = parsed.timestamp_millis();
    if now_ms <= then_ms {
        return 0;
    }
    ((now_ms - then_ms) / 86_400_000) as u64
}

/// Plan (but do not apply) lifecycle transitions for the store, given `now_ms`
/// (epoch millis — injectable for tests). Pure: returns the transitions; the
/// caller applies + persists them. Only agent-created, non-pinned skills are
/// considered; archival is terminal (no Archived→anything here — revival happens
/// only on real use).
pub fn plan_transitions(store: &SkillUsageStore, cfg: &CuratorConfig, now_ms: i64) -> CuratorPlan {
    let mut transitions = Vec::new();
    for (name, rec) in store.records() {
        // Safety filter: only the curator's own domain.
        if rec.created_by != CreatedBy::Agent || rec.pinned {
            continue;
        }
        let idle = idle_days(rec, now_ms);
        let next = match rec.state {
            SkillState::Active if idle >= cfg.archive_after_days => Some(SkillState::Archived),
            SkillState::Active if idle >= cfg.stale_after_days => Some(SkillState::Stale),
            SkillState::Stale if idle >= cfg.archive_after_days => Some(SkillState::Archived),
            // Already archived, or not idle enough — no change.
            _ => None,
        };
        if let Some(to) = next {
            transitions.push(SkillTransition {
                skill: name.clone(),
                from: rec.state,
                to,
            });
        }
    }
    CuratorPlan { transitions }
}

/// Apply a [`CuratorPlan`] to the store in memory (does not save). Returns the
/// number of transitions applied. Uses `set_state`, which is unconditional, so
/// the plan's safety filtering (done in [`plan_transitions`]) is authoritative.
pub fn apply_plan(store: &mut SkillUsageStore, plan: &CuratorPlan) -> usize {
    for t in &plan.transitions {
        store.set_state(&t.skill, t.to);
    }
    plan.transitions.len()
}

/// Convenience: plan + apply against the current wall clock. Returns the plan
/// that was applied (so the caller can log/report it). Does not persist — the
/// caller decides when to `store.save()`.
pub fn run(store: &mut SkillUsageStore, cfg: &CuratorConfig) -> CuratorPlan {
    let now_ms = chrono::Utc::now().timestamp_millis();
    let plan = plan_transitions(store, cfg, now_ms);
    apply_plan(store, &plan);
    plan
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Duration, Utc};
    use std::collections::BTreeMap;

    /// Build a store with one record backdated by `idle_days`. The on-disk
    /// format is a bare `name -> SkillUsage` map, so we construct that JSON
    /// directly and load it — exercising the real deserialize path.
    fn store_with(name: &str, by: CreatedBy, idle_days: i64, pinned: bool) -> SkillUsageStore {
        let ts = (Utc::now() - Duration::days(idle_days)).to_rfc3339();
        let rec = SkillUsage {
            use_count: 1,
            last_activity_at: Some(ts.clone()),
            created_at: Some(ts),
            created_by: by,
            pinned,
            state: SkillState::Active,
            ..Default::default()
        };
        let map: BTreeMap<String, SkillUsage> = [(name.to_string(), rec)].into_iter().collect();

        let dir = std::env::temp_dir().join(format!(
            "jfc_curator_{}",
            Utc::now().timestamp_nanos_opt().unwrap_or(0)
        ));
        let path = SkillUsageStore::path_for(&dir);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, serde_json::to_vec(&map).unwrap()).unwrap();
        let store = SkillUsageStore::load(&path);
        std::fs::remove_dir_all(&dir).ok();
        store
    }

    // Normal: an agent skill idle past stale_after but under archive_after → Stale.
    #[test]
    fn agent_skill_goes_stale_normal() {
        let store = store_with("s", CreatedBy::Agent, 20, false);
        let plan = plan_transitions(
            &store,
            &CuratorConfig::default(),
            Utc::now().timestamp_millis(),
        );
        assert_eq!(plan.len(), 1);
        assert_eq!(plan.transitions[0].to, SkillState::Stale);
    }

    // Normal: idle past archive_after → Archived (even from Active).
    #[test]
    fn agent_skill_archives_normal() {
        let store = store_with("s", CreatedBy::Agent, 45, false);
        let plan = plan_transitions(
            &store,
            &CuratorConfig::default(),
            Utc::now().timestamp_millis(),
        );
        assert_eq!(plan.len(), 1);
        assert_eq!(plan.transitions[0].to, SkillState::Archived);
    }

    // Robust: user-created skills are NEVER transitioned, however idle.
    #[test]
    fn user_skill_never_transitions_robust() {
        let store = store_with("s", CreatedBy::User, 365, false);
        let plan = plan_transitions(
            &store,
            &CuratorConfig::default(),
            Utc::now().timestamp_millis(),
        );
        assert!(plan.is_empty(), "user skills are off-limits");
    }

    // Robust: pinned agent skills are exempt.
    #[test]
    fn pinned_skill_is_exempt_robust() {
        let store = store_with("s", CreatedBy::Agent, 365, true);
        let plan = plan_transitions(
            &store,
            &CuratorConfig::default(),
            Utc::now().timestamp_millis(),
        );
        assert!(plan.is_empty(), "pinned skills are exempt");
    }

    // Normal: a fresh agent skill (recently used) is left Active.
    #[test]
    fn fresh_skill_stays_active_normal() {
        let store = store_with("s", CreatedBy::Agent, 1, false);
        let plan = plan_transitions(
            &store,
            &CuratorConfig::default(),
            Utc::now().timestamp_millis(),
        );
        assert!(plan.is_empty());
    }

    // Normal: apply_plan mutates the store's states.
    #[test]
    fn apply_plan_mutates_state_normal() {
        let mut store = store_with("s", CreatedBy::Agent, 45, false);
        let plan = plan_transitions(
            &store,
            &CuratorConfig::default(),
            Utc::now().timestamp_millis(),
        );
        let n = apply_plan(&mut store, &plan);
        assert_eq!(n, 1);
        assert_eq!(store.get("s").unwrap().state, SkillState::Archived);
    }
}
