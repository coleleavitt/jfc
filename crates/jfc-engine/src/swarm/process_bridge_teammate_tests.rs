use jfc_provider::ModelId;
use tokio::sync::{mpsc, mpsc::UnboundedSender};

use super::dispatch::try_spawn_teammate;
use super::process_bridge_teammate_test_support::{
    NamedProvider, collect_events_until_terminal, teammate_task_input,
};
use super::runner::TeammateEvent;

#[tokio::test(flavor = "current_thread")]
async fn teammate_process_bridge_streams_jsonl_events_normal() {
    jfc_plugin_host::clear_discovered_plugin_state_cache_for_tests();
    let tmp = tempfile::TempDir::new().expect("tempdir");
    install_streaming_launcher(tmp.path());
    let (tx, _rx) = mpsc::channel(8);
    let (teammate_tx, mut teammate_rx): (UnboundedSender<TeammateEvent>, _) =
        mpsc::unbounded_channel();
    let mut task = teammate_task_input();
    task.cwd = Some(tmp.path().to_string_lossy().into_owned());
    task.launcher = Some("variant-agent".to_owned());

    let handled = try_spawn_teammate(
        &task,
        "toolu_teammate_stream",
        &tx,
        NamedProvider::arc("anthropic"),
        ModelId::new("claude-opus-4-7"),
        &[],
        Some("session_1"),
        teammate_tx,
        &[],
        || {},
    );

    assert!(handled);
    let events = collect_events_until_terminal(&mut teammate_rx).await;
    assert!(matches!(
        &events[0],
        TeammateEvent::TextDelta { task_id, agent_id, delta }
        if task_id == "teammate-reviewer@alpha" && agent_id == "reviewer@alpha" && delta == "hello"
    ));
    assert!(matches!(
        &events[1],
        TeammateEvent::Progress { token_count, tool_use_count, last_tool, model_id, cost_usd, .. }
        if *token_count == 11 && *tool_use_count == 2
            && last_tool.as_deref() == Some("Read")
            && model_id.as_deref() == Some("local-agent")
            && *cost_usd == Some(0.001)
    ));
    assert!(matches!(
        &events[2],
        TeammateEvent::Idle { agent_name, reason, summary, .. }
        if agent_name == "Variant Agent"
            && reason.as_deref() == Some("waiting")
            && summary.as_deref() == Some("ready")
    ));
    assert!(matches!(
        &events[3],
        TeammateEvent::MessageSent { from, to, text, .. }
        if from == "reviewer@alpha" && to == "team-lead" && text == "done"
    ));
    assert!(matches!(
        &events[4],
        TeammateEvent::Completed { task_id, agent_id }
        if task_id == "teammate-reviewer@alpha" && agent_id == "reviewer@alpha"
    ));
    match tokio::time::timeout(std::time::Duration::from_millis(100), teammate_rx.recv()).await {
        Err(_) | Ok(None) => {}
        Ok(Some(event)) => panic!("unexpected extra teammate event: {event:?}"),
    }
}

fn install_streaming_launcher(project_root: &std::path::Path) {
    let plugin = project_root.join("plugins").join("agent-plugin");
    std::fs::create_dir_all(plugin.join("workflows")).expect("workflows dir");
    let script = r#"read line
id=$(printf "%s\n" "$line" | sed -n "s/.*\"id\":\"\\([^\"]*\\)\".*/\\1/p")
printf "{\"type\":\"response\",\"id\":\"%s\",\"response\":{\"kind\":\"teammate_event\",\"event\":{\"kind\":\"text_delta\",\"delta\":\"hello\"}}}\n" "$id"
printf "{\"type\":\"response\",\"id\":\"%s\",\"response\":{\"kind\":\"teammate_event\",\"event\":{\"kind\":\"progress\",\"token_count\":11,\"tool_use_count\":2,\"last_tool\":\"Read\",\"model_id\":\"local-agent\",\"cost_usd\":0.001}}}\n" "$id"
printf "{\"type\":\"response\",\"id\":\"%s\",\"response\":{\"kind\":\"teammate_event\",\"event\":{\"kind\":\"idle\",\"agent_name\":\"Variant Agent\",\"reason\":\"waiting\",\"summary\":\"ready\"}}}\n" "$id"
printf "{\"type\":\"response\",\"id\":\"%s\",\"response\":{\"kind\":\"teammate_event\",\"event\":{\"kind\":\"message_sent\",\"from\":\"reviewer@alpha\",\"to\":\"team-lead\",\"text\":\"done\"}}}\n" "$id"
printf "{\"type\":\"response\",\"id\":\"%s\",\"response\":{\"kind\":\"teammate_event\",\"event\":{\"kind\":\"completed\"}}}\n" "$id"
"#;
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
