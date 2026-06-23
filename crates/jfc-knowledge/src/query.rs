//! Insert / recall / supersede / promote / decay operations over the store.
//!
//! These are free functions taking `&Connection` so they're trivially testable
//! against an in-memory DB and reusable from the blocking pool in [`crate::lib`].

use rusqlite::{Connection, params};

use crate::error::{KnowledgeError, Result};
use crate::record::{Kind, KnowledgeRecord, Scope, now_ms};

/// Filters for a recall query.
pub struct RecallFilter<'a> {
    /// The current project's key, so project-scoped rows for THIS repo are
    /// eligible. `None` ⇒ only user/global rows.
    pub project_key: Option<&'a str>,
    /// Max rows to return.
    pub limit: usize,
}

impl Default for RecallFilter<'_> {
    fn default() -> Self {
        Self {
            project_key: None,
            limit: 8,
        }
    }
}

fn row_to_record(row: &rusqlite::Row<'_>) -> rusqlite::Result<KnowledgeRecord> {
    let kind_s: String = row.get("kind")?;
    let scope_s: String = row.get("scope")?;
    let outcome_s: String = row.get("outcome")?;
    Ok(KnowledgeRecord {
        id: row.get("id")?,
        // Unknown enum slugs fall back to a safe default rather than erroring a
        // whole recall — forward-compat if a newer build wrote a new kind.
        kind: Kind::parse(&kind_s).unwrap_or(Kind::Fact),
        scope: Scope::parse(&scope_s).unwrap_or(Scope::Project),
        project_key: row.get("project_key")?,
        title: row.get("title")?,
        body: row.get("body")?,
        tags: row.get("tags")?,
        source: row.get("source")?,
        confidence: row.get("confidence")?,
        created_at_ms: row.get("created_at_ms")?,
        last_used_ms: row.get("last_used_ms")?,
        use_count: row.get("use_count")?,
        superseded_by: row.get("superseded_by")?,
        promoted: row.get::<_, i64>("promoted")? != 0,
        outcome: crate::record::Outcome::parse(&outcome_s).unwrap_or_default(),
        importance: row.get("importance")?,
    })
}

/// Insert a record. Rejects an empty title/body and an out-of-range confidence
/// (validation at the boundary, per the project rules).
pub fn insert(conn: &Connection, rec: &KnowledgeRecord) -> Result<()> {
    if rec.title.trim().is_empty() || rec.body.trim().is_empty() {
        return Err(KnowledgeError::InvalidRecord(
            "title and body must be non-empty".into(),
        ));
    }
    if !(0.0..=1.0).contains(&rec.confidence) {
        return Err(KnowledgeError::InvalidRecord(format!(
            "confidence {} out of range [0,1]",
            rec.confidence
        )));
    }
    // Scope/key invariant: only Project rows carry a project_key; User/Global
    // rows must not (Global is project-independent by definition). Checked as
    // explicit guards rather than a catch-all match so the valid combination is
    // self-documenting.
    let is_project = rec.scope == Scope::Project;
    if is_project && rec.project_key.is_none() {
        return Err(KnowledgeError::InvalidRecord(
            "project records require a project_key".into(),
        ));
    }
    if !is_project && rec.project_key.is_some() {
        return Err(KnowledgeError::InvalidRecord(
            "user/global records must not have a project_key".into(),
        ));
    }
    conn.execute(
        "INSERT INTO knowledge (id, kind, scope, project_key, title, body, tags, source, \
         confidence, created_at_ms, last_used_ms, use_count, superseded_by, promoted, \
         outcome, importance) \
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16)",
        params![
            rec.id,
            rec.kind.slug(),
            rec.scope.slug(),
            rec.project_key,
            rec.title,
            rec.body,
            rec.tags,
            rec.source,
            rec.confidence,
            rec.created_at_ms,
            rec.last_used_ms,
            rec.use_count,
            rec.superseded_by,
            rec.promoted as i64,
            rec.outcome.slug(),
            rec.importance,
        ],
    )?;
    Ok(())
}

/// Mark `old_id` superseded by `new_id` (immutable revision — the old row
/// stays for history but is filtered out of recall).
pub fn supersede(conn: &Connection, old_id: &str, new_id: &str) -> Result<()> {
    conn.execute(
        "UPDATE knowledge SET superseded_by = ?2 WHERE id = ?1",
        params![old_id, new_id],
    )?;
    Ok(())
}

/// Promote a record to global scope. This is the **human-gated** boundary: it is
/// only ever called from an explicit `/knowledge promote` command or an approved
/// proposal — never from the runtime turn loop. Clears `project_key` (global
/// rows are project-independent) and sets `promoted = 1`.
pub fn promote_to_global(conn: &Connection, id: &str) -> Result<bool> {
    let n = conn.execute(
        "UPDATE knowledge SET scope = 'global', project_key = NULL, promoted = 1 \
         WHERE id = ?1 AND superseded_by IS NULL",
        params![id],
    )?;
    Ok(n > 0)
}

/// Recall the most relevant *live* records for `query`, ranked by
/// `confidence * recency_decay * log(use_count + 2)`. Eligible rows are
/// user + global + this-project-only. Lexical match via FTS5 when `query` is
/// non-empty; otherwise the top-ranked recent rows.
pub fn recall(
    conn: &Connection,
    query: &str,
    filter: &RecallFilter<'_>,
) -> Result<Vec<KnowledgeRecord>> {
    let now = now_ms();
    // 30-day half-life recency decay. We avoid SQL `exp()`/`ln()` (not built into
    // SQLite without SQLITE_ENABLE_MATH_FUNCTIONS) and use a portable algebraic
    // approximation that is monotonic in the same direction: a rational recency
    // falloff `hl / (hl + age)` (1.0 when fresh → 0 as it ages) times a usage
    // boost `1 + use_count/(use_count + 4)` (saturating). Ordering, not exact
    // values, is what matters for top-K recall.
    const HALFLIFE_MS: f64 = 30.0 * 24.0 * 3600.0 * 1000.0;
    let scope_clause =
        "(scope IN ('user','global') OR (scope = 'project' AND project_key = :proj))";
    // Score = importance * confidence * verified-boost * recency-falloff *
    // usage-boost. The verified boost (2.0 verified / 1.0 unverified / 0.1
    // refuted) and importance term are schema v2 — they make a verified, salient
    // lesson outrank an unverified ephemeral one on equal lexical relevance.
    let score_expr = "k.importance * k.confidence \
        * (CASE k.outcome WHEN 'verified' THEN 2.0 WHEN 'refuted' THEN 0.1 ELSE 1.0 END) \
        * (:hl / (:hl + CAST(:now - k.created_at_ms AS REAL))) \
        * (1.0 + CAST(k.use_count AS REAL) / (k.use_count + 4))";

    let trimmed = query.trim();
    let mut records = Vec::new();

    if trimmed.is_empty() {
        let sql = format!(
            "SELECT k.* FROM knowledge k \
             WHERE k.superseded_by IS NULL AND {scope_clause} \
             ORDER BY {score_expr} DESC \
             LIMIT :lim"
        );
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map(
            rusqlite::named_params! {
                ":proj": filter.project_key,
                ":now": now,
                ":hl": HALFLIFE_MS,
                ":lim": filter.limit as i64,
            },
            row_to_record,
        )?;
        for r in rows {
            records.push(r?);
        }
        return Ok(records);
    }

    // FTS path: join the fts table on rowid, rank by our score.
    let sql = format!(
        "SELECT k.* FROM knowledge k \
         JOIN knowledge_fts f ON f.rowid = k.rowid \
         WHERE knowledge_fts MATCH :q AND k.superseded_by IS NULL AND {scope_clause} \
         ORDER BY {score_expr} DESC \
         LIMIT :lim"
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(
        rusqlite::named_params! {
            ":q": fts_query(trimmed),
            ":proj": filter.project_key,
            ":now": now,
            ":hl": HALFLIFE_MS,
            ":lim": filter.limit as i64,
        },
        row_to_record,
    )?;
    for r in rows {
        records.push(r?);
    }
    Ok(records)
}

/// Record that a set of records was used (bump use_count + last_used_ms). This
/// is a *metric*, not an action — recall write-back only.
pub fn mark_used(conn: &Connection, ids: &[String]) -> Result<()> {
    let now = now_ms();
    for id in ids {
        conn.execute(
            "UPDATE knowledge SET use_count = use_count + 1, last_used_ms = ?2 WHERE id = ?1",
            params![id, now],
        )?;
    }
    Ok(())
}

/// Bounded-growth maintenance. Hard-deletes superseded rows older than
/// `max_age_ms`, then enforces a per-scope row cap by dropping the
/// lowest-ranked, never-recently-used rows. Returns the number of rows removed.
/// Promoted/global rows are never auto-pruned (a human vouched for them).
pub fn decay(conn: &mut Connection, max_age_ms: i64, max_rows_per_scope: i64) -> Result<usize> {
    let now = now_ms();
    let tx = conn.transaction()?;
    let mut removed = 0usize;

    // 1. Drop old superseded tombstones.
    removed += tx.execute(
        "DELETE FROM knowledge WHERE superseded_by IS NOT NULL AND created_at_ms < ?1",
        params![now - max_age_ms],
    )?;

    // 2. Enforce the per-scope cap for non-promoted project/user rows. Keep the
    //    top `max_rows_per_scope` by score; delete the rest. Global/promoted are
    //    exempt. Uses the same math-function-free score as recall.
    for scope in ["project", "user"] {
        removed += tx.execute(
            "DELETE FROM knowledge WHERE id IN (
                 SELECT id FROM knowledge
                 WHERE scope = ?1 AND promoted = 0 AND superseded_by IS NULL
                 ORDER BY confidence * (1.0 + CAST(use_count AS REAL) / (use_count + 4)) DESC
                 LIMIT -1 OFFSET ?2
             )",
            params![scope, max_rows_per_scope],
        )?;
    }
    tx.commit()?;
    Ok(removed)
}

// ── Obsidian-style typed links (TODO 14) ─────────────────────────────────────

use crate::record::RelKind;

/// Create a typed edge `from -rel-> to`. Idempotent (PK on the triple).
pub fn link(conn: &Connection, from_id: &str, to_id: &str, rel: RelKind) -> Result<()> {
    conn.execute(
        "INSERT OR IGNORE INTO knowledge_links (from_id, to_id, rel, created_at_ms) \
         VALUES (?1, ?2, ?3, ?4)",
        params![from_id, to_id, rel.slug(), now_ms()],
    )?;
    Ok(())
}

/// One outgoing edge + the record it points to.
pub struct LinkedRecord {
    pub rel: RelKind,
    pub record: KnowledgeRecord,
}

/// Records reachable one hop from `id` along outgoing edges (live targets only).
/// This is the recall-expansion primitive: a surfaced error pulls in its
/// `FixedBy` lesson.
pub fn linked(conn: &Connection, id: &str) -> Result<Vec<LinkedRecord>> {
    let mut stmt = conn.prepare(
        "SELECT l.rel AS rel, k.* FROM knowledge_links l \
         JOIN knowledge k ON k.id = l.to_id \
         WHERE l.from_id = ?1 AND k.superseded_by IS NULL",
    )?;
    let rows = stmt.query_map([id], |row| {
        let rel_s: String = row.get("rel")?;
        Ok(LinkedRecord {
            rel: RelKind::parse(&rel_s).unwrap_or(RelKind::RelatesTo),
            record: row_to_record(row)?,
        })
    })?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r?);
    }
    Ok(out)
}

/// Backlinks: ids that point *at* `id` (the "what depends on this" view).
pub fn backlinks(conn: &Connection, id: &str) -> Result<Vec<String>> {
    let mut stmt = conn.prepare("SELECT from_id FROM knowledge_links WHERE to_id = ?1")?;
    let rows = stmt.query_map([id], |row| row.get::<_, String>(0))?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r?);
    }
    Ok(out)
}

// ── Knowledge gaps (TODO 15) ─────────────────────────────────────────────────

/// Record (or bump) a knowledge gap: a referenced-but-absent lesson/skill — the
/// analog of an Obsidian unresolved `[[link]]`. Keyed by a normalized label so
/// repeated references increment `ref_count` (a "learn this next" ranking).
pub fn note_gap(conn: &Connection, label: &str, reason: &str) -> Result<()> {
    let norm = label.trim().to_lowercase();
    if norm.is_empty() {
        return Ok(());
    }
    let id = uuid::Uuid::new_v5(&uuid::Uuid::NAMESPACE_OID, norm.as_bytes())
        .simple()
        .to_string();
    let now = now_ms();
    conn.execute(
        "INSERT INTO knowledge_gaps (id, label, reason, ref_count, first_seen_ms, last_seen_ms) \
         VALUES (?1, ?2, ?3, 1, ?4, ?4) \
         ON CONFLICT(id) DO UPDATE SET ref_count = ref_count + 1, last_seen_ms = ?4",
        params![id, label.trim(), reason, now],
    )?;
    Ok(())
}

/// An open knowledge gap.
pub struct Gap {
    pub id: String,
    pub label: String,
    pub reason: String,
    pub ref_count: i64,
}

/// Open gaps (unresolved), most-referenced first — a ranked "what to learn next".
pub fn gaps(conn: &Connection, limit: usize) -> Result<Vec<Gap>> {
    let mut stmt = conn.prepare(
        "SELECT id, label, reason, ref_count FROM knowledge_gaps \
         WHERE resolved_by IS NULL ORDER BY ref_count DESC, last_seen_ms DESC LIMIT ?1",
    )?;
    let rows = stmt.query_map([limit as i64], |row| {
        Ok(Gap {
            id: row.get(0)?,
            label: row.get(1)?,
            reason: row.get(2)?,
            ref_count: row.get(3)?,
        })
    })?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r?);
    }
    Ok(out)
}

// ── Offline consolidation / dedup (TODO 10) ──────────────────────────────────

/// Consolidate near-duplicate live records: rows sharing the same scope +
/// project + normalized body are collapsed to the strongest (highest
/// importance*confidence, verified beats unverified), the rest superseded by it.
/// Returns the number of rows superseded. Offline/idempotent — running twice is a
/// no-op once duplicates are gone.
pub fn consolidate(conn: &mut Connection) -> Result<usize> {
    // Pull live rows; group in Rust (normalized-body equality is awkward in SQL).
    let mut groups: std::collections::HashMap<String, Vec<(String, f64, i64)>> =
        std::collections::HashMap::new();
    {
        let mut stmt = conn.prepare(
            "SELECT id, scope, COALESCE(project_key,''), body, importance, confidence, \
             CASE outcome WHEN 'verified' THEN 1 ELSE 0 END AS verified \
             FROM knowledge WHERE superseded_by IS NULL",
        )?;
        let rows = stmt.query_map([], |row| {
            let id: String = row.get(0)?;
            let scope: String = row.get(1)?;
            let proj: String = row.get(2)?;
            let body: String = row.get(3)?;
            let importance: f64 = row.get(4)?;
            let confidence: f64 = row.get(5)?;
            let verified: i64 = row.get(6)?;
            let norm = body.split_whitespace().collect::<Vec<_>>().join(" ");
            let key = format!("{scope}\u{1}{proj}\u{1}{norm}");
            let strength = importance * confidence + verified as f64; // verified tiebreak
            Ok((key, id, strength, verified))
        })?;
        for r in rows {
            let (key, id, strength, verified) = r?;
            groups
                .entry(key)
                .or_default()
                .push((id, strength, verified));
        }
    }

    let mut superseded = 0usize;
    let tx = conn.transaction()?;
    for (_key, mut members) in groups {
        if members.len() < 2 {
            continue;
        }
        // Strongest first; the rest are superseded by it.
        members.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        let keeper = members[0].0.clone();
        for (loser, _, _) in &members[1..] {
            tx.execute(
                "UPDATE knowledge SET superseded_by = ?2 WHERE id = ?1",
                params![loser, keeper],
            )?;
            superseded += 1;
        }
    }
    tx.commit()?;
    Ok(superseded)
}

/// Build an FTS5 MATCH expression that treats the query as a bag of OR'd terms,
/// quoting each token so punctuation can't inject FTS syntax.
fn fts_query(raw: &str) -> String {
    let terms: Vec<String> = raw
        .split_whitespace()
        .filter(|t| t.len() >= 2)
        .map(|t| format!("\"{}\"", t.replace('"', "")))
        .collect();
    if terms.is_empty() {
        // Fall back to a quoted whole-string match.
        format!("\"{}\"", raw.replace('"', ""))
    } else {
        terms.join(" OR ")
    }
}
