//! Plan tool implementations for the `/plan` slash command.
//!
//! Each function uses `PlanStore::open_project(None)` and performs a single
//! plan operation, returning an `ExecutionResult`.

use super::ExecutionResult;
use crate::plan::{PlanStatus, PlanStore};

/// Create a new plan.
pub fn execute_plan_create(title: &str, body: Option<&str>) -> ExecutionResult {
    let store = match PlanStore::open_project(None) {
        Ok(s) => s,
        Err(e) => return ExecutionResult::failure(format!("Failed to open plan store: {e}")),
    };

    let body_text = body.unwrap_or("");
    match store.create(title, body_text) {
        Ok(plan) => ExecutionResult::success(format!(
            "Plan created: {} (slug: {})\nStatus: {}\nPath: {}",
            plan.frontmatter.title,
            plan.frontmatter.slug,
            plan.frontmatter.status,
            plan.path.display()
        )),
        Err(e) => ExecutionResult::failure(format!("Failed to create plan: {e}")),
    }
}

/// List plans, optionally filtered by status.
pub fn execute_plan_list(status: Option<&str>) -> ExecutionResult {
    let store = match PlanStore::open_project(None) {
        Ok(s) => s,
        Err(e) => return ExecutionResult::failure(format!("Failed to open plan store: {e}")),
    };

    let filter = status.and_then(|s| s.parse::<PlanStatus>().ok());
    let plans = store.list(filter);

    if plans.is_empty() {
        return ExecutionResult::success("No plans found.");
    }

    let mut output = format!("Plans ({}):\n\n", plans.len());
    for plan in &plans {
        output.push_str(&format!(
            "- [{}] {} ({})\n",
            plan.frontmatter.status, plan.frontmatter.title, plan.frontmatter.slug
        ));
        if let Some(last) = &plan.frontmatter.last_advanced {
            output.push_str(&format!("  Last advanced: {last}\n"));
        }
        if !plan.frontmatter.linked_task_ids.is_empty() {
            output.push_str(&format!(
                "  Linked tasks: {}\n",
                plan.frontmatter.linked_task_ids.join(", ")
            ));
        }
    }

    ExecutionResult::success(output)
}

/// Show a single plan by slug.
pub fn execute_plan_show(slug: &str) -> ExecutionResult {
    let store = match PlanStore::open_project(None) {
        Ok(s) => s,
        Err(e) => return ExecutionResult::failure(format!("Failed to open plan store: {e}")),
    };

    match store.get(slug) {
        Some(plan) => {
            let mut output = format!(
                "# {}\n\nSlug: {}\nStatus: {}\n",
                plan.frontmatter.title, plan.frontmatter.slug, plan.frontmatter.status
            );
            if let Some(created) = &plan.frontmatter.created {
                output.push_str(&format!("Created: {created}\n"));
            }
            if let Some(last) = &plan.frontmatter.last_advanced {
                output.push_str(&format!("Last advanced: {last}\n"));
            }
            if !plan.frontmatter.linked_task_ids.is_empty() {
                output.push_str(&format!(
                    "Linked tasks: {}\n",
                    plan.frontmatter.linked_task_ids.join(", ")
                ));
            }
            if !plan.frontmatter.tags.is_empty() {
                output.push_str(&format!("Tags: {}\n", plan.frontmatter.tags.join(", ")));
            }
            output.push_str(&format!("\n---\n\n{}", plan.body));
            ExecutionResult::success(output)
        }
        None => ExecutionResult::failure(format!("Plan '{slug}' not found.")),
    }
}

/// Advance a plan with a progress summary.
pub fn execute_plan_advance(slug: &str, summary: &str) -> ExecutionResult {
    let store = match PlanStore::open_project(None) {
        Ok(s) => s,
        Err(e) => return ExecutionResult::failure(format!("Failed to open plan store: {e}")),
    };

    match store.advance(slug, summary) {
        Ok(plan) => ExecutionResult::success(format!(
            "Plan '{}' advanced.\nLast advanced: {}",
            plan.frontmatter.title,
            plan.frontmatter.last_advanced.unwrap_or_default()
        )),
        Err(e) => ExecutionResult::failure(format!("Failed to advance plan: {e}")),
    }
}

/// Archive a plan with an optional reason.
pub fn execute_plan_archive(slug: &str, reason: Option<&str>) -> ExecutionResult {
    let store = match PlanStore::open_project(None) {
        Ok(s) => s,
        Err(e) => return ExecutionResult::failure(format!("Failed to open plan store: {e}")),
    };

    let reason_text = reason.unwrap_or("Archived by user");
    match store.archive(slug, reason_text) {
        Ok(()) => ExecutionResult::success(format!("Plan '{slug}' archived: {reason_text}")),
        Err(e) => ExecutionResult::failure(format!("Failed to archive plan: {e}")),
    }
}

/// Materialize tasks from a plan's TODOs section.
pub fn execute_plan_materialize(slug: &str) -> ExecutionResult {
    let store = match PlanStore::open_project(None) {
        Ok(s) => s,
        Err(e) => return ExecutionResult::failure(format!("Failed to open plan store: {e}")),
    };

    let task_store = jfc_session::TaskStore::open_project(None);
    match store.materialize_tasks(slug, &task_store) {
        Ok(ids) => {
            if ids.is_empty() {
                ExecutionResult::success(format!(
                    "No new tasks to materialize from plan '{slug}' (all TODOs already linked or none found)."
                ))
            } else {
                ExecutionResult::success(format!(
                    "Materialized {} task(s) from plan '{slug}': {}",
                    ids.len(),
                    ids.join(", ")
                ))
            }
        }
        Err(e) => ExecutionResult::failure(format!("Failed to materialize tasks: {e}")),
    }
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    /// Override the current directory to a temp dir for testing.
    fn with_temp_project<F: FnOnce(&std::path::Path)>(f: F) {
        let dir = TempDir::new().unwrap();
        let plans_dir = dir.path().join(".jfc").join("plans");
        std::fs::create_dir_all(&plans_dir).unwrap();
        // We test using PlanStore::open_at directly since open_project(None)
        // uses cwd, which is hard to control in tests.
        f(dir.path());
    }

    #[test]
    fn plan_create_tool_normal() {
        let dir = TempDir::new().unwrap();
        let plans_dir = dir.path().join("plans");
        let store = PlanStore::open_at(&plans_dir).unwrap();

        let plan = store.create("Tool Test Plan", "Body content").unwrap();
        assert_eq!(plan.frontmatter.slug, "tool-test-plan");
        assert_eq!(plan.frontmatter.status, PlanStatus::Draft);

        // Verify file exists
        assert!(plans_dir.join("tool-test-plan.md").exists());
    }

    #[test]
    fn plan_list_tool_normal() {
        let dir = TempDir::new().unwrap();
        let plans_dir = dir.path().join("plans");
        let store = PlanStore::open_at(&plans_dir).unwrap();

        store.create("Plan A", "Body A").unwrap();
        store.create("Plan B", "Body B").unwrap();

        let all = store.list(None);
        assert_eq!(all.len(), 2);

        let drafts = store.list(Some(PlanStatus::Draft));
        assert_eq!(drafts.len(), 2);

        let active = store.list(Some(PlanStatus::Active));
        assert_eq!(active.len(), 0);
    }
}
