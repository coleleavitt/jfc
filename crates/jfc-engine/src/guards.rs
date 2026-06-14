//! Quality-guard framework.
//!
//! The user's direction was that the slop guard, the planned coverage guard,
//! and the testsprite-style wiring guard "all belong to the same thing". This
//! module is that thing: a single [`Guard`] trait plus a [`GuardPipeline`] that
//! runs each member over a post-edit [`GuardContext`] and merges their output
//! into one [`SlopReport`].
//!
//! Deliberately *thin*. It does NOT re-implement the existing slop checks —
//! [`SlopGuard`] just forwards to [`crate::slop_guard::run_all_checks_with_old`],
//! which is already the focused, well-tested pipeline of `check_*` functions.
//! Each guard stays a focused module that owns its own analysis; the pipeline
//! owns only orchestration and reporting (severity, suppression, the shared
//! [`SlopFinding`] format). This keeps the no-god-object boundary: adding a
//! guard means adding a `Guard` impl, not growing a central struct.
//!
//! Members today:
//! - [`SlopGuard`] — the existing per-edit Rust slop/coherence/security checks.
//! - [`WiringGuard`] — testsprite-style static wiring checks: a tool that is
//!   dispatched but never advertised, or advertised but never dispatched, is a
//!   feature that silently can't be reached. See [`wiring`].

use std::path::Path;

use crate::slop_guard::{SlopFinding, SlopReport};

pub mod wiring;

/// Everything a guard needs to evaluate a single just-applied edit. Borrowed so
/// the pipeline can hand the same context to every member without cloning the
/// (potentially large) file content.
pub struct GuardContext<'a> {
    /// Absolute path of the file that was written/edited.
    pub file_path: &'a Path,
    /// The new (post-edit) file content.
    pub new_content: &'a str,
    /// The previous content, when the edit was a modification (enables
    /// diff-based checks). `None` for a freshly created file.
    pub old_content: Option<&'a str>,
    /// Workspace root, for repo-wide checks (git churn, cross-file wiring).
    pub cwd: &'a Path,
}

impl<'a> GuardContext<'a> {
    pub fn new(
        file_path: &'a Path,
        new_content: &'a str,
        old_content: Option<&'a str>,
        cwd: &'a Path,
    ) -> Self {
        Self {
            file_path,
            new_content,
            old_content,
            cwd,
        }
    }

    /// True when the edited file is Rust source — most guards only apply there.
    pub fn is_rust(&self) -> bool {
        self.file_path.extension().and_then(|e| e.to_str()) == Some("rs")
    }
}

/// A single quality guard. Each implementor owns one focused analysis and
/// returns findings in the shared [`SlopFinding`] shape so the pipeline can
/// merge and format every guard's output uniformly.
#[async_trait::async_trait]
pub trait Guard: Send + Sync {
    /// Stable identifier (used in tracing and to let a guard be skipped).
    fn name(&self) -> &'static str;

    /// Evaluate the edit. Must be best-effort: never panic, never block
    /// indefinitely — the pipeline already wraps the whole run in a timeout,
    /// but a guard should still bound its own work.
    async fn check(&self, ctx: &GuardContext<'_>) -> Vec<SlopFinding>;
}

/// The existing slop checks, exposed as a [`Guard`]. Forwards verbatim to the
/// established entry point so there is exactly one implementation of those
/// rules.
pub struct SlopGuard;

#[async_trait::async_trait]
impl Guard for SlopGuard {
    fn name(&self) -> &'static str {
        "slop"
    }

    async fn check(&self, ctx: &GuardContext<'_>) -> Vec<SlopFinding> {
        crate::slop_guard::run_all_checks_with_old(
            ctx.file_path,
            ctx.new_content,
            ctx.old_content,
            ctx.cwd,
        )
        .await
        .findings
    }
}

/// Static wiring guard: flags tools that are wired on only one side
/// (dispatched-but-unadvertised or advertised-but-undispatched). See
/// [`wiring::check_tool_wiring`].
pub struct WiringGuard;

#[async_trait::async_trait]
impl Guard for WiringGuard {
    fn name(&self) -> &'static str {
        "wiring"
    }

    async fn check(&self, ctx: &GuardContext<'_>) -> Vec<SlopFinding> {
        // Wiring is a cross-file property of the dispatch/advertise tables, so
        // it only runs when one of those tables is the file being edited. The
        // check itself is cheap text analysis over a couple of known files.
        if !ctx.is_rust() {
            return Vec::new();
        }
        wiring::check_tool_wiring(ctx.cwd, ctx.file_path)
    }
}

/// Runs a set of guards over one edit and merges their findings.
pub struct GuardPipeline {
    guards: Vec<Box<dyn Guard>>,
}

impl Default for GuardPipeline {
    fn default() -> Self {
        Self::with_default_guards()
    }
}

impl GuardPipeline {
    /// An empty pipeline; add guards with [`GuardPipeline::with_guard`].
    pub fn new() -> Self {
        Self { guards: Vec::new() }
    }

    /// The default post-edit pipeline: slop checks plus the wiring guard.
    pub fn with_default_guards() -> Self {
        Self::new()
            .with_guard(Box::new(SlopGuard))
            .with_guard(Box::new(WiringGuard))
    }

    pub fn with_guard(mut self, guard: Box<dyn Guard>) -> Self {
        self.guards.push(guard);
        self
    }

    /// Run every guard sequentially and concatenate their findings into one
    /// report. Sequential (not joined) on purpose: the members touch the same
    /// working tree / git index and the per-edit budget is small, so the
    /// determinism is worth more than the marginal parallelism.
    pub async fn run(&self, ctx: &GuardContext<'_>) -> SlopReport {
        let mut findings = Vec::new();
        for guard in &self.guards {
            let mut produced = guard.check(ctx).await;
            tracing::debug!(
                target: "jfc::guards",
                guard = guard.name(),
                count = produced.len(),
                "guard finished"
            );
            findings.append(&mut produced);
        }
        SlopReport {
            has_findings: !findings.is_empty(),
            findings,
        }
    }
}

/// Convenience wrapper mirroring [`crate::slop_guard::run_all_checks_with_old`]
/// but running the full guard pipeline (slop + wiring) instead of slop alone.
pub async fn run_guard_pipeline(
    file_path: &Path,
    new_content: &str,
    old_content: Option<&str>,
    cwd: &Path,
) -> SlopReport {
    let ctx = GuardContext::new(file_path, new_content, old_content, cwd);
    GuardPipeline::with_default_guards().run(&ctx).await
}

#[cfg(test)]
mod tests {
    use super::*;

    struct CountingGuard(&'static str, usize);

    #[async_trait::async_trait]
    impl Guard for CountingGuard {
        fn name(&self) -> &'static str {
            self.0
        }
        async fn check(&self, _ctx: &GuardContext<'_>) -> Vec<SlopFinding> {
            (0..self.1)
                .map(|i| SlopFinding {
                    rule: self.0.into(),
                    message: format!("finding {i}"),
                    file: None,
                    line: None,
                })
                .collect()
        }
    }

    // The pipeline merges every member's findings into one report.
    #[tokio::test]
    async fn pipeline_merges_member_findings_normal() {
        let pipeline = GuardPipeline::new()
            .with_guard(Box::new(CountingGuard("a", 2)))
            .with_guard(Box::new(CountingGuard("b", 3)));
        let cwd = std::env::temp_dir();
        let path = cwd.join("x.rs");
        let ctx = GuardContext::new(&path, "fn main() {}", None, &cwd);
        let report = pipeline.run(&ctx).await;
        assert!(report.has_findings);
        assert_eq!(report.findings.len(), 5);
        assert_eq!(report.findings.iter().filter(|f| f.rule == "a").count(), 2);
        assert_eq!(report.findings.iter().filter(|f| f.rule == "b").count(), 3);
    }

    // An empty pipeline (or all-empty guards) yields a no-findings report.
    #[tokio::test]
    async fn pipeline_empty_is_clean_robust() {
        let cwd = std::env::temp_dir();
        let path = cwd.join("x.rs");
        let ctx = GuardContext::new(&path, "", None, &cwd);
        let report = GuardPipeline::new().run(&ctx).await;
        assert!(!report.has_findings);
        assert!(report.findings.is_empty());
    }
}
