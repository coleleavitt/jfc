//! Handler for `EngineEvent::WorkflowProgress` — applies incremental progress
//! updates from the workflow runner to the matching `BackgroundTask`.

use crate::app::EngineState;
use crate::runtime::WorkflowProgressEvent;
use crate::workflows::meta::WorkflowMeta;
use crate::workflows::{AgentProgress, AgentStatus, WorkflowTaskProgress};

/// Apply a single `WorkflowProgressEvent` to the matching `BackgroundTask`.
pub fn handle_workflow_progress(state: &mut EngineState, ev: WorkflowProgressEvent) {
    // Every workflow progress event IS task activity. Only
    // `TaskEvent::Progress` used to refresh `last_activity_at`, and workflows
    // never emit it — so a live auto-review workflow froze its activity clock
    // at Started and the agents panel showed "⚠ stalled Ns" forever.
    if let Some(bt) = state.background_tasks.get_mut(ev.task_id().as_str()) {
        bt.last_activity_at = std::time::Instant::now();
    }
    match ev {
        WorkflowProgressEvent::Phase { task_id, title } => {
            apply_phase(state, task_id.as_str(), title);
        }
        WorkflowProgressEvent::AgentStarted {
            task_id,
            index,
            label,
            phase,
        } => {
            apply_agent_started(state, task_id.as_str(), index, label, phase);
        }
        WorkflowProgressEvent::AgentCacheHit {
            task_id,
            index,
            label,
            phase,
        } => {
            apply_agent_cache_hit(state, task_id.as_str(), index, label, phase);
        }
        WorkflowProgressEvent::AgentDone { task_id, index } => {
            apply_agent_done(state, task_id.as_str(), index);
        }
        WorkflowProgressEvent::AgentFailed {
            task_id,
            index,
            error,
        } => {
            apply_agent_failed(state, task_id.as_str(), index, error);
        }
        WorkflowProgressEvent::Log { task_id, message } => {
            apply_log(state, task_id.as_str(), message);
        }
    }
}

fn apply_phase(state: &mut EngineState, task_id: &str, title: String) {
    let Some(bt) = state.background_tasks.get_mut(task_id) else {
        return;
    };
    let wfp = ensure_progress(bt, task_id);
    wfp.current_phase = Some(title.clone());
    wfp.logs.push(format!("phase: {title}"));
}

fn apply_agent_started(
    state: &mut EngineState,
    task_id: &str,
    index: u32,
    label: String,
    phase: Option<String>,
) {
    let Some(bt) = state.background_tasks.get_mut(task_id) else {
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
    state: &mut EngineState,
    task_id: &str,
    index: u32,
    label: String,
    phase: Option<String>,
) {
    let Some(bt) = state.background_tasks.get_mut(task_id) else {
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

fn apply_agent_done(state: &mut EngineState, task_id: &str, index: u32) {
    let Some(bt) = state.background_tasks.get_mut(task_id) else {
        return;
    };
    if let Some(ref mut wfp) = bt.workflow_progress
        && let Some(agent) = wfp.agents.iter_mut().find(|a| a.index == index)
    {
        agent.status = AgentStatus::Done;
    }
}

fn apply_agent_failed(state: &mut EngineState, task_id: &str, index: u32, error: String) {
    let Some(bt) = state.background_tasks.get_mut(task_id) else {
        return;
    };
    if let Some(ref mut wfp) = bt.workflow_progress {
        if let Some(agent) = wfp.agents.iter_mut().find(|a| a.index == index) {
            agent.status = AgentStatus::Failed;
        }
        wfp.logs.push(format!("agent {index} failed: {error}"));
    }
}

fn apply_log(state: &mut EngineState, task_id: &str, message: String) {
    let Some(bt) = state.background_tasks.get_mut(task_id) else {
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
