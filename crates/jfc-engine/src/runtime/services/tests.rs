use std::sync::Arc;

use async_trait::async_trait;

use super::*;
use crate::ids::SessionId;
use crate::session::{
    AutosaveOutcome, ListSessionsRequest, SearchSessionsRequest, SessionTranscript,
    StoredSessionMessage,
};
use jfc_provider::{EventStream, ModelInfo, Provider, ProviderMessage, StreamOptions};

#[derive(Default)]
struct FakeSessionStore;

#[async_trait]
impl SessionStore for FakeSessionStore {
    async fn save_transcript(&self, _request: jfc_session::SaveTranscriptRequest<'_>) {}

    async fn load_transcript(&self, _session_id: &SessionId) -> Option<SessionTranscript> {
        Some(SessionTranscript {
            messages: Vec::<StoredSessionMessage>::new(),
            model: Some("fake-model".to_owned()),
        })
    }

    async fn set_title(&self, _session_id: &SessionId, _title: &str) {}

    async fn list_sessions(
        &self,
        _request: ListSessionsRequest<'_>,
    ) -> Vec<jfc_session::SessionMetadata> {
        Vec::new()
    }

    fn search_sessions(&self, _request: SearchSessionsRequest<'_>) -> Vec<jfc_session::SessionHit> {
        Vec::new()
    }

    async fn request_autosave(
        &self,
        _request: jfc_session::AutosaveRequest<'_>,
    ) -> AutosaveOutcome {
        AutosaveOutcome::Saved
    }
}

struct FakeProviderRegistry;

impl RuntimeService for FakeProviderRegistry {
    fn service_name(&self) -> &'static str {
        "fake-provider-registry"
    }
}

impl ProviderRegistry for FakeProviderRegistry {
    fn resolve_provider_model(
        &self,
        model_id: &str,
    ) -> Result<ProviderModelResolution, ProviderRegistryError> {
        if model_id == "fake-model" {
            return Ok(ProviderModelResolution {
                provider: Arc::new(NamedProvider("fake-registry-provider")),
                model: jfc_provider::ModelId::new(model_id),
                resolved_model: jfc_provider::ResolvedModel::new(
                    jfc_provider::ModelSpec::bare(model_id),
                    jfc_provider::ModelSpec::qualified(
                        jfc_provider::ProviderId::new("fake-registry-provider"),
                        jfc_provider::ModelId::new(model_id),
                    ),
                    jfc_provider::ModelResolutionReason::ExplicitProvider,
                    None,
                ),
            });
        }

        Err(ProviderRegistryError::MissingProvider {
            model: model_id.to_owned(),
        })
    }
}

struct NamedProvider(&'static str);

#[async_trait]
impl Provider for NamedProvider {
    fn name(&self) -> &str {
        self.0
    }

    fn available_models(&self) -> Vec<ModelInfo> {
        Vec::new()
    }

    async fn stream(
        &self,
        _messages: Vec<ProviderMessage>,
        _options: &StreamOptions,
    ) -> anyhow::Result<EventStream> {
        anyhow::bail!("unused")
    }
}

impl jfc_provider::seal::Sealed for NamedProvider {}

struct FakeToolRuntime;

impl RuntimeService for FakeToolRuntime {
    fn service_name(&self) -> &'static str {
        "fake-tool-runtime"
    }
}

impl ToolRuntime for FakeToolRuntime {}

struct FakePolicy;

impl RuntimeService for FakePolicy {
    fn service_name(&self) -> &'static str {
        "fake-policy"
    }
}

impl RuntimePolicy for FakePolicy {
    fn tool_decision(
        &self,
        mode: crate::app::PermissionMode,
        kind: &crate::types::ToolKind,
        input: &crate::types::ToolInput,
    ) -> crate::app::PermissionDecision {
        mode.decide_parts(kind, input)
    }
}

struct FakePluginRuntime;

impl RuntimeService for FakePluginRuntime {
    fn service_name(&self) -> &'static str {
        "fake-plugin-runtime"
    }
}

impl PluginRuntime for FakePluginRuntime {}

struct FakeContextAssembler;

impl RuntimeService for FakeContextAssembler {
    fn service_name(&self) -> &'static str {
        "fake-context-assembler"
    }
}

impl ContextAssembler for FakeContextAssembler {}

struct FakeDiagnostics;

impl RuntimeService for FakeDiagnostics {
    fn service_name(&self) -> &'static str {
        "fake-diagnostics"
    }
}

impl RuntimeDiagnostics for FakeDiagnostics {
    fn snapshot(&self) -> RuntimeDiagnosticsSnapshot {
        RuntimeDiagnosticsSnapshot {
            service_name: self.service_name(),
            status: RuntimeDiagnosticsStatus::Healthy,
            detail: Some("all fake services online".to_owned()),
        }
    }

    fn diagnostic_entries(&self) -> Vec<crate::diagnostics::DiagnosticEntry> {
        vec![crate::diagnostics::DiagnosticEntry {
            file: "runtime_services.rs".to_owned(),
            line: 7,
            col: 11,
            message: "fake diagnostics reached runtime services".to_owned(),
            code: Some("FAKE001".to_owned()),
            source: Some("fake".to_owned()),
            severity: crate::diagnostics::Severity::Warning,
        }]
    }
}

struct FakeHandles {
    session_store: Arc<dyn SessionStore>,
    provider_registry: Arc<dyn ProviderRegistry>,
    tool_runtime: Arc<dyn ToolRuntime>,
    policy: Arc<dyn RuntimePolicy>,
    plugin_runtime: Arc<dyn PluginRuntime>,
    context_assembler: Arc<dyn ContextAssembler>,
    diagnostics: Arc<dyn RuntimeDiagnostics>,
}

impl FakeHandles {
    fn new() -> Self {
        Self {
            session_store: Arc::new(FakeSessionStore),
            provider_registry: Arc::new(FakeProviderRegistry),
            tool_runtime: Arc::new(FakeToolRuntime),
            policy: Arc::new(FakePolicy),
            plugin_runtime: Arc::new(FakePluginRuntime),
            context_assembler: Arc::new(FakeContextAssembler),
            diagnostics: Arc::new(FakeDiagnostics),
        }
    }

    fn builder(&self) -> RuntimeServicesBuilder {
        RuntimeServices::builder()
            .session_store(Arc::clone(&self.session_store))
            .provider_registry(Arc::clone(&self.provider_registry))
            .tool_runtime(Arc::clone(&self.tool_runtime))
            .policy(Arc::clone(&self.policy))
            .plugin_runtime(Arc::clone(&self.plugin_runtime))
            .context_assembler(Arc::clone(&self.context_assembler))
            .diagnostics(Arc::clone(&self.diagnostics))
    }
}

#[test]
fn runtime_services_constructs_with_all_required_services_normal() {
    let handles = FakeHandles::new();

    let services = handles.builder().build().expect("all services provided");

    assert_eq!(
        services.provider_registry().service_name(),
        "fake-provider-registry"
    );
    assert_eq!(services.tool_runtime().service_name(), "fake-tool-runtime");
    assert_eq!(services.policy().service_name(), "fake-policy");
    assert_eq!(
        services.plugin_runtime().service_name(),
        "fake-plugin-runtime"
    );
    assert_eq!(
        services.context_assembler().service_name(),
        "fake-context-assembler"
    );
    assert_eq!(services.diagnostics().service_name(), "fake-diagnostics");
}

#[test]
fn runtime_services_agent_runtime_exposes_required_services_and_diagnostics_normal() {
    let handles = FakeHandles::new();
    let services = handles.builder().build().expect("all services provided");

    let runtime = AgentRuntime::new(services);

    let snapshot = runtime.diagnostics_snapshot();
    assert_eq!(
        runtime.services().provider_registry().service_name(),
        "fake-provider-registry"
    );
    assert_eq!(snapshot.service_name, "fake-diagnostics");
    assert_eq!(snapshot.status, RuntimeDiagnosticsStatus::Healthy);
    assert_eq!(snapshot.detail.as_deref(), Some("all fake services online"));
    assert_eq!(runtime.diagnostic_entries().len(), 1);
    assert_eq!(
        runtime.diagnostic_entries()[0].message,
        "fake diagnostics reached runtime services"
    );
}

#[test]
fn runtime_services_provider_registry_fake_resolves_and_errors_robust() {
    let handles = FakeHandles::new();
    let services = handles.builder().build().expect("all services provided");

    let resolved = services
        .provider_registry()
        .resolve_provider_model("fake-model")
        .expect("fake registry should resolve fake-model");
    assert_eq!(resolved.provider.name(), "fake-registry-provider");
    assert_eq!(resolved.model.as_str(), "fake-model");

    let missing = services
        .provider_registry()
        .resolve_provider_model("missing-model")
        .expect_err("missing model must return typed provider registry error");
    assert_eq!(
        missing,
        ProviderRegistryError::MissingProvider {
            model: "missing-model".to_owned()
        }
    );
}

#[test]
fn runtime_services_missing_required_service_returns_typed_error_robust() {
    let handles = FakeHandles::new();

    let result = RuntimeServices::builder()
        .session_store(Arc::clone(&handles.session_store))
        .provider_registry(Arc::clone(&handles.provider_registry))
        .tool_runtime(Arc::clone(&handles.tool_runtime))
        .policy(Arc::clone(&handles.policy))
        .plugin_runtime(Arc::clone(&handles.plugin_runtime))
        .context_assembler(Arc::clone(&handles.context_assembler))
        .build();

    match result {
        Err(error) => assert_eq!(
            error,
            RuntimeServicesError::Missing(RuntimeServiceKind::Diagnostics)
        ),
        Ok(_) => panic!("missing diagnostics must not construct silently"),
    }
}
