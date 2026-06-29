//! Serialize runtime types into on-disk `Serialized*` format.

use super::serialization::*;
use crate::types::{
    ChatMessage, DiffHunk, DiffLine, DiffLineKind, MessagePart, Role, TaskLifecycle, ToolInput,
    ToolOutput, ToolStatus,
};

pub fn serialize_message(msg: &ChatMessage) -> SerializedMessage {
    SerializedMessage {
        role: match msg.role {
            Role::User => "user".into(),
            Role::Assistant => "assistant".into(),
        },
        agent_name: msg.agent_name.clone(),
        model_name: msg.model_name.clone(),
        cost_tier: msg.cost_tier.clone(),
        elapsed: msg.elapsed.clone(),
        usage: msg.usage.clone(),
        created_at: msg.created_at,
        parts: msg.parts.iter().map(serialize_part).collect(),
    }
}

pub fn serialize_part(part: &MessagePart) -> SerializedPart {
    match part {
        MessagePart::Text(t) => SerializedPart::Text { content: t.clone() },
        MessagePart::Reasoning(t) => SerializedPart::Reasoning { content: t.clone() },
        MessagePart::ReasoningSignature(signature) => SerializedPart::ReasoningSignature {
            signature: signature.clone(),
        },
        MessagePart::Tool(tc) => SerializedPart::Tool {
            tool: Box::new(SerializedToolPart {
                id: tc.id.as_str().to_owned(),
                kind: tc.kind.label().to_owned(),
                status: serialize_tool_status(tc.status),
                // Persist only the teaser bit — the only display state
                // worth surviving a session reload (see
                // `SerializedPart::Tool::is_collapsed` doc comment).
                is_collapsed: tc.display.is_collapsed(),
                input: Some(serialize_tool_input(&tc.input)),
                output: Some(serialize_tool_output(&tc.output)),
                thought_signature: tc.thought_signature.clone(),
            }),
        },
        MessagePart::TaskStatus(ts) => SerializedPart::TaskStatus {
            task_id: ts.task_id.as_str().to_owned(),
            description: ts.description.clone(),
            status: serialize_task_lifecycle(ts.status),
            summary: ts.summary.clone(),
            error: ts.error.clone(),
            elapsed_ms: ts.elapsed_ms,
        },
        MessagePart::CompactBoundary { pre_tokens } => SerializedPart::CompactBoundary {
            pre_tokens: *pre_tokens,
        },
        MessagePart::Advisor(t) => SerializedPart::Advisor { content: t.clone() },
        MessagePart::RedactedThinking(data) => {
            SerializedPart::RedactedThinking { data: data.clone() }
        }
    }
}

pub fn serialize_tool_status(status: ToolStatus) -> String {
    // ToolStatus is now an alias for ExecutionStatus, which has two
    // extra variants (Idle, Cancelled) that tools didn't historically
    // produce. Map them to the closest tool-shaped value so legacy
    // session readers (which only know about pending/running/complete/
    // failed) still see something sensible:
    //   - Idle → "running" (the tool is still in flight, just quiet)
    //   - Cancelled → "failed" (denied / abandoned tools surface as
    //     failures from the model's perspective)
    // Wire format remains "complete" for Completed (NOT "completed")
    // — preserves backward compatibility with on-disk session JSON.
    match status {
        ToolStatus::Pending => "pending".into(),
        ToolStatus::Running | ToolStatus::Idle => "running".into(),
        ToolStatus::Completed => "complete".into(),
        ToolStatus::Failed | ToolStatus::Cancelled => "failed".into(),
    }
}

pub fn serialize_task_lifecycle(status: TaskLifecycle) -> String {
    match status {
        TaskLifecycle::Pending => "pending".into(),
        TaskLifecycle::Running => "running".into(),
        TaskLifecycle::Idle => "idle".into(),
        TaskLifecycle::Completed => "completed".into(),
        TaskLifecycle::Failed => "failed".into(),
        TaskLifecycle::Cancelled => "cancelled".into(),
    }
}

pub fn serialize_tool_input(input: &ToolInput) -> SerializedToolInput {
    match input {
        ToolInput::Edit {
            file_path,
            old_string,
            new_string,
            replacement,
        } => SerializedToolInput::Edit {
            file_path: file_path.clone(),
            old_string: old_string.clone(),
            new_string: new_string.clone(),
            replace_all: replacement.replace_all(),
        },
        ToolInput::Write { file_path, content } => SerializedToolInput::Write {
            file_path: file_path.clone(),
            content: content.clone(),
        },
        ToolInput::Read {
            file_path,
            offset,
            limit,
        } => SerializedToolInput::Read {
            file_path: file_path.clone(),
            offset: *offset,
            limit: *limit,
        },
        ToolInput::Bash {
            command,
            timeout,
            workdir,
            run_in_background,
            suppress_output,
        } => SerializedToolInput::Bash {
            command: command.clone(),
            timeout: *timeout,
            workdir: workdir.clone(),
            run_in_background: *run_in_background,
            suppress_output: *suppress_output,
        },
        ToolInput::BashOutput {
            task_id,
            offset,
            limit,
            block,
            timeout,
            wait_up_to,
        } => SerializedToolInput::BashOutput {
            task_id: task_id.clone(),
            offset: *offset,
            limit: *limit,
            block: *block,
            timeout: *timeout,
            wait_up_to: *wait_up_to,
        },
        ToolInput::Glob { pattern, path } => SerializedToolInput::Glob {
            pattern: pattern.clone(),
            path: path.clone(),
        },
        ToolInput::Grep {
            pattern,
            path,
            glob,
            output_mode,
        } => SerializedToolInput::Grep {
            pattern: pattern.clone(),
            path: path.clone(),
            glob: glob.clone(),
            output_mode: output_mode.clone(),
        },
        ToolInput::Search { query, path } => SerializedToolInput::Search {
            query: query.clone(),
            path: path.clone(),
        },
        ToolInput::ApplyPatch { patch } => SerializedToolInput::ApplyPatch {
            patch: patch.clone(),
        },
        ToolInput::Task(ti) => SerializedToolInput::Task {
            description: ti.description.clone(),
            prompt: ti.prompt.clone(),
            subagent_type: ti.subagent_type.clone(),
            category: ti.category.clone(),
            run_in_background: ti.run_in_background,
            model: ti.model.clone(),
            launcher: ti.launcher.clone(),
            effort: ti.effort.clone(),
            name: ti.name.clone(),
            team_name: ti.team_name.clone(),
            mode: ti.mode.clone(),
            isolation: ti.isolation.clone(),
            parent_task_id: ti.parent_task_id.clone(),
            schema: ti.schema.clone(),
            allowed_tools: ti.allowed_tools.clone(),
            disallowed_tools: ti.disallowed_tools.clone(),
            cwd: ti.cwd.clone(),
        },
        ToolInput::TaskCreate {
            subject,
            description,
            active_form,
            blocked_by,
            acceptance_criteria,
            verification_command,
            risk,
            parent_id,
            kind,
            tags: _,
            priority: _,
            effort: _,
            model: _,
        } => SerializedToolInput::TaskCreate {
            subject: subject.clone(),
            description: description.clone(),
            active_form: active_form.clone(),
            blocked_by: blocked_by.clone(),
            acceptance_criteria: acceptance_criteria.clone(),
            verification_command: verification_command.clone(),
            risk: risk.clone(),
            parent_id: parent_id.clone(),
            kind: kind.clone(),
        },
        ToolInput::TaskUpdate {
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
            blocked_by: _,
            tags: _,
            priority: _,
            effort: _,
            model: _,
        } => SerializedToolInput::TaskUpdate {
            task_id: task_id.clone(),
            status: status.clone(),
            subject: subject.clone(),
            description: description.clone(),
            owner: owner.clone(),
            acceptance_criteria: acceptance_criteria.clone(),
            verification_command: verification_command.clone(),
            risk: risk.clone(),
            parent_id: parent_id.clone(),
            kind: kind.clone(),
        },
        ToolInput::TaskList {
            status_filter,
            owner_filter,
            include_history,
            history_query,
        } => SerializedToolInput::TaskList {
            status_filter: status_filter.clone(),
            owner_filter: owner_filter.clone(),
            include_history: *include_history,
            history_query: history_query.clone(),
        },
        ToolInput::TaskDone { task_id } => SerializedToolInput::TaskDone {
            task_id: task_id.clone(),
        },
        ToolInput::TaskStop { task_id } => SerializedToolInput::TaskStop {
            task_id: task_id.clone(),
        },
        ToolInput::TaskGet { task_id } => SerializedToolInput::TaskGet {
            task_id: task_id.clone(),
        },
        ToolInput::TaskValidate => SerializedToolInput::TaskValidate,
        ToolInput::Skill { name, args } => SerializedToolInput::Skill {
            name: name.clone(),
            args: args.clone(),
        },
        ToolInput::ToolSearch { query, limit } => SerializedToolInput::ToolSearch {
            query: query.clone(),
            limit: *limit,
        },
        ToolInput::ToolSuggest { intent, limit } => SerializedToolInput::ToolSuggest {
            intent: intent.clone(),
            limit: *limit,
        },
        ToolInput::MemoryCreate {
            level,
            memory_type,
            scope,
            body,
        } => SerializedToolInput::MemoryCreate {
            level: level.clone(),
            memory_type: memory_type.clone(),
            scope: scope.clone(),
            body: body.clone(),
        },
        ToolInput::MemoryDelete { path } => {
            SerializedToolInput::MemoryDelete { path: path.clone() }
        }
        ToolInput::TeamCreate {
            team_name,
            description,
        } => SerializedToolInput::TeamCreate {
            team_name: team_name.clone(),
            description: description.clone(),
        },
        ToolInput::TeamDelete => SerializedToolInput::TeamDelete,
        ToolInput::SendMessage {
            to,
            message,
            summary,
        } => SerializedToolInput::SendMessage {
            to: to.clone(),
            message: message.clone(),
            summary: summary.clone(),
        },
        ToolInput::TeamMemberMode { member_name, mode } => SerializedToolInput::TeamMemberMode {
            member_name: member_name.clone(),
            mode: mode.clone(),
        },
        ToolInput::HcomStatus { .. }
        | ToolInput::HcomList { .. }
        | ToolInput::HcomSend { .. }
        | ToolInput::HcomEvents { .. }
        | ToolInput::HcomListen { .. }
        | ToolInput::HcomTranscript { .. }
        | ToolInput::HcomBundle { .. }
        | ToolInput::HcomTerm { .. }
        | ToolInput::HcomLaunch { .. }
        | ToolInput::HcomResume { .. }
        | ToolInput::HcomFork { .. }
        | ToolInput::HcomKill { .. }
        | ToolInput::HcomRelay { .. }
        | ToolInput::HcomRun { .. } => serialize_generic_tool_input_json(input),
        ToolInput::PlanCreate { title, .. } => SerializedToolInput::Generic {
            summary: format!("plan_create: {title}"),
        },
        ToolInput::PlanList { .. } => SerializedToolInput::Generic {
            summary: "plan_list".into(),
        },
        ToolInput::PlanShow { slug, .. } => SerializedToolInput::Generic {
            summary: format!("plan_show: {slug}"),
        },
        ToolInput::PlanAdvance { slug, .. } => SerializedToolInput::Generic {
            summary: format!("plan_advance: {slug}"),
        },
        ToolInput::PlanArchive { slug, .. } => SerializedToolInput::Generic {
            summary: format!("plan_archive: {slug}"),
        },
        ToolInput::PlanMaterialize { slug } => SerializedToolInput::Generic {
            summary: format!("plan_materialize: {slug}"),
        },
        ToolInput::LearnStatus { .. } => SerializedToolInput::Generic {
            summary: "learn_status".into(),
        },
        ToolInput::LearnHistorize { .. } => SerializedToolInput::Generic {
            summary: "learn_historize".into(),
        },
        ToolInput::LearnDream { .. } => SerializedToolInput::Generic {
            summary: "learn_dream".into(),
        },
        ToolInput::LearnRsiList { status, limit } => SerializedToolInput::Generic {
            summary: match (status, limit) {
                (Some(status), Some(limit)) => format!("learn_rsi_list {status} limit={limit}"),
                (Some(status), None) => format!("learn_rsi_list {status}"),
                (None, Some(limit)) => format!("learn_rsi_list limit={limit}"),
                (None, None) => "learn_rsi_list".into(),
            },
        },
        ToolInput::LearnRsiPromote { kind, name } => SerializedToolInput::Generic {
            summary: format!("learn_rsi_promote {kind}/{name}"),
        },
        ToolInput::LearnRsiRollback { kind, name } => SerializedToolInput::Generic {
            summary: format!("learn_rsi_rollback {kind}/{name}"),
        },
        ToolInput::LearnKeyFilesList { .. } => SerializedToolInput::Generic {
            summary: "learn_key_files_list".into(),
        },
        ToolInput::LearnUserProfileShow { .. } => SerializedToolInput::Generic {
            summary: "learn_user_profile_show".into(),
        },
        ToolInput::PostBounty {
            description,
            budget,
            acceptance_criteria,
            max_solvers,
            auto_dispatch,
            parent_task_id,
        } => SerializedToolInput::PostBounty {
            description: description.clone(),
            budget: *budget,
            acceptance_criteria: acceptance_criteria.clone(),
            max_solvers: *max_solvers,
            auto_dispatch: *auto_dispatch,
            parent_task_id: parent_task_id.clone(),
        },
        ToolInput::MarketStatus { bounty_id } => SerializedToolInput::MarketStatus {
            bounty_id: bounty_id.clone(),
        },
        ToolInput::RunBounty {
            bounty_id,
            max_solvers,
        } => SerializedToolInput::RunBounty {
            bounty_id: bounty_id.clone(),
            max_solvers: *max_solvers,
        },
        ToolInput::ExitPlanMode { plan } => {
            SerializedToolInput::ExitPlanMode { plan: plan.clone() }
        }
        ToolInput::SubmitPlan { .. }
        | ToolInput::AddReviewComment { .. }
        | ToolInput::SuggestCommitMessage { .. } => serialize_generic_tool_input_json(input),
        ToolInput::MultiEdit { file_path, edits } => SerializedToolInput::MultiEdit {
            file_path: file_path.clone(),
            edits: edits.clone(),
        },
        ToolInput::AskUserQuestion { questions } => SerializedToolInput::AskUserQuestion {
            questions: questions.clone(),
        },
        ToolInput::WebFetch { url, prompt } => SerializedToolInput::WebFetch {
            url: url.clone(),
            prompt: prompt.clone(),
        },
        ToolInput::WebSearch { query, max_results } => SerializedToolInput::WebSearch {
            query: query.clone(),
            max_results: *max_results,
        },
        ToolInput::Mcp { name, arguments } => SerializedToolInput::Mcp {
            name: name.clone(),
            arguments: arguments.clone(),
        },
        ToolInput::CronCreate {
            schedule,
            command,
            description,
        } => SerializedToolInput::CronCreate {
            schedule: schedule.clone(),
            command: command.clone(),
            description: description.clone(),
        },
        ToolInput::CronList => SerializedToolInput::CronList,
        ToolInput::CronDelete { id } => SerializedToolInput::CronDelete { id: id.clone() },
        ToolInput::ScheduleWakeup {
            delay_seconds,
            prompt,
            reason,
        } => SerializedToolInput::ScheduleWakeup {
            delay_seconds: *delay_seconds,
            prompt: prompt.clone(),
            reason: reason.clone(),
        },
        ToolInput::Monitor { command, until } => SerializedToolInput::Monitor {
            command: command.clone(),
            until: until.clone(),
        },
        ToolInput::Lsp {
            kind,
            file,
            line,
            column,
        } => SerializedToolInput::Lsp {
            kind: kind.clone(),
            file: file.clone(),
            line: *line,
            column: *column,
        },
        ToolInput::PushNotification { message, title } => SerializedToolInput::PushNotification {
            message: message.clone(),
            title: title.clone(),
        },
        ToolInput::RemoteTrigger {
            trigger_id,
            payload,
        } => SerializedToolInput::RemoteTrigger {
            trigger_id: trigger_id.clone(),
            payload: payload.clone(),
        },
        ToolInput::EnterPlanMode { reason } => SerializedToolInput::EnterPlanMode {
            reason: reason.clone(),
        },
        ToolInput::EnterWorktree { name, branch } => SerializedToolInput::EnterWorktree {
            name: name.clone(),
            branch: branch.clone(),
        },
        ToolInput::ExitWorktree => SerializedToolInput::ExitWorktree,
        ToolInput::NotebookRead { path } => {
            SerializedToolInput::NotebookRead { path: path.clone() }
        }
        ToolInput::NotebookEdit {
            path,
            cell_id,
            new_source,
            edit_mode,
        } => SerializedToolInput::NotebookEdit {
            path: path.clone(),
            cell_id: cell_id.clone(),
            new_source: new_source.clone(),
            edit_mode: edit_mode.clone(),
        },
        ToolInput::ScratchpadRead { key } => {
            SerializedToolInput::ScratchpadRead { key: key.clone() }
        }
        ToolInput::ScratchpadWrite { key, value } => SerializedToolInput::ScratchpadWrite {
            key: key.clone(),
            value: value.clone(),
        },
        ToolInput::Workflow {
            script: _,
            name,
            script_path,
            args: _,
            resume_from_run_id: _,
        } => SerializedToolInput::Generic {
            summary: format!(
                "Workflow({})",
                name.as_deref()
                    .or(script_path.as_deref())
                    .unwrap_or("inline")
            ),
        },
        ToolInput::Generic { summary } => SerializedToolInput::Generic {
            summary: summary.clone(),
        },
        ToolInput::SendUserMessage { message, .. } => {
            let preview: String = message.chars().take(80).collect();
            SerializedToolInput::Generic {
                summary: format!("SendUserMessage: {preview}"),
            }
        }
        ToolInput::SendUserFile { caption, .. } => SerializedToolInput::Generic {
            summary: format!("SendUserFile: {}", caption.as_deref().unwrap_or("file(s)")),
        },
        ToolInput::StructuredOutput { .. } => SerializedToolInput::Generic {
            summary: "StructuredOutput".into(),
        },
        ToolInput::WaitForMcpServers { .. } => SerializedToolInput::Generic {
            summary: "WaitForMcpServers".into(),
        },
        ToolInput::ListMcpResources { .. } => SerializedToolInput::Generic {
            summary: "ListMcpResources".into(),
        },
        ToolInput::ReadMcpResource { .. } => SerializedToolInput::Generic {
            summary: "ReadMcpResource".into(),
        },
        ToolInput::Advisor {} => SerializedToolInput::Generic {
            summary: "Advisor".into(),
        },
        ToolInput::ConnectGitHub {} => SerializedToolInput::Generic {
            summary: "ConnectGitHub".into(),
        },
        ToolInput::DesignProjectCreate { .. }
        | ToolInput::DesignProjectList {}
        | ToolInput::DesignProjectSetMeta { .. }
        | ToolInput::DesignListFiles { .. }
        | ToolInput::DesignReadFile { .. }
        | ToolInput::DesignWriteFile { .. }
        | ToolInput::DesignDeleteFile { .. }
        | ToolInput::DesignCopyFile { .. }
        | ToolInput::DesignRegisterAsset { .. }
        | ToolInput::DesignUnregisterAsset { .. }
        | ToolInput::DesignBundleHtml { .. }
        | ToolInput::DesignHandoff { .. }
        | ToolInput::DesignCheckSystem { .. }
        | ToolInput::DesignCapabilities { .. }
        | ToolInput::DesignServe { .. }
        | ToolInput::SetGoal { .. }
        | ToolInput::Research { .. }
        | ToolInput::SlashCommand { .. }
        | ToolInput::Council { .. }
        | ToolInput::AskModel { .. }
        | ToolInput::SkillCreate { .. } => serialize_generic_tool_input_json(input),
    }
}
pub fn serialize_generic_tool_input_json(input: &ToolInput) -> SerializedToolInput {
    SerializedToolInput::Generic {
        summary: input.to_value().to_string(),
    }
}

pub fn serialize_tool_output(output: &ToolOutput) -> SerializedToolOutput {
    match output {
        ToolOutput::Text(content) => SerializedToolOutput::Text {
            content: content.clone(),
        },
        ToolOutput::LargeText(lt) => SerializedToolOutput::LargeText {
            content: lt.content.clone(),
            line_count: lt.line_count,
            byte_count: lt.byte_count,
        },
        ToolOutput::Diff(d) => SerializedToolOutput::Diff {
            file_path: d.file_path.clone(),
            additions: d.additions,
            deletions: d.deletions,
            hunks: d.hunks.iter().map(serialize_diff_hunk).collect(),
        },
        ToolOutput::FileContent {
            path,
            content,
            language,
        } => SerializedToolOutput::FileContent {
            path: path.clone(),
            content: content.clone(),
            language: language.clone(),
        },
        ToolOutput::Command {
            stdout,
            stderr,
            exit_code,
        } => SerializedToolOutput::Command {
            stdout: stdout.clone(),
            stderr: stderr.clone(),
            exit_code: *exit_code,
        },
        ToolOutput::FileList(files) => SerializedToolOutput::FileList {
            files: files.clone(),
        },
        ToolOutput::ServerToolResult { tool_kind, content } => {
            SerializedToolOutput::ServerToolResult {
                wire_type: tool_kind.wire_type().to_owned(),
                content: content.clone(),
            }
        }
        ToolOutput::Empty => SerializedToolOutput::Empty,
    }
}

pub fn serialize_diff_hunk(hunk: &DiffHunk) -> SerializedDiffHunk {
    SerializedDiffHunk {
        old_start: hunk.old_start,
        new_start: hunk.new_start,
        header: hunk.header.clone(),
        lines: hunk.lines.iter().map(serialize_diff_line).collect(),
    }
}

pub fn serialize_diff_line(line: &DiffLine) -> SerializedDiffLine {
    SerializedDiffLine {
        kind: match line.kind {
            DiffLineKind::Context => "context".into(),
            DiffLineKind::Added => "added".into(),
            DiffLineKind::Removed => "removed".into(),
        },
        old_line: line.old_line,
        new_line: line.new_line,
        content: line.content.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn send_user_message_preview_is_utf8_safe_regression() {
        let message = format!("{}🙂", "a".repeat(79));
        let serialized = serialize_tool_input(&ToolInput::SendUserMessage {
            message,
            summary: None,
            attachments: None,
            status: None,
        });

        let SerializedToolInput::Generic { summary } = serialized else {
            panic!("SendUserMessage should serialize as a generic summary");
        };
        assert!(summary.ends_with("🙂"));
    }
}
