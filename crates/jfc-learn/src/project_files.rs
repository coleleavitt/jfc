//! Project files as agent context.
//!
//! Mirrors Perplexity Computer's project/space reference-file surface found in
//! the 2026-06-11 mindemon dump (`/rest/collections/{uuid}/project_files`:
//! tree / content / upload, "Add reference docs, data, or files that Computer
//! should use as context"). A project registers a set of on-disk reference
//! files that the agent loads as context for every turn in that project.
//!
//! This is deliberately a thin, deterministic registry — it owns *which files
//! are reference context* and *how to assemble them into a bounded context
//! block*. It does not embed, chunk, or rank; it reads the registered files in
//! order and concatenates them under a budget. Embedding/RAG is a separate
//! concern that can layer on top.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Default per-project context budget in bytes (~16k tokens of plain text).
pub const DEFAULT_CONTEXT_BUDGET_BYTES: usize = 64 * 1024;

/// A single registered reference file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectFile {
    /// Path to the file (absolute, or relative to the project root).
    pub path: PathBuf,
    /// Optional human label shown in the assembled context header.
    pub label: Option<String>,
}

impl ProjectFile {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            label: None,
        }
    }

    pub fn with_label(mut self, label: impl Into<String>) -> Self {
        self.label = Some(label.into());
        self
    }

    fn display_label(&self) -> String {
        self.label
            .clone()
            .unwrap_or_else(|| self.path.display().to_string())
    }
}

/// The registry of reference files for one project/space. Order is preserved
/// (insertion order) so the assembled context is stable; duplicate paths are
/// ignored.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ProjectFileSet {
    files: Vec<ProjectFile>,
    /// Byte budget for the assembled context block.
    #[serde(default = "default_budget")]
    budget_bytes: usize,
}

fn default_budget() -> usize {
    DEFAULT_CONTEXT_BUDGET_BYTES
}

impl ProjectFileSet {
    pub fn new() -> Self {
        Self {
            files: Vec::new(),
            budget_bytes: DEFAULT_CONTEXT_BUDGET_BYTES,
        }
    }

    pub fn with_budget(mut self, bytes: usize) -> Self {
        self.budget_bytes = bytes;
        self
    }

    /// Register a reference file. Returns `false` if the path was already
    /// registered (no duplicate added).
    pub fn add(&mut self, file: ProjectFile) -> bool {
        if self.files.iter().any(|f| f.path == file.path) {
            return false;
        }
        self.files.push(file);
        true
    }

    /// Remove a registered file by path. Returns `true` if it was present.
    pub fn remove(&mut self, path: &Path) -> bool {
        let before = self.files.len();
        self.files.retain(|f| f.path != path);
        self.files.len() != before
    }

    pub fn contains(&self, path: &Path) -> bool {
        self.files.iter().any(|f| f.path == path)
    }

    /// The registered files, in order (the "tree").
    pub fn list(&self) -> &[ProjectFile] {
        &self.files
    }

    pub fn len(&self) -> usize {
        self.files.len()
    }

    pub fn is_empty(&self) -> bool {
        self.files.is_empty()
    }

    /// Read a single registered file's content (relative paths resolved against
    /// `root`).
    pub fn read(&self, root: &Path, path: &Path) -> std::io::Result<String> {
        if !self.contains(path) {
            return Err(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "file not registered in project",
            ));
        }
        std::fs::read_to_string(resolve(root, path))
    }

    /// Assemble the reference files into a single bounded context block, in
    /// registration order. Each file is wrapped in a labelled fenced section.
    /// Files are included whole until the byte budget is reached; the first
    /// file that would overflow is truncated with a marker, and subsequent
    /// files are listed as skipped. Missing/unreadable files are noted inline
    /// rather than aborting.
    pub fn assemble_context(&self, root: &Path) -> ProjectContext {
        let mut out = String::new();
        let mut included = 0usize;
        let mut skipped = Vec::new();
        let mut truncated = false;

        for file in &self.files {
            if out.len() >= self.budget_bytes {
                skipped.push(file.display_label());
                continue;
            }
            let header = format!("\n=== {} ===\n", file.display_label());
            let content = match std::fs::read_to_string(resolve(root, &file.path)) {
                Ok(c) => c,
                Err(e) => format!("(could not read: {e})\n"),
            };
            let remaining = self.budget_bytes.saturating_sub(out.len() + header.len());
            out.push_str(&header);
            if content.len() <= remaining {
                out.push_str(&content);
            } else {
                let end = floor_char_boundary(&content, remaining);
                out.push_str(&content[..end]);
                out.push_str("\n…(truncated)\n");
                truncated = true;
            }
            included += 1;
        }

        ProjectContext {
            body: out,
            included,
            skipped,
            truncated,
        }
    }

    /// Distinct registered paths (used by callers that want set semantics).
    pub fn paths(&self) -> BTreeSet<&Path> {
        self.files.iter().map(|f| f.path.as_path()).collect()
    }
}

/// The assembled project context block plus metadata about what was included.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectContext {
    /// The concatenated, budget-bounded context text.
    pub body: String,
    /// Number of files contributed (whole or truncated).
    pub included: usize,
    /// Labels of files skipped because the budget was exhausted.
    pub skipped: Vec<String>,
    /// Whether any file was truncated to fit the budget.
    pub truncated: bool,
}

impl ProjectContext {
    pub fn is_empty(&self) -> bool {
        self.body.is_empty()
    }

    /// Wrap the body in a `<project-files>` block for prompt injection, or an
    /// empty string when there's nothing to inject.
    pub fn to_prompt_block(&self) -> String {
        if self.body.trim().is_empty() {
            return String::new();
        }
        format!("<project-files>{}\n</project-files>", self.body)
    }
}

fn resolve(root: &Path, path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        root.join(path)
    }
}

/// Largest char-boundary index `<= max` in `s` (std's is nightly-only).
fn floor_char_boundary(s: &str, max: usize) -> usize {
    if max >= s.len() {
        return s.len();
    }
    let mut i = max;
    while i > 0 && !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(dir: &Path, name: &str, content: &str) -> PathBuf {
        let p = dir.join(name);
        std::fs::write(&p, content).unwrap();
        p
    }

    // ── Registry CRUD ──────────────────────────────────────────────────────────

    #[test]
    fn add_list_remove_normal() {
        let mut set = ProjectFileSet::new();
        assert!(set.add(ProjectFile::new("a.md")));
        assert!(set.add(ProjectFile::new("b.md").with_label("Spec")));
        assert_eq!(set.len(), 2);
        assert!(set.contains(Path::new("a.md")));
        // Duplicate path rejected.
        assert!(!set.add(ProjectFile::new("a.md")));
        assert_eq!(set.len(), 2);
        // Remove.
        assert!(set.remove(Path::new("a.md")));
        assert!(!set.remove(Path::new("a.md")));
        assert_eq!(set.len(), 1);
        assert_eq!(set.list()[0].display_label(), "Spec");
    }

    // ── Read ───────────────────────────────────────────────────────────────────

    #[test]
    fn read_registered_file_normal() {
        let dir = tempfile::TempDir::new().unwrap();
        write(dir.path(), "notes.md", "hello world");
        let mut set = ProjectFileSet::new();
        set.add(ProjectFile::new("notes.md"));
        assert_eq!(
            set.read(dir.path(), Path::new("notes.md")).unwrap(),
            "hello world"
        );
    }

    #[test]
    fn read_unregistered_is_error_robust() {
        let dir = tempfile::TempDir::new().unwrap();
        write(dir.path(), "secret.md", "x");
        let set = ProjectFileSet::new();
        assert!(set.read(dir.path(), Path::new("secret.md")).is_err());
    }

    // ── Context assembly ───────────────────────────────────────────────────────

    #[test]
    fn assemble_context_concatenates_in_order_normal() {
        let dir = tempfile::TempDir::new().unwrap();
        write(dir.path(), "a.md", "AAA");
        write(dir.path(), "b.md", "BBB");
        let mut set = ProjectFileSet::new();
        set.add(ProjectFile::new("a.md").with_label("First"));
        set.add(ProjectFile::new("b.md").with_label("Second"));

        let ctx = set.assemble_context(dir.path());
        assert_eq!(ctx.included, 2);
        assert!(!ctx.truncated);
        assert!(ctx.skipped.is_empty());
        let first = ctx.body.find("First").unwrap();
        let second = ctx.body.find("Second").unwrap();
        assert!(first < second, "files must keep registration order");
        assert!(ctx.body.contains("AAA"));
        assert!(ctx.body.contains("BBB"));
        assert!(ctx.to_prompt_block().starts_with("<project-files>"));
    }

    #[test]
    fn assemble_context_truncates_and_skips_under_budget_robust() {
        let dir = tempfile::TempDir::new().unwrap();
        write(dir.path(), "big.md", &"x".repeat(1000));
        write(dir.path(), "next.md", "should be skipped");
        let mut set = ProjectFileSet::new().with_budget(120);
        set.add(ProjectFile::new("big.md"));
        set.add(ProjectFile::new("next.md"));

        let ctx = set.assemble_context(dir.path());
        assert!(ctx.truncated, "big file should be truncated");
        assert!(ctx.body.contains("…(truncated)"));
        // Budget exhausted → second file skipped.
        assert_eq!(ctx.skipped.len(), 1);
        assert!(ctx.body.len() <= 200);
    }

    #[test]
    fn assemble_context_notes_missing_file_robust() {
        let dir = tempfile::TempDir::new().unwrap();
        let mut set = ProjectFileSet::new();
        set.add(ProjectFile::new("gone.md"));
        let ctx = set.assemble_context(dir.path());
        // Included (with an inline error note), not aborted.
        assert_eq!(ctx.included, 1);
        assert!(ctx.body.contains("could not read"));
    }

    #[test]
    fn empty_set_yields_empty_prompt_block_normal() {
        let dir = tempfile::TempDir::new().unwrap();
        let set = ProjectFileSet::new();
        let ctx = set.assemble_context(dir.path());
        assert!(ctx.is_empty());
        assert_eq!(ctx.to_prompt_block(), "");
    }

    #[test]
    fn set_roundtrips_serde_robust() {
        let mut set = ProjectFileSet::new().with_budget(123);
        set.add(ProjectFile::new("a.md").with_label("A"));
        let json = serde_json::to_string(&set).unwrap();
        let back: ProjectFileSet = serde_json::from_str(&json).unwrap();
        assert_eq!(back.len(), 1);
        assert_eq!(back.budget_bytes, 123);
    }
}
