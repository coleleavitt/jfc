//! t444 — `jfc changes` CLI end-to-end test.
//!
//! Drives the actual compiled `jfc` binary (via `CARGO_BIN_EXE_jfc`) against a
//! throwaway git repo + seeded change-store, asserting the headless command
//! surface works from a clean process — the integration coverage Dolt-style CI
//! calls for. Hermetic: no network, temp dirs only.

use std::path::Path;
use std::process::Command;

use tempfile::TempDir;

fn jfc() -> Command {
    Command::new(env!("CARGO_BIN_EXE_jfc"))
}

fn git(args: &[&str], dir: &Path) {
    // Keep the repo truly hermetic: a developer's *global* git config (a
    // `core.hooksPath` commit-msg linter, `commit.gpgsign`, etc.) must not leak
    // in and fail an otherwise-clean temp commit. Per-invocation `-c` overrides
    // take precedence over global/system config and are cross-platform (no
    // `/dev/null` env tricks): empty `core.hooksPath` disables inherited hooks,
    // and `*.gpgsign=false` disables inherited commit/tag signing.
    let ok = Command::new("git")
        .args([
            "-c",
            "core.hooksPath=",
            "-c",
            "commit.gpgsign=false",
            "-c",
            "tag.gpgsign=false",
        ])
        .args(args)
        .current_dir(dir)
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    assert!(ok, "git {args:?} failed");
}

fn init_repo(dir: &Path) {
    git(&["init", "-q"], dir);
    git(&["config", "user.email", "t@t"], dir);
    git(&["config", "user.name", "t"], dir);
    std::fs::write(dir.join("seed.txt"), "seed\n").unwrap();
    git(&["add", "."], dir);
    git(&["commit", "-q", "-m", "seed"], dir);
}

/// Seed one change-set row directly into the JSONL store the CLI reads.
fn seed_change(dir: &Path, id: &str, state: &str, branch: &str) {
    let cdir = dir.join(".jfc").join("changes");
    std::fs::create_dir_all(&cdir).unwrap();
    let row = format!(
        "{{\"id\":\"{id}\",\"state\":\"{state}\",\"task_id\":\"t1\",\"agent_id\":\"task\",\
         \"session_id\":null,\"base_head\":\"deadbeef\",\"branch\":\"{branch}\",\
         \"worktree_path\":\"/tmp/wt\",\"changed_files\":[{{\"path\":\"a.rs\",\
         \"insertions\":5,\"deletions\":1}}],\"diff_summary\":\"1 file changed\",\
         \"patch_path\":null,\"ledger_refs\":[],\"test_runs\":[],\"approval\":null,\
         \"created_at_ms\":1,\"updated_at_ms\":2}}\n"
    );
    std::fs::write(cdir.join("changes.jsonl"), row).unwrap();
}

// Normal: `jfc changes list` prints a seeded change-set's id + state.
#[test]
fn changes_list_shows_seeded_change_normal() {
    let dir = TempDir::new().unwrap();
    init_repo(dir.path());
    seed_change(dir.path(), "abc1234567890def", "Ready", "jfc/agent-x");

    let out = jfc()
        .arg("changes")
        .arg("list")
        .current_dir(dir.path())
        .output()
        .expect("run jfc changes list");

    assert!(out.status.success(), "exit: {:?}", out.status);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("abc1234567890def"), "stdout: {stdout}");
    assert!(stdout.contains("ready"), "stdout: {stdout}");
    assert!(stdout.contains("jfc/agent-x"), "stdout: {stdout}");
}

// Normal: `jfc changes show <id>` prints full detail.
#[test]
fn changes_show_prints_detail_normal() {
    let dir = TempDir::new().unwrap();
    init_repo(dir.path());
    seed_change(dir.path(), "feed0000beef1234", "Ready", "jfc/agent-y");

    let out = jfc()
        .args(["changes", "show", "feed0000beef1234"])
        .current_dir(dir.path())
        .output()
        .expect("run jfc changes show");

    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("branch:   jfc/agent-y"), "stdout: {stdout}");
    assert!(stdout.contains("a.rs (+5 -1)"), "stdout: {stdout}");
}

// Robust: an empty store reports "No change-sets recorded." and exits 0.
#[test]
fn changes_list_empty_is_graceful_robust() {
    let dir = TempDir::new().unwrap();
    init_repo(dir.path());

    let out = jfc()
        .args(["changes", "list"])
        .current_dir(dir.path())
        .output()
        .expect("run jfc changes list");

    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("No change-sets recorded."),
        "stdout: {stdout}"
    );
}

// Robust: applying a non-Approved change is refused (the review/test gate),
// the process still exits 0, and the message explains why.
#[test]
fn changes_apply_unapproved_is_refused_robust() {
    let dir = TempDir::new().unwrap();
    init_repo(dir.path());
    seed_change(dir.path(), "0000aaaa1111bbbb", "Ready", "jfc/agent-z");

    let out = jfc()
        .args(["changes", "apply", "0000aaaa1111bbbb"])
        .current_dir(dir.path())
        .output()
        .expect("run jfc changes apply");

    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("only an Approved change can be applied"),
        "stdout: {stdout}"
    );
}
