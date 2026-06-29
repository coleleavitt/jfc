use super::{load_archives, render::archive_text};
use std::path::Path;

const MAX_SNIPPET_CHARS: usize = 900;
const MAX_RECALL_BLOCK_CHARS: usize = 6_000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ProviderHistoryArchiveHit {
    pub(crate) id: String,
    pub(crate) created_at: String,
    pub(crate) message_count: usize,
    pub(crate) snippet: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ProviderHistoryArchiveRecall {
    pub(crate) block: String,
    pub(crate) archive_ids: Vec<String>,
}

pub(crate) fn list_provider_history_archives(limit: usize) -> Vec<ProviderHistoryArchiveHit> {
    let Ok(root) = std::env::current_dir() else {
        return Vec::new();
    };
    let mut archives = load_archives(&root);
    archives.sort_by(|a, b| b.created_at.cmp(&a.created_at));
    archives
        .into_iter()
        .take(limit)
        .map(|archive| {
            let text = archive_text(&archive);
            ProviderHistoryArchiveHit {
                id: archive.id,
                created_at: archive.created_at,
                message_count: archive.messages.len(),
                snippet: first_nonempty_line(&text).unwrap_or_default(),
            }
        })
        .collect()
}

pub(crate) fn search_provider_history_archives(
    query: &str,
    limit: usize,
) -> Vec<ProviderHistoryArchiveHit> {
    let Ok(root) = std::env::current_dir() else {
        return Vec::new();
    };
    search_provider_history_archives_in(&root, query, limit)
}

pub(crate) fn search_provider_history_archives_in(
    root: &Path,
    query: &str,
    limit: usize,
) -> Vec<ProviderHistoryArchiveHit> {
    let needle = query.trim().to_lowercase();
    if needle.is_empty() {
        return Vec::new();
    }
    let terms = query_terms(&needle);

    let mut exact = Vec::new();
    let mut soft = Vec::new();
    for archive in load_archives(root) {
        let text = archive_text(&archive);
        let text_lc = text.to_lowercase();
        let hit = ProviderHistoryArchiveHit {
            id: archive.id,
            created_at: archive.created_at,
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
    soft.into_iter().map(|(_, hit)| hit).take(limit).collect()
}

pub(crate) fn provider_history_archive_recall_block(
    query: &str,
    limit: usize,
    seen_archive_ids: &std::collections::BTreeSet<String>,
) -> Option<ProviderHistoryArchiveRecall> {
    let hits = search_provider_history_archives(query, limit);
    let hits: Vec<_> = hits
        .into_iter()
        .filter(|hit| !seen_archive_ids.contains(&hit.id))
        .collect();
    if hits.is_empty() {
        return None;
    }

    let mut out = String::from(
        "\n\n<provider-history-recall source=\"local_archive\" bounded=\"true\">\n\
         These snippets are from older provider-visible messages that were archived after a context overflow transform. \
         Treat them as continuity hints, not current live transcript. Use `/expand <id>` when exact replay is needed.\n",
    );
    let mut appended = 0;
    let mut archive_ids = Vec::new();
    for hit in hits {
        let entry = format!(
            "\n<archive id=\"{}\" created_at=\"{}\" messages=\"{}\">\n{}\n</archive>\n",
            hit.id,
            hit.created_at,
            hit.message_count,
            truncate_chars(hit.snippet.trim(), MAX_SNIPPET_CHARS),
        );
        if out.len() + entry.len() + "</provider-history-recall>\n".len() > MAX_RECALL_BLOCK_CHARS {
            break;
        }
        out.push_str(&entry);
        archive_ids.push(hit.id);
        appended += 1;
    }
    if appended == 0 {
        return None;
    }
    out.push_str("</provider-history-recall>\n");
    Some(ProviderHistoryArchiveRecall {
        block: out,
        archive_ids,
    })
}

fn first_nonempty_line(text: &str) -> Option<String> {
    text.lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map(|line| truncate_chars(line, 160))
}

fn query_terms(needle: &str) -> Vec<&str> {
    needle
        .split(|c: char| !c.is_alphanumeric())
        .filter(|term| term.chars().count() >= 2)
        .take(12)
        .collect()
}

fn score_text(text_lc: &str, terms: &[&str]) -> usize {
    terms.iter().map(|term| text_lc.matches(term).count()).sum()
}

fn snippet_around_terms(text: &str, terms: &[&str]) -> String {
    for term in terms {
        let text_lc = text.to_lowercase();
        if text_lc.contains(term) {
            return snippet_around(text, term);
        }
    }
    truncate_chars(text.trim(), MAX_SNIPPET_CHARS)
}

fn snippet_around(text: &str, needle_lc: &str) -> String {
    let text_lc = text.to_lowercase();
    let Some(byte_idx) = text_lc.find(needle_lc) else {
        return truncate_chars(text.trim(), MAX_SNIPPET_CHARS);
    };
    let byte_idx = floor_char_boundary(text, byte_idx.min(text.len()));
    let start = text[..byte_idx]
        .char_indices()
        .rev()
        .nth(160)
        .map(|(idx, _)| idx)
        .unwrap_or(0);
    let end = text[byte_idx..]
        .char_indices()
        .nth(needle_lc.chars().count() + 160)
        .map(|(idx, _)| byte_idx + idx)
        .unwrap_or(text.len());
    let end = floor_char_boundary(text, end.min(text.len()));

    let mut out = String::new();
    if start > 0 {
        out.push_str("...");
    }
    out.push_str(text[start..end].trim());
    if end < text.len() {
        out.push_str("...");
    }
    truncate_chars(&out, MAX_SNIPPET_CHARS)
}

fn floor_char_boundary(text: &str, mut idx: usize) -> usize {
    while idx > 0 && !text.is_char_boundary(idx) {
        idx -= 1;
    }
    idx
}

fn truncate_chars(text: &str, max_chars: usize) -> String {
    let mut chars = text.chars();
    let mut out: String = chars.by_ref().take(max_chars).collect();
    if chars.next().is_some() {
        out.push_str("...");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snippet_around_handles_non_ascii_without_panicking_regression() {
        let text = "before café-context after";
        let snippet = snippet_around(text, "café");

        assert!(snippet.contains("café-context"));
    }

    #[test]
    fn recall_block_is_bounded_regression() {
        let long = "x".repeat(MAX_RECALL_BLOCK_CHARS * 2);
        let hit = ProviderHistoryArchiveHit {
            id: "provider-history-test".to_owned(),
            created_at: "2026-06-26T00:00:00Z".to_owned(),
            message_count: 1,
            snippet: long,
        };
        let entry = format!(
            "\n<archive id=\"{}\" created_at=\"{}\" messages=\"{}\">\n{}\n</archive>\n",
            hit.id,
            hit.created_at,
            hit.message_count,
            truncate_chars(hit.snippet.trim(), MAX_SNIPPET_CHARS),
        );

        assert!(entry.len() < MAX_RECALL_BLOCK_CHARS);
    }
}
