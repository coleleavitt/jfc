use std::path::Path;
use std::sync::Arc;

use async_trait::async_trait;
use thiserror::Error;
use tokio::sync::Mutex;

use crate::app::{PermissionDecision, PermissionMode};
use crate::context::ReadDedupCache;
use crate::runtime::ExecutionResult;
use crate::session::SessionStore;
use crate::types::{ToolInput, ToolKind};
use jfc_provider::{ModelId, Provider, ResolvedModel};

pub trait RuntimeService: Send + Sync {
    fn service_name(&self) -> &'static str;
}

pub trait ProviderRegistry: RuntimeService {
    fn resolve_provider_model(
        &self,
        model_id: &str,
    ) -> Result<ProviderModelResolution, ProviderRegistryError>;
}

pub struct ProviderModelResolution {
    pub provider: Arc<dyn Provider>,
    pub model: ModelId,
    pub resolved_model: ResolvedModel,
}

impl std::fmt::Debug for ProviderModelResolution {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProviderModelResolution")
            .field("provider", &self.provider.name())
            .field("model", &self.model)
            .field("resolved_model", &self.resolved_model)
            .finish()
    }
}

#[derive(Debug, Clone, Error, PartialEq, Eq)]
pub enum ProviderRegistryError {
    #[error("provider registry has no provider for model `{model}`")]
    MissingProvider { model: String },
}

#[derive(Clone, Debug, PartialEq)]
pub struct ToolRuntimeCatalogEntry {
    pub kind: ToolKind,
    pub name: String,
    pub read_only: bool,
}

impl ToolRuntimeCatalogEntry {
    pub fn read_only(kind: ToolKind) -> Self {
        Self {
            name: kind.api_name().to_owned(),
            kind,
            read_only: true,
        }
    }
}

pub struct ToolRuntimeRequest<'a> {
    pub kind: &'a ToolKind,
    pub input: &'a ToolInput,
    pub cwd: &'a Path,
    pub dedup: Option<&'a Arc<Mutex<ReadDedupCache>>>,
    pub runtime_tool_id: Option<&'a str>,
}

#[async_trait]
pub trait ToolRuntime: RuntimeService {
    fn catalog(&self) -> Vec<ToolRuntimeCatalogEntry> {
        Vec::new()
    }

    async fn dispatch(&self, _request: ToolRuntimeRequest<'_>) -> Option<ExecutionResult> {
        None
    }
}

pub trait RuntimePolicy: RuntimeService {
    /// Mode-level auto-approval decision for a tool call, evaluated *before*
    /// session/always allowlists are consulted. Fronts
    /// [`PermissionMode::decide_parts`] so the runtime reads tool gating
    /// through the policy service boundary instead of calling the mode enum
    /// directly. Mirrors how [`RuntimeDiagnostics::diagnostic_entries`] fronts
    /// the global diagnostics store.
    fn tool_decision(
        &self,
        mode: PermissionMode,
        kind: &ToolKind,
        input: &ToolInput,
    ) -> PermissionDecision;
}

pub trait PluginRuntime: RuntimeService {}

pub trait ContextAssembler: RuntimeService {}

pub trait RuntimeDiagnostics: RuntimeService {
    fn snapshot(&self) -> RuntimeDiagnosticsSnapshot;

    fn diagnostic_entries(&self) -> Vec<crate::diagnostics::DiagnosticEntry>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeDiagnosticsSnapshot {
    pub service_name: &'static str,
    pub status: RuntimeDiagnosticsStatus,
    pub detail: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeDiagnosticsStatus {
    Healthy,
    Degraded,
}

pub struct RuntimeServices {
    session_store: Arc<dyn SessionStore>,
    provider_registry: Arc<dyn ProviderRegistry>,
    tool_runtime: Arc<dyn ToolRuntime>,
    policy: Arc<dyn RuntimePolicy>,
    plugin_runtime: Arc<dyn PluginRuntime>,
    context_assembler: Arc<dyn ContextAssembler>,
    diagnostics: Arc<dyn RuntimeDiagnostics>,
}

impl RuntimeServices {
    pub fn builder() -> RuntimeServicesBuilder {
        RuntimeServicesBuilder::default()
    }

    pub fn session_store(&self) -> Arc<dyn SessionStore> {
        Arc::clone(&self.session_store)
    }

    pub fn provider_registry(&self) -> Arc<dyn ProviderRegistry> {
        Arc::clone(&self.provider_registry)
    }

    pub fn tool_runtime(&self) -> Arc<dyn ToolRuntime> {
        Arc::clone(&self.tool_runtime)
    }

    pub fn policy(&self) -> Arc<dyn RuntimePolicy> {
        Arc::clone(&self.policy)
    }

    pub fn plugin_runtime(&self) -> Arc<dyn PluginRuntime> {
        Arc::clone(&self.plugin_runtime)
    }

    pub fn context_assembler(&self) -> Arc<dyn ContextAssembler> {
        Arc::clone(&self.context_assembler)
    }

    pub fn diagnostics(&self) -> Arc<dyn RuntimeDiagnostics> {
        Arc::clone(&self.diagnostics)
    }
}

#[derive(Default)]
pub struct RuntimeServicesBuilder {
    session_store: Option<Arc<dyn SessionStore>>,
    provider_registry: Option<Arc<dyn ProviderRegistry>>,
    tool_runtime: Option<Arc<dyn ToolRuntime>>,
    policy: Option<Arc<dyn RuntimePolicy>>,
    plugin_runtime: Option<Arc<dyn PluginRuntime>>,
    context_assembler: Option<Arc<dyn ContextAssembler>>,
    diagnostics: Option<Arc<dyn RuntimeDiagnostics>>,
}

impl RuntimeServicesBuilder {
    pub fn session_store(mut self, service: Arc<dyn SessionStore>) -> Self {
        self.session_store = Some(service);
        self
    }

    pub fn provider_registry(mut self, service: Arc<dyn ProviderRegistry>) -> Self {
        self.provider_registry = Some(service);
        self
    }

    pub fn tool_runtime(mut self, service: Arc<dyn ToolRuntime>) -> Self {
        self.tool_runtime = Some(service);
        self
    }

    pub fn policy(mut self, service: Arc<dyn RuntimePolicy>) -> Self {
        self.policy = Some(service);
        self
    }

    pub fn plugin_runtime(mut self, service: Arc<dyn PluginRuntime>) -> Self {
        self.plugin_runtime = Some(service);
        self
    }

    pub fn context_assembler(mut self, service: Arc<dyn ContextAssembler>) -> Self {
        self.context_assembler = Some(service);
        self
    }

    pub fn diagnostics(mut self, service: Arc<dyn RuntimeDiagnostics>) -> Self {
        self.diagnostics = Some(service);
        self
    }

    pub fn build(self) -> Result<RuntimeServices, RuntimeServicesError> {
        Ok(RuntimeServices {
            session_store: self.session_store.ok_or(RuntimeServicesError::Missing(
                RuntimeServiceKind::SessionStore,
            ))?,
            provider_registry: self.provider_registry.ok_or(RuntimeServicesError::Missing(
                RuntimeServiceKind::ProviderRegistry,
            ))?,
            tool_runtime: self.tool_runtime.ok_or(RuntimeServicesError::Missing(
                RuntimeServiceKind::ToolRuntime,
            ))?,
            policy: self
                .policy
                .ok_or(RuntimeServicesError::Missing(RuntimeServiceKind::Policy))?,
            plugin_runtime: self.plugin_runtime.ok_or(RuntimeServicesError::Missing(
                RuntimeServiceKind::PluginRuntime,
            ))?,
            context_assembler: self.context_assembler.ok_or(RuntimeServicesError::Missing(
                RuntimeServiceKind::ContextAssembler,
            ))?,
            diagnostics: self.diagnostics.ok_or(RuntimeServicesError::Missing(
                RuntimeServiceKind::Diagnostics,
            ))?,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeServiceKind {
    SessionStore,
    ProviderRegistry,
    ToolRuntime,
    Policy,
    PluginRuntime,
    ContextAssembler,
    Diagnostics,
}

impl std::fmt::Display for RuntimeServiceKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::SessionStore => "session store",
            Self::ProviderRegistry => "provider registry",
            Self::ToolRuntime => "tool runtime",
            Self::Policy => "policy",
            Self::PluginRuntime => "plugin runtime",
            Self::ContextAssembler => "context assembler",
            Self::Diagnostics => "diagnostics",
        })
    }
}

#[derive(Debug, Clone, Copy, Error, PartialEq, Eq)]
pub enum RuntimeServicesError {
    #[error("missing required runtime service: {0}")]
    Missing(RuntimeServiceKind),
}

pub struct AgentRuntime {
    services: RuntimeServices,
}

impl AgentRuntime {
    pub fn new(services: RuntimeServices) -> Self {
        Self { services }
    }

    pub fn services(&self) -> &RuntimeServices {
        &self.services
    }

    pub fn diagnostics_snapshot(&self) -> RuntimeDiagnosticsSnapshot {
        self.services.diagnostics.snapshot()
    }

    pub fn diagnostic_entries(&self) -> Vec<crate::diagnostics::DiagnosticEntry> {
        self.services.diagnostics.diagnostic_entries()
    }
}

/// Process-default no-op [`PluginRuntime`]. `PluginRuntime` is still a marker
/// trait (cutover #3 gives the service traits real methods incrementally); this
/// stub lets the live engine hold a complete [`RuntimeServices`] today without
/// rerouting any plugin behavior through it yet.
pub struct DefaultPluginRuntime;

impl RuntimeService for DefaultPluginRuntime {
    fn service_name(&self) -> &'static str {
        "default-plugin-runtime"
    }
}

impl PluginRuntime for DefaultPluginRuntime {}

/// Process-default no-op [`ContextAssembler`] (marker trait today). Same purpose
/// as [`DefaultPluginRuntime`]: a placeholder so the service bundle is complete.
pub struct DefaultContextAssembler;

impl RuntimeService for DefaultContextAssembler {
    fn service_name(&self) -> &'static str {
        "default-context-assembler"
    }
}

impl ContextAssembler for DefaultContextAssembler {}

#[cfg(test)]
mod tests;
