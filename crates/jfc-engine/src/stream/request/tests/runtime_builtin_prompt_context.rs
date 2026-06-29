use std::sync::Arc;

use jfc_provider::{ModelId, Provider, StreamConvention};

use super::super::prepare_stream_request;
use super::runtime_extensions_support::CurrentDirGuard;
use super::{TestProvider, user_text};

struct FeatureGateGuard {
    gate: crate::feature_gates::FeatureGate,
    previous: bool,
}

impl FeatureGateGuard {
    fn set(gate: crate::feature_gates::FeatureGate, enabled: bool) -> Self {
        let previous = crate::feature_gates::is_enabled(gate);
        crate::feature_gates::set(gate, enabled);
        Self { gate, previous }
    }
}

impl Drop for FeatureGateGuard {
    fn drop(&mut self) {
        crate::feature_gates::set(self.gate, self.previous);
    }
}

struct MarshDrainGuard;

impl Drop for MarshDrainGuard {
    fn drop(&mut self) {
        let _ = crate::feature_gates::marsh_drain();
    }
}

struct OutputStyleGuard {
    previous: String,
}

impl OutputStyleGuard {
    fn set(style: crate::output_style::OutputStyle) -> Self {
        let previous = crate::output_style::active().name().to_owned();
        crate::output_style::set_active(style);
        Self { previous }
    }
}

impl Drop for OutputStyleGuard {
    fn drop(&mut self) {
        crate::output_style::set_active_named(&self.previous);
    }
}

#[tokio::test]
#[serial_test::serial]
async fn prepare_appends_feature_gate_prompt_context_runtime_extension_normal() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let _gate = FeatureGateGuard::set(crate::feature_gates::FeatureGate::Harrier, false);
    let _cwd = CurrentDirGuard::enter(tmp.path());
    let provider: Arc<dyn Provider> = Arc::new(TestProvider {
        name: "openai-test",
        convention: StreamConvention::OpenAiNative,
    });

    let request = prepare_stream_request(
        provider,
        &[user_text("show feature prompt context")],
        &ModelId::new("test-model"),
        Default::default(),
    )
    .await;

    let system = request.opts.system.as_deref().unwrap_or_default();
    assert!(system.contains("## Plugin Prompt Context: Feature gates"));
    assert!(system.contains("## Feature gates (deviations from default)"));
    assert!(system.contains("harrier"));
    assert!(system.contains("OFF"));
}

#[tokio::test]
#[serial_test::serial]
async fn prepare_appends_output_style_prompt_context_runtime_extension_normal() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let _style = OutputStyleGuard::set(crate::output_style::OutputStyle::Verbose);
    let _cwd = CurrentDirGuard::enter(tmp.path());
    let provider: Arc<dyn Provider> = Arc::new(TestProvider {
        name: "openai-test",
        convention: StreamConvention::OpenAiNative,
    });

    let request = prepare_stream_request(
        provider,
        &[user_text("show output style prompt context")],
        &ModelId::new("test-model"),
        Default::default(),
    )
    .await;

    let system = request.opts.system.as_deref().unwrap_or_default();
    assert!(system.contains("## Plugin Prompt Context: Output style"));
    assert!(system.contains("Output style: VERBOSE"));
}

#[tokio::test]
#[serial_test::serial]
async fn prepare_appends_harrier_prompt_context_runtime_extension_normal() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let _gate = FeatureGateGuard::set(crate::feature_gates::FeatureGate::Harrier, true);
    let _cwd = CurrentDirGuard::enter(tmp.path());
    let provider: Arc<dyn Provider> = Arc::new(TestProvider {
        name: "openai-test",
        convention: StreamConvention::OpenAiNative,
    });

    let request = prepare_stream_request(
        provider,
        &[user_text("show harrier prompt context")],
        &ModelId::new("test-model"),
        Default::default(),
    )
    .await;

    let system = request.opts.system.as_deref().unwrap_or_default();
    assert!(system.contains("## Plugin Prompt Context: Investigate before asking"));
    assert!(system.contains("Prefer one CodeGraph query or one precise search"));
}

#[tokio::test]
#[serial_test::serial]
async fn prepare_appends_marsh_prompt_context_runtime_extension_normal() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let _gate = FeatureGateGuard::set(crate::feature_gates::FeatureGate::Marsh, true);
    let _drain = MarshDrainGuard;
    let _cwd = CurrentDirGuard::enter(tmp.path());
    crate::feature_gates::marsh_push("first command line");
    crate::feature_gates::marsh_push("second command line");
    let provider: Arc<dyn Provider> = Arc::new(TestProvider {
        name: "openai-test",
        convention: StreamConvention::OpenAiNative,
    });

    let request = prepare_stream_request(
        provider,
        &[user_text("show marsh prompt context")],
        &ModelId::new("test-model"),
        Default::default(),
    )
    .await;

    let system = request.opts.system.as_deref().unwrap_or_default();
    assert!(system.contains("## Plugin Prompt Context: Bash subprocess output"));
    assert!(system.contains("<system-reminder>"));
    assert!(system.contains("Bash subprocess output captured since last turn"));
    assert!(system.contains("first command line"));
    assert!(system.contains("second command line"));
}

#[tokio::test]
#[serial_test::serial]
async fn prepare_appends_background_reminders_prompt_context_runtime_extension_normal() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let _cwd = CurrentDirGuard::enter(tmp.path());
    let provider: Arc<dyn Provider> = Arc::new(TestProvider {
        name: "openai-test",
        convention: StreamConvention::OpenAiNative,
    });
    let overrides = crate::runtime::StreamRequestOverrides {
        background_reminders: vec![
            "file watcher noticed Cargo.toml changed".to_owned(),
            "MCP registry refreshed".to_owned(),
        ],
        ..Default::default()
    };

    let request = prepare_stream_request(
        provider,
        &[user_text("show background reminders")],
        &ModelId::new("test-model"),
        overrides,
    )
    .await;

    let system = request.opts.system.as_deref().unwrap_or_default();
    assert!(system.contains("## Plugin Prompt Context: Background reminders"));
    assert!(system.contains("<system-reminder>"));
    assert!(system.contains("file watcher noticed Cargo.toml changed"));
    assert!(system.contains("MCP registry refreshed"));
}

#[tokio::test]
#[serial_test::serial]
async fn prepare_appends_total_tokens_prompt_context_runtime_extension_normal() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let _cwd = CurrentDirGuard::enter(tmp.path());
    let provider: Arc<dyn Provider> = Arc::new(TestProvider {
        name: "openai-test",
        convention: StreamConvention::OpenAiNative,
    });
    let overrides = crate::runtime::StreamRequestOverrides {
        last_usage_input_tokens: Some(150),
        context_window_tokens: Some(200),
        total_tokens_reminder_mode: Some(
            crate::total_tokens_reminder::TotalTokensReminderMode::Countdown,
        ),
        ..Default::default()
    };

    let request = prepare_stream_request(
        provider,
        &[user_text("show total token reminder")],
        &ModelId::new("test-model"),
        overrides,
    )
    .await;

    let system = request.opts.system.as_deref().unwrap_or_default();
    assert!(system.contains("## Plugin Prompt Context: Total tokens"));
    assert!(system.contains("<total_tokens>50 tokens left</total_tokens>"));
}

#[tokio::test]
#[serial_test::serial]
async fn prepare_appends_previous_handoff_prompt_context_runtime_extension_normal() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    std::fs::create_dir(tmp.path().join(".git")).expect("git marker");
    let summaries = tmp.path().join(".jfc").join("session_summaries");
    std::fs::create_dir_all(&summaries).expect("summary dir");
    std::fs::write(summaries.join("2026-06-27T10-00-00.md"), "older handoff").expect("old handoff");
    std::fs::write(
        summaries.join("2026-06-27T11-00-00.md"),
        "new handoff summary",
    )
    .expect("new handoff");
    let _cwd = CurrentDirGuard::enter(tmp.path());
    let provider: Arc<dyn Provider> = Arc::new(TestProvider {
        name: "openai-test",
        convention: StreamConvention::OpenAiNative,
    });

    let request = prepare_stream_request(
        provider,
        &[user_text("show handoff prompt context")],
        &ModelId::new("test-model"),
        Default::default(),
    )
    .await;

    let system = request.opts.system.as_deref().unwrap_or_default();
    assert!(system.contains("## Plugin Prompt Context: Previous Session Handoff"));
    assert!(system.contains("new handoff summary"));
    assert!(!system.contains("older handoff"));
}
