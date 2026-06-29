use jfc_provider::{
    EventStream, Provider, ProviderContent, ProviderMessage, ProviderRole, StreamConvention,
    StreamOptions,
};

mod budget;
mod intent;
mod memory;
mod prepare;
mod rsi_runtime;
mod runtime_builtin_prompt_context;
mod runtime_builtin_prompt_context_advisor;
mod runtime_builtin_prompt_context_behavior;
mod runtime_extensions;
mod runtime_extensions_support;
mod thinking;

pub(super) struct TestProvider {
    pub(super) name: &'static str,
    pub(super) convention: StreamConvention,
}

#[async_trait::async_trait]
impl Provider for TestProvider {
    fn name(&self) -> &str {
        self.name
    }

    fn available_models(&self) -> Vec<jfc_provider::ModelInfo> {
        Vec::new()
    }

    fn stream_convention(&self) -> StreamConvention {
        self.convention
    }

    async fn stream(
        &self,
        _messages: Vec<ProviderMessage>,
        _options: &StreamOptions,
    ) -> anyhow::Result<EventStream> {
        Ok(Box::pin(futures::stream::empty()))
    }
}

impl jfc_provider::seal::Sealed for TestProvider {}

pub(super) fn user_text(s: &str) -> ProviderMessage {
    ProviderMessage {
        role: ProviderRole::User,
        content: vec![ProviderContent::Text(s.into())],
    }
}

pub(super) fn user_tool_result(id: &str) -> ProviderMessage {
    ProviderMessage {
        role: ProviderRole::User,
        content: vec![ProviderContent::ToolResult {
            tool_use_id: id.into(),
            content: "ok".into(),
            is_error: false,
        }],
    }
}
