//! Deserialize on-disk `Serialized*` types back into runtime types.

use super::serialization::*;
use crate::types::{
    ChatMessage, DiffHunk, DiffLine, DiffLineKind, DiffView, LargeText, MessagePart,
    ReplacementMode, Role, TaskInput, TaskLifecycle, TaskStatusPart, ToolCall, ToolInput, ToolKind,
    ToolOutput, ToolStatus,
};

pub fn deserialize_message(msg: SerializedMessage) -> ChatMessage {
    let role = if msg.role == "user" {
        Role::User
    } else {
        Role::Assistant
    };
    let parts: Vec<MessagePart> = msg.parts.into_iter().map(deserialize_part).collect();
    let elapsed = normalize_elapsed(role, &parts, msg.elapsed);
    ChatMessage {
        role,
        parts,
        agent_name: msg.agent_name,
        model_name: msg.model_name,
        cost_tier: msg.cost_tier,
        elapsed,
        usage: msg.usage,
        // Queued is a runtime-only marker — resumed sessions never have
        // unsent queued prompts because drain_queued_prompts runs as
        // part of the turn lifecycle before save_session ever fires.
        queued: false,
        // Attachments (images) are not persisted in session files — they
        // would bloat JSON to hundreds of MB. Default to empty on load.
        attachments: Vec::new(),
        // 0 means "unknown" for old sessions that predate this field.
        created_at: msg.created_at,
    }
}

fn normalize_elapsed(role: Role, parts: &[MessagePart], elapsed: Option<String>) -> Option<String> {
    let elapsed = elapsed?;
    if role == Role::Assistant && assistant_is_error(parts) {
        return None;
    }
    if elapsed.starts_with("took ") {
        return Some(elapsed);
    }
    elapsed
        .rsplit_once(" for ")
        .map_or(Some(elapsed.clone()), |(_, duration)| {
            Some(format!("took {}", duration.trim()))
        })
}

fn assistant_is_error(parts: &[MessagePart]) -> bool {
    parts.iter().any(|part| {
        matches!(part, MessagePart::Text(text) if text.trim_start().starts_with("**Error:**"))
    })
}

pub fn deserialize_part(part: SerializedPart) -> MessagePart {
    match part {
        SerializedPart::Text { content } => MessagePart::Text(content),
        SerializedPart::Reasoning { content } => MessagePart::Reasoning(content),
        SerializedPart::ReasoningSignature { signature } => {
            MessagePart::ReasoningSignature(signature)
        }
        SerializedPart::Tool { tool } => {
            let SerializedToolPart {
                id,
                kind,
                status,
                is_collapsed,
                input,
                output,
                thought_signature,
            } = *tool;
            let tool_kind = ToolKind::from_name(&kind);
            MessagePart::tool(ToolCall {
                id: crate::ids::ToolId::from(id),
                kind: tool_kind,
                status: deserialize_tool_status(&status),
                // Tolerate missing input/output on legacy session files.
                // The unknown-input fallback (a no-op Bash entry) lets the
                // resumed transcript render the tool row with whatever
                // chrome we have (id, kind, status) without panicking on a
                // missing field that older writers never produced.
                input: match input {
                    Some(i) => deserialize_tool_input_for_kind(&kind, i),
                    None => ToolInput::Bash {
                        command: String::new(),
                        timeout: None,
                        workdir: None,
                        run_in_background: None,
                        suppress_output: None,
                    },
                },
                output: match output {
                    Some(o) => deserialize_tool_output(o),
                    None => ToolOutput::Empty,
                },
                // Reconstruct the tri-state from the legacy on-disk
                // `is_collapsed` bool. Expanded + pinned were never
                // persisted (storing UI chrome state in the on-disk
                // format would round-trip stale state), so loaded sessions
                // always come back as either Collapsed (huge teaser
                // preserved) or Default. The user can re-expand or re-pin
                // with `o` / Ctrl+O / double-click.
                display: if is_collapsed {
                    crate::types::ToolDisplayState::Collapsed
                } else {
                    crate::types::ToolDisplayState::DEFAULT
                },
                // elapsed_ms could in principle round-trip, but it's
                // cosmetic — leave None on resume so we don't lock in a
                // stale duration. started_at is meaningless after a
                // reload (would always say "elapsed since session-load").
                elapsed_ms: None,
                started_at: None,
                thought_signature,
            })
        }
        SerializedPart::TaskStatus {
            task_id,
            description,
            status,
            summary,
            error,
            elapsed_ms,
        } => MessagePart::TaskStatus(TaskStatusPart {
            task_id: crate::ids::TaskId::from(task_id),
            description,
            status: deserialize_task_lifecycle(&status),
            summary,
            error,
            elapsed_ms,
            model: None,
        }),
        SerializedPart::CompactBoundary { pre_tokens } => {
            MessagePart::CompactBoundary { pre_tokens }
        }
        SerializedPart::Advisor { content } => MessagePart::Advisor(content),
        SerializedPart::RedactedThinking { data } => MessagePart::RedactedThinking(data),
    }
}

pub fn deserialize_tool_status(status: &str) -> ToolStatus {
    // Backward-compat: legacy sessions wrote "complete" (Tool's
    // pre-unification spelling). Also accept "completed" / "idle" /
    // "cancelled" so a future serializer that emits the canonical
    // ExecutionStatus names stays readable. Falls back to Completed
    // (rather than Pending) on unknown — a tool that landed on disk
    // without a recognized state is almost certainly done by the
    // time a session reload reads it.
    match status {
        "pending" => ToolStatus::Pending,
        "running" => ToolStatus::Running,
        "idle" => ToolStatus::Idle,
        "complete" | "Complete" | "completed" | "Completed" => ToolStatus::Completed,
        "failed" | "Failed" => ToolStatus::Failed,
        "cancelled" | "Cancelled" => ToolStatus::Cancelled,
        _ => ToolStatus::Completed,
    }
}

pub fn deserialize_task_lifecycle(status: &str) -> TaskLifecycle {
    match status {
        "pending" => TaskLifecycle::Pending,
        "running" => TaskLifecycle::Running,
        "completed" => TaskLifecycle::Completed,
        "failed" => TaskLifecycle::Failed,
        "cancelled" => TaskLifecycle::Cancelled,
        _ => TaskLifecycle::Pending,
    }
}

pub fn deserialize_tool_input_for_kind(kind: &str, input: SerializedToolInput) -> ToolInput {
    match input {
        SerializedToolInput::Generic { summary } => {
            deserialize_generic_tool_input(kind, &summary).unwrap_or(ToolInput::Generic { summary })
        }
        other => deserialize_tool_input(other),
    }
}

pub fn deserialize_generic_tool_input(kind: &str, summary: &str) -> Option<ToolInput> {
    if let Ok(value) = serde_json::from_str::<serde_json::Value>(summary)
        && value.is_object()
        && let Ok(input) = ToolInput::from_value(kind, value)
    {
        return Some(input);
    }

    match ToolKind::from_name(kind) {
        ToolKind::WebSearch => {
            summary
                .strip_prefix("WebSearch: ")
                .map(|query| ToolInput::WebSearch {
                    query: query.to_owned(),
                    max_results: None,
                })
        }
        ToolKind::WebFetch => summary
            .strip_prefix("WebFetch: ")
            .map(|url| ToolInput::WebFetch {
                url: url.to_owned(),
                prompt: None,
            }),
        ToolKind::EnterPlanMode => {
            summary
                .strip_prefix("EnterPlanMode: ")
                .map(|reason| ToolInput::EnterPlanMode {
                    reason: reason.to_owned(),
                })
        }
        ToolKind::ExitPlanMode => {
            summary
                .strip_prefix("ExitPlanMode: ")
                .map(|plan| ToolInput::ExitPlanMode {
                    plan: plan.to_owned(),
                })
        }
        ToolKind::MultiEdit => parse_legacy_multi_edit(summary),
        ToolKind::MarketStatus => parse_legacy_market_status(summary),
        ToolKind::RunBounty => {
            summary
                .strip_prefix("RunBounty: ")
                .map(|bounty_id| ToolInput::RunBounty {
                    bounty_id: bounty_id.to_owned(),
                    max_solvers: None,
                })
        }
        ToolKind::TeamCreate => parse_legacy_team_create(summary),
        ToolKind::TeamDelete if summary == "TeamDelete" => Some(ToolInput::TeamDelete),
        ToolKind::TeamMemberMode => parse_legacy_team_member_mode(summary),
        ToolKind::PushNotification => parse_legacy_push_notification(summary),
        ToolKind::RemoteTrigger => {
            summary
                .strip_prefix("RemoteTrigger: ")
                .map(|trigger_id| ToolInput::RemoteTrigger {
                    trigger_id: trigger_id.to_owned(),
                    payload: None,
                })
        }
        ToolKind::AskUserQuestion => parse_legacy_ask_user_question(summary),
        ToolKind::EnterWorktree => parse_legacy_enter_worktree(summary),
        ToolKind::NotebookRead => {
            summary
                .strip_prefix("NotebookRead: ")
                .map(|path| ToolInput::NotebookRead {
                    path: path.to_owned(),
                })
        }
        ToolKind::ScratchpadRead => {
            summary
                .strip_prefix("ScratchpadRead: ")
                .map(|key| ToolInput::ScratchpadRead {
                    key: key.to_owned(),
                })
        }
        _ => None,
    }
}

pub fn strip_any_prefix<'a>(summary: &'a str, prefixes: &[&str]) -> Option<&'a str> {
    prefixes
        .iter()
        .find_map(|prefix| summary.strip_prefix(prefix))
}

pub fn parse_legacy_multi_edit(summary: &str) -> Option<ToolInput> {
    let rest = summary.strip_prefix("MultiEdit: ")?;
    let file_path = rest.split_once(" (").map_or(rest, |(path, _)| path);
    Some(ToolInput::MultiEdit {
        file_path: file_path.to_owned(),
        edits: serde_json::json!([]),
    })
}

pub fn parse_legacy_market_status(summary: &str) -> Option<ToolInput> {
    if summary == "MarketStatus" {
        return Some(ToolInput::MarketStatus { bounty_id: None });
    }
    summary
        .strip_prefix("MarketStatus: ")
        .map(|id| ToolInput::MarketStatus {
            bounty_id: Some(id.to_owned()),
        })
}

pub fn parse_legacy_team_create(summary: &str) -> Option<ToolInput> {
    let rest = summary.strip_prefix("TeamCreate: ")?;
    let (team_name, description) = rest
        .split_once(" — ")
        .map_or((rest, None), |(name, desc)| (name, Some(desc.to_owned())));
    Some(ToolInput::TeamCreate {
        team_name: team_name.to_owned(),
        description,
    })
}

pub fn parse_legacy_team_member_mode(summary: &str) -> Option<ToolInput> {
    let rest = summary.strip_prefix("TeamMemberMode ")?;
    let (member_name, mode) = rest.split_once(": ")?;
    Some(ToolInput::TeamMemberMode {
        member_name: member_name.to_owned(),
        mode: mode.to_owned(),
    })
}

pub fn parse_legacy_push_notification(summary: &str) -> Option<ToolInput> {
    let rest = summary.strip_prefix("PushNotification: ")?;
    let (title, message) = rest
        .split_once(": ")
        .map_or((None, rest), |(title, message)| {
            (Some(title.to_owned()), message)
        });
    Some(ToolInput::PushNotification {
        message: message.to_owned(),
        title,
    })
}

pub fn parse_legacy_enter_worktree(summary: &str) -> Option<ToolInput> {
    let rest = summary.strip_prefix("EnterWorktree: ")?;
    let (name, branch) = if let Some((name, branch)) = rest
        .strip_suffix(')')
        .and_then(|trimmed| trimmed.split_once(" ("))
    {
        (name, Some(branch.to_owned()))
    } else {
        (rest, None)
    };
    Some(ToolInput::EnterWorktree {
        name: name.to_owned(),
        branch,
    })
}

pub fn parse_legacy_ask_user_question(summary: &str) -> Option<ToolInput> {
    let question = strip_any_prefix(summary, &["AskUserQuestion: ", "ask: "])?;
    Some(ToolInput::AskUserQuestion {
        questions: serde_json::json!([{
            "question": question,
            "options": [],
            "multiSelect": false,
        }]),
    })
}

pub fn deserialize_tool_input(input: SerializedToolInput) -> ToolInput {
    match input {
        SerializedToolInput::Edit {
            file_path,
            old_string,
            new_string,
            replace_all,
        } => ToolInput::Edit {
            file_path,
            old_string,
            new_string,
            replacement: ReplacementMode::from_replace_all(replace_all),
        },
        SerializedToolInput::Write { file_path, content } => {
            ToolInput::Write { file_path, content }
        }
        SerializedToolInput::Read {
            file_path,
            offset,
            limit,
        } => ToolInput::Read {
            file_path,
            offset,
            limit,
        },
        SerializedToolInput::Bash {
            command,
            timeout,
            workdir,
            run_in_background,
            suppress_output,
        } => ToolInput::Bash {
            command,
            timeout,
            workdir,
            run_in_background,
            suppress_output,
        },
        SerializedToolInput::BashOutput {
            task_id,
            offset,
            limit,
            block,
            timeout,
            wait_up_to,
        } => ToolInput::BashOutput {
            task_id,
            offset,
            limit,
            block,
            timeout,
            wait_up_to,
        },
        SerializedToolInput::Glob { pattern, path } => ToolInput::Glob { pattern, path },
        SerializedToolInput::Grep {
            pattern,
            path,
            glob,
            output_mode,
        } => ToolInput::Grep {
            pattern,
            path,
            glob,
            output_mode,
        },
        SerializedToolInput::Search { query, path } => ToolInput::Search { query, path },
        SerializedToolInput::ApplyPatch { patch } => ToolInput::ApplyPatch { patch },
        SerializedToolInput::Task {
            description,
            prompt,
            subagent_type,
            category,
            run_in_background,
            model,
            launcher,
            effort,
            name,
            team_name,
            mode,
            isolation,
            parent_task_id,
            schema,
            allowed_tools,
            disallowed_tools,
            cwd,
        } => ToolInput::Task(TaskInput {
            description,
            prompt,
            subagent_type,
            category,
            run_in_background,
            model,
            launcher,
            effort,
            name,
            team_name,
            mode,
            isolation,
            parent_task_id,
            schema,
            allowed_tools,
            disallowed_tools,
            cwd,
        }),
        SerializedToolInput::TaskCreate {
            subject,
            description,
            active_form,
            blocked_by,
            acceptance_criteria,
            verification_command,
            risk,
            parent_id,
            kind,
        } => ToolInput::TaskCreate {
            subject,
            description,
            active_form,
            blocked_by,
            acceptance_criteria,
            verification_command,
            risk,
            parent_id,
            kind,
            tags: Vec::new(),
            priority: None,
            effort: None,
            model: None,
        },
        SerializedToolInput::TaskUpdate {
            task_id,
            status,
            subject,
            description,
            owner,
            acceptance_criteria,
            verification_command,
            risk,
            parent_id,
            kind,
        } => ToolInput::TaskUpdate {
            task_id,
            status,
            subject,
            description,
            owner,
            acceptance_criteria,
            verification_command,
            risk,
            parent_id,
            kind,
            blocked_by: Vec::new(),
            tags: Vec::new(),
            priority: None,
            effort: None,
            model: None,
        },
        SerializedToolInput::TaskList {
            status_filter,
            owner_filter,
            include_history,
            history_query,
        } => ToolInput::TaskList {
            status_filter,
            owner_filter,
            include_history,
            history_query,
        },
        SerializedToolInput::TaskDone { task_id } => ToolInput::TaskDone { task_id },
        SerializedToolInput::TaskStop { task_id } => ToolInput::TaskStop { task_id },
        SerializedToolInput::TaskGet { task_id } => ToolInput::TaskGet { task_id },
        SerializedToolInput::TaskValidate => ToolInput::TaskValidate,
        SerializedToolInput::Skill { name, args } => ToolInput::Skill { name, args },
        SerializedToolInput::ToolSearch { query, limit } => ToolInput::ToolSearch { query, limit },
        SerializedToolInput::ToolSuggest { intent, limit } => {
            ToolInput::ToolSuggest { intent, limit }
        }
        SerializedToolInput::MemoryCreate {
            level,
            memory_type,
            scope,
            body,
        } => ToolInput::MemoryCreate {
            level,
            memory_type,
            scope,
            body,
        },
        SerializedToolInput::MemoryDelete { path } => ToolInput::MemoryDelete { path },
        SerializedToolInput::TeamCreate {
            team_name,
            description,
        } => ToolInput::TeamCreate {
            team_name,
            description,
        },
        SerializedToolInput::TeamDelete => ToolInput::TeamDelete,
        SerializedToolInput::SendMessage {
            to,
            message,
            summary,
        } => ToolInput::SendMessage {
            to,
            message,
            summary,
        },
        SerializedToolInput::TeamMemberMode { member_name, mode } => {
            ToolInput::TeamMemberMode { member_name, mode }
        }
        // Back-compat read-only: the in-tree graph tools were unwired (jfc-graph
        // removed; code intelligence now flows through the external codegraph MCP
        // server). Sessions saved while those native tools existed still carry
        // these serialized forms, so we rebuild them into Generic records rather
        // than failing the load. Nothing produces these variants anymore.
        SerializedToolInput::CodeIndex { .. } => ToolInput::Generic {
            summary: "code_index".into(),
        },
        SerializedToolInput::GraphQuery { query, .. } => ToolInput::Generic {
            summary: format!("graph_query: {query}"),
        },
        SerializedToolInput::PostBounty {
            description,
            budget,
            acceptance_criteria,
            max_solvers,
            auto_dispatch,
            parent_task_id,
        } => ToolInput::PostBounty {
            description,
            budget,
            acceptance_criteria,
            max_solvers,
            auto_dispatch,
            parent_task_id,
        },
        SerializedToolInput::MarketStatus { bounty_id } => ToolInput::MarketStatus { bounty_id },
        SerializedToolInput::RunBounty {
            bounty_id,
            max_solvers,
        } => ToolInput::RunBounty {
            bounty_id,
            max_solvers,
        },
        SerializedToolInput::RunCoverage { lcov_path, .. } => ToolInput::Generic {
            summary: format!("coverage({})", lcov_path.as_deref().unwrap_or("auto")),
        },
        SerializedToolInput::SymbolEdit { handle, .. } => ToolInput::Generic {
            summary: format!("symbol_edit: {handle}"),
        },
        SerializedToolInput::ExitPlanMode { plan } => ToolInput::ExitPlanMode { plan },
        SerializedToolInput::MultiEdit { file_path, edits } => {
            ToolInput::MultiEdit { file_path, edits }
        }
        SerializedToolInput::AskUserQuestion { questions } => {
            ToolInput::AskUserQuestion { questions }
        }
        SerializedToolInput::WebFetch { url, prompt } => ToolInput::WebFetch { url, prompt },
        SerializedToolInput::WebSearch { query, max_results } => {
            ToolInput::WebSearch { query, max_results }
        }
        SerializedToolInput::Mcp { name, arguments } => ToolInput::Mcp { name, arguments },
        SerializedToolInput::CronCreate {
            schedule,
            command,
            description,
        } => ToolInput::CronCreate {
            schedule,
            command,
            description,
        },
        SerializedToolInput::CronList => ToolInput::CronList,
        SerializedToolInput::CronDelete { id } => ToolInput::CronDelete { id },
        SerializedToolInput::ScheduleWakeup {
            delay_seconds,
            prompt,
            reason,
        } => ToolInput::ScheduleWakeup {
            delay_seconds,
            prompt,
            reason,
        },
        SerializedToolInput::Monitor { command, until } => ToolInput::Monitor { command, until },
        SerializedToolInput::Lsp {
            kind,
            file,
            line,
            column,
        } => ToolInput::Lsp {
            kind,
            file,
            line,
            column,
        },
        SerializedToolInput::PushNotification { message, title } => {
            ToolInput::PushNotification { message, title }
        }
        SerializedToolInput::RemoteTrigger {
            trigger_id,
            payload,
        } => ToolInput::RemoteTrigger {
            trigger_id,
            payload,
        },
        SerializedToolInput::EnterPlanMode { reason } => ToolInput::EnterPlanMode { reason },
        SerializedToolInput::EnterWorktree { name, branch } => {
            ToolInput::EnterWorktree { name, branch }
        }
        SerializedToolInput::ExitWorktree => ToolInput::ExitWorktree,
        SerializedToolInput::NotebookRead { path } => ToolInput::NotebookRead { path },
        SerializedToolInput::NotebookEdit {
            path,
            cell_id,
            new_source,
            edit_mode,
        } => ToolInput::NotebookEdit {
            path,
            cell_id,
            new_source,
            edit_mode,
        },
        SerializedToolInput::ScratchpadRead { key } => ToolInput::ScratchpadRead { key },
        SerializedToolInput::ScratchpadWrite { key, value } => {
            ToolInput::ScratchpadWrite { key, value }
        }
        SerializedToolInput::Generic { summary } => ToolInput::Generic { summary },
    }
}

pub fn deserialize_tool_output(output: SerializedToolOutput) -> ToolOutput {
    match output {
        SerializedToolOutput::Text { content } => ToolOutput::Text(content),
        SerializedToolOutput::LargeText {
            content,
            line_count,
            byte_count,
        } => ToolOutput::LargeText(LargeText {
            content,
            line_count,
            byte_count,
        }),
        SerializedToolOutput::Diff {
            file_path,
            additions,
            deletions,
            hunks,
        } => ToolOutput::Diff(DiffView {
            file_path,
            additions,
            deletions,
            hunks: hunks.into_iter().map(deserialize_diff_hunk).collect(),
        }),
        SerializedToolOutput::FileContent {
            path,
            content,
            language,
        } => ToolOutput::FileContent {
            path,
            content,
            language,
        },
        SerializedToolOutput::Command {
            stdout,
            stderr,
            exit_code,
        } => ToolOutput::Command {
            stdout,
            stderr,
            exit_code,
        },
        SerializedToolOutput::FileList { files } => ToolOutput::FileList(files),
        SerializedToolOutput::ServerToolResult { wire_type, content } => {
            ToolOutput::ServerToolResult {
                tool_kind: jfc_provider::ServerToolResultKind::from_wire_type(&wire_type),
                content,
            }
        }
        SerializedToolOutput::Empty => ToolOutput::Empty,
    }
}

pub fn deserialize_diff_hunk(hunk: SerializedDiffHunk) -> DiffHunk {
    DiffHunk {
        old_start: hunk.old_start,
        new_start: hunk.new_start,
        header: hunk.header,
        lines: hunk.lines.into_iter().map(deserialize_diff_line).collect(),
    }
}

pub fn deserialize_diff_line(line: SerializedDiffLine) -> DiffLine {
    DiffLine {
        kind: match line.kind.as_str() {
            "added" => DiffLineKind::Added,
            "removed" => DiffLineKind::Removed,
            _ => DiffLineKind::Context,
        },
        old_line: line.old_line,
        new_line: line.new_line,
        content: line.content,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn serialized_assistant(content: &str, elapsed: &str) -> SerializedMessage {
        SerializedMessage {
            role: "assistant".to_owned(),
            agent_name: None,
            model_name: None,
            cost_tier: None,
            elapsed: Some(elapsed.to_owned()),
            usage: None,
            created_at: 0,
            parts: vec![SerializedPart::Text {
                content: content.to_owned(),
            }],
        }
    }

    #[test]
    fn legacy_decorative_elapsed_normalizes_to_took_regression() {
        let msg = deserialize_message(serialized_assistant("answer", "Sautéed for 0s"));

        assert_eq!(msg.elapsed.as_deref(), Some("took 0s"));
    }

    #[test]
    fn error_assistant_drops_legacy_elapsed_regression() {
        let msg = deserialize_message(serialized_assistant(
            "**Error:** Rate limited",
            "Sautéed for 0s",
        ));

        assert_eq!(msg.elapsed, None);
    }

    #[test]
    fn current_elapsed_format_survives_normal() {
        let msg = deserialize_message(serialized_assistant("answer", "took 4s"));

        assert_eq!(msg.elapsed.as_deref(), Some("took 4s"));
    }

    // PLAN TODO 24: the DB session read stores each message's verbatim JSON in
    // `meta` and rebuilds via `from_str::<SerializedMessage>` → `deserialize_
    // message`. This proves that contract is lossless: to_string → from_string →
    // deserialize yields the same ChatMessage as deserializing the original.
    // (The parity verifier already confirms this over the real 344-session
    // corpus; this is the unit-level guard.)
    #[test]
    fn serialized_message_meta_roundtrip_is_lossless_regression() {
        let original = serialized_assistant("the answer is 42", "took 4s");
        let direct = deserialize_message(serialized_assistant("the answer is 42", "took 4s"));

        // Simulate the DB path: meta = to_string(msg); later from_str(meta).
        let meta = serde_json::to_string(&original).expect("serialize");
        let from_db: SerializedMessage = serde_json::from_str(&meta).expect("deserialize");
        let via_db = deserialize_message(from_db);

        assert_eq!(via_db.role, direct.role);
        assert_eq!(via_db.elapsed, direct.elapsed);
        assert_eq!(via_db.parts.len(), direct.parts.len());
        // Text content survives identically.
        let text = |m: &crate::types::ChatMessage| {
            m.parts
                .iter()
                .filter_map(|p| match p {
                    crate::types::MessagePart::Text(t) => Some(t.clone()),
                    _ => None,
                })
                .collect::<Vec<_>>()
        };
        assert_eq!(text(&via_db), text(&direct));
        assert_eq!(text(&via_db), vec!["the answer is 42".to_string()]);
    }
}
