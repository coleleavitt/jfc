//! `jfc-knowledge` — a durable, cross-project memory & learning store.
//!
//! This is the Phase 1 storage layer from `PLAN.md`: a single SQLite database at
//! `~/.local/share/jfc/knowledge.db` holding facts, preferences, induced skills,
//! verification findings, and conventions that accumulate **across every project**
//! the user works in — the bounded, scaffolding-level self-improvement flywheel.
//!
//! ## Safety boundary (load-bearing — see `PLAN.md` §3)
//!
//! - **Cross-project leakage is bounded.** A record becomes [`Scope::Global`]
//!   (visible to every project) only through explicit [`KnowledgeStore::promote`]
//!   or [`KnowledgeStore::auto_promote`], which requires verified, repeatedly
//!   seen, generalizable lessons.
//! - **Recall is advisory context, never an action.** [`KnowledgeStore::recall`]
//!   returns rows to fold into a prompt; nothing here executes anything.
//! - **Bounded growth.** [`KnowledgeStore::decay`] caps rows and prunes
//!   tombstones, so the store can't grow without bound.
//! - **Kill switch.** The entire store is one file; deleting it is a full reset.
//!
//! Phase 1 ships dormant: the crate compiles and is tested, but no other crate
//! reads it yet (that's Phase 2).

mod agent_events;
pub mod definitions;
pub mod error;
pub mod import;
pub mod memory;
pub mod project;
pub mod query;
pub mod record;
pub mod redact;
mod schema;
pub mod session_mine;

use std::path::{Path, PathBuf};
use std::str::FromStr;

use sqlx::AssertSqlSafe;
use sqlx::Row;
use sqlx::sqlite::{
    SqliteConnectOptions, SqliteJournalMode, SqlitePool, SqlitePoolOptions, SqliteRow,
    SqliteSynchronous,
};

pub use agent_events::{
    AgentEventRow, AgentMailboxRow, AgentSessionRow, ContextEventRow, LearningEventRow,
    ToolRunLedgerRow,
};
pub use definitions::{DefinitionRecord, DefinitionScope, DefinitionStatus, NewDefinition};
pub use error::{KnowledgeError, Result};
pub use import::{ImportReport, ImportableMemory};
pub use memory::{MemLevel, MemoryRow, NewMemory, memory_id};
pub use project::project_key;
pub use query::{Gap, LinkedRecord, RecallFilter};
pub use record::{Kind, KnowledgeRecord, Outcome, RelKind, Scope};
// `SessionRow` is defined in this module; re-stated in the public surface for
// discoverability alongside the other exported types.

/// Run an async knowledge-store future to completion from a **synchronous**
/// call site (a `Drop` impl, a `Mutex`-guarded persist path, a non-async trait
/// method).
///
/// The knowledge store is async (sqlx), but a handful of callers are
/// structurally synchronous and cannot be made async without a second viral
/// cascade. This bridge drives the future on a **dedicated OS thread that owns a
/// fresh current-thread runtime**. Running on a separate thread is what makes it
/// safe from *any* context:
///
/// - From inside a multi-thread Tokio runtime (JFC's `#[tokio::main]` binary):
///   the worker thread isn't blocked re-entrantly, and there is no
///   `block_in_place` requirement.
/// - From inside a **current-thread** runtime (`#[tokio::test]` default flavor):
///   a plain `Handle::block_on` / `block_in_place` would panic ("Cannot start a
///   runtime from within a runtime" / "can call blocking only when running on
///   the multi-threaded runtime"). The dedicated thread has no ambient runtime,
///   so its `block_on` is valid.
/// - From a pure sync context (no runtime at all): same path, works directly.
///
/// `F` and its output must be `Send` because they cross the thread boundary —
/// satisfied by every call site (sqlx `SqlitePool` and its query futures are
/// `Send`).
pub fn block_on_knowledge<F>(fut: F) -> F::Output
where
    F: std::future::Future + Send,
    F::Output: Send,
{
    use std::sync::OnceLock;
    use tokio::runtime::Runtime;

    // One persistent multi-thread runtime, shared by every bridge call. This is
    // load-bearing for the `sqlx` SQLite pool: a pooled connection is bound to
    // the runtime that created it, and an in-memory (`:memory:`, shared-cache)
    // database lives only as long as that connection. A fresh runtime-per-call
    // would tear down a runtime after each bridged operation, dropping the
    // pool's connection (and, for in-memory stores, destroying the database —
    // "no such table" on the next read). Reusing one runtime keeps the
    // connection — and the data — alive across calls.
    static BRIDGE: OnceLock<Runtime> = OnceLock::new();
    let runtime = BRIDGE.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .enable_all()
            .build()
            .expect("build shared runtime for block_on_knowledge")
    });

    // Drive the future on a short-lived scoped thread that has NO ambient Tokio
    // runtime, so `Runtime::block_on` is valid even when the CALLER is inside a
    // runtime (the app's `#[tokio::main]`, or a `#[tokio::test]` — where a plain
    // `block_on`/`block_in_place` on the caller thread would panic with "Cannot
    // start a runtime from within a runtime"). The scoped thread reuses the
    // shared runtime above (keeping the pool connection alive) and joins before
    // returning, so `fut` may borrow non-`'static` local state (e.g. `&store`).
    std::thread::scope(|scope| {
        scope
            .spawn(|| runtime.block_on(fut))
            .join()
            .expect("block_on_knowledge worker thread panicked")
    })
}

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
/// Holds a [`SqlitePool`] (capped at one connection so writes serialize like the
/// old single `Connection`, and an in-memory DB isn't dropped under the pool).
/// All methods are `async`. WAL mode + a busy timeout (set on the connect
/// options) make concurrent JFC processes safe.
pub struct KnowledgeStore {
    pool: SqlitePool,
}

impl KnowledgeStore {
    /// Crate-internal pool accessor, so the split-out modules (`memory`,
    /// `definitions`, `agent_events`) can add `impl KnowledgeStore` methods and
    /// the `schema` tests can reach the pool without the field being `pub`.
    pub fn pool(&self) -> &SqlitePool {
        &self.pool
    }

    /// Open (creating if needed) and migrate the store at the default path
    /// `~/.local/share/jfc/knowledge.db`.
    pub async fn open_default() -> Result<Self> {
        let path = default_db_path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        Self::open(&path).await
    }

    /// Open (creating if needed) and migrate the store at `path`.
    pub async fn open(path: &Path) -> Result<Self> {
        let opts = SqliteConnectOptions::new()
            .filename(path)
            .create_if_missing(true)
            .journal_mode(SqliteJournalMode::Wal)
            .synchronous(SqliteSynchronous::Normal)
            .foreign_keys(true)
            .busy_timeout(std::time::Duration::from_secs(5));
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(opts)
            .await?;
        schema::migrate(&pool).await?;
        Ok(Self { pool })
    }

    /// An in-memory store — for tests and ephemeral use.
    ///
    /// A `sqlite::memory:` database lives only as long as its connection. The
    /// pool is therefore pinned to exactly one permanent connection
    /// (`min_connections(1)` + `max_connections(1)`, no idle/max-lifetime
    /// eviction) so the database isn't destroyed when the pool would otherwise
    /// reap an idle connection — which happened between a write on one runtime
    /// and a read driven from the `block_on_knowledge` bridge thread.
    pub async fn open_in_memory() -> Result<Self> {
        let opts = SqliteConnectOptions::from_str("sqlite::memory:")?;
        let pool = SqlitePoolOptions::new()
            .min_connections(1)
            .max_connections(1)
            .idle_timeout(None)
            .max_lifetime(None)
            .connect_with(opts)
            .await?;
        schema::migrate(&pool).await?;
        Ok(Self { pool })
    }

    /// Insert a record (validated at the boundary).
    pub async fn insert(&self, rec: &KnowledgeRecord) -> Result<()> {
        query::insert(&self.pool, rec).await
    }

    /// Fold mined session lessons (`session_mine`) into **project-scoped**
    /// candidate records. Compounding: a lesson whose `norm_key` already exists
    /// bumps that row's `use_count` (support) and upgrades it to `Verified` if
    /// the new evidence is verified — instead of inserting a duplicate. Never
    /// promotes directly; callers run [`Self::auto_promote`] after compounding
    /// enough verified evidence. Returns
    /// `(inserted, compounded)`.
    pub async fn ingest_mined(
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

            if self.contains(&id).await? {
                // Compound: bump support, and upgrade outcome if newly verified.
                sqlx::query(
                    "UPDATE knowledge SET use_count = use_count + 1, last_used_ms = ?2, \
                     outcome = CASE WHEN ?3 = 'verified' THEN 'verified' ELSE outcome END \
                     WHERE id = ?1",
                )
                .bind(&id)
                .bind(record::now_ms())
                .bind(lesson.outcome.slug())
                .execute(&self.pool)
                .await?;
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
            self.insert(&rec).await?;
            inserted += 1;
        }
        Ok((inserted, compounded))
    }

    /// Whether a record id already exists (live or superseded).
    pub async fn contains(&self, id: &str) -> Result<bool> {
        Ok(sqlx::query("SELECT 1 FROM knowledge WHERE id = ?1 LIMIT 1")
            .bind(id)
            .fetch_optional(&self.pool)
            .await?
            .is_some())
    }

    /// Idempotently import legacy `.md` memories. Each item gets a deterministic
    /// id (uuid-v5 over its normalized content), so re-running is a no-op: items
    /// already present are skipped, not duplicated. Never deletes any source.
    /// Per-item failures are collected into the report rather than aborting.
    pub async fn import_memories(&self, items: &[ImportableMemory]) -> Result<ImportReport> {
        let mut report = ImportReport::default();
        for item in items {
            let id = import::deterministic_id(item);
            match self.contains(&id).await {
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
            let (level, project_key) = import_memory_level(item);
            let hash = import_memory_hash(item);
            let meta_json = import_memory_meta_json(item, &hash);
            match self
                .insert_memory(&NewMemory {
                    id,
                    level,
                    project_key,
                    title: &item.title,
                    body: &item.body,
                    hash: &hash,
                    meta_json: &meta_json,
                })
                .await
            {
                Ok(()) => report.imported += 1,
                Err(e) => report.errors.push(format!("{}: {e}", item.title)),
            }
        }
        Ok(report)
    }

    /// Mark `old_id` superseded by `new_id` (immutable revision).
    pub async fn supersede(&self, old_id: &str, new_id: &str) -> Result<()> {
        query::supersede(&self.pool, old_id, new_id).await
    }

    /// Promote a record to global (cross-project) scope. Returns `true` if a
    /// live record was promoted. Used by the explicit `/knowledge promote`
    /// command or an approved proposal.
    pub async fn promote(&self, id: &str) -> Result<bool> {
        query::promote_to_global(&self.pool, id).await
    }

    /// Promote project lessons that have *proven themselves* to global
    /// (cross-project) scope.
    ///
    /// **Only *generalizable* kinds auto-promote.** A `Fact` is by definition
    /// project-specific ("this repo uses vite", a path, a quirk) — promoting it
    /// would poison every other project's recall with wrong-context truth, which
    /// redaction can't catch (it guards secrets, not context). So auto-promotion
    /// is restricted to `Finding`/`Skill`/`Convention`/`Preference` — lessons
    /// whose value transfers. A project-specific fact can still be promoted
    /// deliberately via `/knowledge promote <id>`.
    pub async fn auto_promote(&self, min_support: i64) -> Result<usize> {
        let result = sqlx::query(
            "UPDATE knowledge SET scope = 'global', project_key = NULL, promoted = 1 \
             WHERE scope = 'project' AND superseded_by IS NULL \
               AND outcome = 'verified' AND use_count >= ?1 \
               AND kind IN ('finding','skill','convention','preference')",
        )
        .bind(min_support)
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected() as usize)
    }

    /// Recall advisory context for `query` (lexical FTS). Eligible rows are
    /// user + global + this-project. Does not bump usage — call [`Self::mark_used`]
    /// on the records you actually surface.
    pub async fn recall(
        &self,
        query: &str,
        filter: &RecallFilter<'_>,
    ) -> Result<Vec<KnowledgeRecord>> {
        query::recall(&self.pool, query, filter).await
    }

    /// Bump usage metrics for records that were surfaced.
    pub async fn mark_used(&self, ids: &[String]) -> Result<()> {
        query::mark_used(&self.pool, ids).await
    }

    /// Bounded-growth maintenance. Returns the number of rows removed.
    pub async fn decay(&self, max_age_ms: i64, max_rows_per_scope: i64) -> Result<usize> {
        query::decay(&self.pool, max_age_ms, max_rows_per_scope).await
    }

    /// Consolidate near-duplicate live records (offline). Returns rows superseded.
    pub async fn consolidate(&self) -> Result<usize> {
        query::consolidate(&self.pool).await
    }

    /// Create a typed link `from -rel-> to` (Obsidian-style graph edge).
    pub async fn link(&self, from_id: &str, to_id: &str, rel: RelKind) -> Result<()> {
        query::link(&self.pool, from_id, to_id, rel).await
    }

    /// Records one hop out from `id` along outgoing edges (live targets).
    pub async fn linked(&self, id: &str) -> Result<Vec<LinkedRecord>> {
        query::linked(&self.pool, id).await
    }

    /// Ids that link *at* `id` (backlinks — "what depends on this").
    pub async fn backlinks(&self, id: &str) -> Result<Vec<String>> {
        query::backlinks(&self.pool, id).await
    }

    /// Record/bump a knowledge gap (referenced-but-absent lesson).
    pub async fn note_gap(&self, label: &str, reason: &str) -> Result<()> {
        query::note_gap(&self.pool, label, reason).await
    }

    /// Open knowledge gaps, most-referenced first ("what to learn next").
    pub async fn gaps(&self, limit: usize) -> Result<Vec<Gap>> {
        query::gaps(&self.pool, limit).await
    }

    /// Permanently delete one record by id. Returns rows removed (0 or 1).
    pub async fn forget(&self, id: &str) -> Result<usize> {
        let result = sqlx::query("DELETE FROM knowledge WHERE id = ?1")
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(result.rows_affected() as usize)
    }

    /// Upsert a session-index row. The SQLite session catalog is the primary
    /// picker/search surface.
    pub async fn upsert_session(&self, row: &SessionRow) -> Result<()> {
        sqlx::query(
            "INSERT INTO sessions \
             (id, cwd, model, created_at, updated_at, first_prompt, title, message_count) \
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8) \
             ON CONFLICT(id) DO UPDATE SET \
                cwd=excluded.cwd, model=excluded.model, created_at=excluded.created_at, \
                updated_at=excluded.updated_at, first_prompt=excluded.first_prompt, \
                title=excluded.title, message_count=excluded.message_count",
        )
        .bind(&row.id)
        .bind(&row.cwd)
        .bind(&row.model)
        .bind(&row.created_at)
        .bind(&row.updated_at)
        .bind(&row.first_prompt)
        .bind(&row.title)
        .bind(row.message_count)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// One session-index row by id (or `None`).
    pub async fn get_session(&self, id: &str) -> Result<Option<SessionRow>> {
        sqlx::query(
            "SELECT id, cwd, model, created_at, updated_at, first_prompt, title, message_count \
             FROM sessions WHERE id = ?1",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?
        .as_ref()
        .map(session_row_from)
        .transpose()
    }

    /// Session-index rows, most-recently-updated first. `cwd` filters to one
    /// project when `Some`.
    pub async fn list_sessions(&self, cwd: Option<&str>, limit: usize) -> Result<Vec<SessionRow>> {
        let rows = if let Some(cwd) = cwd {
            sqlx::query(
                "SELECT id, cwd, model, created_at, updated_at, first_prompt, title, message_count \
                 FROM sessions WHERE cwd = ?1 ORDER BY updated_at DESC LIMIT ?2",
            )
            .bind(cwd)
            .bind(limit as i64)
            .fetch_all(&self.pool)
            .await?
        } else {
            sqlx::query(
                "SELECT id, cwd, model, created_at, updated_at, first_prompt, title, message_count \
                 FROM sessions ORDER BY updated_at DESC LIMIT ?1",
            )
            .bind(limit as i64)
            .fetch_all(&self.pool)
            .await?
        };
        rows.iter().map(session_row_from).collect()
    }

    /// Delete one session row and its transcript. Returns the number of DB rows
    /// removed across session-owned tables.
    pub async fn delete_session(&self, id: &str) -> Result<usize> {
        let mut tx = self.pool.begin().await?;
        let agent_scoped = agent_events::delete_session_scoped_rows(&mut tx, id).await?;
        let mut removed = agent_scoped;
        for sql in [
            "DELETE FROM session_artifact_events WHERE session_id = ?1",
            "DELETE FROM session_artifacts WHERE session_id = ?1",
            "DELETE FROM session_findings WHERE session_id = ?1",
            "DELETE FROM session_compactions WHERE session_id = ?1",
            "DELETE FROM session_retrieval_events WHERE session_id = ?1",
            "DELETE FROM session_tool_runs WHERE session_id = ?1",
            "DELETE FROM session_turns WHERE session_id = ?1",
            "DELETE FROM session_events WHERE session_id = ?1",
            "DELETE FROM session_messages WHERE session_id = ?1",
            "DELETE FROM sessions WHERE id = ?1",
        ] {
            let result = sqlx::query(sql).bind(id).execute(&mut *tx).await?;
            removed += result.rows_affected() as usize;
        }
        tx.commit().await?;
        Ok(removed)
    }

    /// Count of indexed sessions — for tests/metrics.
    pub async fn session_count(&self) -> Result<i64> {
        Ok(sqlx::query("SELECT count(*) FROM sessions")
            .fetch_one(&self.pool)
            .await?
            .try_get(0)?)
    }

    /// Replace a session's full transcript. The header upsert and
    /// all message rows commit in ONE transaction (council decision 1+5:
    /// cross-row atomicity the per-file rename never gave us). We delete+reinsert
    /// rather than diff because the engine coalesces messages on save, so seq
    /// numbers can shift; the FTS mirror stays consistent via triggers.
    pub async fn replace_transcript(
        &self,
        row: &SessionRow,
        messages: &[SessionMessage],
    ) -> Result<()> {
        let mut tx = self.pool.begin().await?;
        sqlx::query(
            "INSERT INTO sessions \
             (id, cwd, model, created_at, updated_at, first_prompt, title, message_count) \
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8) \
             ON CONFLICT(id) DO UPDATE SET \
                cwd=excluded.cwd, model=excluded.model, created_at=excluded.created_at, \
                updated_at=excluded.updated_at, first_prompt=excluded.first_prompt, \
                title=excluded.title, message_count=excluded.message_count",
        )
        .bind(&row.id)
        .bind(&row.cwd)
        .bind(&row.model)
        .bind(&row.created_at)
        .bind(&row.updated_at)
        .bind(&row.first_prompt)
        .bind(&row.title)
        .bind(row.message_count)
        .execute(&mut *tx)
        .await?;
        for sql in [
            "DELETE FROM session_messages WHERE session_id = ?1",
            "DELETE FROM session_events WHERE session_id = ?1",
            "DELETE FROM session_turns WHERE session_id = ?1",
            "DELETE FROM session_tool_runs WHERE session_id = ?1",
        ] {
            sqlx::query(sql).bind(&row.id).execute(&mut *tx).await?;
        }
        agent_events::clear_derived_context_events(&mut tx, &row.id).await?;
        for m in messages {
            sqlx::query(
                "INSERT INTO session_messages (session_id, seq, role, content, meta) \
                 VALUES (?1,?2,?3,?4,?5)",
            )
            .bind(&row.id)
            .bind(m.seq)
            .bind(&m.role)
            .bind(&m.content)
            .bind(&m.meta)
            .execute(&mut *tx)
            .await?;
        }
        insert_derived_session_rows(&mut tx, row, messages).await?;
        agent_events::insert_context_events_from_messages(&mut tx, row, messages, record::now_ms())
            .await?;
        tx.commit().await?;
        Ok(())
    }

    /// Load a session's full transcript in `seq` order (resume path, post-flip).
    pub async fn load_transcript(&self, session_id: &str) -> Result<Vec<SessionMessage>> {
        let rows = sqlx::query(
            "SELECT seq, role, content, meta FROM session_messages \
             WHERE session_id = ?1 ORDER BY seq ASC",
        )
        .bind(session_id)
        .fetch_all(&self.pool)
        .await?;
        rows.iter()
            .map(|r| {
                Ok(SessionMessage {
                    seq: r.try_get(0)?,
                    role: r.try_get(1)?,
                    content: r.try_get(2)?,
                    meta: r.try_get(3)?,
                })
            })
            .collect()
    }

    /// Whether a session has any transcript rows (parity bookkeeping).
    pub async fn has_transcript(&self, session_id: &str) -> Result<bool> {
        Ok(
            sqlx::query("SELECT 1 FROM session_messages WHERE session_id = ?1 LIMIT 1")
                .bind(session_id)
                .fetch_optional(&self.pool)
                .await?
                .is_some(),
        )
    }

    pub async fn list_session_events(
        &self,
        session_id: &str,
        limit: usize,
    ) -> Result<Vec<SessionEventRow>> {
        let rows = sqlx::query(
            "SELECT id, session_id, seq, kind, created_at_ms, payload \
             FROM session_events WHERE session_id = ?1 ORDER BY seq ASC, created_at_ms ASC \
             LIMIT ?2",
        )
        .bind(session_id)
        .bind(limit as i64)
        .fetch_all(&self.pool)
        .await?;
        rows.iter()
            .map(|r| {
                Ok(SessionEventRow {
                    id: r.try_get(0)?,
                    session_id: r.try_get(1)?,
                    seq: r.try_get(2)?,
                    kind: r.try_get(3)?,
                    created_at_ms: r.try_get(4)?,
                    payload: r.try_get(5)?,
                })
            })
            .collect()
    }

    pub async fn list_session_turns(&self, session_id: &str) -> Result<Vec<SessionTurnRow>> {
        let rows = sqlx::query(
            "SELECT session_id, turn_index, user_seq, assistant_seq, user_text, assistant_text, \
                    status, model, created_at_ms \
             FROM session_turns WHERE session_id = ?1 ORDER BY turn_index ASC",
        )
        .bind(session_id)
        .fetch_all(&self.pool)
        .await?;
        rows.iter()
            .map(|r| {
                Ok(SessionTurnRow {
                    session_id: r.try_get(0)?,
                    turn_index: r.try_get(1)?,
                    user_seq: r.try_get(2)?,
                    assistant_seq: r.try_get(3)?,
                    user_text: r.try_get(4)?,
                    assistant_text: r.try_get(5)?,
                    status: r.try_get(6)?,
                    model: r.try_get(7)?,
                    created_at_ms: r.try_get(8)?,
                })
            })
            .collect()
    }

    pub async fn list_session_tool_runs(&self, session_id: &str) -> Result<Vec<SessionToolRunRow>> {
        let rows = sqlx::query(
            "SELECT id, session_id, message_seq, part_index, tool_call_id, runtime_id, kind, \
                    status, input_json, output_json, duration_ms, created_at_ms \
             FROM session_tool_runs WHERE session_id = ?1 \
             ORDER BY message_seq ASC, part_index ASC",
        )
        .bind(session_id)
        .fetch_all(&self.pool)
        .await?;
        rows.iter()
            .map(|r| {
                Ok(SessionToolRunRow {
                    id: r.try_get(0)?,
                    session_id: r.try_get(1)?,
                    message_seq: r.try_get(2)?,
                    part_index: r.try_get(3)?,
                    tool_call_id: r.try_get(4)?,
                    runtime_id: r.try_get(5)?,
                    kind: r.try_get(6)?,
                    status: r.try_get(7)?,
                    input_json: r.try_get(8)?,
                    output_json: r.try_get(9)?,
                    duration_ms: r.try_get(10)?,
                    created_at_ms: r.try_get(11)?,
                })
            })
            .collect()
    }

    pub async fn record_retrieval_event(&self, event: &SessionRetrievalEvent) -> Result<()> {
        sqlx::query(
            "INSERT INTO session_retrieval_events \
             (id, session_id, query, source, result_count, payload, created_at_ms) \
             VALUES (?1,?2,?3,?4,?5,?6,?7)",
        )
        .bind(&event.id)
        .bind(&event.session_id)
        .bind(&event.query)
        .bind(&event.source)
        .bind(event.result_count)
        .bind(&event.payload)
        .bind(event.created_at_ms)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn list_session_retrieval_events(
        &self,
        session_id: &str,
    ) -> Result<Vec<SessionRetrievalEvent>> {
        let rows = sqlx::query(
            "SELECT id, session_id, query, source, result_count, payload, created_at_ms \
             FROM session_retrieval_events WHERE session_id = ?1 ORDER BY created_at_ms ASC",
        )
        .bind(session_id)
        .fetch_all(&self.pool)
        .await?;
        rows.iter()
            .map(|r| {
                Ok(SessionRetrievalEvent {
                    id: r.try_get(0)?,
                    session_id: r.try_get(1)?,
                    query: r.try_get(2)?,
                    source: r.try_get(3)?,
                    result_count: r.try_get(4)?,
                    payload: r.try_get(5)?,
                    created_at_ms: r.try_get(6)?,
                })
            })
            .collect()
    }

    pub async fn record_compaction(&self, compaction: &SessionCompactionRow) -> Result<()> {
        sqlx::query(
            "INSERT INTO session_compactions \
             (id, session_id, before_tokens, after_tokens, summary, payload, created_at_ms) \
             VALUES (?1,?2,?3,?4,?5,?6,?7)",
        )
        .bind(&compaction.id)
        .bind(&compaction.session_id)
        .bind(compaction.before_tokens)
        .bind(compaction.after_tokens)
        .bind(&compaction.summary)
        .bind(&compaction.payload)
        .bind(compaction.created_at_ms)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn record_session_finding(&self, finding: &SessionFindingRow) -> Result<()> {
        sqlx::query(
            "INSERT INTO session_findings \
             (id, session_id, kind, summary, evidence, status, created_at_ms, resolved_at_ms) \
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8)",
        )
        .bind(&finding.id)
        .bind(&finding.session_id)
        .bind(&finding.kind)
        .bind(&finding.summary)
        .bind(&finding.evidence)
        .bind(&finding.status)
        .bind(finding.created_at_ms)
        .bind(finding.resolved_at_ms)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn upsert_session_artifact(
        &self,
        session_id: &str,
        kind: &str,
        key: &str,
        value_json: &str,
    ) -> Result<()> {
        let now = record::now_ms();
        sqlx::query(
            "INSERT INTO session_artifacts \
             (session_id, kind, key, value_json, created_at_ms, updated_at_ms) \
             VALUES (?1,?2,?3,?4,?5,?5) \
             ON CONFLICT(session_id, kind, key) DO UPDATE SET \
                value_json=excluded.value_json, updated_at_ms=excluded.updated_at_ms",
        )
        .bind(session_id)
        .bind(kind)
        .bind(key)
        .bind(value_json)
        .bind(now)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn get_session_artifact(
        &self,
        session_id: &str,
        kind: &str,
        key: &str,
    ) -> Result<Option<SessionArtifactRow>> {
        sqlx::query(
            "SELECT session_id, kind, key, value_json, created_at_ms, updated_at_ms \
             FROM session_artifacts WHERE session_id = ?1 AND kind = ?2 AND key = ?3",
        )
        .bind(session_id)
        .bind(kind)
        .bind(key)
        .fetch_optional(&self.pool)
        .await?
        .as_ref()
        .map(session_artifact_from)
        .transpose()
    }

    pub async fn list_session_artifacts(
        &self,
        session_id: &str,
        kind: &str,
        limit: usize,
    ) -> Result<Vec<SessionArtifactRow>> {
        let rows = sqlx::query(
            "SELECT session_id, kind, key, value_json, created_at_ms, updated_at_ms \
             FROM session_artifacts WHERE session_id = ?1 AND kind = ?2 \
             ORDER BY updated_at_ms DESC LIMIT ?3",
        )
        .bind(session_id)
        .bind(kind)
        .bind(limit as i64)
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(session_artifact_from).collect()
    }

    pub async fn delete_session_artifact(
        &self,
        session_id: &str,
        kind: &str,
        key: &str,
    ) -> Result<usize> {
        let result = sqlx::query(
            "DELETE FROM session_artifacts WHERE session_id = ?1 AND kind = ?2 AND key = ?3",
        )
        .bind(session_id)
        .bind(kind)
        .bind(key)
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected() as usize)
    }

    pub async fn append_session_artifact_event(
        &self,
        session_id: &str,
        kind: &str,
        key: &str,
        value_json: &str,
    ) -> Result<i64> {
        let result = sqlx::query(
            "INSERT INTO session_artifact_events (session_id, kind, key, value_json, created_at_ms) \
             VALUES (?1,?2,?3,?4,?5)",
        )
        .bind(session_id)
        .bind(kind)
        .bind(key)
        .bind(value_json)
        .bind(record::now_ms())
        .execute(&self.pool)
        .await?;
        Ok(result.last_insert_rowid())
    }

    pub async fn list_session_artifact_events(
        &self,
        session_id: &str,
        kind: &str,
        key: Option<&str>,
        limit: usize,
    ) -> Result<Vec<SessionArtifactEventRow>> {
        let limit = limit as i64;
        let rows = if let Some(key) = key {
            sqlx::query(
                "SELECT id, session_id, kind, key, value_json, created_at_ms \
                 FROM session_artifact_events \
                 WHERE session_id = ?1 AND kind = ?2 AND key = ?3 \
                 ORDER BY id ASC LIMIT ?4",
            )
            .bind(session_id)
            .bind(kind)
            .bind(key)
            .bind(limit)
            .fetch_all(&self.pool)
            .await?
        } else {
            sqlx::query(
                "SELECT id, session_id, kind, key, value_json, created_at_ms \
                 FROM session_artifact_events \
                 WHERE session_id = ?1 AND kind = ?2 \
                 ORDER BY id ASC LIMIT ?3",
            )
            .bind(session_id)
            .bind(kind)
            .bind(limit)
            .fetch_all(&self.pool)
            .await?
        };
        rows.iter().map(session_artifact_event_from).collect()
    }

    pub async fn list_recent_session_artifact_events(
        &self,
        session_id: &str,
        kind: &str,
        key: Option<&str>,
        limit: usize,
    ) -> Result<Vec<SessionArtifactEventRow>> {
        let limit = limit as i64;
        let rows = if let Some(key) = key {
            sqlx::query(
                "SELECT id, session_id, kind, key, value_json, created_at_ms \
                 FROM session_artifact_events \
                 WHERE session_id = ?1 AND kind = ?2 AND key = ?3 \
                 ORDER BY id DESC LIMIT ?4",
            )
            .bind(session_id)
            .bind(kind)
            .bind(key)
            .bind(limit)
            .fetch_all(&self.pool)
            .await?
        } else {
            sqlx::query(
                "SELECT id, session_id, kind, key, value_json, created_at_ms \
                 FROM session_artifact_events \
                 WHERE session_id = ?1 AND kind = ?2 \
                 ORDER BY id DESC LIMIT ?3",
            )
            .bind(session_id)
            .bind(kind)
            .bind(limit)
            .fetch_all(&self.pool)
            .await?
        };
        let mut out: Vec<SessionArtifactEventRow> = rows
            .iter()
            .map(session_artifact_event_from)
            .collect::<Result<_>>()?;
        out.reverse();
        Ok(out)
    }

    pub async fn clear_session_artifact_events(
        &self,
        session_id: &str,
        kind: &str,
        key: Option<&str>,
    ) -> Result<usize> {
        let result = if let Some(key) = key {
            sqlx::query(
                "DELETE FROM session_artifact_events \
                 WHERE session_id = ?1 AND kind = ?2 AND key = ?3",
            )
            .bind(session_id)
            .bind(kind)
            .bind(key)
            .execute(&self.pool)
            .await?
        } else {
            sqlx::query("DELETE FROM session_artifact_events WHERE session_id = ?1 AND kind = ?2")
                .bind(session_id)
                .bind(kind)
                .execute(&self.pool)
                .await?
        };
        Ok(result.rows_affected() as usize)
    }

    /// Session ids whose transcript matches an FTS query (substring search path).
    pub async fn search_transcripts(&self, query: &str, limit: usize) -> Result<Vec<String>> {
        let terms = query
            .split_whitespace()
            .filter(|t| t.len() >= 2)
            .map(|t| format!("\"{}\"", t.replace('"', "")))
            .collect::<Vec<_>>()
            .join(" OR ");
        if terms.is_empty() {
            return Ok(Vec::new());
        }
        let rows = sqlx::query(
            "SELECT DISTINCT m.session_id FROM session_messages_fts f \
             JOIN session_messages m ON m.rowid = f.rowid \
             WHERE session_messages_fts MATCH ?1 LIMIT ?2",
        )
        .bind(&terms)
        .bind(limit as i64)
        .fetch_all(&self.pool)
        .await?;
        rows.iter()
            .map(|r| Ok(r.try_get::<String, _>(0)?))
            .collect()
    }

    /// Fast, consistent file-level backup (council decision 5: one DB is a single
    /// failure domain, so keep a recoverable snapshot). `VACUUM INTO` writes a
    /// fully-consistent copy without blocking writers for long.
    pub async fn backup_to(&self, path: &Path) -> Result<()> {
        // SQLite's `VACUUM INTO` does not accept a bound parameter for the target
        // path (the bind silently no-ops and no file is written), so the path is
        // inlined. Single-quotes are SQL-escaped to keep this injection-safe even
        // though the caller controls the path.
        // `VACUUM INTO` can't be a prepared statement; run it via `raw_sql`.
        let target = path.to_string_lossy().replace('\'', "''");
        sqlx::raw_sql(AssertSqlSafe(format!("VACUUM INTO '{target}'")))
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Whether an autonomous maintenance pass is due for `project_key` (no pass
    /// within `throttle_ms`). True on the first ever run.
    pub async fn maintain_due(&self, project_key: &str, throttle_ms: i64) -> Result<bool> {
        let last: Option<i64> =
            sqlx::query("SELECT last_run_ms FROM maintain_state WHERE project_key = ?1")
                .bind(project_key)
                .fetch_optional(&self.pool)
                .await?
                .map(|r| r.try_get(0))
                .transpose()?;
        Ok(match last {
            Some(ts) => record::now_ms() - ts >= throttle_ms,
            None => true,
        })
    }

    /// Record that a maintenance pass ran now for `project_key`.
    pub async fn stamp_maintain(&self, project_key: &str) -> Result<()> {
        sqlx::query(
            "INSERT INTO maintain_state (project_key, last_run_ms) VALUES (?1, ?2) \
             ON CONFLICT(project_key) DO UPDATE SET last_run_ms = ?2",
        )
        .bind(project_key)
        .bind(record::now_ms())
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Count of live (non-superseded) records — for tests/metrics.
    pub async fn live_count(&self) -> Result<i64> {
        Ok(
            sqlx::query("SELECT count(*) FROM knowledge WHERE superseded_by IS NULL")
                .fetch_one(&self.pool)
                .await?
                .try_get(0)?,
        )
    }
}

fn import_memory_level(item: &ImportableMemory) -> (MemLevel, Option<&str>) {
    match item.scope {
        Scope::User => (MemLevel::User, None),
        Scope::Project => (MemLevel::Project, item.project_key.as_deref()),
        Scope::Global => (MemLevel::External, None),
    }
}

fn import_memory_hash(item: &ImportableMemory) -> String {
    item.body
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

fn import_memory_meta_json(item: &ImportableMemory, hash: &str) -> String {
    let memory_type = match item.kind {
        Kind::Preference => "preference",
        Kind::Finding => "feedback",
        Kind::Fact | Kind::Skill | Kind::Convention => "context",
    };
    let memory_scope = if item.scope == Scope::Project {
        "team"
    } else {
        "private"
    };
    let source_path = item
        .source_path
        .as_ref()
        .map(|path| path.display().to_string());
    serde_json::json!({
        "type": memory_type,
        "scope": memory_scope,
        "normalized_hash": hash,
        "source_type": "legacy-import",
        "source_path": source_path,
    })
    .to_string()
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

/// One row of the primary session catalog.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionRow {
    pub id: String,
    pub cwd: Option<String>,
    pub model: Option<String>,
    pub created_at: Option<String>,
    pub updated_at: Option<String>,
    pub first_prompt: Option<String>,
    pub title: Option<String>,
    pub message_count: i64,
}

fn session_row_from(row: &SqliteRow) -> Result<SessionRow> {
    Ok(SessionRow {
        id: row.try_get(0)?,
        cwd: row.try_get(1)?,
        model: row.try_get(2)?,
        created_at: row.try_get(3)?,
        updated_at: row.try_get(4)?,
        first_prompt: row.try_get(5)?,
        title: row.try_get(6)?,
        message_count: row.try_get(7)?,
    })
}

/// One message of a session transcript (PLAN TODO 23). `meta` is opaque
/// serialized JSON (tool calls, parts, usage) the engine round-trips verbatim,
/// so the knowledge crate stays free of the engine's message types.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionMessage {
    pub seq: i64,
    pub role: String,
    pub content: String,
    pub meta: Option<String>,
}

/// A self-improvement backlog suggestion (input form). `scope` is `"self"`
/// (JFC's own reasoning/prompt/skill/tool) or `"project"` (an optimization for
/// another codebase, keyed by `project_key`).
#[derive(Debug, Clone)]
pub struct BacklogItem {
    pub scope: String,
    pub project_key: Option<String>,
    pub category: String,
    pub title: String,
    pub body: String,
    pub evidence: String,
    pub confidence: f64,
    pub source_session_id: Option<String>,
}

impl BacklogItem {
    /// Stable dedup id — the same suggestion re-proposed bumps `recurrence`
    /// rather than creating a new row.
    pub fn id(&self) -> String {
        uuid::Uuid::new_v5(
            &uuid::Uuid::NAMESPACE_OID,
            format!(
                "backlog:{}:{}:{}:{}",
                self.scope,
                self.project_key.as_deref().unwrap_or(""),
                self.category,
                self.title
            )
            .as_bytes(),
        )
        .simple()
        .to_string()
    }
}

/// A backlog row as read back (listing / review).
#[derive(Debug, Clone)]
pub struct BacklogRow {
    pub id: String,
    pub scope: String,
    pub project_key: Option<String>,
    pub category: String,
    pub title: String,
    pub body: String,
    pub status: String,
    pub confidence: f64,
    pub recurrence: i64,
    pub created_at_ms: i64,
    pub applied_at_ms: Option<i64>,
}

/// A `(scope, status) → count` metric row.
#[derive(Debug, Clone)]
pub struct BacklogMetricRow {
    pub scope: String,
    pub status: String,
    pub count: i64,
}

impl KnowledgeStore {
    /// Record a backlog suggestion. New rows land `proposed`; an identical
    /// suggestion (same id) bumps `recurrence` + `updated_at`, keeps the best
    /// confidence and existing status (evidence accrues). Returns `(id, is_new)`.
    pub async fn upsert_backlog_item(&self, item: &BacklogItem) -> Result<(String, bool)> {
        let id = item.id();
        let now = record::now_ms();
        let exists = sqlx::query("SELECT 1 FROM improvement_backlog WHERE id = ?1")
            .bind(&id)
            .fetch_optional(&self.pool)
            .await?
            .is_some();
        if exists {
            sqlx::query(
                "UPDATE improvement_backlog
                 SET recurrence = recurrence + 1, updated_at_ms = ?2,
                     confidence = MAX(confidence, ?3)
                 WHERE id = ?1",
            )
            .bind(&id)
            .bind(now)
            .bind(item.confidence)
            .execute(&self.pool)
            .await?;
            Ok((id, false))
        } else {
            sqlx::query(
                "INSERT INTO improvement_backlog
                   (id, scope, project_key, category, title, body, evidence,
                    status, confidence, source_session_id, recurrence,
                    created_at_ms, updated_at_ms)
                 VALUES (?1,?2,?3,?4,?5,?6,?7,'proposed',?8,?9,1,?10,?10)",
            )
            .bind(&id)
            .bind(&item.scope)
            .bind(&item.project_key)
            .bind(&item.category)
            .bind(&item.title)
            .bind(&item.body)
            .bind(&item.evidence)
            .bind(item.confidence)
            .bind(&item.source_session_id)
            .bind(now)
            .execute(&self.pool)
            .await?;
            Ok((id, true))
        }
    }

    /// Transition a backlog item's status (proven|applied|rejected|superseded);
    /// stamps `applied_at_ms` on `applied`.
    pub async fn set_backlog_status(&self, id: &str, status: &str) -> Result<()> {
        let now = record::now_ms();
        let applied = (status == "applied").then_some(now);
        sqlx::query(
            "UPDATE improvement_backlog
             SET status = ?2, updated_at_ms = ?3,
                 applied_at_ms = COALESCE(?4, applied_at_ms)
             WHERE id = ?1",
        )
        .bind(id)
        .bind(status)
        .bind(now)
        .bind(applied)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Backlog metrics: counts by `(scope, status)` — the queryable
    /// "is it improving itself" view.
    pub async fn backlog_metrics(&self) -> Result<Vec<BacklogMetricRow>> {
        let rows = sqlx::query(
            "SELECT scope, status, COUNT(*) FROM improvement_backlog
             GROUP BY scope, status ORDER BY scope, status",
        )
        .fetch_all(&self.pool)
        .await?;
        rows.iter()
            .map(|r| {
                Ok(BacklogMetricRow {
                    scope: r.try_get(0)?,
                    status: r.try_get(1)?,
                    count: r.try_get(2)?,
                })
            })
            .collect()
    }

    /// List backlog items, highest-evidence first, optionally filtered.
    pub async fn list_backlog(
        &self,
        scope: Option<&str>,
        status: Option<&str>,
        limit: i64,
    ) -> Result<Vec<BacklogRow>> {
        let rows = sqlx::query(
            "SELECT id, scope, project_key, category, title, body, status,
                    confidence, recurrence, created_at_ms, applied_at_ms
             FROM improvement_backlog
             WHERE (?1 IS NULL OR scope = ?1)
               AND (?2 IS NULL OR status = ?2)
             ORDER BY recurrence DESC, updated_at_ms DESC
             LIMIT ?3",
        )
        .bind(scope)
        .bind(status)
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;
        rows.iter()
            .map(|r| {
                Ok(BacklogRow {
                    id: r.try_get(0)?,
                    scope: r.try_get(1)?,
                    project_key: r.try_get(2)?,
                    category: r.try_get(3)?,
                    title: r.try_get(4)?,
                    body: r.try_get(5)?,
                    status: r.try_get(6)?,
                    confidence: r.try_get(7)?,
                    recurrence: r.try_get(8)?,
                    created_at_ms: r.try_get(9)?,
                    applied_at_ms: r.try_get(10)?,
                })
            })
            .collect()
    }

    /// Supersede harness-noise preferences (system reminders / continuation
    /// nudges mistakenly mined as user preferences) so they stop polluting
    /// recall. Reversible — sets `superseded_by`, doesn't delete. Returns count.
    pub async fn prune_noisy_preferences(&self) -> Result<u64> {
        let result = sqlx::query(
            "UPDATE knowledge SET superseded_by = 'pruned:harness_noise'
             WHERE kind = 'preference' AND superseded_by IS NULL
               AND ( body LIKE '%system-reminder%'
                  OR body LIKE '%/system-reminder%'
                  OR body LIKE '%Continue — do the next%'
                  OR body LIKE '%continue the remaining%'
                  OR body LIKE '%automated background-task%' )",
        )
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected())
    }

    /// Upsert a held-out eval case (the ground-truth fixture). Dedups by
    /// source+failure+prompt; an identical case bumps `weight` (importance).
    /// Returns `(id, is_new)`.
    pub async fn upsert_eval_case(&self, case: &EvalCaseInput) -> Result<(String, bool)> {
        let id = case.id();
        let now = record::now_ms();
        let existed = sqlx::query("SELECT 1 FROM eval_cases WHERE id = ?1")
            .bind(&id)
            .fetch_optional(&self.pool)
            .await?
            .is_some();
        if existed {
            sqlx::query("UPDATE eval_cases SET weight = weight + ?2 WHERE id = ?1")
                .bind(&id)
                .bind(case.weight)
                .execute(&self.pool)
                .await?;
            Ok((id, false))
        } else {
            sqlx::query(
                "INSERT INTO eval_cases
                   (id, source, prompt, failure_mode, expected, project_key,
                    source_session_id, weight, created_at_ms)
                 VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)",
            )
            .bind(&id)
            .bind(&case.source)
            .bind(&case.prompt)
            .bind(&case.failure_mode)
            .bind(&case.expected)
            .bind(&case.project_key)
            .bind(&case.source_session_id)
            .bind(case.weight)
            .bind(now)
            .execute(&self.pool)
            .await?;
            Ok((id, true))
        }
    }

    pub async fn eval_case_count(&self) -> Result<i64> {
        let row = sqlx::query("SELECT COUNT(*) FROM eval_cases")
            .fetch_one(&self.pool)
            .await?;
        Ok(row.try_get(0)?)
    }

    /// Record an eval-run result for a variant (`control` | `candidate:<def_id>`).
    pub async fn record_eval_run(
        &self,
        eval_id: &str,
        variant: &str,
        passed: Option<bool>,
        score: Option<f64>,
        detail: &str,
    ) -> Result<()> {
        let now = record::now_ms();
        let id = format!("evalrun:{eval_id}:{variant}:{now}");
        sqlx::query(
            "INSERT OR IGNORE INTO eval_runs
               (id, eval_id, variant, passed, score, detail, run_at_ms)
             VALUES (?1,?2,?3,?4,?5,?6,?7)",
        )
        .bind(&id)
        .bind(eval_id)
        .bind(variant)
        .bind(passed.map(|p| i64::from(p)))
        .bind(score)
        .bind(detail)
        .bind(now)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// `(total_runs, passed)` for a variant — the pass-rate signal.
    pub async fn eval_pass_rate(&self, variant: &str) -> Result<(i64, i64)> {
        let row = sqlx::query(
            "SELECT COUNT(*), COALESCE(SUM(passed), 0) FROM eval_runs WHERE variant = ?1",
        )
        .bind(variant)
        .fetch_one(&self.pool)
        .await?;
        Ok((row.try_get(0)?, row.try_get(1)?))
    }

    /// Score statistics for a variant — the NOISE CEILING. A single pass-rate
    /// hides whether a difference between variants is real or sampling noise;
    /// mean ± stddev over repeated runs is what tells them apart. Returns `None`
    /// when the variant has no scored runs.
    pub async fn eval_score_stats(&self, variant: &str) -> Result<Option<EvalScoreStats>> {
        let row = sqlx::query(
            "SELECT COUNT(score), AVG(score), AVG(score*score), MIN(score), MAX(score)
             FROM eval_runs WHERE variant = ?1 AND score IS NOT NULL",
        )
        .bind(variant)
        .fetch_one(&self.pool)
        .await?;
        let runs: i64 = row.try_get(0)?;
        if runs == 0 {
            return Ok(None);
        }
        let mean: f64 = row.try_get(1)?;
        let mean_sq: f64 = row.try_get(2)?;
        let min: f64 = row.try_get(3)?;
        let max: f64 = row.try_get(4)?;
        // Population variance = E[x^2] - E[x]^2, clamped at 0 for fp slop.
        let variance = (mean_sq - mean * mean).max(0.0);
        Ok(Some(EvalScoreStats {
            runs,
            mean,
            stddev: variance.sqrt(),
            min,
            max,
        }))
    }

    /// Whether the mean-score gap between two variants clears the noise band:
    /// `|mean_a - mean_b| >= k * pooled_stddev`. The Fan-talk lesson — a higher
    /// score isn't an improvement if it sits inside the noise. `k` is the number
    /// of pooled standard deviations to require (≈2 ≈ 95% under normality).
    /// Returns `None` if either variant lacks scored runs.
    pub async fn eval_difference_is_significant(
        &self,
        variant_a: &str,
        variant_b: &str,
        k: f64,
    ) -> Result<Option<EvalSignificance>> {
        let (Some(a), Some(b)) = (
            self.eval_score_stats(variant_a).await?,
            self.eval_score_stats(variant_b).await?,
        ) else {
            return Ok(None);
        };
        // Equal-weight pooled stddev; a degenerate 0 band would call an
        // exactly-equal pair "significant", so floor it at EPSILON.
        let pooled = ((a.stddev * a.stddev + b.stddev * b.stddev) / 2.0).sqrt();
        let delta = (a.mean - b.mean).abs();
        let band = (k * pooled).max(f64::EPSILON);
        Ok(Some(EvalSignificance {
            delta,
            band,
            significant: delta >= band,
            better: if a.mean >= b.mean { variant_a } else { variant_b }.to_owned(),
        }))
    }

    /// Record an error-pattern SIGNATURE for a failed eval run. `signature` is a
    /// `FailureKind` bucket (optionally `<kind>:<step>`), so two variants with
    /// the same pass-rate stay distinguishable by *how* they fail. Upserts the
    /// per-`(variant, signature)` count.
    pub async fn record_eval_error_signature(
        &self,
        variant: &str,
        eval_id: &str,
        signature: &str,
    ) -> Result<()> {
        let now = record::now_ms();
        let id = uuid::Uuid::new_v5(
            &uuid::Uuid::NAMESPACE_OID,
            format!("errsig:{variant}:{eval_id}:{signature}").as_bytes(),
        )
        .simple()
        .to_string();
        sqlx::query(
            "INSERT INTO eval_error_signatures
               (id, variant, eval_id, signature, count, first_seen_ms, last_seen_ms)
             VALUES (?1,?2,?3,?4,1,?5,?5)
             ON CONFLICT(id) DO UPDATE SET count = count + 1, last_seen_ms = ?5",
        )
        .bind(&id)
        .bind(variant)
        .bind(eval_id)
        .bind(signature)
        .bind(now)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// The error-pattern DISTRIBUTION for a variant — `(signature, count)`
    /// descending. The comparison surface: two variants with equal pass-rates
    /// whose distributions differ are NOT the same; the shift says what changed.
    pub async fn eval_error_distribution(&self, variant: &str) -> Result<Vec<(String, i64)>> {
        let rows = sqlx::query(
            "SELECT signature, SUM(count) FROM eval_error_signatures
             WHERE variant = ?1 GROUP BY signature ORDER BY SUM(count) DESC",
        )
        .bind(variant)
        .fetch_all(&self.pool)
        .await?;
        rows.iter()
            .map(|r| Ok((r.try_get(0)?, r.try_get(1)?)))
            .collect()
    }

    /// Record a knowledge gap (what it doesn't know). Bumps `ref_count` on an
    /// existing label so recurring gaps rise to the top of the curriculum.
    pub async fn record_knowledge_gap(&self, label: &str, reason: &str) -> Result<()> {
        let now = record::now_ms();
        let id = uuid::Uuid::new_v5(&uuid::Uuid::NAMESPACE_OID, format!("gap:{label}").as_bytes())
            .simple()
            .to_string();
        sqlx::query(
            "INSERT INTO knowledge_gaps (id, label, reason, ref_count, first_seen_ms, last_seen_ms)
             VALUES (?1,?2,?3,1,?4,?4)
             ON CONFLICT(id) DO UPDATE SET ref_count = ref_count + 1, last_seen_ms = ?4",
        )
        .bind(&id)
        .bind(label)
        .bind(reason)
        .bind(now)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Top open knowledge gaps (`label`, `ref_count`) — the self-directed
    /// curriculum: what it should go learn.
    pub async fn open_knowledge_gaps(&self, limit: i64) -> Result<Vec<(String, i64)>> {
        let rows = sqlx::query(
            "SELECT label, ref_count FROM knowledge_gaps
             WHERE resolved_by IS NULL ORDER BY ref_count DESC LIMIT ?1",
        )
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(|r| Ok((r.try_get(0)?, r.try_get(1)?))).collect()
    }

    /// CONSOLIDATE: supersede exact-duplicate-body preferences, keeping the
    /// most-used one per body. Collapses the "bigger, not stronger" mush so
    /// recall surfaces one representative instead of N near-identical rows.
    /// Reversible (`superseded_by = 'consolidated'`). Returns rows collapsed.
    pub async fn consolidate_duplicate_preferences(&self) -> Result<u64> {
        let result = sqlx::query(
            "UPDATE knowledge SET superseded_by = 'consolidated'
             WHERE kind = 'preference' AND superseded_by IS NULL
               AND body IN (
                 SELECT body FROM knowledge
                 WHERE kind = 'preference' AND superseded_by IS NULL
                 GROUP BY body HAVING COUNT(*) > 1
               )
               AND id NOT IN (
                 SELECT id FROM (
                   SELECT id, ROW_NUMBER() OVER (
                     PARTITION BY body ORDER BY use_count DESC, id ASC
                   ) AS rn
                   FROM knowledge WHERE kind = 'preference' AND superseded_by IS NULL
                 ) WHERE rn = 1
               )",
        )
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected())
    }

    /// Seed the eval suite from the distilled `finding` lessons — each known
    /// failure mode becomes a held-out regression fixture (scenario = the error
    /// class, expected = the lesson). Deterministic + idempotent (id = `eval:`+
    /// knowledge id). Returns the number of new cases added.
    pub async fn seed_eval_cases_from_findings(&self) -> Result<u64> {
        let now = record::now_ms();
        let result = sqlx::query(
            "INSERT OR IGNORE INTO eval_cases
               (id, source, prompt, failure_mode, expected, weight, created_at_ms)
             SELECT 'eval:' || id, 'tool_failure', title, title, body,
                    CAST(use_count AS REAL) + 1.0, ?1
             FROM knowledge
             WHERE kind = 'finding' AND superseded_by IS NULL",
        )
        .bind(now)
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected())
    }

    /// Seed the self-directed curriculum: record a knowledge gap for every tool
    /// that fails repeatedly (the agent demonstrably struggles with it). The
    /// `open_knowledge_gaps` list is then "what to go learn." Idempotent.
    pub async fn seed_knowledge_gaps_from_failures(&self, min_failures: i64) -> Result<u64> {
        let now = record::now_ms();
        let result = sqlx::query(
            "INSERT INTO knowledge_gaps
               (id, label, reason, ref_count, first_seen_ms, last_seen_ms)
             SELECT 'gap:tool:' || kind,
                    'reliability of the ' || kind || ' tool',
                    'recurring failures across sessions',
                    COUNT(*), ?1, ?1
             FROM session_tool_runs
             WHERE status = 'failed'
             GROUP BY kind HAVING COUNT(*) >= ?2
             ON CONFLICT(id) DO UPDATE SET ref_count = excluded.ref_count, last_seen_ms = ?1",
        )
        .bind(now)
        .bind(min_failures)
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected())
    }

    /// IMPROVEMENT: promote verified, project-scoped `finding` lessons to GLOBAL
    /// scope. A verified error-lesson ("re-read the exact bytes before retrying
    /// Edit") is universal, so scoping it to one repo makes it invisible to
    /// cross-project recall. Promoting makes the learned lessons actually
    /// recallable everywhere. Returns the number promoted.
    pub async fn promote_verified_findings_to_global(&self) -> Result<u64> {
        let result = sqlx::query(
            "UPDATE knowledge SET scope = 'global', project_key = NULL, promoted = 1
             WHERE kind = 'finding' AND outcome = 'verified'
               AND scope = 'project' AND superseded_by IS NULL",
        )
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected())
    }

    /// Deterministic eval RUN (no model needed): for each known-failure eval
    /// case, does the recall engine actually surface its lesson? Fills
    /// `eval_runs` (variant `recall_coverage`) and returns `(total, passed)` —
    /// a real "are the learned lessons actually recallable" measurement, which
    /// is exactly what proves the loop is *useful*, not just busy.
    pub async fn run_recall_coverage_eval(&self) -> Result<(i64, i64)> {
        let cases = sqlx::query("SELECT id, prompt FROM eval_cases")
            .fetch_all(&self.pool)
            .await?;
        let mut total = 0i64;
        let mut passed = 0i64;
        for row in &cases {
            let id: String = row.try_get(0)?;
            let prompt: String = row.try_get(1)?;
            let filter = RecallFilter {
                project_key: None,
                limit: 8,
            };
            let hits = self.recall(&prompt, &filter).await.unwrap_or_default();
            let covered = hits
                .iter()
                .any(|h| h.title.trim().eq_ignore_ascii_case(prompt.trim()));
            total += 1;
            if covered {
                passed += 1;
            }
            self.record_eval_run(
                &id,
                "recall_coverage",
                Some(covered),
                Some(if covered { 1.0 } else { 0.0 }),
                "",
            )
            .await?;
        }
        Ok((total, passed))
    }
}

/// Input form for a held-out eval case (regression fixture).
#[derive(Debug, Clone)]
pub struct EvalCaseInput {
    pub source: String,
    pub prompt: String,
    pub failure_mode: Option<String>,
    pub expected: Option<String>,
    pub project_key: Option<String>,
    pub source_session_id: Option<String>,
    pub weight: f64,
}

/// Score statistics for a variant's eval runs — the noise ceiling.
/// See [`KnowledgeStore::eval_score_stats`].
#[derive(Debug, Clone, PartialEq)]
pub struct EvalScoreStats {
    pub runs: i64,
    pub mean: f64,
    pub stddev: f64,
    pub min: f64,
    pub max: f64,
}

/// Result of comparing two variants' mean scores against the noise band.
/// See [`KnowledgeStore::eval_difference_is_significant`].
#[derive(Debug, Clone, PartialEq)]
pub struct EvalSignificance {
    /// `|mean_a - mean_b|`.
    pub delta: f64,
    /// `k * pooled_stddev` — the band the delta must clear to count as real.
    pub band: f64,
    /// Whether `delta >= band`.
    pub significant: bool,
    /// The higher-mean variant (only meaningful when `significant`).
    pub better: String,
}

impl EvalCaseInput {
    /// Stable dedup id (same scenario re-seeded bumps weight, not a new row).
    pub fn id(&self) -> String {
        uuid::Uuid::new_v5(
            &uuid::Uuid::NAMESPACE_OID,
            format!(
                "eval:{}:{}:{}",
                self.source,
                self.failure_mode.as_deref().unwrap_or(""),
                self.prompt
            )
            .as_bytes(),
        )
        .simple()
        .to_string()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionEventRow {
    pub id: String,
    pub session_id: String,
    pub seq: i64,
    pub kind: String,
    pub created_at_ms: i64,
    pub payload: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionTurnRow {
    pub session_id: String,
    pub turn_index: i64,
    pub user_seq: Option<i64>,
    pub assistant_seq: Option<i64>,
    pub user_text: String,
    pub assistant_text: String,
    pub status: String,
    pub model: Option<String>,
    pub created_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionToolRunRow {
    pub id: String,
    pub session_id: String,
    pub message_seq: i64,
    pub part_index: i64,
    pub tool_call_id: Option<String>,
    pub runtime_id: Option<String>,
    pub kind: String,
    pub status: String,
    pub input_json: Option<String>,
    pub output_json: Option<String>,
    pub duration_ms: Option<i64>,
    pub created_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionRetrievalEvent {
    pub id: String,
    pub session_id: String,
    pub query: String,
    pub source: String,
    pub result_count: i64,
    pub payload: String,
    pub created_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionCompactionRow {
    pub id: String,
    pub session_id: String,
    pub before_tokens: Option<i64>,
    pub after_tokens: Option<i64>,
    pub summary: String,
    pub payload: String,
    pub created_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionFindingRow {
    pub id: String,
    pub session_id: String,
    pub kind: String,
    pub summary: String,
    pub evidence: String,
    pub status: String,
    pub created_at_ms: i64,
    pub resolved_at_ms: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionArtifactRow {
    pub session_id: String,
    pub kind: String,
    pub key: String,
    pub value_json: String,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionArtifactEventRow {
    pub id: i64,
    pub session_id: String,
    pub kind: String,
    pub key: String,
    pub value_json: String,
    pub created_at_ms: i64,
}

fn session_artifact_from(row: &SqliteRow) -> Result<SessionArtifactRow> {
    Ok(SessionArtifactRow {
        session_id: row.try_get(0)?,
        kind: row.try_get(1)?,
        key: row.try_get(2)?,
        value_json: row.try_get(3)?,
        created_at_ms: row.try_get(4)?,
        updated_at_ms: row.try_get(5)?,
    })
}

fn session_artifact_event_from(row: &SqliteRow) -> Result<SessionArtifactEventRow> {
    Ok(SessionArtifactEventRow {
        id: row.try_get(0)?,
        session_id: row.try_get(1)?,
        kind: row.try_get(2)?,
        key: row.try_get(3)?,
        value_json: row.try_get(4)?,
        created_at_ms: row.try_get(5)?,
    })
}

async fn insert_derived_session_rows(
    tx: &mut sqlx::sqlite::SqliteConnection,
    row: &SessionRow,
    messages: &[SessionMessage],
) -> Result<()> {
    let created_at_ms = record::now_ms();
    insert_session_events(tx, row, messages, created_at_ms).await?;
    insert_session_turns(tx, row, messages, created_at_ms).await?;
    insert_session_tool_runs(tx, row, messages, created_at_ms).await?;
    Ok(())
}

async fn insert_session_events(
    tx: &mut sqlx::sqlite::SqliteConnection,
    row: &SessionRow,
    messages: &[SessionMessage],
    created_at_ms: i64,
) -> Result<()> {
    for message in messages {
        let id = deterministic_session_row_id("message", &row.id, message.seq, 0);
        let kind = format!("message:{}", message.role);
        let payload = session_event_payload(message);
        sqlx::query(
            "INSERT INTO session_events (id, session_id, seq, kind, created_at_ms, payload) \
             VALUES (?1,?2,?3,?4,?5,?6)",
        )
        .bind(id)
        .bind(&row.id)
        .bind(message.seq)
        .bind(kind)
        .bind(created_at_ms)
        .bind(payload)
        .execute(&mut *tx)
        .await?;
    }
    Ok(())
}

async fn insert_session_turns(
    tx: &mut sqlx::sqlite::SqliteConnection,
    row: &SessionRow,
    messages: &[SessionMessage],
    created_at_ms: i64,
) -> Result<()> {
    const TURN_SQL: &str = "INSERT INTO session_turns \
         (session_id, turn_index, user_seq, assistant_seq, user_text, assistant_text, status, \
          model, created_at_ms) \
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)";
    let mut pending_user: Option<(i64, String)> = None;
    let mut turn_index = 0_i64;
    for message in messages {
        match message.role.as_str() {
            "user" => {
                if let Some((seq, text)) = pending_user.take() {
                    sqlx::query(TURN_SQL)
                        .bind(&row.id)
                        .bind(turn_index)
                        .bind(seq)
                        .bind(Option::<i64>::None)
                        .bind(text)
                        .bind("")
                        .bind("open")
                        .bind(&row.model)
                        .bind(created_at_ms)
                        .execute(&mut *tx)
                        .await?;
                    turn_index += 1;
                }
                pending_user = Some((message.seq, message.content.clone()));
            }
            "assistant" => {
                if let Some((seq, text)) = pending_user.take() {
                    sqlx::query(TURN_SQL)
                        .bind(&row.id)
                        .bind(turn_index)
                        .bind(seq)
                        .bind(message.seq)
                        .bind(text)
                        .bind(&message.content)
                        .bind("complete")
                        .bind(&row.model)
                        .bind(created_at_ms)
                        .execute(&mut *tx)
                        .await?;
                    turn_index += 1;
                }
            }
            _ => {}
        }
    }
    if let Some((seq, text)) = pending_user.take() {
        sqlx::query(TURN_SQL)
            .bind(&row.id)
            .bind(turn_index)
            .bind(seq)
            .bind(Option::<i64>::None)
            .bind(text)
            .bind("")
            .bind("open")
            .bind(&row.model)
            .bind(created_at_ms)
            .execute(&mut *tx)
            .await?;
    }
    Ok(())
}

async fn insert_session_tool_runs(
    tx: &mut sqlx::sqlite::SqliteConnection,
    row: &SessionRow,
    messages: &[SessionMessage],
    created_at_ms: i64,
) -> Result<()> {
    const TOOL_SQL: &str = "INSERT INTO session_tool_runs \
         (id, session_id, message_seq, part_index, tool_call_id, runtime_id, kind, status, \
          input_json, output_json, duration_ms, created_at_ms) \
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12)";
    const EVENT_SQL: &str = "INSERT INTO session_events (id, session_id, seq, kind, created_at_ms, payload) \
         VALUES (?1,?2,?3,?4,?5,?6)";
    for message in messages {
        let Some(meta) = message.meta.as_deref().and_then(parse_json) else {
            continue;
        };
        for (part_index, part) in tool_parts(&meta).into_iter().enumerate() {
            let Some(tool) = tool_value(part) else {
                continue;
            };
            let part_index = part_index as i64;
            let id = deterministic_session_row_id("tool", &row.id, message.seq, part_index);
            let kind = tool_string(tool, "kind").unwrap_or_else(|| "unknown".to_owned());
            let status = tool_string(tool, "status").unwrap_or_else(|| "unknown".to_owned());
            let input = tool.get("input");
            let output = tool.get("output");
            let input_json = input.map(serde_json::Value::to_string);
            let output_json = output.map(serde_json::Value::to_string);
            let tool_call_id = tool_string(tool, "id");
            let runtime_id = runtime_id_from_tool(tool);
            let duration_ms = tool
                .get("elapsed_ms")
                .or_else(|| output.and_then(|v| v.get("elapsed_ms")))
                .and_then(serde_json::Value::as_i64);
            sqlx::query(TOOL_SQL)
                .bind(id)
                .bind(&row.id)
                .bind(message.seq)
                .bind(part_index)
                .bind(tool_call_id)
                .bind(runtime_id)
                .bind(kind)
                .bind(status)
                .bind(input_json)
                .bind(output_json)
                .bind(duration_ms)
                .bind(created_at_ms)
                .execute(&mut *tx)
                .await?;
            let event_id =
                deterministic_session_row_id("tool_event", &row.id, message.seq, part_index);
            sqlx::query(EVENT_SQL)
                .bind(event_id)
                .bind(&row.id)
                .bind(message.seq)
                .bind("tool_run")
                .bind(created_at_ms)
                .bind(tool.to_string())
                .execute(&mut *tx)
                .await?;
        }
    }
    Ok(())
}

fn deterministic_session_row_id(prefix: &str, session_id: &str, seq: i64, index: i64) -> String {
    uuid::Uuid::new_v5(
        &uuid::Uuid::NAMESPACE_OID,
        format!("{prefix}:{session_id}:{seq}:{index}").as_bytes(),
    )
    .simple()
    .to_string()
}

fn session_event_payload(message: &SessionMessage) -> String {
    let meta = message.meta.as_deref().and_then(parse_json);
    let payload = serde_json::json!({
        "role": message.role,
        "content": message.content,
        "meta": meta,
    });
    payload.to_string()
}

fn parse_json(raw: &str) -> Option<serde_json::Value> {
    serde_json::from_str(raw).ok()
}

fn tool_parts(meta: &serde_json::Value) -> Vec<&serde_json::Value> {
    if let Some(parts) = meta.get("parts").and_then(serde_json::Value::as_array) {
        return parts.iter().collect();
    }
    if meta.get("type").and_then(serde_json::Value::as_str) == Some("tool")
        || meta.get("tool").is_some()
    {
        return vec![meta];
    }
    Vec::new()
}

fn tool_value(part: &serde_json::Value) -> Option<&serde_json::Value> {
    if part.get("type").and_then(serde_json::Value::as_str) == Some("tool") {
        return Some(part.get("tool").unwrap_or(part));
    }
    part.get("tool")
}

fn tool_string(tool: &serde_json::Value, key: &str) -> Option<String> {
    tool.get(key)
        .and_then(serde_json::Value::as_str)
        .map(ToOwned::to_owned)
}

fn runtime_id_from_tool(tool: &serde_json::Value) -> Option<String> {
    ["runtime_id", "task_id", "taskId"]
        .into_iter()
        .find_map(|key| tool_string(tool, key))
        .or_else(|| {
            tool.get("input").and_then(|input| {
                ["runtime_id", "task_id", "taskId"]
                    .into_iter()
                    .find_map(|key| tool_string(input, key))
            })
        })
        .or_else(|| {
            tool.get("output").and_then(|output| {
                ["runtime_id", "task_id", "taskId"]
                    .into_iter()
                    .find_map(|key| tool_string(output, key))
            })
        })
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
/// session history into project lessons, (3) consolidates duplicates, and (4)
/// auto-promotes verified, repeated, generalizable lessons. Growth is
/// **unbounded** — no decay/forget here.
/// Redaction (in mining) and the recall-time injection screen still apply; those
/// protect the user's secrets and can't be "expansion", so they stay.
///
/// **Throttled**: if a pass ran within [`MAINTAIN_THROTTLE_MS`], this is a no-op
/// (returns a zero report) so startup never re-processes the whole corpus. Pass
/// `force = true` (the `/knowledge mine` command) to bypass.
///
/// `sessions_dir` is kept for caller compatibility with the old migration
/// surface; DB-only mining ignores it.
pub async fn auto_maintain(
    project_root: &Path,
    sessions_dir: Option<&Path>,
    user_memory_dir: Option<&Path>,
    project_memory_dir: Option<&Path>,
) -> Result<MaintainReport> {
    auto_maintain_inner(
        project_root,
        sessions_dir,
        user_memory_dir,
        project_memory_dir,
        false,
    )
    .await
}

/// Like [`auto_maintain`] but `force` bypasses the throttle (manual `/knowledge
/// mine`).
pub async fn auto_maintain_forced(
    project_root: &Path,
    sessions_dir: Option<&Path>,
    user_memory_dir: Option<&Path>,
    project_memory_dir: Option<&Path>,
) -> Result<MaintainReport> {
    auto_maintain_inner(
        project_root,
        sessions_dir,
        user_memory_dir,
        project_memory_dir,
        true,
    )
    .await
}

async fn auto_maintain_inner(
    project_root: &Path,
    _sessions_dir: Option<&Path>,
    user_memory_dir: Option<&Path>,
    project_memory_dir: Option<&Path>,
    force: bool,
) -> Result<MaintainReport> {
    let store = KnowledgeStore::open_default().await?;
    let project = project::project_key(project_root);
    let mut report = MaintainReport::default();

    // Throttle: skip if a pass ran recently (per-project stamp), unless forced.
    if !force && !store.maintain_due(&project, MAINTAIN_THROTTLE_MS).await? {
        return Ok(report);
    }
    store.stamp_maintain(&project).await?;

    // 1. Import legacy .md memories (idempotent; never deletes sources).
    let mut items = Vec::new();
    if let Some(dir) = user_memory_dir {
        items.extend(import::scan_markdown_dir(dir, Scope::User, None));
    }
    if let Some(dir) = project_memory_dir {
        items.extend(import::scan_markdown_dir(
            dir,
            Scope::Project,
            Some(project.clone()),
        ));
    }
    if !items.is_empty() {
        report.imported = store.import_memories(&items).await?.imported;
    }

    // 2. Mine DB-backed session history.
    let (lessons, mine_report) = session_mine::mine_store(&store, 10_000).await;
    report.sessions_scanned = mine_report.sessions_scanned;
    let (ins, comp) = store.ingest_mined(&project, &lessons).await?;
    report.mined_inserted = ins;
    report.mined_compounded = comp;

    // 3. Consolidate duplicates (dedup only — no decay; the store grows).
    report.consolidated = store.consolidate().await?;
    report.auto_promoted = store.auto_promote(DEFAULT_AUTO_PROMOTE_SUPPORT).await?;

    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Mutex, MutexGuard, OnceLock};

    fn rec(scope: Scope, project: Option<&str>, title: &str, body: &str) -> KnowledgeRecord {
        KnowledgeRecord::new(Kind::Fact, scope, project.map(str::to_owned), title, body)
    }

    #[tokio::test]
    async fn insert_and_recall_round_trip_normal() {
        let store = KnowledgeStore::open_in_memory().await.unwrap();
        let r = rec(
            Scope::Global,
            None,
            "Rust edition",
            "This workspace uses edition 2024",
        )
        .with_confidence(0.9);
        store.insert(&r).await.unwrap();

        let hits = store
            .recall("edition", &RecallFilter::default())
            .await
            .unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].id, r.id);
        assert_eq!(hits[0].title, "Rust edition");
    }

    // SAFETY INVARIANT (PLAN §3.3): a record reaches global scope ONLY via the
    // explicit promote() gate, never via insert at runtime.
    #[tokio::test]
    async fn project_record_is_not_global_until_promoted_regression() {
        let store = KnowledgeStore::open_in_memory().await.unwrap();
        let r = rec(
            Scope::Project,
            Some("projA"),
            "local lesson",
            "use ripgrep here",
        );
        store.insert(&r).await.unwrap();

        // From a DIFFERENT project, the project-scoped row must NOT be recalled.
        let other = RecallFilter {
            project_key: Some("projB"),
            limit: 8,
        };
        assert!(store.recall("ripgrep", &other).await.unwrap().is_empty());

        // After human-gated promotion, it becomes visible everywhere.
        assert!(store.promote(&r.id).await.unwrap());
        assert_eq!(store.recall("ripgrep", &other).await.unwrap().len(), 1);
    }

    // SAFETY INVARIANT (PLAN §3): project rows for THIS project are visible;
    // user/global always visible; other projects' rows never.
    #[tokio::test]
    async fn recall_scope_isolation_normal() {
        let store = KnowledgeStore::open_in_memory().await.unwrap();
        store
            .insert(&rec(Scope::Project, Some("A"), "a-only", "alpha secret"))
            .await
            .unwrap();
        store
            .insert(&rec(Scope::Project, Some("B"), "b-only", "beta secret"))
            .await
            .unwrap();
        store
            .insert(&rec(Scope::User, None, "user-pref", "alpha beta gamma"))
            .await
            .unwrap();

        let from_a = RecallFilter {
            project_key: Some("A"),
            limit: 8,
        };
        let hits = store.recall("alpha", &from_a).await.unwrap();
        let titles: Vec<_> = hits.iter().map(|h| h.title.as_str()).collect();
        assert!(titles.contains(&"a-only"), "{titles:?}");
        assert!(titles.contains(&"user-pref"), "{titles:?}");
        assert!(
            !titles.contains(&"b-only"),
            "B's row leaked into A: {titles:?}"
        );
    }

    #[tokio::test]
    async fn supersede_hides_old_row_normal() {
        let store = KnowledgeStore::open_in_memory().await.unwrap();
        let old = rec(Scope::Global, None, "stack", "uses webpack");
        store.insert(&old).await.unwrap();
        let new = rec(Scope::Global, None, "stack", "uses vite now");
        store.insert(&new).await.unwrap();
        store.supersede(&old.id, &new.id).await.unwrap();

        let hits = store
            .recall("stack", &RecallFilter::default())
            .await
            .unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].id, new.id, "stale row should be filtered out");
    }

    #[tokio::test]
    async fn insert_rejects_invalid_records_robust() {
        let store = KnowledgeStore::open_in_memory().await.unwrap();
        assert!(
            store
                .insert(&rec(Scope::Global, None, "t", "  "))
                .await
                .is_err()
        );
        assert!(
            store
                .insert(&rec(Scope::Project, None, "t", "b"))
                .await
                .is_err()
        );
        assert!(
            store
                .insert(&rec(Scope::Global, Some("x"), "t", "b"))
                .await
                .is_err()
        );
        let mut bad = rec(Scope::Global, None, "t", "b");
        bad.confidence = 5.0;
        assert!(store.insert(&bad).await.is_err());
    }

    #[tokio::test]
    async fn insert_rejects_unredacted_secrets_before_sqlite_regression() {
        let store = KnowledgeStore::open_in_memory().await.unwrap();
        let raw_secret = rec(
            Scope::User,
            None,
            "credential",
            "token=ghp_0123456789abcdefghij",
        );

        assert!(store.insert(&raw_secret).await.is_err());
        assert_eq!(store.live_count().await.unwrap(), 0);
    }

    // SAFETY INVARIANT (PLAN §3.4): bounded growth. Insert well past the cap and
    // assert decay holds the live count at/under it, and that a promoted/global
    // row survives the cull.
    #[tokio::test]
    async fn decay_enforces_row_cap_and_spares_promoted_regression() {
        let store = KnowledgeStore::open_in_memory().await.unwrap();
        let mut keep = rec(Scope::Global, None, "promoted keeper", "must survive decay");
        keep.promoted = true;
        store.insert(&keep).await.unwrap();

        for i in 0..50 {
            store
                .insert(&rec(
                    Scope::Project,
                    Some("P"),
                    &format!("row {i}"),
                    "filler body",
                ))
                .await
                .unwrap();
        }
        for i in 0..50 {
            store
                .insert(&rec(
                    Scope::Global,
                    None,
                    &format!("global row {i}"),
                    "global filler body",
                ))
                .await
                .unwrap();
        }
        assert_eq!(store.live_count().await.unwrap(), 101);

        let removed = store.decay(DEFAULT_MAX_AGE_MS, 10).await.unwrap();
        assert!(
            removed >= 80,
            "decay should prune project/global rows over the cap; removed={removed}"
        );
        assert!(store.live_count().await.unwrap() <= 21);

        let hits = store
            .recall("keeper", &RecallFilter::default())
            .await
            .unwrap();
        assert_eq!(hits.len(), 1);
    }

    #[tokio::test]
    async fn mark_used_bumps_usage_and_influences_rank_normal() {
        let store = KnowledgeStore::open_in_memory().await.unwrap();
        let a = rec(Scope::Global, None, "shared term apple", "alpha").with_confidence(0.5);
        let b = rec(Scope::Global, None, "shared term apple", "beta").with_confidence(0.5);
        store.insert(&a).await.unwrap();
        store.insert(&b).await.unwrap();
        // Use `a` repeatedly → it should rank first on the next recall.
        for _ in 0..5 {
            store.mark_used(std::slice::from_ref(&a.id)).await.unwrap();
        }
        let hits = store
            .recall("apple", &RecallFilter::default())
            .await
            .unwrap();
        assert_eq!(hits.first().map(|h| h.id.as_str()), Some(a.id.as_str()));
    }

    // SAFETY INVARIANT (PLAN §F3): import is idempotent — re-running adds rows
    // once. Mirrors the legacy `.md` → DB migration's no-deletion contract.
    #[tokio::test]
    async fn import_memories_is_idempotent_regression() {
        use crate::import::ImportableMemory;
        let store = KnowledgeStore::open_in_memory().await.unwrap();
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

        let r1 = store.import_memories(&items).await.unwrap();
        assert_eq!(r1.imported, 2);
        assert_eq!(r1.skipped, 0);
        assert_eq!(store.live_count().await.unwrap(), 2);
        assert_eq!(store.memory_count().await.unwrap(), 2);
        let rows = store.load_memories(Some("P")).await.unwrap();
        assert_eq!(rows.len(), 2);
        let project_row = rows
            .iter()
            .find(|row| row.body == "This repo is edition 2024.")
            .unwrap();
        assert_eq!(project_row.level, MemLevel::Project);
        let meta: serde_json::Value =
            serde_json::from_str(project_row.meta.as_deref().unwrap()).unwrap();
        assert_eq!(meta["type"], "context");
        assert_eq!(meta["scope"], "team");

        // Second run: same content → all skipped, no duplicates.
        let r2 = store.import_memories(&items).await.unwrap();
        assert_eq!(r2.imported, 0);
        assert_eq!(r2.skipped, 2);
        assert_eq!(
            store.live_count().await.unwrap(),
            2,
            "re-import must not duplicate"
        );
    }

    #[tokio::test]
    async fn imported_project_memory_is_hidden_from_other_projects_regression() {
        use crate::import::ImportableMemory;
        let store = KnowledgeStore::open_in_memory().await.unwrap();
        let items = vec![ImportableMemory {
            source_path: None,
            kind: Kind::Fact,
            scope: Scope::Project,
            project_key: Some("P".into()),
            title: "stack".into(),
            body: "This repo uses ratatui.".into(),
        }];

        let report = store.import_memories(&items).await.unwrap();

        assert_eq!(report.imported, 1);
        assert_eq!(store.load_memories(Some("P")).await.unwrap().len(), 1);
        assert!(
            store.load_memories(Some("Q")).await.unwrap().is_empty(),
            "project-scoped imported memories must not leak into unrelated projects"
        );
    }

    // TODO 7+8: a verified, salient lesson outranks an unverified one on equal
    // lexical relevance.
    #[tokio::test]
    async fn verified_lesson_outranks_unverified_normal() {
        let store = KnowledgeStore::open_in_memory().await.unwrap();
        let weak = rec(Scope::Global, None, "edit term apple", "alpha")
            .with_confidence(0.5)
            .with_importance(0.5);
        let strong = rec(Scope::Global, None, "edit term apple", "beta")
            .with_confidence(0.5)
            .with_importance(0.5)
            .with_outcome(Outcome::Verified);
        store.insert(&weak).await.unwrap();
        store.insert(&strong).await.unwrap();
        let hits = store
            .recall("apple", &RecallFilter::default())
            .await
            .unwrap();
        assert_eq!(
            hits.first().map(|h| h.id.as_str()),
            Some(strong.id.as_str()),
            "verified lesson must rank first"
        );
    }

    // TODO 14: typed links + recall expansion + backlinks.
    #[tokio::test]
    async fn typed_links_and_backlinks_normal() {
        let store = KnowledgeStore::open_in_memory().await.unwrap();
        let err = rec(Scope::Global, None, "error", "old_string not found");
        let fix = rec(
            Scope::Global,
            None,
            "fix",
            "strip the line-number gutter first",
        );
        store.insert(&err).await.unwrap();
        store.insert(&fix).await.unwrap();
        store
            .link(&err.id, &fix.id, RelKind::FixedBy)
            .await
            .unwrap();

        let linked = store.linked(&err.id).await.unwrap();
        assert_eq!(linked.len(), 1);
        assert_eq!(linked[0].rel, RelKind::FixedBy);
        assert_eq!(linked[0].record.id, fix.id);

        assert_eq!(
            store.backlinks(&fix.id).await.unwrap(),
            vec![err.id.clone()]
        );
        // Idempotent: re-linking the same edge doesn't duplicate.
        store
            .link(&err.id, &fix.id, RelKind::FixedBy)
            .await
            .unwrap();
        assert_eq!(store.linked(&err.id).await.unwrap().len(), 1);
    }

    // TODO 15: knowledge gaps rank by reference count.
    #[tokio::test]
    async fn knowledge_gaps_rank_by_ref_count_normal() {
        let store = KnowledgeStore::open_in_memory().await.unwrap();
        store
            .note_gap("how to mock the network layer", "referenced, no lesson")
            .await
            .unwrap();
        store
            .note_gap("how to mock the network layer", "again")
            .await
            .unwrap();
        store.note_gap("CI cache config", "once").await.unwrap();
        let gaps = store.gaps(10).await.unwrap();
        assert_eq!(gaps.len(), 2);
        assert_eq!(gaps[0].label, "how to mock the network layer");
        assert_eq!(gaps[0].ref_count, 2);
    }

    // TODO 10: consolidation collapses duplicates to the strongest, supersedes
    // the rest, and is idempotent.
    #[tokio::test]
    async fn consolidate_collapses_duplicates_regression() {
        let store = KnowledgeStore::open_in_memory().await.unwrap();
        let weak = rec(Scope::Project, Some("P"), "dup", "use ripgrep here").with_confidence(0.3);
        let strong = rec(Scope::Project, Some("P"), "dup", "use   ripgrep here")
            .with_confidence(0.9)
            .with_outcome(Outcome::Verified);
        store.insert(&weak).await.unwrap();
        store.insert(&strong).await.unwrap();
        assert_eq!(store.live_count().await.unwrap(), 2);

        let n = store.consolidate().await.unwrap();
        assert_eq!(n, 1, "one duplicate should be superseded");
        assert_eq!(store.live_count().await.unwrap(), 1);
        // The verified/stronger one survives.
        let hits = store
            .recall(
                "ripgrep",
                &crate::query::RecallFilter {
                    project_key: Some("P"),
                    limit: 8,
                },
            )
            .await
            .unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].id, strong.id);
        // Idempotent.
        assert_eq!(store.consolidate().await.unwrap(), 0);
    }

    // TODO 11-13: mined lessons fold into project candidates and COMPOUND by
    // norm_key (support bump + verified upgrade) instead of duplicating.
    #[tokio::test]
    async fn ingest_mined_compounds_by_norm_key_regression() {
        use crate::session_mine::MinedLesson;
        let store = KnowledgeStore::open_in_memory().await.unwrap();
        let unverified = MinedLesson {
            kind: Kind::Finding,
            trigger: "Edit failed: old_string-not-found".into(),
            claim: "re-read exact bytes before retry".into(),
            outcome: Outcome::Unverified,
            norm_key: "err:edit:old_string-not-found".into(),
            session_id: "s1".into(),
        };
        let (ins, comp) = store
            .ingest_mined("P", std::slice::from_ref(&unverified))
            .await
            .unwrap();
        assert_eq!((ins, comp), (1, 0));
        assert_eq!(store.live_count().await.unwrap(), 1);

        // Same norm_key, now VERIFIED → compounds onto the existing row + upgrades.
        let mut verified = unverified;
        verified.outcome = Outcome::Verified;
        verified.session_id = "s2".into();
        let (ins2, comp2) = store.ingest_mined("P", &[verified]).await.unwrap();
        assert_eq!((ins2, comp2), (0, 1), "should compound, not duplicate");
        assert_eq!(store.live_count().await.unwrap(), 1);

        let hits = store
            .recall(
                "retry",
                &crate::query::RecallFilter {
                    project_key: Some("P"),
                    limit: 8,
                },
            )
            .await
            .unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(
            hits[0].outcome,
            Outcome::Verified,
            "outcome upgraded by new evidence"
        );
        assert!(hits[0].use_count >= 1, "support compounded");

        // A DIFFERENT project must not see P's mined lesson.
        let other = store
            .recall(
                "retry",
                &crate::query::RecallFilter {
                    project_key: Some("Q"),
                    limit: 8,
                },
            )
            .await
            .unwrap();
        assert!(other.is_empty());
    }

    // Autonomy: a verified, repeatedly-seen project lesson auto-promotes to
    // global; an unverified or rarely-seen one does NOT.
    #[tokio::test]
    async fn auto_promote_lifts_verified_repeated_lessons_normal() {
        use crate::record::Kind;
        let store = KnowledgeStore::open_in_memory().await.unwrap();
        let mk = |kind: Kind, title: &str, body: &str| {
            KnowledgeRecord::new(kind, Scope::Project, Some("P".into()), title, body)
        };
        // Generalizable (Finding), verified + enough support → promoted.
        let mut hot = mk(Kind::Finding, "hot", "use ripgrep").with_outcome(Outcome::Verified);
        hot.use_count = 3;
        store.insert(&hot).await.unwrap();
        // Verified but under the support bar → stays project.
        let mut rare = mk(Kind::Finding, "rare", "niche tip").with_outcome(Outcome::Verified);
        rare.use_count = 1;
        store.insert(&rare).await.unwrap();
        // High support but unverified → stays project.
        let mut noisy = mk(Kind::Finding, "noisy", "unconfirmed");
        noisy.use_count = 9;
        store.insert(&noisy).await.unwrap();
        // A project-specific FACT, verified + well-supported, must NOT auto-promote
        // (it would poison other projects with wrong-context truth).
        let mut fact =
            mk(Kind::Fact, "stack", "this repo uses vite").with_outcome(Outcome::Verified);
        fact.use_count = 9;
        store.insert(&fact).await.unwrap();

        let promoted = store.auto_promote(3).await.unwrap();
        assert_eq!(
            promoted, 1,
            "only the verified, well-supported, generalizable lesson promotes"
        );

        // The promoted one is now recalled from a DIFFERENT project.
        let other = crate::query::RecallFilter {
            project_key: Some("Q"),
            limit: 8,
        };
        let hits = store.recall("ripgrep", &other).await.unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].id, hot.id);
        // The unverified/rare/fact ones did not leak across projects.
        assert!(store.recall("niche", &other).await.unwrap().is_empty());
        assert!(
            store
                .recall("unconfirmed", &other)
                .await
                .unwrap()
                .is_empty()
        );
        assert!(
            store.recall("vite", &other).await.unwrap().is_empty(),
            "project-specific fact must not leak"
        );
    }

    // The throttle prevents re-processing on a second startup within the window.
    #[tokio::test]
    async fn maintain_throttle_blocks_rapid_repeat_normal() {
        let store = KnowledgeStore::open_in_memory().await.unwrap();
        assert!(
            store.maintain_due("P", MAINTAIN_THROTTLE_MS).await.unwrap(),
            "first run is due"
        );
        store.stamp_maintain("P").await.unwrap();
        assert!(
            !store.maintain_due("P", MAINTAIN_THROTTLE_MS).await.unwrap(),
            "just-stamped is not due"
        );
        // A different project is independently due.
        assert!(store.maintain_due("Q", MAINTAIN_THROTTLE_MS).await.unwrap());
        // With a zero window, it's due again immediately.
        assert!(store.maintain_due("P", 0).await.unwrap());
    }

    #[tokio::test]
    async fn auto_maintain_imports_and_grows_normal() {
        // Hermetic: point the store + dirs at temp paths via JFC_KNOWLEDGE_DB.
        let dir = tempfile::tempdir().unwrap();
        let dbpath = dir.path().join("k.db");
        let user_mem = dir.path().join("umem");
        std::fs::create_dir_all(&user_mem).unwrap();
        std::fs::write(
            user_mem.join("p.md"),
            "---\ntype: preference\n---\nuse spaces not tabs",
        )
        .unwrap();
        // Isolate the global store path for this test.
        let _guard = EnvGuard::set("JFC_KNOWLEDGE_DB", dbpath.to_str().unwrap());
        let store = KnowledgeStore::open(&dbpath).await.unwrap();
        store
            .replace_transcript(
                &SessionRow {
                    id: "ses_1".into(),
                    cwd: Some(dir.path().to_string_lossy().into_owned()),
                    model: Some("claude".into()),
                    created_at: Some("2026-01-01T00:00:00Z".into()),
                    updated_at: Some("2026-01-01T01:00:00Z".into()),
                    first_prompt: Some("fix edit".into()),
                    title: None,
                    message_count: 2,
                },
                &[
                    SessionMessage {
                        seq: 0,
                        role: "assistant".into(),
                        content: "edit failed".into(),
                        meta: Some(
                            serde_json::json!({
                                "role": "assistant",
                                "parts": [{
                                    "type": "tool",
                                    "kind": "Edit",
                                    "status": "failed",
                                    "output": {
                                        "type": "text",
                                        "content": "old_string not found"
                                    }
                                }]
                            })
                            .to_string(),
                        ),
                    },
                    SessionMessage {
                        seq: 1,
                        role: "assistant".into(),
                        content: "edit succeeded".into(),
                        meta: Some(
                            serde_json::json!({
                                "role": "assistant",
                                "parts": [{
                                    "type": "tool",
                                    "kind": "Edit",
                                    "status": "complete",
                                    "output": {
                                        "type": "text",
                                        "content": "ok"
                                    }
                                }]
                            })
                            .to_string(),
                        ),
                    },
                ],
            )
            .await
            .unwrap();
        drop(store);

        let report = auto_maintain(dir.path(), None, Some(&user_mem), None)
            .await
            .unwrap();
        assert_eq!(report.imported, 1, "imported the .md preference");
        assert_eq!(report.sessions_scanned, 1);
        assert!(report.mined_inserted >= 1, "mined the recovered Edit error");

        // The store actually grew and persists.
        let store = KnowledgeStore::open(&dbpath).await.unwrap();
        assert!(store.live_count().await.unwrap() >= 2);
    }

    #[tokio::test]
    async fn auto_maintain_promotes_proven_generalizable_lessons_regression() {
        let dir = tempfile::tempdir().unwrap();
        let dbpath = dir.path().join("k.db");
        let _guard = EnvGuard::set("JFC_KNOWLEDGE_DB", dbpath.to_str().unwrap());
        let project = project::project_key(dir.path());
        let store = KnowledgeStore::open(&dbpath).await.unwrap();
        let mut lesson = KnowledgeRecord::new(
            Kind::Finding,
            Scope::Project,
            Some(project),
            "Prefer rg",
            "use ripgrep for repository searches",
        )
        .with_outcome(Outcome::Verified);
        lesson.use_count = DEFAULT_AUTO_PROMOTE_SUPPORT;
        let id = lesson.id.clone();
        store.insert(&lesson).await.unwrap();
        drop(store);

        let report = auto_maintain(dir.path(), None, None, None).await.unwrap();

        assert_eq!(report.auto_promoted, 1);
        let store = KnowledgeStore::open(&dbpath).await.unwrap();
        let hits = store
            .recall(
                "ripgrep",
                &RecallFilter {
                    project_key: Some("other-project"),
                    limit: 8,
                },
            )
            .await
            .unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].id, id);
        assert_eq!(hits[0].scope, Scope::Global);
    }

    fn env_guard_lock() -> MutexGuard<'static, ()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
            .lock()
            .unwrap_or_else(|e| e.into_inner())
    }

    /// Minimal scoped env setter for hermetic tests that share process env.
    struct EnvGuard {
        key: &'static str,
        prev: Option<std::ffi::OsString>,
        _lock: MutexGuard<'static, ()>,
    }
    impl EnvGuard {
        fn set(key: &'static str, val: &str) -> Self {
            let lock = env_guard_lock();
            let prev = std::env::var_os(key);
            // SAFETY: test-only, serialized by env_guard_lock(), and restored on drop.
            unsafe { std::env::set_var(key, val) };
            Self {
                key,
                prev,
                _lock: lock,
            }
        }
    }
    impl Drop for EnvGuard {
        fn drop(&mut self) {
            unsafe {
                match &self.prev {
                    Some(v) => std::env::set_var(self.key, v),
                    None => std::env::remove_var(self.key),
                }
            }
        }
    }

    // TODO 22: the session index upserts (idempotent), lists most-recent-first,
    // and filters by cwd — additive, no JSON involved.
    // TODO 23: full-transcript store — replace/load round-trips in seq
    // order, FTS search finds the right session, replace is idempotent (coalesce-
    // safe), and backup writes a consistent copy.
    #[tokio::test]
    async fn session_transcript_roundtrip_and_search_normal() {
        // File-backed (not in-memory) because this test exercises `backup_to`,
        // and SQLite's `VACUUM INTO` produces no output file for a `:memory:`
        // source database.
        let src_dir = tempfile::tempdir().unwrap();
        let store = KnowledgeStore::open(&src_dir.path().join("src.db"))
            .await
            .unwrap();
        let hdr = SessionRow {
            id: "ses_x".into(),
            cwd: Some("/proj".into()),
            model: Some("m".into()),
            created_at: Some("2026-01-01T00:00:00Z".into()),
            updated_at: Some("2026-01-01T01:00:00Z".into()),
            first_prompt: Some("hello".into()),
            title: None,
            message_count: 2,
        };
        let msgs = vec![
            SessionMessage {
                seq: 0,
                role: "user".into(),
                content: "fix the ripgrep gutter bug".into(),
                meta: None,
            },
            SessionMessage {
                seq: 1,
                role: "assistant".into(),
                content: "done, edited bash.rs".into(),
                meta: Some("{\"k\":1}".into()),
            },
        ];
        store.replace_transcript(&hdr, &msgs).await.unwrap();

        // Round-trips in order with meta intact.
        let loaded = store.load_transcript("ses_x").await.unwrap();
        assert_eq!(loaded, msgs);
        assert!(store.has_transcript("ses_x").await.unwrap());
        assert_eq!(
            store.session_count().await.unwrap(),
            1,
            "header upserted in same txn"
        );

        // FTS search finds the session by a content term.
        assert_eq!(
            store.search_transcripts("ripgrep", 10).await.unwrap(),
            vec!["ses_x".to_string()]
        );
        assert!(
            store
                .search_transcripts("nonexistentterm", 10)
                .await
                .unwrap()
                .is_empty()
        );

        // Replace with a coalesced (shorter) transcript — no stale rows, FTS updated.
        let shorter = vec![SessionMessage {
            seq: 0,
            role: "user".into(),
            content: "different now".into(),
            meta: None,
        }];
        store
            .replace_transcript(
                &SessionRow {
                    message_count: 1,
                    ..hdr.clone()
                },
                &shorter,
            )
            .await
            .unwrap();
        assert_eq!(store.load_transcript("ses_x").await.unwrap(), shorter);
        assert!(
            store
                .search_transcripts("ripgrep", 10)
                .await
                .unwrap()
                .is_empty(),
            "old content gone from FTS"
        );

        let deleted = store.delete_session("ses_x").await.unwrap();
        assert!(deleted >= 2);
        assert_eq!(store.session_count().await.unwrap(), 0);
        assert!(store.load_transcript("ses_x").await.unwrap().is_empty());

        store
            .replace_transcript(
                &SessionRow {
                    message_count: 1,
                    ..hdr.clone()
                },
                &shorter,
            )
            .await
            .unwrap();

        // Backup writes a consistent, openable copy.
        let dir = tempfile::tempdir().unwrap();
        let bpath = dir.path().join("backup.db");
        store.backup_to(&bpath).await.unwrap();
        let restored = KnowledgeStore::open(&bpath).await.unwrap();
        assert_eq!(restored.load_transcript("ses_x").await.unwrap(), shorter);
    }

    #[tokio::test]
    async fn session_event_substrate_is_derived_from_transcript_normal() {
        let store = KnowledgeStore::open_in_memory().await.unwrap();
        let hdr = SessionRow {
            id: "ses_events".into(),
            cwd: Some("/proj".into()),
            model: Some("claude-sonnet-4".into()),
            created_at: Some("2026-01-01T00:00:00Z".into()),
            updated_at: Some("2026-01-01T01:00:00Z".into()),
            first_prompt: Some("run tests".into()),
            title: None,
            message_count: 2,
        };
        let assistant_meta = serde_json::json!({
            "role": "assistant",
            "model_name": "anthropic/claude-opus-4-7",
            "usage": {
                "input_tokens": 12000,
                "output_tokens": 600,
                "thinking_tokens": 42,
                "cache_read_tokens": 0,
                "cache_write_tokens": 0
            },
            "parts": [
                {"type": "reasoning", "content": "checking"},
                {
                    "type": "tool",
                    "id": "toolu_1",
                    "kind": "Bash",
                    "status": "success",
                    "input": {
                        "type": "bash",
                        "command": "cargo test",
                        "task_id": "bash_123"
                    },
                    "output": {
                        "content": "ok",
                        "elapsed_ms": 742
                    }
                },
                {"type": "text", "content": "done"}
            ]
        });
        let msgs = vec![
            SessionMessage {
                seq: 0,
                role: "user".into(),
                content: "run tests".into(),
                meta: None,
            },
            SessionMessage {
                seq: 1,
                role: "assistant".into(),
                content: "done".into(),
                meta: Some(assistant_meta.to_string()),
            },
        ];

        store.replace_transcript(&hdr, &msgs).await.unwrap();

        let events = store.list_session_events("ses_events", 20).await.unwrap();
        assert_eq!(events.len(), 3);
        assert!(events.iter().any(|event| event.kind == "message:user"));
        assert!(events.iter().any(|event| event.kind == "tool_run"));

        let turns = store.list_session_turns("ses_events").await.unwrap();
        assert_eq!(turns.len(), 1);
        assert_eq!(turns[0].user_text, "run tests");
        assert_eq!(turns[0].assistant_text, "done");
        assert_eq!(turns[0].status, "complete");
        assert_eq!(turns[0].model.as_deref(), Some("claude-sonnet-4"));

        let tools = store.list_session_tool_runs("ses_events").await.unwrap();
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0].tool_call_id.as_deref(), Some("toolu_1"));
        assert_eq!(tools[0].runtime_id.as_deref(), Some("bash_123"));
        assert_eq!(tools[0].kind, "Bash");
        assert_eq!(tools[0].status, "success");
        assert_eq!(tools[0].duration_ms, Some(742));

        let context = store
            .list_context_events(Some("ses_events"), 20)
            .await
            .unwrap();
        assert_eq!(context.len(), 1);
        assert_eq!(context[0].turn_id.as_deref(), Some("ses_events:1"));
        assert_eq!(context[0].model, "anthropic/claude-opus-4-7");
        assert_eq!(context[0].input_tokens, 12000);
        assert_eq!(context[0].output_tokens, 600);
        assert_eq!(context[0].thinking_tokens, 42);
        assert_eq!(context[0].bust_cause.as_deref(), Some("cache_miss"));
    }

    #[tokio::test]
    async fn agent_learning_substrate_roundtrips_normal() {
        let store = KnowledgeStore::open_in_memory().await.unwrap();
        let now = record::now_ms();
        let agent = AgentSessionRow {
            id: "agent_advisor_1".into(),
            parent_session_id: Some("ses_parent".into()),
            role: "advisor".into(),
            model: Some("claude-sonnet-4".into()),
            status: "running".into(),
            budget_tokens: Some(50_000),
            task_id: Some("task_1".into()),
            team_id: Some("team_alpha".into()),
            created_at_ms: now,
            updated_at_ms: now,
        };
        store.upsert_agent_session(&agent).await.unwrap();

        let event = AgentEventRow {
            id: "evt_1".into(),
            session_id: "ses_parent".into(),
            from_agent: Some("main".into()),
            to_agent: Some("agent_advisor_1".into()),
            kind: "delegate".into(),
            content: "review the context plan".into(),
            turn_id: Some("turn_1".into()),
            causal_parent_id: None,
            created_at_ms: now,
        };
        store.record_agent_event(&event).await.unwrap();

        let mail = AgentMailboxRow {
            id: "mail_1".into(),
            to_agent: "agent_advisor_1".into(),
            from_agent: Some("main".into()),
            thread_id: Some("thread_1".into()),
            task_id: Some("task_1".into()),
            priority: 3,
            content: "send synthesis when done".into(),
            read_at_ms: None,
            summarized_at_ms: None,
            created_at_ms: now,
        };
        store.enqueue_agent_mailbox(&mail).await.unwrap();

        let run = ToolRunLedgerRow {
            id: "tool_1".into(),
            agent_id: Some("agent_advisor_1".into()),
            session_id: Some("ses_parent".into()),
            runtime_id: Some("bash_1".into()),
            kind: "Bash".into(),
            command: Some("cargo test".into()),
            input_json: Some("{\"command\":\"cargo test\"}".into()),
            output_ref: Some("artifact:tool_1".into()),
            status: "success".into(),
            duration_ms: Some(1234),
            background: false,
            created_at_ms: now,
            updated_at_ms: now,
        };
        store.record_tool_run(&run).await.unwrap();

        let learning = LearningEventRow {
            id: "learn_1".into(),
            source_session_id: Some("ses_parent".into()),
            source_turn_id: Some("turn_1".into()),
            source_tool_run_id: Some("tool_1".into()),
            candidate_rule: "Run focused tests before workspace tests.".into(),
            status: "candidate".into(),
            verifier_evidence: "tool_1 passed".into(),
            recurrence_count: 1,
            created_at_ms: now,
            updated_at_ms: now,
        };
        store.record_learning_event(&learning).await.unwrap();

        assert_eq!(
            store.get_agent_session("agent_advisor_1").await.unwrap(),
            Some(agent.clone())
        );
        assert_eq!(
            store
                .list_agent_sessions_by_team("team_alpha", 10)
                .await
                .unwrap(),
            vec![agent]
        );
        assert_eq!(
            store.list_agent_events("ses_parent", 10).await.unwrap(),
            vec![event]
        );
        assert_eq!(
            store
                .list_agent_mailbox("agent_advisor_1", true)
                .await
                .unwrap(),
            vec![mail.clone()]
        );
        assert_eq!(store.mark_agent_mailbox_read("mail_1").await.unwrap(), 1);
        assert!(
            store
                .list_agent_mailbox("agent_advisor_1", true)
                .await
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            store
                .list_learning_events(Some("candidate"), 10)
                .await
                .unwrap(),
            vec![learning]
        );

        let deleted = store.delete_session("ses_parent").await.unwrap();
        assert!(deleted >= 3);
        assert!(
            store
                .list_agent_events("ses_parent", 10)
                .await
                .unwrap()
                .is_empty()
        );
        assert!(
            store
                .list_learning_events(Some("candidate"), 10)
                .await
                .unwrap()
                .is_empty()
        );
    }

    #[tokio::test]
    async fn session_index_upsert_and_list_normal() {
        let store = KnowledgeStore::open_in_memory().await.unwrap();
        let row = |id: &str, cwd: &str, updated: &str, n: i64| SessionRow {
            id: id.into(),
            cwd: Some(cwd.into()),
            model: Some("m".into()),
            created_at: Some("2026-01-01T00:00:00Z".into()),
            updated_at: Some(updated.into()),
            first_prompt: Some("hi".into()),
            title: None,
            message_count: n,
        };
        store
            .upsert_session(&row("s1", "/a", "2026-01-01T01:00:00Z", 2))
            .await
            .unwrap();
        store
            .upsert_session(&row("s2", "/a", "2026-01-01T03:00:00Z", 4))
            .await
            .unwrap();
        store
            .upsert_session(&row("s3", "/b", "2026-01-01T02:00:00Z", 6))
            .await
            .unwrap();
        assert_eq!(store.session_count().await.unwrap(), 3);

        // Re-upsert s1 with a new message_count → updates, not duplicates.
        store
            .upsert_session(&row("s1", "/a", "2026-01-01T05:00:00Z", 9))
            .await
            .unwrap();
        assert_eq!(store.session_count().await.unwrap(), 3);
        assert_eq!(
            store
                .get_session("s1")
                .await
                .unwrap()
                .unwrap()
                .message_count,
            9
        );

        // List for /a, most-recently-updated first (s1 now newest after re-upsert).
        let a = store.list_sessions(Some("/a"), 10).await.unwrap();
        assert_eq!(
            a.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(),
            ["s1", "s2"]
        );
        // Global list includes all three.
        assert_eq!(store.list_sessions(None, 10).await.unwrap().len(), 3);
        // Unknown cwd → empty.
        assert!(
            store
                .list_sessions(Some("/nope"), 10)
                .await
                .unwrap()
                .is_empty()
        );
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
