use std::sync::Arc;

use jfc_provider::{ModelId, Provider, StreamConvention};

use super::super::prepare_stream_request;
use super::runtime_extensions_support::{CurrentDirGuard, EnvVarGuard};
use super::{TestProvider, user_text};

#[tokio::test]
#[serial_test::serial]
async fn prepare_appends_project_prompt_context_runtime_extension_normal() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let plugin = tmp.path().join(".jfc/plugins/context-pack");
    std::fs::create_dir_all(&plugin).expect("create plugin");
    std::fs::write(
        plugin.join(".jfc-plugin.toml"),
        r#"[plugin]
name = "context-pack"

[[runtime_extensions]]
target = "prompt_context"
id = "context.review-rules"
label = "Review Rules"
priority = 30

[runtime_extensions.executor]
kind = "static_text"
handler = "Always include the project plugin review rules."
"#,
    )
    .expect("write manifest");
    let _data_home = EnvVarGuard::set_path("XDG_DATA_HOME", &tmp.path().join("data"));
    let _cwd = CurrentDirGuard::enter(tmp.path());
    let provider: Arc<dyn Provider> = Arc::new(TestProvider {
        name: "openai-test",
        convention: StreamConvention::OpenAiNative,
    });

    let request = prepare_stream_request(
        provider,
        &[user_text("review this repo")],
        &ModelId::new("test-model"),
        Default::default(),
    )
    .await;

    let system = request.opts.system.as_deref().unwrap_or_default();
    assert!(
        system.contains("## Plugin Prompt Context: Review Rules"),
        "{system}"
    );
    assert!(
        system.contains("Always include the project plugin review rules."),
        "{system}"
    );
}

#[tokio::test]
#[serial_test::serial]
async fn prepare_appends_process_bridge_prompt_context_runtime_extension_normal() {
    use std::os::unix::fs::PermissionsExt;

    let tmp = tempfile::TempDir::new().expect("tempdir");
    let plugin = tmp.path().join(".jfc/plugins/context-bridge");
    std::fs::create_dir_all(&plugin).expect("create plugin");
    std::fs::write(
        plugin.join("context.sh"),
        "#!/usr/bin/env sh\nread line\nid=$(printf '%s' \"$line\" | sed -n 's/.*\"id\":\"\\([^\"]*\\)\".*/\\1/p')\nprintf '{\"type\":\"response\",\"id\":\"%s\",\"response\":{\"kind\":\"prompt_context_refresh\",\"result\":{\"body\":\"Bridge supplied prompt context.\"}}}\\n' \"$id\"\n",
    )
    .expect("write bridge");
    let mut perms = std::fs::metadata(plugin.join("context.sh"))
        .expect("metadata")
        .permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(plugin.join("context.sh"), perms).expect("chmod bridge");
    std::fs::write(
        plugin.join(".jfc-plugin.toml"),
        r#"[plugin]
name = "context-bridge"

[[runtime_extensions]]
target = "prompt_context"
id = "context.bridge"
label = "Bridge Context"
priority = 40

[runtime_extensions.executor]
kind = "process_bridge"
handler = "context.sh"
"#,
    )
    .expect("write manifest");
    let _data_home = EnvVarGuard::set_path("XDG_DATA_HOME", &tmp.path().join("data"));
    let _cwd = CurrentDirGuard::enter(tmp.path());
    let provider: Arc<dyn Provider> = Arc::new(TestProvider {
        name: "openai-test",
        convention: StreamConvention::OpenAiNative,
    });

    let request = prepare_stream_request(
        provider,
        &[user_text("use plugin context")],
        &ModelId::new("test-model"),
        Default::default(),
    )
    .await;

    let system = request.opts.system.as_deref().unwrap_or_default();
    assert!(system.contains("## Plugin Prompt Context: Bridge Context"));
    assert!(system.contains("Bridge supplied prompt context."));
}

#[tokio::test]
#[serial_test::serial]
async fn prepare_reuses_prompt_context_snapshot_inside_refresh_interval_normal() {
    use std::os::unix::fs::PermissionsExt;

    let tmp = tempfile::TempDir::new().expect("tempdir");
    let plugin = tmp.path().join(".jfc/plugins/context-bridge");
    let hits = tmp.path().join("hits.txt");
    std::fs::create_dir_all(&plugin).expect("create plugin");
    std::fs::write(
        plugin.join("context.sh"),
        format!(
            "#!/usr/bin/env sh\nread line\nid=$(printf '%s' \"$line\" | sed -n 's/.*\"id\":\"\\([^\"]*\\)\".*/\\1/p')\nhits={hits:?}\ncount=0\nif [ -f \"$hits\" ]; then count=$(cat \"$hits\"); fi\ncount=$((count + 1))\nprintf '%s' \"$count\" > \"$hits\"\nprintf '{{\"type\":\"response\",\"id\":\"%s\",\"response\":{{\"kind\":\"prompt_context_refresh\",\"result\":{{\"body\":\"Bridge cached prompt #%s\",\"state\":{{\"count\":%s}}}}}}}}\\n' \"$id\" \"$count\" \"$count\"\n",
            hits = hits.to_string_lossy(),
        ),
    )
    .expect("write bridge");
    let mut perms = std::fs::metadata(plugin.join("context.sh"))
        .expect("metadata")
        .permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(plugin.join("context.sh"), perms).expect("chmod bridge");
    std::fs::write(
        plugin.join(".jfc-plugin.toml"),
        r#"[plugin]
name = "context-bridge"

[[runtime_extensions]]
target = "prompt_context"
id = "context.bridge"
label = "Bridge Context"
priority = 40
refresh = { kind = "process_bridge", min_interval_ms = 600000 }

[runtime_extensions.executor]
kind = "process_bridge"
handler = "context.sh"
"#,
    )
    .expect("write manifest");
    let _data_home = EnvVarGuard::set_path("XDG_DATA_HOME", &tmp.path().join("data"));
    let _cwd = CurrentDirGuard::enter(tmp.path());
    let provider: Arc<dyn Provider> = Arc::new(TestProvider {
        name: "openai-test",
        convention: StreamConvention::OpenAiNative,
    });

    let first = prepare_stream_request(
        Arc::clone(&provider),
        &[user_text("use plugin context")],
        &ModelId::new("test-model"),
        Default::default(),
    )
    .await;
    let second = prepare_stream_request(
        provider,
        &[user_text("use plugin context again")],
        &ModelId::new("test-model"),
        Default::default(),
    )
    .await;

    let first_system = first.opts.system.as_deref().unwrap_or_default();
    let second_system = second.opts.system.as_deref().unwrap_or_default();
    assert!(first_system.contains("Bridge cached prompt #1"));
    assert!(second_system.contains("Bridge cached prompt #1"));
    assert_eq!(
        std::fs::read_to_string(&hits).expect("hits file"),
        "1",
        "second request should use cached prompt-context body inside min_interval_ms"
    );
}

#[tokio::test]
#[serial_test::serial]
async fn prepare_sends_previous_prompt_context_state_on_refresh_normal() {
    use std::os::unix::fs::PermissionsExt;

    let tmp = tempfile::TempDir::new().expect("tempdir");
    let plugin = tmp.path().join(".jfc/plugins/context-bridge");
    std::fs::create_dir_all(&plugin).expect("create plugin");
    std::fs::write(
        plugin.join("context.sh"),
        "#!/usr/bin/env sh\nread line\nid=$(printf '%s' \"$line\" | sed -n 's/.*\"id\":\"\\([^\"]*\\)\".*/\\1/p')\nprevious=$(printf '%s' \"$line\" | sed -n 's/.*\"state\":{\"count\":\\([0-9][0-9]*\\)}.*/\\1/p')\nif [ -z \"$previous\" ]; then previous=0; fi\nnext=$((previous + 1))\nprintf '{\"type\":\"response\",\"id\":\"%s\",\"response\":{\"kind\":\"prompt_context_refresh\",\"result\":{\"body\":\"Bridge state previous=%s next=%s\",\"state\":{\"count\":%s}}}}\\n' \"$id\" \"$previous\" \"$next\" \"$next\"\n",
    )
    .expect("write bridge");
    let mut perms = std::fs::metadata(plugin.join("context.sh"))
        .expect("metadata")
        .permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(plugin.join("context.sh"), perms).expect("chmod bridge");
    std::fs::write(
        plugin.join(".jfc-plugin.toml"),
        r#"[plugin]
name = "context-bridge"

[[runtime_extensions]]
target = "prompt_context"
id = "context.bridge"
label = "Bridge Context"
priority = 40
refresh = { kind = "process_bridge", min_interval_ms = 0 }

[runtime_extensions.executor]
kind = "process_bridge"
handler = "context.sh"
"#,
    )
    .expect("write manifest");
    let _data_home = EnvVarGuard::set_path("XDG_DATA_HOME", &tmp.path().join("data"));
    let _cwd = CurrentDirGuard::enter(tmp.path());
    let provider: Arc<dyn Provider> = Arc::new(TestProvider {
        name: "openai-test",
        convention: StreamConvention::OpenAiNative,
    });

    let first = prepare_stream_request(
        Arc::clone(&provider),
        &[user_text("use plugin context")],
        &ModelId::new("test-model"),
        Default::default(),
    )
    .await;
    let second = prepare_stream_request(
        provider,
        &[user_text("use plugin context again")],
        &ModelId::new("test-model"),
        Default::default(),
    )
    .await;

    let first_system = first.opts.system.as_deref().unwrap_or_default();
    let second_system = second.opts.system.as_deref().unwrap_or_default();
    assert!(first_system.contains("Bridge state previous=0 next=1"));
    assert!(second_system.contains("Bridge state previous=1 next=2"));
}

#[tokio::test]
#[serial_test::serial]
async fn prepare_appends_builtin_project_documents_prompt_context_normal() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    std::fs::write(tmp.path().join("PARITY.md"), "# Parity\n").expect("write parity");
    let _cwd = CurrentDirGuard::enter(tmp.path());
    let provider: Arc<dyn Provider> = Arc::new(TestProvider {
        name: "openai-test",
        convention: StreamConvention::OpenAiNative,
    });

    let request = prepare_stream_request(
        provider,
        &[user_text("update the architecture plan")],
        &ModelId::new("test-model"),
        Default::default(),
    )
    .await;

    let system = request.opts.system.as_deref().unwrap_or_default();
    assert!(system.contains("## Plugin Prompt Context: Project documents"));
    assert!(system.contains("## Project documents"));
    assert!(system.contains("`PARITY.md`"));
    assert!(!system.contains("`PLAN.md`"));
}
