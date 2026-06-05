//! `@filename` autocomplete for the input bar.
//!
//! When the user types `@` followed by a partial path, the input pane
//! shows a popup of matching files (relative to the project's `cwd`).
//! Selecting one rewrites `@partial` to the full relative path so the
//! model receives the canonical reference. Mirrors v126's autocomplete
//! plumbing (cli.js:161602 — `autocomplete:accept` / `autocomplete:dismiss`).
//!
//! ## Activation
//!
//! - `@` at the start of a line OR preceded by whitespace → enters mention
//!   mode.
//! - Subsequent typed chars extend the query; matching candidates are
//!   re-ranked.
//! - `Esc` exits without inserting; `Enter` / `Tab` accepts the selected
//!   candidate; `↑`/`↓` cycles.
//!
//! ## Pure-data design
//!
//! All scanning/filtering lives here. The input handler calls into
//! `MentionState::update_query` and `MentionState::accept`; the renderer
//! reads `state.candidates` and `state.selected`. No I/O outside of
//! `scan_files()` (called once per activation).

use std::path::Path;

#[derive(Clone, Debug, Default)]
pub struct MentionState {
    pub active: bool,
    /// Byte offset in the input buffer where the `@` lives. Used by the
    /// accept handler to know what range to replace.
    pub anchor_byte: usize,
    pub query: String,
    pub candidates: Vec<String>,
    pub selected: usize,
}

impl MentionState {
    pub fn activate(&mut self, anchor_byte: usize, candidates: Vec<String>) {
        self.active = true;
        self.anchor_byte = anchor_byte;
        self.query = String::new();
        self.candidates = candidates;
        self.selected = 0;
    }

    pub fn dismiss(&mut self) {
        self.active = false;
        self.query.clear();
        self.candidates.clear();
        self.selected = 0;
    }

    pub fn move_selection(&mut self, delta: i32) {
        if self.candidates.is_empty() {
            return;
        }
        let len = self.candidates.len() as i32;
        let cur = self.selected as i32;
        let next = ((cur + delta) % len + len) % len;
        self.selected = next as usize;
    }

    pub fn update_query(&mut self, query: String, all: &[String]) {
        self.query = query;
        self.candidates = filter_candidates(all, &self.query);
        self.selected = 0;
    }

    pub fn accepted(&self) -> Option<&str> {
        self.candidates.get(self.selected).map(String::as_str)
    }
}

/// Decide whether typing `@` at the given (post-keystroke) cursor
/// position should activate mention mode. Returns the byte offset of the
/// `@` if activation should fire, else `None`.
///
/// Activation rule (matches v126's at-mention guard): `@` is preceded by
/// nothing (start of line) or by whitespace. We don't activate on
/// `email@host.com` because the `@` there is mid-token.
pub fn should_activate(line_so_far: &str) -> Option<usize> {
    if !line_so_far.ends_with('@') {
        return None;
    }
    let anchor = line_so_far.len() - '@'.len_utf8();
    if anchor == 0 {
        return Some(anchor);
    }
    // Look at the char immediately before the `@`.
    let prev = line_so_far[..anchor].chars().next_back();
    match prev {
        Some(c) if c.is_whitespace() => Some(anchor),
        _ => None,
    }
}

/// Walk `cwd` collecting up to `cap` candidate paths (relative). Skips
/// `.git`, `target`, `node_modules`, and dotfiles. Sorted lexicographically
/// for stable ordering. Pulled out as its own function so the test suite
/// can exercise it against a temp dir without driving the full input loop.
pub fn scan_files(cwd: &Path, cap: usize) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    walk(cwd, cwd, &mut out, cap);
    out.sort();
    out.truncate(cap);
    out
}

fn walk(root: &Path, dir: &Path, out: &mut Vec<String>, cap: usize) {
    if out.len() >= cap {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        if out.len() >= cap {
            return;
        }
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        // Skip hidden + heavyweight build dirs. v126 also filters these
        // (and reads .gitignore for finer control — we punt on that).
        if name_str.starts_with('.')
            || name_str == "target"
            || name_str == "node_modules"
            || name_str == "dist"
            || name_str == "build"
        {
            continue;
        }
        let path = entry.path();
        if path.is_dir() {
            walk(root, &path, out, cap);
            continue;
        }
        if let Ok(rel) = path.strip_prefix(root) {
            out.push(rel.to_string_lossy().into_owned());
        }
    }
}

/// Substring + prefix match. Prefix matches outrank substring; ties broken
/// alphabetically. Mirrors fuzzy-finder UX where what you typed first
/// surfaces first.
pub fn filter_candidates(all: &[String], query: &str) -> Vec<String> {
    if query.is_empty() {
        return all.iter().take(20).cloned().collect();
    }
    let q = query.to_lowercase();
    let mut prefix: Vec<&String> = Vec::new();
    let mut substring: Vec<&String> = Vec::new();
    for c in all {
        let lower = c.to_lowercase();
        // Match against the full path AND the basename. v126 surfaces
        // `Cargo.toml` when you type `@cargo` even though the full
        // string is `crates/jfc-ui/Cargo.toml`.
        let basename = Path::new(c)
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or(c)
            .to_lowercase();
        if basename.starts_with(&q) || lower.starts_with(&q) {
            prefix.push(c);
        } else if basename.contains(&q) || lower.contains(&q) {
            substring.push(c);
        }
    }
    prefix.sort();
    substring.sort();
    prefix
        .into_iter()
        .chain(substring)
        .take(20)
        .cloned()
        .collect()
}

/// Build the replacement that should slot into the input buffer when the
/// user accepts a candidate. Returns `(new_buffer, new_cursor_byte)`.
/// The `@` and any partial query are removed; the candidate is inserted
/// with a trailing space so the user can keep typing.
pub fn apply_acceptance(
    buffer: &str,
    anchor_byte: usize,
    query_len: usize,
    pick: &str,
) -> (String, usize) {
    // Anchor points to the `@`; we replace `@<query>` with `pick `.
    let end = anchor_byte + '@'.len_utf8() + query_len;
    let end = end.min(buffer.len());
    let mut out = String::with_capacity(buffer.len() + pick.len());
    out.push_str(&buffer[..anchor_byte]);
    out.push_str(pick);
    out.push(' ');
    let new_cursor = out.len();
    out.push_str(&buffer[end..]);
    (out, new_cursor)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn ss(strs: &[&str]) -> Vec<String> {
        strs.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn activates_at_start_of_line_normal() {
        assert_eq!(should_activate("@"), Some(0));
    }

    #[test]
    fn activates_after_whitespace_normal() {
        assert_eq!(should_activate("hello @"), Some(6));
        assert_eq!(should_activate("foo\n@"), Some(4));
        assert_eq!(should_activate("foo\t@"), Some(4));
    }

    #[test]
    fn does_not_activate_in_email_robust() {
        // `email@host` mid-token — must not pop the file picker.
        assert_eq!(should_activate("user@"), None);
        assert_eq!(should_activate("see foo@bar"), None);
    }

    #[test]
    fn does_not_activate_without_at_robust() {
        assert_eq!(should_activate("hello"), None);
        assert_eq!(should_activate(""), None);
    }

    #[test]
    fn filter_empty_query_returns_first_n_normal() {
        let all = ss(&["a.rs", "b.rs", "c.rs"]);
        let r = filter_candidates(&all, "");
        assert_eq!(r, all);
    }

    #[test]
    fn filter_prefix_outranks_substring_normal() {
        let all = ss(&["zebra/main.rs", "main.rs", "src/lib.rs"]);
        let r = filter_candidates(&all, "main");
        // basename "main.rs" should be first (basename prefix match).
        assert_eq!(r[0], "main.rs");
        // "zebra/main.rs" — basename starts with "main" too, so it's
        // also a prefix match. Both prefix matches sort alphabetically:
        // "main.rs" then "zebra/main.rs".
        assert_eq!(r[1], "zebra/main.rs");
    }

    #[test]
    fn filter_basename_match_robust() {
        // Typing `@cargo` should surface `Cargo.toml` even when the path
        // is `crates/jfc-ui/Cargo.toml`.
        let all = ss(&["crates/jfc-ui/Cargo.toml", "src/main.rs", "Cargo.lock"]);
        let r = filter_candidates(&all, "cargo");
        assert!(
            r.iter().any(|s| s == "Cargo.lock"),
            "case-insensitive match should surface Cargo.lock; got: {r:?}"
        );
    }

    #[test]
    fn filter_case_insensitive_robust() {
        let all = ss(&["Cargo.toml", "main.rs"]);
        let r = filter_candidates(&all, "CARGO");
        assert_eq!(r[0], "Cargo.toml");
    }

    #[test]
    fn move_selection_wraps_normal() {
        let mut s = MentionState::default();
        s.activate(0, ss(&["a", "b", "c"]));
        s.move_selection(1);
        assert_eq!(s.selected, 1);
        s.move_selection(1);
        assert_eq!(s.selected, 2);
        s.move_selection(1);
        assert_eq!(s.selected, 0, "wraps forward");
        s.move_selection(-1);
        assert_eq!(s.selected, 2, "wraps backward");
    }

    #[test]
    fn move_selection_empty_no_panic_robust() {
        let mut s = MentionState::default();
        s.move_selection(1);
        s.move_selection(-1);
        assert_eq!(s.selected, 0);
    }

    #[test]
    fn apply_acceptance_replaces_at_token_normal() {
        // Buffer:   "see @car|"  cursor after partial
        // Pick:     "Cargo.toml"
        // Expected: "see Cargo.toml |"  cursor after the inserted space
        let (out, cursor) = apply_acceptance("see @car", 4, 3, "Cargo.toml");
        assert_eq!(out, "see Cargo.toml ");
        assert_eq!(cursor, "see Cargo.toml ".len());
    }

    #[test]
    fn apply_acceptance_preserves_trailing_text_robust() {
        // User has typed past the @-token: `see @c after`. Accept should
        // still only replace the @-token, leaving `after` intact.
        let (out, cursor) = apply_acceptance("see @c after", 4, 1, "Cargo.toml");
        assert_eq!(out, "see Cargo.toml  after");
        assert_eq!(cursor, "see Cargo.toml ".len());
    }

    #[test]
    fn apply_acceptance_at_end_of_buffer_robust() {
        let (out, cursor) = apply_acceptance("@", 0, 0, "main.rs");
        assert_eq!(out, "main.rs ");
        assert_eq!(cursor, 8);
    }

    #[test]
    fn scan_files_walks_cwd_normal() {
        // Use a temp dir we own so the test is hermetic.
        let dir = tempdir_or_skip();
        let Some(dir) = dir else {
            return;
        };
        std::fs::write(dir.join("a.txt"), "x").unwrap();
        std::fs::write(dir.join("b.txt"), "x").unwrap();
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        std::fs::write(dir.join("sub/c.txt"), "x").unwrap();
        std::fs::create_dir_all(dir.join("target")).unwrap();
        std::fs::write(dir.join("target/skip.txt"), "x").unwrap();
        std::fs::create_dir_all(dir.join(".hidden")).unwrap();
        std::fs::write(dir.join(".hidden/skip.txt"), "x").unwrap();
        let found = scan_files(&dir, 100);
        assert!(
            found.iter().any(|s| s == "a.txt"),
            "missing a.txt: {found:?}"
        );
        assert!(
            found.iter().any(|s| s == "b.txt"),
            "missing b.txt: {found:?}"
        );
        // sub/c.txt should appear (recursion works)
        assert!(
            found.iter().any(|s| s.ends_with("c.txt")),
            "missing sub/c.txt: {found:?}"
        );
        // target/skip.txt should NOT (heavyweight dir filter)
        assert!(
            !found.iter().any(|s| s.contains("target")),
            "target/ should be filtered: {found:?}"
        );
        // hidden should NOT (dotfile filter)
        assert!(
            !found.iter().any(|s| s.contains(".hidden")),
            ".hidden should be filtered: {found:?}"
        );
    }

    /// Best-effort temp-dir helper — uses `std::env::temp_dir`. Returns
    /// `None` if creation fails so the test skips rather than fails on
    /// CI sandboxes without writable temp.
    fn tempdir_or_skip() -> Option<PathBuf> {
        let mut p = std::env::temp_dir();
        p.push(format!(
            "jfc_mention_test_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .ok()?
                .as_nanos()
        ));
        std::fs::create_dir_all(&p).ok()?;
        Some(p)
    }
}
