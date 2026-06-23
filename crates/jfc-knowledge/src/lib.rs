//! `jfc-knowledge` — a durable, cross-project memory & learning store.
//!
//! This is the Phase 1 storage layer from `PLAN.md`: a single SQLite database at
//! `~/.local/share/jfc/knowledge.db` holding facts, preferences, induced skills,
//! verification findings, and conventions that accumulate **across every project**
//! the user works in — the bounded, scaffolding-level self-improvement flywheel.
//!
//! ## Safety boundary (load-bearing — see `PLAN.md` §3)
//!
//! - **Cross-project leakage is human-gated.** A record is only [`Scope::Global`]
//!   (visible to every project) after explicit [`KnowledgeStore::promote`]; the
//!   runtime never promotes autonomously.
//! - **Recall is advisory context, never an action.** [`KnowledgeStore::recall`]
//!   returns rows to fold into a prompt; nothing here executes anything.
//! - **Bounded growth.** [`KnowledgeStore::decay`] caps rows and prunes
//!   tombstones, so the store can't grow without bound.
//! - **Kill switch.** The entire store is one file; deleting it is a full reset.
//!
//! Phase 1 ships dormant: the crate compiles and is tested, but no other crate
//! reads it yet (that's Phase 2).

pub mod error;
pub mod project;
pub mod query;
pub mod record;
mod schema;

use std::path::{Path, PathBuf};

use rusqlite::Connection;

pub use error::{KnowledgeError, Result};
pub use project::project_key;
pub use query::RecallFilter;
pub use record::{Kind, KnowledgeRecord, Scope};

/// Default per-scope row cap used by [`KnowledgeStore::decay`].
pub const DEFAULT_MAX_ROWS_PER_SCOPE: i64 = 2_000;
/// Default tombstone age before a superseded row is hard-deleted (90 days).
pub const DEFAULT_MAX_AGE_MS: i64 = 90 * 24 * 3600 * 1000;

/// A handle to the knowledge database.
///
/// Holds an owned [`Connection`]. SQLite calls are synchronous; callers in the
/// async engine should wrap usage in `tokio::task::spawn_blocking`. WAL mode +
/// a busy timeout (set in [`schema::apply_pragmas`]) make concurrent JFC
/// processes safe.
pub struct KnowledgeStore {
    conn: Connection,
}

impl KnowledgeStore {
    /// Open (creating if needed) and migrate the store at the default path
    /// `~/.local/share/jfc/knowledge.db`.
    pub fn open_default() -> Result<Self> {
        let path = default_db_path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        Self::open(&path)
    }

    /// Open (creating if needed) and migrate the store at `path`.
    pub fn open(path: &Path) -> Result<Self> {
        let mut conn = Connection::open(path)?;
        schema::apply_pragmas(&conn)?;
        schema::migrate(&mut conn)?;
        Ok(Self { conn })
    }

    /// An in-memory store — for tests and ephemeral use.
    pub fn open_in_memory() -> Result<Self> {
        let mut conn = Connection::open_in_memory()?;
        schema::apply_pragmas(&conn)?;
        schema::migrate(&mut conn)?;
        Ok(Self { conn })
    }

    /// Insert a record (validated at the boundary).
    pub fn insert(&self, rec: &KnowledgeRecord) -> Result<()> {
        query::insert(&self.conn, rec)
    }

    /// Mark `old_id` superseded by `new_id` (immutable revision).
    pub fn supersede(&self, old_id: &str, new_id: &str) -> Result<()> {
        query::supersede(&self.conn, old_id, new_id)
    }

    /// **Human-gated** promotion of a record to global (cross-project) scope.
    /// Returns `true` if a live record was promoted. Call ONLY from an explicit
    /// user command / approved proposal — never from the runtime turn loop.
    pub fn promote(&self, id: &str) -> Result<bool> {
        query::promote_to_global(&self.conn, id)
    }

    /// Recall advisory context for `query` (lexical FTS). Eligible rows are
    /// user + global + this-project. Does not bump usage — call [`Self::mark_used`]
    /// on the records you actually surface.
    pub fn recall(&self, query: &str, filter: &RecallFilter<'_>) -> Result<Vec<KnowledgeRecord>> {
        query::recall(&self.conn, query, filter)
    }

    /// Bump usage metrics for records that were surfaced.
    pub fn mark_used(&self, ids: &[String]) -> Result<()> {
        query::mark_used(&self.conn, ids)
    }

    /// Bounded-growth maintenance. Returns the number of rows removed.
    pub fn decay(&mut self, max_age_ms: i64, max_rows_per_scope: i64) -> Result<usize> {
        query::decay(&mut self.conn, max_age_ms, max_rows_per_scope)
    }

    /// Count of live (non-superseded) records — for tests/metrics.
    pub fn live_count(&self) -> Result<i64> {
        Ok(self.conn.query_row(
            "SELECT count(*) FROM knowledge WHERE superseded_by IS NULL",
            [],
            |r| r.get(0),
        )?)
    }
}

/// `~/.local/share/jfc/knowledge.db`, honoring `JFC_KNOWLEDGE_DB` and
/// `XDG_DATA_HOME`.
pub fn default_db_path() -> PathBuf {
    if let Some(p) = std::env::var_os("JFC_KNOWLEDGE_DB") {
        return PathBuf::from(p);
    }
    let base = dirs::data_dir().unwrap_or_else(|| {
        std::env::var_os("HOME")
            .map(|h| PathBuf::from(h).join(".local/share"))
            .unwrap_or_else(|| PathBuf::from("."))
    });
    base.join("jfc").join("knowledge.db")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec(scope: Scope, project: Option<&str>, title: &str, body: &str) -> KnowledgeRecord {
        KnowledgeRecord::new(
            Kind::Fact,
            scope,
            project.map(str::to_owned),
            title,
            body,
        )
    }

    #[test]
    fn insert_and_recall_round_trip_normal() {
        let store = KnowledgeStore::open_in_memory().unwrap();
        let r = rec(Scope::Global, None, "Rust edition", "This workspace uses edition 2024")
            .with_confidence(0.9);
        store.insert(&r).unwrap();

        let hits = store
            .recall("edition", &RecallFilter::default())
            .unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].id, r.id);
        assert_eq!(hits[0].title, "Rust edition");
    }

    // SAFETY INVARIANT (PLAN §3.3): a record reaches global scope ONLY via the
    // explicit promote() gate, never via insert at runtime.
    #[test]
    fn project_record_is_not_global_until_promoted_regression() {
        let store = KnowledgeStore::open_in_memory().unwrap();
        let r = rec(Scope::Project, Some("projA"), "local lesson", "use ripgrep here");
        store.insert(&r).unwrap();

        // From a DIFFERENT project, the project-scoped row must NOT be recalled.
        let other = RecallFilter {
            project_key: Some("projB"),
            limit: 8,
        };
        assert!(store.recall("ripgrep", &other).unwrap().is_empty());

        // After human-gated promotion, it becomes visible everywhere.
        assert!(store.promote(&r.id).unwrap());
        assert_eq!(store.recall("ripgrep", &other).unwrap().len(), 1);
    }

    // SAFETY INVARIANT (PLAN §3): project rows for THIS project are visible;
    // user/global always visible; other projects' rows never.
    #[test]
    fn recall_scope_isolation_normal() {
        let store = KnowledgeStore::open_in_memory().unwrap();
        store.insert(&rec(Scope::Project, Some("A"), "a-only", "alpha secret")).unwrap();
        store.insert(&rec(Scope::Project, Some("B"), "b-only", "beta secret")).unwrap();
        store.insert(&rec(Scope::User, None, "user-pref", "alpha beta gamma")).unwrap();

        let from_a = RecallFilter { project_key: Some("A"), limit: 8 };
        let hits = store.recall("alpha", &from_a).unwrap();
        let titles: Vec<_> = hits.iter().map(|h| h.title.as_str()).collect();
        assert!(titles.contains(&"a-only"), "{titles:?}");
        assert!(titles.contains(&"user-pref"), "{titles:?}");
        assert!(!titles.contains(&"b-only"), "B's row leaked into A: {titles:?}");
    }

    #[test]
    fn supersede_hides_old_row_normal() {
        let store = KnowledgeStore::open_in_memory().unwrap();
        let old = rec(Scope::Global, None, "stack", "uses webpack");
        store.insert(&old).unwrap();
        let new = rec(Scope::Global, None, "stack", "uses vite now");
        store.insert(&new).unwrap();
        store.supersede(&old.id, &new.id).unwrap();

        let hits = store.recall("stack", &RecallFilter::default()).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].id, new.id, "stale row should be filtered out");
    }

    #[test]
    fn insert_rejects_invalid_records_robust() {
        let store = KnowledgeStore::open_in_memory().unwrap();
        // Empty body.
        assert!(store.insert(&rec(Scope::Global, None, "t", "  ")).is_err());
        // Project scope without a key.
        assert!(store.insert(&rec(Scope::Project, None, "t", "b")).is_err());
        // Global scope WITH a key.
        assert!(store.insert(&rec(Scope::Global, Some("x"), "t", "b")).is_err());
        // Out-of-range confidence (constructed directly, bypassing the clamp).
        let mut bad = rec(Scope::Global, None, "t", "b");
        bad.confidence = 5.0;
        assert!(store.insert(&bad).is_err());
    }

    // SAFETY INVARIANT (PLAN §3.4): bounded growth. Insert well past the cap and
    // assert decay holds the live count at/under it, and that a promoted/global
    // row survives the cull.
    #[test]
    fn decay_enforces_row_cap_and_spares_promoted_regression() {
        let mut store = KnowledgeStore::open_in_memory().unwrap();
        let keep = rec(Scope::Global, None, "promoted keeper", "must survive decay");
        store.insert(&keep).unwrap();

        for i in 0..50 {
            store
                .insert(&rec(Scope::Project, Some("P"), &format!("row {i}"), "filler body"))
                .unwrap();
        }
        assert_eq!(store.live_count().unwrap(), 51);

        let removed = store.decay(DEFAULT_MAX_AGE_MS, 10).unwrap();
        assert!(removed >= 40, "decay should prune project rows over the cap; removed={removed}");
        // Project rows capped at 10; the global keeper is exempt → <= 11 live.
        assert!(store.live_count().unwrap() <= 11);

        // The promoted/global row survived.
        let hits = store.recall("keeper", &RecallFilter::default()).unwrap();
        assert_eq!(hits.len(), 1);
    }

    #[test]
    fn mark_used_bumps_usage_and_influences_rank_normal() {
        let store = KnowledgeStore::open_in_memory().unwrap();
        let a = rec(Scope::Global, None, "shared term apple", "alpha").with_confidence(0.5);
        let b = rec(Scope::Global, None, "shared term apple", "beta").with_confidence(0.5);
        store.insert(&a).unwrap();
        store.insert(&b).unwrap();
        // Use `a` repeatedly → it should rank first on the next recall.
        for _ in 0..5 {
            store.mark_used(&[a.id.clone()]).unwrap();
        }
        let hits = store.recall("apple", &RecallFilter::default()).unwrap();
        assert_eq!(hits.first().map(|h| h.id.as_str()), Some(a.id.as_str()));
    }

    #[test]
    fn default_db_path_honors_env_override_normal() {
        // SAFETY: single-threaded test mutating a process env var, restored after.
        let prev = std::env::var_os("JFC_KNOWLEDGE_DB");
        unsafe { std::env::set_var("JFC_KNOWLEDGE_DB", "/tmp/custom-knowledge.db") };
        assert_eq!(default_db_path(), PathBuf::from("/tmp/custom-knowledge.db"));
        unsafe {
            match prev {
                Some(v) => std::env::set_var("JFC_KNOWLEDGE_DB", v),
                None => std::env::remove_var("JFC_KNOWLEDGE_DB"),
            }
        }
    }
}
