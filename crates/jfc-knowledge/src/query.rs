//! Insert / recall / supersede / promote / decay operations over the store.
//!
//! These are free functions taking `&SqlitePool` or `&mut SqliteConnection` so they're
//! trivially testable against an in-memory DB and reusable from the async runtime in [`crate::lib`].

use sqlx::AssertSqlSafe;
use sqlx::{Row, SqlitePool};

use crate::error::{KnowledgeError, Result};
use crate::record::{Kind, KnowledgeRecord, Scope, now_ms};
use crate::redact::redact;

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

fn row_to_record(row: &sqlx::sqlite::SqliteRow) -> Result<KnowledgeRecord> {
    let kind_s: String = row.try_get("kind")?;
    let scope_s: String = row.try_get("scope")?;
    let outcome_s: String = row.try_get("outcome")?;
    Ok(KnowledgeRecord {
        id: row.try_get("id")?,
        // Unknown enum slugs fall back to a safe default rather than erroring a
        // whole recall — forward-compat if a newer build wrote a new kind.
        kind: Kind::parse(&kind_s).unwrap_or(Kind::Fact),
        scope: Scope::parse(&scope_s).unwrap_or(Scope::Project),
        project_key: row.try_get("project_key")?,
        title: row.try_get("title")?,
        body: row.try_get("body")?,
        tags: row.try_get("tags")?,
        source: row.try_get("source")?,
        confidence: row.try_get("confidence")?,
        created_at_ms: row.try_get("created_at_ms")?,
        last_used_ms: row.try_get("last_used_ms")?,
        use_count: row.try_get("use_count")?,
        superseded_by: row.try_get("superseded_by")?,
        promoted: row.try_get::<i64, _>("promoted")? != 0,
        outcome: crate::record::Outcome::parse(&outcome_s).unwrap_or_default(),
        importance: row.try_get("importance")?,
    })
}

/// Insert a record. Rejects an empty title/body and an out-of-range confidence
/// (validation at the boundary, per the project rules).
pub async fn insert(pool: &SqlitePool, rec: &KnowledgeRecord) -> Result<()> {
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
    reject_sensitive_field("title", &rec.title)?;
    reject_sensitive_field("body", &rec.body)?;
    reject_sensitive_field("tags", &rec.tags)?;
    if let Some(source) = &rec.source {
        reject_sensitive_field("source", source)?;
    }
    sqlx::query(
        "INSERT INTO knowledge (id, kind, scope, project_key, title, body, tags, source, \
         confidence, created_at_ms, last_used_ms, use_count, superseded_by, promoted, \
         outcome, importance) \
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16)"
    )
        .bind(&rec.id)
        .bind(rec.kind.slug())
        .bind(rec.scope.slug())
        .bind(&rec.project_key)
        .bind(&rec.title)
        .bind(&rec.body)
        .bind(&rec.tags)
        .bind(&rec.source)
        .bind(rec.confidence)
        .bind(rec.created_at_ms)
        .bind(rec.last_used_ms)
        .bind(rec.use_count)
        .bind(&rec.superseded_by)
        .bind(rec.promoted as i64)
        .bind(rec.outcome.slug())
        .bind(rec.importance)
        .execute(pool)
        .await?;
    Ok(())
}

fn reject_sensitive_field(field: &str, value: &str) -> Result<()> {
    if redact(value, false) != value {
        return Err(KnowledgeError::InvalidRecord(format!(
            "{field} contains sensitive material"
        )));
    }
    Ok(())
}

/// Mark `old_id` superseded by `new_id` (immutable revision — the old row
/// stays for history but is filtered out of recall).
pub async fn supersede(pool: &SqlitePool, old_id: &str, new_id: &str) -> Result<()> {
    sqlx::query("UPDATE knowledge SET superseded_by = ?2 WHERE id = ?1")
        .bind(old_id)
        .bind(new_id)
        .execute(pool)
        .await?;
    Ok(())
}

/// Promote a record to global scope. Explicit `/knowledge promote` uses this
/// path; autonomous promotion uses the stricter `KnowledgeStore::auto_promote`
/// query. Clears `project_key` (global rows are project-independent) and sets
/// `promoted = 1`.
pub async fn promote_to_global(pool: &SqlitePool, id: &str) -> Result<bool> {
    let result = sqlx::query(
        "UPDATE knowledge SET scope = 'global', project_key = NULL, promoted = 1 \
         WHERE id = ?1 AND superseded_by IS NULL"
    )
        .bind(id)
        .execute(pool)
        .await?;
    Ok(result.rows_affected() > 0)
}

/// Recall the most relevant *live* records for `query`, ranked by
/// `confidence * recency_decay * log(use_count + 2)`. Eligible rows are
/// user + global + this-project-only. Lexical match via FTS5 when `query` is
/// non-empty; otherwise the top-ranked recent rows.
pub async fn recall(
    pool: &SqlitePool,
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
    // Numbered placeholders are reused across the SELECT (SQLite binds each `?N`
    // once even when it appears multiple times). The non-FTS and FTS branches use
    // a DIFFERENT numbering because the FTS branch adds the leading `MATCH ?1`, so
    // each branch builds its own scope/score fragments to keep the numbers aligned
    // with the bind order below.
    let trimmed = query.trim();
    let mut records = Vec::new();

    if trimmed.is_empty() {
        // Binds: ?1=project_key, ?2=halflife, ?3=now, ?4=limit.
        let scope_clause =
            "(scope IN ('user','global') OR (scope = 'project' AND project_key = ?1))";
        let score_expr = "k.importance * k.confidence \
            * (CASE k.outcome WHEN 'verified' THEN 2.0 WHEN 'refuted' THEN 0.1 ELSE 1.0 END) \
            * (?2 / (?2 + CAST(?3 - k.created_at_ms AS REAL))) \
            * (1.0 + CAST(k.use_count AS REAL) / (k.use_count + 4))";
        let sql = format!(
            "SELECT k.* FROM knowledge k \
             WHERE k.superseded_by IS NULL AND {scope_clause} \
             ORDER BY {score_expr} DESC \
             LIMIT ?4"
        );
        let rows = sqlx::query(AssertSqlSafe(sql))
            .bind(filter.project_key)
            .bind(HALFLIFE_MS)
            .bind(now)
            .bind(filter.limit as i64)
            .fetch_all(pool)
            .await?;
        for row in rows {
            records.push(row_to_record(&row)?);
        }
        return Ok(records);
    }

    // FTS path: join the fts table on rowid, rank by our score.
    // Binds: ?1=fts_query, ?2=project_key, ?3=halflife, ?4=now, ?5=limit.
    let scope_clause =
        "(scope IN ('user','global') OR (scope = 'project' AND project_key = ?2))";
    let score_expr = "k.importance * k.confidence \
        * (CASE k.outcome WHEN 'verified' THEN 2.0 WHEN 'refuted' THEN 0.1 ELSE 1.0 END) \
        * (?3 / (?3 + CAST(?4 - k.created_at_ms AS REAL))) \
        * (1.0 + CAST(k.use_count AS REAL) / (k.use_count + 4))";
    let sql = format!(
        "SELECT k.* FROM knowledge k \
         JOIN knowledge_fts ON knowledge_fts.rowid = k.rowid \
         WHERE knowledge_fts MATCH ?1 AND k.superseded_by IS NULL AND {scope_clause} \
         ORDER BY {score_expr} DESC \
         LIMIT ?5"
    );
    let rows = sqlx::query(AssertSqlSafe(sql))
        .bind(fts_query(trimmed))
        .bind(filter.project_key)
        .bind(HALFLIFE_MS)
        .bind(now)
        .bind(filter.limit as i64)
        .fetch_all(pool)
        .await?;
    for row in rows {
        records.push(row_to_record(&row)?);
    }
    Ok(records)
}

/// Record that a set of records was used (bump use_count + last_used_ms). This
/// is a *metric*, not an action — recall write-back only.
pub async fn mark_used(pool: &SqlitePool, ids: &[String]) -> Result<()> {
    let now = now_ms();
    for id in ids {
        sqlx::query(
            "UPDATE knowledge SET use_count = use_count + 1, last_used_ms = ?2 WHERE id = ?1"
        )
            .bind(id)
            .bind(now)
            .execute(pool)
            .await?;
    }
    Ok(())
}

/// Bounded-growth maintenance. Hard-deletes superseded rows older than
/// `max_age_ms`, then enforces a per-scope row cap by dropping the
/// lowest-ranked, never-recently-used rows. Returns the number of rows removed.
/// Explicitly promoted rows are never auto-pruned.
pub async fn decay(pool: &SqlitePool, max_age_ms: i64, max_rows_per_scope: i64) -> Result<usize> {
    let now = now_ms();
    let mut tx = pool.begin().await?;
    let mut removed = 0usize;

    // 1. Drop old superseded tombstones.
    let result = sqlx::query(
        "DELETE FROM knowledge WHERE superseded_by IS NOT NULL AND created_at_ms < ?1"
    )
        .bind(now - max_age_ms)
        .execute(&mut *tx)
        .await?;
    removed += result.rows_affected() as usize;

    // 2. Enforce the per-scope cap for non-promoted rows. Keep the top
    //    `max_rows_per_scope` by score; delete the rest.
    for scope in ["project", "user", "global"] {
        let result = sqlx::query(
            "DELETE FROM knowledge WHERE id IN (
                 SELECT id FROM knowledge
                 WHERE scope = ?1 AND promoted = 0 AND superseded_by IS NULL
                 ORDER BY confidence * (1.0 + CAST(use_count AS REAL) / (use_count + 4)) DESC
                 LIMIT -1 OFFSET ?2
             )"
        )
            .bind(scope)
            .bind(max_rows_per_scope)
            .execute(&mut *tx)
            .await?;
        removed += result.rows_affected() as usize;
    }
    tx.commit().await?;
    Ok(removed)
}

// ── Obsidian-style typed links (TODO 14) ─────────────────────────────────────

use crate::record::RelKind;

/// Create a typed edge `from -rel-> to`. Idempotent (PK on the triple).
pub async fn link(pool: &SqlitePool, from_id: &str, to_id: &str, rel: RelKind) -> Result<()> {
    sqlx::query(
        "INSERT OR IGNORE INTO knowledge_links (from_id, to_id, rel, created_at_ms) \
         VALUES (?1, ?2, ?3, ?4)"
    )
        .bind(from_id)
        .bind(to_id)
        .bind(rel.slug())
        .bind(now_ms())
        .execute(pool)
        .await?;
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
pub async fn linked(pool: &SqlitePool, id: &str) -> Result<Vec<LinkedRecord>> {
    let rows = sqlx::query(
        "SELECT l.rel AS rel, k.* FROM knowledge_links l \
         JOIN knowledge k ON k.id = l.to_id \
         WHERE l.from_id = ?1 AND k.superseded_by IS NULL"
    )
        .bind(id)
        .fetch_all(pool)
        .await?;
    let mut out = Vec::new();
    for row in rows {
        let rel_s: String = row.try_get("rel")?;
        out.push(LinkedRecord {
            rel: RelKind::parse(&rel_s).unwrap_or(RelKind::RelatesTo),
            record: row_to_record(&row)?,
        });
    }
    Ok(out)
}

/// Backlinks: ids that point *at* `id` (the "what depends on this" view).
pub async fn backlinks(pool: &SqlitePool, id: &str) -> Result<Vec<String>> {
    let rows = sqlx::query("SELECT from_id FROM knowledge_links WHERE to_id = ?1")
        .bind(id)
        .fetch_all(pool)
        .await?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row.try_get::<String, _>("from_id")?);
    }
    Ok(out)
}

// ── Knowledge gaps (TODO 15) ─────────────────────────────────────────────────

/// Record (or bump) a knowledge gap: a referenced-but-absent lesson/skill — the
/// analog of an Obsidian unresolved `[[link]]`. Keyed by a normalized label so
/// repeated references increment `ref_count` (a "learn this next" ranking).
pub async fn note_gap(pool: &SqlitePool, label: &str, reason: &str) -> Result<()> {
    let norm = label.trim().to_lowercase();
    if norm.is_empty() {
        return Ok(());
    }
    let id = uuid::Uuid::new_v5(&uuid::Uuid::NAMESPACE_OID, norm.as_bytes())
        .simple()
        .to_string();
    let now = now_ms();
    sqlx::query(
        "INSERT INTO knowledge_gaps (id, label, reason, ref_count, first_seen_ms, last_seen_ms) \
         VALUES (?1, ?2, ?3, 1, ?4, ?4) \
         ON CONFLICT(id) DO UPDATE SET ref_count = ref_count + 1, last_seen_ms = ?4"
    )
        .bind(&id)
        .bind(label.trim())
        .bind(reason)
        .bind(now)
        .execute(pool)
        .await?;
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
pub async fn gaps(pool: &SqlitePool, limit: usize) -> Result<Vec<Gap>> {
    let rows = sqlx::query(
        "SELECT id, label, reason, ref_count FROM knowledge_gaps \
         WHERE resolved_by IS NULL ORDER BY ref_count DESC, last_seen_ms DESC LIMIT ?1"
    )
        .bind(limit as i64)
        .fetch_all(pool)
        .await?;
    let mut out = Vec::new();
    for row in rows {
        out.push(Gap {
            id: row.try_get("id")?,
            label: row.try_get("label")?,
            reason: row.try_get("reason")?,
            ref_count: row.try_get("ref_count")?,
        });
    }
    Ok(out)
}

// ── Offline consolidation / dedup (TODO 10) ──────────────────────────────────

/// Consolidate near-duplicate live records: rows sharing the same scope +
/// project + normalized body are collapsed to the strongest (highest
/// importance*confidence, verified beats unverified), the rest superseded by it.
/// Returns the number of rows superseded. Offline/idempotent — running twice is a
/// no-op once duplicates are gone.
pub async fn consolidate(pool: &SqlitePool) -> Result<usize> {
    // Pull live rows; group in Rust (normalized-body equality is awkward in SQL).
    let mut groups: std::collections::HashMap<String, Vec<(String, f64, i64)>> =
        std::collections::HashMap::new();
    {
        let rows = sqlx::query(
            "SELECT id, scope, COALESCE(project_key,'') as proj, body, importance, confidence, \
             CASE outcome WHEN 'verified' THEN 1 ELSE 0 END AS verified \
             FROM knowledge WHERE superseded_by IS NULL"
        )
            .fetch_all(pool)
            .await?;
        for row in rows {
            let id: String = row.try_get("id")?;
            let scope: String = row.try_get("scope")?;
            let proj: String = row.try_get("proj")?;
            let body: String = row.try_get("body")?;
            let importance: f64 = row.try_get("importance")?;
            let confidence: f64 = row.try_get("confidence")?;
            let verified: i64 = row.try_get("verified")?;
            let norm = body.split_whitespace().collect::<Vec<_>>().join(" ");
            let key = format!("{scope}\u{1}{proj}\u{1}{norm}");
            let strength = importance * confidence + verified as f64; // verified tiebreak
            groups
                .entry(key)
                .or_default()
                .push((id, strength, verified));
        }
    }

    let mut superseded = 0usize;
    let mut tx = pool.begin().await?;
    for (_key, mut members) in groups {
        if members.len() < 2 {
            continue;
        }
        // Strongest first; the rest are superseded by it.
        members.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        let keeper = members[0].0.clone();
        for (loser, _, _) in &members[1..] {
            sqlx::query("UPDATE knowledge SET superseded_by = ?2 WHERE id = ?1")
                .bind(loser)
                .bind(&keeper)
                .execute(&mut *tx)
                .await?;
            superseded += 1;
        }
    }
    tx.commit().await?;
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
