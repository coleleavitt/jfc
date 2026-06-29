//! The `Engine` handle — the blessed embedding API for jfc frontends.
//!
//! A frontend owns an [`Engine`] plus the receiving half of its event
//! channel, and runs a loop of three moves:
//!
//! ```ignore
//! let (tx, mut rx) = jfc_engine::engine::channel();
//! let mut engine = Engine::new(provider, model, tx);
//! engine.submit("do the thing".into(), Vec::new(), None).await?;
//! while let Some(ev) = rx.recv().await {
//!     match engine.handle_event(ev).await? {
//!         Some(FrontendDirective::SubmitPrompt(text)) => {
//!             engine.submit(text, Vec::new(), None).await?;
//!         }
//!         Some(FrontendDirective::RunCommand(_)) | None => {}
//!     }
//!     for effect in engine.drain_effects() {
//!         // apply scroll/cache/picker reactions — or ignore when headless
//!         let _ = effect;
//!     }
//!     if engine.is_idle() {
//!         break;
//!     }
//! }
//! ```
//!
//! Everything here is sugar over [`EngineState`] + [`runtime::ops`] +
//! [`runtime::handle_engine_event`] — frontends with bespoke needs (the TUI
//! embeds `EngineState` directly inside its `App`) can keep using those
//! primitives; both layers are supported.

use std::sync::Arc;

use crate::app::{EngineEffect, EngineState};
use crate::runtime::{
    self, APP_EVENT_BUFFER, AgentRuntime, DefaultContextAssembler, DefaultPluginRuntime,
    EngineEvent, EventReceiver, EventSender, FrontendDirective, RuntimeServices,
    RuntimeServicesBuilder, RuntimeServicesError,
};

/// Create the engine event channel with the standard buffer size.
pub fn channel() -> (EventSender, EventReceiver) {
    tokio::sync::mpsc::channel(APP_EVENT_BUFFER)
}

/// Build the process-default [`RuntimeServices`] for the live engine from the
/// real builtin service impls (provider registry, tool runtime, permission
/// policy, diagnostics, session store) plus no-op plugin/context stubs.
///
/// Returns `None` (logged) if the builder is ever incomplete, so a missing
/// service can never panic engine construction. Cutover #1 keystone: this makes
/// a real services bundle exist on the live engine; live dispatch/session/cwd
/// paths do NOT yet read it (that is a later, test-backed step).
fn build_default_runtime_services(
    provider: Arc<dyn jfc_provider::Provider>,
) -> Option<RuntimeServices> {
    match RuntimeServices::builder()
        .session_store(Arc::new(crate::session::DefaultSessionStore))
        .provider_registry(Arc::new(
            crate::runtime::bootstrap::BuiltInProviderRegistry::new(vec![provider]),
        ))
        .tool_runtime(crate::tools::builtin_tool_runtime())
        .policy(Arc::new(crate::app::BuiltinRuntimePolicy))
        .plugin_runtime(Arc::new(DefaultPluginRuntime))
        .context_assembler(Arc::new(DefaultContextAssembler))
        .diagnostics(Arc::new(crate::diagnostics::GlobalDiagnosticsService))
        .build()
    {
        Ok(services) => Some(services),
        Err(error) => {
            tracing::warn!(
                target: "jfc::runtime",
                %error,
                "default runtime services unavailable; agent_runtime stays None"
            );
            None
        }
    }
}

/// An owned engine instance: the state plus the sending half of its event
/// bus. See the module docs for the canonical frontend loop.
pub struct Engine {
    pub state: EngineState,
    tx: EventSender,
    agent_runtime: Option<AgentRuntime>,
}

impl Engine {
    /// Construct the engine and register the global tool-event sender so
    /// detached producers (plan-mode tools, economy agents, schedulers)
    /// reach this engine's bus — the same wiring every frontend previously
    /// hand-rolled.
    pub fn new(
        provider: Arc<dyn jfc_provider::Provider>,
        model: impl Into<jfc_provider::ModelId>,
        tx: EventSender,
    ) -> Self {
        crate::tools::register_event_sender(tx.clone());
        // Register the elicitation event channel so jfc-mcp transports can
        // notify the engine when elicitation/create arrives from an MCP server.
        let engine_tx_for_elicit = tx.clone();
        let (elicit_tx, mut elicit_rx) =
            tokio::sync::mpsc::channel::<jfc_core::mcp_elicitation::ElicitationEvent>(64);
        jfc_core::mcp_elicitation::register_elicitation_event_sender(elicit_tx);
        tokio::spawn(async move {
            while let Some(ev) = elicit_rx.recv().await {
                match ev {
                    jfc_core::mcp_elicitation::ElicitationEvent::Arrived(snapshot) => {
                        let fe = crate::runtime::EngineEvent::Frontend(
                            crate::runtime::FrontendEvent::ElicitationRequest {
                                id: snapshot.id.clone(),
                                server_name: snapshot.server_name.clone(),
                                kind: snapshot.kind,
                            },
                        );
                        if engine_tx_for_elicit.send(fe).await.is_err() {
                            break; // engine shut down
                        }
                        // Fire OnElicitation hook
                        crate::hooks::fire_async(
                            crate::hooks::HookPoint::OnElicitation,
                            &crate::hooks::HookContext::for_session("<mcp-elicitation>")
                                .with_extra("server_name", snapshot.server_name.clone())
                                .with_extra("elicitation_id", snapshot.id.clone()),
                        );
                        // Also fire the unified OnUserInputRequired hook —
                        // elicitation blocks the turn until the user responds.
                        crate::hooks::fire_async(
                            crate::hooks::HookPoint::OnUserInputRequired,
                            &crate::hooks::HookContext::for_session("<mcp-elicitation>")
                                .with_extra("kind", "elicitation")
                                .with_extra(
                                    "message",
                                    format!(
                                        "MCP server '{}' is requesting structured input",
                                        snapshot.server_name
                                    ),
                                ),
                        );
                    }
                    jfc_core::mcp_elicitation::ElicitationEvent::Resolved {
                        id,
                        server_name,
                        mode,
                        action,
                    } => {
                        // Fire OnElicitationResult hook
                        crate::hooks::fire_async(
                            crate::hooks::HookPoint::OnElicitationResult,
                            &crate::hooks::HookContext::for_session("<mcp-elicitation>")
                                .with_extra("server_name", server_name)
                                .with_extra("elicitation_id", id)
                                .with_extra("mode", mode)
                                .with_extra("action", action),
                        );
                    }
                }
            }
        });
        let agent_runtime =
            build_default_runtime_services(Arc::clone(&provider)).map(AgentRuntime::new);
        Self {
            state: EngineState::new(provider, model),
            tx,
            agent_runtime,
        }
    }

    pub fn new_with_runtime_services(
        provider: Arc<dyn jfc_provider::Provider>,
        model: impl Into<jfc_provider::ModelId>,
        tx: EventSender,
        services: RuntimeServices,
    ) -> Self {
        let mut engine = Self::new(provider, model, tx);
        engine.agent_runtime = Some(AgentRuntime::new(services));
        engine
    }

    pub fn try_new_with_runtime_services(
        provider: Arc<dyn jfc_provider::Provider>,
        model: impl Into<jfc_provider::ModelId>,
        tx: EventSender,
        services: RuntimeServicesBuilder,
    ) -> Result<Self, RuntimeServicesError> {
        let services = services.build()?;
        Ok(Self::new_with_runtime_services(
            provider, model, tx, services,
        ))
    }

    pub fn agent_runtime(&self) -> Option<&AgentRuntime> {
        self.agent_runtime.as_ref()
    }

    /// A clone of the event sender for detached producers (remote hosts,
    /// schedulers, background workers).
    pub fn sender(&self) -> EventSender {
        self.tx.clone()
    }

    /// Dispatch one engine event. Returns the directives the engine cannot
    /// interpret on its own (prompt submission, slash commands) for the
    /// frontend to act on.
    pub async fn handle_event(
        &mut self,
        ev: EngineEvent,
    ) -> anyhow::Result<Option<FrontendDirective>> {
        runtime::handle_engine_event(&mut self.state, &self.tx, ev).await
    }

    /// Submit a user prompt (hooks, mention resolution, compaction gate,
    /// message push, stream spawn). The frontend pre-processes its own
    /// surface first (paste expansion, staged attachments, edit cursors).
    pub async fn submit(
        &mut self,
        text: String,
        attachments: Vec<jfc_core::Attachment>,
        edit_at: Option<usize>,
    ) -> anyhow::Result<runtime::ops::SubmitOutcome> {
        runtime::ops::submit_prompt(&mut self.state, &self.tx, text, attachments, edit_at).await
    }

    /// Start a turn over an externally seeded transcript (session resume,
    /// stream-json input).
    pub async fn start_turn_from_transcript(&mut self, turn_text: &str) {
        runtime::ops::start_turn_from_transcript(&mut self.state, &self.tx, turn_text).await;
    }

    /// Interrupt the current turn: cancel the stream, abort in-flight tools,
    /// deny pending approvals, kill bash subprocesses.
    pub fn interrupt(&mut self) {
        runtime::ops::interrupt(&mut self.state, &self.tx);
    }

    /// Load a session by id; view-side resets ride on the
    /// [`EngineEffect::SessionSwitched`] effect.
    pub async fn load_session(&mut self, id: jfc_core::SessionId) {
        runtime::ops::load_session(&mut self.state, id).await;
    }

    /// Resolve a parked tool approval by id (modal keys, remote control,
    /// headless permission policies all funnel here).
    pub fn resolve_approval(&mut self, tool_use_id: String, approved: bool) {
        runtime::approvals::handle_remote_approval_response(
            &mut self.state,
            &self.tx,
            tool_use_id,
            approved,
        );
    }

    /// Drain the view-facing effects queued since the last call. Headless
    /// frontends typically drop these; interactive ones map them onto
    /// scroll/cache/picker reactions.
    pub fn drain_effects(&mut self) -> Vec<EngineEffect> {
        std::mem::take(&mut self.state.effects)
    }

    /// True when no turn is running and nothing is parked: no live stream,
    /// no in-flight tools, no pending approvals, no queued prompts, no
    /// compaction. The standard headless termination check.
    pub fn is_idle(&self) -> bool {
        !self.state.has_interruptible_work()
            && self.state.pending_approval.is_none()
            && self.state.approval_queue.is_empty()
            && self.state.queued_prompts.is_empty()
            && self.state.compacting_started_at.is_none()
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use async_trait::async_trait;
    use jfc_provider::{EventStream, ModelInfo, Provider, ProviderMessage, StreamOptions};

    use super::*;
    use crate::ids::SessionId;
    use crate::runtime::{
        ContextAssembler, PluginRuntime, ProviderModelResolution, ProviderRegistry,
        ProviderRegistryError, RuntimeDiagnostics, RuntimeDiagnosticsSnapshot,
        RuntimeDiagnosticsStatus, RuntimePolicy, RuntimeService, RuntimeServiceKind,
        RuntimeServices, RuntimeServicesError, ToolRuntime,
    };
    use crate::session::{
        AutosaveOutcome, ListSessionsRequest, SearchSessionsRequest, SessionStore,
        SessionTranscript, StoredSessionMessage,
    };

    struct NoopProvider;

    impl jfc_provider::seal::Sealed for NoopProvider {}

    #[async_trait]
    impl Provider for NoopProvider {
        fn name(&self) -> &str {
            "noop-provider"
        }

        fn available_models(&self) -> Vec<ModelInfo> {
            vec![ModelInfo::new("test-model", "Test Model", self.name())]
        }

        async fn stream(
            &self,
            _messages: Vec<ProviderMessage>,
            _options: &StreamOptions,
        ) -> anyhow::Result<EventStream> {
            Ok(Box::pin(futures::stream::empty()))
        }
    }

    #[derive(Default)]
    struct FakeSessionStore;

    #[async_trait]
    impl SessionStore for FakeSessionStore {
        async fn save_transcript(&self, _request: jfc_session::SaveTranscriptRequest<'_>) {}

        async fn load_transcript(&self, _session_id: &SessionId) -> Option<SessionTranscript> {
            Some(SessionTranscript {
                messages: Vec::<StoredSessionMessage>::new(),
                model: Some("test-model".to_owned()),
            })
        }

        async fn set_title(&self, _session_id: &SessionId, _title: &str) {}

        async fn list_sessions(
            &self,
            _request: ListSessionsRequest<'_>,
        ) -> Vec<jfc_session::SessionMetadata> {
            Vec::new()
        }

        fn search_sessions(
            &self,
            _request: SearchSessionsRequest<'_>,
        ) -> Vec<jfc_session::SessionHit> {
            Vec::new()
        }

        async fn request_autosave(
            &self,
            _request: jfc_session::AutosaveRequest<'_>,
        ) -> AutosaveOutcome {
            AutosaveOutcome::Saved
        }
    }

    macro_rules! fake_runtime_service {
        ($name:ident, $trait_name:ident, $service_name:literal) => {
            struct $name;

            impl RuntimeService for $name {
                fn service_name(&self) -> &'static str {
                    $service_name
                }
            }

            impl $trait_name for $name {}
        };
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
            if model_id != "test-model" {
                return Err(ProviderRegistryError::MissingProvider {
                    model: model_id.to_owned(),
                });
            }

            let model = jfc_provider::ModelId::new(model_id);
            Ok(ProviderModelResolution {
                provider: Arc::new(NoopProvider),
                model: model.clone(),
                resolved_model: jfc_provider::ResolvedModel::new(
                    jfc_provider::ModelSpec::bare(model_id),
                    jfc_provider::ModelSpec::qualified(
                        jfc_provider::ProviderId::new("noop-provider"),
                        model,
                    ),
                    jfc_provider::ModelResolutionReason::CatalogMatch,
                    None,
                ),
            })
        }
    }
    fake_runtime_service!(FakeToolRuntime, ToolRuntime, "fake-tool-runtime");

    // RuntimePolicy now carries `tool_decision`, which the no-method
    // `fake_runtime_service!` macro can't generate — spell the fake out and
    // delegate to the same pure mode logic the real `BuiltinRuntimePolicy` uses.
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

    fake_runtime_service!(FakePluginRuntime, PluginRuntime, "fake-plugin-runtime");
    fake_runtime_service!(
        FakeContextAssembler,
        ContextAssembler,
        "fake-context-assembler"
    );

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
                detail: Some("engine service boundary".to_owned()),
            }
        }

        fn diagnostic_entries(&self) -> Vec<crate::diagnostics::DiagnosticEntry> {
            Vec::new()
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

        fn services(&self) -> RuntimeServices {
            self.builder().build().expect("all services provided")
        }

        fn builder(&self) -> crate::runtime::RuntimeServicesBuilder {
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

    fn provider() -> Arc<dyn Provider> {
        Arc::new(NoopProvider)
    }

    #[tokio::test]
    async fn engine_constructs_from_explicit_runtime_services_normal() {
        let (tx, _rx) = channel();
        let handles = FakeHandles::new();

        let engine =
            Engine::new_with_runtime_services(provider(), "test-model", tx, handles.services());

        let runtime = engine.agent_runtime().expect("runtime services attached");
        assert_eq!(
            runtime.services().provider_registry().service_name(),
            "fake-provider-registry"
        );
        assert_eq!(
            runtime.diagnostics_snapshot(),
            RuntimeDiagnosticsSnapshot {
                service_name: "fake-diagnostics",
                status: RuntimeDiagnosticsStatus::Healthy,
                detail: Some("engine service boundary".to_owned()),
            }
        );
    }

    #[tokio::test]
    async fn engine_missing_explicit_runtime_service_returns_typed_error_robust() {
        let (tx, _rx) = channel();
        let handles = FakeHandles::new();

        let result = Engine::try_new_with_runtime_services(
            provider(),
            "test-model",
            tx,
            RuntimeServices::builder()
                .session_store(Arc::clone(&handles.session_store))
                .provider_registry(Arc::clone(&handles.provider_registry))
                .tool_runtime(Arc::clone(&handles.tool_runtime))
                .policy(Arc::clone(&handles.policy))
                .plugin_runtime(Arc::clone(&handles.plugin_runtime))
                .context_assembler(Arc::clone(&handles.context_assembler)),
        );

        match result {
            Err(error) => assert_eq!(
                error,
                RuntimeServicesError::Missing(RuntimeServiceKind::Diagnostics)
            ),
            Ok(_) => panic!("missing diagnostics must be typed"),
        }
    }
}
