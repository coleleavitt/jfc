use std::sync::Arc;

use async_trait::async_trait;
use jfc_core::TaskInput;
use jfc_provider::{
    CompletionResponse, EventStream, ModelId, ModelInfo, Provider, ProviderMessage as PMsg,
    StreamConvention, StreamOptions as SOpts,
};
use tokio::sync::{mpsc, mpsc::UnboundedSender};

use super::{bind_teammate_provider, try_spawn_teammate};
use crate::runtime::{EngineEvent, ToolEvent};
use crate::swarm::runner::TeammateEvent;

struct NamedProvider {
    name: &'static str,
}

#[async_trait]
impl Provider for NamedProvider {
    fn name(&self) -> &str {
        self.name
    }

    fn available_models(&self) -> Vec<ModelInfo> {
        Vec::new()
    }

    fn stream_convention(&self) -> StreamConvention {
        StreamConvention::AnthropicNative
    }

    async fn stream(&self, _m: Vec<PMsg>, _o: &SOpts) -> anyhow::Result<EventStream> {
        anyhow::bail!("unused")
    }

    async fn complete(&self, _m: Vec<PMsg>, _o: &SOpts) -> anyhow::Result<CompletionResponse> {
        anyhow::bail!("unused")
    }
}

impl jfc_provider::seal::Sealed for NamedProvider {}

fn registry() -> Vec<Arc<dyn Provider>> {
    vec![
        Arc::new(NamedProvider { name: "openai" }) as Arc<dyn Provider>,
        Arc::new(NamedProvider { name: "anthropic" }) as Arc<dyn Provider>,
    ]
}

#[test]
fn teammate_bound_to_its_own_provider_normal() {
    let leader = Arc::new(NamedProvider { name: "anthropic" }) as Arc<dyn Provider>;
    let (provider, model) =
        bind_teammate_provider(&registry(), leader, ModelId::new("openai/gpt-5.5"));
    assert_eq!(provider.name(), "openai");
    assert_eq!(model.as_str(), "gpt-5.5");
}

#[test]
fn teammate_falls_back_to_leader_when_unresolved_robust() {
    let leader = Arc::new(NamedProvider { name: "anthropic" }) as Arc<dyn Provider>;
    let (provider, model) =
        bind_teammate_provider(&registry(), leader, ModelId::new("mystery-model"));
    assert_eq!(provider.name(), "anthropic");
    assert_eq!(model.as_str(), "mystery-model");
}

#[test]
fn teammate_falls_back_with_empty_registry_robust() {
    let leader = Arc::new(NamedProvider { name: "openai" }) as Arc<dyn Provider>;
    let (provider, model) = bind_teammate_provider(&[], leader, ModelId::new("openai/gpt-5.5"));
    assert_eq!(provider.name(), "openai");
    assert_eq!(model.as_str(), "openai/gpt-5.5");
}

#[tokio::test(flavor = "current_thread")]
async fn teammate_spawn_runs_plugin_process_bridge_lifecycle_normal() {
    // Given: a teammate Task names a project plugin process-bridge launcher.
    jfc_plugin_host::clear_discovered_plugin_state_cache_for_tests();
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let request_file = tmp.path().join("bridge-request.jsonl");
    install_variant_launcher(tmp.path(), &request_file);
    let (tx, mut rx) = mpsc::channel(8);
    let (teammate_tx, mut teammate_rx): (UnboundedSender<TeammateEvent>, _) =
        mpsc::unbounded_channel();
    let mut task = teammate_task_input();
    task.cwd = Some(tmp.path().to_string_lossy().into_owned());
    task.launcher = Some("variant-agent".to_owned());
    let leader = Arc::new(NamedProvider { name: "anthropic" }) as Arc<dyn Provider>;

    // When: the teammate spawn path resolves and starts the launcher.
    let handled = try_spawn_teammate(
        &task,
        "toolu_teammate",
        &tx,
        leader,
        ModelId::new("claude-opus-4-7"),
        &[],
        Some("session_1"),
        teammate_tx,
        &[],
        || {},
    );

    // Then: the plugin process bridge owns the live teammate route.
    assert!(handled);
    let mut abort_tx = None;
    let mut saw_task_started = false;
    let mut saw_tool_success = false;
    for _ in 0..3 {
        match rx.recv().await.expect("spawn event") {
            EngineEvent::Team(crate::runtime::TeamEvent::Spawned {
                agent_id,
                abort_tx: next_abort_tx,
                ..
            }) => {
                assert_eq!(agent_id, "reviewer@alpha");
                abort_tx = next_abort_tx;
            }
            EngineEvent::Task(crate::runtime::TaskEvent::Started { task_id, .. }) => {
                assert_eq!(task_id.as_str(), "teammate-reviewer@alpha");
                saw_task_started = true;
            }
            EngineEvent::Tool(ToolEvent::Result { result, .. }) => {
                assert!(!result.is_error());
                assert!(result.output.contains("variant-agent"));
                assert!(result.output.contains("process_bridge"));
                saw_tool_success = true;
            }
            _ => panic!("expected teammate lifecycle event"),
        }
    }
    assert!(abort_tx.is_some());
    assert!(saw_task_started);
    assert!(saw_tool_success);
    let request = tokio::time::timeout(std::time::Duration::from_secs(2), async {
        loop {
            if let Ok(request) = std::fs::read_to_string(&request_file) {
                break request;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("bridge request");
    assert!(request.contains(r#""kind":"agent_launch""#));
    assert!(request.contains(r#""launcher":"variant-agent""#));

    abort_tx
        .expect("abort handle")
        .send(true)
        .expect("abort send");
    match tokio::time::timeout(std::time::Duration::from_secs(2), teammate_rx.recv())
        .await
        .expect("cancel event")
        .expect("teammate event")
    {
        TeammateEvent::Cancelled { agent_id, .. } => assert_eq!(agent_id, "reviewer@alpha"),
        event => panic!("expected cancelled event, got {event:?}"),
    }
}

fn teammate_task_input() -> TaskInput {
    TaskInput {
        description: "spawn teammate".to_owned(),
        prompt: "inspect".to_owned(),
        subagent_type: None,
        category: None,
        run_in_background: false,
        model: None,
        launcher: None,
        effort: None,
        name: Some("reviewer".to_owned()),
        team_name: Some("alpha".to_owned()),
        mode: None,
        isolation: None,
        parent_task_id: None,
        schema: None,
        allowed_tools: Vec::new(),
        disallowed_tools: Vec::new(),
        cwd: None,
    }
}

fn install_variant_launcher(project_root: &std::path::Path, request_file: &std::path::Path) {
    let plugin = project_root.join("plugins").join("agent-plugin");
    std::fs::create_dir_all(plugin.join("workflows")).expect("workflows dir");
    let script = format!(
        "read line\nprintf \"%s\\n\" \"$line\" > \"{}\"\nwhile true; do sleep 1; done",
        request_file.display()
    );
    let handler = serde_json::json!({
        "command": "sh",
        "args": ["-c", script],
    });
    std::fs::write(
        plugin.join(".jfc-plugin.toml"),
        format!(
            r#"[plugin]
name = "agent-plugin"
workflows_dir = "workflows"

[[agent_launches]]
name = "variant-agent"
label = "Variant Agent"
description = "Launches a plugin-defined variant agent."

[agent_launches.executor]
kind = "process_bridge"
handler = '{}'
"#,
            handler
        ),
    )
    .expect("write plugin manifest");
}
