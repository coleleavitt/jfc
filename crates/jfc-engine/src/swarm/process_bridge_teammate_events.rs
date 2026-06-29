use jfc_plugin_sdk::{
    BridgeAgentLaunchResult, BridgeEnvelope, BridgeRequest, BridgeResponse, BridgeTeammateEvent,
};

use super::runner::TeammateEvent;

pub(crate) struct TeammateBridgeEventContext<'a> {
    pub(crate) launcher_name: &'a str,
    pub(crate) request_id: &'a str,
    pub(crate) task_id: &'a str,
    pub(crate) agent_id: &'a str,
    pub(crate) agent_name: &'a str,
}

pub(crate) enum BridgeTeammateFrame {
    Event(TeammateEvent),
    HostRequest { id: String, request: BridgeRequest },
}

pub(crate) fn response_line_to_teammate_frame(
    context: &TeammateBridgeEventContext<'_>,
    line: &str,
) -> Result<BridgeTeammateFrame, String> {
    let frame = serde_json::from_str::<BridgeEnvelope>(line).map_err(|error| {
        format!(
            "ProcessBridge teammate launcher `{}` returned invalid JSONL: {error}",
            context.launcher_name
        )
    })?;
    match frame {
        BridgeEnvelope::Response { id, response } => {
            if id != context.request_id {
                return Err(format!(
                    "ProcessBridge teammate launcher `{}` response id `{id}` did not match `{}`",
                    context.launcher_name, context.request_id
                ));
            }
            bridge_response_to_teammate_event(context, response).map(BridgeTeammateFrame::Event)
        }
        BridgeEnvelope::Request { id, request } => {
            Ok(BridgeTeammateFrame::HostRequest { id, request })
        }
    }
}

pub(crate) fn is_terminal_event(event: &TeammateEvent) -> bool {
    matches!(
        event,
        TeammateEvent::Completed { .. }
            | TeammateEvent::Cancelled { .. }
            | TeammateEvent::Failed { .. }
    )
}

fn bridge_response_to_teammate_event(
    context: &TeammateBridgeEventContext<'_>,
    response: BridgeResponse,
) -> Result<TeammateEvent, String> {
    match response {
        BridgeResponse::TeammateEvent { event } => Ok(map_teammate_event(context, event)),
        BridgeResponse::AgentLaunchResult { result } => {
            Ok(map_agent_launch_result(context, result))
        }
        BridgeResponse::Error(error) => Ok(TeammateEvent::Failed {
            task_id: context.task_id.to_owned(),
            agent_id: context.agent_id.to_owned(),
            error: format!("ProcessBridge error `{}`: {}", error.code, error.message),
        }),
        other => Err(format!(
            "ProcessBridge teammate launcher `{}` returned unexpected response: {other:?}",
            context.launcher_name
        )),
    }
}

fn map_teammate_event(
    context: &TeammateBridgeEventContext<'_>,
    event: BridgeTeammateEvent,
) -> TeammateEvent {
    match event {
        BridgeTeammateEvent::TextDelta { delta } => TeammateEvent::TextDelta {
            task_id: context.task_id.to_owned(),
            agent_id: context.agent_id.to_owned(),
            delta,
        },
        BridgeTeammateEvent::Progress {
            token_count,
            tool_use_count,
            last_tool,
            model_id,
            cost_usd,
        } => TeammateEvent::Progress {
            task_id: context.task_id.to_owned(),
            agent_id: context.agent_id.to_owned(),
            token_count,
            tool_use_count,
            last_tool,
            model_id,
            cost_usd,
        },
        BridgeTeammateEvent::Idle {
            agent_name,
            reason,
            summary,
        } => TeammateEvent::Idle {
            task_id: context.task_id.to_owned(),
            agent_id: context.agent_id.to_owned(),
            agent_name: agent_name.unwrap_or_else(|| context.agent_name.to_owned()),
            reason,
            summary,
        },
        BridgeTeammateEvent::MessageSent {
            from,
            to,
            text,
            summary,
        } => TeammateEvent::MessageSent {
            task_id: context.task_id.to_owned(),
            agent_id: context.agent_id.to_owned(),
            from,
            to,
            text,
            summary,
        },
        BridgeTeammateEvent::Completed => TeammateEvent::Completed {
            task_id: context.task_id.to_owned(),
            agent_id: context.agent_id.to_owned(),
        },
        BridgeTeammateEvent::Cancelled => TeammateEvent::Cancelled {
            task_id: context.task_id.to_owned(),
            agent_id: context.agent_id.to_owned(),
        },
        BridgeTeammateEvent::Failed { error } => TeammateEvent::Failed {
            task_id: context.task_id.to_owned(),
            agent_id: context.agent_id.to_owned(),
            error,
        },
    }
}

fn map_agent_launch_result(
    context: &TeammateBridgeEventContext<'_>,
    result: BridgeAgentLaunchResult,
) -> TeammateEvent {
    if result.is_error {
        TeammateEvent::Failed {
            task_id: context.task_id.to_owned(),
            agent_id: context.agent_id.to_owned(),
            error: result.output,
        }
    } else {
        TeammateEvent::Completed {
            task_id: context.task_id.to_owned(),
            agent_id: context.agent_id.to_owned(),
        }
    }
}
