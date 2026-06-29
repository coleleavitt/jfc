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

#[tokio::test]
#[serial_test::serial]
async fn prepare_appends_brief_mode_prompt_context_runtime_extension_normal() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let _cwd = CurrentDirGuard::enter(tmp.path());
    let provider: Arc<dyn Provider> = Arc::new(TestProvider {
        name: "openai-test",
        convention: StreamConvention::OpenAiNative,
    });
    let overrides = crate::runtime::StreamRequestOverrides {
        brief_mode: true,
        ..Default::default()
    };

    let request = prepare_stream_request(
        provider,
        &[user_text("show brief mode")],
        &ModelId::new("test-model"),
        overrides,
    )
    .await;

    let system = request.opts.system.as_deref().unwrap_or_default();
    assert!(system.contains("## Plugin Prompt Context: Brief user messages"));
    assert!(system.contains("Plain assistant text is hidden from the main chat view"));
}

#[tokio::test]
#[serial_test::serial]
async fn prepare_appends_pewter_owl_prompt_context_runtime_extension_normal() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let _gate = FeatureGateGuard::set(crate::feature_gates::FeatureGate::PewterOwlTool, true);
    let _cwd = CurrentDirGuard::enter(tmp.path());
    let provider: Arc<dyn Provider> = Arc::new(TestProvider {
        name: "openai-test",
        convention: StreamConvention::OpenAiNative,
    });

    let request = prepare_stream_request(
        provider,
        &[user_text("show pewter owl")],
        &ModelId::new("test-model"),
        Default::default(),
    )
    .await;

    let system = request.opts.system.as_deref().unwrap_or_default();
    assert!(system.contains("## Plugin Prompt Context: Pewter Owl messaging"));
    assert!(system.contains("`SendUserMessage` is available for exact user-visible content"));
}

#[tokio::test]
#[serial_test::serial]
async fn prepare_appends_interaction_mode_prompt_context_runtime_extension_normal() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let _cwd = CurrentDirGuard::enter(tmp.path());
    let provider: Arc<dyn Provider> = Arc::new(TestProvider {
        name: "openai-test",
        convention: StreamConvention::OpenAiNative,
    });
    let overrides = crate::runtime::StreamRequestOverrides {
        interaction_mode: crate::interaction_mode::InteractionMode::Fast,
        ..Default::default()
    };

    let request = prepare_stream_request(
        provider,
        &[user_text("show interaction mode")],
        &ModelId::new("test-model"),
        overrides,
    )
    .await;

    let system = request.opts.system.as_deref().unwrap_or_default();
    assert!(system.contains("## Plugin Prompt Context: Interaction mode"));
    assert!(system.contains("## Interaction mode: Fast"));
}
