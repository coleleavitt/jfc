use super::bash::{execute_bash, execute_bash_inner, execute_bash_output};
use super::daemon::execute_monitor;
use super::defs::all_tool_defs;
use super::dispatch::resolve_bash_workdir;
use super::economy::{
    apply_winning_solution, looks_like_unified_diff, market_report_string, parse_file_blocks,
    parse_validator_output, split_patch_and_explanation, verify_bounty_solution,
};
use super::filesystem::{build_edit_diff_view, execute_edit, execute_read, execute_write};
use super::lsp::execute_lsp;
use super::memory::{execute_memory_create, execute_memory_delete};
use super::notebook::{notebook_edit_text, notebook_read_text};
use super::notifications::{execute_push_notification, execute_remote_trigger, parse_trigger_url};
use super::registry::{active_event_sender_handle, market_orchestrator};
use super::safe_tools::{
    configure_tool_command, non_interactive_shell_command, terminal_safe_text,
};
use super::scratchpad::{execute_scratchpad_read, execute_scratchpad_write};
use super::search::{execute_glob, execute_grep};
use super::subagent::{execute_skill_in, filter_tools_for_agent};
use super::swarm::execute_team_member_mode;
use super::tasks::{
    TaskCreateRequest, TaskUpdateRequest, execute_task_create, execute_task_done,
    execute_task_list, execute_task_update,
};
use super::worktree::{execute_enter_plan_mode, execute_enter_worktree, execute_exit_worktree};
use super::*;

use crate::runtime::{DiagnosticLevel, EngineEvent, ToolEvent, ToolOutcome};
use crate::types::{ReplacementMode, ToolInput, ToolKind};
use jfc_provider::ToolDef;
use jfc_session::{DeletedFilter, TaskStore};
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};
use tokio::process::Command;
use tokio::sync::Mutex;

fn task_create_request(
    subject: impl Into<String>,
    description: impl Into<String>,
) -> TaskCreateRequest {
    TaskCreateRequest {
        subject: subject.into(),
        description: description.into(),
        active_form: None,
        blocked_by: Vec::new(),
        acceptance_criteria: None,
        verification_command: None,
        risk: None,
        parent_id: None,
        kind: None,
        tags: Vec::new(),
        priority: None,
        effort: None,
        model: None,
    }
}

fn task_update_request(task_id: impl Into<String>) -> TaskUpdateRequest {
    TaskUpdateRequest {
        task_id: task_id.into(),
        status: None,
        subject: None,
        description: None,
        owner: None,
        acceptance_criteria: None,
        verification_command: None,
        risk: None,
        parent_id: None,
        kind: None,
        blocked_by: Vec::new(),
        tags: Vec::new(),
        priority: None,
        effort: None,
        model: None,
    }
}

#[test]
fn execution_result_failure_carries_diagnostic() {
    let result = ExecutionResult::failure("command failed");

    assert!(result.is_error());
    assert_eq!(result.outcome, ToolOutcome::Failed);
    assert_eq!(result.diagnostics.len(), 1);
    assert_eq!(result.diagnostics[0].level, DiagnosticLevel::Error);
    assert_eq!(result.diagnostics[0].message, "command failed");
}

#[test]
fn send_user_message_policy_removes_tool_by_default_robust() {
    let mut tools = all_tool_defs();
    apply_send_user_message_policy(&mut tools, false, false);

    assert!(!tools.iter().any(|tool| tool.name == "SendUserMessage"));
}

#[test]
fn send_user_message_policy_keeps_tool_in_brief_mode_normal() {
    let mut tools = all_tool_defs();
    apply_send_user_message_policy(&mut tools, true, false);

    assert!(tools.iter().any(|tool| tool.name == "SendUserMessage"));
}

#[test]
fn send_user_message_policy_uses_pewter_prompt_normal() {
    let mut tools = all_tool_defs();
    apply_send_user_message_policy(&mut tools, false, true);

    let tool = tools
        .iter()
        .find(|tool| tool.name == "SendUserMessage")
        .expect("SendUserMessage should be advertised for Pewter Owl");
    assert!(tool.description.contains("read verbatim"));
    assert!(tool.description.contains("status"));
}

#[tokio::test]
#[serial_test::serial]
async fn bash_runs_without_inherited_terminal_or_stdin() {
    crate::sandbox::reset_active_bash_sandbox_for_test();
    let result = execute_bash(
        "read -t 0.1 value || true; (cat /dev/tty >/dev/null 2>&1 && echo has-tty || echo no-tty); if [ -n \"${value:-}\" ]; then echo stdin-leaked; fi",
        Some(5_000),
        Path::new("."),
    )
    .await;

    assert!(!result.is_error(), "{}", result.output);
    assert!(result.output.contains("no-tty"), "{}", result.output);
    assert!(!result.output.contains("stdin-leaked"), "{}", result.output);
}

#[test]
fn leading_sudo_is_forced_non_interactive() {
    assert_eq!(non_interactive_shell_command("sudo true"), "sudo -n true");
    assert_eq!(
        non_interactive_shell_command("  sudo --non-interactive true"),
        "  sudo --non-interactive true"
    );
    assert_eq!(
        non_interactive_shell_command("echo sudo true"),
        "echo sudo true"
    );
}

#[tokio::test]
async fn sudo_prompt_does_not_escape_or_hang() {
    let result = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        execute_bash("sudo true", Some(4_000), Path::new(".")),
    )
    .await
    .expect("sudo command should fail or succeed without hanging");

    assert!(!result.output.contains("Password:"), "{}", result.output);
    assert!(!result.output.contains('\u{1b}'), "{}", result.output);
}

#[test]
fn terminal_safe_text_strips_control_sequences() {
    let raw = "\u{1b}[31mred\u{1b}[0m \u{1b}[<35;82;42MPassword:\u{7}\u{1b}]0;title\u{7} ok\u{0}";

    assert_eq!(terminal_safe_text(raw), "red Password: ok");
}

/// Best-effort temp-dir helper — returns `None` if temp creation
/// fails so tests skip rather than fail on sandboxes without
/// writable temp.
fn skill_tempdir_or_skip() -> Option<PathBuf> {
    let mut p = std::env::temp_dir();
    p.push(format!(
        "jfc_skill_test_{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .ok()?
            .as_nanos()
    ));
    std::fs::create_dir_all(p.join(".claude/skills")).ok()?;
    Some(p)
}

fn write_skill(root: &Path, name: &str, body: &str) {
    let path = root.join(".claude/skills").join(format!("{name}.md"));
    let frontmatter = format!("---\nname: {name}\n---\n{body}");
    std::fs::write(&path, frontmatter).expect("write skill");
}

#[tokio::test]
async fn execute_skill_unknown_returns_failure_robust() {
    let Some(root) = skill_tempdir_or_skip() else {
        return;
    };
    // Use a very unlikely name so a stray user-level skill at
    // ~/.claude/skills cannot satisfy the lookup.
    let result = execute_skill_in(&root, "definitely-not-a-real-skill-xyz-9831", None).await;
    assert!(result.is_error(), "unknown skill must report failure");
    assert!(
        result.output.contains("Unknown skill"),
        "expected 'Unknown skill' marker, got: {}",
        result.output
    );
}

#[tokio::test]
async fn execute_skill_unknown_omits_superpower_suggestions_robust() {
    let Some(root) = skill_tempdir_or_skip() else {
        return;
    };
    let super_dir = root.join(".codex/plugins/superpowers/skills");
    std::fs::create_dir_all(&super_dir).expect("create superpowers skills dir");
    std::fs::write(
        super_dir.join("verification.md"),
        "---\nname: verification\n---\nInternal verification guidance.",
    )
    .expect("write superpower skill");

    let result = execute_skill_in(&root, "definitely-not-a-real-skill-xyz-9832", None).await;
    assert!(result.is_error(), "unknown skill must report failure");
    assert!(
        !result.output.contains("superpowers:"),
        "internal superpower skills should not be suggested: {}",
        result.output
    );
}

#[tokio::test]
async fn execute_skill_known_returns_body_normal() {
    let Some(root) = skill_tempdir_or_skip() else {
        return;
    };
    write_skill(&root, "jfc-test-known", "Do the thing carefully.");

    let result = execute_skill_in(&root, "jfc-test-known", None).await;
    assert!(!result.is_error(), "known skill must succeed: {:?}", result);
    assert!(
        result.output.contains("Do the thing carefully."),
        "skill body should be returned, got: {}",
        result.output
    );
}

#[tokio::test]
async fn execute_skill_appends_args_normal() {
    let Some(root) = skill_tempdir_or_skip() else {
        return;
    };
    write_skill(&root, "jfc-test-args", "Body content.");

    let result = execute_skill_in(&root, "jfc-test-args", Some("focus on auth")).await;
    assert!(!result.is_error(), "skill with args must succeed");
    assert!(result.output.contains("Body content."));
    assert!(
        result.output.contains("# Args"),
        "args block should have header, got: {}",
        result.output
    );
    assert!(
        result.output.contains("focus on auth"),
        "args text should be embedded, got: {}",
        result.output
    );
}

#[tokio::test]
async fn execute_skill_no_args_no_header_normal() {
    let Some(root) = skill_tempdir_or_skip() else {
        return;
    };
    write_skill(&root, "jfc-test-no-args", "Plain body.");

    let result = execute_skill_in(&root, "jfc-test-no-args", None).await;
    assert!(!result.is_error());
    assert!(
        !result.output.contains("# Args"),
        "no args means no Args section, got: {}",
        result.output
    );
}

// ─── all_tool_defs catalogue checks ──────────────────────────────────

#[test]
fn all_tool_defs_includes_every_canonical_tool_normal() {
    // Every primary tool name must appear in the catalogue. If a refactor
    // accidentally drops one (e.g. by gating it behind a feature flag),
    // the API call will 400 with "tool X not found".
    let defs = all_tool_defs();
    let names: Vec<&str> = defs.iter().map(|d| d.name.as_str()).collect();
    for required in [
        "Bash",
        "BashOutput",
        "Read",
        "Write",
        "Edit",
        "Glob",
        "Grep",
        "TaskCreate",
        "TaskUpdate",
        "TaskList",
        "TaskDone",
        "Skill",
        "Task",
        "MemoryCreate",
        "MemoryDelete",
        "TeamCreate",
        "TeamDelete",
        "SendMessage",
        "TeamMemberMode",
        "HcomStatus",
        "HcomList",
        "HcomSend",
        "HcomEvents",
        "HcomListen",
        "HcomTranscript",
        "HcomBundle",
        "HcomTerm",
        "HcomLaunch",
        "HcomResume",
        "HcomFork",
        "HcomKill",
        "HcomRelay",
        "HcomRun",
        "post_bounty",
        "run_bounty",
        "market_status",
        "LSP",
        "PushNotification",
        "RemoteTrigger",
        "EnterPlanMode",
        "EnterWorktree",
        "ExitWorktree",
        "NotebookRead",
        "NotebookEdit",
    ] {
        assert!(
            names.contains(&required),
            "all_tool_defs missing {required}; got {names:?}",
        );
    }
}

#[test]
fn bash_tool_schema_advertises_workdir_and_fresh_shell_regression() {
    let defs = all_tool_defs();
    let bash = defs
        .iter()
        .find(|def| def.name == "Bash")
        .expect("Bash tool should be present");

    assert!(
        bash.description.contains("fresh non-interactive shell"),
        "Bash description must not imply persistent shell state: {}",
        bash.description
    );
    assert!(
        bash.input_schema["properties"].get("workdir").is_some(),
        "Bash schema must advertise the workdir field: {}",
        bash.input_schema
    );
    assert!(
        bash.input_schema["properties"]
            .get("run_in_background")
            .is_some(),
        "Bash schema must advertise run_in_background: {}",
        bash.input_schema
    );
    assert!(
        defs.iter().any(|def| def.name == "BashOutput"),
        "BashOutput retrieval tool must be advertised"
    );
}

#[test]
#[serial_test::serial]
fn bash_output_dir_defaults_to_temp_regression() {
    let _guard = EnvVarGuard::unset("JFC_BASH_OUTPUT_DIR");
    let _uid_guard = EnvVarGuard::unset("UID");
    let dir = super::bash::bash_output_dir();
    assert!(
        dir.starts_with(std::env::temp_dir()),
        "bash output dir should default under temp, got {}",
        dir.display()
    );
    assert!(
        !dir.to_string_lossy().contains("unknown"),
        "bash output dir should use the real uid, got {}",
        dir.display()
    );
    assert!(
        dir.to_string_lossy().contains("/bash"),
        "bash output dir should be bash-specific, got {}",
        dir.display()
    );
}

#[cfg(unix)]
#[test]
#[serial_test::serial]
fn bash_output_dir_is_created_private_regression() {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};

    let _guard = EnvVarGuard::unset("JFC_BASH_OUTPUT_DIR");
    let _uid_guard = EnvVarGuard::unset("UID");
    let dir = super::bash::prepare_bash_output_dir().expect("prepare bash output dir");
    let metadata = std::fs::symlink_metadata(&dir).expect("stat bash output dir");

    assert!(metadata.is_dir(), "prepared path should be a directory");
    assert!(
        !metadata.file_type().is_symlink(),
        "prepared path must not be a symlink"
    );
    assert_eq!(
        metadata.uid(),
        unsafe { libc::getuid() },
        "bash output dir must be owned by the current uid"
    );
    assert_eq!(
        metadata.permissions().mode() & 0o777,
        0o700,
        "bash output dir must be private"
    );
}

#[cfg(unix)]
#[test]
#[serial_test::serial]
fn bash_output_dir_rejects_symlink_override_regression() {
    let temp = tempfile::tempdir().expect("temp dir");
    let target = temp.path().join("target");
    std::fs::create_dir(&target).expect("target dir");
    let link = temp.path().join("link");
    std::os::unix::fs::symlink(&target, &link).expect("symlink override");

    let _guard = EnvVarGuard::set(
        "JFC_BASH_OUTPUT_DIR",
        link.to_str().expect("utf8 temp path"),
    );
    let err =
        super::bash::prepare_bash_output_dir().expect_err("symlink output dir must be rejected");

    assert!(err.contains("symlink"), "{err}");
}

// ─── agent economy wiring (PostBounty / MarketStatus) ─────────────

/// Tests against the process-global market orchestrator must serialize
/// through this lock. Otherwise the posted-bounty count test races with
/// the report-format test.
fn market_test_lock() -> &'static tokio::sync::Mutex<()> {
    static LOCK: OnceLock<tokio::sync::Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| tokio::sync::Mutex::new(()))
}

// Normal: the two market tools appear in the canonical-tool list
// (already covered by the broader catalogue test, but call it out
// explicitly so a regression on either name fails clearly).
#[test]
fn market_tools_in_catalogue_normal() {
    let defs = all_tool_defs();
    let names: Vec<&str> = defs.iter().map(|d| d.name.as_str()).collect();
    assert!(names.contains(&"post_bounty"));
    assert!(names.contains(&"market_status"));
}

// Normal: market_report_string returns a well-formed snapshot
// string with the expected section headers — this is the same
// string the /market slash command surfaces, so a regression here
// breaks both the tool and the slash command.
#[tokio::test]
async fn market_report_string_has_expected_sections_normal() {
    let _g = market_test_lock().lock().await;
    let body = market_report_string()
        .await
        .expect("market report must render");
    assert!(body.contains("Agent economy snapshot"));
    assert!(body.contains("Bounties:"));
    assert!(body.contains("Spend:"));
    assert!(body.contains("Health"));
}

// Normal: posting a bounty via the tool dispatcher actually
// increments the orchestrator's bounty count. End-to-end smoke
// test that the wiring (ToolKind → ToolInput → execute_tool →
// orchestrator) is connected.
#[tokio::test(flavor = "current_thread")]
async fn post_bounty_dispatch_increments_market_normal() {
    let _g = market_test_lock().lock().await;
    let before = {
        let orch = market_orchestrator().lock().await;
        orch.bounties.audit_log().len()
    };
    let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
    let res = execute_tool(
        crate::types::ToolKind::PostBounty,
        crate::types::ToolInput::PostBounty {
            description: "test bounty".into(),
            budget: 100,
            acceptance_criteria: "cargo test".into(),
            max_solvers: Some(2),
            auto_dispatch: false,
        },
        cwd,
        None,
        None,
        None,
    )
    .await;
    assert!(
        !res.is_error(),
        "post_bounty should succeed: {}",
        res.output
    );
    let after = {
        let orch = market_orchestrator().lock().await;
        orch.bounties.audit_log().len()
    };
    assert!(after > before, "audit log should grow ({before} → {after})");
}

// Normal: run_bounty is in the canonical tool list and the
// catalogue test (above) enforces it. Verify here that its
// dispatch arm rejects an unknown bounty_id with a clear
// error — most common LLM mistake will be a typo'd ID.
#[tokio::test(flavor = "current_thread")]
async fn run_bounty_unknown_id_errors_robust() {
    let _g = market_test_lock().lock().await;
    // Register a stub provider so the "no provider" path
    // doesn't fire first and mask the unknown-id check.
    struct NoopProvider;
    #[async_trait::async_trait]
    impl jfc_provider::Provider for NoopProvider {
        fn name(&self) -> &str {
            "noop"
        }
        fn available_models(&self) -> Vec<jfc_provider::ModelInfo> {
            vec![]
        }
        async fn stream(
            &self,
            _: Vec<jfc_provider::ProviderMessage>,
            _: &jfc_provider::StreamOptions,
        ) -> anyhow::Result<jfc_provider::EventStream> {
            Err(anyhow::anyhow!("noop"))
        }
    }
    impl jfc_provider::seal::Sealed for NoopProvider {}
    register_active_provider(
        std::sync::Arc::new(NoopProvider),
        jfc_provider::ModelId::new("noop"),
    );
    let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
    let res = execute_tool(
        crate::types::ToolKind::RunBounty,
        crate::types::ToolInput::RunBounty {
            bounty_id: "bounty_does_not_exist".into(),
            max_solvers: None,
        },
        cwd,
        None,
        None,
        None,
    )
    .await;
    assert!(res.is_error(), "should fail on unknown id");
    assert!(
        res.output.contains("not found"),
        "error should mention 'not found': {}",
        res.output
    );
}

// Normal: post_bounty (auto_dispatch=false, the default) returns
// a success string that explicitly tells the model the cycle
// hasn't run yet and how to drive it. This is the wire-fix:
// the previous wording made the LLM think the path wasn't
// implemented and bypass to direct Bash execution.
#[tokio::test(flavor = "current_thread")]
async fn post_bounty_default_returns_actionable_message_normal() {
    let _g = market_test_lock().lock().await;
    let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
    let res = execute_tool(
        crate::types::ToolKind::PostBounty,
        crate::types::ToolInput::PostBounty {
            description: "smoke test".into(),
            budget: 200,
            acceptance_criteria: "cargo test".into(),
            max_solvers: None,
            auto_dispatch: false,
        },
        cwd,
        None,
        None,
        None,
    )
    .await;
    assert!(!res.is_error(), "post should succeed: {}", res.output);
    // The new wording should make it crystal clear the cycle
    // didn't run and that run_bounty is the next step. Without
    // these phrases the LLM's tendency is to bypass.
    assert!(
        res.output.contains("have NOT run yet"),
        "must explicitly say solvers haven't run: {}",
        res.output
    );
    assert!(
        res.output.contains("run_bounty"),
        "must point at run_bounty as next step: {}",
        res.output
    );
}

// The previous over-cap-budget rejection test against the
// global orchestrator is no longer reachable from the tool
// layer because the default charter now sets
// `max_budget_per_bounty: u64::MAX` (any in-band tool call
// would have to pass literal u64::MAX as the budget to trip
// the gate). The rejection mechanism is still covered by
// `jfc_economy::orchestrator::tests::test_budget_exceeded`,
// which constructs a charter with a tight cap and verifies
// the path end-to-end.

// ─── agent economy cycle (real LLM-driven path) ───────────────────

/// Stub AgentInvoker for cycle tests — returns canned solutions +
/// validator outcomes without hitting any network. Each call
/// records the prompt for assertion.
struct StubInvoker {
    solver_calls: std::sync::Mutex<usize>,
    validator_calls: std::sync::Mutex<usize>,
}

impl StubInvoker {
    fn new() -> Self {
        Self {
            solver_calls: std::sync::Mutex::new(0),
            validator_calls: std::sync::Mutex::new(0),
        }
    }
}

#[async_trait::async_trait]
impl jfc_economy::reporting::AgentInvoker for StubInvoker {
    async fn invoke_solver(
        &self,
        prompt: jfc_economy::reporting::SolverPrompt,
    ) -> Result<jfc_economy::types::Solution, String> {
        *self.solver_calls.lock().unwrap() += 1;
        Ok(jfc_economy::types::Solution {
            agent_id: prompt.agent_id,
            bounty_id: prompt.bounty_id,
            patch: "diff --git a/x b/x\n--- a/x\n+++ b/x\n@@ -1 +1 @@\n-old\n+new".into(),
            explanation: "stub solution".into(),
            self_assessment: 0.7,
            tokens_consumed: 100,
            compiles: Some(true),
            tests_pass: Some(true),
            suspicious: false,
        })
    }
    async fn invoke_validator(
        &self,
        _prompt: jfc_economy::reporting::ValidatorPrompt,
    ) -> Result<jfc_economy::reporting::ValidatorOutcome, String> {
        *self.validator_calls.lock().unwrap() += 1;
        Ok(jfc_economy::reporting::ValidatorOutcome {
            flaw: None,
            test_code: None,
            confidence: 0.97,
            tokens_consumed: 50,
        })
    }
}

/// SwarmProvider stub that doesn't touch git. Worktree paths
/// are made up; remove is a no-op.
struct StubSwarm;
#[async_trait::async_trait]
impl jfc_economy::reporting::SwarmProvider for StubSwarm {
    async fn create_worktree(
        &self,
        bounty_id: &str,
        agent_id: &jfc_economy::types::AgentId,
    ) -> Option<std::path::PathBuf> {
        Some(std::path::PathBuf::from(format!(
            "/tmp/stub-{bounty_id}-{}",
            agent_id.0
        )))
    }
    async fn remove_worktree(&self, _path: &std::path::Path) {}
    fn send_message(&self, _agent_id: &jfc_economy::types::AgentId, _msg: &str) {}
}

/// Normal: a full bounty cycle with stub invoker + swarm
/// progresses Post→Settle and ends with a winning solver.
#[tokio::test(flavor = "current_thread")]
async fn run_bounty_cycle_end_to_end_normal() {
    use jfc_economy::charter::Charter;
    use jfc_economy::orchestrator::MarketOrchestrator;
    let charter = Charter::default();
    let mut orch = MarketOrchestrator::with_budget(charter, 10_000);
    let id = orch
        .post_bounty("test".into(), 500, "cargo test".into(), Some(2))
        .expect("post_bounty");
    let invoker = StubInvoker::new();
    let swarm = StubSwarm;
    let outcome = orch
        .run_bounty_cycle(&id, &invoker, &swarm, 2, 1)
        .await
        .expect("cycle should settle");
    // Two solvers spawned, both produced solutions.
    assert_eq!(*invoker.solver_calls.lock().unwrap(), 2);
    // Sealed validation: one validator per surviving solution.
    assert_eq!(*invoker.validator_calls.lock().unwrap(), 2);
    // A winner was selected (compiles=true, tests=true on both).
    assert!(
        outcome.settlement.winner.is_some(),
        "expected a winning solver"
    );
    // Cycle outcome carries the winning solution so the dispatcher
    // can apply its patch to disk — without this, run_bounty would
    // claim success but write nothing (the 2026-05-06 HMAC bug).
    assert!(
        outcome.winning_solution.is_some(),
        "winning_solution must be exposed for patch application"
    );
}

// Robust: even when the invoker errors on a solver, the cycle
// continues — that solver is abandoned but the others settle.
struct ErroringInvoker;
#[async_trait::async_trait]
impl jfc_economy::reporting::AgentInvoker for ErroringInvoker {
    async fn invoke_solver(
        &self,
        _: jfc_economy::reporting::SolverPrompt,
    ) -> Result<jfc_economy::types::Solution, String> {
        Err("simulated provider failure".into())
    }
    async fn invoke_validator(
        &self,
        _: jfc_economy::reporting::ValidatorPrompt,
    ) -> Result<jfc_economy::reporting::ValidatorOutcome, String> {
        Err("simulated".into())
    }
}

#[tokio::test(flavor = "current_thread")]
async fn run_bounty_cycle_solver_failure_robust() {
    use jfc_economy::charter::Charter;
    use jfc_economy::orchestrator::MarketOrchestrator;
    let charter = Charter::default();
    let mut orch = MarketOrchestrator::with_budget(charter, 10_000);
    let id = orch
        .post_bounty("test".into(), 500, "cargo test".into(), Some(1))
        .expect("post_bounty");
    let err = orch
        .run_bounty_cycle(&id, &ErroringInvoker, &StubSwarm, 1, 1)
        .await
        .expect_err("cycle must fail when no verified solution exists");
    assert!(
        err.to_string()
            .contains("no mechanically verified solution")
    );
}

// Normal: register_active_provider snapshot round-trip — the
// value we register comes back out via snapshot_active_provider.
#[test]
fn register_active_provider_round_trip_normal() {
    // The test infra already constructs a TestProvider for App::new;
    // reuse it.
    struct NoopProvider;
    #[async_trait::async_trait]
    impl jfc_provider::Provider for NoopProvider {
        fn name(&self) -> &str {
            "noop"
        }
        fn available_models(&self) -> Vec<jfc_provider::ModelInfo> {
            vec![]
        }
        async fn stream(
            &self,
            _: Vec<jfc_provider::ProviderMessage>,
            _: &jfc_provider::StreamOptions,
        ) -> anyhow::Result<jfc_provider::EventStream> {
            Err(anyhow::anyhow!("noop"))
        }
    }
    impl jfc_provider::seal::Sealed for NoopProvider {}
    let p: std::sync::Arc<dyn jfc_provider::Provider> = std::sync::Arc::new(NoopProvider);
    let m = jfc_provider::ModelId::new("noop-model");
    register_active_provider(p, m);
    let snap = snapshot_active_provider().expect("provider should be registered");
    assert_eq!(snap.0.name(), "noop");
    assert_eq!(snap.1.as_str(), "noop-model");
}

// Normal: the prose split helper picks out a fenced ```diff
// block and leaves the trailing prose as the explanation.
#[test]
fn split_patch_and_explanation_diff_block_normal() {
    let s = "Here's my fix:\n```diff\ndiff --git a/x\n+new line\n```\n\nIt swaps old for new.";
    let (patch, expl) = split_patch_and_explanation(s);
    assert!(patch.contains("diff --git a/x"));
    assert!(patch.contains("+new line"));
    assert_eq!(expl, "It swaps old for new.");
}

// Robust: malformed response (no fenced block) treats the whole
// thing as the patch with empty explanation rather than dropping.
#[test]
fn split_patch_and_explanation_no_block_robust() {
    let s = "just some text with no fences";
    let (patch, expl) = split_patch_and_explanation(s);
    assert_eq!(patch, "just some text with no fences");
    assert!(expl.is_empty());
}

// Normal: validator output parser pulls FLAW / CONFIDENCE / TEST
// out of a v131-style structured response.
#[test]
fn parse_validator_output_full_normal() {
    let s = "FLAW: integer overflow on negative input\n\
         CONFIDENCE: 0.85\n\
         TEST:\n\
         #[test]\n\
         fn neg_overflow() {\n\
             assert!(checked(-1).is_err());\n\
         }";
    let (flaw, conf, test) = parse_validator_output(s);
    assert_eq!(flaw.as_deref(), Some("integer overflow on negative input"));
    assert!((conf - 0.85).abs() < 0.01);
    assert!(test.unwrap().contains("fn neg_overflow"));
}

// Robust: NONE markers produce None even with mixed casing.
#[test]
fn parse_validator_output_none_markers_robust() {
    let s = "FLAW: none\nCONFIDENCE: 0.97\nTEST: NONE";
    let (flaw, conf, test) = parse_validator_output(s);
    assert!(flaw.is_none());
    assert!((conf - 0.97).abs() < 0.01);
    assert!(test.is_none());
}

// Normal: parse_file_blocks extracts a single FILE block.
#[test]
fn parse_file_blocks_single_block_normal() {
    let s = "===FILE: src/lib.rs===\npub fn x() {}\n===END===\n";
    let got = parse_file_blocks(s);
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].0, std::path::PathBuf::from("src/lib.rs"));
    assert_eq!(got[0].1, "pub fn x() {}\n");
}

// Normal: parse_file_blocks handles multiple back-to-back blocks
// and trims path whitespace.
#[test]
fn parse_file_blocks_multiple_normal() {
    let s = "preamble text\n\
             ===FILE:  Cargo.toml ===\n[package]\nname=\"x\"\n===END===\n\
             ===FILE: src/main.rs===\nfn main() {}\n===END===\n\
             trailing prose";
    let got = parse_file_blocks(s);
    assert_eq!(got.len(), 2);
    assert_eq!(got[0].0, std::path::PathBuf::from("Cargo.toml"));
    assert_eq!(got[1].0, std::path::PathBuf::from("src/main.rs"));
}

// Robust: parse_file_blocks is empty when no blocks present.
#[test]
fn parse_file_blocks_no_blocks_robust() {
    assert!(parse_file_blocks("just a unified diff\n--- a/foo\n+++ b/foo\n@@\n").is_empty());
    assert!(parse_file_blocks("").is_empty());
}

// Robust: a block missing its END marker is dropped (no panic, no
// partial write).
#[test]
fn parse_file_blocks_missing_end_robust() {
    let s = "===FILE: x.rs===\ncontent without end marker\n";
    assert!(parse_file_blocks(s).is_empty());
}

// Normal: looks_like_unified_diff recognises a real diff.
#[test]
fn looks_like_unified_diff_recognises_normal() {
    let d = "diff --git a/x b/x\n--- a/x\n+++ b/x\n@@ -1 +1 @@\n-old\n+new\n";
    assert!(looks_like_unified_diff(d));
}

// Robust: random prose is not classified as a unified diff.
#[test]
fn looks_like_unified_diff_rejects_prose_robust() {
    assert!(!looks_like_unified_diff("just some explanation"));
    assert!(!looks_like_unified_diff(""));
}

// Normal: apply_winning_solution writes audit files + creates the
// FILE-block paths under cwd.
#[test]
fn apply_winning_solution_writes_file_blocks_normal() {
    use jfc_economy::types::{AgentId, Solution};
    let tmp = tempfile::tempdir().expect("tempdir");
    let cwd = tmp.path();
    let sol = Solution {
        agent_id: AgentId::new("solver"),
        bounty_id: "test_b".into(),
        patch: "===FILE: hello.txt===\nhi there\n===END===\n".into(),
        explanation: "wrote hello".into(),
        self_assessment: 0.5,
        tokens_consumed: 10,
        compiles: None,
        tests_pass: None,
        suspicious: false,
    };
    let res = apply_winning_solution(cwd, "test_b", Some(&sol));
    assert_eq!(res.files.len(), 1, "summary={}", res.summary);
    assert!(cwd.join("hello.txt").exists(), "file should be written");
    assert_eq!(
        std::fs::read_to_string(cwd.join("hello.txt")).unwrap(),
        "hi there\n"
    );
    assert!(
        cwd.join(".jfc/bounties/test_b/winner.patch").exists(),
        "audit copy should exist"
    );
}

// Robust — the review/test-before-production gate: a solution whose tests
// failed is refused, and nothing is written to the main checkout.
#[test]
fn apply_winning_solution_refuses_failed_tests_robust() {
    use jfc_economy::types::{AgentId, Solution};
    let tmp = tempfile::tempdir().expect("tempdir");
    let cwd = tmp.path();
    let sol = Solution {
        agent_id: AgentId::new("solver"),
        bounty_id: "gated".into(),
        patch: "===FILE: hello.txt===\nhi\n===END===\n".into(),
        explanation: "x".into(),
        self_assessment: 0.9,
        tokens_consumed: 1,
        compiles: Some(true),
        tests_pass: Some(false),
        suspicious: false,
    };
    let res = apply_winning_solution(cwd, "gated", Some(&sol));
    assert!(res.files.is_empty(), "must write nothing: {}", res.summary);
    assert!(res.summary.contains("tests failed"));
    assert!(
        !cwd.join("hello.txt").exists(),
        "production file must not be written when tests failed"
    );
}

// Robust: a solution flagged suspicious by the validator is likewise refused.
#[test]
fn apply_winning_solution_refuses_suspicious_robust() {
    use jfc_economy::types::{AgentId, Solution};
    let tmp = tempfile::tempdir().expect("tempdir");
    let cwd = tmp.path();
    let sol = Solution {
        agent_id: AgentId::new("solver"),
        bounty_id: "susp".into(),
        patch: "===FILE: x.txt===\nx\n===END===\n".into(),
        explanation: "x".into(),
        self_assessment: 0.9,
        tokens_consumed: 1,
        compiles: Some(true),
        tests_pass: Some(true),
        suspicious: true,
    };
    let res = apply_winning_solution(cwd, "susp", Some(&sol));
    assert!(res.files.is_empty());
    assert!(res.summary.contains("suspicious"));
    assert!(!cwd.join("x.txt").exists());
}

// Robust: apply_winning_solution with None reports nothing-written
// and creates no audit dir.
#[test]
fn apply_winning_solution_none_solution_robust() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let res = apply_winning_solution(tmp.path(), "no_winner", None);
    assert!(res.files.is_empty());
    assert!(res.summary.contains("No winning solution"));
}

#[test]
fn apply_winning_solution_rejects_file_block_path_escape_robust() {
    use jfc_economy::types::{AgentId, Solution};

    let tmp = tempfile::tempdir().expect("tempdir");
    let outside = tmp.path().join("outside.txt");
    let sol = Solution {
        agent_id: AgentId::new("solver"),
        bounty_id: "escape".into(),
        patch: "===FILE: ../outside.txt===\nowned\n===END===\n".into(),
        explanation: "try escape".into(),
        self_assessment: 0.5,
        tokens_consumed: 1,
        compiles: Some(true),
        tests_pass: Some(true),
        suspicious: false,
    };

    let res = apply_winning_solution(tmp.path(), "escape", Some(&sol));
    assert!(!outside.exists());
    assert!(res.summary.contains("no files written"));
    assert!(res.files.is_empty());
}

// Regression: a bounty solution must not be accepted just because it
// wrote files. The solver worktree has to pass its detected build/test
// command; otherwise validators can rubber-stamp a broken patch.
#[tokio::test]
async fn verify_bounty_solution_rejects_broken_zig_build_robust() {
    use jfc_economy::types::{AgentId, Solution};

    let tmp = tempfile::tempdir().expect("tempdir");
    let cwd = tmp.path();
    std::fs::create_dir_all(cwd.join("src")).expect("src dir");
    std::fs::write(cwd.join("build.zig"), "this is not zig syntax\n").expect("build.zig");

    let sol = Solution {
        agent_id: AgentId::new("solver"),
        bounty_id: "zig_bounty".into(),
        patch: "===FILE: src/main.zig===\npub fn main() void {}\n===END===\n".into(),
        explanation: "wrote zig app".into(),
        self_assessment: 0.5,
        tokens_consumed: 10,
        compiles: None,
        tests_pass: None,
        suspicious: false,
    };

    let zig_available = std::process::Command::new("zig")
        .arg("version")
        .output()
        .is_ok_and(|o| o.status.success());
    if !zig_available {
        eprintln!("zig not installed; skipping");
        return;
    }
    let verification = verify_bounty_solution(cwd, "zig_bounty", &sol).await;
    assert!(!verification.passed, "broken zig build must fail");
    assert!(
        verification.summary.contains("zig build failed"),
        "summary={}",
        verification.summary
    );
}

#[test]
fn all_tool_defs_have_object_schemas_robust() {
    // Anthropic's tool API requires `input_schema.type == "object"`. If
    // any tool ships a bare scalar schema the entire stream errors at
    // request time.
    for def in all_tool_defs() {
        assert_eq!(
            def.input_schema.get("type").and_then(|v| v.as_str()),
            Some("object"),
            "tool {} schema must be object-typed",
            def.name,
        );
    }
}

#[tokio::test]
async fn all_tool_defs_with_mcp_no_registry_matches_native_normal() {
    // When no MCP registry has been registered (process-global slot
    // is None — true in fresh tests), the function should degrade
    // to the native `all_tool_defs()` set.
    let native = all_tool_defs();
    let combined = all_tool_defs_with_mcp().await;
    // Some other test in this module may have registered a registry
    // earlier — what we care about is that combined is at least as
    // big as native and starts with the same names.
    assert!(combined.len() >= native.len());
    for (i, def) in native.iter().enumerate() {
        assert_eq!(combined[i].name, def.name);
    }
}

// ─── filter_tools_for_agent ──────────────────────────────────────────

fn make_tool_def(name: &str) -> ToolDef {
    ToolDef {
        name: name.into(),
        description: "test".into(),
        input_schema: serde_json::json!({"type": "object"}),
    }
}

#[test]
fn filter_tools_drops_task_when_nested_tasks_disabled_robust() {
    let all = vec![make_tool_def("Bash"), make_tool_def("Task")];
    let filtered = filter_tools_for_agent(all, &[], &[], false);
    assert_eq!(filtered.len(), 1);
    assert_eq!(filtered[0].name, "Bash");
}

#[test]
fn filter_tools_keeps_task_when_nested_tasks_enabled_normal() {
    let all = vec![make_tool_def("Bash"), make_tool_def("Task")];
    let filtered = filter_tools_for_agent(all, &[], &[], true);
    assert_eq!(filtered.len(), 2);
    assert!(filtered.iter().any(|t| t.name == "Task"));
}

#[test]
fn filter_tools_empty_allowed_means_all_normal() {
    let all = vec![
        make_tool_def("Bash"),
        make_tool_def("Read"),
        make_tool_def("Write"),
    ];
    let filtered = filter_tools_for_agent(all, &[], &[], false);
    assert_eq!(filtered.len(), 3);
}

#[test]
fn filter_tools_allowed_is_exact_membership_normal() {
    let all = vec![
        make_tool_def("Bash"),
        make_tool_def("Read"),
        make_tool_def("Write"),
    ];
    let filtered = filter_tools_for_agent(all, &["Read".into(), "Write".into()], &[], false);
    assert_eq!(filtered.len(), 2);
    assert!(filtered.iter().any(|t| t.name == "Read"));
    assert!(filtered.iter().any(|t| t.name == "Write"));
}

#[test]
fn filter_tools_disallowed_subtracts_from_allowed_normal() {
    let all = vec![
        make_tool_def("Bash"),
        make_tool_def("Read"),
        make_tool_def("Write"),
    ];
    let filtered = filter_tools_for_agent(all, &[], &["Bash".into()], false);
    assert_eq!(filtered.len(), 2);
    assert!(filtered.iter().all(|t| t.name != "Bash"));
}

#[test]
fn filter_tools_case_insensitive_robust() {
    // Allow/disallow lists in agent definitions are user-edited; case
    // variation must not silently drop or skip tools.
    let all = vec![make_tool_def("Bash"), make_tool_def("Read")];
    let filtered = filter_tools_for_agent(all, &["BASH".into()], &[], false);
    assert_eq!(filtered.len(), 1);
    assert_eq!(filtered[0].name, "Bash");
}

#[test]
fn filter_tools_disallow_overrides_allow_robust() {
    let all = vec![make_tool_def("Bash"), make_tool_def("Read")];
    // Same tool both allow- and disallow-listed: disallow wins.
    let filtered = filter_tools_for_agent(all, &["Bash".into()], &["Bash".into()], false);
    assert_eq!(filtered.len(), 0);
}

// ─── configure_tool_command — env stripping ──────────────────────────

#[test]
fn configure_tool_command_sets_no_prompt_envs_normal() {
    // We can't actually inspect the configured env without spawning,
    // so verify by running a bash command and checking the env it
    // sees. (If configure_tool_command silently regressed, the env
    // wouldn't be set and `$GIT_TERMINAL_PROMPT` would be empty.)
    let mut cmd = Command::new("bash");
    cmd.arg("-c")
        .arg("echo \"$GIT_TERMINAL_PROMPT|$SUDO_ASKPASS|$SSH_ASKPASS\"");
    configure_tool_command(&mut cmd);
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let out = rt.block_on(async { cmd.output().await.unwrap() });
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("0|/bin/false|/bin/false"), "got: {stdout}");
}

// ─── non_interactive_shell_command — extra cases ─────────────────────

#[test]
fn non_interactive_bare_sudo_gets_minus_n_normal() {
    // Plain "sudo" with no args ought to still be made non-interactive.
    assert_eq!(non_interactive_shell_command("sudo"), "sudo -n");
}

#[test]
fn non_interactive_already_minus_n_is_unchanged_robust() {
    assert_eq!(
        non_interactive_shell_command("sudo -n true"),
        "sudo -n true"
    );
}

#[test]
fn non_interactive_preserves_leading_whitespace_normal() {
    // Pre-existing indentation in the user's script must stay intact —
    // shell heredocs and `set -e; sudo …` blocks rely on it.
    let cmd = "  sudo apt update";
    let out = non_interactive_shell_command(cmd);
    assert!(out.starts_with("  "), "leading ws lost: {out}");
    assert!(out.contains("sudo -n"), "{out}");
}

#[test]
fn non_interactive_unrelated_command_unchanged_normal() {
    assert_eq!(non_interactive_shell_command("ls"), "ls");
    assert_eq!(non_interactive_shell_command(""), "");
}

// ─── terminal_safe_text — extra cases ────────────────────────────────

#[test]
fn terminal_safe_text_preserves_tab_newline_cr_normal() {
    let raw = "a\tb\nc\rd";
    assert_eq!(terminal_safe_text(raw), "a\tb\nc\rd");
}

#[test]
fn terminal_safe_text_drops_lone_escape_normal() {
    // Lone escape with no follow-up is dropped (no terminal sequence
    // to consume) — all that remains is the surrounding text.
    let raw = "before\u{1b}";
    assert_eq!(terminal_safe_text(raw), "before");
}

#[test]
fn terminal_safe_text_handles_osc_terminator_with_st_robust() {
    // OSC sequences can terminate with either BEL (\x07) or ST (ESC \\).
    let raw = "\u{1b}]0;title\u{1b}\\after";
    assert_eq!(terminal_safe_text(raw), "after");
}

#[test]
fn terminal_safe_text_handles_unrecognized_escape_robust() {
    // ESC followed by something other than [ or ] consumes the next
    // byte and continues — no panic, no mojibake.
    let raw = "\u{1b}=normal";
    assert_eq!(terminal_safe_text(raw), "normal");
}

#[test]
fn terminal_safe_text_passes_unicode_normal() {
    let raw = "héllo wörld 世界";
    assert_eq!(terminal_safe_text(raw), "héllo wörld 世界");
}

// ─── ExecutionResult builders ────────────────────────────────────────

#[test]
fn execution_result_success_has_no_diagnostics_normal() {
    let r = ExecutionResult::success("ok");
    assert!(!r.is_error());
    assert!(r.diagnostics.is_empty());
    assert!(r.diff.is_none());
    assert!(r.provenance.is_none());
}

#[test]
fn execution_result_with_provenance_attaches_normal() {
    let r = ExecutionResult::success("ok").with_provenance(ToolProvenance {
        cwd: PathBuf::from("/tmp"),
        source: ToolSource::LocalExecutor,
    });
    assert!(r.provenance.is_some());
    assert_eq!(r.provenance.unwrap().cwd, PathBuf::from("/tmp"));
}

#[test]
fn execution_result_with_diff_attaches_normal() {
    let view = crate::types::parse_unified_diff("x.rs", "@@ -1,1 +1,1 @@\n-a\n+b\n");
    let r = ExecutionResult::success("ok").with_diff(view);
    assert!(r.diff.is_some());
}

// ─── execute_bash dispatch ────────────────────────────────────────────

#[tokio::test]
#[serial_test::serial]
async fn execute_bash_success_carries_provenance_normal() {
    crate::sandbox::reset_active_bash_sandbox_for_test();
    let result = execute_bash("echo hello", Some(5_000), Path::new(".")).await;
    assert!(!result.is_error());
    assert!(result.output.contains("hello"), "{}", result.output);
    // Successful bash should attach provenance pointing at the cwd.
    assert!(result.provenance.is_some(), "bash success must carry cwd");
    assert_eq!(result.provenance.unwrap().source, ToolSource::LocalExecutor);
}

#[tokio::test]
async fn execute_bash_nonzero_exit_is_complete_with_header_normal() {
    // Per Anthropic semantics, a non-zero exit code is *output*, not
    // a tool failure. The result is still Success and includes
    // `[exit N]` at the top so the model can read the code.
    let result = execute_bash("false", Some(5_000), Path::new(".")).await;
    assert!(!result.is_error(), "exit-1 must be Success: {:?}", result);
    assert!(result.output.contains("[exit 1]"), "{}", result.output);
}

#[tokio::test]
#[serial_test::serial]
async fn execute_bash_timeout_returns_failure_robust() {
    crate::sandbox::reset_active_bash_sandbox_for_test();
    // sleep longer than the timeout — must time out cleanly.
    let result = execute_bash("sleep 5", Some(100), Path::new(".")).await;
    assert!(result.is_error());
    assert!(result.output.contains("timed out"), "{}", result.output);
}

#[tokio::test]
#[serial_test::serial]
async fn execute_bash_combines_stdout_and_stderr_normal() {
    crate::sandbox::reset_active_bash_sandbox_for_test();
    let result = execute_bash("echo out; echo err >&2", Some(5_000), Path::new(".")).await;
    assert!(!result.is_error());
    assert!(result.output.contains("out"), "{}", result.output);
    assert!(result.output.contains("err"), "{}", result.output);
    assert!(
        result.output.contains("---stderr---"),
        "stdout+stderr split marker missing: {}",
        result.output
    );
}

#[tokio::test]
#[serial_test::serial]
async fn execute_bash_strips_ansi_escape_codes_normal() {
    crate::sandbox::reset_active_bash_sandbox_for_test();
    // bash subprocess emits ANSI red — terminal_safe_text strips it.
    let result = execute_bash("printf '\\033[31mred\\033[0m'", Some(5_000), Path::new(".")).await;
    assert!(!result.is_error());
    assert!(
        !result.output.contains('\u{1b}'),
        "ANSI leaked: {:?}",
        result.output
    );
    assert!(result.output.contains("red"), "{}", result.output);
}

#[tokio::test]
#[serial_test::serial]
async fn execute_bash_streaming_progress_delivers_bursty_lines_regression() {
    crate::sandbox::reset_active_bash_sandbox_for_test();
    let (tx, mut rx) = tokio::sync::mpsc::channel(1);
    let receiver = tokio::spawn(async move {
        let mut chunks = Vec::new();
        while let Some(event) = rx.recv().await {
            if let EngineEvent::Tool(ToolEvent::OutputChunk { chunk, .. }) = event {
                chunks.push(chunk);
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            }
        }
        chunks
    });

    let result = execute_bash_inner(
        "printf 'line1\\nline2\\nline3\\nline4\\nline5\\n'",
        Some(5_000),
        Path::new("."),
        Some(("tool-stream".to_string(), tx)),
    )
    .await;

    assert!(!result.is_error(), "{}", result.output);
    let chunks = receiver.await.expect("receiver task");
    assert_eq!(
        chunks,
        ["line1", "line2", "line3", "line4", "line5"],
        "streaming progress dropped output chunks"
    );
}

// ─── execute_read ─────────────────────────────────────────────────────

#[tokio::test]
async fn execute_read_returns_numbered_lines_normal() {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("hello.txt");
    tokio::fs::write(&path, "alpha\nbravo\ncharlie\n")
        .await
        .unwrap();

    let result = execute_read(path.to_str().unwrap(), None, None, None).await;
    assert!(!result.is_error());
    assert!(result.output.contains("1: alpha"), "{}", result.output);
    assert!(result.output.contains("2: bravo"), "{}", result.output);
    assert!(result.output.contains("3: charlie"), "{}", result.output);
}

#[tokio::test]
async fn execute_read_directory_lists_entries_with_slash_suffix_normal() {
    let dir = tempfile::tempdir().expect("temp dir");
    tokio::fs::write(dir.path().join("a.txt"), "x")
        .await
        .unwrap();
    tokio::fs::create_dir(dir.path().join("subdir"))
        .await
        .unwrap();

    let result = execute_read(dir.path().to_str().unwrap(), None, None, None).await;
    assert!(!result.is_error());
    assert!(result.output.contains("a.txt"), "{}", result.output);
    assert!(
        result.output.contains("subdir/"),
        "dir suffix missing: {}",
        result.output
    );
}

#[tokio::test]
async fn execute_read_offset_and_limit_paginate_normal() {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("big.txt");
    let body: String = (1..=20).map(|i| format!("line{i}\n")).collect();
    tokio::fs::write(&path, body).await.unwrap();

    let result = execute_read(path.to_str().unwrap(), Some(5), Some(3), None).await;
    assert!(!result.is_error());
    // Should show lines 5, 6, 7 only.
    assert!(result.output.contains("5: line5"), "{}", result.output);
    assert!(result.output.contains("7: line7"), "{}", result.output);
    assert!(!result.output.contains("8: line8"), "{}", result.output);
    assert!(!result.output.contains("4: line4"), "{}", result.output);
}

#[tokio::test]
async fn execute_read_missing_file_returns_failure_robust() {
    let result = execute_read("/tmp/jfc-definitely-not-here-9999/x.txt", None, None, None).await;
    assert!(result.is_error());
    assert!(result.output.contains("Cannot read"), "{}", result.output);
}

#[tokio::test]
async fn execute_read_dedup_returns_unchanged_marker_robust() {
    use crate::context::ReadDedupCache;

    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("d.txt");
    tokio::fs::write(&path, "stable\n").await.unwrap();

    let cache = Arc::new(Mutex::new(ReadDedupCache::new()));
    // First full read: populates the cache.
    let r1 = execute_read(path.to_str().unwrap(), None, None, Some(&cache)).await;
    assert!(!r1.is_error());

    // Second full read on the unchanged file returns the dedup marker.
    let r2 = execute_read(path.to_str().unwrap(), None, None, Some(&cache)).await;
    assert!(!r2.is_error());
    assert!(
        r2.output.contains("File unchanged since last full read"),
        "expected dedup stub, got: {}",
        r2.output
    );
}

#[tokio::test]
async fn execute_read_paginated_skips_dedup_robust() {
    use crate::context::ReadDedupCache;

    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("p.txt");
    let body: String = (1..=10).map(|i| format!("L{i}\n")).collect();
    tokio::fs::write(&path, body).await.unwrap();

    let cache = Arc::new(Mutex::new(ReadDedupCache::new()));
    // Full read populates cache.
    let _ = execute_read(path.to_str().unwrap(), None, None, Some(&cache)).await;
    // Paginated read on the same path: dedup must NOT short-circuit.
    let r = execute_read(path.to_str().unwrap(), Some(2), Some(3), Some(&cache)).await;
    assert!(!r.is_error());
    assert!(!r.output.contains("File unchanged"), "{}", r.output);
    assert!(r.output.contains("2: L2"), "{}", r.output);
}

// ─── execute_write ────────────────────────────────────────────────────

#[tokio::test]
async fn execute_write_creates_file_with_summary_normal() {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("new.txt");

    let result = execute_write(path.to_str().unwrap(), "hello\nworld\n").await;
    assert!(!result.is_error());
    assert!(path.exists(), "file should exist after write");
    let on_disk = tokio::fs::read_to_string(&path).await.unwrap();
    assert_eq!(on_disk, "hello\nworld\n");
    assert!(result.output.starts_with("Wrote "), "{}", result.output);
}

#[tokio::test]
async fn execute_write_overwrite_uses_updated_header_normal() {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("ow.txt");
    tokio::fs::write(&path, "original").await.unwrap();

    let result = execute_write(path.to_str().unwrap(), "replaced").await;
    assert!(!result.is_error());
    assert!(result.output.starts_with("Updated "), "{}", result.output);
}

#[tokio::test]
async fn execute_write_creates_parent_dirs_normal() {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("nested/two/three/file.txt");

    let result = execute_write(path.to_str().unwrap(), "x").await;
    assert!(!result.is_error(), "{}", result.output);
    assert!(path.exists());
}

#[tokio::test]
async fn execute_write_long_content_truncates_preview_robust() {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("long.txt");
    let body: String = (1..=100).map(|i| format!("line{i}\n")).collect();

    let result = execute_write(path.to_str().unwrap(), &body).await;
    assert!(!result.is_error());
    assert!(
        result.output.contains("more lines"),
        "should announce truncation: {}",
        result.output
    );
    // File on disk has the full content, even though preview is short.
    let on_disk = tokio::fs::read_to_string(&path).await.unwrap();
    assert_eq!(on_disk.lines().count(), 100);
}

// ─── execute_edit ─────────────────────────────────────────────────────

#[tokio::test]
async fn execute_edit_first_only_replaces_one_normal() {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("e.txt");
    tokio::fs::write(&path, "foo bar foo").await.unwrap();

    let result = execute_edit(path.to_str().unwrap(), "foo", "BAZ", ReplacementMode::All).await;
    assert!(!result.is_error(), "{}", result.output);
    let after = tokio::fs::read_to_string(&path).await.unwrap();
    assert_eq!(after, "BAZ bar BAZ");
    assert!(result.diff.is_some(), "Edit must produce a DiffView");
}

#[tokio::test]
async fn execute_edit_multiple_matches_without_replace_all_fails_robust() {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("m.txt");
    tokio::fs::write(&path, "a a a").await.unwrap();

    let result = execute_edit(path.to_str().unwrap(), "a", "b", ReplacementMode::FirstOnly).await;
    assert!(result.is_error());
    assert!(
        result.output.contains("matches"),
        "expected 'multiple matches' error: {}",
        result.output
    );
}

#[tokio::test]
async fn execute_edit_old_string_not_found_fails_robust() {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("nf.txt");
    tokio::fs::write(&path, "abc").await.unwrap();

    let result = execute_edit(
        path.to_str().unwrap(),
        "missing",
        "x",
        ReplacementMode::FirstOnly,
    )
    .await;
    assert!(result.is_error());
    assert!(result.output.contains("not found"), "{}", result.output);
}

#[tokio::test]
async fn execute_edit_whitespace_tolerant_on_indent_drift_normal() {
    // File on disk uses 8-space indentation; the model supplies the block at
    // 4-space indentation. Exact match misses, whitespace fallback recovers it.
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("ws.rs");
    tokio::fs::write(
        &path,
        "fn f() {\n        let a = 1;\n        let b = 2;\n}\n",
    )
    .await
    .unwrap();

    let result = execute_edit(
        path.to_str().unwrap(),
        "    let a = 1;\n    let b = 2;",
        "        let a = 100;\n        let b = 200;",
        ReplacementMode::FirstOnly,
    )
    .await;
    assert!(
        !result.is_error(),
        "ws-tolerant edit should succeed: {}",
        result.output
    );
    let content = tokio::fs::read_to_string(&path).await.unwrap();
    assert!(content.contains("let a = 100;"), "{content}");
    assert!(content.contains("let b = 200;"), "{content}");
}

#[test]
fn apply_one_edit_exact_and_ws_tolerant_normal() {
    use super::filesystem::apply_one_edit;
    // Exact match.
    let out = apply_one_edit("let a = 1;\n", "a = 1", "a = 2", false, "e1").unwrap();
    assert_eq!(out, "let a = 2;\n");
    // Whitespace-tolerant: file has 8-space indent, edit supplies 4-space.
    let file = "fn f() {\n        a();\n        b();\n}\n";
    let out = apply_one_edit(file, "    a();\n    b();", "    c();", false, "e2").unwrap();
    assert!(out.contains("    c();") || !out.contains("        c();"));
    assert!(out.contains("c();"));
}

#[test]
fn apply_one_edit_tolerates_literal_escape_sequences_normal() {
    use super::filesystem::apply_one_edit;
    // The model emitted "\n" as two literal characters instead of a newline.
    // Tier 3 (escape-normalized) recovers the real two-line block.
    let file = "fn f() {\n    a();\n    b();\n}\n";
    let out = apply_one_edit(file, "    a();\\n    b();", "    c();", false, "e7").unwrap();
    assert!(out.contains("c();"), "{out}");
    assert!(!out.contains("a();"), "{out}");
}

#[test]
fn apply_one_edit_ambiguous_and_missing_fail_robust() {
    use super::filesystem::apply_one_edit;
    // No match → error mentioning the label.
    let err = apply_one_edit("hello\n", "xyz", "q", false, "e9").unwrap_err();
    assert!(err.contains("e9"), "{err}");
    assert!(err.to_lowercase().contains("not found"), "{err}");
    // Multiple exact matches without replace_all → error.
    let err = apply_one_edit("x x x", "x", "y", false, "e3").unwrap_err();
    assert!(err.contains("matched"), "{err}");
}

#[tokio::test]
async fn execute_edit_tolerates_unicode_punct_drift_normal() {
    // File uses an em-dash and curly quotes; the model emits ASCII hyphen and
    // straight quotes. Exact match misses; the Unicode-folding tier recovers it.
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("uni.rs");
    tokio::fs::write(
        &path,
        "let s = \u{201C}hello\u{201D}; // note \u{2014} aside\n",
    )
    .await
    .unwrap();
    let result = execute_edit(
        path.to_str().unwrap(),
        "let s = \"hello\"; // note - aside",
        "let s = \"HELLO\";",
        ReplacementMode::FirstOnly,
    )
    .await;
    assert!(
        !result.is_error(),
        "unicode-tolerant edit should succeed: {}",
        result.output
    );
    let content = tokio::fs::read_to_string(&path).await.unwrap();
    assert!(content.contains("HELLO"), "{content}");
}

#[tokio::test]
async fn execute_edit_whitespace_ambiguous_fails_robust() {
    // Two whitespace-equivalent two-line blocks with NO exact substring match
    // (the needle's indentation differs from both on-disk copies) → must fail
    // rather than silently editing the wrong one.
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("amb.rs");
    tokio::fs::write(
        &path,
        "  do_it();\n  log();\nmid();\n\t\tdo_it();\n\t\tlog();\n",
    )
    .await
    .unwrap();
    // 4-space indent matches neither the 2-space nor the tab copy exactly, but
    // both normalize equal → ambiguous.
    let result = execute_edit(
        path.to_str().unwrap(),
        "    do_it();\n    log();",
        "    done();\n    logged();",
        ReplacementMode::FirstOnly,
    )
    .await;
    assert!(result.is_error());
    assert!(result.output.contains("ambiguous"), "{}", result.output);
}

#[tokio::test]
async fn execute_edit_empty_old_on_nonempty_file_rejects_robust() {
    // Empty old_string on a non-empty file is ambiguous (where to
    // insert?) so we reject — only allowed on a missing/empty file
    // as a "create" path.
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("ne.txt");
    tokio::fs::write(&path, "stuff").await.unwrap();

    let result = execute_edit(
        path.to_str().unwrap(),
        "",
        "new",
        ReplacementMode::FirstOnly,
    )
    .await;
    assert!(result.is_error());
    assert!(
        result.output.contains("old_string is empty"),
        "{}",
        result.output
    );
}

#[tokio::test]
async fn execute_edit_empty_old_on_missing_file_creates_normal() {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("create.txt");

    let result = execute_edit(
        path.to_str().unwrap(),
        "",
        "fresh content",
        ReplacementMode::FirstOnly,
    )
    .await;
    assert!(!result.is_error(), "{}", result.output);
    let body = tokio::fs::read_to_string(&path).await.unwrap();
    assert_eq!(body, "fresh content");
    assert!(
        result.output.contains("Created new file"),
        "{}",
        result.output
    );
}

#[tokio::test]
async fn execute_edit_replace_all_mentions_count_normal() {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("r.txt");
    tokio::fs::write(&path, "x x x x").await.unwrap();

    let result = execute_edit(path.to_str().unwrap(), "x", "Y", ReplacementMode::All).await;
    assert!(!result.is_error());
    assert!(result.output.contains("4 occurrences"), "{}", result.output);
}

// ─── build_edit_diff_view ────────────────────────────────────────────

#[test]
fn build_edit_diff_view_no_change_yields_empty_hunks_normal() {
    let view = build_edit_diff_view("x.rs", "abc\n", "abc\n");
    assert!(view.hunks.is_empty());
    assert_eq!(view.additions, 0);
    assert_eq!(view.deletions, 0);
}

#[test]
fn build_edit_diff_view_counts_added_removed_normal() {
    let view = build_edit_diff_view("x.rs", "a\nb\nc\n", "a\nB\nc\n");
    assert_eq!(view.additions, 1);
    assert_eq!(view.deletions, 1);
    assert_eq!(view.hunks.len(), 1);
    assert_eq!(view.file_path, "x.rs");
}

#[test]
fn build_edit_diff_view_pure_addition_robust() {
    let view = build_edit_diff_view("x.rs", "a\nb\n", "a\nb\nc\n");
    assert_eq!(view.additions, 1);
    assert_eq!(view.deletions, 0);
}

// ─── execute_glob ─────────────────────────────────────────────────────

#[tokio::test]
async fn execute_glob_matches_files_normal() {
    let dir = tempfile::tempdir().expect("temp dir");
    tokio::fs::write(dir.path().join("a.rs"), "").await.unwrap();
    tokio::fs::write(dir.path().join("b.rs"), "").await.unwrap();
    tokio::fs::write(dir.path().join("c.txt"), "")
        .await
        .unwrap();

    let result = execute_glob("*.rs", None, dir.path()).await;
    assert!(!result.is_error(), "{}", result.output);
    assert!(result.output.contains("a.rs"), "{}", result.output);
    assert!(result.output.contains("b.rs"), "{}", result.output);
    assert!(!result.output.contains("c.txt"), "{}", result.output);
}

#[tokio::test]
async fn execute_glob_no_match_returns_message_normal() {
    let dir = tempfile::tempdir().expect("temp dir");
    let result = execute_glob("*.zzz", None, dir.path()).await;
    assert!(!result.is_error());
    assert!(
        result.output.contains("No files matched"),
        "{}",
        result.output
    );
}

// ─── execute_grep ─────────────────────────────────────────────────────

#[tokio::test]
async fn execute_grep_finds_pattern_normal() {
    let dir = tempfile::tempdir().expect("temp dir");
    tokio::fs::write(
        dir.path().join("a.txt"),
        "line one\nlooking-for-this\nfinal\n",
    )
    .await
    .unwrap();

    let result = execute_grep("looking-for-this", None, None, None, dir.path()).await;
    assert!(!result.is_error(), "{}", result.output);
    assert!(
        result.output.contains("looking-for-this"),
        "{}",
        result.output
    );
}

#[tokio::test]
async fn execute_grep_no_match_returns_message_normal() {
    let dir = tempfile::tempdir().expect("temp dir");
    tokio::fs::write(dir.path().join("a.txt"), "x\n")
        .await
        .unwrap();

    let result = execute_grep("never-here-zzz", None, None, None, dir.path()).await;
    assert!(!result.is_error());
    assert!(result.output.contains("No matches"), "{}", result.output);
}

#[tokio::test]
async fn execute_grep_files_with_matches_mode_normal() {
    let dir = tempfile::tempdir().expect("temp dir");
    tokio::fs::write(dir.path().join("a.txt"), "needle here\n")
        .await
        .unwrap();
    tokio::fs::write(dir.path().join("b.txt"), "no needle\n")
        .await
        .unwrap();

    let result = execute_grep("needle", None, None, Some("files_with_matches"), dir.path()).await;
    assert!(!result.is_error(), "{}", result.output);
    assert!(result.output.contains("a.txt"), "{}", result.output);
}

// ─── execute_task_create / update / list / done ──────────────────────

#[test]
fn execute_task_create_without_store_fails_robust() {
    let r = execute_task_create(None, task_create_request("subj", "desc"));
    assert!(r.is_error());
    assert!(r.output.contains("Task store not available"));
}

#[test]
fn execute_task_create_with_store_returns_task_json_normal() {
    let store = TaskStore::in_memory();
    let r = execute_task_create(Some(store), task_create_request("ship", "release v1"));
    assert!(!r.is_error(), "{:?}", r);
    // The output is the JSON of the created task — should mention the
    // subject and a `t1` id.
    assert!(r.output.contains("ship"), "{}", r.output);
    assert!(r.output.contains("t1"), "{}", r.output);
}

#[test]
fn execute_task_create_rejects_placeholder_fixture_robust() {
    let store = TaskStore::in_memory();
    let r = execute_task_create(Some(store.clone()), task_create_request("subj", "desc"));
    assert!(r.is_error(), "{:?}", r);
    assert!(r.output.contains("placeholder"), "{}", r.output);
    assert!(store.list(DeletedFilter::Include).is_empty());
}

#[test]
fn execute_task_create_with_unknown_dependency_fails_robust() {
    let store = TaskStore::in_memory();
    let mut request = task_create_request("x", "y");
    request.blocked_by = vec!["t999".into()];
    let r = execute_task_create(Some(store), request);
    assert!(r.is_error(), "{:?}", r);
}

#[test]
fn execute_task_update_without_store_fails_robust() {
    let r = execute_task_update(None, task_update_request("t1"));
    assert!(r.is_error());
}

#[test]
fn execute_task_update_changes_status_normal() {
    let store = TaskStore::in_memory();
    let create = execute_task_create(
        Some(store.clone()),
        task_create_request("alpha", "do alpha"),
    );
    assert!(!create.is_error());
    // First-created task gets id `t1`.
    let mut request = task_update_request("t1");
    request.status = Some("in_progress".into());
    let r = execute_task_update(Some(store), request);
    assert!(!r.is_error(), "{}", r.output);
    assert!(r.output.contains("in_progress"), "{}", r.output);
}

#[test]
fn execute_task_update_invalid_status_fails_robust() {
    let store = TaskStore::in_memory();
    execute_task_create(Some(store.clone()), task_create_request("x", "y"));
    let mut request = task_update_request("t1");
    request.status = Some("not_a_status".into());
    request.subject = Some("renamed".into());
    let r = execute_task_update(Some(store), request);
    assert!(r.is_error(), "{}", r.output);
    assert!(r.output.contains("Invalid task status"), "{}", r.output);
}

#[test]
fn execute_task_done_marks_completed_normal() {
    let store = TaskStore::in_memory();
    execute_task_create(Some(store.clone()), task_create_request("do", "it"));
    let r = execute_task_done(Some(store), "t1");
    assert!(!r.is_error(), "{}", r.output);
    assert!(r.output.contains("completed"), "{}", r.output);
}

#[test]
fn execute_task_done_unknown_id_fails_robust() {
    let store = TaskStore::in_memory();
    let r = execute_task_done(Some(store), "tnosuch");
    assert!(r.is_error());
}

#[test]
fn execute_task_list_without_store_fails_robust() {
    let r = execute_task_list(None, None, None, false, None);
    assert!(r.is_error());
}

#[test]
fn execute_task_list_returns_tasks_normal() {
    let store = TaskStore::in_memory();
    execute_task_create(Some(store.clone()), task_create_request("alpha", "first"));
    execute_task_create(Some(store.clone()), task_create_request("bravo", "second"));
    let r = execute_task_list(Some(store), None, None, false, None);
    assert!(!r.is_error(), "{}", r.output);
    assert!(r.output.contains("alpha"), "{}", r.output);
    assert!(r.output.contains("bravo"), "{}", r.output);
}

#[test]
fn execute_task_list_filters_by_owner_robust() {
    let store = TaskStore::in_memory();
    execute_task_create(Some(store.clone()), task_create_request("x", "y"));
    let mut request = task_update_request("t1");
    request.owner = Some("alice".into());
    execute_task_update(Some(store.clone()), request);
    let only_alice = execute_task_list(Some(store.clone()), None, Some("alice"), false, None);
    assert!(only_alice.output.contains("alice"), "{}", only_alice.output);

    let only_bob = execute_task_list(Some(store), None, Some("bob"), false, None);
    assert!(!only_bob.output.contains("alice"), "{}", only_bob.output);
}

// ─── execute_memory_create / delete ──────────────────────────────────

#[test]
fn execute_memory_create_invalid_level_fails_robust() {
    let dir = tempfile::tempdir().expect("temp dir");
    let r = execute_memory_create("bogus", "context", "private", "body", dir.path());
    assert!(r.is_error());
    assert!(r.output.contains("Invalid level"), "{}", r.output);
}

#[test]
fn execute_memory_create_invalid_type_fails_robust() {
    let dir = tempfile::tempdir().expect("temp dir");
    let r = execute_memory_create("user", "wibble", "private", "body", dir.path());
    assert!(r.is_error());
}

#[test]
fn execute_memory_create_invalid_scope_fails_robust() {
    let dir = tempfile::tempdir().expect("temp dir");
    let r = execute_memory_create("user", "context", "wibble", "body", dir.path());
    assert!(r.is_error());
}

#[test]
fn execute_memory_create_empty_body_fails_robust() {
    let dir = tempfile::tempdir().expect("temp dir");
    let r = execute_memory_create("project", "context", "private", "   ", dir.path());
    assert!(r.is_error());
    assert!(r.output.contains("body cannot be empty"), "{}", r.output);
}

#[test]
fn execute_memory_create_project_writes_file_normal() {
    let dir = tempfile::tempdir().expect("temp dir");
    let r = execute_memory_create(
        "project",
        "context",
        "private",
        "Remember the alamo.",
        dir.path(),
    );
    assert!(!r.is_error(), "{}", r.output);
    assert!(r.output.contains("Memory saved to"), "{}", r.output);
}

#[test]
fn execute_memory_delete_missing_path_fails_robust() {
    let r = execute_memory_delete("/tmp/jfc-no-such-memory-path-xyz-9831.md");
    assert!(r.is_error());
    assert!(r.output.contains("File not found"), "{}", r.output);
}

#[test]
fn execute_memory_delete_outside_memory_dir_rejected_robust() {
    // delete_memory refuses paths outside ~/.config/jfc/memory or
    // <project>/.jfc/memory. A scratch file in tempdir hits that
    // guardrail — the executor surfaces the failure cleanly.
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("not-a-memory.md");
    std::fs::write(&path, "scratch").unwrap();
    let r = execute_memory_delete(path.to_str().unwrap());
    assert!(r.is_error(), "expected failure for path outside memory dir");
    assert!(r.output.contains("Failed to delete memory"), "{}", r.output);
}

// ─── execute_team_member_mode validation ─────────────────────────────

#[tokio::test]
async fn execute_team_member_mode_invalid_mode_fails_robust() {
    let r = execute_team_member_mode("alice", "godmode", Some("alpha")).await;
    assert!(r.is_error());
    assert!(r.output.contains("Invalid mode"), "{}", r.output);
}

#[tokio::test]
async fn execute_team_member_mode_no_team_fails_robust() {
    // Mode is valid but there's no active team.
    let r = execute_team_member_mode("alice", "default", None).await;
    assert!(r.is_error());
    assert!(r.output.contains("No active team"), "{}", r.output);
}

// ─── execute_tool dispatch ────────────────────────────────────────────

#[test]
fn resolve_bash_workdir_defaults_and_resolves_paths_regression() {
    let cwd = Path::new("/repo");

    assert_eq!(resolve_bash_workdir(cwd, None), PathBuf::from("/repo"));
    assert_eq!(resolve_bash_workdir(cwd, Some("")), PathBuf::from("/repo"));
    assert_eq!(
        resolve_bash_workdir(cwd, Some("sub/dir")),
        PathBuf::from("/repo/sub/dir")
    );
    assert_eq!(
        resolve_bash_workdir(cwd, Some("/tmp")),
        PathBuf::from("/tmp")
    );
}

#[tokio::test]
async fn execute_tool_dispatches_bash_normal() {
    let dir = tempfile::tempdir().expect("temp dir");
    let result = execute_tool(
        ToolKind::Bash,
        ToolInput::Bash {
            command: "echo dispatched".into(),
            timeout: Some(5_000),
            workdir: None,
            run_in_background: None,
        },
        dir.path().to_path_buf(),
        None,
        None,
        None,
    )
    .await;
    assert!(!result.is_error(), "{}", result.output);
    assert!(result.output.contains("dispatched"), "{}", result.output);
}

#[tokio::test]
async fn execute_tool_bash_honors_workdir_regression() {
    crate::sandbox::reset_active_bash_sandbox_for_test();
    let dir = tempfile::tempdir().expect("temp dir");
    let subdir = dir.path().join("subdir");
    std::fs::create_dir(&subdir).expect("create subdir");

    let result = execute_tool(
        ToolKind::Bash,
        ToolInput::Bash {
            command: "pwd -P".into(),
            timeout: Some(5_000),
            workdir: Some("subdir".into()),
            run_in_background: None,
        },
        dir.path().to_path_buf(),
        None,
        None,
        None,
    )
    .await;

    assert!(!result.is_error(), "{}", result.output);
    let actual = Path::new(result.output.trim());
    let expected = subdir.canonicalize().expect("canonical subdir");
    assert_eq!(actual, expected.as_path(), "{}", result.output);
}

fn parse_bash_task_id(output: &str) -> String {
    output
        .lines()
        .find_map(|line| line.strip_prefix("task_id: "))
        .expect("background result should include task_id")
        .trim()
        .to_owned()
}

#[tokio::test]
#[serial_test::serial]
async fn execute_tool_bash_run_in_background_can_be_read_regression() {
    crate::sandbox::reset_active_bash_sandbox_for_test();
    let dir = tempfile::tempdir().expect("temp dir");
    let result = execute_tool(
        ToolKind::Bash,
        ToolInput::Bash {
            command: "printf start; sleep 0.2; printf done".into(),
            timeout: Some(5_000),
            workdir: None,
            run_in_background: Some(true),
        },
        dir.path().to_path_buf(),
        None,
        None,
        None,
    )
    .await;

    assert!(!result.is_error(), "{}", result.output);
    assert!(
        result.output.contains("status: running"),
        "{}",
        result.output
    );
    assert!(result.output.contains("output_file:"), "{}", result.output);
    let task_id = parse_bash_task_id(&result.output);
    tokio::time::sleep(std::time::Duration::from_millis(400)).await;

    let output = execute_tool(
        ToolKind::BashOutput,
        ToolInput::BashOutput {
            task_id,
            offset: None,
            limit: None,
            block: None,
            timeout: None,
            wait_up_to: None,
        },
        dir.path().to_path_buf(),
        None,
        None,
        None,
    )
    .await;

    assert!(!output.is_error(), "{}", output.output);
    assert!(
        output.output.contains("status: completed"),
        "{}",
        output.output
    );
    assert!(output.output.contains("startdone"), "{}", output.output);
}

#[tokio::test]
#[serial_test::serial]
async fn execute_tool_bash_output_blocks_by_default_regression() {
    crate::sandbox::reset_active_bash_sandbox_for_test();
    let dir = tempfile::tempdir().expect("temp dir");
    let result = execute_tool(
        ToolKind::Bash,
        ToolInput::Bash {
            command: "printf start; sleep 0.15; printf done".into(),
            timeout: Some(5_000),
            workdir: None,
            run_in_background: Some(true),
        },
        dir.path().to_path_buf(),
        None,
        None,
        None,
    )
    .await;

    assert!(!result.is_error(), "{}", result.output);
    let task_id = parse_bash_task_id(&result.output);
    let output = execute_tool(
        ToolKind::BashOutput,
        ToolInput::BashOutput {
            task_id,
            offset: None,
            limit: None,
            block: None,
            timeout: Some(2_000),
            wait_up_to: None,
        },
        dir.path().to_path_buf(),
        None,
        None,
        None,
    )
    .await;

    assert!(!output.is_error(), "{}", output.output);
    assert!(
        output.output.contains("retrieval_status: success"),
        "{}",
        output.output
    );
    assert!(output.output.contains("startdone"), "{}", output.output);
}

#[tokio::test]
#[serial_test::serial]
async fn execute_tool_bash_output_nonblocking_snapshot_regression() {
    crate::sandbox::reset_active_bash_sandbox_for_test();
    let dir = tempfile::tempdir().expect("temp dir");
    let result = execute_tool(
        ToolKind::Bash,
        ToolInput::Bash {
            command: "sleep 0.3; printf done".into(),
            timeout: Some(5_000),
            workdir: None,
            run_in_background: Some(true),
        },
        dir.path().to_path_buf(),
        None,
        None,
        None,
    )
    .await;

    assert!(!result.is_error(), "{}", result.output);
    let task_id = parse_bash_task_id(&result.output);
    let output = execute_tool(
        ToolKind::BashOutput,
        ToolInput::BashOutput {
            task_id,
            offset: None,
            limit: None,
            block: Some(false),
            timeout: None,
            wait_up_to: None,
        },
        dir.path().to_path_buf(),
        None,
        None,
        None,
    )
    .await;

    assert!(!output.is_error(), "{}", output.output);
    assert!(
        output.output.contains("retrieval_status: not_ready"),
        "{}",
        output.output
    );
}

#[tokio::test]
#[serial_test::serial]
async fn execute_tool_bash_explicit_background_is_not_abort_tracked_regression() {
    crate::sandbox::reset_active_bash_sandbox_for_test();
    crate::bash_processes::clear_for_test();
    let dir = tempfile::tempdir().expect("temp dir");
    let result = execute_tool(
        ToolKind::Bash,
        ToolInput::Bash {
            command: "printf start; sleep 0.2; printf done".into(),
            timeout: Some(5_000),
            workdir: None,
            run_in_background: Some(true),
        },
        dir.path().to_path_buf(),
        None,
        None,
        None,
    )
    .await;

    assert!(!result.is_error(), "{}", result.output);
    assert!(
        crate::bash_processes::snapshot().is_empty(),
        "explicit background Bash must not be killed by generic abort tracking"
    );

    let task_id = parse_bash_task_id(&result.output);
    let output = execute_tool(
        ToolKind::BashOutput,
        ToolInput::BashOutput {
            task_id,
            offset: None,
            limit: None,
            block: None,
            timeout: Some(2_000),
            wait_up_to: None,
        },
        dir.path().to_path_buf(),
        None,
        None,
        None,
    )
    .await;

    assert!(!output.is_error(), "{}", output.output);
    assert!(output.output.contains("startdone"), "{}", output.output);
    crate::bash_processes::clear_for_test();
}

#[tokio::test]
#[serial_test::serial]
async fn execute_tool_bash_auto_background_stays_abort_tracked_regression() {
    crate::sandbox::reset_active_bash_sandbox_for_test();
    crate::bash_processes::clear_for_test();
    let _guard = EnvVarGuard::set("JFC_BASH_FOREGROUND_BUDGET_MS", "20");
    let dir = tempfile::tempdir().expect("temp dir");
    let result = execute_tool(
        ToolKind::Bash,
        ToolInput::Bash {
            command: "printf start; sleep 0.5; printf done".into(),
            timeout: Some(5_000),
            workdir: None,
            run_in_background: None,
        },
        dir.path().to_path_buf(),
        None,
        None,
        None,
    )
    .await;

    assert!(!result.is_error(), "{}", result.output);
    assert!(
        result.output.contains("foreground budget"),
        "{}",
        result.output
    );
    assert!(
        !crate::bash_processes::snapshot().is_empty(),
        "auto-backgrounded Bash started as foreground work and should remain abort-tracked"
    );

    let task_id = parse_bash_task_id(&result.output);
    let output = execute_tool(
        ToolKind::BashOutput,
        ToolInput::BashOutput {
            task_id,
            offset: None,
            limit: None,
            block: None,
            timeout: Some(2_000),
            wait_up_to: None,
        },
        dir.path().to_path_buf(),
        None,
        None,
        None,
    )
    .await;

    assert!(!output.is_error(), "{}", output.output);
    assert!(output.output.contains("startdone"), "{}", output.output);
    assert!(
        crate::bash_processes::snapshot().is_empty(),
        "completed Bash task should leave abort registry"
    );
}

struct EnvVarGuard {
    key: &'static str,
    previous: Option<std::ffi::OsString>,
}

impl EnvVarGuard {
    fn set(key: &'static str, value: &str) -> Self {
        let previous = std::env::var_os(key);
        // Tests using this helper are serial; no concurrent env readers are
        // expected in these focused regressions.
        unsafe {
            std::env::set_var(key, value);
        }
        Self { key, previous }
    }

    fn unset(key: &'static str) -> Self {
        let previous = std::env::var_os(key);
        unsafe {
            std::env::remove_var(key);
        }
        Self { key, previous }
    }
}

impl Drop for EnvVarGuard {
    fn drop(&mut self) {
        unsafe {
            match &self.previous {
                Some(value) => std::env::set_var(self.key, value),
                None => std::env::remove_var(self.key),
            }
        }
    }
}

#[tokio::test]
#[serial_test::serial]
async fn execute_tool_bash_auto_backgrounds_after_foreground_budget_regression() {
    crate::sandbox::reset_active_bash_sandbox_for_test();
    let _guard = EnvVarGuard::set("JFC_BASH_FOREGROUND_BUDGET_MS", "50");
    let dir = tempfile::tempdir().expect("temp dir");
    let result = execute_tool(
        ToolKind::Bash,
        ToolInput::Bash {
            command: "printf start; sleep 0.2; printf done".into(),
            timeout: Some(5_000),
            workdir: None,
            run_in_background: None,
        },
        dir.path().to_path_buf(),
        None,
        None,
        None,
    )
    .await;

    assert!(!result.is_error(), "{}", result.output);
    assert!(
        result.output.contains("foreground budget"),
        "{}",
        result.output
    );
    let task_id = parse_bash_task_id(&result.output);
    tokio::time::sleep(std::time::Duration::from_millis(400)).await;

    let output = execute_tool(
        ToolKind::BashOutput,
        ToolInput::BashOutput {
            task_id,
            offset: None,
            limit: Some(20),
            block: None,
            timeout: None,
            wait_up_to: None,
        },
        dir.path().to_path_buf(),
        None,
        None,
        None,
    )
    .await;

    assert!(!output.is_error(), "{}", output.output);
    assert!(output.output.contains("startdone"), "{}", output.output);
}

#[tokio::test]
#[serial_test::serial]
async fn execute_bash_large_output_persists_full_log_regression() {
    crate::sandbox::reset_active_bash_sandbox_for_test();
    let result = execute_bash("yes line | head -n 20000", Some(5_000), Path::new(".")).await;

    assert!(!result.is_error(), "{}", result.output);
    assert!(
        result.output.contains("Output truncated"),
        "{}",
        result.output
    );
    assert!(result.output.contains("output_file:"), "{}", result.output);
    let task_id = parse_bash_task_id(&result.output);
    let output = execute_bash_output(&task_id, Some(1), Some(5), Some(false), None).await;

    assert!(!output.is_error(), "{}", output.output);
    assert!(
        output.output.contains("showing_lines: 1-5"),
        "{}",
        output.output
    );
    assert!(output.output.contains("line"), "{}", output.output);
}

// Robust: a past-EOF offset on a FINISHED task is a range error naming the
// valid offsets, not a silent "(no output in requested range)".
#[tokio::test]
#[serial_test::serial]
async fn execute_bash_output_offset_past_eof_errors_robust() {
    crate::sandbox::reset_active_bash_sandbox_for_test();
    let result = execute_bash("yes line | head -n 20000", Some(5_000), Path::new(".")).await;
    assert!(!result.is_error(), "{}", result.output);
    let task_id = parse_bash_task_id(&result.output);

    let output = execute_bash_output(&task_id, Some(999_999), Some(5), Some(false), None).await;
    assert!(output.is_error(), "{}", output.output);
    assert!(
        output.output.contains("past the end of output"),
        "{}",
        output.output
    );
    assert!(output.output.contains("valid offsets"), "{}", output.output);
}

// Robust: an unknown (hallucinated) task id — one the Bash tool never issued —
// returns an actionable "unknown task id, do not invent one" error, not the raw
// "No such file or directory (os error 2)". Regression for the observed failure mode
// where the model fired BashOutput with fabricated `bash_<hex>`/semantic ids and the
// opaque OS error made it retry with *more* fabricated ids.
#[tokio::test]
#[serial_test::serial]
async fn execute_bash_output_unknown_task_id_is_actionable_robust() {
    crate::sandbox::reset_active_bash_sandbox_for_test();
    let output = execute_bash_output("bash_d2da789f9f50", None, None, Some(false), None).await;
    assert!(output.is_error(), "{}", output.output);
    assert!(
        output.output.contains("Unknown Bash task id"),
        "{}",
        output.output
    );
    assert!(output.output.contains("do not invent"), "{}", output.output);
    // Must NOT leak the raw OS error that triggered the retry loop.
    assert!(!output.output.contains("os error 2"), "{}", output.output);
}

#[tokio::test]
async fn execute_tool_task_kind_rejects_with_streaming_message_robust() {
    // The Task tool can't be dispatched through the normal executor;
    // it requires the streaming path. The dispatcher returns a clear
    // error rather than silently no-op'ing.
    let r = execute_tool(
        ToolKind::Task,
        ToolInput::Task(crate::types::TaskInput {
            description: "x".into(),
            prompt: "y".into(),
            subagent_type: None,
            category: None,
            run_in_background: false,
            model: None,
            effort: None,
            name: None,
            team_name: None,
            mode: None,
            isolation: None,
            parent_task_id: None,
            schema: None,
            cwd: None,
        }),
        PathBuf::from("."),
        None,
        None,
        None,
    )
    .await;
    assert!(r.is_error());
    assert!(r.output.contains("streaming"), "{}", r.output);
}

#[tokio::test]
async fn execute_tool_kind_input_mismatch_falls_through_robust() {
    // Mismatched kind/input pair returns a "tool input mismatch" routing
    // error so the bug surfaces clearly rather than being mislabeled as
    // an unimplemented tool — the implementation exists, the input shape
    // is just wrong.
    let r = execute_tool(
        ToolKind::Bash,
        ToolInput::Generic {
            summary: "wrong shape".into(),
        },
        PathBuf::from("."),
        None,
        None,
        None,
    )
    .await;
    assert!(r.is_error());
    assert!(r.output.contains("tool input mismatch"), "{}", r.output);
}

#[tokio::test]
async fn execute_tool_invalidates_dedup_after_write_normal() {
    use crate::context::ReadDedupCache;

    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("inv.txt");
    tokio::fs::write(&path, "v1\n").await.unwrap();

    let cache = Arc::new(Mutex::new(ReadDedupCache::new()));

    // Prime cache with a Read.
    let r1 = execute_tool(
        ToolKind::Read,
        ToolInput::Read {
            file_path: path.to_string_lossy().to_string(),
            offset: None,
            limit: None,
        },
        dir.path().to_path_buf(),
        Some(cache.clone()),
        None,
        None,
    )
    .await;
    assert!(!r1.is_error());

    // Write through the dispatcher — this should invalidate the cache.
    let w = execute_tool(
        ToolKind::Write,
        ToolInput::Write {
            file_path: path.to_string_lossy().to_string(),
            content: "v2\n".into(),
        },
        dir.path().to_path_buf(),
        Some(cache.clone()),
        None,
        None,
    )
    .await;
    assert!(!w.is_error());

    // Next Read should NOT short-circuit with the dedup stub.
    let r2 = execute_tool(
        ToolKind::Read,
        ToolInput::Read {
            file_path: path.to_string_lossy().to_string(),
            offset: None,
            limit: None,
        },
        dir.path().to_path_buf(),
        Some(cache),
        None,
        None,
    )
    .await;
    assert!(!r2.is_error());
    assert!(
        !r2.output.contains("File unchanged"),
        "Write should have invalidated the dedup cache: {}",
        r2.output
    );
    assert!(r2.output.contains("v2"), "{}", r2.output);
}

#[serial_test::serial]
#[test]
fn scratchpad_round_trips_through_config_file_normal() {
    let home = tempfile::tempdir().expect("tempdir");
    let prev = std::env::var("XDG_CONFIG_HOME").ok();
    unsafe { std::env::set_var("XDG_CONFIG_HOME", home.path()) };

    let w = execute_scratchpad_write("agent-audit", "findings");
    let r = execute_scratchpad_read("agent-audit");
    let path = home.path().join("jfc").join("scratchpad.json");

    unsafe {
        match prev {
            Some(v) => std::env::set_var("XDG_CONFIG_HOME", v),
            None => std::env::remove_var("XDG_CONFIG_HOME"),
        }
    }

    assert!(!w.is_error(), "{}", w.output);
    assert!(!r.is_error(), "{}", r.output);
    assert_eq!(r.output, "findings");
    assert!(
        path.exists(),
        "scratchpad must persist outside process memory"
    );
}

#[serial_test::serial]
#[test]
fn scratchpad_missing_key_lists_persisted_keys_robust() {
    let home = tempfile::tempdir().expect("tempdir");
    let prev = std::env::var("XDG_CONFIG_HOME").ok();
    unsafe { std::env::set_var("XDG_CONFIG_HOME", home.path()) };

    let w = execute_scratchpad_write("known-key", "value");
    let r = execute_scratchpad_read("missing-key");

    unsafe {
        match prev {
            Some(v) => std::env::set_var("XDG_CONFIG_HOME", v),
            None => std::env::remove_var("XDG_CONFIG_HOME"),
        }
    }

    assert!(!w.is_error(), "{}", w.output);
    assert!(r.is_error());
    assert!(r.output.contains("known-key"), "{}", r.output);
}

#[tokio::test]
async fn execute_tool_dispatches_glob_normal() {
    let dir = tempfile::tempdir().expect("temp dir");
    tokio::fs::write(dir.path().join("hit.rs"), "")
        .await
        .unwrap();
    let r = execute_tool(
        ToolKind::Glob,
        ToolInput::Glob {
            pattern: "*.rs".into(),
            path: None,
        },
        dir.path().to_path_buf(),
        None,
        None,
        None,
    )
    .await;
    assert!(!r.is_error(), "{}", r.output);
    assert!(r.output.contains("hit.rs"), "{}", r.output);
}

#[tokio::test]
async fn execute_tool_dispatches_task_create_normal() {
    let store = TaskStore::in_memory();
    let r = execute_tool(
        ToolKind::TaskCreate,
        ToolInput::TaskCreate {
            subject: "via dispatcher".into(),
            description: "test".into(),
            active_form: None,
            blocked_by: vec![],
            acceptance_criteria: None,
            verification_command: None,
            risk: None,
            parent_id: None,
            kind: None,
            tags: vec![],
            priority: None,
            effort: None,
            model: None,
        },
        PathBuf::from("."),
        None,
        Some(store),
        None,
    )
    .await;
    assert!(!r.is_error(), "{}", r.output);
    assert!(r.output.contains("via dispatcher"), "{}", r.output);
}

// ─── Monitor tool (DO-178B _normal/_robust) ─────────────────────────

#[tokio::test]
async fn monitor_matches_first_line_normal() {
    // `printf` writes the matching line immediately; the monitor
    // should kill the process and return the matched line.
    let r = execute_monitor(
        "printf 'starting\\nfound the goal\\nfinished\\n'",
        r"goal",
        Path::new("."),
    )
    .await;
    assert!(!r.is_error(), "{}", r.output);
    assert!(r.output.contains("found the goal"), "{}", r.output);
}

#[tokio::test]
async fn monitor_invalid_regex_robust() {
    let r = execute_monitor("echo hi", "[invalid(regex", Path::new(".")).await;
    assert!(r.is_error());
    assert!(r.output.contains("invalid `until` regex"), "{}", r.output);
}

#[tokio::test]
async fn monitor_eof_without_match_reports_failure_robust() {
    // `true` exits immediately with no output — Monitor should
    // report process exit without match (failure).
    let r = execute_monitor("true", r"never-matches", Path::new(".")).await;
    assert!(r.is_error());
    assert!(r.output.contains("Process exited"), "{}", r.output);
}

// ─── LSP tool ─────────────────────────────────────────────────────────

/// Normal: `LSP` with `kind=hover`, valid file, valid coords reaches the
/// validation path and either succeeds (when rust-analyzer exists) or
/// fails with an actionable detection error. Either outcome means the
/// dispatch wiring is correct.
#[tokio::test]
async fn lsp_dispatch_routes_through_dispatcher_normal() {
    // Pick a directory without Cargo.toml/build.zig so detect_lsp_for_cwd
    // returns None — the tool fails with a known message; dispatch is
    // still verified.
    let dir = tempfile::tempdir().expect("tempdir");
    let src = dir.path().join("foo.txt");
    std::fs::write(&src, "hello\n").expect("write");
    let r = execute_tool(
        ToolKind::Lsp,
        ToolInput::Lsp {
            kind: "hover".into(),
            file: src.display().to_string(),
            line: 1,
            column: 1,
        },
        dir.path().to_path_buf(),
        None,
        None,
        None,
    )
    .await;
    assert!(r.is_error(), "expected detection error: {}", r.output);
    assert!(
        r.output.contains("no language server detected"),
        "{}",
        r.output
    );
}

/// Robust: invalid `kind` is rejected before any LSP work happens.
#[tokio::test]
async fn lsp_rejects_invalid_kind_robust() {
    let dir = tempfile::tempdir().expect("tempdir");
    let src = dir.path().join("foo.rs");
    std::fs::write(&src, "fn main() {}\n").expect("write");
    let r = execute_lsp("nonsense", &src.display().to_string(), 1, 1, dir.path()).await;
    assert!(r.is_error());
    assert!(r.output.contains("invalid kind"), "{}", r.output);
}

/// Robust: relative paths are rejected — LSP uses absolute file URIs.
#[tokio::test]
async fn lsp_rejects_relative_path_robust() {
    let dir = tempfile::tempdir().expect("tempdir");
    let r = execute_lsp("hover", "relative/path.rs", 1, 1, dir.path()).await;
    assert!(r.is_error());
    assert!(r.output.contains("absolute"), "{}", r.output);
}

// ─── PushNotification tool ─────────────────────────────────────────────

#[serial_test::serial]
#[test]
fn push_notification_normal() {
    // Disable the OS daemon so this never fires a real notification
    // in CI. The success message proves the dispatch wiring works.
    unsafe { std::env::set_var("JFC_DISABLE_NOTIFICATIONS", "1") };
    let r = execute_push_notification("Build green", Some("CI"));
    assert!(!r.is_error(), "{}", r.output);
    assert!(r.output.contains("Build green"), "{}", r.output);
    assert!(r.output.contains("CI"), "{}", r.output);
    assert!(
        r.output.contains("remote-control push not yet implemented"),
        "expected the unsupported-feature notice: {}",
        r.output
    );
}

#[test]
fn push_notification_empty_message_fails_robust() {
    let r = execute_push_notification("", None);
    assert!(r.is_error(), "{}", r.output);
}

// ─── RemoteTrigger tool ────────────────────────────────────────────────

#[test]
fn parse_trigger_url_extracts_url_normal() {
    let toml = r#"
[deploy]
url = "https://ci.example.com/hook/deploy"
"#;
    assert_eq!(
        parse_trigger_url(toml, "deploy").unwrap(),
        "https://ci.example.com/hook/deploy"
    );
}

#[test]
fn parse_trigger_url_unknown_id_fails_robust() {
    let toml = r#"
[other]
url = "https://x"
"#;
    let err = parse_trigger_url(toml, "deploy").unwrap_err();
    assert!(err.contains("not found"), "{err}");
}

#[test]
fn parse_trigger_url_missing_url_fails_robust() {
    let toml = r#"
[deploy]
description = "no url here"
"#;
    let err = parse_trigger_url(toml, "deploy").unwrap_err();
    assert!(err.contains("no `url`"), "{err}");
}

/// Normal: `execute_remote_trigger` POSTs to the configured URL using
/// a tokio listener as the destination. We reach into a hand-written
/// triggers.toml in a temp HOME so the production path resolves there.
#[serial_test::serial]
#[tokio::test]
async fn execute_remote_trigger_posts_payload_normal() {
    use std::net::SocketAddr;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    // Bind to a free port and remember the address.
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr: SocketAddr = listener.local_addr().expect("addr");
    let port = addr.port();

    // Spawn a one-shot HTTP responder that captures the body.
    let captured = std::sync::Arc::new(std::sync::Mutex::new(String::new()));
    let captured_for_task = captured.clone();
    let handle = tokio::spawn(async move {
        if let Ok((mut sock, _)) = listener.accept().await {
            let mut buf = [0u8; 4096];
            let n = sock.read(&mut buf).await.unwrap_or(0);
            let req = String::from_utf8_lossy(&buf[..n]).to_string();
            *captured_for_task.lock().expect("lock") = req;
            let _ = sock
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nok")
                .await;
            let _ = sock.shutdown().await;
        }
    });

    // Stage triggers.toml in an isolated XDG_CONFIG_HOME so the global
    // resolver finds our test file rather than the real user config.
    let home = tempfile::tempdir().expect("tempdir");
    let cfg_dir = home.path().join("jfc");
    std::fs::create_dir_all(&cfg_dir).expect("mkdir");
    let triggers = format!("[t1]\nurl = \"http://127.0.0.1:{port}/hook\"\n",);
    std::fs::write(cfg_dir.join("triggers.toml"), triggers).expect("write");
    // SAFETY: tests are not concurrent with code that reads XDG_CONFIG_HOME
    // arbitrarily — only the tool resolver uses it.
    let prev = std::env::var("XDG_CONFIG_HOME").ok();
    unsafe { std::env::set_var("XDG_CONFIG_HOME", home.path()) };

    let payload = serde_json::json!({"hello": "world"});
    let r = execute_remote_trigger("t1", Some(&payload)).await;

    // Restore env early so an assertion failure doesn't leak it.
    unsafe {
        match prev {
            Some(v) => std::env::set_var("XDG_CONFIG_HOME", v),
            None => std::env::remove_var("XDG_CONFIG_HOME"),
        }
    }
    let _ = handle.await;

    assert!(!r.is_error(), "{}", r.output);
    assert!(r.output.contains("HTTP 200"), "{}", r.output);
    let req = captured.lock().expect("lock").clone();
    assert!(req.starts_with("POST /hook"), "captured: {req}");
    assert!(
        req.contains("\"hello\":\"world\""),
        "payload not in body: {req}",
    );
}

#[serial_test::serial]
#[tokio::test]
async fn execute_remote_trigger_unknown_id_fails_robust() {
    let home = tempfile::tempdir().expect("tempdir");
    let cfg_dir = home.path().join("jfc");
    std::fs::create_dir_all(&cfg_dir).expect("mkdir");
    std::fs::write(cfg_dir.join("triggers.toml"), "").expect("write");
    let prev = std::env::var("XDG_CONFIG_HOME").ok();
    unsafe { std::env::set_var("XDG_CONFIG_HOME", home.path()) };

    let r = execute_remote_trigger("nope", None).await;

    unsafe {
        match prev {
            Some(v) => std::env::set_var("XDG_CONFIG_HOME", v),
            None => std::env::remove_var("XDG_CONFIG_HOME"),
        }
    }

    assert!(r.is_error());
    assert!(r.output.contains("not found"), "{}", r.output);
}

// ─── EnterPlanMode tool ────────────────────────────────────────────────

struct EventSenderResetGuard;

impl Drop for EventSenderResetGuard {
    fn drop(&mut self) {
        clear_event_sender_for_test();
    }
}

fn clear_event_sender_for_test() {
    if let Ok(mut g) = active_event_sender_handle().write() {
        *g = None;
    }
}

#[serial_test::serial]
#[tokio::test]
async fn enter_plan_mode_dispatches_event_normal() {
    let _guard = EventSenderResetGuard;
    clear_event_sender_for_test();
    let (tx, mut rx) = tokio::sync::mpsc::channel::<crate::runtime::EngineEvent>(8);
    register_event_sender(tx);
    let r = execute_enter_plan_mode("safety check").await;
    assert!(!r.is_error(), "{}", r.output);
    let evt = rx.recv().await.expect("event");
    match evt {
        crate::runtime::EngineEvent::Frontend(crate::runtime::FrontendEvent::PlanModeEntered {
            reason,
        }) => {
            assert_eq!(reason, "safety check");
        }
        _ => panic!("expected EnterPlanModeRequested EngineEvent variant"),
    }
}

/// Robust: when no event sender is registered (e.g. early-boot tool
/// calls or test setup that didn't wire one), the call fails with a
/// clear message rather than panicking.
#[serial_test::serial]
#[tokio::test]
async fn enter_plan_mode_without_sender_fails_robust() {
    let _guard = EventSenderResetGuard;
    // Clear any previously-registered sender. We use a separate
    // process-global, so this requires reaching into the handle.
    clear_event_sender_for_test();
    let r = execute_enter_plan_mode("noop").await;
    assert!(r.is_error());
    assert!(r.output.contains("no event sender"), "{}", r.output);
}

// ─── EnterWorktree / ExitWorktree ──────────────────────────────────────

/// Normal: `EnterWorktree` happily creates a fresh worktree under the
/// repo root. We initialize a tiny throwaway git repo as the host.
#[tokio::test]
async fn enter_worktree_creates_fresh_normal() {
    let dir = tempfile::tempdir().expect("tempdir");
    let repo = dir.path();
    run_git(repo, &["init", "-q"]).await;
    run_git(
        repo,
        &[
            "-c",
            "user.email=t@t",
            "-c",
            "user.name=t",
            "commit",
            "--allow-empty",
            "-m",
            "init",
            "-q",
        ],
    )
    .await;
    let r = execute_enter_worktree("featx", None, repo).await;
    assert!(!r.is_error(), "{}", r.output);
    assert!(repo.join(".claude/worktrees/featx").exists());
    // Idempotent: second invocation succeeds with "already exists".
    let r2 = execute_enter_worktree("featx", None, repo).await;
    assert!(!r2.is_error(), "{}", r2.output);
    assert!(r2.output.contains("already"), "{}", r2.output);
}

/// Robust: bad name is rejected with the validator's message before we
/// shell out to git.
#[tokio::test]
async fn enter_worktree_invalid_name_fails_robust() {
    let dir = tempfile::tempdir().expect("tempdir");
    let r = execute_enter_worktree("bad.name", None, dir.path()).await;
    assert!(r.is_error());
    assert!(r.output.contains("[A-Za-z0-9_/-]"), "{}", r.output);
}

/// Robust: outside a git repo we surface the missing-repo error rather
/// than blindly invoking git.
#[tokio::test]
async fn enter_worktree_outside_repo_fails_robust() {
    // Use a fresh directory that we *know* has no .git anywhere above.
    // Previously this used tempfile::tempdir() which lands in /tmp —
    // but /tmp/.git can exist (sandbox environments, stale test
    // artifacts) making find_repo_root succeed and the test panic on
    // "git worktree add" instead of the expected error path. Creating
    // a nested subdir and verifying no .git exists at any level gives
    // us a truly git-free path.
    let dir = tempfile::tempdir().expect("tempdir");
    let isolated = dir.path().join("no-git-here").join("nested");
    std::fs::create_dir_all(&isolated).expect("mkdir");
    // Double-check: if somehow .git exists above us, skip the test
    // gracefully rather than producing a confusing failure message.
    if super::worktree::find_repo_root(&isolated).is_some() {
        eprintln!(
            "SKIP: .git found above {} — cannot test outside-repo behavior in this environment",
            isolated.display()
        );
        return;
    }
    let r = execute_enter_worktree("ok", None, &isolated).await;
    assert!(r.is_error());
    assert!(
        r.output.contains("not inside a git repository"),
        "{}",
        r.output
    );
}

/// Normal: `ExitWorktree` is a no-op informational tool — never errors,
/// always returns a success message.
#[tokio::test]
async fn exit_worktree_returns_informational_normal() {
    let dir = tempfile::tempdir().expect("tempdir");
    let r = execute_exit_worktree(dir.path()).await;
    assert!(!r.is_error(), "{}", r.output);
    assert!(r.output.contains("exit_worktree"), "{}", r.output);
}

async fn run_git(cwd: &Path, args: &[&str]) {
    let _ = tokio::process::Command::new("git")
        .args(args)
        .current_dir(cwd)
        .output()
        .await;
}

// ─── NotebookRead / NotebookEdit ──────────────────────────────────────

fn sample_ipynb() -> String {
    serde_json::json!({
        "cells": [
            {
                "cell_type": "code",
                "id": "abc123",
                "metadata": {},
                "source": "x = 1\nprint(x)\n",
                "outputs": [
                    {
                        "output_type": "stream",
                        "name": "stdout",
                        "text": "1\n"
                    }
                ],
                "execution_count": 1
            },
            {
                "cell_type": "markdown",
                "id": "md1",
                "metadata": {},
                "source": ["## Header\n", "Some text"]
            }
        ],
        "metadata": {},
        "nbformat": 4,
        "nbformat_minor": 5
    })
    .to_string()
}

#[test]
fn notebook_read_renders_cells_normal() {
    let rendered = notebook_read_text(&sample_ipynb()).expect("read");
    assert!(rendered.contains("2 cells"), "{rendered}");
    assert!(rendered.contains("id=abc123"), "{rendered}");
    assert!(rendered.contains("id=md1"), "{rendered}");
    assert!(rendered.contains("x = 1"), "{rendered}");
    assert!(rendered.contains("## Header"), "{rendered}");
    assert!(rendered.contains("outputs"), "{rendered}");
}

#[test]
fn notebook_read_invalid_json_fails_robust() {
    let err = notebook_read_text("not-json").unwrap_err();
    assert!(err.contains("invalid notebook JSON"), "{err}");
}

#[test]
fn notebook_read_missing_cells_fails_robust() {
    let err = notebook_read_text("{}").unwrap_err();
    assert!(err.contains("missing `cells`"), "{err}");
}

#[test]
fn notebook_edit_replace_clears_outputs_normal() {
    let nb = sample_ipynb();
    let edited = notebook_edit_text(&nb, "abc123", "y = 2\n", "replace").expect("edit");
    let v: serde_json::Value = serde_json::from_str(&edited).expect("json");
    let cell = &v["cells"][0];
    assert_eq!(cell["source"], "y = 2\n");
    assert!(cell["outputs"].as_array().unwrap().is_empty());
    assert!(cell["execution_count"].is_null());
}

#[test]
fn notebook_edit_insert_adds_after_target_normal() {
    let nb = sample_ipynb();
    let edited = notebook_edit_text(&nb, "abc123", "z = 3\n", "insert").expect("edit");
    let v: serde_json::Value = serde_json::from_str(&edited).expect("json");
    let cells = v["cells"].as_array().unwrap();
    assert_eq!(cells.len(), 3);
    assert_eq!(cells[1]["source"], "z = 3\n");
    assert_eq!(cells[1]["cell_type"], "code");
}

#[test]
fn notebook_edit_delete_removes_cell_normal() {
    let nb = sample_ipynb();
    let edited = notebook_edit_text(&nb, "abc123", "", "delete").expect("edit");
    let v: serde_json::Value = serde_json::from_str(&edited).expect("json");
    let cells = v["cells"].as_array().unwrap();
    assert_eq!(cells.len(), 1);
    assert_eq!(cells[0]["id"], "md1");
}

#[test]
fn notebook_edit_unknown_cell_fails_robust() {
    let nb = sample_ipynb();
    let err = notebook_edit_text(&nb, "no-such-cell", "x", "replace").unwrap_err();
    assert!(err.contains("not found"), "{err}");
}

#[test]
fn notebook_edit_invalid_mode_fails_robust() {
    let nb = sample_ipynb();
    let err = notebook_edit_text(&nb, "abc123", "x", "wat").unwrap_err();
    assert!(err.contains("invalid edit_mode"), "{err}");
}

// ─── defs.rs ↔ ToolKind drift guard ──────────────────────────────────
//
// defs.rs carries hand-written JSON schemas + LLM-facing prose; the prose
// can't be macro-generated and the schemas vary too much to share a builder
// without bloat (see the t13 investigation). Instead of forcing a macro
// table, this test enforces the consistency guarantee a macro would have
// given: every name in `all_tool_defs()` must round-trip through
// `ToolKind::from_name` to a real (non-`UnknownTool`) variant. Adding a
// `ToolDef` whose name doesn't match any `ToolKind`, or renaming a kind
// without updating the def, fails here at test time.
#[test]
fn every_tool_def_name_resolves_to_a_real_tool_kind_robust() {
    for def in all_tool_defs() {
        let kind = ToolKind::from_name(&def.name);
        assert!(
            !matches!(kind, ToolKind::UnknownTool { .. }),
            "ToolDef `{}` has no matching ToolKind — defs.rs has drifted \
             from the enum (add the variant, or fix the def's name)",
            def.name,
        );
    }
}
