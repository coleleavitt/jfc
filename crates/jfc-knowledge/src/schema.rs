//! Embedded schema migrations for the knowledge store.
//!
//! Migrations are an ordered list; the `schema_version` table records how many
//! have been applied. `migrate` applies any not-yet-applied steps inside a
//! transaction, so an interrupted upgrade never leaves a half-migrated DB.
//! Adding a new version = append a `&str` to [`MIGRATIONS`]; never edit or
//! reorder existing entries.

use sqlx::AssertSqlSafe;
use sqlx::Row;
use sqlx::sqlite::SqlitePool;

use crate::error::{KnowledgeError, Result};

/// Ordered DDL steps. Index + 1 is the resulting schema version.
const MIGRATIONS: &[&str] = &[
    // v1 — initial schema: knowledge table + FTS5 mirror + triggers.
    r#"
    CREATE TABLE knowledge (
        id            TEXT PRIMARY KEY,
        kind          TEXT NOT NULL,
        scope         TEXT NOT NULL,
        project_key   TEXT,
        title         TEXT NOT NULL,
        body          TEXT NOT NULL,
        tags          TEXT NOT NULL DEFAULT '',
        source        TEXT,
        confidence    REAL NOT NULL DEFAULT 0.5,
        created_at_ms INTEGER NOT NULL,
        last_used_ms  INTEGER,
        use_count     INTEGER NOT NULL DEFAULT 0,
        superseded_by TEXT,
        promoted      INTEGER NOT NULL DEFAULT 0
    );
    CREATE INDEX idx_knowledge_scope ON knowledge(scope);
    CREATE INDEX idx_knowledge_project ON knowledge(project_key);
    CREATE INDEX idx_knowledge_live ON knowledge(superseded_by);

    CREATE VIRTUAL TABLE knowledge_fts USING fts5(
        title, body, tags,
        content='knowledge', content_rowid='rowid'
    );

    -- Keep the FTS mirror in sync with the base table.
    CREATE TRIGGER knowledge_ai AFTER INSERT ON knowledge BEGIN
        INSERT INTO knowledge_fts(rowid, title, body, tags)
        VALUES (new.rowid, new.title, new.body, new.tags);
    END;
    CREATE TRIGGER knowledge_ad AFTER DELETE ON knowledge BEGIN
        INSERT INTO knowledge_fts(knowledge_fts, rowid, title, body, tags)
        VALUES ('delete', old.rowid, old.title, old.body, old.tags);
    END;
    CREATE TRIGGER knowledge_au AFTER UPDATE ON knowledge BEGIN
        INSERT INTO knowledge_fts(knowledge_fts, rowid, title, body, tags)
        VALUES ('delete', old.rowid, old.title, old.body, old.tags);
        INSERT INTO knowledge_fts(rowid, title, body, tags)
        VALUES (new.rowid, new.title, new.body, new.tags);
    END;
    "#,
    // v2 — verification outcome + salience (TODOs 7,8), the Obsidian-style typed
    // link-graph (TODO 14), and unresolved-reference knowledge gaps (TODO 15).
    r#"
    ALTER TABLE knowledge ADD COLUMN outcome TEXT NOT NULL DEFAULT 'unverified';
    ALTER TABLE knowledge ADD COLUMN importance REAL NOT NULL DEFAULT 0.5;
    CREATE INDEX idx_knowledge_outcome ON knowledge(outcome);

    CREATE TABLE knowledge_links (
        from_id    TEXT NOT NULL,
        to_id      TEXT NOT NULL,
        rel        TEXT NOT NULL,
        created_at_ms INTEGER NOT NULL,
        PRIMARY KEY (from_id, to_id, rel)
    );
    CREATE INDEX idx_links_from ON knowledge_links(from_id);
    CREATE INDEX idx_links_to ON knowledge_links(to_id);

    CREATE TABLE knowledge_gaps (
        id            TEXT PRIMARY KEY,
        label         TEXT NOT NULL,
        reason        TEXT NOT NULL DEFAULT '',
        ref_count     INTEGER NOT NULL DEFAULT 1,
        first_seen_ms INTEGER NOT NULL,
        last_seen_ms  INTEGER NOT NULL,
        resolved_by   TEXT
    );
    "#,
    // v3 — per-project maintenance throttle stamp, so autonomous startup
    // maintenance doesn't re-process the whole session corpus every launch.
    r#"
    CREATE TABLE maintain_state (
        project_key  TEXT PRIMARY KEY,
        last_run_ms  INTEGER NOT NULL
    );
    "#,
    // v4 — primary session catalog. Legacy JSON files may still be backfilled
    // into this table, but picker/search readers should treat the DB as source.
    r#"
    CREATE TABLE sessions (
        id            TEXT PRIMARY KEY,
        cwd           TEXT,
        model         TEXT,
        created_at    TEXT,
        updated_at    TEXT,
        first_prompt  TEXT,
        title         TEXT,
        message_count INTEGER NOT NULL DEFAULT 0
    );
    CREATE INDEX idx_sessions_cwd ON sessions(cwd);
    CREATE INDEX idx_sessions_updated ON sessions(updated_at);
    "#,
    // v5 — full session TRANSCRIPT (PLAN TODO 23, council decision 1: row-per-
    // message, not a blob, so search is a query and saves append deltas instead
    // of rewriting the whole session). `meta` holds serialized tool-call/parts
    // JSON so structured fidelity survives the round trip. FTS5 mirror powers
    // substring search.
    r#"
    CREATE TABLE session_messages (
        session_id TEXT NOT NULL,
        seq        INTEGER NOT NULL,
        role       TEXT NOT NULL,
        content    TEXT NOT NULL DEFAULT '',
        meta       TEXT,
        PRIMARY KEY (session_id, seq)
    );
    CREATE INDEX idx_session_messages_sid ON session_messages(session_id);

    CREATE VIRTUAL TABLE session_messages_fts USING fts5(
        content, content='session_messages', content_rowid='rowid'
    );
    CREATE TRIGGER session_messages_ai AFTER INSERT ON session_messages BEGIN
        INSERT INTO session_messages_fts(rowid, content) VALUES (new.rowid, new.content);
    END;
    CREATE TRIGGER session_messages_ad AFTER DELETE ON session_messages BEGIN
        INSERT INTO session_messages_fts(session_messages_fts, rowid, content)
        VALUES ('delete', old.rowid, old.content);
    END;
    "#,
    // v6 — memory backing (MD→DB cutover). `mem_level` distinguishes the four
    // memory levels (user|project|team|external) that the old .md layout encoded
    // by directory; the knowledge `scope` axis (user/project/global) is coarser.
    // `mem_meta` holds the serialized MemoryFrontmatter JSON so a `MemoryEntry`
    // synthesized from a row is lossless (source_session_id, seen_count,
    // verification_status, expires_at, …). Both nullable: rows that aren't
    // memories (mined lessons, etc.) leave them NULL.
    r#"
    ALTER TABLE knowledge ADD COLUMN mem_level TEXT;
    ALTER TABLE knowledge ADD COLUMN mem_meta TEXT;
    CREATE INDEX idx_knowledge_mem_level ON knowledge(mem_level);
    "#,
    // v7 — session event substrate. The transcript remains the renderable view,
    // while these tables make turns, tool runs, retrieval, compaction, and
    // findings queryable facts for maintenance and learning.
    r#"
    CREATE TABLE session_events (
        id            TEXT PRIMARY KEY,
        session_id    TEXT NOT NULL,
        seq           INTEGER NOT NULL,
        kind          TEXT NOT NULL,
        created_at_ms INTEGER NOT NULL,
        payload       TEXT NOT NULL DEFAULT '{}'
    );
    CREATE INDEX idx_session_events_sid_seq ON session_events(session_id, seq);
    CREATE INDEX idx_session_events_kind ON session_events(kind);

    CREATE TABLE session_turns (
        session_id       TEXT NOT NULL,
        turn_index       INTEGER NOT NULL,
        user_seq         INTEGER,
        assistant_seq    INTEGER,
        user_text        TEXT NOT NULL DEFAULT '',
        assistant_text   TEXT NOT NULL DEFAULT '',
        status           TEXT NOT NULL DEFAULT 'complete',
        model            TEXT,
        created_at_ms    INTEGER NOT NULL,
        PRIMARY KEY (session_id, turn_index)
    );
    CREATE INDEX idx_session_turns_sid ON session_turns(session_id);

    CREATE TABLE session_tool_runs (
        id              TEXT PRIMARY KEY,
        session_id      TEXT NOT NULL,
        message_seq     INTEGER NOT NULL,
        part_index      INTEGER NOT NULL,
        tool_call_id    TEXT,
        runtime_id      TEXT,
        kind            TEXT NOT NULL,
        status          TEXT NOT NULL,
        input_json      TEXT,
        output_json     TEXT,
        duration_ms     INTEGER,
        created_at_ms   INTEGER NOT NULL
    );
    CREATE INDEX idx_session_tool_runs_sid ON session_tool_runs(session_id);
    CREATE INDEX idx_session_tool_runs_status ON session_tool_runs(status);

    CREATE TABLE session_retrieval_events (
        id              TEXT PRIMARY KEY,
        session_id      TEXT NOT NULL,
        query           TEXT NOT NULL,
        source          TEXT NOT NULL,
        result_count    INTEGER NOT NULL DEFAULT 0,
        payload         TEXT NOT NULL DEFAULT '{}',
        created_at_ms   INTEGER NOT NULL
    );
    CREATE INDEX idx_session_retrieval_events_sid ON session_retrieval_events(session_id);

    CREATE TABLE session_compactions (
        id              TEXT PRIMARY KEY,
        session_id      TEXT NOT NULL,
        before_tokens   INTEGER,
        after_tokens    INTEGER,
        summary         TEXT NOT NULL DEFAULT '',
        payload         TEXT NOT NULL DEFAULT '{}',
        created_at_ms   INTEGER NOT NULL
    );
    CREATE INDEX idx_session_compactions_sid ON session_compactions(session_id);

    CREATE TABLE session_findings (
        id              TEXT PRIMARY KEY,
        session_id      TEXT NOT NULL,
        kind            TEXT NOT NULL,
        summary         TEXT NOT NULL,
        evidence        TEXT NOT NULL DEFAULT '',
        status          TEXT NOT NULL DEFAULT 'open',
        created_at_ms   INTEGER NOT NULL,
        resolved_at_ms  INTEGER
    );
    CREATE INDEX idx_session_findings_sid ON session_findings(session_id);
    CREATE INDEX idx_session_findings_status ON session_findings(status);
    "#,
    // v8 — session artifact/event substrate for the remaining legacy sidecars.
    // `session_artifacts` stores latest state (goal, task snapshot, compact
    // archive body), while `session_artifact_events` stores append-only streams
    // (inbox, prompt rewrite exemplars, workflow journal entries).
    r#"
    CREATE TABLE session_artifacts (
        session_id    TEXT NOT NULL,
        kind          TEXT NOT NULL,
        key           TEXT NOT NULL,
        value_json    TEXT NOT NULL,
        created_at_ms INTEGER NOT NULL,
        updated_at_ms INTEGER NOT NULL,
        PRIMARY KEY (session_id, kind, key)
    );
    CREATE INDEX idx_session_artifacts_sid_kind ON session_artifacts(session_id, kind);

    CREATE TABLE session_artifact_events (
        id            INTEGER PRIMARY KEY AUTOINCREMENT,
        session_id    TEXT NOT NULL,
        kind          TEXT NOT NULL,
        key           TEXT NOT NULL,
        value_json    TEXT NOT NULL,
        created_at_ms INTEGER NOT NULL
    );
    CREATE INDEX idx_session_artifact_events_sid_kind
        ON session_artifact_events(session_id, kind, key, id);
    "#,
    // v9 — unified agent/context/learning substrate. This is the shared
    // persistence layer for advisor, council, bounty, teams, hidden historian
    // workers, and Magic-Context-style diagnostics. User-facing concepts stay
    // distinct; their runtime state lands in one event model.
    r#"
    CREATE TABLE agent_sessions (
        id                 TEXT PRIMARY KEY,
        parent_session_id  TEXT,
        role               TEXT NOT NULL,
        model              TEXT,
        status             TEXT NOT NULL,
        budget_tokens      INTEGER,
        task_id            TEXT,
        team_id            TEXT,
        created_at_ms      INTEGER NOT NULL,
        updated_at_ms      INTEGER NOT NULL
    );
    CREATE INDEX idx_agent_sessions_parent ON agent_sessions(parent_session_id);
    CREATE INDEX idx_agent_sessions_team ON agent_sessions(team_id);
    CREATE INDEX idx_agent_sessions_status ON agent_sessions(status);

    CREATE TABLE agent_events (
        id                TEXT PRIMARY KEY,
        session_id         TEXT NOT NULL,
        from_agent         TEXT,
        to_agent           TEXT,
        kind               TEXT NOT NULL,
        content            TEXT NOT NULL DEFAULT '',
        turn_id            TEXT,
        causal_parent_id   TEXT,
        created_at_ms      INTEGER NOT NULL
    );
    CREATE INDEX idx_agent_events_session ON agent_events(session_id, created_at_ms);
    CREATE INDEX idx_agent_events_to_agent ON agent_events(to_agent, created_at_ms);

    CREATE TABLE agent_mailbox (
        id                TEXT PRIMARY KEY,
        to_agent           TEXT NOT NULL,
        from_agent         TEXT,
        thread_id          TEXT,
        task_id            TEXT,
        priority           INTEGER NOT NULL DEFAULT 0,
        content            TEXT NOT NULL DEFAULT '',
        read_at_ms         INTEGER,
        summarized_at_ms   INTEGER,
        created_at_ms      INTEGER NOT NULL
    );
    CREATE INDEX idx_agent_mailbox_to_agent ON agent_mailbox(to_agent, read_at_ms, priority, created_at_ms);
    CREATE INDEX idx_agent_mailbox_thread ON agent_mailbox(thread_id);

    CREATE TABLE tool_runs (
        id                TEXT PRIMARY KEY,
        agent_id           TEXT,
        session_id         TEXT,
        runtime_id         TEXT,
        kind               TEXT NOT NULL,
        command            TEXT,
        input_json         TEXT,
        output_ref         TEXT,
        status             TEXT NOT NULL,
        duration_ms        INTEGER,
        background         INTEGER NOT NULL DEFAULT 0,
        created_at_ms      INTEGER NOT NULL,
        updated_at_ms      INTEGER NOT NULL
    );
    CREATE INDEX idx_tool_runs_session ON tool_runs(session_id, created_at_ms);
    CREATE INDEX idx_tool_runs_runtime ON tool_runs(runtime_id);
    CREATE INDEX idx_tool_runs_status ON tool_runs(status);

    CREATE TABLE learning_events (
        id                  TEXT PRIMARY KEY,
        source_session_id   TEXT,
        source_turn_id      TEXT,
        source_tool_run_id  TEXT,
        candidate_rule      TEXT NOT NULL,
        status              TEXT NOT NULL,
        verifier_evidence   TEXT NOT NULL DEFAULT '',
        recurrence_count    INTEGER NOT NULL DEFAULT 0,
        created_at_ms       INTEGER NOT NULL,
        updated_at_ms       INTEGER NOT NULL
    );
    CREATE INDEX idx_learning_events_status ON learning_events(status, updated_at_ms);
    CREATE INDEX idx_learning_events_source_session ON learning_events(source_session_id);

    CREATE TABLE context_events (
        id                  TEXT PRIMARY KEY,
        session_id           TEXT NOT NULL,
        turn_id              TEXT,
        agent_id             TEXT,
        subagent_id          TEXT,
        model                TEXT NOT NULL,
        input_tokens         INTEGER NOT NULL DEFAULT 0,
        output_tokens        INTEGER NOT NULL DEFAULT 0,
        thinking_tokens      INTEGER NOT NULL DEFAULT 0,
        cache_read_tokens    INTEGER NOT NULL DEFAULT 0,
        cache_write_tokens   INTEGER NOT NULL DEFAULT 0,
        context_limit        INTEGER,
        bust_cause           TEXT,
        drop_cause           TEXT,
        payload              TEXT NOT NULL DEFAULT '{}',
        created_at_ms        INTEGER NOT NULL
    );
    CREATE INDEX idx_context_events_session ON context_events(session_id, created_at_ms);
    CREATE INDEX idx_context_events_model ON context_events(model, created_at_ms);
    CREATE INDEX idx_context_events_agent ON context_events(agent_id, created_at_ms);
    "#,
    // v10 — runtime definition store. Prompt text, skills, agents, slash
    // commands, and tool-schema snapshots are durable editable definitions now;
    // legacy Markdown files are import sources, not the canonical runtime state.
    r#"
    CREATE TABLE definitions (
        id              TEXT PRIMARY KEY,
        kind            TEXT NOT NULL,
        scope           TEXT NOT NULL,
        project_key     TEXT,
        namespace       TEXT,
        name            TEXT NOT NULL,
        title           TEXT,
        description     TEXT,
        body            TEXT NOT NULL,
        metadata_json   TEXT NOT NULL DEFAULT '{}',
        source_path     TEXT,
        source_hash     TEXT,
        status          TEXT NOT NULL DEFAULT 'active',
        version         INTEGER NOT NULL DEFAULT 1,
        created_by      TEXT NOT NULL DEFAULT 'import',
        created_at_ms   INTEGER NOT NULL,
        updated_at_ms   INTEGER NOT NULL,
        superseded_by   TEXT
    );
    CREATE INDEX idx_definitions_kind_name ON definitions(kind, name);
    CREATE INDEX idx_definitions_project ON definitions(project_key, kind);
    CREATE INDEX idx_definitions_status ON definitions(status, updated_at_ms);
    CREATE INDEX idx_definitions_source_hash ON definitions(source_hash);

    CREATE VIRTUAL TABLE definitions_fts USING fts5(
        name, description, body, metadata_json,
        content='definitions', content_rowid='rowid'
    );
    CREATE TRIGGER definitions_ai AFTER INSERT ON definitions BEGIN
        INSERT INTO definitions_fts(rowid, name, description, body, metadata_json)
        VALUES (
            new.rowid,
            new.name,
            COALESCE(new.description, ''),
            new.body,
            new.metadata_json
        );
    END;
    CREATE TRIGGER definitions_ad AFTER DELETE ON definitions BEGIN
        INSERT INTO definitions_fts(
            definitions_fts, rowid, name, description, body, metadata_json
        )
        VALUES (
            'delete',
            old.rowid,
            old.name,
            COALESCE(old.description, ''),
            old.body,
            old.metadata_json
        );
    END;
    CREATE TRIGGER definitions_au AFTER UPDATE ON definitions BEGIN
        INSERT INTO definitions_fts(
            definitions_fts, rowid, name, description, body, metadata_json
        )
        VALUES (
            'delete',
            old.rowid,
            old.name,
            COALESCE(old.description, ''),
            old.body,
            old.metadata_json
        );
        INSERT INTO definitions_fts(rowid, name, description, body, metadata_json)
        VALUES (
            new.rowid,
            new.name,
            COALESCE(new.description, ''),
            new.body,
            new.metadata_json
        );
    END;
    "#,
    // v11 — self-improvement BACKLOG. A single trackable ledger of suggestions /
    // optimizations the system proposes — for ITSELF (`scope='self'`, JFC's own
    // reasoning/prompt/skill/tool) and for OTHER projects (`scope='project'`,
    // `project_key` = target). Status moves proposed → proven → applied (or
    // rejected/superseded). `impact_json` records measured before/after metrics
    // so improvement is queryable as numbers, not prose. This is the durable
    // home for what the self-critique loop generates.
    r#"
    CREATE TABLE improvement_backlog (
        id                TEXT PRIMARY KEY,
        scope             TEXT NOT NULL,                       -- 'self' | 'project'
        project_key       TEXT,                                -- target project (NULL for self)
        category          TEXT NOT NULL,                       -- reasoning_policy|system_prompt|skill|tool_definition|optimization|...
        title             TEXT NOT NULL,
        body              TEXT NOT NULL,
        evidence          TEXT NOT NULL DEFAULT '',
        status            TEXT NOT NULL DEFAULT 'proposed',    -- proposed|proven|applied|rejected|superseded
        confidence        REAL NOT NULL DEFAULT 0.5,
        impact_json       TEXT,                                -- measured before/after metrics
        source_session_id TEXT,
        recurrence        INTEGER NOT NULL DEFAULT 1,          -- distinct times re-proposed (evidence weight)
        created_at_ms     INTEGER NOT NULL,
        updated_at_ms     INTEGER NOT NULL,
        applied_at_ms     INTEGER
    );
    CREATE INDEX idx_backlog_scope_status ON improvement_backlog(scope, status);
    CREATE INDEX idx_backlog_project ON improvement_backlog(project_key);
    CREATE INDEX idx_backlog_category ON improvement_backlog(category);
    "#,
    // v12 — EVAL HARNESS (the ground-truth signal). `eval_cases` is a held-out
    // suite of regression scenarios (seeded from real corrections/failures);
    // `eval_runs` records pass/score per case per variant (control vs a
    // candidate self-mutation) over time. This is what turns "I think it's
    // better" into a NUMBER, and what lets promotion verify a fix actually helps
    // (vs. merely recurring) + the watchdog detect a regression.
    r#"
    CREATE TABLE eval_cases (
        id                TEXT PRIMARY KEY,
        source            TEXT NOT NULL,                  -- correction|tool_failure|manual
        prompt            TEXT NOT NULL,                  -- the request / scenario
        failure_mode      TEXT,                           -- what went wrong (to avoid)
        expected          TEXT,                           -- correct behavior / fix
        project_key       TEXT,
        source_session_id TEXT,
        weight            REAL NOT NULL DEFAULT 1.0,       -- recurrence-weighted importance
        created_at_ms     INTEGER NOT NULL
    );
    CREATE INDEX idx_eval_cases_source ON eval_cases(source);

    CREATE TABLE eval_runs (
        id            TEXT PRIMARY KEY,
        eval_id       TEXT NOT NULL,
        variant       TEXT NOT NULL,                       -- control | candidate:<def_id>
        passed        INTEGER,                             -- 1 | 0 | NULL
        score         REAL,
        detail        TEXT,
        run_at_ms     INTEGER NOT NULL
    );
    CREATE INDEX idx_eval_runs_eval ON eval_runs(eval_id, run_at_ms);
    CREATE INDEX idx_eval_runs_variant ON eval_runs(variant, run_at_ms);
    "#,
    // v13 — ERROR-PATTERN SIGNATURES. Pass-rate is a scalar; two variants with
    // the SAME pass-rate can fail in completely different ways. This table
    // records WHICH failure bucket (the shared FailureKind taxonomy:
    // perception|reasoning|knowledge_gap|verification|other) each failed run
    // hit, so promotion/regression decisions compare error *distributions*, not
    // just a single number — and a failure localizes to a capability instead of
    // "it was wrong". `signature` is `<kind>` or `<kind>:<step>` for finer
    // grain; `count` aggregates recurrences of the same signature per variant.
    r#"
    CREATE TABLE eval_error_signatures (
        id            TEXT PRIMARY KEY,
        variant       TEXT NOT NULL,
        eval_id       TEXT NOT NULL,
        signature     TEXT NOT NULL,                       -- <FailureKind>[:<step>]
        count         INTEGER NOT NULL DEFAULT 1,
        first_seen_ms INTEGER NOT NULL,
        last_seen_ms  INTEGER NOT NULL
    );
    CREATE INDEX idx_eval_errsig_variant ON eval_error_signatures(variant, signature);
    "#,
];

/// The schema version this build expects (== number of migrations).
pub const CURRENT_VERSION: i64 = MIGRATIONS.len() as i64;

/// Pragmas every connection needs (WAL for multi-process safety, a busy timeout
/// so concurrent JFC instances back off instead of erroring, foreign-keys on).
///
/// These are also configured on the [`SqliteConnectOptions`] used to build the
/// pool (see `lib.rs`); this helper applies them to an arbitrary pool too —
/// harmless and idempotent — for parity with the old per-open behavior.
///
/// NORMAL synchronous is WAL-safe and durable across an app crash / SIGKILL (an
/// uncommitted txn rolls back on next open); FULL would add fsync cost per commit
/// without a meaningful win for this single-file store.
///
/// Currently the constructors set these on [`SqliteConnectOptions`] directly, so
/// this is retained as a public helper for callers that build a pool by other
/// means (and for parity with the pre-sqlx per-open behavior).
#[allow(dead_code)]
pub async fn apply_pragmas(pool: &SqlitePool) -> Result<()> {
    for pragma in [
        "PRAGMA journal_mode = WAL;",
        "PRAGMA synchronous = NORMAL;",
        "PRAGMA foreign_keys = ON;",
        "PRAGMA busy_timeout = 5000;",
    ] {
        sqlx::query(pragma).execute(pool).await?;
    }
    Ok(())
}

/// Run all pending migrations. Idempotent: a fully-migrated DB is a no-op.
pub async fn migrate(pool: &SqlitePool) -> Result<()> {
    sqlx::query("CREATE TABLE IF NOT EXISTS schema_version (version INTEGER NOT NULL);")
        .execute(pool)
        .await?;

    let applied: i64 = sqlx::query("SELECT COALESCE(MAX(version), 0) FROM schema_version")
        .fetch_one(pool)
        .await?
        .try_get(0)?;

    if applied > CURRENT_VERSION {
        return Err(KnowledgeError::Migration(format!(
            "database schema v{applied} is newer than this build's v{CURRENT_VERSION}; \
             refusing to operate to avoid data loss"
        )));
    }

    for (idx, ddl) in MIGRATIONS.iter().enumerate() {
        let version = (idx + 1) as i64;
        if version <= applied {
            continue;
        }
        let mut tx = pool.begin().await?;
        // Each migration step is a batch of DDL statements; run them one at a
        // time on the transaction connection. We split on `;` boundaries that
        // are not inside a trigger body so FTS5 virtual tables + triggers
        // survive verbatim.
        for stmt in split_sql_statements(ddl) {
            sqlx::query(AssertSqlSafe(stmt)).execute(&mut *tx).await?;
        }
        sqlx::query("INSERT INTO schema_version (version) VALUES (?1)")
            .bind(version)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        tracing::debug!(target: "jfc::knowledge", version, "applied knowledge migration");
    }
    Ok(())
}

/// Split a batch DDL string into individual statements on `;` boundaries that
/// are not inside a `BEGIN ... END` trigger body. SQLite's trigger DDL contains
/// internal `;` (one per body statement) plus a terminating `END`, so a naive
/// split on `;` would truncate a trigger mid-body. We track trigger depth via
/// `BEGIN`/`END` keyword pairs.
fn split_sql_statements(ddl: &str) -> Vec<String> {
    let mut statements = Vec::new();
    let mut current = String::new();
    let mut trigger_depth = 0usize;
    for raw_line in ddl.lines() {
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with("--") {
            continue;
        }
        let upper = line.to_ascii_uppercase();
        if upper.contains("BEGIN") {
            trigger_depth += 1;
        }
        current.push_str(line);
        current.push('\n');
        let ends_stmt = line.ends_with(';');
        if upper.starts_with("END;") || upper == "END" {
            trigger_depth = trigger_depth.saturating_sub(1);
            if trigger_depth == 0 {
                statements.push(std::mem::take(&mut current));
            }
            continue;
        }
        if ends_stmt && trigger_depth == 0 {
            statements.push(std::mem::take(&mut current));
        }
    }
    let tail = current.trim();
    if !tail.is_empty() {
        statements.push(tail.to_owned());
    }
    statements
        .into_iter()
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::KnowledgeStore;

    #[tokio::test]
    async fn migrate_is_idempotent_and_sets_version_normal() {
        let store = KnowledgeStore::open_in_memory().await.unwrap();
        let pool = store.pool();
        // open_in_memory already migrated; a second run is a no-op.
        migrate(pool).await.unwrap();

        let version: i64 = sqlx::query("SELECT MAX(version) FROM schema_version")
            .fetch_one(pool)
            .await
            .unwrap()
            .try_get(0)
            .unwrap();
        assert_eq!(version, CURRENT_VERSION);

        let n: i64 = sqlx::query(
            "SELECT count(*) FROM sqlite_master WHERE name IN ('knowledge','knowledge_fts')",
        )
        .fetch_one(pool)
        .await
        .unwrap()
        .try_get(0)
        .unwrap();
        assert_eq!(n, 2);
    }

    #[test]
    fn split_sql_keeps_trigger_bodies_intact() {
        // The v1 migration has 3 triggers each with an internal `;`. Splitting
        // must not cut a trigger body. Count CREATE TRIGGER survivors.
        let parts = split_sql_statements(MIGRATIONS[0]);
        let triggers = parts
            .iter()
            .filter(|s| s.to_ascii_uppercase().contains("CREATE TRIGGER"))
            .count();
        assert_eq!(triggers, 3, "all three v1 triggers must survive intact");
        for stmt in parts
            .iter()
            .filter(|s| s.to_ascii_uppercase().contains("CREATE TRIGGER"))
        {
            assert!(
                stmt.to_ascii_uppercase().contains("END"),
                "trigger truncated: {stmt}"
            );
        }
    }
}
