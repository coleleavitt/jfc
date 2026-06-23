//! Embedded schema migrations for the knowledge store.
//!
//! Migrations are an ordered list; the `schema_version` table records how many
//! have been applied. `migrate` applies any not-yet-applied steps inside a
//! transaction, so an interrupted upgrade never leaves a half-migrated DB.
//! Adding a new version = append a `&str` to [`MIGRATIONS`]; never edit or
//! reorder existing entries.

use rusqlite::Connection;

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
    // v4 — session index (PLAN TODO 22). ADDITIVE: the canonical session store is
    // still the JSON files; this is a queryable index `save_session` dual-writes
    // so the catalog/picker can avoid byte-scanning every JSON header. No reader
    // depends on it yet.
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
];

/// The schema version this build expects (== number of migrations).
pub const CURRENT_VERSION: i64 = MIGRATIONS.len() as i64;

/// Apply pragmas that every connection needs (WAL for multi-process safety,
/// a busy timeout so concurrent JFC instances back off instead of erroring,
/// and foreign-keys on for future relations).
pub fn apply_pragmas(conn: &Connection) -> Result<()> {
    // WAL persists on the DB file, but setting it per-open is harmless and
    // guarantees it even on a freshly created file.
    conn.pragma_update(None, "journal_mode", "WAL")?;
    conn.pragma_update(None, "synchronous", "NORMAL")?;
    conn.pragma_update(None, "foreign_keys", "ON")?;
    conn.busy_timeout(std::time::Duration::from_secs(5))?;
    Ok(())
}

/// Run all pending migrations. Idempotent: a fully-migrated DB is a no-op.
pub fn migrate(conn: &mut Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS schema_version (version INTEGER NOT NULL);",
    )?;

    let applied: i64 = conn
        .query_row("SELECT COALESCE(MAX(version), 0) FROM schema_version", [], |row| {
            row.get(0)
        })
        .unwrap_or(0);

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
        let tx = conn.transaction()?;
        tx.execute_batch(ddl)?;
        tx.execute("INSERT INTO schema_version (version) VALUES (?1)", [version])?;
        tx.commit()?;
        tracing::debug!(target: "jfc::knowledge", version, "applied knowledge migration");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migrate_is_idempotent_and_sets_version_normal() {
        let mut conn = Connection::open_in_memory().unwrap();
        apply_pragmas(&conn).unwrap();
        migrate(&mut conn).unwrap();
        // Second run is a no-op and must not error or double-apply.
        migrate(&mut conn).unwrap();

        let version: i64 = conn
            .query_row("SELECT MAX(version) FROM schema_version", [], |r| r.get(0))
            .unwrap();
        assert_eq!(version, CURRENT_VERSION);

        // The base table and FTS mirror both exist.
        let n: i64 = conn
            .query_row(
                "SELECT count(*) FROM sqlite_master WHERE name IN ('knowledge','knowledge_fts')",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(n, 2);
    }

    #[test]
    fn migrate_refuses_newer_schema_robust() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE schema_version (version INTEGER NOT NULL);
             INSERT INTO schema_version (version) VALUES (9999);",
        )
        .unwrap();
        let err = migrate(&mut conn).unwrap_err();
        assert!(matches!(err, KnowledgeError::Migration(_)), "{err:?}");
    }
}
