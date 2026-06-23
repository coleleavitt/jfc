//! Idempotent importer: bring existing `.md` memory files into the store.
//!
//! This is the migration path from the legacy per-file `.md` memories
//! (`jfc-memory`'s `~/.config/jfc/memory/` + `<repo>/.jfc/memory/`) into the
//! unified knowledge DB. It is deliberately **self-contained** — it parses the
//! YAML-ish frontmatter itself rather than depending on `jfc-memory`, so the
//! storage crate stays dependency-light and the "where are the dirs" policy
//! lives in the caller (the engine `/knowledge import` command).
//!
//! **Import only — it never deletes the source files.** Re-import is a no-op:
//! each memory maps to a deterministic id (uuid-v5 over its normalized content),
//! so running the importer twice adds rows once. This is the safe half of the
//! `.md` → DB cutover; deleting originals is a separate, user-gated step.

use std::path::{Path, PathBuf};

use crate::record::{Kind, Scope};

/// A memory ready to be imported. A plain data carrier with no external deps so
/// the importer doesn't couple the storage crate to the memory-file format.
#[derive(Debug, Clone)]
pub struct ImportableMemory {
    pub source_path: Option<PathBuf>,
    pub kind: Kind,
    pub scope: Scope,
    pub project_key: Option<String>,
    pub title: String,
    pub body: String,
}

/// Outcome of an import run.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ImportReport {
    /// New rows inserted this run.
    pub imported: usize,
    /// Items already present (deterministic-id match) — skipped.
    pub skipped: usize,
    /// Per-item errors (e.g. invalid record), as human-readable strings.
    pub errors: Vec<String>,
}

/// Deterministic id for an importable memory: uuid-v5 over scope + project +
/// normalized body, so the same memory file always maps to the same row and
/// re-import is idempotent. Body is normalized (trim + collapse whitespace) so
/// trivial reformatting doesn't create a duplicate.
pub fn deterministic_id(item: &ImportableMemory) -> String {
    let norm_body = item.body.split_whitespace().collect::<Vec<_>>().join(" ");
    let basis = format!(
        "md:{}:{}:{}",
        item.scope.slug(),
        item.project_key.as_deref().unwrap_or(""),
        norm_body
    );
    uuid::Uuid::new_v5(&uuid::Uuid::NAMESPACE_OID, basis.as_bytes())
        .simple()
        .to_string()
}

/// Map a legacy memory `type:` frontmatter value to a [`Kind`].
fn kind_from_type(type_field: &str) -> Kind {
    match type_field.trim().to_ascii_lowercase().as_str() {
        "user" | "preference" => Kind::Preference,
        // Feedback = corrections/confirmations of approach → a repeatable finding.
        "feedback" => Kind::Finding,
        // Project = ongoing work/decisions; context = external-system pointers.
        "project" | "context" => Kind::Fact,
        _ => Kind::Fact,
    }
}

/// Parse one `.md` memory file (frontmatter + body) into an [`ImportableMemory`].
///
/// Returns `None` when the body is empty (nothing to store). `default_scope` is
/// used when the frontmatter doesn't pin a scope; `project_key` must be `Some`
/// when `default_scope` is [`Scope::Project`].
pub fn parse_markdown_memory(
    path: &Path,
    content: &str,
    default_scope: Scope,
    project_key: Option<String>,
) -> Option<ImportableMemory> {
    let (frontmatter, body) = split_frontmatter(content);
    let body = body.trim();
    if body.is_empty() {
        return None;
    }

    let kind = frontmatter
        .iter()
        .find(|(k, _)| k == "type")
        .map(|(_, v)| kind_from_type(v))
        .unwrap_or(Kind::Fact);

    // Title: first non-empty body line (a memory leads with its rule/fact),
    // truncated; fall back to the file stem.
    let title = body
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .map(|l| l.chars().take(80).collect::<String>())
        .or_else(|| path.file_stem().and_then(|s| s.to_str()).map(str::to_owned))
        .unwrap_or_else(|| "imported memory".to_owned());

    Some(ImportableMemory {
        source_path: Some(path.to_path_buf()),
        kind,
        scope: default_scope,
        project_key: if default_scope == Scope::Project {
            project_key
        } else {
            None
        },
        title,
        body: body.to_owned(),
    })
}

/// Scan a directory tree for `*.md` memory files and parse each into an
/// [`ImportableMemory`]. Non-`.md` files are ignored. Unreadable/empty files are
/// skipped silently (they're not errors — there's just nothing to import).
pub fn scan_markdown_dir(
    dir: &Path,
    scope: Scope,
    project_key: Option<String>,
) -> Vec<ImportableMemory> {
    let mut out = Vec::new();
    scan_dir_recursive(dir, scope, project_key.as_deref(), &mut out);
    out
}

fn scan_dir_recursive(
    dir: &Path,
    scope: Scope,
    project_key: Option<&str>,
    out: &mut Vec<ImportableMemory>,
) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            scan_dir_recursive(&path, scope, project_key, out);
        } else if path.extension().and_then(|e| e.to_str()) == Some("md") {
            if let Ok(content) = std::fs::read_to_string(&path)
                && let Some(mem) =
                    parse_markdown_memory(&path, &content, scope, project_key.map(str::to_owned))
            {
                out.push(mem);
            }
        }
    }
}

/// Split YAML-ish frontmatter (between leading `---` fences) from the body.
/// Returns `(key/value pairs, body)`. When there's no frontmatter the whole
/// input is the body.
fn split_frontmatter(content: &str) -> (Vec<(String, String)>, &str) {
    let rest = match content.strip_prefix("---\n") {
        Some(r) => r,
        // Tolerate a leading BOM/whitespace-free `---\r\n` too.
        None => match content.strip_prefix("---\r\n") {
            Some(r) => r,
            None => return (Vec::new(), content),
        },
    };
    // Find the closing fence at the start of a line.
    let Some(end) = find_closing_fence(rest) else {
        return (Vec::new(), content);
    };
    let (fm_block, after) = rest.split_at(end);
    let mut pairs = Vec::new();
    for line in fm_block.lines() {
        if let Some((k, v)) = line.split_once(':') {
            let k = k.trim().to_ascii_lowercase();
            let v = v.trim().trim_matches('"').trim_matches('\'').to_owned();
            if !k.is_empty() {
                pairs.push((k, v));
            }
        }
    }
    // Skip the closing fence line itself.
    let body = after
        .trim_start_matches("---\n")
        .trim_start_matches("---\r\n")
        .trim_start_matches("---");
    (pairs, body)
}

fn find_closing_fence(s: &str) -> Option<usize> {
    let mut offset = 0usize;
    for line in s.split_inclusive('\n') {
        let trimmed = line.trim_end_matches(['\n', '\r']);
        if trimmed == "---" {
            return Some(offset);
        }
        offset += line.len();
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_markdown_memory_extracts_kind_and_title_normal() {
        let content =
            "---\ntype: feedback\nscope: user\n---\nUse ripgrep, not grep.\n\nWhy: faster.";
        let mem = parse_markdown_memory(Path::new("/m/x.md"), content, Scope::User, None)
            .expect("parses");
        assert_eq!(mem.kind, Kind::Finding); // feedback → finding
        assert_eq!(mem.scope, Scope::User);
        assert_eq!(mem.title, "Use ripgrep, not grep.");
        assert!(mem.body.contains("Why: faster."));
        assert!(mem.project_key.is_none());
    }

    #[test]
    fn parse_markdown_memory_no_frontmatter_uses_body_robust() {
        let mem = parse_markdown_memory(
            Path::new("/m/note.md"),
            "just a plain note with no frontmatter",
            Scope::User,
            None,
        )
        .expect("parses");
        assert_eq!(mem.kind, Kind::Fact);
        assert_eq!(mem.title, "just a plain note with no frontmatter");
    }

    #[test]
    fn parse_markdown_memory_empty_body_is_skipped_robust() {
        assert!(
            parse_markdown_memory(
                Path::new("/m/e.md"),
                "---\ntype: project\n---\n   \n",
                Scope::User,
                None
            )
            .is_none()
        );
    }

    #[test]
    fn project_scope_keeps_project_key_normal() {
        let mem = parse_markdown_memory(
            Path::new("/m/p.md"),
            "decision: use vite",
            Scope::Project,
            Some("projX".into()),
        )
        .expect("parses");
        assert_eq!(mem.scope, Scope::Project);
        assert_eq!(mem.project_key.as_deref(), Some("projX"));
    }

    #[test]
    fn deterministic_id_is_stable_and_content_sensitive_normal() {
        let a = ImportableMemory {
            source_path: None,
            kind: Kind::Fact,
            scope: Scope::User,
            project_key: None,
            title: "t".into(),
            body: "uses   edition 2024".into(),
        };
        // Whitespace-only reformatting of the body must NOT change the id.
        let mut b = a.clone();
        b.body = "uses edition 2024".into();
        assert_eq!(deterministic_id(&a), deterministic_id(&b));

        // Different content → different id.
        let mut c = a.clone();
        c.body = "uses edition 2021".into();
        assert_ne!(deterministic_id(&a), deterministic_id(&c));
    }

    #[test]
    fn scan_markdown_dir_walks_recursively_normal() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("a.md"),
            "---\ntype: project\n---\nalpha fact",
        )
        .unwrap();
        let sub = dir.path().join("sub");
        std::fs::create_dir(&sub).unwrap();
        std::fs::write(sub.join("b.md"), "beta fact").unwrap();
        std::fs::write(dir.path().join("ignore.txt"), "not markdown").unwrap();

        let mems = scan_markdown_dir(dir.path(), Scope::Project, Some("P".into()));
        assert_eq!(
            mems.len(),
            2,
            "should find both .md files, skip .txt: {mems:?}"
        );
        assert!(mems.iter().all(|m| m.project_key.as_deref() == Some("P")));
    }
}
