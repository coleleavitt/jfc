//! Git-commit search source for cross-session recall.
//!
//! magic-context (cortexkit) folds a per-project HEAD commit corpus into its
//! unified search so "when/why did we change X" is answerable alongside memory
//! and message history. This is the dependency-free jfc analogue: it shells out
//! to `git log` (already a hard dependency of the workflow) and keyword-filters
//! the result. No index DB, no second source of truth — the repo is the corpus.
//!
//! Pairs with [`crate::search`]: that searches past *sessions*; this searches
//! past *commits*. A future unified facade can rank across both.

use crate::soft_match::{best_line, query_terms, score_text};
use serde::Serialize;
use std::path::Path;
use std::process::Command;

/// One commit hit.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct CommitHit {
    /// Abbreviated commit hash.
    pub short_hash: String,
    /// Author date, ISO-8601 (`%cI`).
    pub date: String,
    /// First line of the commit message.
    pub subject: String,
    /// The matching line from the commit body (or the subject if the match was
    /// in the subject), trimmed for display.
    pub snippet: String,
}

/// Unit-separator and record-separator used to frame `git log` output so commit
/// messages (which contain newlines) parse unambiguously.
const FIELD_SEP: char = '\x1f'; // ASCII US
const REC_SEP: char = '\x1e'; // ASCII RS

/// Run `git log` in `repo_root` and return raw `(hash, date, subject, body)`
/// tuples, newest first. Returns an empty vec if git is unavailable or the dir
/// isn't a repo (callers treat "no commits" and "no git" identically).
fn read_commits(repo_root: &Path, max_commits: usize) -> Vec<(String, String, String, String)> {
    // %h short hash, %cI committer date ISO, %s subject, %b body — framed with
    // FIELD_SEP between fields and REC_SEP between records.
    let format = format!("--pretty=format:%h{FIELD_SEP}%cI{FIELD_SEP}%s{FIELD_SEP}%b{REC_SEP}");
    let output = Command::new("git")
        .arg("-C")
        .arg(repo_root)
        .arg("log")
        .arg(format!("--max-count={max_commits}"))
        .arg(format)
        .output();
    let Ok(output) = output else {
        return Vec::new();
    };
    if !output.status.success() {
        return Vec::new();
    }
    let text = String::from_utf8_lossy(&output.stdout);
    text.split(REC_SEP)
        .filter(|rec| !rec.trim().is_empty())
        .filter_map(|rec| {
            let mut fields = rec.trim_start_matches('\n').splitn(4, FIELD_SEP);
            let hash = fields.next()?.trim().to_string();
            let date = fields.next()?.trim().to_string();
            let subject = fields.next()?.to_string();
            let body = fields.next().unwrap_or("").to_string();
            if hash.is_empty() {
                return None;
            }
            Some((hash, date, subject, body))
        })
        .collect()
}

/// Search the project's commit messages (subject + body) for `query`
/// (case-insensitive substring), newest first. Returns up to `limit` hits.
pub fn search(repo_root: &Path, query: &str, limit: usize, max_commits: usize) -> Vec<CommitHit> {
    let needle = query.trim().to_lowercase();
    if needle.is_empty() {
        return Vec::new();
    }
    let terms = query_terms(&needle);
    let mut exact_hits = Vec::new();
    let mut soft_hits = Vec::new();
    for (short_hash, date, subject, body) in read_commits(repo_root, max_commits) {
        let subject_match = subject.to_lowercase().contains(&needle);
        // Find the first body line that matches, for the snippet.
        let body_line = body
            .lines()
            .find(|l| l.to_lowercase().contains(&needle))
            .map(str::trim);
        if subject_match || body_line.is_some() {
            let snippet = if subject_match {
                subject.clone()
            } else {
                body_line.unwrap_or(&subject).to_string()
            };
            exact_hits.push(CommitHit {
                short_hash,
                date,
                subject,
                snippet,
            });
            if exact_hits.len() >= limit {
                break;
            }
        } else {
            let combined = format!("{subject}\n{body}");
            let score = score_text(&combined, &terms);
            if score > 0 {
                let snippet = best_line(&combined, &terms)
                    .unwrap_or(&subject)
                    .trim()
                    .to_string();
                soft_hits.push((
                    score,
                    CommitHit {
                        short_hash,
                        date,
                        subject,
                        snippet,
                    },
                ));
            }
        }
    }
    if !exact_hits.is_empty() {
        return exact_hits;
    }
    soft_hits.sort_by_key(|hit| std::cmp::Reverse(hit.0));
    soft_hits.into_iter().map(|(_, h)| h).take(limit).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command as Cmd;

    fn git(dir: &Path, args: &[&str]) {
        // Keep temp repos hermetic: a developer's *global* git config (a
        // `core.hooksPath` commit-msg linter, `commit.gpgsign`, etc.) must not
        // leak in and fail an otherwise-clean temp commit. Per-invocation `-c`
        // overrides take precedence over global/system config and are
        // cross-platform — empty `core.hooksPath` disables inherited hooks and
        // `*.gpgsign=false` disables inherited commit/tag signing.
        let ok = Cmd::new("git")
            .args([
                "-c",
                "core.hooksPath=",
                "-c",
                "commit.gpgsign=false",
                "-c",
                "tag.gpgsign=false",
            ])
            .arg("-C")
            .arg(dir)
            .args(args)
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);
        assert!(ok, "git {args:?} failed");
    }

    fn temp_repo() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path();
        git(p, &["init", "-q"]);
        git(p, &["config", "user.email", "t@t"]);
        git(p, &["config", "user.name", "t"]);
        std::fs::write(p.join("a.txt"), "x").unwrap();
        git(p, &["add", "."]);
        git(
            p,
            &[
                "commit",
                "-q",
                "-m",
                "feat: add the widget parser\n\nHandles edge cases in parsing.",
            ],
        );
        std::fs::write(p.join("b.txt"), "y").unwrap();
        git(p, &["add", "."]);
        git(
            p,
            &["commit", "-q", "-m", "fix: correct off-by-one in loop"],
        );
        dir
    }

    // Normal: a subject keyword match returns the commit, subject as snippet.
    #[test]
    fn search_matches_subject_normal() {
        let repo = temp_repo();
        let hits = search(repo.path(), "off-by-one", 10, 100);
        assert_eq!(hits.len(), 1);
        assert!(hits[0].subject.contains("off-by-one"));
        assert!(!hits[0].short_hash.is_empty());
    }

    // Robust: a body-only keyword match returns the matching body line as snippet.
    #[test]
    fn search_matches_body_robust() {
        let repo = temp_repo();
        let hits = search(repo.path(), "edge cases", 10, 100);
        assert_eq!(hits.len(), 1);
        assert!(hits[0].subject.contains("widget parser"));
        assert!(hits[0].snippet.to_lowercase().contains("edge cases"));
    }

    // Robust: an empty query and a non-repo dir both yield no hits (no panic).
    #[test]
    fn search_empty_and_non_repo_robust() {
        let repo = temp_repo();
        assert!(search(repo.path(), "   ", 10, 100).is_empty());
        let non_repo = tempfile::tempdir().unwrap();
        assert!(search(non_repo.path(), "anything", 10, 100).is_empty());
    }

    // Normal: limit caps the number of hits.
    #[test]
    fn search_respects_limit_normal() {
        let repo = temp_repo();
        // Both commits contain a lowercase letter; match the common ":" via "fix"/"feat".
        let hits = search(repo.path(), "e", 1, 100);
        assert_eq!(hits.len(), 1);
    }
}
