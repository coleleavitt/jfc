//! Handler for `AppEvent::WorkflowProgress` — applies incremental progress
//! updates from the workflow runner to the matching `BackgroundTask`.

use crate::app::App;
use crate::runtime::WorkflowProgressEvent;
use crate::workflows::meta::WorkflowMeta;
use crate::workflows::{AgentProgress, AgentStatus, WorkflowTaskProgress};

/// Apply a single `WorkflowProgressEvent` to the matching `BackgroundTask`.
pub(crate) fn handle_workflow_progress(app: &mut App, ev: WorkflowProgressEvent) {
    match ev {
        WorkflowProgressEvent::Phase { task_id, title } => {
            apply_phase(app, task_id.as_str(), title);
        }
        WorkflowProgressEvent::AgentStarted {
            task_id,
            index,
            label,
            phase,
        } => {
            apply_agent_started(app, task_id.as_str(), index, label, phase);
        }
        WorkflowProgressEvent::AgentCacheHit {
            task_id,
            index,
            label,
            phase,
        } => {
            apply_agent_cache_hit(app, task_id.as_str(), index, label, phase);
        }
        WorkflowProgressEvent::AgentDone { task_id, index } => {
            apply_agent_done(app, task_id.as_str(), index);
        }
        WorkflowProgressEvent::AgentFailed {
            task_id,
            index,
            error,
        } => {
            apply_agent_failed(app, task_id.as_str(), index, error);
        }
        WorkflowProgressEvent::Log { task_id, message } => {
            apply_log(app, task_id.as_str(), message);
        }
    }
}

fn apply_phase(app: &mut App, task_id: &str, title: String) {
    let Some(bt) = app.background_tasks.get_mut(task_id) else {
        return;
    };
    let wfp = ensure_progress(bt, task_id);
    wfp.current_phase = Some(title.clone());
    wfp.logs.push(format!("phase: {title}"));
}

fn apply_agent_started(
    app: &mut App,
    task_id: &str,
    index: u32,
    label: String,
    phase: Option<String>,
) {
    let Some(bt) = app.background_tasks.get_mut(task_id) else {
        return;
    };
    let wfp = ensure_progress(bt, task_id);
    let effective_phase = phase.or_else(|| wfp.current_phase.clone());
    wfp.agents.push(AgentProgress {
        index,
        label,
        phase: effective_phase,
        status: AgentStatus::Running,
    });
    wfp.total_dispatched += 1;
}

fn apply_agent_cache_hit(
    app: &mut App,
    task_id: &str,
    index: u32,
    label: String,
    phase: Option<String>,
) {
    let Some(bt) = app.background_tasks.get_mut(task_id) else {
        return;
    };
    let wfp = ensure_progress(bt, task_id);
    let effective_phase = phase.or_else(|| wfp.current_phase.clone());
    // Cache hits appear as Done immediately — no Running transition needed.
    wfp.agents.push(AgentProgress {
        index,
        label,
        phase: effective_phase,
        status: AgentStatus::Done,
    });
    wfp.cache_hits += 1;
}

fn apply_agent_done(app: &mut App, task_id: &str, index: u32) {
    let Some(bt) = app.background_tasks.get_mut(task_id) else {
        return;
    };
    if let Some(ref mut wfp) = bt.workflow_progress
        && let Some(agent) = wfp.agents.iter_mut().find(|a| a.index == index)
    {
        agent.status = AgentStatus::Done;
    }
}

fn apply_agent_failed(app: &mut App, task_id: &str, index: u32, error: String) {
    let Some(bt) = app.background_tasks.get_mut(task_id) else {
        return;
    };
    if let Some(ref mut wfp) = bt.workflow_progress {
        if let Some(agent) = wfp.agents.iter_mut().find(|a| a.index == index) {
            agent.status = AgentStatus::Failed;
        }
        wfp.logs.push(format!("agent {index} failed: {error}"));
    }
}

fn apply_log(app: &mut App, task_id: &str, message: String) {
    let Some(bt) = app.background_tasks.get_mut(task_id) else {
        return;
    };
    let wfp = ensure_progress(bt, task_id);
    wfp.logs.push(message);
}

/// Return `bt.workflow_progress`, initialising it from the task's description
/// if it hasn't been set yet (first event received for this workflow task).
fn ensure_progress<'a>(
    bt: &'a mut crate::app::BackgroundTask,
    task_id: &str,
) -> &'a mut WorkflowTaskProgress {
    if bt.workflow_progress.is_none() {
        let run_id = task_id.strip_prefix("bgwf_").unwrap_or(task_id).to_owned();
        let meta = WorkflowMeta {
            name: bt.description.clone(),
            description: String::new(),
            when_to_use: None,
            phases: Vec::new(),
        };
        bt.workflow_progress = Some(WorkflowTaskProgress::new(run_id, meta));
    }
    bt.workflow_progress.as_mut().unwrap()
}
