//! AI file-access exclusion via `.jfcignore` / `.claudeignore`.
//!
//! `.gitignore` is the wrong tool for "hide this from the agent": many files
//! a user wants AI-excluded (`.env`, credentials, proprietary docs, large
//! generated dumps) are deliberately git-*tracked*, and conversely build
//! artifacts that ARE gitignored are often fine for the agent to read. This
//! module adds a dedicated exclusion layer the agent honors on its read/search
//! surfaces, in gitignore syntax (negation, globs, directory rules all work).
//!
//! Precedence: `.jfcignore` (native) is read first, then `.claudeignore`
//! (Claude Code compatibility) is layered on top, so a project migrating from
//! Claude Code keeps its existing rules while jfc-native rules can extend them.
//!
//! **This is not a hard security boundary.** The agent can still run arbitrary
//! shell via Bash (`cat secret.env`), so a determined model — or a user prompt
//! — can bypass it. It is a "don't trip over secrets by accident" guard for the
//! structured file tools (Read / Glob / Grep), not a sandbox. Real isolation is
//! the sandbox + permission layers' job.

use std::path::{Path, PathBuf};

use ignore::gitignore::{Gitignore, GitignoreBuilder};

/// The ignore-file basenames we honor, in precedence order (earlier = lower
/// precedence; later rules can override earlier ones via gitignore semantics).
const IGNORE_FILENAMES: [&str; 2] = [".jfcignore", ".claudeignore"];

/// A compiled access policy for one root directory. Cheap to query; build it
/// once per tool batch (or per root) rather than per path.
#[derive(Debug, Clone)]
pub struct AccessPolicy {
    root: PathBuf,
    matcher: Gitignore,
    /// The on-disk ignore files that contributed rules, so callers can hand
    /// them to `rg --ignore-file=PATH` for native filtering of search output.
    ignore_files: Vec<PathBuf>,
}

impl AccessPolicy {
    /// Build the policy for `root` by reading any `.jfcignore` / `.claudeignore`
    /// found directly in it. Missing files are skipped silently; a policy with
    /// no rules blocks nothing (the common case — no overhead for projects that
    /// don't use the feature).
    pub fn for_root(root: &Path) -> Self {
        let mut builder = GitignoreBuilder::new(root);
        let mut ignore_files = Vec::new();
        for name in IGNORE_FILENAMES {
            let candidate = root.join(name);
            if candidate.is_file() {
                // `add` returns Some(err) on a malformed line; we log and keep
                // going so one bad rule doesn't disable the whole policy.
                if let Some(err) = builder.add(&candidate) {
                    tracing::warn!(
                        target: "jfc::access_policy",
                        file = %candidate.display(),
                        error = %err,
                        "ignore file had unparseable rules; continuing with the rest"
                    );
                }
                ignore_files.push(candidate);
            }
        }
        let matcher = builder.build().unwrap_or_else(|err| {
            tracing::warn!(
                target: "jfc::access_policy",
                error = %err,
                "failed to compile access policy; treating as empty (nothing blocked)"
            );
            Gitignore::empty()
        });
        Self {
            root: root.to_path_buf(),
            matcher,
            ignore_files,
        }
    }

    /// True when this policy has no ignore files (so every query is allowed).
    /// Lets hot paths skip work entirely.
    pub fn is_empty(&self) -> bool {
        self.ignore_files.is_empty()
    }

    /// The ignore-file paths that fed this policy, for `rg --ignore-file=PATH`.
    pub fn ignore_files(&self) -> &[PathBuf] {
        &self.ignore_files
    }

    /// Whether `path` is AI-blocked by this policy. Relative paths are resolved
    /// against the policy root the matcher was built with. A directory match
    /// blocks everything beneath it. Never errors — on any ambiguity it
    /// defaults to *allow* so the guard can't wedge legitimate reads.
    pub fn is_blocked(&self, path: &Path) -> bool {
        if self.ignore_files.is_empty() {
            return false;
        }
        // The matcher is rooted at `self.root`; a relative query (`secret.env`,
        // how the model usually names files) must be joined onto it so the
        // gitignore matcher sees a path under its root. Absolute paths pass
        // through unchanged.
        let resolved = if path.is_absolute() {
            path.to_path_buf()
        } else {
            self.root.join(path)
        };
        // `matched_path_or_any_parents` walks parent dirs so a rule like
        // `secrets/` blocks `secrets/key.pem` even when only the file is
        // queried. `is_dir` is best-effort metadata; a missing file is treated
        // as a non-dir, which still matches file rules correctly.
        let is_dir = resolved.is_dir();
        self.matcher
            .matched_path_or_any_parents(&resolved, is_dir)
            .is_ignore()
    }

    /// Standard refusal message for a blocked path — uniform across tools so
    /// the model learns the pattern and can tell the user why.
    pub fn refusal(path: &str) -> String {
        format!(
            "Access to `{path}` is blocked by an AI-access rule (.jfcignore / \
             .claudeignore). This file is intentionally hidden from the agent. \
             If you need it, remove the rule or ask the user to share the \
             relevant contents directly."
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn write(dir: &Path, name: &str, body: &str) {
        fs::write(dir.join(name), body).unwrap();
    }

    // Normal: a plain rule blocks the named file and allows others.
    #[test]
    fn jfcignore_blocks_named_file_normal() {
        let tmp = tempfile::tempdir().unwrap();
        write(tmp.path(), ".jfcignore", "secret.env\n");
        write(tmp.path(), "secret.env", "TOKEN=x");
        write(tmp.path(), "main.rs", "fn main() {}");

        let policy = AccessPolicy::for_root(tmp.path());
        assert!(policy.is_blocked(&tmp.path().join("secret.env")));
        assert!(!policy.is_blocked(&tmp.path().join("main.rs")));
    }

    // Normal: a directory rule blocks everything beneath it.
    #[test]
    fn directory_rule_blocks_children_normal() {
        let tmp = tempfile::tempdir().unwrap();
        write(tmp.path(), ".jfcignore", "secrets/\n");
        fs::create_dir(tmp.path().join("secrets")).unwrap();
        write(&tmp.path().join("secrets"), "key.pem", "----");

        let policy = AccessPolicy::for_root(tmp.path());
        assert!(policy.is_blocked(&tmp.path().join("secrets/key.pem")));
        assert!(policy.is_blocked(&tmp.path().join("secrets")));
    }

    // Robust: .claudeignore is honored for Claude Code compatibility.
    #[test]
    fn claudeignore_is_honored_robust() {
        let tmp = tempfile::tempdir().unwrap();
        write(tmp.path(), ".claudeignore", "*.key\n");
        write(tmp.path(), "id.key", "priv");

        let policy = AccessPolicy::for_root(tmp.path());
        assert!(policy.is_blocked(&tmp.path().join("id.key")));
    }

    // Robust: both files layer, and a negation in the later file re-allows.
    #[test]
    fn negation_reallows_robust() {
        let tmp = tempfile::tempdir().unwrap();
        // .jfcignore blocks all .env; .claudeignore (added after) re-allows one.
        write(tmp.path(), ".jfcignore", "*.env\n");
        write(tmp.path(), ".claudeignore", "!public.env\n");
        write(tmp.path(), "secret.env", "x");
        write(tmp.path(), "public.env", "y");

        let policy = AccessPolicy::for_root(tmp.path());
        assert!(policy.is_blocked(&tmp.path().join("secret.env")));
        assert!(
            !policy.is_blocked(&tmp.path().join("public.env")),
            "negation in .claudeignore should re-allow public.env"
        );
    }

    // Robust: no ignore files → nothing blocked, is_empty() true (fast path).
    #[test]
    fn no_ignore_files_blocks_nothing_robust() {
        let tmp = tempfile::tempdir().unwrap();
        write(tmp.path(), "anything.txt", "x");
        let policy = AccessPolicy::for_root(tmp.path());
        assert!(policy.is_empty());
        assert!(!policy.is_blocked(&tmp.path().join("anything.txt")));
    }

    // Robust: ignore_files() reports the contributing files for rg hand-off.
    #[test]
    fn reports_ignore_files_for_rg_robust() {
        let tmp = tempfile::tempdir().unwrap();
        write(tmp.path(), ".jfcignore", "a\n");
        write(tmp.path(), ".claudeignore", "b\n");
        let policy = AccessPolicy::for_root(tmp.path());
        assert_eq!(policy.ignore_files().len(), 2);
    }
}
