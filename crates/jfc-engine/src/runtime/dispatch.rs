//! The frontend-neutral engine event dispatch — stage 4 of the jfc-engine
//! extraction. One function, shared by every frontend.

use crate::app::EngineState;
use crate::runtime::ControlEvent;
use crate::runtime::{
    EngineEvent, EventSender, FrontendEvent, GoalEvent, StreamEvent, TaskEvent, ToolEvent,
};

/// Events the engine cannot interpret on its own — the owning frontend
/// decides what they mean (the TUI runs its submit/slash pipelines; headless
/// maps them onto `ops` directly or reports them unsupported).
#[derive(Debug)]
pub enum FrontendDirective {
    /// Submit this text as a user prompt (frontend pre-processing applies:
    /// paste-chip expansion, staged attachments, edit cursors).
    SubmitPrompt(String),
    /// Run a slash command through the frontend's command dispatch.
    RunCommand(String),
}

/// Dispatch one engine event against the engine state. This is the entire
/// frontend-neutral event pump — every frontend (TUI loop, headless print
/// mode, remote/daemon drivers) funnels engine events through here. It must
/// never touch view state or terminal handles. The few events that are the
/// frontend's to interpret (prompt submission, slash commands) come back as
/// a [`FrontendDirective`].
pub async fn handle_engine_event(
    state: &mut EngineState,
    tx: &EventSender,
    ev: EngineEvent,
) -> anyhow::Result<Option<FrontendDirective>> {
    match ev {
        // ── Team events ─────────────────────────────────────────
        EngineEvent::Team(ev) => {
            crate::runtime::event_loop::handlers::team::handle_team_event(state, &tx, ev).await;
        }

        // ── Stream: chunk / tool-input / redacted / response-id ─
        EngineEvent::Stream(StreamEvent::Chunk { text, reasoning }) => {
            crate::runtime::event_loop::handlers::stream_chunk::handle_chunk(
                state, text, reasoning,
            );
        }
        EngineEvent::Stream(StreamEvent::ToolInputDelta { delta, .. }) => {
            crate::runtime::event_loop::handlers::stream_chunk::handle_tool_input_delta(
                state,
                delta.len(),
            );
        }
        EngineEvent::Stream(StreamEvent::ThinkingTokens(tokens)) => {
            crate::runtime::event_loop::handlers::stream_chunk::handle_thinking_tokens(
                state, tokens,
            );
        }
        EngineEvent::Stream(StreamEvent::RedactedThinking(data)) => {
            crate::runtime::event_loop::handlers::stream_chunk::handle_redacted_thinking(
                state, data,
            );
        }
        EngineEvent::Stream(StreamEvent::ResponseId { id, .. }) => {
            crate::runtime::event_loop::handlers::stream_chunk::handle_response_id(state, id);
        }

        // ── Stream: tool announcement ───────────────────────────
        EngineEvent::Stream(StreamEvent::Tool(tool)) => {
            crate::runtime::event_loop::handlers::stream_tool::handle_stream_tool(state, &tx, tool)
                .await;
        }
        EngineEvent::Tool(ToolEvent::ClassifierDecision {
            tool,
            blocked,
            reason,
        }) => {
            crate::runtime::event_loop::handlers::stream_tool::handle_classifier_decision(
                state, &tx, tool, blocked, reason,
            )
            .await;
        }
        EngineEvent::Tool(ToolEvent::SetInProgressToolUseIds { action, ids }) => {
            crate::runtime::event_loop::handlers::tools::handle_set_in_progress_tool_use_ids(
                state, action, ids,
            );
        }
        EngineEvent::Tool(ToolEvent::DeferredToolUse {
            id,
            name,
            input_preview,
            reason,
        }) => {
            crate::runtime::event_loop::handlers::tools::handle_deferred_tool_use(
                state,
                id,
                name,
                input_preview,
                reason,
            );
        }
        EngineEvent::Tool(ToolEvent::UseSummary {
            summary,
            preceding_tool_use_ids,
        }) => {
            crate::runtime::event_loop::handlers::tools::handle_tool_use_summary(
                state,
                summary,
                preceding_tool_use_ids,
            );
        }
        EngineEvent::Stream(StreamEvent::ServerToolResult {
            tool_use_id,
            tool_kind,
            content,
        }) => {
            crate::runtime::event_loop::handlers::stream_tool::handle_server_tool_result(
                state,
                &tx,
                tool_use_id,
                tool_kind,
                content,
            );
        }

        // ── Stream: done ────────────────────────────────────────
        EngineEvent::Stream(StreamEvent::Done(stop_reason)) => {
            crate::runtime::event_loop::handlers::stream_done::handle_stream_done(
                state,
                &tx,
                stop_reason,
            )
            .await;
        }

        // ── Stream: error ───────────────────────────────────────
        EngineEvent::Stream(StreamEvent::Error(e)) => {
            crate::runtime::event_loop::handlers::stream_error::handle_stream_error(state, &tx, e)
                .await;
        }

        // ── Stream: fallback ────────────────────────────────────
        EngineEvent::Stream(StreamEvent::FallbackTriggered {
            original_model,
            fallback_model,
            reason,
        }) => {
            crate::runtime::event_loop::handlers::stream_error::handle_fallback_triggered(
                state,
                &original_model,
                &fallback_model,
                &reason,
            );
        }

        // ── Stream: usage ───────────────────────────────────────
        EngineEvent::Stream(StreamEvent::Usage {
            input_tokens,
            output_tokens,
            cache_read_tokens,
            cache_write_tokens,
        }) => {
            crate::runtime::event_loop::handlers::stream_usage::handle_stream_usage(
                state,
                input_tokens,
                output_tokens,
                cache_read_tokens,
                cache_write_tokens,
            );
        }

        // ── Stream: metadata ────────────────────────────────────
        EngineEvent::Stream(StreamEvent::SystemPromptLen(len)) => {
            crate::runtime::event_loop::handlers::ui_actions::handle_system_prompt_len(state, len);
        }
        EngineEvent::Stream(StreamEvent::MemoryRecalled(chars)) => {
            // The recall block was injected this turn; show its size in
            // the same chars/4 token model the context gauge uses (no
            // `~` prefix — it's presented consistently with every other
            // token figure in the UI, not flagged as a guess).
            let tokens = chars / 4;
            crate::toast::push_with_cap(
                &mut state.toasts,
                crate::toast::Toast::new(
                    crate::toast::ToastKind::Info,
                    format!("↻ Recalled memory ({tokens} tokens of context)"),
                ),
            );
        }
        EngineEvent::Stream(StreamEvent::RequestMetadata(meta)) => {
            crate::runtime::event_loop::handlers::ui_actions::handle_request_metadata(state, meta);
        }
        EngineEvent::Stream(StreamEvent::Lifecycle(status)) => {
            crate::runtime::event_loop::handlers::ui_actions::handle_stream_lifecycle(
                state, status,
            );
        }
        EngineEvent::Stream(StreamEvent::Keepalive) => {
            // Wire-liveness tick (SSE ping/keepalive) — reset the stream idle
            // watchdog clock and nothing else. This is what lets a slow-but-
            // alive stream (long thinking pause, large tool-input generation)
            // survive instead of being cancelled by `check_stream_watchdog`.
            state.record_stream_activity();
        }

        // ── Provider events ─────────────────────────────────────
        EngineEvent::Provider(ev) => {
            crate::runtime::event_loop::handlers::provider::handle_provider_event(state, ev);
        }

        // ── Tool execution events ───────────────────────────────
        EngineEvent::Tool(ToolEvent::OutputChunk { tool_id, chunk }) => {
            crate::runtime::event_loop::handlers::tools::handle_output_chunk(state, tool_id, chunk);
        }
        EngineEvent::Tool(ToolEvent::Result { tool_id, result }) => {
            crate::runtime::event_loop::handlers::tools::handle_tool_result(
                state, &tx, tool_id, result,
            );
            if crate::runtime::event_loop::handlers::tools::should_recheck_completion_after_tool_result(&*state) {
                tracing::warn!(
            target: "jfc::stream",
            "ToolResult completed a turn after its AllComplete signal — rechecking continuation"
                );
                crate::runtime::event_loop::handlers::tools::handle_all_complete(state, &tx).await;
            }
        }
        EngineEvent::Tool(ToolEvent::AllComplete) => {
            crate::runtime::event_loop::handlers::tools::handle_all_complete(state, &tx).await;
        }

        // ── Goal evaluation ─────────────────────────────────────
        EngineEvent::Goal(GoalEvent::Verdict { ok, reason }) => {
            crate::runtime::handle_goal_verdict(state, &tx, ok, reason).await;
        }

        // ── Voice events (routed to TUI via FrontendEvent) ───────
        // Voice events are forwarded to the TUI's event loop unchanged;
        // the engine itself has no special handling — the voice pipeline
        // runs entirely inside the jfc crate.
        EngineEvent::Voice(_) => {
            // Handled by the TUI event loop (jfc/src/runtime/event_loop/mod.rs).
        }

        // ── Compaction events ───────────────────────────────────
        EngineEvent::Compaction(ev) => {
            crate::runtime::event_loop::handlers::compaction::handle_compaction_event(
                state, &tx, ev,
            )
            .await;
        }

        // ── UI actions ──────────────────────────────────────────
        EngineEvent::Frontend(FrontendEvent::PlanModeEntered { reason }) => {
            crate::runtime::event_loop::handlers::ui_actions::handle_enter_plan_mode(state, reason);
        }
        EngineEvent::Control(ControlEvent::SubmitPrompt(text)) => {
            return Ok(Some(FrontendDirective::SubmitPrompt(text)));
        }
        EngineEvent::Control(ControlEvent::Notice { kind, text }) => {
            crate::runtime::event_loop::handlers::ui_actions::handle_toast(state, kind, text);
        }
        EngineEvent::Control(ControlEvent::LoadSession(session_id)) => {
            crate::runtime::ops::load_session(state, session_id).await;
        }
        EngineEvent::Control(ControlEvent::WorktreeCountLoaded(count)) => {
            state.worktree_count = count;
        }
        EngineEvent::Control(ControlEvent::ResolveApproval {
            tool_use_id,
            approved,
        }) => {
            crate::runtime::approvals::handle_remote_approval_response(
                state,
                &tx,
                tool_use_id,
                approved,
            );
        }
        EngineEvent::Frontend(FrontendEvent::PlanReview { plan }) => {
            crate::runtime::event_loop::handlers::ui_actions::handle_exit_plan_mode(state, plan);
        }
        EngineEvent::Frontend(FrontendEvent::GoalSet { condition }) => {
            crate::runtime::event_loop::handlers::ui_actions::handle_set_goal(state, condition);
        }
        EngineEvent::Frontend(FrontendEvent::ElicitationRequest {
            id,
            server_name,
            kind,
        }) => {
            state
                .pending_elicitations
                .push_back(jfc_core::mcp_elicitation::ElicitationSnapshot {
                    id,
                    server_name,
                    kind,
                });
        }
        // ── Task (subagent) events ──────────────────────────────
        EngineEvent::Task(TaskEvent::AgentChunk { task_id, text }) => {
            crate::runtime::event_loop::handlers::task::handle_agent_chunk(state, task_id, text);
        }
        EngineEvent::Task(TaskEvent::Started {
            task_id,
            description,
            model_used,
            max_input_tokens,
            is_detached,
            parent_task_id,
        }) => {
            crate::runtime::event_loop::handlers::task::handle_task_started(
                state,
                task_id,
                description,
                model_used,
                max_input_tokens,
                is_detached,
                parent_task_id,
            );
        }
        EngineEvent::Task(TaskEvent::Progress {
            task_id,
            last_tool,
            elapsed_ms,
            tool_use_count,
            input_tokens,
            cache_read_tokens,
            cache_write_tokens,
            output_tokens,
        }) => {
            crate::runtime::event_loop::handlers::task::handle_task_progress(
                state,
                task_id,
                last_tool,
                elapsed_ms,
                tool_use_count,
                input_tokens,
                cache_read_tokens,
                cache_write_tokens,
                output_tokens,
            );
        }
        EngineEvent::Task(TaskEvent::Completed {
            task_id,
            summary,
            elapsed_ms,
        }) => {
            crate::runtime::event_loop::handlers::task::handle_task_completed(
                state, &tx, task_id, summary, elapsed_ms,
            )
            .await;
        }
        EngineEvent::Task(TaskEvent::Failed { task_id, error }) => {
            crate::runtime::event_loop::handlers::task::handle_task_failed(
                state, &tx, task_id, error,
            )
            .await;
        }
        EngineEvent::WorkflowProgress(ev) => {
            crate::runtime::event_loop::handlers::workflow::handle_workflow_progress(state, ev);
        }
        EngineEvent::Control(ControlEvent::RunCommand(text)) => {
            return Ok(Some(FrontendDirective::RunCommand(text)));
        }
        EngineEvent::Control(ControlEvent::ResolveElicitation { id, response }) => {
            // Resolve a pending MCP elicitation — unblocks the waiting
            // JfcClientHandler::create_elicitation future.
            let action_label = match &response {
                crate::mcp_elicitation::ElicitationResponse::Accept { .. } => "accept",
                crate::mcp_elicitation::ElicitationResponse::Decline => "decline",
                crate::mcp_elicitation::ElicitationResponse::Cancel => "cancel",
            };
            if crate::mcp_elicitation::resolve(&id, response) {
                tracing::debug!(
                    target: "jfc::mcp::elicitation",
                    elicitation_id = %id,
                    action = %action_label,
                    "elicitation resolved via ControlEvent"
                );
                // Hooks fire asynchronously via the elicitation background task
                // (ElicitationEvent::Resolved). Nothing more to do here.
            } else {
                tracing::warn!(
                    target: "jfc::mcp::elicitation",
                    elicitation_id = %id,
                    "ResolveElicitation: no pending elicitation found for id"
                );
            }
        }
        EngineEvent::Control(ControlEvent::Interrupt) => {
            crate::runtime::ops::interrupt(state, tx);
        }
        EngineEvent::Control(ControlEvent::ResolvePlan { approved }) => {
            // Resolve the pending plan-gate approval (the ExitPlanMode tool
            // parked in `pending_approval`). Replaces the remote host's
            // synthetic 'y'/'n' keystrokes with an addressed resolution.
            let target = state
                .pending_approval
                .as_ref()
                .map(|p| p.tool.id.as_str().to_owned());
            match target {
                Some(tool_use_id) => {
                    crate::runtime::approvals::handle_remote_approval_response(
                        state,
                        tx,
                        tool_use_id,
                        approved,
                    );
                }
                None => {
                    tracing::warn!(
                        target: "jfc::remote",
                        approved,
                        "ResolvePlan with no pending approval; dropping"
                    );
                }
            }
        }
    }
    Ok(None)
}
