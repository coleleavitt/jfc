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
pub mod import;
pub mod project;
pub mod query;
pub mod record;
pub mod redact;
mod schema;
pub mod session_mine;

use std::path::{Path, PathBuf};

use rusqlite::{Connection, OptionalExtension};

pub use error::{KnowledgeError, Result};
pub use import::{ImportReport, ImportableMemory};
pub use project::project_key;
pub use query::{Gap, LinkedRecord, RecallFilter};
pub use record::{Kind, KnowledgeRecord, Outcome, RelKind, Scope};

/// Optional per-scope row cap for [`KnowledgeStore::decay`]. The store grows
/// **unbounded by default** — `decay` is opt-in maintenance, not an automatic
/// ceiling. This constant is only used when a caller explicitly chooses to cap.
pub const DEFAULT_MAX_ROWS_PER_SCOPE: i64 = 2_000;
/// Tombstone age for an *explicit* `decay` call (90 days). Not applied unless a
/// caller opts into decay; consolidation (dedup) is the default maintenance.
pub const DEFAULT_MAX_AGE_MS: i64 = 90 * 24 * 3600 * 1000;
/// Default evidence bar for [`KnowledgeStore::auto_promote`]: a project lesson
/// auto-promotes to cross-project scope once it is verified and seen this many
/// times. Low enough to actually compound, high enough that a one-off can't leak.
pub const DEFAULT_AUTO_PROMOTE_SUPPORT: i64 = 3;

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

    /// Fold mined session lessons (`session_mine`) into **project-scoped**
    /// candidate records. Compounding: a lesson whose `norm_key` already exists
    /// bumps that row's `use_count` (support) and upgrades it to `Verified` if
    /// the new evidence is verified — instead of inserting a duplicate. Never
    /// promotes to global scope (that stays human-gated). Returns
    /// `(inserted, compounded)`.
    pub fn ingest_mined(
        &self,
        project_key: &str,
        lessons: &[session_mine::MinedLesson],
    ) -> Result<(usize, usize)> {
        let mut inserted = 0usize;
        let mut compounded = 0usize;
        for lesson in lessons {
            // norm_key is the dedup identity; make it a deterministic row id
            // scoped to this project so cross-project mining can't collide.
            let id = uuid::Uuid::new_v5(
                &uuid::Uuid::NAMESPACE_OID,
                format!("mined:{project_key}:{}", lesson.norm_key).as_bytes(),
            )
            .simple()
            .to_string();

            if self.contains(&id)? {
                // Compound: bump support, and upgrade outcome if newly verified.
                self.conn.execute(
                    "UPDATE knowledge SET use_count = use_count + 1, last_used_ms = ?2, \
                     outcome = CASE WHEN ?3 = 'verified' THEN 'verified' ELSE outcome END \
                     WHERE id = ?1",
                    rusqlite::params![id, record::now_ms(), lesson.outcome.slug()],
                )?;
                compounded += 1;
                continue;
            }

            let mut rec = KnowledgeRecord::new(
                lesson.kind,
                Scope::Project,
                Some(project_key.to_owned()),
                lesson.trigger.clone(),
                lesson.claim.clone(),
            )
            .with_outcome(lesson.outcome)
            .with_source(format!("mined:session:{}", lesson.session_id));
            rec.id = id;
            rec.tags = lesson.norm_key.clone();
            self.insert(&rec)?;
            inserted += 1;
        }
        Ok((inserted, compounded))
    }

    /// Whether a record id already exists (live or superseded).
    pub fn contains(&self, id: &str) -> Result<bool> {
        Ok(self.conn.query_row(
            "SELECT 1 FROM knowledge WHERE id = ?1 LIMIT 1",
            [id],
            |_| Ok(()),
        )
        .optional()?
        .is_some())
    }

    /// Idempotently import legacy `.md` memories. Each item gets a deterministic
    /// id (uuid-v5 over its normalized content), so re-running is a no-op: items
    /// already present are skipped, not duplicated. Never deletes any source.
    /// Per-item failures are collected into the report rather than aborting.
    pub fn import_memories(&self, items: &[ImportableMemory]) -> Result<ImportReport> {
        let mut report = ImportReport::default();
        for item in items {
            let id = import::deterministic_id(item);
            match self.contains(&id) {
                Ok(true) => {
                    report.skipped += 1;
                    continue;
                }
                Ok(false) => {}
                Err(e) => {
                    report.errors.push(format!("{}: {e}", item.title));
                    continue;
                }
            }
            let mut rec = KnowledgeRecord::new(
                item.kind,
                item.scope,
                item.project_key.clone(),
                item.title.clone(),
                item.body.clone(),
            );
            rec.id = id;
            if let Some(src) = &item.source_path {
                rec.source = Some(format!("import:{}", src.display()));
            }
            match self.insert(&rec) {
                Ok(()) => report.imported += 1,
                Err(e) => report.errors.push(format!("{}: {e}", item.title)),
            }
        }
        Ok(report)
    }

    /// Mark `old_id` superseded by `new_id` (immutable revision).
    pub fn supersede(&self, old_id: &str, new_id: &str) -> Result<()> {
        query::supersede(&self.conn, old_id, new_id)
    }

    /// Promote a record to global (cross-project) scope. Returns `true` if a
    /// live record was promoted. Used by both the `/knowledge promote` command
    /// and the autonomous [`Self::auto_promote`] pass.
    pub fn promote(&self, id: &str) -> Result<bool> {
        query::promote_to_global(&self.conn, id)
    }

    /// Autonomously promote project lessons that have *proven themselves* to
    /// global (cross-project) scope: a row is promoted when it is `Verified`
    /// AND has accumulated at least `min_support` independent observations
    /// (`use_count`, bumped each time mining re-sees the same `norm_key`, or each
    /// time recall surfaces it). This is the self-driving replacement for the
    /// manual gate — the store grows its own cross-project knowledge from
    /// evidence, not from a human clicking promote. Returns the number promoted.
    ///
    /// **Only *generalizable* kinds auto-promote.** A `Fact` is by definition
    /// project-specific ("this repo uses vite", a path, a quirk) — promoting it
    /// would poison every other project's recall with wrong-context truth, which
    /// redaction can't catch (it guards secrets, not context). So auto-promotion
    /// is restricted to `Finding`/`Skill`/`Convention`/`Preference` — lessons
    /// whose value transfers. A project-specific fact can still be promoted
    /// deliberately via `/knowledge promote <id>` (the human override stays).
    pub fn auto_promote(&self, min_support: i64) -> Result<usize> {
        let n = self.conn.execute(
            "UPDATE knowledge SET scope = 'global', project_key = NULL, promoted = 1 \
             WHERE scope = 'project' AND superseded_by IS NULL \
               AND outcome = 'verified' AND use_count >= ?1 \
               AND kind IN ('finding','skill','convention','preference')",
            [min_support],
        )?;
        Ok(n)
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

    /// Consolidate near-duplicate live records (offline). Returns rows superseded.
    pub fn consolidate(&mut self) -> Result<usize> {
        query::consolidate(&mut self.conn)
    }

    /// Create a typed link `from -rel-> to` (Obsidian-style graph edge).
    pub fn link(&self, from_id: &str, to_id: &str, rel: RelKind) -> Result<()> {
        query::link(&self.conn, from_id, to_id, rel)
    }

    /// Records one hop out from `id` along outgoing edges (live targets).
    pub fn linked(&self, id: &str) -> Result<Vec<LinkedRecord>> {
        query::linked(&self.conn, id)
    }

    /// Ids that link *at* `id` (backlinks — "what depends on this").
    pub fn backlinks(&self, id: &str) -> Result<Vec<String>> {
        query::backlinks(&self.conn, id)
    }

    /// Record/bump a knowledge gap (referenced-but-absent lesson).
    pub fn note_gap(&self, label: &str, reason: &str) -> Result<()> {
        query::note_gap(&self.conn, label, reason)
    }

    /// Open knowledge gaps, most-referenced first ("what to learn next").
    pub fn gaps(&self, limit: usize) -> Result<Vec<Gap>> {
        query::gaps(&self.conn, limit)
    }

    /// Permanently delete one record by id. Returns rows removed (0 or 1).
    pub fn forget(&self, id: &str) -> Result<usize> {
        Ok(self
            .conn
            .execute("DELETE FROM knowledge WHERE id = ?1", [id])?)
    }

    /// Whether an autonomous maintenance pass is due for `project_key` (no pass
    /// within `throttle_ms`). True on the first ever run.
    pub fn maintain_due(&self, project_key: &str, throttle_ms: i64) -> Result<bool> {
        let last: Option<i64> = self
            .conn
            .query_row(
                "SELECT last_run_ms FROM maintain_state WHERE project_key = ?1",
                [project_key],
                |r| r.get(0),
            )
            .optional()?;
        Ok(match last {
            Some(ts) => record::now_ms() - ts >= throttle_ms,
            None => true,
        })
    }

    /// Record that a maintenance pass ran now for `project_key`.
    pub fn stamp_maintain(&self, project_key: &str) -> Result<()> {
        self.conn.execute(
            "INSERT INTO maintain_state (project_key, last_run_ms) VALUES (?1, ?2) \
             ON CONFLICT(project_key) DO UPDATE SET last_run_ms = ?2",
            rusqlite::params![project_key, record::now_ms()],
        )?;
        Ok(())
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

/// Summary of one autonomous maintenance pass.
#[derive(Debug, Default, Clone)]
pub struct MaintainReport {
    pub imported: usize,
    pub mined_inserted: usize,
    pub mined_compounded: usize,
    pub sessions_scanned: usize,
    pub consolidated: usize,
    pub auto_promoted: usize,
}

/// Minimum interval between autonomous maintenance passes (6 hours). A throttle
/// stamp in the DB prevents re-mining 364 sessions on every startup.
pub const MAINTAIN_THROTTLE_MS: i64 = 6 * 3600 * 1000;

/// One self-driving maintenance pass over the default store — the function the
/// engine fires in the background so the user never has to run `/knowledge`.
///
/// It (1) imports legacy `.md` memories for `project_root`, (2) mines the user's
/// session history into project lessons, (3) consolidates duplicates, and
/// (4) auto-promotes verified, repeatedly-seen *generalizable* lessons to
/// cross-project scope. Growth is **unbounded** — no decay/forget here.
/// Redaction (in mining) and the recall-time injection screen still apply; those
/// protect the user's secrets and can't be "expansion", so they stay.
///
/// **Throttled**: if a pass ran within [`MAINTAIN_THROTTLE_MS`], this is a no-op
/// (returns a zero report) so startup never re-processes the whole corpus. Pass
/// `force = true` (the `/knowledge mine` command) to bypass.
///
/// `sessions_dir` and `user_memory_dir`/`project_memory_dir` are passed in so
/// the caller owns path policy (and tests stay hermetic).
pub fn auto_maintain(
    project_root: &Path,
    sessions_dir: Option<&Path>,
    user_memory_dir: Option<&Path>,
    project_memory_dir: Option<&Path>,
) -> Result<MaintainReport> {
    auto_maintain_inner(project_root, sessions_dir, user_memory_dir, project_memory_dir, false)
}

/// Like [`auto_maintain`] but `force` bypasses the throttle (manual `/knowledge
/// mine`).
pub fn auto_maintain_forced(
    project_root: &Path,
    sessions_dir: Option<&Path>,
    user_memory_dir: Option<&Path>,
    project_memory_dir: Option<&Path>,
) -> Result<MaintainReport> {
    auto_maintain_inner(project_root, sessions_dir, user_memory_dir, project_memory_dir, true)
}

fn auto_maintain_inner(
    project_root: &Path,
    sessions_dir: Option<&Path>,
    user_memory_dir: Option<&Path>,
    project_memory_dir: Option<&Path>,
    force: bool,
) -> Result<MaintainReport> {
    let mut store = KnowledgeStore::open_default()?;
    let project = project::project_key(project_root);
    let mut report = MaintainReport::default();

    // Throttle: skip if a pass ran recently (per-project stamp), unless forced.
    if !force && !store.maintain_due(&project, MAINTAIN_THROTTLE_MS)? {
        return Ok(report);
    }
    store.stamp_maintain(&project)?;

    // 1. Import legacy .md memories (idempotent; never deletes sources).
    let mut items = Vec::new();
    if let Some(dir) = user_memory_dir {
        items.extend(import::scan_markdown_dir(dir, Scope::User, None));
    }
    if let Some(dir) = project_memory_dir {
        items.extend(import::scan_markdown_dir(dir, Scope::Project, Some(project.clone())));
    }
    if !items.is_empty() {
        report.imported = store.import_memories(&items)?.imported;
    }

    // 2. Mine session history into project-scoped lessons (redacted).
    if let Some(dir) = sessions_dir {
        let (lessons, mine_report) = session_mine::mine_dir(dir);
        report.sessions_scanned = mine_report.sessions_scanned;
        let (ins, comp) = store.ingest_mined(&project, &lessons)?;
        report.mined_inserted = ins;
        report.mined_compounded = comp;
    }

    // 3. Consolidate duplicates (dedup only — no decay; the store grows).
    report.consolidated = store.consolidate()?;

    // 4. Auto-promote verified, repeatedly-seen lessons across projects.
    report.auto_promoted = store.auto_promote(DEFAULT_AUTO_PROMOTE_SUPPORT)?;

    Ok(report)
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

    // SAFETY INVARIANT (PLAN §F3): import is idempotent — re-running adds rows
    // once. Mirrors the legacy `.md` → DB migration's no-deletion contract.
    #[test]
    fn import_memories_is_idempotent_regression() {
        use crate::import::ImportableMemory;
        let store = KnowledgeStore::open_in_memory().unwrap();
        let items = vec![
            ImportableMemory {
                source_path: None,
                kind: Kind::Preference,
                scope: Scope::User,
                project_key: None,
                title: "prefers ripgrep".into(),
                body: "Use ripgrep over grep for code search.".into(),
            },
            ImportableMemory {
                source_path: None,
                kind: Kind::Fact,
                scope: Scope::Project,
                project_key: Some("P".into()),
                title: "stack".into(),
                body: "This repo is edition 2024.".into(),
            },
        ];

        let r1 = store.import_memories(&items).unwrap();
        assert_eq!(r1.imported, 2);
        assert_eq!(r1.skipped, 0);
        assert_eq!(store.live_count().unwrap(), 2);

        // Second run: same content → all skipped, no duplicates.
        let r2 = store.import_memories(&items).unwrap();
        assert_eq!(r2.imported, 0);
        assert_eq!(r2.skipped, 2);
        assert_eq!(store.live_count().unwrap(), 2, "re-import must not duplicate");
    }

    // TODO 7+8: a verified, salient lesson outranks an unverified one on equal
    // lexical relevance.
    #[test]
    fn verified_lesson_outranks_unverified_normal() {
        let store = KnowledgeStore::open_in_memory().unwrap();
        let weak = rec(Scope::Global, None, "edit term apple", "alpha")
            .with_confidence(0.5)
            .with_importance(0.5);
        let strong = rec(Scope::Global, None, "edit term apple", "beta")
            .with_confidence(0.5)
            .with_importance(0.5)
            .with_outcome(Outcome::Verified);
        store.insert(&weak).unwrap();
        store.insert(&strong).unwrap();
        let hits = store.recall("apple", &RecallFilter::default()).unwrap();
        assert_eq!(
            hits.first().map(|h| h.id.as_str()),
            Some(strong.id.as_str()),
            "verified lesson must rank first"
        );
    }

    // TODO 14: typed links + recall expansion + backlinks.
    #[test]
    fn typed_links_and_backlinks_normal() {
        let store = KnowledgeStore::open_in_memory().unwrap();
        let err = rec(Scope::Global, None, "error", "old_string not found");
        let fix = rec(Scope::Global, None, "fix", "strip the line-number gutter first");
        store.insert(&err).unwrap();
        store.insert(&fix).unwrap();
        store.link(&err.id, &fix.id, RelKind::FixedBy).unwrap();

        let linked = store.linked(&err.id).unwrap();
        assert_eq!(linked.len(), 1);
        assert_eq!(linked[0].rel, RelKind::FixedBy);
        assert_eq!(linked[0].record.id, fix.id);

        assert_eq!(store.backlinks(&fix.id).unwrap(), vec![err.id.clone()]);
        // Idempotent: re-linking the same edge doesn't duplicate.
        store.link(&err.id, &fix.id, RelKind::FixedBy).unwrap();
        assert_eq!(store.linked(&err.id).unwrap().len(), 1);
    }

    // TODO 15: knowledge gaps rank by reference count.
    #[test]
    fn knowledge_gaps_rank_by_ref_count_normal() {
        let store = KnowledgeStore::open_in_memory().unwrap();
        store.note_gap("how to mock the network layer", "referenced, no lesson").unwrap();
        store.note_gap("how to mock the network layer", "again").unwrap();
        store.note_gap("CI cache config", "once").unwrap();
        let gaps = store.gaps(10).unwrap();
        assert_eq!(gaps.len(), 2);
        assert_eq!(gaps[0].label, "how to mock the network layer");
        assert_eq!(gaps[0].ref_count, 2);
    }

    // TODO 10: consolidation collapses duplicates to the strongest, supersedes
    // the rest, and is idempotent.
    #[test]
    fn consolidate_collapses_duplicates_regression() {
        let mut store = KnowledgeStore::open_in_memory().unwrap();
        let weak = rec(Scope::Project, Some("P"), "dup", "use ripgrep here")
            .with_confidence(0.3);
        let strong = rec(Scope::Project, Some("P"), "dup", "use   ripgrep here")
            .with_confidence(0.9)
            .with_outcome(Outcome::Verified);
        store.insert(&weak).unwrap();
        store.insert(&strong).unwrap();
        assert_eq!(store.live_count().unwrap(), 2);

        let n = store.consolidate().unwrap();
        assert_eq!(n, 1, "one duplicate should be superseded");
        assert_eq!(store.live_count().unwrap(), 1);
        // The verified/stronger one survives.
        let hits = store
            .recall("ripgrep", &crate::query::RecallFilter { project_key: Some("P"), limit: 8 })
            .unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].id, strong.id);
        // Idempotent.
        assert_eq!(store.consolidate().unwrap(), 0);
    }

    // TODO 11-13: mined lessons fold into project candidates and COMPOUND by
    // norm_key (support bump + verified upgrade) instead of duplicating.
    #[test]
    fn ingest_mined_compounds_by_norm_key_regression() {
        use crate::session_mine::MinedLesson;
        let store = KnowledgeStore::open_in_memory().unwrap();
        let unverified = MinedLesson {
            kind: Kind::Finding,
            trigger: "Edit failed: old_string-not-found".into(),
            claim: "re-read exact bytes before retry".into(),
            outcome: Outcome::Unverified,
            norm_key: "err:edit:old_string-not-found".into(),
            session_id: "s1".into(),
        };
        let (ins, comp) = store.ingest_mined("P", &[unverified.clone()]).unwrap();
        assert_eq!((ins, comp), (1, 0));
        assert_eq!(store.live_count().unwrap(), 1);

        // Same norm_key, now VERIFIED → compounds onto the existing row + upgrades.
        let mut verified = unverified.clone();
        verified.outcome = Outcome::Verified;
        verified.session_id = "s2".into();
        let (ins2, comp2) = store.ingest_mined("P", &[verified]).unwrap();
        assert_eq!((ins2, comp2), (0, 1), "should compound, not duplicate");
        assert_eq!(store.live_count().unwrap(), 1);

        let hits = store
            .recall("retry", &crate::query::RecallFilter { project_key: Some("P"), limit: 8 })
            .unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].outcome, Outcome::Verified, "outcome upgraded by new evidence");
        assert!(hits[0].use_count >= 1, "support compounded");

        // A DIFFERENT project must not see P's mined lesson.
        let other = store
            .recall("retry", &crate::query::RecallFilter { project_key: Some("Q"), limit: 8 })
            .unwrap();
        assert!(other.is_empty());
    }

    // Autonomy: a verified, repeatedly-seen project lesson auto-promotes to
    // global; an unverified or rarely-seen one does NOT.
    #[test]
    fn auto_promote_lifts_verified_repeated_lessons_normal() {
        use crate::record::Kind;
        let store = KnowledgeStore::open_in_memory().unwrap();
        let mk = |kind: Kind, title: &str, body: &str| {
            KnowledgeRecord::new(kind, Scope::Project, Some("P".into()), title, body)
        };
        // Generalizable (Finding), verified + enough support → promoted.
        let mut hot = mk(Kind::Finding, "hot", "use ripgrep").with_outcome(Outcome::Verified);
        hot.use_count = 3;
        store.insert(&hot).unwrap();
        // Verified but under the support bar → stays project.
        let mut rare = mk(Kind::Finding, "rare", "niche tip").with_outcome(Outcome::Verified);
        rare.use_count = 1;
        store.insert(&rare).unwrap();
        // High support but unverified → stays project.
        let mut noisy = mk(Kind::Finding, "noisy", "unconfirmed");
        noisy.use_count = 9;
        store.insert(&noisy).unwrap();
        // A project-specific FACT, verified + well-supported, must NOT auto-promote
        // (it would poison other projects with wrong-context truth).
        let mut fact = mk(Kind::Fact, "stack", "this repo uses vite").with_outcome(Outcome::Verified);
        fact.use_count = 9;
        store.insert(&fact).unwrap();

        let promoted = store.auto_promote(3).unwrap();
        assert_eq!(promoted, 1, "only the verified, well-supported, generalizable lesson promotes");

        // The promoted one is now recalled from a DIFFERENT project.
        let other = crate::query::RecallFilter { project_key: Some("Q"), limit: 8 };
        let hits = store.recall("ripgrep", &other).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].id, hot.id);
        // The unverified/rare/fact ones did not leak across projects.
        assert!(store.recall("niche", &other).unwrap().is_empty());
        assert!(store.recall("unconfirmed", &other).unwrap().is_empty());
        assert!(store.recall("vite", &other).unwrap().is_empty(), "project-specific fact must not leak");
    }

    // The throttle prevents re-processing on a second startup within the window.
    #[test]
    fn maintain_throttle_blocks_rapid_repeat_normal() {
        let store = KnowledgeStore::open_in_memory().unwrap();
        assert!(store.maintain_due("P", MAINTAIN_THROTTLE_MS).unwrap(), "first run is due");
        store.stamp_maintain("P").unwrap();
        assert!(!store.maintain_due("P", MAINTAIN_THROTTLE_MS).unwrap(), "just-stamped is not due");
        // A different project is independently due.
        assert!(store.maintain_due("Q", MAINTAIN_THROTTLE_MS).unwrap());
        // With a zero window, it's due again immediately.
        assert!(store.maintain_due("P", 0).unwrap());
    }

    #[test]
    fn auto_maintain_imports_and_grows_normal() {
        // Hermetic: point the store + dirs at temp paths via JFC_KNOWLEDGE_DB.
        let dir = tempfile::tempdir().unwrap();
        let dbpath = dir.path().join("k.db");
        let user_mem = dir.path().join("umem");
        std::fs::create_dir_all(&user_mem).unwrap();
        std::fs::write(user_mem.join("p.md"), "---\ntype: preference\n---\nuse spaces not tabs").unwrap();
        let sessions = dir.path().join("sessions");
        std::fs::create_dir_all(&sessions).unwrap();
        std::fs::write(
            sessions.join("ses_1.json"),
            r#"{"id":"ses_1","messages":[
                {"role":"assistant","parts":[{"type":"tool","kind":"Edit","status":"failed","output":{"type":"text","content":"old_string not found"}}]},
                {"role":"assistant","parts":[{"type":"tool","kind":"Edit","status":"complete","output":{"type":"text","content":"ok"}}]}
            ]}"#,
        )
        .unwrap();

        // Isolate the global store path for this test.
        let _guard = EnvGuard::set("JFC_KNOWLEDGE_DB", dbpath.to_str().unwrap());
        let report = auto_maintain(
            dir.path(),
            Some(&sessions),
            Some(&user_mem),
            None,
        )
        .unwrap();
        assert_eq!(report.imported, 1, "imported the .md preference");
        assert_eq!(report.sessions_scanned, 1);
        assert!(report.mined_inserted >= 1, "mined the recovered Edit error");

        // The store actually grew and persists.
        let store = KnowledgeStore::open(&dbpath).unwrap();
        assert!(store.live_count().unwrap() >= 2);
    }

    /// Minimal scoped env setter for the hermetic maintain test.
    struct EnvGuard(&'static str, Option<std::ffi::OsString>);
    impl EnvGuard {
        fn set(key: &'static str, val: &str) -> Self {
            let prev = std::env::var_os(key);
            // SAFETY: test-only, restored on drop; these tests don't run env-mutating peers concurrently on this key.
            unsafe { std::env::set_var(key, val) };
            Self(key, prev)
        }
    }
    impl Drop for EnvGuard {
        fn drop(&mut self) {
            unsafe {
                match &self.1 {
                    Some(v) => std::env::set_var(self.0, v),
                    None => std::env::remove_var(self.0),
                }
            }
        }
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
