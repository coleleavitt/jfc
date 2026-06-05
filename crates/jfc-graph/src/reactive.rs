//! Demand-driven reactive query framework (Salsa-lite).
//!
//! Tracks fine-grained dependencies between query inputs and outputs.
//! Only recomputes queries whose exact input sub-trees changed.
//!
//! ## Relationship to [`crate::incremental`]
//!
//! [`QueryCache`](crate::incremental::QueryCache) takes a coarse-grained
//! approach: when a file changes we bump the global graph revision and
//! every entry whose read-set touches that file is dropped. That's
//! correct but pessimistic — a one-line comment edit blows away every
//! query that ever looked at the file.
//!
//! This module is the **foundation** for a finer-grained alternative
//! inspired by [Salsa](https://github.com/salsa-rs/salsa):
//!
//! * Every observable thing (file content, derived value) is an
//!   **input** identified by a stable [`InputId`].
//! * Each input has a [`Revision`] — the global tick at which it was
//!   last *semantically* changed. Comment-only / whitespace-only edits
//!   leave the revision untouched, because [`ReactiveDb::semantic_hash`]
//!   strips them before hashing.
//! * Each cached query result ([`QueryMemo`]) records the inputs it
//!   read and the revision at which it was last verified. Validity is
//!   a simple comparison: *all* deps must have `revision <= verified_at`.
//!
//! This module deliberately does **not** integrate with the existing
//! `QueryCache` yet. It's a parallel implementation that downstream
//! callers can swap in once the protocol is exercised end-to-end.

use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::path::Path;

/// A unique identifier for a query input (file content, node data, edge set).
///
/// Inputs are content-addressed by hashing a stable key (e.g. an absolute
/// file path). Callers compute the id once and reuse it across mutations.
pub type InputId = u64;

/// A revision counter that monotonically increases on each mutation.
///
/// Revision `0` is reserved to mean "before any mutation"; the first
/// [`InputRevisions::bump`] call returns `1`.
pub type Revision = u64;

/// Tracks the revision at which each input was last changed.
///
/// Acts as the single source of truth for "did this input change since
/// the memo was verified?" Lookups for unknown inputs return revision
/// `0`, so freshly-recorded dependencies on never-mutated inputs do not
/// spuriously invalidate memos.
#[derive(Debug, Clone, Default)]
pub struct InputRevisions {
    revisions: HashMap<InputId, Revision>,
    current: Revision,
}

impl InputRevisions {
    /// Construct an empty revision tracker. `current` starts at 0.
    pub fn new() -> Self {
        Self::default()
    }

    /// The current global revision counter.
    pub fn current(&self) -> Revision {
        self.current
    }

    /// Bump the global revision counter and return the new value.
    ///
    /// Call this once per atomic mutation batch; individual
    /// [`set_input`](Self::set_input) calls within the batch share the
    /// resulting revision so a single user action presents a consistent
    /// snapshot to queries.
    pub fn bump(&mut self) -> Revision {
        self.current += 1;
        self.current
    }

    /// Mark `id` as last-changed at `revision`.
    ///
    /// Typically passed `self.current()` after a [`bump`](Self::bump),
    /// but the explicit parameter lets callers replay change logs at
    /// arbitrary revisions during recovery.
    pub fn set_input(&mut self, id: InputId, revision: Revision) {
        self.revisions.insert(id, revision);
    }

    /// Revision at which `id` was last changed; `0` if never seen.
    pub fn revision_of(&self, id: InputId) -> Revision {
        self.revisions.get(&id).copied().unwrap_or(0)
    }
}

/// A cached query result with its dependency set.
///
/// The memo is considered valid as long as every input in
/// `dependencies` still reports a revision `<= verified_at`. Once any
/// dependency advances past that watermark the memo must be recomputed.
#[derive(Debug, Clone)]
pub struct QueryMemo<V> {
    /// The cached output value.
    pub value: V,
    /// Revision at which this memo was last confirmed correct.
    pub verified_at: Revision,
    /// Inputs the query observed while producing `value`.
    pub dependencies: Vec<InputId>,
}

impl<V> QueryMemo<V> {
    /// Build a fresh memo verified at `verified_at`.
    pub fn new(value: V, verified_at: Revision, dependencies: Vec<InputId>) -> Self {
        Self {
            value,
            verified_at,
            dependencies,
        }
    }

    /// `true` iff every recorded dependency has a revision `<= verified_at`.
    ///
    /// Inputs never registered with the tracker count as revision 0, so
    /// dependencies on stable inputs cannot invalidate the memo.
    pub fn is_valid(&self, inputs: &InputRevisions) -> bool {
        self.dependencies
            .iter()
            .all(|dep| inputs.revision_of(*dep) <= self.verified_at)
    }
}

/// The reactive database.
///
/// Owns the [`InputRevisions`] table; memos are held externally by
/// callers (typically in a type-erased map keyed by query identity) so
/// this struct stays free of generic parameters.
#[derive(Debug, Default)]
pub struct ReactiveDb {
    inputs: InputRevisions,
}

impl ReactiveDb {
    /// Construct an empty database.
    pub fn new() -> Self {
        Self::default()
    }

    /// Immutable access to the revision table for memo validity checks.
    pub fn inputs(&self) -> &InputRevisions {
        &self.inputs
    }

    /// Mutable access — primarily for tests and bulk replay.
    pub fn inputs_mut(&mut self) -> &mut InputRevisions {
        &mut self.inputs
    }

    /// Compute the stable [`InputId`] for a file path.
    ///
    /// Uses the path string verbatim — callers should canonicalise
    /// before calling if they want symlink-insensitive identity.
    pub fn input_id_for_path(file_path: &Path) -> InputId {
        let mut h = std::collections::hash_map::DefaultHasher::new();
        file_path.hash(&mut h);
        h.finish()
    }

    /// Record that `file_path` changed. Bumps the global revision and
    /// stamps the file's input id with the new value.
    ///
    /// Callers who want to skip no-op edits should hash content with
    /// [`Self::semantic_hash`] first and only invoke this when the hash
    /// actually differs from the previous one.
    pub fn file_changed(&mut self, file_path: &Path) {
        let id = Self::input_id_for_path(file_path);
        let rev = self.inputs.bump();
        self.inputs.set_input(id, rev);
    }

    /// Hash only the semantic content of `content` (comments + extra
    /// whitespace stripped).
    ///
    /// Used to decide whether a file edit is observable: if the
    /// semantic hash matches the previous one, the edit was a comment
    /// or formatting tweak and queries that touched the file remain
    /// valid.
    pub fn semantic_hash(content: &str) -> u64 {
        let stripped = strip_for_hash(content);
        let mut h = std::collections::hash_map::DefaultHasher::new();
        stripped.hash(&mut h);
        h.finish()
    }
}

/// Strip single-line comments (`//`, `#`, `--`), multi-line `/* … */`
/// comments, and collapse runs of whitespace to a single space.
///
/// This is intentionally simple — it doesn't try to be aware of string
/// literals, so `"// not a comment"` will be mangled. That's acceptable
/// for the hash use-case because we only care about *change detection*:
/// the same input produces the same stripped output deterministically.
fn strip_for_hash(content: &str) -> String {
    let bytes = content.as_bytes();
    let mut out = String::with_capacity(content.len());
    let mut i = 0;

    // Push a space iff the buffer doesn't already end in one.
    // Comments and whitespace runs both fold into a single space, so a
    // sequence like "}  // comment\n  foo" collapses to "} foo".
    let push_space = |out: &mut String| {
        if !out.ends_with(' ') {
            out.push(' ');
        }
    };

    while i < bytes.len() {
        let b = bytes[i];

        // Block comment /* ... */
        if b == b'/' && i + 1 < bytes.len() && bytes[i + 1] == b'*' {
            i += 2;
            while i + 1 < bytes.len() && !(bytes[i] == b'*' && bytes[i + 1] == b'/') {
                i += 1;
            }
            // Skip closing */ if present
            i = (i + 2).min(bytes.len());
            push_space(&mut out);
            continue;
        }

        // Line comments: // …, -- …, # …
        // The latter two are language-specific but the hash only cares
        // about deterministic stripping, not about being syntactically
        // honest in every language.
        let is_slash_line = b == b'/' && i + 1 < bytes.len() && bytes[i + 1] == b'/';
        let is_dash_line = b == b'-' && i + 1 < bytes.len() && bytes[i + 1] == b'-';
        let is_hash_line = b == b'#';
        if is_slash_line || is_dash_line || is_hash_line {
            while i < bytes.len() && bytes[i] != b'\n' {
                i += 1;
            }
            push_space(&mut out);
            continue;
        }

        // Collapse whitespace
        if b.is_ascii_whitespace() {
            push_space(&mut out);
            i += 1;
            continue;
        }

        // Preserve byte. ASCII bytes map directly to chars; for
        // multi-byte UTF-8 we decode one codepoint so we don't split it.
        if b < 128 {
            out.push(b as char);
            i += 1;
        } else {
            let rest = &content[i..];
            if let Some(ch) = rest.chars().next() {
                out.push(ch);
                i += ch.len_utf8();
            } else {
                i += 1;
            }
        }
    }
    // Trim leading/trailing space for stability across inputs that
    // start or end with comments / whitespace.
    out.trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn semantic_hash_ignores_line_comments() {
        let a = "fn main() { let x = 1; }";
        let b = "fn main() { // hello\n let x = 1; }";
        let c = "fn main() { let x = 1; } // trailing";
        assert_eq!(ReactiveDb::semantic_hash(a), ReactiveDb::semantic_hash(b));
        assert_eq!(ReactiveDb::semantic_hash(a), ReactiveDb::semantic_hash(c));
    }

    #[test]
    fn semantic_hash_ignores_block_comments() {
        let a = "fn main() { let x = 1; }";
        let b = "fn main() { /* hi */ let x = 1; }";
        let c = "fn main() { let /* mid */ x = 1; }";
        assert_eq!(ReactiveDb::semantic_hash(a), ReactiveDb::semantic_hash(b));
        assert_eq!(ReactiveDb::semantic_hash(a), ReactiveDb::semantic_hash(c));
    }

    #[test]
    fn semantic_hash_ignores_whitespace_changes() {
        // Same tokens, same separators — only the *amount* of whitespace
        // differs. The stripper collapses runs to a single space, so
        // both inputs hash identically.
        let a = "fn main ( ) { let x = 1 ; }";
        let b = "fn   main (\t) {\n\n  let  x  =  1 ;   }";
        assert_eq!(ReactiveDb::semantic_hash(a), ReactiveDb::semantic_hash(b));
    }

    #[test]
    fn semantic_hash_detects_real_changes() {
        let a = "fn main() { let x = 1; }";
        let b = "fn main() { let x = 2; }";
        assert_ne!(ReactiveDb::semantic_hash(a), ReactiveDb::semantic_hash(b));
    }

    #[test]
    fn semantic_hash_ignores_hash_and_dash_comments() {
        // Python-ish
        let a = "x = 1\ny = 2";
        let b = "x = 1 # set x\ny = 2 # set y";
        assert_eq!(ReactiveDb::semantic_hash(a), ReactiveDb::semantic_hash(b));
        // SQL-ish
        let c = "SELECT 1;";
        let d = "SELECT 1; -- comment";
        assert_eq!(ReactiveDb::semantic_hash(c), ReactiveDb::semantic_hash(d));
    }

    #[test]
    fn memo_is_valid_when_deps_unchanged() {
        let mut inputs = InputRevisions::new();
        let r1 = inputs.bump();
        inputs.set_input(10, r1);
        inputs.set_input(20, r1);

        let memo = QueryMemo::new("result", r1, vec![10, 20]);
        assert!(memo.is_valid(&inputs));
    }

    #[test]
    fn memo_is_valid_when_dep_never_changed() {
        // Dependency on input never registered should still be valid:
        // revision_of unknown inputs is 0, which is <= verified_at.
        let mut inputs = InputRevisions::new();
        let r1 = inputs.bump();
        let memo = QueryMemo::new("result", r1, vec![999]);
        assert!(memo.is_valid(&inputs));
    }

    #[test]
    fn memo_is_invalid_when_dep_changed() {
        let mut inputs = InputRevisions::new();
        let r1 = inputs.bump();
        inputs.set_input(10, r1);
        inputs.set_input(20, r1);

        let memo = QueryMemo::new("result", r1, vec![10, 20]);
        assert!(memo.is_valid(&inputs));

        // Mutate input 10 at a later revision.
        let r2 = inputs.bump();
        inputs.set_input(10, r2);

        assert!(!memo.is_valid(&inputs));
    }

    #[test]
    fn memo_is_invalid_only_for_changed_dep() {
        let mut inputs = InputRevisions::new();
        let r1 = inputs.bump();
        inputs.set_input(10, r1);
        inputs.set_input(20, r1);

        let memo_a = QueryMemo::new("a", r1, vec![10]);
        let memo_b = QueryMemo::new("b", r1, vec![20]);

        let r2 = inputs.bump();
        inputs.set_input(10, r2);

        assert!(!memo_a.is_valid(&inputs));
        assert!(memo_b.is_valid(&inputs)); // independent dep — still good
    }

    #[test]
    fn file_changed_bumps_revision_for_that_path() {
        let mut db = ReactiveDb::new();
        let path = Path::new("/tmp/foo.rs");
        let id = ReactiveDb::input_id_for_path(path);

        assert_eq!(db.inputs().revision_of(id), 0);

        db.file_changed(path);
        let r1 = db.inputs().current();
        assert_eq!(db.inputs().revision_of(id), r1);

        // Unrelated input untouched.
        let other = ReactiveDb::input_id_for_path(Path::new("/tmp/bar.rs"));
        assert_eq!(db.inputs().revision_of(other), 0);
    }

    #[test]
    fn input_id_is_stable_per_path() {
        let p = Path::new("/a/b/c.rs");
        assert_eq!(
            ReactiveDb::input_id_for_path(p),
            ReactiveDb::input_id_for_path(p)
        );
        assert_ne!(
            ReactiveDb::input_id_for_path(p),
            ReactiveDb::input_id_for_path(Path::new("/a/b/d.rs"))
        );
    }

    #[test]
    fn bump_returns_monotonic_revisions() {
        let mut inputs = InputRevisions::new();
        let a = inputs.bump();
        let b = inputs.bump();
        let c = inputs.bump();
        assert!(a < b && b < c);
    }
}
