use jfc_provider::ModelId;
use tokio::sync::{mpsc, mpsc::UnboundedSender};

use super::dispatch::try_spawn_teammate;
use super::process_bridge_teammate_test_support::{
    NamedProvider, collect_events_until_terminal, teammate_task_input,
};
use super::runner::TeammateEvent;
use super::test_support::HomeOverride;
use super::types::MailboxMessage;
use super::{TEAM_LEAD_NAME, mailbox};

#[tokio::test(flavor = "current_thread")]
async fn teammate_process_bridge_handles_mailbox_and_ready_helpers_normal() {
    jfc_plugin_host::clear_discovered_plugin_state_cache_for_tests();
    let _home = HomeOverride::new();
    let tmp = tempfile::TempDir::new().expect("tempdir");
    install_helper_launcher(tmp.path());
    mailbox::write_to_mailbox(
        "reviewer",
        MailboxMessage {
            from: TEAM_LEAD_NAME.to_owned(),
            text: "please inspect".to_owned(),
            timestamp: "2026-06-27T00:00:00Z".to_owned(),
            color: None,
            summary: Some("inspect".to_owned()),
            read: false,
        },
        "alpha",
    )
    .await
    .expect("seed mailbox");
    let (tx, _rx) = mpsc::channel(8);
    let (teammate_tx, mut teammate_rx): (UnboundedSender<TeammateEvent>, _) =
        mpsc::unbounded_channel();
    let mut task = teammate_task_input();
    task.cwd = Some(tmp.path().to_string_lossy().into_owned());
    task.launcher = Some("variant-agent".to_owned());

    let handled = try_spawn_teammate(
        &task,
        "toolu_teammate_helpers",
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
        TeammateEvent::TextDelta { delta, .. } if delta == "mailbox-ok"
    ));
    assert!(matches!(
        &events[1],
        TeammateEvent::Idle { agent_name, reason, summary, .. }
            if agent_name == "reviewer"
                && reason.as_deref() == Some("waiting")
                && summary.as_deref() == Some("ready")
    ));
    assert!(matches!(&events[2], TeammateEvent::Completed { .. }));
    let reviewer_messages = mailbox::read_mailbox("reviewer", "alpha").await;
    assert!(reviewer_messages.iter().all(|message| message.read));
    let leader_messages = mailbox::read_mailbox(TEAM_LEAD_NAME, "alpha").await;
    assert!(
        leader_messages
            .iter()
            .any(|message| message.from == "reviewer" && message.text == "plugin says done")
    );
    assert!(
        leader_messages
            .iter()
            .any(|message| message.from == "reviewer"
                && mailbox::is_idle_notification(&message.text))
    );
}

fn install_helper_launcher(project_root: &std::path::Path) {
    let plugin = project_root.join("plugins").join("agent-plugin");
    std::fs::create_dir_all(plugin.join("workflows")).expect("workflows dir");
    let script = r#"read line
id=$(printf "%s\n" "$line" | sed -n "s/.*\"id\":\"\([^\"]*\)\".*/\1/p")
printf '{"type":"request","id":"mailbox-1","request":{"kind":"teammate_mailbox_poll","request":{"agent_name":"reviewer","team_name":"alpha","unread_only":true,"mark_read":true}}}\n'
read mailbox_response
case "$mailbox_response" in
  *please\ inspect*) printf "{\"type\":\"response\",\"id\":\"%s\",\"response\":{\"kind\":\"teammate_event\",\"event\":{\"kind\":\"text_delta\",\"delta\":\"mailbox-ok\"}}}\n" "$id" ;;
  *) printf "{\"type\":\"response\",\"id\":\"%s\",\"response\":{\"kind\":\"teammate_event\",\"event\":{\"kind\":\"failed\",\"error\":\"mailbox response missing\"}}}\n" "$id"; exit 0 ;;
esac
printf '{"type":"request","id":"mailbox-2","request":{"kind":"teammate_mailbox_send","request":{"to":"team-lead","from":"reviewer","team_name":"alpha","text":"plugin says done","summary":"done"}}}\n'
read send_response
printf '{"type":"request","id":"ready-1","request":{"kind":"teammate_ready","ready":{"agent_name":"reviewer","team_name":"alpha","reason":"waiting","summary":"ready"}}}\n'
read ready_response
printf "{\"type\":\"response\",\"id\":\"%s\",\"response\":{\"kind\":\"teammate_event\",\"event\":{\"kind\":\"completed\"}}}\n" "$id"
"#;
    let script_path = plugin.join("helper.sh");
    std::fs::write(&script_path, script).expect("write helper script");
    let handler = serde_json::json!({
        "command": "sh",
        "args": [script_path.to_string_lossy().into_owned()],
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
