use std::path::Path;
use std::process::Command;

use tempfile::TempDir;

fn jfc() -> Command {
    Command::new(env!("CARGO_BIN_EXE_jfc"))
}

fn run_jfc_plugin(config: &Path, args: &[&str]) -> std::process::Output {
    jfc()
        .args(args)
        .env("XDG_CONFIG_HOME", config)
        .output()
        .expect("run jfc plugin command")
}

#[test]
fn plugin_install_template_seeds_teammate_helper_normal() {
    let config = TempDir::new().expect("temp config");

    let install = run_jfc_plugin(
        config.path(),
        &[
            "plugin",
            "install",
            "--template",
            "teammate-helper",
            "--name",
            "demo-helper",
        ],
    );

    assert!(install.status.success(), "exit: {:?}", install.status);
    let stdout = String::from_utf8_lossy(&install.stdout);
    assert!(
        stdout.contains("installed plugin template teammate-helper"),
        "stdout: {stdout}"
    );
    let plugin = config.path().join("jfc/plugins/demo-helper");
    assert!(plugin.join(".jfc-plugin.toml").is_file());
    assert!(plugin.join("Cargo.toml").is_file());
    assert!(plugin.join("examples/teammate_helper_agent.rs").is_file());

    let doctor = run_jfc_plugin(config.path(), &["plugin", "doctor"]);

    assert!(doctor.status.success(), "exit: {:?}", doctor.status);
    let stdout = String::from_utf8_lossy(&doctor.stdout);
    assert!(stdout.contains("agent_launches=1"), "stdout: {stdout}");
    assert!(
        stdout.contains("plugin_template_catalog jfc plugin templates"),
        "stdout: {stdout}"
    );
    assert!(stdout.contains("- demo-helper"), "stdout: {stdout}");
}

#[test]
fn plugin_install_template_seeds_prompt_context_normal() {
    let config = TempDir::new().expect("temp config");

    let install = run_jfc_plugin(
        config.path(),
        &[
            "plugin",
            "install",
            "--template",
            "prompt-context",
            "--name",
            "demo-prompt",
        ],
    );

    assert!(install.status.success(), "exit: {:?}", install.status);
    let stdout = String::from_utf8_lossy(&install.stdout);
    assert!(
        stdout.contains("installed plugin template prompt-context"),
        "stdout: {stdout}"
    );
    let plugin = config.path().join("jfc/plugins/demo-prompt");
    assert!(plugin.join(".jfc-plugin.toml").is_file());
    assert!(plugin.join("Cargo.toml").is_file());
    assert!(plugin.join("examples/prompt_context_provider.rs").is_file());

    let doctor = run_jfc_plugin(config.path(), &["plugin", "doctor"]);

    assert!(doctor.status.success(), "exit: {:?}", doctor.status);
    let stdout = String::from_utf8_lossy(&doctor.stdout);
    assert!(stdout.contains("runtime_extensions=1"), "stdout: {stdout}");
    assert!(
        stdout.contains("demo-prompt context.cached-note Cached Note"),
        "stdout: {stdout}"
    );
    assert!(
        stdout.contains("refresh=process_bridge; min_interval_ms=1000; auto_refresh_ms=60000"),
        "stdout: {stdout}"
    );
}

#[test]
fn plugin_install_template_seeds_process_tool_normal() {
    let config = TempDir::new().expect("temp config");

    let install = run_jfc_plugin(
        config.path(),
        &[
            "plugin",
            "install",
            "--template",
            "process-tool",
            "--name",
            "demo-tool",
        ],
    );

    assert!(install.status.success(), "exit: {:?}", install.status);
    let stdout = String::from_utf8_lossy(&install.stdout);
    assert!(
        stdout.contains("installed plugin template process-tool"),
        "stdout: {stdout}"
    );
    let plugin = config.path().join("jfc/plugins/demo-tool");
    assert!(plugin.join(".jfc-plugin.toml").is_file());
    assert!(plugin.join("Cargo.toml").is_file());
    assert!(plugin.join("examples/process_bridge_tool.rs").is_file());

    let doctor = run_jfc_plugin(config.path(), &["plugin", "doctor"]);

    assert!(doctor.status.success(), "exit: {:?}", doctor.status);
    let stdout = String::from_utf8_lossy(&doctor.stdout);
    assert!(stdout.contains("tools=1"), "stdout: {stdout}");
    assert!(stdout.contains("tools:"), "stdout: {stdout}");
    assert!(
        stdout.contains(
            "- demo-tool external_echo External Echo [process_bridge; read_only; model_visible]"
        ),
        "stdout: {stdout}"
    );
}

#[test]
fn plugin_smoke_runs_process_tool_template_normal() {
    let config = TempDir::new().expect("temp config");

    let install = run_jfc_plugin(
        config.path(),
        &[
            "plugin",
            "install",
            "--template",
            "process-tool",
            "--name",
            "demo-tool",
        ],
    );
    assert!(install.status.success(), "exit: {:?}", install.status);

    let smoke = run_jfc_plugin(config.path(), &["plugin", "smoke", "demo-tool"]);

    assert!(smoke.status.success(), "exit: {:?}", smoke.status);
    let stdout = String::from_utf8_lossy(&smoke.stdout);
    assert!(
        stdout.contains("plugin smoke: demo-tool"),
        "stdout: {stdout}"
    );
    assert!(
        stdout.contains("descriptors: tools=1 providers=0"),
        "stdout: {stdout}"
    );
    assert!(
        stdout.contains("describe external_echo: ok"),
        "stdout: {stdout}"
    );
    assert!(
        stdout.contains("tool external_echo: external echo: smoke test"),
        "stdout: {stdout}"
    );
}

#[test]
fn plugin_install_template_seeds_process_provider_normal() {
    let config = TempDir::new().expect("temp config");

    let install = run_jfc_plugin(
        config.path(),
        &[
            "plugin",
            "install",
            "--template",
            "process-provider",
            "--name",
            "demo-provider",
        ],
    );

    assert!(install.status.success(), "exit: {:?}", install.status);
    let stdout = String::from_utf8_lossy(&install.stdout);
    assert!(
        stdout.contains("installed plugin template process-provider"),
        "stdout: {stdout}"
    );
    let plugin = config.path().join("jfc/plugins/demo-provider");
    assert!(plugin.join(".jfc-plugin.toml").is_file());
    assert!(plugin.join("Cargo.toml").is_file());
    assert!(plugin.join("examples/process_bridge_provider.rs").is_file());

    let doctor = run_jfc_plugin(config.path(), &["plugin", "doctor"]);

    assert!(doctor.status.success(), "exit: {:?}", doctor.status);
    let stdout = String::from_utf8_lossy(&doctor.stdout);
    assert!(stdout.contains("providers=1"), "stdout: {stdout}");
    assert!(stdout.contains("providers:"), "stdout: {stdout}");
    assert!(
        stdout.contains("- demo-provider external-demo [process_bridge; host_visible; models=external-demo-chat]"),
        "stdout: {stdout}"
    );
}

#[test]
fn plugin_smoke_runs_process_provider_template_normal() {
    let config = TempDir::new().expect("temp config");

    let install = run_jfc_plugin(
        config.path(),
        &[
            "plugin",
            "install",
            "--template",
            "process-provider",
            "--name",
            "demo-provider",
        ],
    );
    assert!(install.status.success(), "exit: {:?}", install.status);

    let smoke = run_jfc_plugin(config.path(), &["plugin", "smoke", "demo-provider"]);

    assert!(smoke.status.success(), "exit: {:?}", smoke.status);
    let stdout = String::from_utf8_lossy(&smoke.stdout);
    assert!(
        stdout.contains("plugin smoke: demo-provider"),
        "stdout: {stdout}"
    );
    assert!(
        stdout.contains("descriptors: tools=0 providers=1"),
        "stdout: {stdout}"
    );
    assert!(
        stdout.contains("describe external-demo: ok"),
        "stdout: {stdout}"
    );
    assert!(
        stdout.contains(
            "provider external-demo/external-demo-chat: external-demo-chat received: smoke test"
        ),
        "stdout: {stdout}"
    );
}

#[test]
fn plugin_templates_lists_first_party_templates_normal() {
    let config = TempDir::new().expect("temp config");

    let templates = run_jfc_plugin(config.path(), &["plugin", "templates"]);

    assert!(templates.status.success(), "exit: {:?}", templates.status);
    let stdout = String::from_utf8_lossy(&templates.stdout);
    assert!(stdout.contains("plugin templates:"), "stdout: {stdout}");
    assert!(stdout.contains("teammate-helper"), "stdout: {stdout}");
    assert!(
        stdout.contains("jfc plugin install --template teammate-helper"),
        "stdout: {stdout}"
    );
    assert!(stdout.contains("prompt-context"), "stdout: {stdout}");
    assert!(
        stdout.contains("jfc plugin install --template prompt-context"),
        "stdout: {stdout}"
    );
    assert!(stdout.contains("process-tool"), "stdout: {stdout}");
    assert!(
        stdout.contains("jfc plugin install --template process-tool"),
        "stdout: {stdout}"
    );
    assert!(stdout.contains("process-provider"), "stdout: {stdout}");
    assert!(
        stdout.contains("jfc plugin install --template process-provider"),
        "stdout: {stdout}"
    );
}
