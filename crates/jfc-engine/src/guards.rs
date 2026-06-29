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

/// Post-edit diagnostics guard (Junie `ErrorCheckingService` parity, phase 1).
///
/// Surfaces *already-known* compiler/LSP diagnostics for the file the agent just
/// edited, inline in the edit's tool result, so the model self-corrects on the
/// SAME turn instead of only seeing them in the next turn's prompt seed.
///
/// Phase 1 is deliberately **snapshot-only**: it reads
/// [`crate::diagnostics::global_snapshot`] (populated by the existing cargo-check
/// producer / LSP push) and never compiles anything itself. A real type-check
/// (`cargo check`, seconds) can NOT run on this synchronous 2s-budgeted path —
/// see `docs/jfc-post-edit-diagnostics.md` §3.1 — so type-checking stays on the
/// existing async producer. This guard adds ~zero latency: it's a vector filter.
///
/// Off by default. Opt in with `JFC_POST_EDIT_DIAGNOSTICS=1` so the default
/// post-edit pipeline stays byte-identical until the §7 baseline A/B justifies it.
pub struct DiagnosticsGuard;

impl DiagnosticsGuard {
    /// Runtime opt-in. Mirrors the `JFC_DISABLE_CARGO_CHECK` / voice-neural gate
    /// style: a single env flag, default off.
    pub fn enabled() -> bool {
        matches!(
            std::env::var("JFC_POST_EDIT_DIAGNOSTICS").as_deref(),
            Ok("1") | Ok("true")
        )
    }
}

#[async_trait::async_trait]
impl Guard for DiagnosticsGuard {
    fn name(&self) -> &'static str {
        "diagnostics"
    }

    async fn check(&self, ctx: &GuardContext<'_>) -> Vec<SlopFinding> {
        if !Self::enabled() {
            return Vec::new();
        }
        let entries = crate::diagnostics::global_snapshot();
        if entries.is_empty() {
            return Vec::new();
        }
        // Match the edited file the same way the LSP `diagnostics` tool does:
        // exact absolute-path match first, then a basename fallback (snapshots
        // can carry differently-rooted absolute paths across producers/OSes).
        let file_str = ctx.file_path.display().to_string();
        let mut matched: Vec<&crate::diagnostics::DiagnosticEntry> =
            entries.iter().filter(|e| e.file == file_str).collect();
        if matched.is_empty()
            && let Some(name) = ctx.file_path.file_name().and_then(|s| s.to_str())
        {
            matched = entries.iter().filter(|e| e.file.ends_with(name)).collect();
        }
        matched
            .into_iter()
            .map(|e| SlopFinding {
                rule: "diagnostics".into(),
                message: crate::diagnostics::format_entry(e).trim().to_owned(),
                file: Some(e.file.clone()),
                line: Some(e.line as usize),
            })
            .collect()
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

    /// The default post-edit pipeline: slop checks, the wiring guard, and the
    /// (opt-in) diagnostics guard. `DiagnosticsGuard` self-skips unless
    /// `JFC_POST_EDIT_DIAGNOSTICS=1`, so registering it here is free until
    /// enabled.
    pub fn with_default_guards() -> Self {
        Self::new()
            .with_guard(Box::new(SlopGuard))
            .with_guard(Box::new(WiringGuard))
            .with_guard(Box::new(DiagnosticsGuard))
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

    // ── DiagnosticsGuard (post-edit diagnostics, phase 1) ──────────────────
    //
    // These mutate two process-global resources — the diagnostics snapshot and
    // the `JFC_POST_EDIT_DIAGNOSTICS` env flag — so they run under one mutex and
    // restore both on exit. Distinct per-test basenames keep the snapshot filter
    // from cross-matching.
    use tokio::sync::Mutex as AsyncMutex;
    static DIAG_GUARD_LOCK: AsyncMutex<()> = AsyncMutex::const_new(());

    fn diag_entry(file: &str, line: u32, msg: &str) -> crate::diagnostics::DiagnosticEntry {
        crate::diagnostics::DiagnosticEntry {
            file: file.to_owned(),
            line,
            col: 1,
            message: msg.to_owned(),
            code: None,
            source: Some("rustc".into()),
            severity: crate::diagnostics::Severity::Error,
        }
    }

    /// Run `body` with the opt-in flag forced on and the snapshot set, restoring
    /// both afterward. Guards against a poisoned lock so one failing test can't
    /// cascade.
    async fn with_diag_env<F, Fut>(snapshot: Vec<crate::diagnostics::DiagnosticEntry>, body: F)
    where
        F: FnOnce() -> Fut,
        Fut: std::future::Future<Output = ()>,
    {
        let _g = DIAG_GUARD_LOCK.lock().await;
        let prev_flag = std::env::var("JFC_POST_EDIT_DIAGNOSTICS").ok();
        let prev_snapshot = crate::diagnostics::global_snapshot();
        // SAFETY: env mutation is serialized by DIAG_GUARD_LOCK and restored below.
        unsafe { std::env::set_var("JFC_POST_EDIT_DIAGNOSTICS", "1") };
        crate::diagnostics::set_global_snapshot(snapshot);

        body().await;

        crate::diagnostics::set_global_snapshot(prev_snapshot);
        unsafe {
            match prev_flag {
                Some(v) => std::env::set_var("JFC_POST_EDIT_DIAGNOSTICS", v),
                None => std::env::remove_var("JFC_POST_EDIT_DIAGNOSTICS"),
            }
        }
    }

    // Normal: a known snapshot error for the edited file is surfaced inline.
    #[tokio::test]
    async fn diagnostics_guard_surfaces_known_snapshot_entry_for_edited_file_normal() {
        let cwd = std::env::temp_dir();
        let path = cwd.join("dg_surfaced_unit.rs");
        let file_str = path.display().to_string();
        with_diag_env(
            vec![diag_entry(&file_str, 12, "unresolved import `foo`")],
            || async {
                let ctx = GuardContext::new(&path, "use foo;", None, &cwd);
                let findings = DiagnosticsGuard.check(&ctx).await;
                assert_eq!(findings.len(), 1, "{findings:?}");
                assert_eq!(findings[0].rule, "diagnostics");
                assert!(
                    findings[0].message.contains("unresolved import"),
                    "{:?}",
                    findings[0]
                );
                assert_eq!(findings[0].line, Some(12));
            },
        )
        .await;
    }

    // Robust: snapshot entries for OTHER files must not leak onto this edit.
    #[tokio::test]
    async fn diagnostics_guard_ignores_entries_for_other_files_robust() {
        let cwd = std::env::temp_dir();
        let edited = cwd.join("dg_edited_unit.rs");
        let other = cwd.join("dg_other_unit.rs");
        let other_str = other.display().to_string();
        with_diag_env(
            vec![diag_entry(&other_str, 3, "mismatched types")],
            || async {
                let ctx = GuardContext::new(&edited, "fn x() {}", None, &cwd);
                let findings = DiagnosticsGuard.check(&ctx).await;
                assert!(
                    findings.is_empty(),
                    "other-file diagnostics leaked: {findings:?}"
                );
            },
        )
        .await;
    }

    // Robust: an empty snapshot produces no findings (no noise on clean trees).
    #[tokio::test]
    async fn diagnostics_guard_empty_snapshot_is_silent_robust() {
        let cwd = std::env::temp_dir();
        let path = cwd.join("dg_empty_unit.rs");
        with_diag_env(Vec::new(), || async {
            let ctx = GuardContext::new(&path, "fn x() {}", None, &cwd);
            assert!(DiagnosticsGuard.check(&ctx).await.is_empty());
        })
        .await;
    }

    // Regression: with the opt-in flag OFF, the guard is inert even when the
    // snapshot has a matching entry — guarantees the default pipeline is
    // byte-identical to pre-feature behavior.
    #[tokio::test]
    async fn diagnostics_guard_disabled_by_default_regression() {
        let _g = DIAG_GUARD_LOCK.lock().await;
        let prev_flag = std::env::var("JFC_POST_EDIT_DIAGNOSTICS").ok();
        let prev_snapshot = crate::diagnostics::global_snapshot();
        let cwd = std::env::temp_dir();
        let path = cwd.join("dg_disabled_unit.rs");
        let file_str = path.display().to_string();
        // SAFETY: serialized by DIAG_GUARD_LOCK; restored below.
        unsafe { std::env::remove_var("JFC_POST_EDIT_DIAGNOSTICS") };
        crate::diagnostics::set_global_snapshot(vec![diag_entry(&file_str, 1, "boom")]);

        let ctx = GuardContext::new(&path, "fn x() {}", None, &cwd);
        let findings = DiagnosticsGuard.check(&ctx).await;
        assert!(
            findings.is_empty(),
            "guard ran while disabled: {findings:?}"
        );

        crate::diagnostics::set_global_snapshot(prev_snapshot);
        unsafe {
            if let Some(v) = prev_flag {
                std::env::set_var("JFC_POST_EDIT_DIAGNOSTICS", v);
            }
        }
    }

    // Matches the basename fallback the LSP tool uses: a snapshot entry whose
    // path differs in root but shares the basename still matches.
    #[tokio::test]
    async fn diagnostics_guard_basename_fallback_matches_normal() {
        let cwd = std::env::temp_dir();
        let path = cwd.join("dg_basename_unit.rs");
        // Snapshot path has a different absolute root but the same basename.
        with_diag_env(
            vec![diag_entry(
                "/elsewhere/proj/dg_basename_unit.rs",
                7,
                "type error",
            )],
            || async {
                let ctx = GuardContext::new(&path, "fn x() {}", None, &cwd);
                let findings = DiagnosticsGuard.check(&ctx).await;
                assert_eq!(findings.len(), 1, "basename fallback failed: {findings:?}");
            },
        )
        .await;
    }
}
