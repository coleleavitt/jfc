use crate::{
    app::App,
    runtime::{AppEvent, EventSender, UiEvent},
    tasks::{TaskPatch, TaskRisk, TaskStatus},
};

pub(crate) fn factory_mode_enabled() -> bool {
    !matches!(
        std::env::var("JFC_FACTORY_MODE").as_deref(),
        Ok("0" | "false" | "off" | "no")
    )
}

pub(crate) async fn maybe_continue_task_factory(app: &mut App, tx: &EventSender) {
    if !factory_mode_enabled()
        || app.is_streaming
        || app.pending_approval.is_some()
        || !app.approval_queue.is_empty()
        || !app.pending_tool_calls.is_empty()
        || !app.queued_prompts.is_empty()
        || app
            .background_tasks
            .values()
            .any(|task| task.status.is_alive())
    {
        return;
    }

    let counts = app.task_store.counts();
    if counts.pending >= 3 && counts.in_progress == 0 && !app.plan_verified_this_batch {
        app.plan_verified_this_batch = true;
        let tasks = app.task_store.list_all();
        let pending: Vec<_> = tasks
            .iter()
            .filter(|task| task.status == TaskStatus::Pending)
            .collect();
        let task_list = pending
            .iter()
            .map(|task| {
                let mut line = format!(
                    "- {} (blocked_by: {:?}): {}",
                    task.id, task.blocked_by, task.subject
                );
                if let Some(ref risk) = task.risk {
                    line.push_str(&format!(" [risk: {risk:?}]"));
                }
                if let Some(ref criteria) = task.acceptance_criteria {
                    line.push_str(&format!(" | criteria: {criteria}"));
                }
                if let Some(ref kind) = task.kind {
                    line.push_str(&format!(" | kind: {kind:?}"));
                }
                line
            })
            .collect::<Vec<_>>()
            .join("\n");
        let prompt = format!(
            "Before executing the task queue, verify this plan is sound:\n\n{task_list}\n\n\
             Check for: missing dependencies, circular deps, tasks that should be parallel but are serial, \
             tasks that are too broad to complete in one agent turn, high-risk tasks that need user review, \
             tasks missing acceptance criteria. \
             If the plan is good, say 'Plan verified' and I'll start execution. \
             If changes are needed, use TaskUpdate/TaskCreate/TaskDone to revise, then say 'Plan revised'."
        );
        let _ = tx.send(AppEvent::Ui(UiEvent::Submit(prompt))).await;
        return;
    }

    let Some(task) = app.task_store.claim_next_available("jfc-factory") else {
        return;
    };

    if matches!(task.risk, Some(TaskRisk::High)) {
        let _ = app.task_store.update(
            task.id.as_str(),
            TaskPatch {
                status: Some(TaskStatus::Pending),
                owner: None,
                ..Default::default()
            },
        );
        tracing::info!(
            target: "jfc::tasks::factory",
            task_id = %task.id,
            "high-risk task requires user approval; skipping auto-execution"
        );
        let prompt = format!(
            "Task `{}` ('{}') is marked high-risk. Please review and approve before I execute it.\n\
             Description: {}\n\
             Acceptance criteria: {}",
            task.id,
            task.subject,
            task.description,
            task.acceptance_criteria.as_deref().unwrap_or("(none)")
        );
        let _ = tx.send(AppEvent::Ui(UiEvent::Submit(prompt))).await;
        return;
    }

    let mut prompt = format!(
        "Continue the task queue. Work on task `{}`: {}\n\n{}",
        task.id, task.subject, task.description
    );
    if let Some(ref criteria) = task.acceptance_criteria {
        prompt.push_str(&format!("\n\nAcceptance criteria: {criteria}"));
    }
    if let Some(ref command) = task.verification_command {
        prompt.push_str(&format!("\nVerification command: `{command}`"));
    }
    prompt.push_str(&format!(
        "\n\nWhen this task is done, update its task status before stopping. \
         If you delegate this work via the Task tool, pass `parent_task_id: \"{}\"` \
         so the runtime auto-marks the task in_progress/completed/failed as the \
         subagent runs - no separate TaskUpdate/TaskDone needed. \
         If more unblocked tasks remain, continue with the next one.",
        task.id
    ));
    tracing::info!(
        target: "jfc::tasks::factory",
        task_id = %task.id,
        subject = %task.subject,
        "auto-continuing next available task"
    );
    let _ = tx.send(AppEvent::Ui(UiEvent::Submit(prompt))).await;
}
