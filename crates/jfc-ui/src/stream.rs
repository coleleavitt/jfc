use std::{collections::HashMap, sync::Arc};

use futures::StreamExt;
use tokio::sync::{Mutex, mpsc};

use crate::app::{App, AppEvent};
use crate::context::ReadDedupCache;
use crate::provider::{
    ModelId, Provider, ProviderContent, ProviderMessage, ProviderRole, StopReason, StreamEvent,
    StreamOptions,
};
use crate::scheduler;
use crate::tools;
use crate::types::*;

const MAX_TOOL_RESULT_CHARS: usize = 30_000;

/// Truncate `s` to at most `MAX_TOOL_RESULT_CHARS` bytes by keeping the first
/// half and the last half, with an ellipsis marker in the middle. Slice
/// boundaries are snapped to the nearest UTF-8 char boundary so the function
/// can never panic on multi-byte content (emoji, accented chars, or binary
/// blobs that happen to land in the slice — exactly the panic in the
/// screenshot's stack trace at stream.rs:334:14, fired from inside
/// build_provider_messages_with_tool_results' FilterMap closure).
fn truncate_tool_result(s: &str) -> String {
    if s.len() <= MAX_TOOL_RESULT_CHARS {
        return s.to_owned();
    }
    let half = MAX_TOOL_RESULT_CHARS / 2;
    let head_end = floor_char_boundary(s, half);
    let tail_start = ceil_char_boundary(s, s.len().saturating_sub(half));
    let head = &s[..head_end];
    let tail = &s[tail_start..];
    let omitted = s.len() - head_end - (s.len() - tail_start);
    format!("{head}\n\n... [{omitted} bytes omitted] ...\n\n{tail}")
}

/// Round `i` down to the nearest UTF-8 char boundary in `s`. `str::is_char_boundary`
/// is true at byte 0 and `s.len()`, plus every codepoint boundary in between —
/// so the loop terminates in O(4) steps for any valid UTF-8.
fn floor_char_boundary(s: &str, mut i: usize) -> usize {
    if i >= s.len() {
        return s.len();
    }
    while i > 0 && !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}

/// Round `i` up to the nearest UTF-8 char boundary in `s`.
fn ceil_char_boundary(s: &str, mut i: usize) -> usize {
    if i >= s.len() {
        return s.len();
    }
    while i < s.len() && !s.is_char_boundary(i) {
        i += 1;
    }
    i
}

#[cfg(test)]
mod truncate_tests {
    use super::*;

    // Normal: short input passes through unchanged.
    #[test]
    fn truncate_short_passes_through_normal() {
        assert_eq!(truncate_tool_result("hello"), "hello");
    }

    // Robust: the original panic. A multi-byte char (4-byte emoji) sitting
    // exactly at the byte-`half` boundary used to crash with "byte index N
    // is not a char boundary". Fix snaps to the nearest valid boundary.
    #[test]
    fn truncate_does_not_panic_on_multibyte_char_at_split_boundary_robust() {
        // Build a string where MAX/2 lands inside a 🦀 (4 bytes).
        let prefix_bytes = MAX_TOOL_RESULT_CHARS / 2 - 2;
        let mut s = String::with_capacity(MAX_TOOL_RESULT_CHARS * 2);
        for _ in 0..prefix_bytes {
            s.push('a');
        }
        s.push('🦀'); // straddles byte-`half` (2 bytes before, 2 after)
        for _ in 0..(MAX_TOOL_RESULT_CHARS) {
            s.push('b');
        }
        // Must not panic.
        let _ = truncate_tool_result(&s);
    }

    // Robust: input with mixed ASCII + multibyte content still produces a
    // valid UTF-8 result (no half-codepoints in the output).
    #[test]
    fn truncate_output_is_valid_utf8_robust() {
        let s: String = std::iter::repeat("héllo 🌟 ").take(5000).collect();
        let out = truncate_tool_result(&s);
        // The .chars() iterator panics on invalid UTF-8 — driving it to
        // completion proves the output is well-formed.
        let _ = out.chars().count();
    }

    // Normal: head and tail are preserved across truncation.
    #[test]
    fn truncate_keeps_head_and_tail_normal() {
        let mid: String = "x".repeat(MAX_TOOL_RESULT_CHARS * 2);
        let s = format!("HEAD{mid}TAIL");
        let out = truncate_tool_result(&s);
        assert!(out.starts_with("HEAD"));
        assert!(out.ends_with("TAIL"));
        assert!(out.contains("bytes omitted"));
    }
}

#[tracing::instrument(
    target = "jfc::stream",
    skip_all,
    fields(
        provider = %provider.name(),
        model = %model,
        messages = messages.len(),
    ),
)]
pub async fn stream_response(
    provider: Arc<dyn Provider>,
    messages: Vec<ProviderMessage>,
    model: ModelId,
    tx: mpsc::UnboundedSender<AppEvent>,
) {
    let cwd = std::env::current_dir()
        .ok()
        .and_then(|p| p.to_str().map(str::to_owned))
        .unwrap_or_default();
    let mut system_prompt = format!(
        "You are jfc, a coding assistant running as a CLI in the user's terminal. \
         You have direct access to the user's filesystem and shell via tools \
         (Bash, Read, Write, Edit, Glob, Grep). When the user asks you to do \
         something — read a file, run a command, write code — USE the tools to \
         do it directly. Don't describe how the user could do it manually; you \
         are the one doing it. Working directory: {cwd}\n\n\
         ## Task tracking\n\
         For any request with 2 or more distinct steps, use TaskCreate to plan \
         before starting. Call TaskCreate once per step with a short description. \
         Mark each step complete with TaskDone immediately after finishing it — \
         never batch completions. Update a step's description mid-work with \
         TaskUpdate if scope changes. TaskList shows the user your current plan \
         in the sidebar. This is the primary way users track your progress, so \
         use it consistently on all non-trivial work."
    );

    // v126 CLAUDE.md hierarchy — managed → user → project → .claude/ → local
    // overrides. Each layer is appended with its origin labeled so the model
    // can tell which rule came from which file. We load on every stream call
    // so live edits to CLAUDE.md take effect on the next turn (matching CC).
    if let Ok(cwd_path) = std::env::current_dir() {
        let hierarchy = crate::context::ClaudeMdHierarchy::load(&cwd_path);
        if let Some(layered) = hierarchy.render() {
            system_prompt.push_str("\n\n");
            system_prompt.push_str(&layered);
        }

        // v126 skills listing — discovery surface for the model. Loaded on
        // every stream call so newly-added skills (or edited descriptions)
        // take effect on the next turn, matching cli.js:151-160's per-stream
        // re-read pattern.
        let skills = crate::agents::load_skills(&cwd_path);
        let block = crate::agents::render_skills_section(&skills);
        if !block.is_empty() {
            system_prompt.push_str(&block);
        }
    }
    let opts = StreamOptions::new(model)
        .system(system_prompt)
        .tools(tools::all_tool_defs())
        .thinking(8000);

    let mut stream = match provider.stream(messages, &opts).await {
        Ok(s) => s,
        Err(e) => {
            let _ = tx.send(AppEvent::StreamError(e.to_string()));
            return;
        }
    };

    let mut stop_reason = StopReason::EndTurn;
    let mut tool_accum: HashMap<usize, (String, String, String)> = HashMap::new();

    while let Some(event) = stream.next().await {
        let event = match event {
            Ok(e) => e,
            Err(e) => {
                let _ = tx.send(AppEvent::StreamError(e.to_string()));
                return;
            }
        };

        match event {
            StreamEvent::TextDelta { delta, .. } => {
                let _ = tx.send(AppEvent::StreamChunk {
                    text: Some(delta),
                    reasoning: None,
                });
            }
            StreamEvent::ThinkingDelta { delta, .. } => {
                let _ = tx.send(AppEvent::StreamChunk {
                    text: None,
                    reasoning: Some(delta),
                });
            }
            StreamEvent::ToolDelta { index, delta } => {
                tool_accum.entry(index).or_default().2.push_str(&delta);
            }
            StreamEvent::ToolDone {
                index,
                tool_name,
                tool_use_id,
                input_json,
            } => {
                // Prefer the input_json the provider assembled (Anthropic SSE
                // builds the full payload before firing ToolDone). When that's
                // empty, fall back to the accumulator we filled from
                // ToolDelta — required by OpenWebUI's OpenAI-compatible
                // streaming, which only ever ships fragments and doesn't
                // assemble the full string itself.
                let assembled = if input_json.is_empty() {
                    tool_accum
                        .get(&index)
                        .map(|(_, _, buf)| buf.clone())
                        .unwrap_or_default()
                } else {
                    input_json
                };
                tracing::debug!(
                    target: "jfc::stream",
                    index,
                    tool_name = %tool_name,
                    tool_use_id = %tool_use_id,
                    input_len = assembled.len(),
                    "tool_done"
                );
                let input_val: serde_json::Value =
                    serde_json::from_str(&assembled).unwrap_or(serde_json::Value::Null);
                let tool = ToolCall {
                    id: tool_use_id,
                    kind: ToolKind::from_name(&tool_name),
                    status: ToolStatus::Pending,
                    input: ToolInput::from_value(&tool_name, input_val),
                    output: ToolOutput::Empty,
                    is_collapsed: false,
                    expanded: false,
                };
                tool_accum.remove(&index);
                let _ = tx.send(AppEvent::StreamTool(tool));
            }
            StreamEvent::Done { stop_reason: r } => {
                // Never downgrade from ToolUse → EndTurn.  The OpenAI SSE
                // protocol sends `[DONE]` after the finish_reason chunk.
                // `push_chunk_events_stateful` already emitted Done{ToolUse}
                // from the finish_reason chunk; the subsequent [DONE] line
                // emits Done{EndTurn}.  If we blindly overwrite we lose the
                // ToolUse signal and pending_tool_calls are silently cleared
                // instead of dispatched.
                if stop_reason != StopReason::ToolUse {
                    stop_reason = r;
                }
            }
            StreamEvent::TextDone { .. } | StreamEvent::ThinkingDone { .. } => {}
            StreamEvent::Usage {
                input_tokens,
                output_tokens,
                cache_read_tokens,
                cache_write_tokens,
            } => {
                let _ = tx.send(AppEvent::StreamUsage {
                    input_tokens,
                    output_tokens,
                    cache_read_tokens,
                    cache_write_tokens,
                });
            }
            StreamEvent::Error { message } => {
                let _ = tx.send(AppEvent::StreamError(message));
                return;
            }
        }
    }

    let _ = tx.send(AppEvent::StreamDone(stop_reason));
}

#[tracing::instrument(target = "jfc::stream", skip(tx, dedup, task_store, provider, model), fields(n = tool_calls.len()))]
pub fn dispatch_tools_batched(
    tool_calls: Vec<ToolCall>,
    tx: &mpsc::UnboundedSender<AppEvent>,
    dedup: Arc<Mutex<ReadDedupCache>>,
    task_store: Option<Arc<crate::tasks::TaskStore>>,
    provider: Arc<dyn crate::provider::Provider>,
    model: crate::provider::ModelId,
) {
    use crate::types::ToolInput;
    use std::sync::atomic::{AtomicUsize, Ordering};

    let cwd = std::env::current_dir().unwrap_or_default();

    let mut regular_calls: Vec<ToolCall> = Vec::new();
    let mut task_calls: Vec<ToolCall> = Vec::new();
    for tc in tool_calls {
        match &tc.input {
            ToolInput::Task(_) => task_calls.push(tc),
            _ => regular_calls.push(tc),
        }
    }

    let task_count = task_calls.len();
    let pending = Arc::new(AtomicUsize::new(
        task_count + usize::from(!regular_calls.is_empty()),
    ));
    let tx_done = tx.clone();
    let send_all_complete = move || {
        if pending.fetch_sub(1, Ordering::AcqRel) == 1 {
            let _ = tx_done.send(AppEvent::AllToolsComplete);
        }
    };

    // Pre-load agent defs once per dispatch so each spawned task can
    // resolve its `subagent_type` without redoing the directory walk.
    let agents = crate::agents::load_agents(&cwd);

    for tc in task_calls {
        let task_input = match tc.input.clone() {
            ToolInput::Task(ti) => ti,
            _ => unreachable!(),
        };
        let tx_task = tx.clone();
        let provider_task = provider.clone();
        let model_task = model.clone();
        let task_id = tc.id.clone();
        let description = task_input.description.clone();
        let done = send_all_complete.clone();

        // Resolve `subagent_type` to a concrete `AgentDef`. When unset
        // or unknown, falls back to `None` and `execute_task` runs with
        // no system prompt (mirrors the prior, agent-less behavior).
        let agent_def = task_input
            .subagent_type
            .as_deref()
            .and_then(|t| agents.iter().find(|a| a.name == t))
            .cloned();

        tokio::spawn(async move {
            let _ = tx_task.send(AppEvent::TaskStarted {
                task_id: task_id.clone(),
                description,
            });

            let started = std::time::Instant::now();
            // Forward the subagent's streaming text into the main event
            // loop (`AppEvent::AgentChunk`) so the task view fills live
            // rather than showing "No messages yet" until the agent
            // finishes. tx + task_id are passed through; the producer
            // (`execute_task`) emits one event per `TextDelta`.
            let result = crate::tools::execute_task(
                &task_input,
                provider_task.as_ref(),
                model_task,
                Some(&tx_task),
                Some(&task_id),
                agent_def.as_ref(),
            )
            .await;
            let elapsed_ms = started.elapsed().as_millis() as u64;

            if result.is_error() {
                let _ = tx_task.send(AppEvent::TaskFailed {
                    task_id: task_id.clone(),
                    error: result.output.clone(),
                });
            } else {
                let _ = tx_task.send(AppEvent::TaskCompleted {
                    task_id: task_id.clone(),
                    summary: result.output.clone(),
                    elapsed_ms,
                });
            }

            let _ = tx_task.send(AppEvent::ToolResult {
                tool_id: task_id,
                result,
            });

            done();
        });
    }

    if !regular_calls.is_empty() {
        let batches = scheduler::schedule_tools(regular_calls);
        let tx_clone = tx.clone();
        let done = send_all_complete.clone();
        tokio::spawn(async move {
            scheduler::execute_batches(batches, &tx_clone, cwd, dedup, task_store).await;
            done();
        });
    }
}

pub fn should_continue_loop(messages: &[ChatMessage]) -> bool {
    let last = match messages.iter().rev().find(|m| m.role == Role::Assistant) {
        Some(m) => m,
        None => return false,
    };
    let has_tools = last.parts.iter().any(|p| matches!(p, MessagePart::Tool(_)));
    if !has_tools {
        return false;
    }
    last.parts.iter().all(|p| match p {
        MessagePart::Tool(tc) => {
            tc.status == ToolStatus::Complete || tc.status == ToolStatus::Failed
        }
        _ => true,
    })
}

pub async fn continue_agentic_loop(app: &mut App, tx: &mpsc::UnboundedSender<AppEvent>) {
    let assistant_idx = app.messages.len();
    app.messages.push(ChatMessage::assistant(String::new()));
    app.streaming_text.clear();
    app.streaming_reasoning.clear();
    app.streaming_assistant_idx = Some(assistant_idx);
    app.is_streaming = true;
    // The sub-stream clock restarts (Anthropic restarts `output_tokens`
    // per request) but the *user-turn* clock keeps running — set in
    // `handle_submit_text` and only cleared when the loop concludes.
    let now = std::time::Instant::now();
    app.streaming_started_at = Some(now);
    app.streaming_last_token_at = Some(now);
    app.last_usage_output = 0;
    app.usage_apply_baseline = (0, 0, 0, 0);

    let provider = app.provider.clone();
    let messages = build_provider_messages_with_tool_results(&app.messages[..assistant_idx]);
    let model = app.model.clone();
    let tx = tx.clone();

    tokio::spawn(async move {
        stream_response(provider, messages, model, tx).await;
    });
}

pub fn build_provider_messages(msgs: &[ChatMessage]) -> Vec<ProviderMessage> {
    let out: Vec<ProviderMessage> = msgs
        .iter()
        .filter_map(|m| {
            let role = match m.role {
                Role::User => ProviderRole::User,
                Role::Assistant => ProviderRole::Assistant,
            };
            let text: String = m
                .parts
                .iter()
                .filter_map(|p| match p {
                    MessagePart::Text(t) if !t.is_empty() => Some(t.as_str()),
                    _ => None,
                })
                .collect::<Vec<_>>()
                .join("\n");
            if text.is_empty() {
                return None;
            }
            Some(ProviderMessage {
                role,
                content: vec![ProviderContent::Text(text)],
            })
        })
        .collect();
    strip_trailing_empty_assistant(out)
}

/// Drop a trailing assistant message that contains no real content before
/// sending to the provider.
///
/// `continue_agentic_loop` (this file, ~line 400) pushes a placeholder
/// `ChatMessage::assistant(String::new())` onto `app.messages` before the
/// stream task starts, to reserve the slot for the streamed response. That
/// empty assistant is sliced off (`&app.messages[..assistant_idx]`) before
/// being passed to `build_provider_messages_with_tool_results`, but other
/// callers — and any future code path that forgets the slice — can leak an
/// empty assistant tail into the provider request.
///
/// Bedrock-via-LiteLLM (OWUI deployment) hard-rejects this with:
///     `BedrockException — "This model does not support assistant message
///     prefill. The conversation must end with a user message."`
/// The native Anthropic API silently treats a trailing assistant turn as
/// prefill, which is also wrong for the agentic continuation use case (we
/// want a fresh assistant turn, not a continuation of an empty one). Thus
/// the strip is provider-agnostic: a trailing empty assistant is wrong
/// everywhere and has no legitimate semantic meaning here.
///
/// Empty means: every `content` block is `Text(s)` with `s.trim().is_empty()`,
/// or the `content` vec is empty. Any non-blank text or any `ToolUse` /
/// `ToolResult` block keeps the message — those are real partial turns we
/// must not lose. The strip is intentionally non-recursive: two empty
/// assistants in a row only loses the very last one, because a deeper
/// build-up of empties points to a separate bug we want to surface.
fn strip_trailing_empty_assistant(mut msgs: Vec<ProviderMessage>) -> Vec<ProviderMessage> {
    let last_is_empty_assistant = msgs
        .last()
        .map(|m| {
            m.role == ProviderRole::Assistant
                && m.content.iter().all(|c| match c {
                    ProviderContent::Text(s) => s.trim().is_empty(),
                    _ => false,
                })
        })
        .unwrap_or(false);
    if last_is_empty_assistant {
        tracing::info!(
            target: "jfc::stream",
            "stripped trailing empty assistant before send"
        );
        msgs.pop();
    }
    msgs
}

fn build_provider_messages_with_tool_results(msgs: &[ChatMessage]) -> Vec<ProviderMessage> {
    let mut out = Vec::new();
    for m in msgs {
        let role = match m.role {
            Role::User => ProviderRole::User,
            Role::Assistant => ProviderRole::Assistant,
        };
        let text: String = m
            .parts
            .iter()
            .filter_map(|p| match p {
                MessagePart::Text(t) if !t.is_empty() => Some(t.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n");

        let tool_uses: Vec<ProviderContent> = m
            .parts
            .iter()
            .filter_map(|p| match p {
                MessagePart::Tool(tc) => Some(ProviderContent::ToolUse {
                    id: tc.id.clone(),
                    name: tc.kind.api_name().to_owned(),
                    input: tc.input.to_value(),
                }),
                _ => None,
            })
            .collect();

        // Anthropic's API enforces: every `tool_use` block in an assistant
        // turn MUST be followed by a matching `tool_result` block in the
        // next user message — including ones that were never approved or
        // executed. The 400 error from the log:
        //   "messages.4: tool_use ids were found without tool_result blocks
        //    immediately after: toolu_012FTQ..., toolu_01DKKy..., …"
        // happens when the user types a new prompt while tools are still
        // pending approval, then we send a request with mismatched counts.
        //
        // Fix: emit a tool_result for EVERY tool_use in the assistant turn.
        // Pending/Running tools (never finished) get a synthetic "abandoned"
        // result with is_error=true so the model knows the tool didn't run.
        let tool_results: Vec<ProviderContent> = m
            .parts
            .iter()
            .filter_map(|p| match p {
                MessagePart::Tool(tc) => {
                    let (result_text, is_error) = match tc.status {
                        ToolStatus::Complete | ToolStatus::Failed => {
                            let text = match &tc.output {
                                ToolOutput::Text(s) => s.clone(),
                                ToolOutput::LargeText(lt) => lt.content.clone(),
                                ToolOutput::Command {
                                    stdout,
                                    stderr,
                                    exit_code,
                                } => format!(
                                    "exit: {}\nstdout: {}\nstderr: {}",
                                    exit_code.unwrap_or(-1),
                                    stdout,
                                    stderr
                                ),
                                ToolOutput::FileContent { content, .. } => content.clone(),
                                ToolOutput::FileList(files) => files.join("\n"),
                                ToolOutput::Diff(d) => {
                                    format!("Applied diff to {}", d.file_path)
                                }
                                ToolOutput::Empty => String::new(),
                            };
                            (text, tc.status == ToolStatus::Failed)
                        }
                        ToolStatus::Pending | ToolStatus::Running => (
                            "Tool was abandoned: the user moved on before \
                             approving or executing it. No output was produced."
                                .to_owned(),
                            true,
                        ),
                    };
                    Some(ProviderContent::ToolResult {
                        tool_use_id: tc.id.clone(),
                        content: truncate_tool_result(&result_text),
                        is_error,
                    })
                }
                _ => None,
            })
            .collect();

        let mut assistant_content = Vec::new();
        if !text.is_empty() {
            assistant_content.push(ProviderContent::Text(text.clone()));
        }
        assistant_content.extend(tool_uses);

        if !assistant_content.is_empty() {
            out.push(ProviderMessage {
                role: role.clone(),
                content: assistant_content,
            });
        } else if !text.is_empty() {
            out.push(ProviderMessage {
                role: role.clone(),
                content: vec![ProviderContent::Text(text)],
            });
        }

        if !tool_results.is_empty() {
            out.push(ProviderMessage {
                role: ProviderRole::User,
                content: tool_results,
            });
        }
    }
    strip_trailing_empty_assistant(out)
}

#[cfg(test)]
mod strip_trailing_empty_assistant_tests {
    use super::*;

    fn user_text(s: &str) -> ProviderMessage {
        ProviderMessage {
            role: ProviderRole::User,
            content: vec![ProviderContent::Text(s.to_owned())],
        }
    }

    fn assistant_text(s: &str) -> ProviderMessage {
        ProviderMessage {
            role: ProviderRole::Assistant,
            content: vec![ProviderContent::Text(s.to_owned())],
        }
    }

    // Normal: the exact bug from the screenshot — `continue_agentic_loop`
    // pushes an empty assistant placeholder, the builder echoes it, Bedrock
    // explodes. After the strip, the conversation ends on the user turn.
    #[test]
    fn strip_drops_trailing_empty_assistant_normal() {
        let input = vec![user_text("hi"), assistant_text("")];
        let out = strip_trailing_empty_assistant(input);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].role, ProviderRole::User);
    }

    // Normal: whitespace-only text counts as empty — a streamed turn that
    // only emitted a newline before being interrupted is still no content.
    #[test]
    fn strip_drops_trailing_whitespace_only_assistant_normal() {
        let input = vec![user_text("hi"), assistant_text("   \n")];
        let out = strip_trailing_empty_assistant(input);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].role, ProviderRole::User);
    }

    // Robust: real assistant text must not be dropped — that would silently
    // discard model output and create a worse bug than the one we're fixing.
    #[test]
    fn strip_keeps_assistant_with_real_content_robust() {
        let input = vec![user_text("hi"), assistant_text("hello")];
        let out = strip_trailing_empty_assistant(input.clone());
        assert_eq!(out.len(), 2);
        assert_eq!(out[1].role, ProviderRole::Assistant);
    }

    // Robust: an assistant turn whose only content is a tool_use is a real
    // mid-flight turn (model called a tool, we're about to send the result
    // back). Dropping it would orphan the tool_use_id on the next request.
    #[test]
    fn strip_keeps_assistant_with_only_toolcall_robust() {
        let assistant_with_tool = ProviderMessage {
            role: ProviderRole::Assistant,
            content: vec![ProviderContent::ToolUse {
                id: "toolu_1".to_owned(),
                name: "Bash".to_owned(),
                input: serde_json::json!({"command": "ls"}),
            }],
        };
        let input = vec![user_text("hi"), assistant_with_tool];
        let out = strip_trailing_empty_assistant(input);
        assert_eq!(out.len(), 2);
        assert_eq!(out[1].role, ProviderRole::Assistant);
    }

    // Normal: if the conversation already ends with a user message (the
    // common tool_result-injection case), the strip is a no-op.
    #[test]
    fn strip_no_op_on_user_last_normal() {
        let input = vec![assistant_text("hi"), user_text("ok")];
        let out = strip_trailing_empty_assistant(input);
        assert_eq!(out.len(), 2);
        assert_eq!(out[1].role, ProviderRole::User);
    }

    // Robust: empty input must round-trip — no panic on `.last()` of an
    // empty vec, no spurious `pop()`.
    #[test]
    fn strip_no_op_on_empty_input_robust() {
        let out = strip_trailing_empty_assistant(Vec::<ProviderMessage>::new());
        assert!(out.is_empty());
    }

    // Robust: non-recursive by design. Two empty assistants in a row means
    // something else is wrong upstream; we drop only the last one so the
    // remaining empty assistant surfaces the bug instead of hiding it.
    #[test]
    fn strip_only_drops_one_trailing_robust() {
        let input = vec![
            user_text("hi"),
            assistant_text(""),
            assistant_text(""),
        ];
        let out = strip_trailing_empty_assistant(input);
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].role, ProviderRole::User);
        assert_eq!(out[1].role, ProviderRole::Assistant);
    }
}
