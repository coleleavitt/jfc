use super::*;
use jfc_provider::{EventStream, ModelInfo, Provider, ProviderMessage, StreamOptions};
use std::process::Command;
use std::sync::Arc;

struct KnowledgeDbEnvGuard {
    prior: Option<std::ffi::OsString>,
    _dir: tempfile::TempDir,
}

impl KnowledgeDbEnvGuard {
    fn new() -> Self {
        let dir = tempfile::tempdir().expect("tempdir");
        let prior = std::env::var_os("JFC_KNOWLEDGE_DB");
        // SAFETY: Category 13, library contract. These tests construct the
        // guard only inside serial test cases before starting async work, so
        // the guard owns this environment override until Drop restores it.
        unsafe { std::env::set_var("JFC_KNOWLEDGE_DB", dir.path().join("knowledge.db")) };
        Self { prior, _dir: dir }
    }
}

impl Drop for KnowledgeDbEnvGuard {
    fn drop(&mut self) {
        // SAFETY: Category 13, library contract. The matching serial test still
        // owns the process environment mutation, and this restores the exact
        // prior value before the next serial test can enter.
        unsafe {
            match &self.prior {
                Some(prior) => std::env::set_var("JFC_KNOWLEDGE_DB", prior),
                None => std::env::remove_var("JFC_KNOWLEDGE_DB"),
            }
        }
    }
}

fn git(dir: &Path, args: &[&str]) {
    let status = Command::new("git")
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
        .status()
        .expect("git should run");
    assert!(status.success(), "git {args:?} failed");
}

fn temp_repo() -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path();
    git(path, &["init", "-q"]);
    git(path, &["config", "user.email", "t@t"]);
    git(path, &["config", "user.name", "t"]);
    std::fs::write(path.join("context.txt"), "context").expect("write fixture");
    git(path, &["add", "."]);
    git(
        path,
        &[
            "commit",
            "-q",
            "-m",
            "fix: repair context body overflow\n\nStops request_too_large loops.",
        ],
    );
    dir
}

struct TestProvider;

impl jfc_provider::seal::Sealed for TestProvider {}

#[async_trait::async_trait]
impl Provider for TestProvider {
    fn name(&self) -> &str {
        "test"
    }

    fn available_models(&self) -> Vec<ModelInfo> {
        Vec::new()
    }

    async fn stream(
        &self,
        _messages: Vec<ProviderMessage>,
        _options: &StreamOptions,
    ) -> anyhow::Result<EventStream> {
        Ok(Box::pin(futures::stream::empty()))
    }
}

#[test]
#[serial_test::serial]
fn collect_context_hits_returns_git_commit_when_session_db_empty_regression() {
    let _env = KnowledgeDbEnvGuard::new();
    let repo = temp_repo();

    let hits = collect_context_hits(repo.path(), "request_too_large", None, 10);

    assert!(
        matches!(hits.first(), Some(ContextSearchHit::Commit(hit)) if hit.snippet.contains("request_too_large"))
    );
}

#[test]
fn format_context_search_results_is_bounded_and_actionable_normal() {
    let hit = ContextSearchHit::Commit(CommitHit {
        short_hash: "abc1234".to_owned(),
        date: "2026-06-27T00:00:00Z".to_owned(),
        subject: "fix: repair context body overflow".to_owned(),
        snippet: "Stops request_too_large loops.".to_owned(),
    });

    let formatted = format_context_search_results("request_too_large", &[hit]);

    assert!(formatted.contains("[git_commit] `abc1234`"));
    assert!(formatted.contains("git show <sha>"));
}

#[test]
fn format_context_search_results_points_archive_hits_to_expand_normal() {
    let hit = ContextSearchHit::ProviderHistoryArchive(ProviderHistoryArchiveHit {
        id: "provider-history-abc".to_owned(),
        created_at: "2026-06-28T00:00:00Z".to_owned(),
        message_count: 2,
        snippet: "archived exact replay".to_owned(),
    });

    let formatted = format_context_search_results("archived", &[hit]);

    assert!(formatted.contains("[provider_history_archive] `provider-history-abc`"));
    assert!(formatted.contains("/expand <archive-id>"));
}

#[tokio::test]
#[serial_test::serial]
async fn ctx_search_command_returns_git_context_through_slash_surface_regression() {
    let _env = KnowledgeDbEnvGuard::new();
    let repo = temp_repo();
    let mut state = EngineState::new(Arc::new(TestProvider), "test-model");
    state.cwd = repo.path().display().to_string();

    let outcome =
        crate::commands::run_command(&mut state, "/ctx-search request_too_large", None).await;

    let body = state
        .messages
        .last()
        .map(|message| {
            message
                .parts
                .iter()
                .map(|part| part.text_only())
                .collect::<String>()
        })
        .unwrap_or_default();
    assert_eq!(outcome, crate::commands::CommandOutcome::Handled);
    assert!(body.contains("[git_commit]"));
    assert!(body.contains("request_too_large"));
}
