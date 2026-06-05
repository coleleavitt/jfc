//! AutoSearchHints — formatting recall hints for auto-injection into context.
//!
//! Two layers:
//!   1. `RecallHint` + `format_hint` — the in-memory hint payload + renderer.
//!   2. `run_pre_turn_hint` — pre-turn scan that walks the user's prompt for
//!      code-path / symbol mentions, looks them up in `.jfc/memory/`, and
//!      returns a ready-to-inject `<!-- recall: ... -->` block.

use std::path::Path;

use serde::{Deserialize, Serialize};

// ─── Types ──────────────────────────────────────────────────────────────────

/// Source of a recall hint.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum HintSource {
    Memory {
        category: String,
    },
    Session {
        date: String,
        file_ref: Option<String>,
    },
    GitCommit {
        sha: String,
    },
}

/// A recall hint with score.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecallHint {
    pub source: HintSource,
    pub content: String,
    pub score: f32,
}

// ─── Functions ──────────────────────────────────────────────────────────────

/// Format hints as `<!-- recall: ... -->` HTML comments.
pub fn format_hint(hints: &[RecallHint]) -> String {
    if hints.is_empty() {
        return String::new();
    }

    let mut out = String::new();
    for hint in hints {
        let source_label = match &hint.source {
            HintSource::Memory { category } => format!("memory:{}", category),
            HintSource::Session { date, file_ref } => {
                if let Some(f) = file_ref {
                    format!("session:{}:{}", date, f)
                } else {
                    format!("session:{}", date)
                }
            }
            HintSource::GitCommit { sha } => format!("git:{}", sha),
        };
        out.push_str(&format!(
            "<!-- recall: [{}] (score={:.2}) {} -->\n",
            source_label, hint.score, hint.content
        ));
    }
    out
}

/// Returns true if any hint score >= min_score.
pub fn should_append_hint(hints: &[RecallHint], min_score: f32) -> bool {
    hints.iter().any(|h| h.score >= min_score)
}

// ─── Pre-turn hint scan ─────────────────────────────────────────────────────

/// Cheap regex-free mention detector. Looks for tokens that look like
/// `foo/bar/baz.rs`, `module::Symbol`, or bare CamelCase symbols longer
/// than 3 chars. Returns deduplicated mentions in source order.
pub fn extract_mentions(query: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();

    for raw in
        query.split(|c: char| c.is_whitespace() || matches!(c, '(' | ')' | ',' | ';' | '"' | '\''))
    {
        let token = raw.trim_matches(|c: char| c == '.' || c == ':' || c == '`');
        if token.is_empty() {
            continue;
        }

        let is_path = token.contains('/') && {
            let last = token.rsplit('/').next().unwrap_or("");
            last.contains('.')
                && last
                    .rsplit('.')
                    .next()
                    .map(|ext| {
                        let ext = ext.to_ascii_lowercase();
                        // common code extensions
                        matches!(
                            ext.as_str(),
                            "rs" | "py"
                                | "ts"
                                | "tsx"
                                | "js"
                                | "jsx"
                                | "go"
                                | "c"
                                | "h"
                                | "cpp"
                                | "hpp"
                                | "java"
                                | "kt"
                                | "swift"
                                | "rb"
                                | "md"
                                | "toml"
                                | "yaml"
                                | "yml"
                                | "json"
                        )
                    })
                    .unwrap_or(false)
        };
        let is_qualified = token.contains("::") && token.chars().any(|c| c.is_alphabetic());
        let is_camel = token.len() >= 4
            && token
                .chars()
                .next()
                .map(|c| c.is_ascii_uppercase())
                .unwrap_or(false)
            && token.chars().skip(1).any(|c| c.is_ascii_lowercase())
            && token.chars().all(|c| c.is_alphanumeric() || c == '_');

        if (is_path || is_qualified || is_camel) && seen.insert(token.to_string()) {
            out.push(token.to_string());
        }
    }

    out
}

/// Walk a directory of `.md` memory files looking for any that mention any of
/// `needles` (case-insensitive substring match). Returns `RecallHint`s built
/// from the first matching line of each file. Caps at `max_hints`.
pub fn scan_memory_dir(dir: &Path, needles: &[String], max_hints: usize) -> Vec<RecallHint> {
    let mut out = Vec::new();
    if needles.is_empty() {
        return out;
    }
    let lower_needles: Vec<String> = needles.iter().map(|n| n.to_lowercase()).collect();

    let Ok(read_dir) = std::fs::read_dir(dir) else {
        return out;
    };
    for entry in read_dir.flatten() {
        if out.len() >= max_hints {
            break;
        }
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("md") {
            continue;
        }
        let Ok(content) = std::fs::read_to_string(&path) else {
            continue;
        };
        let lower = content.to_lowercase();
        // First needle that hits wins. Score = matches/needles.len() capped 1.0.
        let hits = lower_needles
            .iter()
            .filter(|n| lower.contains(n.as_str()))
            .count();
        if hits == 0 {
            continue;
        }
        // Skip YAML frontmatter, then take first non-empty body line.
        let body = strip_frontmatter(&content);
        let preview = body
            .lines()
            .find(|l| !l.trim().is_empty())
            .unwrap_or("(memory)")
            .trim()
            .chars()
            .take(160)
            .collect::<String>();
        let category = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("memory")
            .to_owned();
        let score = (hits as f32 / lower_needles.len() as f32).min(1.0);
        out.push(RecallHint {
            source: HintSource::Memory { category },
            content: preview,
            score,
        });
    }
    out
}

/// Drop the leading `---\n…\n---` YAML frontmatter block, if any.
fn strip_frontmatter(content: &str) -> &str {
    let trimmed = content.trim_start();
    let Some(rest) = trimmed.strip_prefix("---") else {
        return content;
    };
    // Skip the leading newline after the opening `---`.
    let rest = rest.trim_start_matches('\n');
    // Find the next line that is exactly `---`.
    let mut idx = 0usize;
    for line in rest.split_inclusive('\n') {
        let l = line.trim_end_matches('\n');
        if l == "---" {
            let after = &rest[idx + line.len()..];
            return after.trim_start_matches('\n');
        }
        idx += line.len();
    }
    content
}

/// Default memory directories scanned by `run_pre_turn_hint`.
pub fn default_memory_dirs(project_root: &Path) -> Vec<std::path::PathBuf> {
    let mut out = Vec::new();
    out.push(project_root.join(".jfc").join("memory"));
    out.push(project_root.join(".jfc").join("memory").join("team"));
    if let Some(cfg) = ::dirs::config_dir() {
        out.push(cfg.join("jfc").join("memory"));
    }
    out
}

/// Pre-turn recall hint pipeline: extract code-path/symbol mentions from
/// `query`, scan project + user memory for matches, and return a formatted
/// `<!-- recall: ... -->` block ready to splice into the system prompt.
///
/// Returns `None` when nothing matches above `min_score` (default 0.3).
pub fn run_pre_turn_hint(query: &str, project_root: &Path) -> Option<String> {
    run_pre_turn_hint_with(query, project_root, 0.3, 5)
}

/// Configurable variant of `run_pre_turn_hint`. Exposed for tests + callers
/// that want a tighter cap.
pub fn run_pre_turn_hint_with(
    query: &str,
    project_root: &Path,
    min_score: f32,
    max_hints: usize,
) -> Option<String> {
    let mentions = extract_mentions(query);
    if mentions.is_empty() {
        return None;
    }

    let mut all_hints = Vec::new();
    for dir in default_memory_dirs(project_root) {
        for h in scan_memory_dir(&dir, &mentions, max_hints) {
            all_hints.push(h);
            if all_hints.len() >= max_hints {
                break;
            }
        }
        if all_hints.len() >= max_hints {
            break;
        }
    }

    let kept: Vec<RecallHint> = all_hints
        .into_iter()
        .filter(|h| h.score >= min_score)
        .collect();
    if kept.is_empty() {
        return None;
    }
    Some(format_hint(&kept))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_hint_produces_comment_normal() {
        let hints = vec![RecallHint {
            source: HintSource::Memory {
                category: "ARCHITECTURE_DECISIONS".to_string(),
            },
            content: "Project uses serde for serialization".to_string(),
            score: 0.85,
        }];

        let formatted = format_hint(&hints);
        assert!(formatted.contains("<!-- recall:"));
        assert!(formatted.contains("memory:ARCHITECTURE_DECISIONS"));
        assert!(formatted.contains("score=0.85"));
        assert!(formatted.contains("Project uses serde"));
        assert!(formatted.contains("-->"));
    }

    #[test]
    fn threshold_filters_low_scores_normal() {
        let hints = vec![
            RecallHint {
                source: HintSource::GitCommit {
                    sha: "abc123".to_string(),
                },
                content: "Some commit".to_string(),
                score: 0.3,
            },
            RecallHint {
                source: HintSource::Memory {
                    category: "NAMING".to_string(),
                },
                content: "Use snake_case".to_string(),
                score: 0.4,
            },
        ];

        assert!(!should_append_hint(&hints, 0.5));
        assert!(should_append_hint(&hints, 0.3));
    }

    #[test]
    fn empty_hints_returns_empty_robust() {
        let hints: Vec<RecallHint> = Vec::new();
        let formatted = format_hint(&hints);
        assert!(formatted.is_empty());
        assert!(!should_append_hint(&hints, 0.0));
    }

    // ─── Pre-turn hint scan ────────────────────────────────────────────

    #[test]
    fn extract_mentions_finds_path_and_symbol_normal() {
        let q = "Please update crates/jfc-ui/src/main.rs and the StreamHandler module::Foo.";
        let mentions = extract_mentions(q);
        assert!(
            mentions.iter().any(|m| m == "crates/jfc-ui/src/main.rs"),
            "expected file path mention, got {:?}",
            mentions
        );
        assert!(
            mentions.iter().any(|m| m == "module::Foo"),
            "expected qualified symbol, got {:?}",
            mentions
        );
        assert!(
            mentions.iter().any(|m| m == "StreamHandler"),
            "expected CamelCase symbol, got {:?}",
            mentions
        );
    }

    #[test]
    fn extract_mentions_empty_robust() {
        assert!(extract_mentions("").is_empty());
        assert!(extract_mentions("just plain english here").is_empty());
        // Single-word lowercase, no `::`, no path → ignored.
        assert!(extract_mentions("foo bar baz").is_empty());
    }

    #[test]
    fn run_pre_turn_hint_injects_recall_when_path_in_memory_normal() {
        use std::fs;
        let tmp = tempfile::TempDir::new().unwrap();
        let mem_dir = tmp.path().join(".jfc").join("memory");
        fs::create_dir_all(&mem_dir).unwrap();
        fs::write(
            mem_dir.join("note.md"),
            "---\ntype: project\nscope: private\n---\n\
             The streaming pipeline lives in crates/jfc-ui/src/stream/request.rs and \
             enforces tool_choice=auto.",
        )
        .unwrap();

        let q = "what does crates/jfc-ui/src/stream/request.rs do for tool choice";
        let block = run_pre_turn_hint(q, tmp.path()).expect("expected a hint block");
        assert!(block.contains("<!-- recall:"));
        assert!(block.contains("note.md"));
        assert!(block.contains("streaming pipeline"));
    }

    #[test]
    fn run_pre_turn_hint_returns_none_when_no_mentions_robust() {
        let tmp = tempfile::TempDir::new().unwrap();
        let mem_dir = tmp.path().join(".jfc").join("memory");
        std::fs::create_dir_all(&mem_dir).unwrap();
        std::fs::write(mem_dir.join("a.md"), "anything").unwrap();

        let block = run_pre_turn_hint("hello how are you", tmp.path());
        assert!(block.is_none());
    }

    #[test]
    fn run_pre_turn_hint_returns_none_when_no_matches_robust() {
        let tmp = tempfile::TempDir::new().unwrap();
        let mem_dir = tmp.path().join(".jfc").join("memory");
        std::fs::create_dir_all(&mem_dir).unwrap();
        std::fs::write(mem_dir.join("a.md"), "totally unrelated content").unwrap();

        let block = run_pre_turn_hint("touch path/to/Other.rs please", tmp.path());
        assert!(block.is_none());
    }
}
