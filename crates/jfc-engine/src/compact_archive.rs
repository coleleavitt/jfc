use crate::session::{SerializedMessage, deserialize_message, serialize_message};
use crate::types::{ChatMessage, Role};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

const ARCHIVE_SCHEMA_VERSION: u32 = 1;
const COMPACT_ARCHIVE_KIND: &str = "compact_archive";
const MAX_RENDER_MESSAGE_CHARS: usize = 2_000;
const MAX_RENDER_TOTAL_CHARS: usize = 16_000;

#[derive(Debug, Clone)]
pub struct CompactArchiveMeta {
    pub id: String,
    pub path: PathBuf,
    pub message_count: usize,
}

#[derive(Debug, Clone)]
pub struct CompactArchiveHit {
    pub id: String,
    pub created_at: String,
    pub message_count: usize,
    pub snippet: String,
}

#[derive(Serialize, Deserialize)]
struct CompactArchive {
    schema_version: u32,
    id: String,
    created_at: String,
    pre_tokens: usize,
    summary: String,
    messages: Vec<SerializedMessage>,
}

pub fn archive_current_project(
    messages: &[ChatMessage],
    pre_tokens: usize,
    summary: &str,
) -> std::io::Result<Option<CompactArchiveMeta>> {
    if messages.is_empty() {
        return Ok(None);
    }
    let root = std::env::current_dir()?;
    archive_compacted_range(&root, messages, pre_tokens, summary)
}

fn archive_compacted_range(
    root: &Path,
    messages: &[ChatMessage],
    pre_tokens: usize,
    summary: &str,
) -> std::io::Result<Option<CompactArchiveMeta>> {
    if messages.is_empty() {
        return Ok(None);
    }

    let serialized: Vec<SerializedMessage> = messages.iter().map(serialize_message).collect();
    let created_at = chrono::Utc::now().to_rfc3339();
    let id = archive_id(&created_at, &serialized);
    let archive = CompactArchive {
        schema_version: ARCHIVE_SCHEMA_VERSION,
        id: id.clone(),
        created_at,
        pre_tokens,
        summary: summary.to_owned(),
        messages: serialized,
    };
    let store = jfc_knowledge::block_on_knowledge(async {
        jfc_knowledge::KnowledgeStore::open_default()
            .await
            .map_err(std::io::Error::other)
    })?;
    let session_id = project_artifact_session_id(root);
    let json = serde_json::to_string(&archive).map_err(std::io::Error::other)?;
    jfc_knowledge::block_on_knowledge(async {
        store
            .upsert_session_artifact(&session_id, COMPACT_ARCHIVE_KIND, &id, &json)
            .await
            .map_err(std::io::Error::other)
    })?;

    Ok(Some(CompactArchiveMeta {
        id,
        path: PathBuf::from(session_id),
        message_count: archive.messages.len(),
    }))
}

pub fn render_archive_by_id(id: &str) -> Option<String> {
    let root = std::env::current_dir().ok()?;
    let archive = load_archive(&root, id)?;
    Some(render_archive(&archive))
}

pub fn list_archives(limit: usize) -> Vec<CompactArchiveHit> {
    let Ok(root) = std::env::current_dir() else {
        return Vec::new();
    };
    let mut archives = load_archives(&root);
    archives.sort_by(|a, b| b.created_at.cmp(&a.created_at));
    archives
        .into_iter()
        .take(limit)
        .map(|a| CompactArchiveHit {
            message_count: a.messages.len(),
            snippet: first_nonempty_message(&a).unwrap_or_default(),
            id: a.id,
            created_at: a.created_at,
        })
        .collect()
}

pub fn search_archives(query: &str, limit: usize) -> Vec<CompactArchiveHit> {
    let needle = query.trim().to_lowercase();
    if needle.is_empty() {
        return Vec::new();
    }
    let terms = query_terms(&needle);
    let Ok(root) = std::env::current_dir() else {
        return Vec::new();
    };

    let mut exact = Vec::new();
    let mut soft = Vec::new();
    for archive in load_archives(&root) {
        let text = archive_text(&archive);
        let text_lc = text.to_lowercase();
        let hit = CompactArchiveHit {
            id: archive.id.clone(),
            created_at: archive.created_at.clone(),
            message_count: archive.messages.len(),
            snippet: if text_lc.contains(&needle) {
                snippet_around(&text, &needle)
            } else {
                snippet_around_terms(&text, &terms)
            },
        };
        if text_lc.contains(&needle) {
            exact.push(hit);
        } else {
            let score = score_text(&text_lc, &terms);
            if score > 0 {
                soft.push((score, hit));
            }
        }
    }

    if !exact.is_empty() {
        exact.sort_by(|a, b| b.created_at.cmp(&a.created_at));
        exact.truncate(limit);
        return exact;
    }

    soft.sort_by(|a, b| {
        b.0.cmp(&a.0)
            .then_with(|| b.1.created_at.cmp(&a.1.created_at))
    });
    soft.into_iter().map(|(_, h)| h).take(limit).collect()
}

fn archive_id(created_at: &str, messages: &[SerializedMessage]) -> String {
    let stamp = chrono::Utc::now().format("%Y%m%d-%H%M%S");
    let mut hasher = Sha256::new();
    hasher.update(created_at.as_bytes());
    for msg in messages {
        if let Ok(bytes) = serde_json::to_vec(msg) {
            hasher.update(bytes);
        }
    }
    let digest = hasher.finalize();
    let mut suffix = String::new();
    for byte in digest.iter().take(5) {
        let _ = write!(&mut suffix, "{byte:02x}");
    }
    format!("compact-{stamp}-{suffix}")
}

fn safe_archive_id(id: &str) -> Option<&str> {
    let id = id.trim().strip_suffix(".json").unwrap_or(id.trim());
    if id.is_empty()
        || id.contains('/')
        || id.contains('\\')
        || id.contains("..")
        || !id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_'))
    {
        return None;
    }
    Some(id)
}

fn load_archive(root: &Path, id: &str) -> Option<CompactArchive> {
    let id = safe_archive_id(id)?;
    let store = jfc_knowledge::block_on_knowledge(async {
        jfc_knowledge::KnowledgeStore::open_default().await.ok()
    })?;
    let row = jfc_knowledge::block_on_knowledge(async {
        store
            .get_session_artifact(&project_artifact_session_id(root), COMPACT_ARCHIVE_KIND, id)
            .await
            .ok()
            .flatten()
    })?;
    serde_json::from_str(&row.value_json).ok()
}

fn load_archives(root: &Path) -> Vec<CompactArchive> {
    let Ok(store) = jfc_knowledge::block_on_knowledge(async {
        jfc_knowledge::KnowledgeStore::open_default().await
    }) else {
        return Vec::new();
    };
    let Ok(rows) = jfc_knowledge::block_on_knowledge(async {
        store
            .list_session_artifacts(
                &project_artifact_session_id(root),
                COMPACT_ARCHIVE_KIND,
                500,
            )
            .await
    }) else {
        return Vec::new();
    };
    rows.into_iter()
        .filter_map(|row| serde_json::from_str::<CompactArchive>(&row.value_json).ok())
        .collect()
}

fn project_artifact_session_id(root: &Path) -> String {
    format!("project:{}", jfc_knowledge::project_key(root))
}

fn render_archive(archive: &CompactArchive) -> String {
    let mut out = format!(
        "Compaction archive `{}` ({} messages, pre-compact estimate {} tokens, saved {}).\n\
         Raw messages below are the exact range replaced by the compact summary.\n",
        archive.id,
        archive.messages.len(),
        archive.pre_tokens,
        archive.created_at
    );

    for (idx, serialized) in archive.messages.iter().enumerate() {
        let Some(msg) = deserialize_message_ref(serialized) else {
            continue;
        };
        let text = message_text(&msg);
        if text.trim().is_empty() {
            continue;
        }
        let role = match msg.role {
            Role::User => "user",
            Role::Assistant => "assistant",
        };
        let entry = format!(
            "\n[{role} #{idx}]\n{}\n",
            truncate_chars(text.trim(), MAX_RENDER_MESSAGE_CHARS)
        );
        if out.len() + entry.len() > MAX_RENDER_TOTAL_CHARS {
            out.push_str("\n... [compaction archive truncated]\n");
            break;
        }
        out.push_str(&entry);
    }
    out
}

fn archive_text(archive: &CompactArchive) -> String {
    let mut out = String::new();
    out.push_str(&archive.summary);
    for serialized in &archive.messages {
        let Some(msg) = deserialize_message_ref(serialized) else {
            continue;
        };
        let text = message_text(&msg);
        if !text.trim().is_empty() {
            out.push('\n');
            out.push_str(&text);
        }
    }
    out
}

fn message_text(msg: &ChatMessage) -> String {
    msg.parts
        .iter()
        .map(|part| part.text_only())
        .filter(|s| !s.trim().is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

fn first_nonempty_message(archive: &CompactArchive) -> Option<String> {
    archive.messages.iter().find_map(|serialized| {
        let msg = deserialize_message_ref(serialized)?;
        let text = message_text(&msg);
        let text = text.trim();
        (!text.is_empty()).then(|| truncate_chars(text, 120))
    })
}

fn deserialize_message_ref(serialized: &SerializedMessage) -> Option<ChatMessage> {
    let value = serde_json::to_value(serialized).ok()?;
    let owned = serde_json::from_value(value).ok()?;
    Some(deserialize_message(owned))
}

fn truncate_chars(text: &str, max_chars: usize) -> String {
    let mut chars = text.chars();
    let mut out: String = chars.by_ref().take(max_chars).collect();
    if chars.next().is_some() {
        out.push_str("...");
    }
    out
}

fn snippet_around(text: &str, needle_lc: &str) -> String {
    let text_lc = text.to_lowercase();
    let Some(byte_idx) = text_lc.find(needle_lc) else {
        return truncate_chars(text.trim(), 160);
    };
    let start = text[..byte_idx]
        .char_indices()
        .rev()
        .nth(80)
        .map(|(i, _)| i)
        .unwrap_or(0);
    let end = text[byte_idx..]
        .char_indices()
        .nth(needle_lc.chars().count() + 80)
        .map(|(i, _)| byte_idx + i)
        .unwrap_or(text.len());
    let mut out = String::new();
    if start > 0 {
        out.push_str("...");
    }
    out.push_str(text[start..end].trim());
    if end < text.len() {
        out.push_str("...");
    }
    out
}

fn snippet_around_terms(text: &str, terms: &[String]) -> String {
    for term in terms {
        if let Some(snippet) = text
            .lines()
            .find(|line| line.to_lowercase().contains(term))
            .map(|line| truncate_chars(line.trim(), 160))
        {
            return snippet;
        }
    }
    truncate_chars(text.trim(), 160)
}

fn query_terms(query: &str) -> Vec<String> {
    let mut terms = Vec::new();
    for raw in query
        .split(|c: char| !c.is_ascii_alphanumeric())
        .map(|s| stem(s.trim()))
        .filter(|s| s.len() >= 3 && !is_stopword(s))
    {
        if !terms.iter().any(|t| t == &raw) {
            terms.push(raw);
        }
    }
    terms
}

fn score_text(text_lc: &str, terms: &[String]) -> usize {
    if terms.is_empty() {
        return 0;
    }
    let mut score = 0usize;
    for token in text_lc
        .split(|c: char| !c.is_ascii_alphanumeric())
        .map(|s| stem(s.trim()))
        .filter(|s| s.len() >= 3)
    {
        if terms.contains(&token) {
            score += 1;
        }
    }
    score
}

fn stem(token: &str) -> String {
    let mut s = token.to_ascii_lowercase();
    if s.len() > 6 && s.ends_with("tion") {
        s.truncate(s.len() - 3);
        return s;
    }
    for suffix in ["ations", "ation", "ions", "ing", "ers", "ed", "es", "s"] {
        if s.len() > suffix.len() + 3 && s.ends_with(suffix) {
            s.truncate(s.len() - suffix.len());
            break;
        }
    }
    s
}

fn is_stopword(term: &str) -> bool {
    matches!(
        term,
        "the"
            | "and"
            | "for"
            | "with"
            | "from"
            | "that"
            | "this"
            | "what"
            | "when"
            | "where"
            | "why"
            | "how"
            | "into"
            | "about"
            | "there"
            | "their"
            | "have"
            | "has"
            | "was"
            | "were"
            | "are"
            | "not"
            | "you"
            | "your"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn safe_archive_id_rejects_paths_robust() {
        assert_eq!(
            safe_archive_id("compact-20260616-abcd.json"),
            Some("compact-20260616-abcd")
        );
        assert!(safe_archive_id("../secret").is_none());
        assert!(safe_archive_id("foo/bar").is_none());
    }

    #[test]
    fn soft_score_stems_related_terms_normal() {
        let terms = query_terms("compaction loops");
        assert!(score_text("compact loop retry compacted", &terms) > 0);
    }
}
