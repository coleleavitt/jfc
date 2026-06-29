use jfc_plugin_sdk::{
    RuntimeActionDescriptor, RuntimeActionKind, RuntimeActionOpenPanelTarget,
    RuntimeActionPayloadError,
};
use thiserror::Error;

#[derive(Debug, Clone)]
pub enum RuntimeActionFrontendDirective {
    HostAction(FrontendHostActionRequest),
    SlashCommand(String),
    OpenPanel(FrontendOpenPanelRequest),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FrontendHostActionRequest {
    pub source: RuntimeActionSource,
    pub action: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FrontendOpenPanelRequest {
    pub source: RuntimeActionSource,
    pub target: RuntimeActionOpenPanelTarget,
    pub panel: Option<FrontendPanelFocusRequest>,
    pub widget: Option<FrontendWidgetFocusRequest>,
    pub execute_panel_action: bool,
    pub execute_widget_action: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeActionSource {
    pub plugin_id: String,
    pub action_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FrontendPanelFocusRequest {
    pub plugin_id: String,
    pub panel_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FrontendWidgetFocusRequest {
    pub plugin_id: String,
    pub widget_id: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EngineRuntimeAction {
    RefreshMetrics,
    SendTeammateMessage,
    RefreshPromptContext,
    PluginSmoke,
    PluginDiagnostics,
}

#[derive(Debug, Clone)]
pub enum RuntimeActionOutcome {
    Engine(EngineRuntimeAction),
    Frontend(RuntimeActionFrontendDirective),
}

#[derive(Debug, Clone, Copy, Error, PartialEq, Eq)]
pub enum RuntimeActionBoundaryError {
    #[error("invalid {kind:?} runtime action payload: {payload_error:?}")]
    Payload {
        kind: RuntimeActionKind,
        payload_error: RuntimeActionPayloadError,
    },
}

pub fn resolve_runtime_action(
    action: &RuntimeActionDescriptor,
) -> Result<RuntimeActionOutcome, RuntimeActionBoundaryError> {
    match action.kind {
        RuntimeActionKind::HostAction => resolve_host_action(action),
        RuntimeActionKind::SlashCommand => resolve_slash_command(action),
        RuntimeActionKind::RefreshMetrics => Ok(engine_action(EngineRuntimeAction::RefreshMetrics)),
        RuntimeActionKind::OpenPanel => resolve_open_panel(action),
        RuntimeActionKind::SendTeammateMessage => {
            Ok(engine_action(EngineRuntimeAction::SendTeammateMessage))
        }
        RuntimeActionKind::RefreshPromptContext => {
            Ok(engine_action(EngineRuntimeAction::RefreshPromptContext))
        }
        RuntimeActionKind::PluginSmoke => Ok(engine_action(EngineRuntimeAction::PluginSmoke)),
        RuntimeActionKind::PluginDiagnostics => {
            Ok(engine_action(EngineRuntimeAction::PluginDiagnostics))
        }
    }
}

fn resolve_host_action(
    action: &RuntimeActionDescriptor,
) -> Result<RuntimeActionOutcome, RuntimeActionBoundaryError> {
    let host_action = action.host_action_payload().map_err(|payload_error| {
        RuntimeActionBoundaryError::Payload {
            kind: action.kind,
            payload_error,
        }
    })?;
    Ok(RuntimeActionOutcome::Frontend(
        RuntimeActionFrontendDirective::HostAction(FrontendHostActionRequest {
            source: RuntimeActionSource {
                plugin_id: action.plugin_id.as_str().to_owned(),
                action_id: action.id.clone(),
            },
            action: host_action.to_owned(),
        }),
    ))
}

fn resolve_slash_command(
    action: &RuntimeActionDescriptor,
) -> Result<RuntimeActionOutcome, RuntimeActionBoundaryError> {
    let command = action.slash_command_payload().map_err(|payload_error| {
        RuntimeActionBoundaryError::Payload {
            kind: action.kind,
            payload_error,
        }
    })?;
    Ok(RuntimeActionOutcome::Frontend(
        RuntimeActionFrontendDirective::SlashCommand(command.to_owned()),
    ))
}

fn resolve_open_panel(
    action: &RuntimeActionDescriptor,
) -> Result<RuntimeActionOutcome, RuntimeActionBoundaryError> {
    let payload = action.open_panel_payload().map_err(|payload_error| {
        RuntimeActionBoundaryError::Payload {
            kind: action.kind,
            payload_error,
        }
    })?;
    let source = RuntimeActionSource {
        plugin_id: action.plugin_id.as_str().to_owned(),
        action_id: action.id.clone(),
    };
    let panel = payload.panel_id.map(|panel_id| FrontendPanelFocusRequest {
        plugin_id: payload
            .panel_plugin_id
            .or(payload.plugin_id)
            .unwrap_or_else(|| action.plugin_id.as_str())
            .to_owned(),
        panel_id: panel_id.to_owned(),
    });
    let widget = payload
        .widget_id
        .map(|widget_id| FrontendWidgetFocusRequest {
            plugin_id: payload
                .widget_plugin_id
                .or(payload.plugin_id)
                .unwrap_or_else(|| action.plugin_id.as_str())
                .to_owned(),
            widget_id: widget_id.to_owned(),
        });
    Ok(RuntimeActionOutcome::Frontend(
        RuntimeActionFrontendDirective::OpenPanel(FrontendOpenPanelRequest {
            source,
            target: payload.target,
            panel,
            widget,
            execute_panel_action: payload.execute_panel_action,
            execute_widget_action: payload.execute_widget_action,
        }),
    ))
}

const fn engine_action(action: EngineRuntimeAction) -> RuntimeActionOutcome {
    RuntimeActionOutcome::Engine(action)
}

#[cfg(test)]
mod tests {
    use super::*;
    use jfc_plugin_sdk::PluginId;

    #[test]
    fn host_action_runtime_action_resolves_to_frontend_directive_normal() {
        let action = RuntimeActionDescriptor::new(
            PluginId::new("plugin.palette"),
            "host.toggle_info",
            "Toggle Info Sidebar",
            "Toggle the info sidebar from a runtime action",
            RuntimeActionKind::HostAction,
        )
        .with_payload(serde_json::json!({ "action": "toggle_info_sidebar" }));

        let outcome = resolve_runtime_action(&action);

        match outcome {
            Ok(RuntimeActionOutcome::Frontend(RuntimeActionFrontendDirective::HostAction(
                request,
            ))) => {
                assert_eq!(request.source.plugin_id, "plugin.palette");
                assert_eq!(request.source.action_id, "host.toggle_info");
                assert_eq!(request.action, "toggle_info_sidebar");
            }
            other => panic!("expected HostAction frontend directive, got {other:?}"),
        }
    }

    #[test]
    fn open_panel_runtime_action_resolves_to_frontend_directive_normal() {
        let action = RuntimeActionDescriptor::new(
            PluginId::new("plugin.palette"),
            "panel.open",
            "Open Review Panel",
            "Open and focus review panel",
            RuntimeActionKind::OpenPanel,
        )
        .with_payload(serde_json::json!({
            "panel": "info_sidebar",
            "panel_id": "review.panel",
            "execute_panel_action": true
        }));

        let outcome = resolve_runtime_action(&action);

        match outcome {
            Ok(RuntimeActionOutcome::Frontend(RuntimeActionFrontendDirective::OpenPanel(
                request,
            ))) => {
                assert_eq!(request.source.plugin_id, "plugin.palette");
                assert_eq!(request.source.action_id, "panel.open");
                assert_eq!(request.target, RuntimeActionOpenPanelTarget::InfoSidebar);
                assert_eq!(
                    request.panel,
                    Some(FrontendPanelFocusRequest {
                        plugin_id: "plugin.palette".to_owned(),
                        panel_id: "review.panel".to_owned(),
                    })
                );
                assert!(request.widget.is_none());
                assert!(request.execute_panel_action);
                assert!(!request.execute_widget_action);
            }
            other => panic!("expected OpenPanel frontend directive, got {other:?}"),
        }
    }

    #[test]
    fn refresh_metrics_runtime_action_resolves_to_engine_effect_normal() {
        let action = RuntimeActionDescriptor::new(
            PluginId::new("plugin.palette"),
            "metrics.refresh",
            "Refresh Metrics",
            "Refresh metrics state",
            RuntimeActionKind::RefreshMetrics,
        );

        let outcome = resolve_runtime_action(&action);

        match outcome {
            Ok(RuntimeActionOutcome::Engine(EngineRuntimeAction::RefreshMetrics)) => {}
            other => panic!("expected RefreshMetrics engine effect, got {other:?}"),
        }
    }

    #[test]
    fn malformed_host_action_runtime_action_returns_typed_error_robust() {
        let action = RuntimeActionDescriptor::new(
            PluginId::new("plugin.palette"),
            "host.toggle_info",
            "Toggle Info Sidebar",
            "Toggle the info sidebar from a runtime action",
            RuntimeActionKind::HostAction,
        );

        match resolve_runtime_action(&action) {
            Err(error) => assert_eq!(
                error,
                RuntimeActionBoundaryError::Payload {
                    kind: RuntimeActionKind::HostAction,
                    payload_error: RuntimeActionPayloadError::MissingHostAction,
                }
            ),
            Ok(outcome) => panic!("expected typed HostAction payload error, got {outcome:?}"),
        }
    }

    #[test]
    fn malformed_open_panel_runtime_action_returns_typed_error_robust() {
        let action = RuntimeActionDescriptor::new(
            PluginId::new("plugin.palette"),
            "panel.open",
            "Open Floating Debugger",
            "Open an unsupported panel",
            RuntimeActionKind::OpenPanel,
        )
        .with_payload(serde_json::json!({ "panel": "floating_debugger" }));

        match resolve_runtime_action(&action) {
            Err(error) => assert_eq!(
                error,
                RuntimeActionBoundaryError::Payload {
                    kind: RuntimeActionKind::OpenPanel,
                    payload_error: RuntimeActionPayloadError::UnsupportedOpenPanelTarget,
                }
            ),
            Ok(outcome) => panic!("expected typed OpenPanel payload error, got {outcome:?}"),
        }
    }
}
