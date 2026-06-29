use crate::{TaskId, TaskLifecycle};

#[derive(Clone, Debug)]
pub struct TaskStatusPart {
    pub task_id: TaskId,
    pub description: String,
    pub status: TaskLifecycle,
    pub summary: Option<String>,
    pub error: Option<String>,
    pub elapsed_ms: Option<u64>,
    /// Model used by this sub-agent. Surfaced in the inline task block so a
    /// glance reveals which model is doing the work (e.g. an Explore agent
    /// running on haiku while the main loop is on opus).
    pub model: Option<String>,
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct TaskInput {
    pub description: String,
    pub prompt: String,
    pub subagent_type: Option<String>,
    pub category: Option<String>,
    pub run_in_background: bool,
    pub model: Option<String>,
    /// Optional agent launch descriptor name. When set, Task execution chooses
    /// that launcher from active plugin descriptors instead of the default
    /// in-process launcher.
    #[serde(default)]
    pub launcher: Option<String>,
    /// Reasoning effort override for this subagent (e.g. "low", "high", "max").
    /// Precedence: Task.effort > AgentDef.effort > global effort.
    pub effort: Option<String>,
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
    /// Optional JSON Schema that the subagent's StructuredOutput tool will
    /// validate against. Set by the parent agent to enforce output shape.
    pub schema: Option<serde_json::Value>,
    /// Optional per-call tool allowlist. When set, this is intersected with the
    /// selected agent definition's allowlist instead of replacing it.
    #[serde(default)]
    pub allowed_tools: Vec<String>,
    /// Optional per-call tool denylist. This is added to the selected agent
    /// definition's denylist.
    #[serde(default)]
    pub disallowed_tools: Vec<String>,
    /// Optional working directory override for the spawned subagent.
    /// When set, the agent starts in this directory instead of the
    /// parent's cwd. Useful for pointing a subagent at a git worktree
    /// or a different project directory.
    #[serde(default)]
    pub cwd: Option<String>,
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
            launcher: None,
            effort: None,
            name: None,
            team_name: None,
            mode: None,
            isolation: None,
            parent_task_id: None,
            schema: None,
            allowed_tools: Vec::new(),
            disallowed_tools: Vec::new(),
            cwd: None,
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
