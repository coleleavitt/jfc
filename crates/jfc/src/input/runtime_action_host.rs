use crate::app::App;
use jfc_engine::runtime::{
    FrontendHostActionRequest, RuntimeActionBoundaryError, RuntimeActionFrontendDirective,
    RuntimeActionOutcome, resolve_runtime_action,
};
use jfc_plugin_sdk::{RuntimeActionDescriptor, RuntimeActionPayloadError};

pub(super) async fn execute_host_action(app: &mut App, action: &RuntimeActionDescriptor) {
    let Some(request) = host_action_frontend_directive(action) else {
        return;
    };
    apply_host_action_directive(app, &request).await;
}

async fn apply_host_action_directive(app: &mut App, request: &FrontendHostActionRequest) {
    super::host_palette_action::execute_host_palette_action_name(app, request.action.as_str())
        .await;
}

fn host_action_frontend_directive(
    action: &RuntimeActionDescriptor,
) -> Option<FrontendHostActionRequest> {
    match resolve_runtime_action(action) {
        Ok(RuntimeActionOutcome::Frontend(RuntimeActionFrontendDirective::HostAction(request))) => {
            Some(request)
        }
        Ok(outcome) => {
            tracing::warn!(
                target: "jfc::palette",
                plugin = action.plugin_id.as_str(),
                action = action.id.as_str(),
                outcome = ?outcome,
                "runtime action did not resolve to HostAction frontend directive"
            );
            None
        }
        Err(RuntimeActionBoundaryError::Payload {
            payload_error: RuntimeActionPayloadError::MissingHostAction,
            ..
        }) => {
            tracing::warn!(
                target: "jfc::palette",
                plugin = action.plugin_id.as_str(),
                action = action.id.as_str(),
                key = "action",
                "runtime action is missing required payload field"
            );
            None
        }
        Err(RuntimeActionBoundaryError::Payload { payload_error, .. }) => {
            tracing::warn!(
                target: "jfc::palette",
                plugin = action.plugin_id.as_str(),
                action = action.id.as_str(),
                reason = payload_error.as_manifest_reason(),
                "invalid runtime-action HostAction payload"
            );
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use jfc_plugin_sdk::{PluginId, RuntimeActionKind};
    use jfc_provider::{EventStream, ModelInfo, Provider, ProviderMessage, StreamOptions};

    struct TestProvider;

    #[async_trait::async_trait]
    impl Provider for TestProvider {
        fn name(&self) -> &str {
            "test"
        }

        fn available_models(&self) -> Vec<ModelInfo> {
            Vec::new()
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

    fn test_app() -> App {
        let mut app = App::new(Arc::new(TestProvider), "test-model");
        app.engine.task_store = jfc_session::TaskStore::in_memory();
        app
    }

    #[tokio::test]
    async fn host_action_runtime_action_applies_frontend_directive_state_normal() {
        let mut app = test_app();
        app.info_sidebar.visible = false;
        let action = host_action(serde_json::json!({ "action": "toggle_info_sidebar" }));

        execute_host_action(&mut app, &action).await;

        assert!(app.info_sidebar.visible);
    }

    #[tokio::test]
    async fn malformed_host_action_runtime_action_is_ignored_robust() {
        let mut app = test_app();
        app.info_sidebar.visible = false;
        let action = RuntimeActionDescriptor::new(
            PluginId::new("plugin.palette"),
            "host.toggle_info",
            "Toggle Info Sidebar",
            "Toggle the info sidebar from a runtime action",
            RuntimeActionKind::HostAction,
        );

        execute_host_action(&mut app, &action).await;

        assert!(!app.info_sidebar.visible);
    }

    #[tokio::test]
    async fn unknown_host_action_runtime_action_is_ignored_robust() {
        let mut app = test_app();
        app.info_sidebar.visible = false;
        let action = host_action(serde_json::json!({ "action": "shell_out_to_random_code" }));

        execute_host_action(&mut app, &action).await;

        assert!(!app.info_sidebar.visible);
    }

    fn host_action(payload: serde_json::Value) -> RuntimeActionDescriptor {
        RuntimeActionDescriptor::new(
            PluginId::new("plugin.palette"),
            "host.toggle_info",
            "Toggle Info Sidebar",
            "Toggle the info sidebar from a runtime action",
            RuntimeActionKind::HostAction,
        )
        .with_payload(payload)
    }
}
