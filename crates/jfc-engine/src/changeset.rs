//! Git-aware integration layer over the pure `jfc-changeset` crate.
//!
//! `jfc-changeset` is intentionally IO-free: it models the [`AgentChangeSet`]
//! object, its lifecycle state machine, and a JSONL store, but knows nothing
//! about git or worktrees. This module supplies the missing half — resolving
//! the base head, computing the diff against it, and persisting the change-set
//! — so the worktree dispatch paths (foreground Task tool, background worker)
//! can open a change-set when they create a `jfc/<name>` branch and finalize it
//! when the agent finishes.
//!
//! Every operation here is best-effort: a change-set is an audit/review
//! convenience, never a correctness dependency of the agent run. Failures are
//! logged and swallowed so a git hiccup can't break a task.

use std::path::Path;

use jfc_changeset::{
    AgentChangeSet, ChangeStore, ChangedFile, EventKind, LedgerEvent, LedgerFilter, LedgerStore,
};

/// Provenance captured when a worktree-isolated agent starts.
#[derive(Debug, Clone, Default)]
pub struct ChangeOrigin {
    pub task_id: Option<String>,
    pub agent_id: Option<String>,
    pub session_id: Option<String>,
}

/// What a dispatch should do when an agent asked for worktree isolation but
/// the worktree could not be created.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IsolationFallback {
    /// Fail the dispatch — do NOT run the agent in the main checkout.
    FailClosed,
    /// Permissively run in the parent cwd (legacy behaviour).
    AllowCwd,
}

/// Decide the fallback policy from the env override and config. Default is
/// fail-closed so a mutating agent can never silently touch production when its
/// isolation request fails. `JFC_ISOLATION_FAIL_CLOSED=0` (or config
/// `[isolation] fail_closed = false`) restores the permissive fall-back.
///
/// Pure in its single `fail_closed` input so the policy is unit-testable; the
/// env/config resolution lives in [`isolation_fallback`].
pub fn isolation_fallback_for(fail_closed: bool) -> IsolationFallback {
    if fail_closed {
        IsolationFallback::FailClosed
    } else {
        IsolationFallback::AllowCwd
    }
}

/// Resolve the effective isolation fallback policy: env override wins, then
/// `[isolation] fail_closed` config, else the fail-closed default.
pub fn isolation_fallback() -> IsolationFallback {
    if let Ok(v) = std::env::var("JFC_ISOLATION_FAIL_CLOSED") {
        let v = v.trim().to_ascii_lowercase();
        if matches!(v.as_str(), "0" | "false" | "no" | "off") {
            return IsolationFallback::AllowCwd;
        }
        if matches!(v.as_str(), "1" | "true" | "yes" | "on") {
            return IsolationFallback::FailClosed;
        }
    }
    let fail_closed = crate::config::load_arc()
        .isolation
        .as_ref()
        .map(|i| i.fail_closed)
        .unwrap_or(true);
    isolation_fallback_for(fail_closed)
}

/// Current unix-epoch milliseconds (the timestamp unit the store uses).
fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Resolve the project root for ledger/store IO. Best-effort: the repo root
/// when inside a git tree, else the current dir.
fn project_root() -> std::path::PathBuf {
    std::env::current_dir().unwrap_or_default()
}

/// Append an event to the runtime audit ledger. Best-effort and synchronous
/// (an append is a single locked line write); failures log and are swallowed
/// so audit never breaks a tool call. Call from a blocking context or wrap in
/// `spawn_blocking` on hot async paths.
pub fn record_event(event: LedgerEvent) {
    let root = project_root();
    match LedgerStore::open_project(&root).and_then(|s| s.append(&event)) {
        Ok(()) => {}
        Err(e) => tracing::warn!(target: "jfc::audit", error = %e, "failed to append ledger event"),
    }
}

/// Extract a concise, non-sensitive detail string for a mutating tool call —
/// the command for Bash, the target path for Edit/Write/MultiEdit. Long
/// commands are truncated so the ledger stays scannable.
pub fn ledger_detail_for(kind: &crate::types::ToolKind, input: &crate::types::ToolInput) -> String {
    use crate::types::{ToolInput, ToolKind};
    match (kind, input) {
        (ToolKind::Bash, ToolInput::Bash { command, .. }) => {
            let cmd = command.trim();
            if cmd.chars().count() > 120 {
                let head: String = cmd.chars().take(117).collect();
                format!("{head}...")
            } else {
                cmd.to_string()
            }
        }
        (ToolKind::Edit, ToolInput::Edit { file_path, .. })
        | (ToolKind::Write, ToolInput::Write { file_path, .. })
        | (ToolKind::MultiEdit, ToolInput::MultiEdit { file_path, .. }) => file_path.clone(),
        (
            ToolKind::DesignWriteFile
            | ToolKind::DesignDeleteFile
            | ToolKind::DesignRegisterAsset
            | ToolKind::DesignUnregisterAsset,
            ToolInput::DesignWriteFile {
                project_id, path, ..
            }
            | ToolInput::DesignDeleteFile { project_id, path }
            | ToolInput::DesignRegisterAsset {
                project_id, path, ..
            }
            | ToolInput::DesignUnregisterAsset { project_id, path },
        ) => format!("{project_id}/{path}"),
        (
            ToolKind::DesignCopyFile,
            ToolInput::DesignCopyFile {
                project_id,
                from_path,
                to_path,
            },
        ) => format!("{project_id}/{from_path} -> {to_path}"),
        (ToolKind::DesignProjectCreate, ToolInput::DesignProjectCreate { title }) => title.clone(),
        (ToolKind::DesignProjectSetMeta, ToolInput::DesignProjectSetMeta { project_id, .. }) => {
            project_id.clone()
        }
        (ToolKind::DesignBundleHtml, ToolInput::DesignBundleHtml { input, output, .. }) => {
            match output {
                Some(output) => format!("{input} -> {output}"),
                None => input.clone(),
            }
        }
        (
            ToolKind::DesignHandoff,
            ToolInput::DesignHandoff {
                project_dir,
                feature,
                ..
            },
        ) => {
            format!("{project_dir}: {feature}")
        }
        (ToolKind::DesignCheckSystem, ToolInput::DesignCheckSystem { project_dir }) => {
            project_dir.clone()
        }
        _ => String::new(),
    }
}

/// Record an approval grant/denial to the audit ledger — the security trail
/// of "what was the agent allowed to do, and who decided". `granted=false`
/// records a denial.
pub fn record_approval(tool: &str, granted: bool, session_id: Option<String>) {
    let detail = if granted { "granted" } else { "denied" };
    record_event(
        LedgerEvent::new(now_ms(), EventKind::Approval, tool)
            .with_detail(detail)
            .with_session_id(session_id),
    );
}

/// Record a provider/model call (the start of a stream) to the audit ledger.
pub fn record_provider_call(model: &str, session_id: Option<String>) {
    record_event(
        LedgerEvent::new(now_ms(), EventKind::ProviderCall, model).with_session_id(session_id),
    );
}

/// Record a cancellation (user interrupt / turn abort) to the audit ledger.
pub fn record_cancellation(subject: &str, task_id: Option<String>) {
    record_event(
        LedgerEvent::new(now_ms(), EventKind::Cancellation, subject).with_task_id(task_id),
    );
}

/// Record a daemon-driven background job start to the audit ledger.
pub fn record_daemon_job(task_id: &str, detail: &str, session_id: Option<String>) {
    record_event(
        LedgerEvent::new(now_ms(), EventKind::DaemonJob, task_id)
            .with_detail(detail)
            .with_task_id(Some(task_id.to_string()))
            .with_session_id(session_id),
    );
}

/// Convenience: record a tool-call event tagged with optional provenance.
pub fn record_tool_call(
    tool: &str,
    detail: impl Into<String>,
    change_id: Option<String>,
    task_id: Option<String>,
) {
    record_event(
        LedgerEvent::new(now_ms(), EventKind::ToolCall, tool)
            .with_detail(detail)
            .with_change_id(change_id)
            .with_task_id(task_id),
    );
}

/// Query the runtime ledger at `root` (newest-first for display). Best-effort:
/// returns an empty vec on IO error.
pub fn query_ledger_in(root: &Path, filter: &LedgerFilter) -> Vec<LedgerEvent> {
    match LedgerStore::open_project(root).and_then(|s| s.query(filter)) {
        Ok(mut events) => {
            events.reverse();
            events
        }
        Err(e) => {
            tracing::warn!(target: "jfc::audit", error = %e, "failed to query ledger");
            Vec::new()
        }
    }
}

/// Render a queried ledger as a compact text table for `jfc audit` / `/audit`.
/// One line per event: `<iso-ish ms>  <kind>  <subject>  [change=<id>]`.
pub fn render_ledger(events: &[LedgerEvent]) -> String {
    if events.is_empty() {
        return "No audit events recorded.".to_string();
    }
    let mut out = String::with_capacity(events.len() * 48);
    for e in events {
        out.push_str(&format!(
            "{:>14}  {:<13} {}",
            e.at_ms,
            e.kind.label(),
            e.subject
        ));
        if let Some(cid) = &e.change_id {
            out.push_str(&format!("  [change={cid}]"));
        }
        out.push('\n');
    }
    out
}

/// `git -C <dir> rev-parse HEAD`, or `None` if it can't be resolved.
async fn resolve_head(dir: &Path) -> Option<String> {
    let out = tokio::process::Command::new("git")
        .arg("-C")
        .arg(dir)
        .arg("rev-parse")
        .arg("HEAD")
        .output()
        .await
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let head = String::from_utf8_lossy(&out.stdout).trim().to_owned();
    (!head.is_empty()).then_some(head)
}

/// Open a `Draft` change-set for a freshly created worktree and persist it.
///
/// Returns the change-set id on success so the caller can finalize it later.
/// All failures (no repo, store unwritable) degrade to `None` + a warning —
/// the agent still runs, just without a change-set record.
pub async fn open_for_worktree(
    repo_root: &Path,
    worktree_path: &str,
    branch: &str,
    origin: &ChangeOrigin,
) -> Option<String> {
    let base_head = resolve_head(repo_root).await.unwrap_or_default();
    let cs = {
        let mut cs = AgentChangeSet::open(base_head, branch, worktree_path, now_ms());
        cs.task_id = origin.task_id.clone();
        cs.agent_id = origin.agent_id.clone();
        cs.session_id = origin.session_id.clone();
        cs
    };
    let id = cs.id.clone();

    let root = repo_root.to_path_buf();
    let persist = tokio::task::spawn_blocking(move || -> jfc_changeset::Result<()> {
        let mut store = ChangeStore::open_project(&root)?;
        store.upsert(cs)
    })
    .await;

    match persist {
        Ok(Ok(())) => {
            tracing::info!(
                target: "jfc::changeset",
                change_id = %id,
                branch,
                worktree = worktree_path,
                "opened change-set (Draft) for isolated agent"
            );
            Some(id)
        }
        Ok(Err(e)) => {
            tracing::warn!(target: "jfc::changeset", error = %e, "failed to persist new change-set");
            None
        }
        Err(e) => {
            tracing::warn!(target: "jfc::changeset", error = %e, "change-set persist task panicked");
            None
        }
    }
}

/// `git -C <worktree> diff --numstat <base_head>` → per-file insert/delete
/// counts plus a one-line summary. Empty when there are no changes.
async fn diff_against_base(worktree: &str, base_head: &str) -> (Vec<ChangedFile>, String) {
    let mut args = vec!["-C".to_string(), worktree.to_string(), "diff".to_string()];
    if !base_head.is_empty() {
        args.push(base_head.to_string());
    }
    args.push("--numstat".to_string());

    let out = match tokio::process::Command::new("git")
        .args(&args)
        .output()
        .await
    {
        Ok(out) if out.status.success() => out,
        _ => return (Vec::new(), String::new()),
    };

    let text = String::from_utf8_lossy(&out.stdout);
    let mut files = Vec::new();
    let (mut total_ins, mut total_del) = (0u32, 0u32);
    for line in text.lines() {
        // Format: "<insertions>\t<deletions>\t<path>"; binary files use "-".
        let mut parts = line.splitn(3, '\t');
        let ins = parts.next().unwrap_or("0");
        let del = parts.next().unwrap_or("0");
        let Some(path) = parts.next() else { continue };
        let insertions = ins.parse().unwrap_or(0);
        let deletions = del.parse().unwrap_or(0);
        total_ins += insertions;
        total_del += deletions;
        files.push(ChangedFile {
            path: path.to_string(),
            insertions,
            deletions,
        });
    }

    let summary = if files.is_empty() {
        String::new()
    } else {
        format!(
            "{} file{} changed, {total_ins} insertion(+), {total_del} deletion(-)",
            files.len(),
            if files.len() == 1 { "" } else { "s" }
        )
    };
    (files, summary)
}

/// Finalize a change-set after the agent completes: compute the diff against
/// its base head and transition `Draft → Ready` (or `Draft → Abandoned` if the
/// worktree ended up clean — nothing to review). Best-effort.
pub async fn finalize_for_worktree(repo_root: &Path, change_id: &str, worktree_path: &str) {
    let root = repo_root.to_path_buf();
    let id = change_id.to_string();

    // Load the change-set to read its base_head off the store.
    let loaded = {
        let root = root.clone();
        let id = id.clone();
        tokio::task::spawn_blocking(move || {
            ChangeStore::open_project(&root)
                .ok()
                .and_then(|s| s.get(&id).cloned())
        })
        .await
        .ok()
        .flatten()
    };
    let Some(mut cs) = loaded else {
        tracing::warn!(target: "jfc::changeset", change_id = %id, "finalize: change-set not found");
        return;
    };

    let (files, summary) = diff_against_base(worktree_path, &cs.base_head).await;
    let now = now_ms();
    let result = if files.is_empty() {
        // Clean worktree — nothing to apply; mark it abandoned so it doesn't
        // linger as a reviewable Draft forever.
        cs.transition_to(jfc_changeset::ChangeState::Abandoned, now)
    } else {
        cs.mark_ready(files, summary.clone(), now)
    };
    if let Err(e) = result {
        tracing::warn!(target: "jfc::changeset", change_id = %id, error = %e, "finalize transition failed");
        return;
    }
    let terminal_state = cs.state;

    let persisted = tokio::task::spawn_blocking(move || -> jfc_changeset::Result<()> {
        let mut store = ChangeStore::open_project(&root)?;
        store.upsert(cs)
    })
    .await;
    match persisted {
        Ok(Ok(())) => tracing::info!(
            target: "jfc::changeset",
            change_id = %id,
            state = terminal_state.label(),
            summary = %summary,
            "finalized change-set"
        ),
        Ok(Err(e)) => {
            tracing::warn!(target: "jfc::changeset", change_id = %id, error = %e, "finalize persist failed")
        }
        Err(e) => {
            tracing::warn!(target: "jfc::changeset", change_id = %id, error = %e, "finalize task panicked")
        }
    }
}

// ── `jfc changes` operations ───────────────────────────────────────────────
//
// These back both the `jfc changes` CLI subcommand and the `/changes` slash
// command. They wrap the pure jfc-changeset store with the git operations it
// deliberately omits (test in worktree, merge branch, revert merge).

use jfc_changeset::{ChangeFilter, ChangeState, TestRun};

/// Open the change store at `root`, mapping errors to a user string.
fn open_store(root: &Path) -> std::result::Result<ChangeStore, String> {
    ChangeStore::open_project(root).map_err(|e| format!("opening change store: {e}"))
}

/// `jfc changes list` — a compact table of every change-set, newest first.
pub fn list_changes(root: &Path) -> String {
    let store = match open_store(root) {
        Ok(s) => s,
        Err(e) => return e,
    };
    let all = store.query(&ChangeFilter::default());
    if all.is_empty() {
        return "No change-sets recorded.".to_string();
    }
    let mut out = String::from("CHANGE-ID         STATE      FILES  BRANCH\n");
    for cs in all {
        out.push_str(&format!(
            "{:<16}  {:<9}  {:>5}  {}\n",
            cs.id,
            cs.state.label(),
            cs.changed_files.len(),
            cs.branch
        ));
    }
    out
}

/// `jfc changes show <id>` — full detail: provenance, diff summary, files,
/// test runs, approval, and the change's audit-ledger events.
pub fn show_change(root: &Path, id: &str) -> String {
    let store = match open_store(root) {
        Ok(s) => s,
        Err(e) => return e,
    };
    let Some(cs) = store.get(id) else {
        return format!("change-set {id} not found");
    };
    let mut out = format!(
        "change {id}\n  state:    {}\n  branch:   {}\n  base:     {}\n  worktree: {}\n",
        cs.state.label(),
        cs.branch,
        cs.base_head,
        cs.worktree_path,
    );
    if let Some(t) = &cs.task_id {
        out.push_str(&format!("  task:     {t}\n"));
    }
    if !cs.diff_summary.is_empty() {
        out.push_str(&format!("  diff:     {}\n", cs.diff_summary));
    }
    for f in &cs.changed_files {
        out.push_str(&format!(
            "    {} (+{} -{})\n",
            f.path, f.insertions, f.deletions
        ));
    }
    for t in &cs.test_runs {
        out.push_str(&format!(
            "  test:     `{}` exit={} ({}ms)\n",
            t.command, t.exit_code, t.duration_ms
        ));
    }
    let events = query_ledger_in(
        root,
        &LedgerFilter {
            change_id: Some(id.to_string()),
            ..Default::default()
        },
    );
    if !events.is_empty() {
        out.push_str(&format!("  ledger:   {} event(s)\n", events.len()));
    }
    out
}

/// `jfc changes test <id> -- <cmd...>` — run a test command in the change's
/// worktree, record the result, and advance Ready→Tested. Best-effort git.
pub async fn test_change(root: &Path, id: &str, command: &str) -> String {
    let (worktree, base_state) = {
        let store = match open_store(root) {
            Ok(s) => s,
            Err(e) => return e,
        };
        let Some(cs) = store.get(id) else {
            return format!("change-set {id} not found");
        };
        (cs.worktree_path.clone(), cs.state)
    };
    if base_state != ChangeState::Ready {
        return format!(
            "change {id} is {} — only a Ready change can be tested",
            base_state.label()
        );
    }

    let started = std::time::Instant::now();
    let output = tokio::process::Command::new("sh")
        .arg("-c")
        .arg(command)
        .current_dir(&worktree)
        .output()
        .await;
    let (exit_code, duration_ms) = match output {
        Ok(o) => (
            o.status.code().unwrap_or(-1),
            started.elapsed().as_millis() as u64,
        ),
        Err(e) => return format!("failed to run test command: {e}"),
    };

    let run = TestRun {
        command: command.to_string(),
        exit_code,
        duration_ms,
        finished_at_ms: now_ms(),
    };
    let mut store = match open_store(root) {
        Ok(s) => s,
        Err(e) => return e,
    };
    let Some(mut cs) = store.get(id).cloned() else {
        return format!("change-set {id} not found");
    };
    if let Err(e) = cs.record_test_run(run, now_ms()) {
        return format!("recording test run: {e}");
    }
    if let Err(e) = store.upsert(cs) {
        return format!("persisting test run: {e}");
    }
    format!(
        "test {} for {id} (exit {exit_code}, {duration_ms}ms) — state now Tested",
        if exit_code == 0 { "passed" } else { "FAILED" }
    )
}

/// `jfc changes apply <id>` — merge the change's branch into the base. Refuses
/// unless the change is Approved (the review/test-before-production gate is
/// enforced by the state machine). Surfaces merge conflicts instead of losing
/// them. On a clean merge, transitions Approved→Applied.
pub async fn apply_change(root: &Path, id: &str) -> String {
    let (branch, state) = {
        let store = match open_store(root) {
            Ok(s) => s,
            Err(e) => return e,
        };
        let Some(cs) = store.get(id) else {
            return format!("change-set {id} not found");
        };
        (cs.branch.clone(), cs.state)
    };
    if state != ChangeState::Approved {
        return format!(
            "change {id} is {} — only an Approved change can be applied \
             (run tests + approve first; review/test-before-production is enforced)",
            state.label()
        );
    }

    let merge = tokio::process::Command::new("git")
        .arg("-C")
        .arg(root)
        .arg("merge")
        .arg("--no-ff")
        .arg(&branch)
        .output()
        .await;
    match merge {
        Ok(o) if o.status.success() => {}
        Ok(o) => {
            // Abort a conflicted merge so the working tree isn't left dirty,
            // but surface the conflict text — never silently lose it.
            let _ = tokio::process::Command::new("git")
                .arg("-C")
                .arg(root)
                .arg("merge")
                .arg("--abort")
                .output()
                .await;
            return format!(
                "merge of {branch} failed (conflicts surfaced, merge aborted):\n{}",
                String::from_utf8_lossy(&o.stdout)
            );
        }
        Err(e) => return format!("failed to spawn git merge: {e}"),
    }

    let mut store = match open_store(root) {
        Ok(s) => s,
        Err(e) => return e,
    };
    let Some(mut cs) = store.get(id).cloned() else {
        return format!("change-set {id} not found");
    };
    if let Err(e) = cs.transition_to(ChangeState::Applied, now_ms()) {
        return format!("state transition failed: {e}");
    }
    if let Err(e) = store.upsert(cs) {
        return format!("persisting applied state: {e}");
    }
    record_event(
        LedgerEvent::new(now_ms(), EventKind::Approval, "apply")
            .with_detail(format!("merged {branch}"))
            .with_change_id(Some(id.to_string())),
    );
    format!("applied {id}: merged {branch} into base — state now Applied")
}

/// `jfc changes revert <id>` — undo a previously applied change with
/// `git revert`. Transitions Applied→Reverted.
pub async fn revert_change(root: &Path, id: &str) -> String {
    let state = {
        let store = match open_store(root) {
            Ok(s) => s,
            Err(e) => return e,
        };
        match store.get(id) {
            Some(cs) => cs.state,
            None => return format!("change-set {id} not found"),
        }
    };
    if state != ChangeState::Applied {
        return format!(
            "change {id} is {} — only an Applied change can be reverted",
            state.label()
        );
    }

    let revert = tokio::process::Command::new("git")
        .arg("-C")
        .arg(root)
        .arg("revert")
        .arg("--no-edit")
        .arg("-m")
        .arg("1")
        .arg("HEAD")
        .output()
        .await;
    match revert {
        Ok(o) if o.status.success() => {}
        Ok(o) => {
            return format!("git revert failed:\n{}", String::from_utf8_lossy(&o.stderr));
        }
        Err(e) => return format!("failed to spawn git revert: {e}"),
    }

    let mut store = match open_store(root) {
        Ok(s) => s,
        Err(e) => return e,
    };
    let Some(mut cs) = store.get(id).cloned() else {
        return format!("change-set {id} not found");
    };
    if let Err(e) = cs.transition_to(ChangeState::Reverted, now_ms()) {
        return format!("state transition failed: {e}");
    }
    if let Err(e) = store.upsert(cs) {
        return format!("persisting reverted state: {e}");
    }
    format!("reverted {id} — state now Reverted")
}

#[cfg(test)]
mod tests {
    use super::*;
    use jfc_changeset::ChangeState;

    // Normal: the default policy (fail_closed = true) refuses the cwd fallback.
    #[test]
    fn isolation_fail_closed_default_refuses_cwd_normal() {
        assert_eq!(
            isolation_fallback_for(true),
            IsolationFallback::FailClosed,
            "fail_closed=true must refuse the main-checkout fallback"
        );
    }

    // Robust: explicitly opting out (fail_closed = false) restores the legacy
    // permissive fall-back to cwd.
    #[test]
    fn isolation_fail_open_allows_cwd_robust() {
        assert_eq!(isolation_fallback_for(false), IsolationFallback::AllowCwd);
    }

    // Normal: the audit ledger detail extractor pulls the command for Bash and
    // the path for Write.
    #[test]
    fn ledger_detail_extracts_command_and_path_normal() {
        let bash = crate::types::ToolInput::Bash {
            command: "cargo test".into(),
            timeout: None,
            workdir: None,
            run_in_background: None,
            suppress_output: None,
        };
        assert_eq!(
            ledger_detail_for(&crate::types::ToolKind::Bash, &bash),
            "cargo test"
        );
        let write = crate::types::ToolInput::Write {
            file_path: "src/lib.rs".into(),
            content: "x".into(),
        };
        assert_eq!(
            ledger_detail_for(&crate::types::ToolKind::Write, &write),
            "src/lib.rs"
        );
    }

    // Robust — the review-gate at the command layer: `apply_change` refuses a
    // change that is merely Ready (not Tested+Approved). No git is invoked
    // because the gate rejects before the merge.
    #[tokio::test]
    async fn apply_refuses_unapproved_change_robust() {
        let dir = tempfile::TempDir::new().unwrap();
        let root = dir.path();
        let mut store = ChangeStore::open_project(root).unwrap();
        let mut cs = jfc_changeset::AgentChangeSet::open("h", "jfc/x", "/tmp/x", 1);
        cs.mark_ready(Vec::new(), "noop", 2).unwrap();
        let id = cs.id.clone();
        store.upsert(cs).unwrap();

        let msg = apply_change(root, &id).await;
        assert!(
            msg.contains("only an Approved change can be applied"),
            "Ready change must be refused: {msg}"
        );
        // State unchanged on disk.
        let store = ChangeStore::open_project(root).unwrap();
        assert_eq!(store.get(&id).unwrap().state, ChangeState::Ready);
    }

    // Robust: list/show render gracefully on an unknown id / empty store.
    #[test]
    fn list_and_show_handle_empty_and_missing_robust() {
        let dir = tempfile::TempDir::new().unwrap();
        let root = dir.path();
        assert_eq!(list_changes(root), "No change-sets recorded.");
        assert!(show_change(root, "nope").contains("not found"));
    }

    // Robust — the end-to-end audit-ledger contract: append events to a
    // project root, then query-filter by change_id and render. Proves the
    // "what did this agent do, queryable per change" requirement.
    #[test]
    fn audit_ledger_emit_query_render_robust() {
        let dir = tempfile::TempDir::new().unwrap();
        let root = dir.path().to_path_buf();

        // Two events on change cs-1, one on cs-2.
        for (kind, subject, cid) in [
            (EventKind::ToolCall, "Bash", "cs-1"),
            (EventKind::FileWrite, "src/a.rs", "cs-1"),
            (EventKind::ToolCall, "Edit", "cs-2"),
        ] {
            let store = LedgerStore::open_project(&root).unwrap();
            store
                .append(
                    &LedgerEvent::new(now_ms(), kind, subject)
                        .with_change_id(Some(cid.to_string())),
                )
                .unwrap();
        }

        let cs1 = query_ledger_in(
            &root,
            &LedgerFilter {
                change_id: Some("cs-1".into()),
                ..Default::default()
            },
        );
        assert_eq!(cs1.len(), 2, "two events tagged cs-1");
        let rendered = render_ledger(&cs1);
        assert!(rendered.contains("[change=cs-1]"));
        assert!(!rendered.contains("cs-2"), "filter must exclude cs-2");

        // Empty render is graceful.
        assert_eq!(render_ledger(&[]), "No audit events recorded.");
    }

    // Robust — full emission surface: approvals, provider calls, and
    // cancellations all land in the ledger with the right EventKind, queryable
    // by kind. Proves the audit covers more than mutating tool calls.
    #[test]
    fn ledger_records_approval_provider_cancellation_robust() {
        use jfc_changeset::{EventKind, LedgerEvent, LedgerStore};
        let dir = tempfile::TempDir::new().unwrap();
        let root = dir.path();

        // Emit one of each new event kind directly to this root's ledger.
        let store = LedgerStore::open_project(root).unwrap();
        store
            .append(&LedgerEvent::new(now_ms(), EventKind::Approval, "Bash").with_detail("granted"))
            .unwrap();
        store
            .append(&LedgerEvent::new(now_ms(), EventKind::Approval, "Write").with_detail("denied"))
            .unwrap();
        store
            .append(&LedgerEvent::new(now_ms(), EventKind::ProviderCall, "opus"))
            .unwrap();
        store
            .append(&LedgerEvent::new(now_ms(), EventKind::Cancellation, "task"))
            .unwrap();

        let approvals = query_ledger_in(
            root,
            &LedgerFilter {
                kind: Some(EventKind::Approval),
                ..Default::default()
            },
        );
        assert_eq!(approvals.len(), 2, "both grant + denial recorded");

        let provider = query_ledger_in(
            root,
            &LedgerFilter {
                kind: Some(EventKind::ProviderCall),
                ..Default::default()
            },
        );
        assert_eq!(provider.len(), 1);
        assert_eq!(provider[0].subject, "opus");

        let cancels = query_ledger_in(
            root,
            &LedgerFilter {
                kind: Some(EventKind::Cancellation),
                ..Default::default()
            },
        );
        assert_eq!(cancels.len(), 1);
    }

    async fn git(args: &[&str], dir: &Path) {
        // Keep temp repos hermetic: a developer's *global* git config (a
        // `core.hooksPath` commit-msg linter, `commit.gpgsign`, etc.) must not
        // leak in and fail an otherwise-clean temp commit. Per-invocation `-c`
        // overrides take precedence over global/system config and are
        // cross-platform — empty `core.hooksPath` disables inherited hooks and
        // `*.gpgsign=false` disables inherited commit/tag signing.
        let ok = tokio::process::Command::new("git")
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
            .await
            .map(|o| o.status.success())
            .unwrap_or(false);
        assert!(ok, "git {args:?} failed in {}", dir.display());
    }

    async fn init_repo(dir: &Path) {
        // `git init` takes the path directly (no preceding -C into a possibly
        // missing dir).
        let ok = tokio::process::Command::new("git")
            .arg("init")
            .arg("-q")
            .arg(dir)
            .output()
            .await
            .map(|o| o.status.success())
            .unwrap_or(false);
        assert!(ok, "git init failed");
        git(&["config", "user.email", "t@t"], dir).await;
        git(&["config", "user.name", "t"], dir).await;
        std::fs::write(dir.join("seed.txt"), "seed\n").unwrap();
        git(&["add", "."], dir).await;
        git(&["commit", "-q", "-m", "seed"], dir).await;
    }

    // Normal: open then finalize a dirty worktree yields a Ready change-set
    // with the changed file recorded. (Uses the repo root as the "worktree"
    // for simplicity — the diff logic is identical.)
    #[tokio::test]
    async fn open_then_finalize_dirty_is_ready_normal() {
        let dir = tempfile::TempDir::new().unwrap();
        let root = dir.path();
        init_repo(root).await;
        let wt = root.to_string_lossy().to_string();

        let id = open_for_worktree(root, &wt, "jfc/test", &ChangeOrigin::default())
            .await
            .expect("change-set opened");

        // Mutate the worktree so the diff is non-empty.
        std::fs::write(root.join("new.rs"), "fn x() {}\n").unwrap();
        git(&["add", "."], root).await;

        finalize_for_worktree(root, &id, &wt).await;

        let store = ChangeStore::open_project(root).unwrap();
        let cs = store.get(&id).expect("persisted");
        assert_eq!(cs.state, ChangeState::Ready);
        assert!(
            cs.changed_files.iter().any(|f| f.path == "new.rs"),
            "new.rs should be in the diff: {:?}",
            cs.changed_files
        );
    }

    // Robust: a clean worktree (no diff vs base) is Abandoned, not left as a
    // dangling reviewable Draft.
    #[tokio::test]
    async fn finalize_clean_worktree_is_abandoned_robust() {
        let dir = tempfile::TempDir::new().unwrap();
        let root = dir.path();
        init_repo(root).await;
        let wt = root.to_string_lossy().to_string();

        let id = open_for_worktree(root, &wt, "jfc/clean", &ChangeOrigin::default())
            .await
            .expect("change-set opened");
        // No mutation → clean.
        finalize_for_worktree(root, &id, &wt).await;

        let store = ChangeStore::open_project(root).unwrap();
        assert_eq!(store.get(&id).unwrap().state, ChangeState::Abandoned);
    }

    /// Build an Approved change-set whose `branch` is a REAL git branch
    /// carrying one extra commit (a new file), so `apply_change` has something
    /// to merge. Returns the change-set id and the file the branch adds.
    async fn approved_change_with_real_branch(
        root: &Path,
    ) -> (String, jfc_changeset::AgentChangeSet) {
        init_repo(root).await;
        // Create a feature branch with a distinct commit, then return to the
        // base branch so apply's `git merge <branch>` has a real merge to do.
        let base = {
            let out = tokio::process::Command::new("git")
                .arg("-C")
                .arg(root)
                .args(["rev-parse", "--abbrev-ref", "HEAD"])
                .output()
                .await
                .unwrap();
            String::from_utf8_lossy(&out.stdout).trim().to_string()
        };
        git(&["checkout", "-q", "-b", "jfc/feature"], root).await;
        std::fs::write(root.join("feature.txt"), "from the agent branch\n").unwrap();
        git(&["add", "."], root).await;
        git(&["commit", "-q", "-m", "agent change"], root).await;
        git(&["checkout", "-q", &base], root).await;

        // Persist an Approved change-set pointing at that branch.
        let mut store = ChangeStore::open_project(root).unwrap();
        let mut cs = jfc_changeset::AgentChangeSet::open(
            "base",
            "jfc/feature",
            root.to_string_lossy().to_string(),
            1,
        );
        cs.mark_ready(Vec::new(), "feature.txt added", 2).unwrap();
        cs.record_test_run(
            jfc_changeset::TestRun {
                command: "true".into(),
                exit_code: 0,
                duration_ms: 1,
                finished_at_ms: 3,
            },
            3,
        )
        .unwrap();
        cs.approve(
            jfc_changeset::Approval::Human {
                user: "cole".into(),
                at_ms: 4,
            },
            4,
        )
        .unwrap();
        let id = cs.id.clone();
        store.upsert(cs.clone()).unwrap();
        (id, cs)
    }

    // Normal — the full apply→revert git round-trip (the gap that was only
    // unit-tested before). apply_change does a REAL `git merge` that lands the
    // branch's file in the base; revert_change does a REAL `git revert` that
    // removes it again. Both state transitions and git effects are asserted.
    #[tokio::test]
    #[serial_test::serial]
    async fn changeset_apply_revert_round_trip_normal() {
        crate::sandbox::reset_active_bash_sandbox_for_test();
        let dir = tempfile::TempDir::new().unwrap();
        let root = dir.path();
        let (id, _cs) = approved_change_with_real_branch(root).await;

        // The branch's file is NOT in the base checkout yet.
        assert!(
            !root.join("feature.txt").exists(),
            "feature.txt must not exist before apply"
        );

        // APPLY: real git merge. State Approved→Applied, file now in base.
        let apply_msg = apply_change(root, &id).await;
        assert!(apply_msg.contains("applied"), "apply said: {apply_msg}");
        assert!(
            root.join("feature.txt").exists(),
            "apply must merge feature.txt into the base checkout"
        );
        assert_eq!(
            ChangeStore::open_project(root)
                .unwrap()
                .get(&id)
                .unwrap()
                .state,
            ChangeState::Applied
        );

        // REVERT: real git revert of the merge. State Applied→Reverted, file
        // removed from the base checkout again.
        let revert_msg = revert_change(root, &id).await;
        assert!(revert_msg.contains("reverted"), "revert said: {revert_msg}");
        assert!(
            !root.join("feature.txt").exists(),
            "revert must undo the merge — feature.txt should be gone"
        );
        assert_eq!(
            ChangeStore::open_project(root)
                .unwrap()
                .get(&id)
                .unwrap()
                .state,
            ChangeState::Reverted
        );
    }

    // Robust: revert refuses a change that was never applied (only an Applied
    // change can be reverted) — no git is touched.
    #[tokio::test]
    #[serial_test::serial]
    async fn revert_refuses_unapplied_change_robust() {
        crate::sandbox::reset_active_bash_sandbox_for_test();
        let dir = tempfile::TempDir::new().unwrap();
        let root = dir.path();
        let (id, _cs) = approved_change_with_real_branch(root).await;
        // Approved but not applied → revert must refuse.
        let msg = revert_change(root, &id).await;
        assert!(
            msg.contains("only an Applied change can be reverted"),
            "revert said: {msg}"
        );
        assert_eq!(
            ChangeStore::open_project(root)
                .unwrap()
                .get(&id)
                .unwrap()
                .state,
            ChangeState::Approved,
            "state unchanged after refused revert"
        );
    }
}
