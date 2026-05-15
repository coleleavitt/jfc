use crate::{TaskId, TaskLifecycle};

#[derive(Clone, Debug)]
pub struct TaskStatusPart {
    pub task_id: TaskId,
    pub description: String,
    pub status: TaskLifecycle,
    pub summary: Option<String>,
    pub error: Option<String>,
    pub elapsed_ms: Option<u64>,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct TaskInput {
    pub description: String,
    pub prompt: String,
    pub subagent_type: Option<String>,
    pub category: Option<String>,
    pub run_in_background: bool,
    pub model: Option<String>,
    /// Name for the spawned agent — makes it addressable via SendMessage.
    /// When set along with `team_name`, spawns a persistent teammate instead
    /// of a one-shot subagent.
    pub name: Option<String>,
    /// Team to spawn the agent into. Uses current team context if omitted.
    pub team_name: Option<String>,
    /// Permission mode for the spawned teammate (e.g., "plan" to require approval).
    pub mode: Option<String>,
    /// Isolation mode: "worktree" creates a temp git worktree for the agent.
    pub isolation: Option<String>,
    /// Queued-task id (`t<N>`) this delegation is fulfilling.
    pub parent_task_id: Option<String>,
}

impl TaskInput {
    pub fn summary(&self) -> String {
        if let Some(ref name) = self.name {
            format!("spawn teammate: {name} — {}", self.description)
        } else {
            format!(
                "{} ({})",
                self.description,
                if self.run_in_background {
                    "background"
                } else {
                    "foreground"
                }
            )
        }
    }

    /// Whether this Task invocation should spawn a persistent teammate
    /// rather than a one-shot subagent.
    pub fn is_teammate_spawn(&self) -> bool {
        self.name.is_some() && self.team_name.is_some()
    }

    /// Whether this is a fork (no subagent_type specified). Forks inherit
    /// the parent's full conversation context and share the prompt cache.
    /// This is the cheapest delegation path.
    pub fn is_fork(&self) -> bool {
        self.subagent_type.is_none() && !self.is_teammate_spawn()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn task_input() -> TaskInput {
        TaskInput {
            description: "do thing".into(),
            prompt: "please do it".into(),
            subagent_type: None,
            category: None,
            run_in_background: false,
            model: None,
            name: None,
            team_name: None,
            mode: None,
            isolation: None,
            parent_task_id: None,
        }
    }

    #[test]
    fn task_input_summary_background_flag() {
        let fg = task_input();
        assert!(fg.summary().contains("foreground"));

        let bg = TaskInput {
            run_in_background: true,
            ..fg
        };
        assert!(bg.summary().contains("background"));
    }

    #[test]
    fn task_input_teammate_spawn_requires_name_and_team_normal() {
        let input = TaskInput {
            name: Some("reviewer".into()),
            team_name: Some("core".into()),
            ..task_input()
        };
        assert!(input.is_teammate_spawn());
        assert!(!input.is_fork());
    }
}
