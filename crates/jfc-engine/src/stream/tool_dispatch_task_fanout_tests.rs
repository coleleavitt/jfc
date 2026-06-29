use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use async_trait::async_trait;
use jfc_provider::{
    EventStream, ModelId, ModelInfo, Provider, ProviderMessage, StreamConvention, StreamOptions,
};
use tokio::sync::{Mutex, mpsc};
use tokio_util::sync::CancellationToken;

use super::{ToolBatchDispatch, dispatch_tools_batched};
use crate::context::ReadDedupCache;
use crate::runtime::{EngineEvent, TaskEvent};
use crate::types::{ToolCall, ToolInput, ToolKind};

struct EmptyProvider;

#[async_trait]
impl Provider for EmptyProvider {
    fn name(&self) -> &str {
        "empty"
    }

    fn available_models(&self) -> Vec<ModelInfo> {
        Vec::new()
    }

    fn stream_convention(&self) -> StreamConvention {
        StreamConvention::AnthropicNative
    }

    async fn stream(
        &self,
        _messages: Vec<ProviderMessage>,
        _options: &StreamOptions,
    ) -> Result<EventStream> {
        Ok(Box::pin(futures::stream::empty()))
    }
}

impl jfc_provider::seal::Sealed for EmptyProvider {}

fn task_call(id: &str, description: &str) -> ToolCall {
    ToolCall::new_pending(
        crate::ids::ToolId::from(id),
        ToolKind::Task,
        ToolInput::Task(crate::types::TaskInput {
            description: description.to_owned(),
            prompt: description.to_owned(),
            subagent_type: None,
            category: None,
            run_in_background: false,
            model: None,
            launcher: None,
            effort: None,
            name: None,
            team_name: None,
            mode: None,
            isolation: Some("none".to_owned()),
            parent_task_id: None,
            schema: None,
            allowed_tools: Vec::new(),
            disallowed_tools: Vec::new(),
            cwd: None,
        }),
    )
}

fn background_task_call_with_launcher(
    id: &str,
    description: &str,
    launcher: &str,
    cwd: &std::path::Path,
) -> ToolCall {
    let mut call = task_call(id, description);
    if let ToolInput::Task(task) = &mut call.input {
        task.run_in_background = true;
        task.launcher = Some(launcher.to_owned());
        task.cwd = Some(cwd.to_string_lossy().into_owned());
    }
    call
}

#[tokio::test(flavor = "current_thread")]
async fn dispatch_tools_batched_starts_every_task_in_batch_regression() {
    let (tx, mut rx) = mpsc::channel(32);
    let (teammate_tx, _teammate_rx) = mpsc::unbounded_channel();
    let provider = Arc::new(EmptyProvider) as Arc<dyn Provider>;

    dispatch_tools_batched(
        vec![
            task_call("task_a", "inspect a"),
            task_call("task_b", "inspect b"),
        ],
        ToolBatchDispatch {
            tx,
            dedup: Arc::new(Mutex::new(ReadDedupCache::default())),
            task_store: Some(jfc_session::TaskStore::in_memory()),
            active_team_name: None,
            current_session_id: None,
            provider,
            model: ModelId::new("empty-model"),
            providers: Vec::new(),
            teammate_event_tx: teammate_tx,
            local_advisor: None,
            cancel: CancellationToken::new(),
        },
    );

    let started = tokio::time::timeout(Duration::from_secs(5), async {
        let mut ids = Vec::new();
        while ids.len() < 2 {
            match rx.recv().await {
                Some(EngineEvent::Task(TaskEvent::Started { task_id, .. })) => {
                    ids.push(task_id.as_str().to_owned());
                }
                Some(_) => {}
                None => break,
            }
        }
        ids
    })
    .await
    .expect("dispatcher should emit two TaskStarted events");

    assert_eq!(started.len(), 2, "expected one TaskStarted per Task tool");
    assert!(started.contains(&"task_a".to_owned()));
    assert!(started.contains(&"task_b".to_owned()));
}

#[test]
fn background_task_call_with_launcher_preserves_task_metadata_normal() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let call = background_task_call_with_launcher(
        "task_bg",
        "inspect in background",
        "variant-agent",
        dir.path(),
    );

    match call.input {
        ToolInput::Task(task) => {
            assert!(task.run_in_background);
            assert_eq!(task.launcher.as_deref(), Some("variant-agent"));
            assert_eq!(
                task.cwd.as_deref(),
                Some(dir.path().to_string_lossy().as_ref())
            );
        }
        _ => panic!("expected Task input"),
    }
}
