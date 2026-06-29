use std::sync::Arc;

use jfc_provider::{ModelId, Provider, StreamConvention};

use super::super::prepare_stream_request;
use super::runtime_extensions_support::CurrentDirGuard;
use super::{TestProvider, user_text};

struct LocalAdvisorGuard {
    previous: Option<ModelId>,
}

impl LocalAdvisorGuard {
    fn set(model: Option<ModelId>) -> Self {
        let previous = crate::advisor::active_local_advisor_model();
        crate::advisor::set_active_local_advisor_model(model);
        Self { previous }
    }
}

impl Drop for LocalAdvisorGuard {
    fn drop(&mut self) {
        crate::advisor::set_active_local_advisor_model(self.previous.clone());
    }
}

struct ServerAdvisorGuard {
    previous: Option<ModelId>,
}

impl ServerAdvisorGuard {
    fn set(model: Option<ModelId>) -> Self {
        let previous = crate::advisor::active_server_advisor_model();
        crate::advisor::set_active_server_advisor_model(model);
        Self { previous }
    }
}

impl Drop for ServerAdvisorGuard {
    fn drop(&mut self) {
        crate::advisor::set_active_server_advisor_model(self.previous.clone());
    }
}

#[tokio::test]
#[serial_test::serial]
async fn prepare_appends_local_advisor_prompt_context_runtime_extension_normal() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let _advisor = LocalAdvisorGuard::set(Some(ModelId::new("local-advisor-model")));
    let _cwd = CurrentDirGuard::enter(tmp.path());
    let provider: Arc<dyn Provider> = Arc::new(TestProvider {
        name: "openai-test",
        convention: StreamConvention::OpenAiNative,
    });

    let request = prepare_stream_request(
        provider,
        &[user_text("show local advisor prompt context")],
        &ModelId::new("test-model"),
        Default::default(),
    )
    .await;

    let system = request.opts.system.as_deref().unwrap_or_default();
    assert!(system.contains("## Plugin Prompt Context: Local Advisor Tool"));
    assert!(system.contains("You have access to an `Advisor` tool"));
    assert!(system.contains("before declaring substantial work done"));
}

#[tokio::test]
#[serial_test::serial]
async fn prepare_appends_server_advisor_prompt_context_runtime_extension_normal() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let _advisor = ServerAdvisorGuard::set(Some(ModelId::new("claude-opus-4-8")));
    let _cwd = CurrentDirGuard::enter(tmp.path());
    let provider: Arc<dyn Provider> = Arc::new(TestProvider {
        name: "anthropic",
        convention: StreamConvention::AnthropicNative,
    });

    let request = prepare_stream_request(
        provider,
        &[user_text("show server advisor prompt context")],
        &ModelId::new("test-model"),
        Default::default(),
    )
    .await;

    let system = request.opts.system.as_deref().unwrap_or_default();
    assert!(system.contains("## Plugin Prompt Context: Server Advisor Tool"));
    assert!(system.contains("# Advisor Tool"));
    assert!(system.contains("advisor()"));
    assert_eq!(
        request
            .opts
            .advisor_model
            .as_ref()
            .map(|model| model.as_str()),
        Some("claude-opus-4-8")
    );
}
