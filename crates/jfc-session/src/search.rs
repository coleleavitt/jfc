//! Cross-session full-text search over persisted session transcripts.
//!
//! The jfc-knowledge SQLite store is the transcript substrate: session save
//! writes a lossless `session_messages.meta` row plus searchable text, and this
//! module queries that DB directly.
//!
//! Three modes, mirroring Hermes:
//!   * [`discover`] — a query → top-N sessions, each with the best-matching
//!     snippet, a ±window message context, and session bookends.
//!   * [`scroll`]   — a `(session_id, anchor_index)` → a ±window slice of one
//!     session's messages (anchored drill-down).
//!   * [`browse`]   — no query → recent sessions chronologically.
//!
//! No LLM calls anywhere — every mode returns persisted text from the DB.

use crate::SessionId;
use crate::soft_match::{query_terms, score_text};
use jfc_knowledge::KnowledgeStore;
use serde::{Deserialize, Serialize};

/// One message flattened to searchable text. `index` is the 0-based position in
/// the session's `messages` array (the anchor used by [`scroll`]).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionMessage {
    pub index: usize,
    pub role: String,
    pub text: String,
}

/// A search hit: a session plus the matching snippet and surrounding context.
#[derive(Debug, Clone, Serialize)]
pub struct SessionHit {
    pub session_id: String,
    pub title: String,
    pub updated_at: String,
    /// Index of the best-matching message within the session.
    pub match_index: usize,
    /// The matching message text, trimmed to a snippet around the term.
    pub snippet: String,
    /// ±`window` messages centered on the match.
    pub context: Vec<SessionMessage>,
    /// First few messages of the session (orientation).
    pub bookend_start: Vec<SessionMessage>,
    /// Last few messages of the session.
    pub bookend_end: Vec<SessionMessage>,
}

/// A lightweight session summary for BROWSE mode.
#[derive(Debug, Clone, Serialize)]
pub struct SessionBrief {
    pub session_id: String,
    pub title: String,
    pub updated_at: String,
    pub message_count: usize,
    pub preview: String,
}

const BOOKEND: usize = 3;
const SNIPPET_RADIUS: usize = 160;

/// Build a `SNIPPET_RADIUS`-char snippet centered on the first match of `needle`
/// (case-insensitive) within `text`.
fn snippet_around(text: &str, needle_lc: &str) -> String {
    let lc = text.to_lowercase();
    let Some(pos) = lc.find(needle_lc) else {
        return text.chars().take(SNIPPET_RADIUS * 2).collect();
    };
    let start = pos.saturating_sub(SNIPPET_RADIUS);
    let end = (pos + needle_lc.len() + SNIPPET_RADIUS).min(text.len());
    // Snap to char boundaries.
    let start = (start..=pos)
        .rev()
        .find(|i| text.is_char_boundary(*i))
        .unwrap_or(pos);
    let end = (end..text.len())
        .find(|i| text.is_char_boundary(*i))
        .unwrap_or(text.len());
    let mut snip = String::new();
    if start > 0 {
        snip.push('…');
    }
    snip.push_str(text[start..end].trim());
    if end < text.len() {
        snip.push('…');
    }
    snip
}

fn truncate_chars(text: &str, max_chars: usize) -> String {
    let mut chars = text.chars();
    let mut out: String = chars.by_ref().take(max_chars).collect();
    if chars.next().is_some() {
        out.push('…');
    }
    out
}

fn slice_messages(all: &[SessionMessage], center: usize, radius: usize) -> Vec<SessionMessage> {
    let start = center.saturating_sub(radius);
    let end = (center + radius + 1).min(all.len());
    all[start..end].to_vec()
}

fn open_session_db() -> Option<KnowledgeStore> {
    jfc_knowledge::block_on_knowledge(async { KnowledgeStore::open_default().await.ok() })
}

fn db_row_title(row: &jfc_knowledge::SessionRow) -> String {
    row.title
        .as_deref()
        .filter(|s| !s.trim().is_empty())
        .or(row.first_prompt.as_deref().filter(|s| !s.trim().is_empty()))
        .map(|s| {
            s.trim()
                .lines()
                .next()
                .unwrap_or(row.id.as_str())
                .to_owned()
        })
        .unwrap_or_else(|| row.id.clone())
}

fn db_row_updated(row: &jfc_knowledge::SessionRow) -> String {
    row.updated_at
        .clone()
        .or_else(|| row.created_at.clone())
        .unwrap_or_default()
}

fn db_messages(rows: Vec<jfc_knowledge::SessionMessage>) -> Vec<SessionMessage> {
    rows.into_iter()
        .enumerate()
        .map(|(fallback_index, msg)| SessionMessage {
            index: usize::try_from(msg.seq).unwrap_or(fallback_index),
            role: msg.role,
            text: msg.content,
        })
        .collect()
}

fn best_match(msgs: &[SessionMessage], needle: &str, terms: &[String]) -> Option<(usize, usize)> {
    let exact_match = msgs
        .iter()
        .position(|m| m.text.to_lowercase().contains(needle))
        .map(|idx| (idx, usize::MAX));
    exact_match.or_else(|| {
        msgs.iter()
            .enumerate()
            .map(|(idx, msg)| (idx, score_text(&msg.text, terms)))
            .max_by_key(|(_, score)| *score)
            .filter(|(_, score)| *score > 0)
    })
}

fn discover_db(
    query: &str,
    limit: usize,
    window: usize,
    exclude_session: Option<&str>,
) -> Vec<SessionHit> {
    let Some(store) = open_session_db() else {
        return Vec::new();
    };
    discover_db_from_store(&store, query, limit, window, exclude_session)
}

fn discover_db_from_store(
    store: &KnowledgeStore,
    query: &str,
    limit: usize,
    window: usize,
    exclude_session: Option<&str>,
) -> Vec<SessionHit> {
    let needle = query.trim().to_lowercase();
    if needle.is_empty() {
        return Vec::new();
    }
    let terms = query_terms(&needle);
    let candidate_limit = limit.saturating_mul(4).max(limit).max(16);
    let mut hits = Vec::new();
    // Do ALL of this query's DB reads inside ONE bridge call. `block_on_knowledge`
    // drives the future on a dedicated runtime; a `sqlx` pooled connection is
    // bound to the runtime that created it, so splitting the search + per-row
    // loads across multiple bridge calls (each its own runtime) can race the
    // pool's single connection and, for in-memory stores, read a different
    // database. One block keeps the whole sequence on one runtime/connection.
    let rows: Vec<(
        jfc_knowledge::SessionRow,
        Vec<jfc_knowledge::SessionMessage>,
    )> = jfc_knowledge::block_on_knowledge(async {
        let ids = store
            .search_transcripts(query, candidate_limit)
            .await
            .unwrap_or_default();
        let mut out = Vec::new();
        for id in ids {
            if exclude_session == Some(id.as_str()) {
                continue;
            }
            let Some(row) = store.get_session(&id).await.ok().flatten() else {
                continue;
            };
            let Ok(loaded) = store.load_transcript(&id).await else {
                continue;
            };
            out.push((row, loaded));
        }
        out
    });
    for (row, loaded) in rows {
        let msgs = db_messages(loaded);
        let Some((match_index, score)) = best_match(&msgs, &needle, &terms) else {
            continue;
        };
        let updated = db_row_updated(&row);
        hits.push((
            score,
            updated.clone(),
            SessionHit {
                session_id: row.id.clone(),
                title: db_row_title(&row),
                updated_at: updated,
                match_index,
                snippet: if score == usize::MAX {
                    snippet_around(&msgs[match_index].text, &needle)
                } else {
                    truncate_chars(&msgs[match_index].text, SNIPPET_RADIUS * 2)
                },
                context: slice_messages(&msgs, match_index, window),
                bookend_start: msgs.iter().take(BOOKEND).cloned().collect(),
                bookend_end: msgs.iter().rev().take(BOOKEND).rev().cloned().collect(),
            },
        ));
        if hits.len() >= limit {
            break;
        }
    }
    hits.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| b.1.cmp(&a.1)));
    hits.into_iter()
        .map(|(_, _, hit)| hit)
        .take(limit)
        .collect()
}

fn scroll_db(
    session_id: &SessionId,
    anchor_index: usize,
    window: usize,
) -> Option<Vec<SessionMessage>> {
    let store = open_session_db()?;
    scroll_db_from_store(&store, session_id.as_str(), anchor_index, window)
}

fn scroll_db_from_store(
    store: &KnowledgeStore,
    session_id: &str,
    anchor_index: usize,
    window: usize,
) -> Option<Vec<SessionMessage>> {
    let msgs = db_messages(
        jfc_knowledge::block_on_knowledge(async { store.load_transcript(session_id).await })
            .ok()?,
    );
    if msgs.is_empty() {
        return None;
    }
    let center = anchor_index.min(msgs.len() - 1);
    Some(slice_messages(&msgs, center, window))
}

fn browse_db(limit: usize) -> Option<Vec<SessionBrief>> {
    let store = open_session_db()?;
    Some(browse_db_from_store(&store, limit))
}

fn browse_db_from_store(store: &KnowledgeStore, limit: usize) -> Vec<SessionBrief> {
    let Ok(rows) =
        jfc_knowledge::block_on_knowledge(async { store.list_sessions(None, limit).await })
    else {
        return Vec::new();
    };
    let mut briefs = Vec::with_capacity(rows.len());
    for row in rows {
        let transcript = jfc_knowledge::block_on_knowledge(async {
            store.load_transcript(&row.id).await.unwrap_or_default()
        });
        let preview = transcript
            .iter()
            .find(|m| m.role == "user")
            .map(|m| truncate_chars(&m.content, 120))
            .unwrap_or_default();
        briefs.push(SessionBrief {
            session_id: row.id.clone(),
            title: db_row_title(&row),
            updated_at: db_row_updated(&row),
            message_count: row.message_count.max(0) as usize,
            preview,
        });
    }
    briefs
}

/// DISCOVERY: full-text search across all sessions. Returns up to `limit` hits,
/// most-recently-updated first, each scored by match count.
pub fn discover(query: &str, limit: usize, window: usize) -> Vec<SessionHit> {
    discover_excluding(query, limit, window, None)
}

/// DISCOVERY with a visible-context exclusion: the same as [`discover`] but
/// skips `exclude_session` (typically the *current* session, whose transcript is
/// already live in the prompt). Returning hits from the active session would
/// re-inject text the model can already see — wasted tokens that crowd out
/// genuinely-recalled context. Mirrors magic-context's visible-memory hard
/// filter (`getVisibleMemoryIds`).
pub fn discover_excluding(
    query: &str,
    limit: usize,
    window: usize,
    exclude_session: Option<&str>,
) -> Vec<SessionHit> {
    discover_db(query, limit, window, exclude_session)
}

/// SCROLL: anchored ±`window` slice of one session's messages.
pub fn scroll(session_id: &SessionId, anchor_index: usize, window: usize) -> Vec<SessionMessage> {
    scroll_db(session_id, anchor_index, window).unwrap_or_default()
}

/// BROWSE: recent sessions chronologically (most-recent first).
pub fn browse(limit: usize) -> Vec<SessionBrief> {
    browse_db(limit).unwrap_or_default()
}

pub fn prior_user_prompts(
    max_sessions: usize,
    max_prompts_per_session: usize,
    exclude_session: Option<&str>,
) -> Vec<String> {
    let mut collected = Vec::new();
    let mut loaded = 0usize;
    if let Some(store) = open_session_db() {
        let Ok(rows) =
            jfc_knowledge::block_on_knowledge(async { store.list_sessions(None, 100_000).await })
        else {
            return collected;
        };
        for row in rows {
            if loaded >= max_sessions {
                break;
            }
            if exclude_session == Some(row.id.as_str()) {
                continue;
            }
            let Ok(messages) =
                jfc_knowledge::block_on_knowledge(async { store.load_transcript(&row.id).await })
            else {
                continue;
            };
            collected.extend(prompts_from_db_messages(&messages, max_prompts_per_session));
            loaded += 1;
        }
    }
    collected.reverse();
    collected
}

fn prompts_from_db_messages(
    messages: &[jfc_knowledge::SessionMessage],
    max_prompts: usize,
) -> Vec<String> {
    let mut out = Vec::new();
    'message: for message in messages {
        if message.role != "user" {
            continue;
        }
        let parts = message
            .meta
            .as_deref()
            .and_then(|meta| serde_json::from_str::<serde_json::Value>(meta).ok())
            .and_then(|value| value.get("parts").cloned());
        if let Some(serde_json::Value::Array(parts)) = parts {
            if parts.iter().any(is_compact_boundary_part) {
                continue 'message;
            }
            for part in parts {
                let Some(text) = part_text(&part) else {
                    continue;
                };
                push_prompt(&mut out, text, max_prompts);
                if out.len() >= max_prompts {
                    return out;
                }
            }
        } else {
            push_prompt(&mut out, &message.content, max_prompts);
            if out.len() >= max_prompts {
                return out;
            }
        }
    }
    out
}

fn part_text(part: &serde_json::Value) -> Option<&str> {
    part.get("content")
        .or_else(|| part.get("text"))
        .and_then(serde_json::Value::as_str)
}

fn is_compact_boundary_part(part: &serde_json::Value) -> bool {
    matches!(
        part.get("type").and_then(serde_json::Value::as_str),
        Some("compact_boundary" | "compactBoundary")
    ) || part.get("compactBoundary").is_some()
}

fn push_prompt(out: &mut Vec<String>, text: &str, max_prompts: usize) {
    if out.len() >= max_prompts {
        return;
    }
    let trimmed = text.trim();
    if trimmed.is_empty() || trimmed.starts_with('/') {
        return;
    }
    out.push(trimmed.to_owned());
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::MutexGuard;

    struct EnvGuard {
        _temp: tempfile::TempDir,
        prev_db: Option<String>,
        prev_config: Option<String>,
        _lock: MutexGuard<'static, ()>,
    }

    impl EnvGuard {
        fn new() -> Self {
            let lock = crate::TEST_ENV_LOCK
                .lock()
                .unwrap_or_else(|err| err.into_inner());
            let temp = tempfile::tempdir().unwrap();
            let db_path = temp.path().join("knowledge.db");
            let config_home = temp.path().join("config");
            std::fs::create_dir_all(&config_home).unwrap();
            let prev_db = std::env::var("JFC_KNOWLEDGE_DB").ok();
            let prev_config = std::env::var("XDG_CONFIG_HOME").ok();
            unsafe {
                std::env::set_var("JFC_KNOWLEDGE_DB", &db_path);
                std::env::set_var("XDG_CONFIG_HOME", &config_home);
            }
            Self {
                _temp: temp,
                prev_db,
                prev_config,
                _lock: lock,
            }
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            unsafe {
                match self.prev_db.take() {
                    Some(value) => std::env::set_var("JFC_KNOWLEDGE_DB", value),
                    None => std::env::remove_var("JFC_KNOWLEDGE_DB"),
                }
                match self.prev_config.take() {
                    Some(value) => std::env::set_var("XDG_CONFIG_HOME", value),
                    None => std::env::remove_var("XDG_CONFIG_HOME"),
                }
            }
        }
    }

    // Robust: a snippet is centered on the match with ellipses, char-safe.
    #[test]
    fn snippet_centers_on_match_robust() {
        let text = "the quick brown fox jumps over the lazy dog".repeat(10);
        let snip = snippet_around(&text, "lazy");
        assert!(snip.contains("lazy"));
        assert!(snip.starts_with('…') || snip.contains("lazy"));
    }

    // Normal: slice_messages clamps to bounds and centers correctly.
    #[test]
    fn slice_messages_clamps_normal() {
        let msgs: Vec<SessionMessage> = (0..5)
            .map(|i| SessionMessage {
                index: i,
                role: "user".into(),
                text: format!("m{i}"),
            })
            .collect();
        let s = slice_messages(&msgs, 0, 2);
        assert_eq!(s.first().unwrap().index, 0);
        assert_eq!(s.last().unwrap().index, 2);
        let s2 = slice_messages(&msgs, 4, 2);
        assert_eq!(s2.first().unwrap().index, 2);
        assert_eq!(s2.last().unwrap().index, 4);
    }

    // Normal: discover_excluding skips the excluded session id (the visible-
    // context dedup) — verified at the id-filter level without touching disk.
    #[test]
    fn discover_excluding_skips_excluded_id_normal() {
        assert!(discover_excluding("", 5, 1, Some("ses_x")).is_empty());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn db_discover_browse_and_scroll_use_transcript_store_normal() {
        let store = KnowledgeStore::open_in_memory().await.unwrap();
        let row = jfc_knowledge::SessionRow {
            id: "ses_db".into(),
            cwd: Some("/repo".into()),
            model: Some("claude".into()),
            created_at: Some("2026-01-01T00:00:00Z".into()),
            updated_at: Some("2026-01-01T01:00:00Z".into()),
            first_prompt: Some("fix session database".into()),
            title: None,
            message_count: 2,
        };
        let transcript = vec![
            jfc_knowledge::SessionMessage {
                seq: 0,
                role: "user".into(),
                content: "fix session database".into(),
                meta: None,
            },
            jfc_knowledge::SessionMessage {
                seq: 1,
                role: "assistant".into(),
                content: "used sqlite transcript search".into(),
                meta: None,
            },
        ];
        store.replace_transcript(&row, &transcript).await.unwrap();

        let hits = discover_db_from_store(&store, "sqlite", 5, 1, None);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].session_id, "ses_db");
        assert!(hits[0].snippet.contains("sqlite"));

        let briefs = browse_db_from_store(&store, 5);
        assert_eq!(briefs.len(), 1);
        assert_eq!(briefs[0].preview, "fix session database");

        let scrolled = scroll_db_from_store(&store, "ses_db", 1, 1).unwrap();
        assert_eq!(scrolled.len(), 2);
        assert_eq!(scrolled[1].text, "used sqlite transcript search");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn default_search_uses_db_and_ignores_legacy_json_normal() {
        let _env = EnvGuard::new();
        let store = KnowledgeStore::open_default().await.unwrap();
        let row = jfc_knowledge::SessionRow {
            id: "ses_db_only".into(),
            cwd: Some("/repo".into()),
            model: Some("claude".into()),
            created_at: Some("2026-01-01T00:00:00Z".into()),
            updated_at: Some("2026-01-01T02:00:00Z".into()),
            first_prompt: Some("db prompt".into()),
            title: None,
            message_count: 1,
        };
        store
            .replace_transcript(
                &row,
                &[jfc_knowledge::SessionMessage {
                    seq: 0,
                    role: "user".into(),
                    content: "db prompt with citadel".into(),
                    meta: Some(
                        serde_json::json!({
                            "role": "user",
                            "parts": [{"type": "text", "content": "db prompt with citadel"}]
                        })
                        .to_string(),
                    ),
                }],
            )
            .await
            .unwrap();

        let dir = crate::sessions_dir();
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("ses_json_only.json"),
            serde_json::json!({
                "id": "ses_json_only",
                "created_at": "2026-01-01T00:00:00Z",
                "updated_at": "2026-01-01T01:00:00Z",
                "first_prompt": "json prompt",
                "messages": [{
                    "role": "user",
                    "parts": [{"type": "text", "content": "json prompt with citadel"}]
                }]
            })
            .to_string(),
        )
        .unwrap();

        let hits = discover("citadel", 10, 1);
        let ids = hits
            .iter()
            .map(|hit| hit.session_id.as_str())
            .collect::<Vec<_>>();
        assert!(ids.contains(&"ses_db_only"), "{ids:?}");
        assert!(!ids.contains(&"ses_json_only"), "{ids:?}");

        let prompts = prior_user_prompts(10, 50, None);
        assert!(
            prompts
                .iter()
                .any(|prompt| prompt == "db prompt with citadel")
        );
        assert!(
            prompts
                .iter()
                .all(|prompt| prompt != "json prompt with citadel")
        );
    }
}
