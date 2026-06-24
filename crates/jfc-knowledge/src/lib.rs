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

use rusqlite::{Connection, OptionalExtension};

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
    /// Crate-internal connection accessor, so the split-out `memory` module can
    /// add `impl KnowledgeStore` methods without the field being `pub`.
    pub(crate) fn conn(&self) -> &Connection {
        &self.conn
    }

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
    /// promotes directly; callers run [`Self::auto_promote`] after compounding
    /// enough verified evidence. Returns
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
        Ok(self
            .conn
            .query_row(
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
            let (level, project_key) = import_memory_level(item);
            let hash = import_memory_hash(item);
            let meta_json = import_memory_meta_json(item, &hash);
            match self.insert_memory(&NewMemory {
                id,
                level,
                project_key,
                title: &item.title,
                body: &item.body,
                hash: &hash,
                meta_json: &meta_json,
            }) {
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
    /// live record was promoted. Used by the explicit `/knowledge promote`
    /// command or an approved proposal.
    pub fn promote(&self, id: &str) -> Result<bool> {
        query::promote_to_global(&self.conn, id)
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

    /// Upsert a session-index row. The SQLite session catalog is the primary
    /// picker/search surface.
    pub fn upsert_session(&self, row: &SessionRow) -> Result<()> {
        self.conn.execute(
            "INSERT INTO sessions \
             (id, cwd, model, created_at, updated_at, first_prompt, title, message_count) \
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8) \
             ON CONFLICT(id) DO UPDATE SET \
                cwd=excluded.cwd, model=excluded.model, created_at=excluded.created_at, \
                updated_at=excluded.updated_at, first_prompt=excluded.first_prompt, \
                title=excluded.title, message_count=excluded.message_count",
            rusqlite::params![
                row.id,
                row.cwd,
                row.model,
                row.created_at,
                row.updated_at,
                row.first_prompt,
                row.title,
                row.message_count,
            ],
        )?;
        Ok(())
    }

    /// One session-index row by id (or `None`).
    pub fn get_session(&self, id: &str) -> Result<Option<SessionRow>> {
        Ok(self
            .conn
            .query_row(
                "SELECT id, cwd, model, created_at, updated_at, first_prompt, title, message_count \
                 FROM sessions WHERE id = ?1",
                [id],
                session_row_from,
            )
            .optional()?)
    }

    /// Session-index rows, most-recently-updated first. `cwd` filters to one
    /// project when `Some`.
    pub fn list_sessions(&self, cwd: Option<&str>, limit: usize) -> Result<Vec<SessionRow>> {
        let mut out = Vec::new();
        if let Some(cwd) = cwd {
            let mut stmt = self.conn.prepare(
                "SELECT id, cwd, model, created_at, updated_at, first_prompt, title, message_count \
                 FROM sessions WHERE cwd = ?1 ORDER BY updated_at DESC LIMIT ?2",
            )?;
            let rows = stmt.query_map(rusqlite::params![cwd, limit as i64], session_row_from)?;
            for r in rows {
                out.push(r?);
            }
        } else {
            let mut stmt = self.conn.prepare(
                "SELECT id, cwd, model, created_at, updated_at, first_prompt, title, message_count \
                 FROM sessions ORDER BY updated_at DESC LIMIT ?1",
            )?;
            let rows = stmt.query_map([limit as i64], session_row_from)?;
            for r in rows {
                out.push(r?);
            }
        }
        Ok(out)
    }

    /// Delete one session row and its transcript. Returns the number of DB rows
    /// removed across session-owned tables.
    pub fn delete_session(&mut self, id: &str) -> Result<usize> {
        let tx = self.conn.transaction()?;
        let agent_scoped = agent_events::delete_session_scoped_rows(&tx, id)?;
        let artifact_events = tx.execute(
            "DELETE FROM session_artifact_events WHERE session_id = ?1",
            [id],
        )?;
        let artifacts = tx.execute("DELETE FROM session_artifacts WHERE session_id = ?1", [id])?;
        let findings = tx.execute("DELETE FROM session_findings WHERE session_id = ?1", [id])?;
        let compactions = tx.execute(
            "DELETE FROM session_compactions WHERE session_id = ?1",
            [id],
        )?;
        let retrievals = tx.execute(
            "DELETE FROM session_retrieval_events WHERE session_id = ?1",
            [id],
        )?;
        let tools = tx.execute("DELETE FROM session_tool_runs WHERE session_id = ?1", [id])?;
        let turns = tx.execute("DELETE FROM session_turns WHERE session_id = ?1", [id])?;
        let events = tx.execute("DELETE FROM session_events WHERE session_id = ?1", [id])?;
        let messages = tx.execute("DELETE FROM session_messages WHERE session_id = ?1", [id])?;
        let session = tx.execute("DELETE FROM sessions WHERE id = ?1", [id])?;
        tx.commit()?;
        Ok(agent_scoped
            + artifact_events
            + artifacts
            + findings
            + compactions
            + retrievals
            + tools
            + turns
            + events
            + messages
            + session)
    }

    /// Count of indexed sessions — for tests/metrics.
    pub fn session_count(&self) -> Result<i64> {
        Ok(self
            .conn
            .query_row("SELECT count(*) FROM sessions", [], |r| r.get(0))?)
    }

    /// Replace a session's full transcript. The header upsert and
    /// all message rows commit in ONE transaction (council decision 1+5:
    /// cross-row atomicity the per-file rename never gave us). We delete+reinsert
    /// rather than diff because the engine coalesces messages on save, so seq
    /// numbers can shift; the FTS mirror stays consistent via triggers.
    pub fn replace_transcript(
        &mut self,
        row: &SessionRow,
        messages: &[SessionMessage],
    ) -> Result<()> {
        let tx = self.conn.transaction()?;
        tx.execute(
            "INSERT INTO sessions \
             (id, cwd, model, created_at, updated_at, first_prompt, title, message_count) \
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8) \
             ON CONFLICT(id) DO UPDATE SET \
                cwd=excluded.cwd, model=excluded.model, created_at=excluded.created_at, \
                updated_at=excluded.updated_at, first_prompt=excluded.first_prompt, \
                title=excluded.title, message_count=excluded.message_count",
            rusqlite::params![
                row.id,
                row.cwd,
                row.model,
                row.created_at,
                row.updated_at,
                row.first_prompt,
                row.title,
                row.message_count,
            ],
        )?;
        tx.execute(
            "DELETE FROM session_messages WHERE session_id = ?1",
            [&row.id],
        )?;
        tx.execute(
            "DELETE FROM session_events WHERE session_id = ?1",
            [&row.id],
        )?;
        tx.execute("DELETE FROM session_turns WHERE session_id = ?1", [&row.id])?;
        tx.execute(
            "DELETE FROM session_tool_runs WHERE session_id = ?1",
            [&row.id],
        )?;
        agent_events::clear_derived_context_events(&tx, &row.id)?;
        {
            let mut stmt = tx.prepare(
                "INSERT INTO session_messages (session_id, seq, role, content, meta) \
                 VALUES (?1,?2,?3,?4,?5)",
            )?;
            for m in messages {
                stmt.execute(rusqlite::params![row.id, m.seq, m.role, m.content, m.meta])?;
            }
        }
        insert_derived_session_rows(&tx, row, messages)?;
        agent_events::insert_context_events_from_messages(&tx, row, messages, record::now_ms())?;
        tx.commit()?;
        Ok(())
    }

    /// Load a session's full transcript in `seq` order (resume path, post-flip).
    pub fn load_transcript(&self, session_id: &str) -> Result<Vec<SessionMessage>> {
        let mut stmt = self.conn.prepare(
            "SELECT seq, role, content, meta FROM session_messages \
             WHERE session_id = ?1 ORDER BY seq ASC",
        )?;
        let rows = stmt.query_map([session_id], |r| {
            Ok(SessionMessage {
                seq: r.get(0)?,
                role: r.get(1)?,
                content: r.get(2)?,
                meta: r.get(3)?,
            })
        })?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r?);
        }
        Ok(out)
    }

    /// Whether a session has any transcript rows (parity bookkeeping).
    pub fn has_transcript(&self, session_id: &str) -> Result<bool> {
        Ok(self
            .conn
            .query_row(
                "SELECT 1 FROM session_messages WHERE session_id = ?1 LIMIT 1",
                [session_id],
                |_| Ok(()),
            )
            .optional()?
            .is_some())
    }

    pub fn list_session_events(
        &self,
        session_id: &str,
        limit: usize,
    ) -> Result<Vec<SessionEventRow>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, session_id, seq, kind, created_at_ms, payload \
             FROM session_events WHERE session_id = ?1 ORDER BY seq ASC, created_at_ms ASC \
             LIMIT ?2",
        )?;
        let rows = stmt.query_map(rusqlite::params![session_id, limit as i64], |r| {
            Ok(SessionEventRow {
                id: r.get(0)?,
                session_id: r.get(1)?,
                seq: r.get(2)?,
                kind: r.get(3)?,
                created_at_ms: r.get(4)?,
                payload: r.get(5)?,
            })
        })?;
        collect_rows(rows)
    }

    pub fn list_session_turns(&self, session_id: &str) -> Result<Vec<SessionTurnRow>> {
        let mut stmt = self.conn.prepare(
            "SELECT session_id, turn_index, user_seq, assistant_seq, user_text, assistant_text, \
                    status, model, created_at_ms \
             FROM session_turns WHERE session_id = ?1 ORDER BY turn_index ASC",
        )?;
        let rows = stmt.query_map([session_id], |r| {
            Ok(SessionTurnRow {
                session_id: r.get(0)?,
                turn_index: r.get(1)?,
                user_seq: r.get(2)?,
                assistant_seq: r.get(3)?,
                user_text: r.get(4)?,
                assistant_text: r.get(5)?,
                status: r.get(6)?,
                model: r.get(7)?,
                created_at_ms: r.get(8)?,
            })
        })?;
        collect_rows(rows)
    }

    pub fn list_session_tool_runs(&self, session_id: &str) -> Result<Vec<SessionToolRunRow>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, session_id, message_seq, part_index, tool_call_id, runtime_id, kind, \
                    status, input_json, output_json, duration_ms, created_at_ms \
             FROM session_tool_runs WHERE session_id = ?1 \
             ORDER BY message_seq ASC, part_index ASC",
        )?;
        let rows = stmt.query_map([session_id], |r| {
            Ok(SessionToolRunRow {
                id: r.get(0)?,
                session_id: r.get(1)?,
                message_seq: r.get(2)?,
                part_index: r.get(3)?,
                tool_call_id: r.get(4)?,
                runtime_id: r.get(5)?,
                kind: r.get(6)?,
                status: r.get(7)?,
                input_json: r.get(8)?,
                output_json: r.get(9)?,
                duration_ms: r.get(10)?,
                created_at_ms: r.get(11)?,
            })
        })?;
        collect_rows(rows)
    }

    pub fn record_retrieval_event(&self, event: &SessionRetrievalEvent) -> Result<()> {
        self.conn.execute(
            "INSERT INTO session_retrieval_events \
             (id, session_id, query, source, result_count, payload, created_at_ms) \
             VALUES (?1,?2,?3,?4,?5,?6,?7)",
            rusqlite::params![
                event.id,
                event.session_id,
                event.query,
                event.source,
                event.result_count,
                event.payload,
                event.created_at_ms,
            ],
        )?;
        Ok(())
    }

    pub fn record_compaction(&self, compaction: &SessionCompactionRow) -> Result<()> {
        self.conn.execute(
            "INSERT INTO session_compactions \
             (id, session_id, before_tokens, after_tokens, summary, payload, created_at_ms) \
             VALUES (?1,?2,?3,?4,?5,?6,?7)",
            rusqlite::params![
                compaction.id,
                compaction.session_id,
                compaction.before_tokens,
                compaction.after_tokens,
                compaction.summary,
                compaction.payload,
                compaction.created_at_ms,
            ],
        )?;
        Ok(())
    }

    pub fn record_session_finding(&self, finding: &SessionFindingRow) -> Result<()> {
        self.conn.execute(
            "INSERT INTO session_findings \
             (id, session_id, kind, summary, evidence, status, created_at_ms, resolved_at_ms) \
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8)",
            rusqlite::params![
                finding.id,
                finding.session_id,
                finding.kind,
                finding.summary,
                finding.evidence,
                finding.status,
                finding.created_at_ms,
                finding.resolved_at_ms,
            ],
        )?;
        Ok(())
    }

    pub fn upsert_session_artifact(
        &self,
        session_id: &str,
        kind: &str,
        key: &str,
        value_json: &str,
    ) -> Result<()> {
        let now = record::now_ms();
        self.conn.execute(
            "INSERT INTO session_artifacts \
             (session_id, kind, key, value_json, created_at_ms, updated_at_ms) \
             VALUES (?1,?2,?3,?4,?5,?5) \
             ON CONFLICT(session_id, kind, key) DO UPDATE SET \
                value_json=excluded.value_json, updated_at_ms=excluded.updated_at_ms",
            rusqlite::params![session_id, kind, key, value_json, now],
        )?;
        Ok(())
    }

    pub fn get_session_artifact(
        &self,
        session_id: &str,
        kind: &str,
        key: &str,
    ) -> Result<Option<SessionArtifactRow>> {
        Ok(self
            .conn
            .query_row(
                "SELECT session_id, kind, key, value_json, created_at_ms, updated_at_ms \
                 FROM session_artifacts WHERE session_id = ?1 AND kind = ?2 AND key = ?3",
                rusqlite::params![session_id, kind, key],
                session_artifact_from,
            )
            .optional()?)
    }

    pub fn list_session_artifacts(
        &self,
        session_id: &str,
        kind: &str,
        limit: usize,
    ) -> Result<Vec<SessionArtifactRow>> {
        let mut stmt = self.conn.prepare(
            "SELECT session_id, kind, key, value_json, created_at_ms, updated_at_ms \
             FROM session_artifacts WHERE session_id = ?1 AND kind = ?2 \
             ORDER BY updated_at_ms DESC LIMIT ?3",
        )?;
        let rows = stmt.query_map(
            rusqlite::params![session_id, kind, limit as i64],
            session_artifact_from,
        )?;
        collect_rows(rows)
    }

    pub fn delete_session_artifact(
        &self,
        session_id: &str,
        kind: &str,
        key: &str,
    ) -> Result<usize> {
        Ok(self.conn.execute(
            "DELETE FROM session_artifacts WHERE session_id = ?1 AND kind = ?2 AND key = ?3",
            rusqlite::params![session_id, kind, key],
        )?)
    }

    pub fn append_session_artifact_event(
        &self,
        session_id: &str,
        kind: &str,
        key: &str,
        value_json: &str,
    ) -> Result<i64> {
        self.conn.execute(
            "INSERT INTO session_artifact_events (session_id, kind, key, value_json, created_at_ms) \
             VALUES (?1,?2,?3,?4,?5)",
            rusqlite::params![session_id, kind, key, value_json, record::now_ms()],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn list_session_artifact_events(
        &self,
        session_id: &str,
        kind: &str,
        key: Option<&str>,
        limit: usize,
    ) -> Result<Vec<SessionArtifactEventRow>> {
        let limit = limit as i64;
        let mut out = Vec::new();
        if let Some(key) = key {
            let mut stmt = self.conn.prepare(
                "SELECT id, session_id, kind, key, value_json, created_at_ms \
                 FROM session_artifact_events \
                 WHERE session_id = ?1 AND kind = ?2 AND key = ?3 \
                 ORDER BY id ASC LIMIT ?4",
            )?;
            let rows = stmt.query_map(
                rusqlite::params![session_id, kind, key, limit],
                session_artifact_event_from,
            )?;
            for row in rows {
                out.push(row?);
            }
        } else {
            let mut stmt = self.conn.prepare(
                "SELECT id, session_id, kind, key, value_json, created_at_ms \
                 FROM session_artifact_events \
                 WHERE session_id = ?1 AND kind = ?2 \
                 ORDER BY id ASC LIMIT ?3",
            )?;
            let rows = stmt.query_map(
                rusqlite::params![session_id, kind, limit],
                session_artifact_event_from,
            )?;
            for row in rows {
                out.push(row?);
            }
        }
        Ok(out)
    }

    pub fn list_recent_session_artifact_events(
        &self,
        session_id: &str,
        kind: &str,
        key: Option<&str>,
        limit: usize,
    ) -> Result<Vec<SessionArtifactEventRow>> {
        let limit = limit as i64;
        let mut out = Vec::new();
        if let Some(key) = key {
            let mut stmt = self.conn.prepare(
                "SELECT id, session_id, kind, key, value_json, created_at_ms \
                 FROM session_artifact_events \
                 WHERE session_id = ?1 AND kind = ?2 AND key = ?3 \
                 ORDER BY id DESC LIMIT ?4",
            )?;
            let rows = stmt.query_map(
                rusqlite::params![session_id, kind, key, limit],
                session_artifact_event_from,
            )?;
            for row in rows {
                out.push(row?);
            }
        } else {
            let mut stmt = self.conn.prepare(
                "SELECT id, session_id, kind, key, value_json, created_at_ms \
                 FROM session_artifact_events \
                 WHERE session_id = ?1 AND kind = ?2 \
                 ORDER BY id DESC LIMIT ?3",
            )?;
            let rows = stmt.query_map(
                rusqlite::params![session_id, kind, limit],
                session_artifact_event_from,
            )?;
            for row in rows {
                out.push(row?);
            }
        }
        out.reverse();
        Ok(out)
    }

    pub fn clear_session_artifact_events(
        &self,
        session_id: &str,
        kind: &str,
        key: Option<&str>,
    ) -> Result<usize> {
        if let Some(key) = key {
            return Ok(self.conn.execute(
                "DELETE FROM session_artifact_events \
                 WHERE session_id = ?1 AND kind = ?2 AND key = ?3",
                rusqlite::params![session_id, kind, key],
            )?);
        }
        Ok(self.conn.execute(
            "DELETE FROM session_artifact_events WHERE session_id = ?1 AND kind = ?2",
            rusqlite::params![session_id, kind],
        )?)
    }

    /// Session ids whose transcript matches an FTS query (substring search path).
    pub fn search_transcripts(&self, query: &str, limit: usize) -> Result<Vec<String>> {
        let terms = query
            .split_whitespace()
            .filter(|t| t.len() >= 2)
            .map(|t| format!("\"{}\"", t.replace('"', "")))
            .collect::<Vec<_>>()
            .join(" OR ");
        if terms.is_empty() {
            return Ok(Vec::new());
        }
        let mut stmt = self.conn.prepare(
            "SELECT DISTINCT m.session_id FROM session_messages_fts f \
             JOIN session_messages m ON m.rowid = f.rowid \
             WHERE session_messages_fts MATCH ?1 LIMIT ?2",
        )?;
        let rows = stmt.query_map(rusqlite::params![terms, limit as i64], |r| {
            r.get::<_, String>(0)
        })?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r?);
        }
        Ok(out)
    }

    /// Fast, consistent file-level backup (council decision 5: one DB is a single
    /// failure domain, so keep a recoverable snapshot). `VACUUM INTO` writes a
    /// fully-consistent copy without blocking writers for long.
    pub fn backup_to(&self, path: &Path) -> Result<()> {
        self.conn
            .execute("VACUUM INTO ?1", [path.to_string_lossy().as_ref()])?;
        Ok(())
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

fn session_row_from(row: &rusqlite::Row<'_>) -> rusqlite::Result<SessionRow> {
    Ok(SessionRow {
        id: row.get(0)?,
        cwd: row.get(1)?,
        model: row.get(2)?,
        created_at: row.get(3)?,
        updated_at: row.get(4)?,
        first_prompt: row.get(5)?,
        title: row.get(6)?,
        message_count: row.get(7)?,
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

fn session_artifact_from(row: &rusqlite::Row<'_>) -> rusqlite::Result<SessionArtifactRow> {
    Ok(SessionArtifactRow {
        session_id: row.get(0)?,
        kind: row.get(1)?,
        key: row.get(2)?,
        value_json: row.get(3)?,
        created_at_ms: row.get(4)?,
        updated_at_ms: row.get(5)?,
    })
}

fn session_artifact_event_from(
    row: &rusqlite::Row<'_>,
) -> rusqlite::Result<SessionArtifactEventRow> {
    Ok(SessionArtifactEventRow {
        id: row.get(0)?,
        session_id: row.get(1)?,
        kind: row.get(2)?,
        key: row.get(3)?,
        value_json: row.get(4)?,
        created_at_ms: row.get(5)?,
    })
}

fn collect_rows<T>(
    rows: rusqlite::MappedRows<'_, impl FnMut(&rusqlite::Row<'_>) -> rusqlite::Result<T>>,
) -> Result<Vec<T>> {
    let mut out = Vec::new();
    for row in rows {
        out.push(row?);
    }
    Ok(out)
}

fn insert_derived_session_rows(
    tx: &rusqlite::Transaction<'_>,
    row: &SessionRow,
    messages: &[SessionMessage],
) -> Result<()> {
    let created_at_ms = record::now_ms();
    insert_session_events(tx, row, messages, created_at_ms)?;
    insert_session_turns(tx, row, messages, created_at_ms)?;
    insert_session_tool_runs(tx, row, messages, created_at_ms)?;
    Ok(())
}

fn insert_session_events(
    tx: &rusqlite::Transaction<'_>,
    row: &SessionRow,
    messages: &[SessionMessage],
    created_at_ms: i64,
) -> Result<()> {
    let mut stmt = tx.prepare(
        "INSERT INTO session_events (id, session_id, seq, kind, created_at_ms, payload) \
         VALUES (?1,?2,?3,?4,?5,?6)",
    )?;
    for message in messages {
        let id = deterministic_session_row_id("message", &row.id, message.seq, 0);
        let kind = format!("message:{}", message.role);
        let payload = session_event_payload(message);
        stmt.execute(rusqlite::params![
            id,
            row.id,
            message.seq,
            kind,
            created_at_ms,
            payload
        ])?;
    }
    Ok(())
}

fn insert_session_turns(
    tx: &rusqlite::Transaction<'_>,
    row: &SessionRow,
    messages: &[SessionMessage],
    created_at_ms: i64,
) -> Result<()> {
    let mut stmt = tx.prepare(
        "INSERT INTO session_turns \
         (session_id, turn_index, user_seq, assistant_seq, user_text, assistant_text, status, \
          model, created_at_ms) \
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)",
    )?;
    let mut pending_user: Option<(i64, String)> = None;
    let mut turn_index = 0_i64;
    for message in messages {
        match message.role.as_str() {
            "user" => {
                if let Some((seq, text)) = pending_user.take() {
                    stmt.execute(rusqlite::params![
                        row.id,
                        turn_index,
                        seq,
                        Option::<i64>::None,
                        text,
                        "",
                        "open",
                        row.model,
                        created_at_ms,
                    ])?;
                    turn_index += 1;
                }
                pending_user = Some((message.seq, message.content.clone()));
            }
            "assistant" => {
                if let Some((seq, text)) = pending_user.take() {
                    stmt.execute(rusqlite::params![
                        row.id,
                        turn_index,
                        seq,
                        message.seq,
                        text,
                        message.content,
                        "complete",
                        row.model,
                        created_at_ms,
                    ])?;
                    turn_index += 1;
                }
            }
            _ => {}
        }
    }
    if let Some((seq, text)) = pending_user.take() {
        stmt.execute(rusqlite::params![
            row.id,
            turn_index,
            seq,
            Option::<i64>::None,
            text,
            "",
            "open",
            row.model,
            created_at_ms,
        ])?;
    }
    Ok(())
}

fn insert_session_tool_runs(
    tx: &rusqlite::Transaction<'_>,
    row: &SessionRow,
    messages: &[SessionMessage],
    created_at_ms: i64,
) -> Result<()> {
    let mut stmt = tx.prepare(
        "INSERT INTO session_tool_runs \
         (id, session_id, message_seq, part_index, tool_call_id, runtime_id, kind, status, \
          input_json, output_json, duration_ms, created_at_ms) \
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12)",
    )?;
    let mut event_stmt = tx.prepare(
        "INSERT INTO session_events (id, session_id, seq, kind, created_at_ms, payload) \
         VALUES (?1,?2,?3,?4,?5,?6)",
    )?;
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
            stmt.execute(rusqlite::params![
                id,
                row.id,
                message.seq,
                part_index,
                tool_call_id,
                runtime_id,
                kind,
                status,
                input_json,
                output_json,
                duration_ms,
                created_at_ms,
            ])?;
            let event_id =
                deterministic_session_row_id("tool_event", &row.id, message.seq, part_index);
            event_stmt.execute(rusqlite::params![
                event_id,
                row.id,
                message.seq,
                "tool_run",
                created_at_ms,
                tool.to_string(),
            ])?;
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
pub fn auto_maintain(
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
}

/// Like [`auto_maintain`] but `force` bypasses the throttle (manual `/knowledge
/// mine`).
pub fn auto_maintain_forced(
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
}

fn auto_maintain_inner(
    project_root: &Path,
    _sessions_dir: Option<&Path>,
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
        items.extend(import::scan_markdown_dir(
            dir,
            Scope::Project,
            Some(project.clone()),
        ));
    }
    if !items.is_empty() {
        report.imported = store.import_memories(&items)?.imported;
    }

    // 2. Mine DB-backed session history.
    let (lessons, mine_report) = session_mine::mine_store(&store, 10_000);
    report.sessions_scanned = mine_report.sessions_scanned;
    let (ins, comp) = store.ingest_mined(&project, &lessons)?;
    report.mined_inserted = ins;
    report.mined_compounded = comp;

    // 3. Consolidate duplicates (dedup only — no decay; the store grows).
    report.consolidated = store.consolidate()?;
    report.auto_promoted = store.auto_promote(DEFAULT_AUTO_PROMOTE_SUPPORT)?;

    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Mutex, MutexGuard, OnceLock};

    fn rec(scope: Scope, project: Option<&str>, title: &str, body: &str) -> KnowledgeRecord {
        KnowledgeRecord::new(Kind::Fact, scope, project.map(str::to_owned), title, body)
    }

    #[test]
    fn insert_and_recall_round_trip_normal() {
        let store = KnowledgeStore::open_in_memory().unwrap();
        let r = rec(
            Scope::Global,
            None,
            "Rust edition",
            "This workspace uses edition 2024",
        )
        .with_confidence(0.9);
        store.insert(&r).unwrap();

        let hits = store.recall("edition", &RecallFilter::default()).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].id, r.id);
        assert_eq!(hits[0].title, "Rust edition");
    }

    // SAFETY INVARIANT (PLAN §3.3): a record reaches global scope ONLY via the
    // explicit promote() gate, never via insert at runtime.
    #[test]
    fn project_record_is_not_global_until_promoted_regression() {
        let store = KnowledgeStore::open_in_memory().unwrap();
        let r = rec(
            Scope::Project,
            Some("projA"),
            "local lesson",
            "use ripgrep here",
        );
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
        store
            .insert(&rec(Scope::Project, Some("A"), "a-only", "alpha secret"))
            .unwrap();
        store
            .insert(&rec(Scope::Project, Some("B"), "b-only", "beta secret"))
            .unwrap();
        store
            .insert(&rec(Scope::User, None, "user-pref", "alpha beta gamma"))
            .unwrap();

        let from_a = RecallFilter {
            project_key: Some("A"),
            limit: 8,
        };
        let hits = store.recall("alpha", &from_a).unwrap();
        let titles: Vec<_> = hits.iter().map(|h| h.title.as_str()).collect();
        assert!(titles.contains(&"a-only"), "{titles:?}");
        assert!(titles.contains(&"user-pref"), "{titles:?}");
        assert!(
            !titles.contains(&"b-only"),
            "B's row leaked into A: {titles:?}"
        );
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
        assert!(store.insert(&rec(Scope::Global, None, "t", "  ")).is_err());
        assert!(store.insert(&rec(Scope::Project, None, "t", "b")).is_err());
        assert!(
            store
                .insert(&rec(Scope::Global, Some("x"), "t", "b"))
                .is_err()
        );
        let mut bad = rec(Scope::Global, None, "t", "b");
        bad.confidence = 5.0;
        assert!(store.insert(&bad).is_err());
    }

    #[test]
    fn insert_rejects_unredacted_secrets_before_sqlite_regression() {
        let store = KnowledgeStore::open_in_memory().unwrap();
        let raw_secret = rec(
            Scope::User,
            None,
            "credential",
            "token=ghp_0123456789abcdefghij",
        );

        assert!(store.insert(&raw_secret).is_err());
        assert_eq!(store.live_count().unwrap(), 0);
    }

    // SAFETY INVARIANT (PLAN §3.4): bounded growth. Insert well past the cap and
    // assert decay holds the live count at/under it, and that a promoted/global
    // row survives the cull.
    #[test]
    fn decay_enforces_row_cap_and_spares_promoted_regression() {
        let mut store = KnowledgeStore::open_in_memory().unwrap();
        let mut keep = rec(Scope::Global, None, "promoted keeper", "must survive decay");
        keep.promoted = true;
        store.insert(&keep).unwrap();

        for i in 0..50 {
            store
                .insert(&rec(
                    Scope::Project,
                    Some("P"),
                    &format!("row {i}"),
                    "filler body",
                ))
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
                .unwrap();
        }
        assert_eq!(store.live_count().unwrap(), 101);

        let removed = store.decay(DEFAULT_MAX_AGE_MS, 10).unwrap();
        assert!(
            removed >= 80,
            "decay should prune project/global rows over the cap; removed={removed}"
        );
        assert!(store.live_count().unwrap() <= 21);

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
            store.mark_used(std::slice::from_ref(&a.id)).unwrap();
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
        assert_eq!(store.memory_count().unwrap(), 2);
        let rows = store.load_memories(Some("P")).unwrap();
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
        let r2 = store.import_memories(&items).unwrap();
        assert_eq!(r2.imported, 0);
        assert_eq!(r2.skipped, 2);
        assert_eq!(
            store.live_count().unwrap(),
            2,
            "re-import must not duplicate"
        );
    }

    #[test]
    fn imported_project_memory_is_hidden_from_other_projects_regression() {
        use crate::import::ImportableMemory;
        let store = KnowledgeStore::open_in_memory().unwrap();
        let items = vec![ImportableMemory {
            source_path: None,
            kind: Kind::Fact,
            scope: Scope::Project,
            project_key: Some("P".into()),
            title: "stack".into(),
            body: "This repo uses ratatui.".into(),
        }];

        let report = store.import_memories(&items).unwrap();

        assert_eq!(report.imported, 1);
        assert_eq!(store.load_memories(Some("P")).unwrap().len(), 1);
        assert!(
            store.load_memories(Some("Q")).unwrap().is_empty(),
            "project-scoped imported memories must not leak into unrelated projects"
        );
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
        let fix = rec(
            Scope::Global,
            None,
            "fix",
            "strip the line-number gutter first",
        );
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
        store
            .note_gap("how to mock the network layer", "referenced, no lesson")
            .unwrap();
        store
            .note_gap("how to mock the network layer", "again")
            .unwrap();
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
        let weak = rec(Scope::Project, Some("P"), "dup", "use ripgrep here").with_confidence(0.3);
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
            .recall(
                "ripgrep",
                &crate::query::RecallFilter {
                    project_key: Some("P"),
                    limit: 8,
                },
            )
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
        let (ins, comp) = store
            .ingest_mined("P", std::slice::from_ref(&unverified))
            .unwrap();
        assert_eq!((ins, comp), (1, 0));
        assert_eq!(store.live_count().unwrap(), 1);

        // Same norm_key, now VERIFIED → compounds onto the existing row + upgrades.
        let mut verified = unverified;
        verified.outcome = Outcome::Verified;
        verified.session_id = "s2".into();
        let (ins2, comp2) = store.ingest_mined("P", &[verified]).unwrap();
        assert_eq!((ins2, comp2), (0, 1), "should compound, not duplicate");
        assert_eq!(store.live_count().unwrap(), 1);

        let hits = store
            .recall(
                "retry",
                &crate::query::RecallFilter {
                    project_key: Some("P"),
                    limit: 8,
                },
            )
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
        let mut fact =
            mk(Kind::Fact, "stack", "this repo uses vite").with_outcome(Outcome::Verified);
        fact.use_count = 9;
        store.insert(&fact).unwrap();

        let promoted = store.auto_promote(3).unwrap();
        assert_eq!(
            promoted, 1,
            "only the verified, well-supported, generalizable lesson promotes"
        );

        // The promoted one is now recalled from a DIFFERENT project.
        let other = crate::query::RecallFilter {
            project_key: Some("Q"),
            limit: 8,
        };
        let hits = store.recall("ripgrep", &other).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].id, hot.id);
        // The unverified/rare/fact ones did not leak across projects.
        assert!(store.recall("niche", &other).unwrap().is_empty());
        assert!(store.recall("unconfirmed", &other).unwrap().is_empty());
        assert!(
            store.recall("vite", &other).unwrap().is_empty(),
            "project-specific fact must not leak"
        );
    }

    // The throttle prevents re-processing on a second startup within the window.
    #[test]
    fn maintain_throttle_blocks_rapid_repeat_normal() {
        let store = KnowledgeStore::open_in_memory().unwrap();
        assert!(
            store.maintain_due("P", MAINTAIN_THROTTLE_MS).unwrap(),
            "first run is due"
        );
        store.stamp_maintain("P").unwrap();
        assert!(
            !store.maintain_due("P", MAINTAIN_THROTTLE_MS).unwrap(),
            "just-stamped is not due"
        );
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
        std::fs::write(
            user_mem.join("p.md"),
            "---\ntype: preference\n---\nuse spaces not tabs",
        )
        .unwrap();
        // Isolate the global store path for this test.
        let _guard = EnvGuard::set("JFC_KNOWLEDGE_DB", dbpath.to_str().unwrap());
        let mut store = KnowledgeStore::open(&dbpath).unwrap();
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
            .unwrap();
        drop(store);

        let report = auto_maintain(dir.path(), None, Some(&user_mem), None).unwrap();
        assert_eq!(report.imported, 1, "imported the .md preference");
        assert_eq!(report.sessions_scanned, 1);
        assert!(report.mined_inserted >= 1, "mined the recovered Edit error");

        // The store actually grew and persists.
        let store = KnowledgeStore::open(&dbpath).unwrap();
        assert!(store.live_count().unwrap() >= 2);
    }

    #[test]
    fn auto_maintain_promotes_proven_generalizable_lessons_regression() {
        let dir = tempfile::tempdir().unwrap();
        let dbpath = dir.path().join("k.db");
        let _guard = EnvGuard::set("JFC_KNOWLEDGE_DB", dbpath.to_str().unwrap());
        let project = project::project_key(dir.path());
        let store = KnowledgeStore::open(&dbpath).unwrap();
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
        store.insert(&lesson).unwrap();
        drop(store);

        let report = auto_maintain(dir.path(), None, None, None).unwrap();

        assert_eq!(report.auto_promoted, 1);
        let store = KnowledgeStore::open(&dbpath).unwrap();
        let hits = store
            .recall(
                "ripgrep",
                &RecallFilter {
                    project_key: Some("other-project"),
                    limit: 8,
                },
            )
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
    #[test]
    fn session_transcript_roundtrip_and_search_normal() {
        let mut store = KnowledgeStore::open_in_memory().unwrap();
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
        store.replace_transcript(&hdr, &msgs).unwrap();

        // Round-trips in order with meta intact.
        let loaded = store.load_transcript("ses_x").unwrap();
        assert_eq!(loaded, msgs);
        assert!(store.has_transcript("ses_x").unwrap());
        assert_eq!(
            store.session_count().unwrap(),
            1,
            "header upserted in same txn"
        );

        // FTS search finds the session by a content term.
        assert_eq!(
            store.search_transcripts("ripgrep", 10).unwrap(),
            vec!["ses_x".to_string()]
        );
        assert!(
            store
                .search_transcripts("nonexistentterm", 10)
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
            .unwrap();
        assert_eq!(store.load_transcript("ses_x").unwrap(), shorter);
        assert!(
            store.search_transcripts("ripgrep", 10).unwrap().is_empty(),
            "old content gone from FTS"
        );

        let deleted = store.delete_session("ses_x").unwrap();
        assert!(deleted >= 2);
        assert_eq!(store.session_count().unwrap(), 0);
        assert!(store.load_transcript("ses_x").unwrap().is_empty());

        store
            .replace_transcript(
                &SessionRow {
                    message_count: 1,
                    ..hdr.clone()
                },
                &shorter,
            )
            .unwrap();

        // Backup writes a consistent, openable copy.
        let dir = tempfile::tempdir().unwrap();
        let bpath = dir.path().join("backup.db");
        store.backup_to(&bpath).unwrap();
        let restored = KnowledgeStore::open(&bpath).unwrap();
        assert_eq!(restored.load_transcript("ses_x").unwrap(), shorter);
    }

    #[test]
    fn session_event_substrate_is_derived_from_transcript_normal() {
        let mut store = KnowledgeStore::open_in_memory().unwrap();
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

        store.replace_transcript(&hdr, &msgs).unwrap();

        let events = store.list_session_events("ses_events", 20).unwrap();
        assert_eq!(events.len(), 3);
        assert!(events.iter().any(|event| event.kind == "message:user"));
        assert!(events.iter().any(|event| event.kind == "tool_run"));

        let turns = store.list_session_turns("ses_events").unwrap();
        assert_eq!(turns.len(), 1);
        assert_eq!(turns[0].user_text, "run tests");
        assert_eq!(turns[0].assistant_text, "done");
        assert_eq!(turns[0].status, "complete");
        assert_eq!(turns[0].model.as_deref(), Some("claude-sonnet-4"));

        let tools = store.list_session_tool_runs("ses_events").unwrap();
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0].tool_call_id.as_deref(), Some("toolu_1"));
        assert_eq!(tools[0].runtime_id.as_deref(), Some("bash_123"));
        assert_eq!(tools[0].kind, "Bash");
        assert_eq!(tools[0].status, "success");
        assert_eq!(tools[0].duration_ms, Some(742));

        let context = store.list_context_events(Some("ses_events"), 20).unwrap();
        assert_eq!(context.len(), 1);
        assert_eq!(context[0].turn_id.as_deref(), Some("ses_events:1"));
        assert_eq!(context[0].model, "anthropic/claude-opus-4-7");
        assert_eq!(context[0].input_tokens, 12000);
        assert_eq!(context[0].output_tokens, 600);
        assert_eq!(context[0].thinking_tokens, 42);
        assert_eq!(context[0].bust_cause.as_deref(), Some("cache_miss"));
    }

    #[test]
    fn agent_learning_substrate_roundtrips_normal() {
        let mut store = KnowledgeStore::open_in_memory().unwrap();
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
        store.upsert_agent_session(&agent).unwrap();

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
        store.record_agent_event(&event).unwrap();

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
        store.enqueue_agent_mailbox(&mail).unwrap();

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
        store.record_tool_run(&run).unwrap();

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
        store.record_learning_event(&learning).unwrap();

        assert_eq!(
            store.get_agent_session("agent_advisor_1").unwrap(),
            Some(agent)
        );
        assert_eq!(
            store.list_agent_events("ses_parent", 10).unwrap(),
            vec![event]
        );
        assert_eq!(
            store.list_agent_mailbox("agent_advisor_1", true).unwrap(),
            vec![mail.clone()]
        );
        assert_eq!(store.mark_agent_mailbox_read("mail_1").unwrap(), 1);
        assert!(
            store
                .list_agent_mailbox("agent_advisor_1", true)
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            store.list_learning_events(Some("candidate"), 10).unwrap(),
            vec![learning]
        );

        let deleted = store.delete_session("ses_parent").unwrap();
        assert!(deleted >= 3);
        assert!(
            store
                .list_agent_events("ses_parent", 10)
                .unwrap()
                .is_empty()
        );
        assert!(
            store
                .list_learning_events(Some("candidate"), 10)
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn session_index_upsert_and_list_normal() {
        let store = KnowledgeStore::open_in_memory().unwrap();
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
            .unwrap();
        store
            .upsert_session(&row("s2", "/a", "2026-01-01T03:00:00Z", 4))
            .unwrap();
        store
            .upsert_session(&row("s3", "/b", "2026-01-01T02:00:00Z", 6))
            .unwrap();
        assert_eq!(store.session_count().unwrap(), 3);

        // Re-upsert s1 with a new message_count → updates, not duplicates.
        store
            .upsert_session(&row("s1", "/a", "2026-01-01T05:00:00Z", 9))
            .unwrap();
        assert_eq!(store.session_count().unwrap(), 3);
        assert_eq!(store.get_session("s1").unwrap().unwrap().message_count, 9);

        // List for /a, most-recently-updated first (s1 now newest after re-upsert).
        let a = store.list_sessions(Some("/a"), 10).unwrap();
        assert_eq!(
            a.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(),
            ["s1", "s2"]
        );
        // Global list includes all three.
        assert_eq!(store.list_sessions(None, 10).unwrap().len(), 3);
        // Unknown cwd → empty.
        assert!(store.list_sessions(Some("/nope"), 10).unwrap().is_empty());
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
