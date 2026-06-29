use crate::commands::prelude::*;
use crate::context_accounting::ProviderHistoryArchiveHit;
use jfc_session::{CommitHit, SessionHit};
use std::path::Path;

const DEFAULT_CONTEXT_SEARCH_LIMIT: usize = 10;
const SESSION_CONTEXT_WINDOW: usize = 2;
const MAX_COMMITS_SEARCHED: usize = 250;

enum ContextSearchHit {
    Session(SessionHit),
    ProviderHistoryArchive(ProviderHistoryArchiveHit),
    Commit(CommitHit),
}

pub(super) async fn cmd_ctx_search(
    state: &mut EngineState,
    _parts: &[&str],
    text: &str,
    _tx: Option<&mpsc::Sender<EngineEvent>>,
) {
    state.messages.push(ChatMessage::user(text.to_owned()));
    let query = command_query_text(text);
    if query.is_empty() {
        state.messages.push(ChatMessage::assistant(
            "Usage: `/ctx-search <query>` searches prior sessions, provider-history archives, and git commits.".into(),
        ));
        return;
    }

    let cwd = PathBuf::from(&state.cwd);
    let exclude_session = state.current_session_id.as_ref().map(|id| id.as_str());
    let hits = collect_context_hits(&cwd, query, exclude_session, DEFAULT_CONTEXT_SEARCH_LIMIT);
    state
        .messages
        .push(ChatMessage::assistant(format_context_search_results(
            query, &hits,
        )));
}

fn command_query_text(text: &str) -> &str {
    let trimmed = text.trim();
    let Some(idx) = trimmed.find(char::is_whitespace) else {
        return "";
    };
    trimmed[idx..].trim()
}

fn collect_context_hits(
    repo_root: &Path,
    query: &str,
    exclude_session: Option<&str>,
    limit: usize,
) -> Vec<ContextSearchHit> {
    let session_hits = jfc_session::search_sessions_excluding(
        query,
        limit,
        SESSION_CONTEXT_WINDOW,
        exclude_session,
    )
    .into_iter()
    .map(ContextSearchHit::Session);
    let commit_hits = jfc_session::search_commits(repo_root, query, limit, MAX_COMMITS_SEARCHED)
        .into_iter()
        .map(ContextSearchHit::Commit);
    let archive_hits =
        crate::context_accounting::search_provider_history_archives_in(repo_root, query, limit)
            .into_iter()
            .map(ContextSearchHit::ProviderHistoryArchive);

    let mut hits = Vec::with_capacity(limit);
    let mut sessions = session_hits.peekable();
    let mut archives = archive_hits.peekable();
    let mut commits = commit_hits.peekable();
    while hits.len() < limit
        && (sessions.peek().is_some() || archives.peek().is_some() || commits.peek().is_some())
    {
        if let Some(hit) = sessions.next() {
            hits.push(hit);
            if hits.len() >= limit {
                break;
            }
        }
        if let Some(hit) = archives.next() {
            hits.push(hit);
            if hits.len() >= limit {
                break;
            }
        }
        if let Some(hit) = commits.next() {
            hits.push(hit);
        }
    }
    hits
}

fn format_context_search_results(query: &str, hits: &[ContextSearchHit]) -> String {
    if hits.is_empty() {
        return format!("No prior session or git commit context matched `{query}`.");
    }
    let mut body = format!("Context search results for `{query}`:\n\n");
    for (index, hit) in hits.iter().enumerate() {
        match hit {
            ContextSearchHit::Session(hit) => {
                body.push_str(&format!(
                    "{}. [message] session `{}` msg={} updated={}\n   {}\n   title: {}\n\n",
                    index + 1,
                    hit.session_id,
                    hit.match_index,
                    hit.updated_at,
                    truncate_line(&hit.snippet, 240),
                    truncate_line(&hit.title, 120),
                ));
            }
            ContextSearchHit::ProviderHistoryArchive(hit) => {
                body.push_str(&format!(
                    "{}. [provider_history_archive] `{}` saved={} messages={}\n   {}\n\n",
                    index + 1,
                    hit.id,
                    hit.created_at.chars().take(19).collect::<String>(),
                    hit.message_count,
                    truncate_line(&hit.snippet, 240),
                ));
            }
            ContextSearchHit::Commit(hit) => {
                body.push_str(&format!(
                    "{}. [git_commit] `{}` {}\n   {}\n   {}\n\n",
                    index + 1,
                    hit.short_hash,
                    hit.date.chars().take(10).collect::<String>(),
                    truncate_line(&hit.subject, 160),
                    truncate_line(&hit.snippet, 240),
                ));
            }
        }
    }
    body.push_str(
        "Use `/resume <session_id>` for a session hit, `/expand <archive-id>` for an archive hit, or `git show <sha>` for a commit hit.",
    );
    body
}

fn truncate_line(text: &str, max_chars: usize) -> String {
    let normalized = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut chars = normalized.chars();
    let mut out: String = chars.by_ref().take(max_chars).collect();
    if chars.next().is_some() {
        out.push_str("...");
    }
    out
}

#[cfg(test)]
#[path = "context_search/tests.rs"]
mod tests;
