//! Persistent plan store for jfc — file-backed structured plans that track
//! initiatives, link to tasks, and evolve over time.
//!
//! Storage layout: `<project>/.jfc/plans/<slug>.md` with YAML frontmatter.
//!
//! Each plan is a markdown file with `---` delimited YAML frontmatter
//! containing metadata (title, status, linked tasks, etc.) and a body
//! that holds the plan content including a `## Progress Log` section.

use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use anyhow::{Context, Result, bail};
use chrono::Utc;
use serde::{Deserialize, Serialize};

use crate::atomic_write::write_atomic_sync;

// ─── Types ───────────────────────────────────────────────────────────────────

/// Status of a plan in its lifecycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PlanStatus {
    Draft,
    Active,
    Paused,
    Done,
    Archived,
}

impl fmt::Display for PlanStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Draft => write!(f, "draft"),
            Self::Active => write!(f, "active"),
            Self::Paused => write!(f, "paused"),
            Self::Done => write!(f, "done"),
            Self::Archived => write!(f, "archived"),
        }
    }
}

impl std::str::FromStr for PlanStatus {
    type Err = String;
    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s.to_lowercase().trim() {
            "draft" => Ok(Self::Draft),
            "active" => Ok(Self::Active),
            "paused" => Ok(Self::Paused),
            "done" => Ok(Self::Done),
            "archived" => Ok(Self::Archived),
            other => Err(format!("unknown plan status: {other}")),
        }
    }
}

/// YAML frontmatter for a plan file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanFrontmatter {
    pub slug: String,
    pub title: String,
    pub status: PlanStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_advanced: Option<String>,
    #[serde(default)]
    pub linked_task_ids: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub supersedes: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
}

/// A loaded plan (frontmatter + body).
#[derive(Debug, Clone)]
pub struct Plan {
    pub path: PathBuf,
    pub frontmatter: PlanFrontmatter,
    pub body: String,
}

/// Patch struct for partial updates.
#[derive(Debug, Clone, Default)]
pub struct PlanPatch {
    pub title: Option<String>,
    pub status: Option<PlanStatus>,
    pub tags: Option<Vec<String>>,
    pub body: Option<String>,
    pub supersedes: Option<String>,
    pub linked_task_ids: Option<Vec<String>>,
}

// ─── Slug generation ─────────────────────────────────────────────────────────

/// Generate a URL-safe slug from a title: lowercase, spaces→hyphens,
/// strip non-alphanumeric (except hyphens), collapse multiple hyphens,
/// truncate at 60 chars.
pub fn slugify(title: &str) -> String {
    let slug: String = title
        .to_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    // Collapse multiple hyphens
    let mut result = String::with_capacity(slug.len());
    let mut last_was_hyphen = false;
    for c in slug.chars() {
        if c == '-' {
            if !last_was_hyphen && !result.is_empty() {
                result.push('-');
            }
            last_was_hyphen = true;
        } else {
            result.push(c);
            last_was_hyphen = false;
        }
    }
    // Trim trailing hyphens
    let result = result.trim_end_matches('-').to_owned();
    // Truncate at 60 chars on a char boundary
    if result.len() > 60 {
        result[..result.floor_char_boundary(60)].to_owned()
    } else {
        result
    }
}

// ─── Frontmatter parsing ─────────────────────────────────────────────────────

/// Parse YAML frontmatter from a plan file. Uses `---` delimiters.
/// Returns (frontmatter, body) or an error if parsing fails.
fn parse_plan_file(content: &str) -> Result<(PlanFrontmatter, String)> {
    let content = content.trim_start_matches('\u{feff}'); // strip BOM
    if !content.starts_with("---") {
        bail!("plan file missing frontmatter delimiter");
    }
    let after_first = &content[3..];
    let end_idx = after_first
        .find("\n---")
        .context("plan file missing closing frontmatter delimiter")?;
    let yaml_str = &after_first[..end_idx];
    let body_start = end_idx + 4; // skip "\n---"
    let body = after_first[body_start..]
        .trim_start_matches('\n')
        .to_owned();

    let frontmatter: PlanFrontmatter =
        serde_yaml::from_str(yaml_str).context("failed to parse plan frontmatter YAML")?;
    Ok((frontmatter, body))
}

/// Serialize a plan to its file content (frontmatter + body).
fn serialize_plan(frontmatter: &PlanFrontmatter, body: &str) -> String {
    let yaml = serde_yaml::to_string(frontmatter).unwrap_or_default();
    format!("---\n{}---\n{}", yaml, body)
}

// ─── PlanStore ───────────────────────────────────────────────────────────────

/// Persistent plan store backed by `.jfc/plans/` directory.
pub struct PlanStore {
    root: PathBuf,
    index: Mutex<Vec<Plan>>,
}

impl PlanStore {
    /// Open (or create) the project plan store. Uses `<git_root>/.jfc/plans/`
    /// or falls back to `./.jfc/plans/`.
    pub fn open_project(git_root: Option<&Path>) -> Result<Arc<Self>> {
        let root = git_root
            .map(PathBuf::from)
            .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
        let plans_dir = root.join(".jfc").join("plans");
        std::fs::create_dir_all(&plans_dir)
            .with_context(|| format!("failed to create plans dir: {}", plans_dir.display()))?;

        let store = Self {
            root: plans_dir,
            index: Mutex::new(Vec::new()),
        };
        store.load_all();
        Ok(Arc::new(store))
    }

    /// Open a plan store at a specific directory (useful for testing).
    pub fn open_at(plans_dir: &Path) -> Result<Arc<Self>> {
        std::fs::create_dir_all(plans_dir)
            .with_context(|| format!("failed to create plans dir: {}", plans_dir.display()))?;
        let store = Self {
            root: plans_dir.to_path_buf(),
            index: Mutex::new(Vec::new()),
        };
        store.load_all();
        Ok(Arc::new(store))
    }

    /// Load all plan files from disk into the index.
    fn load_all(&self) {
        let mut plans = Vec::new();
        if let Ok(entries) = std::fs::read_dir(&self.root) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().and_then(|e| e.to_str()) == Some("md") {
                    if let Ok(plan) = Self::load_plan_file(&path) {
                        plans.push(plan);
                    }
                }
            }
        }
        if let Ok(mut idx) = self.index.lock() {
            *idx = plans;
        }
    }

    fn load_plan_file(path: &Path) -> Result<Plan> {
        let content = std::fs::read_to_string(path)
            .with_context(|| format!("failed to read plan: {}", path.display()))?;
        let (frontmatter, body) = parse_plan_file(&content)?;
        Ok(Plan {
            path: path.to_path_buf(),
            frontmatter,
            body,
        })
    }

    /// Create a new plan with the given title and body.
    pub fn create(&self, title: &str, body: &str) -> Result<Plan> {
        let slug = slugify(title);
        if slug.is_empty() {
            bail!("cannot create plan with empty title");
        }
        let path = self.root.join(format!("{slug}.md"));
        if path.exists() {
            bail!("plan with slug '{slug}' already exists");
        }

        let now = Utc::now().to_rfc3339();
        let frontmatter = PlanFrontmatter {
            slug: slug.clone(),
            title: title.to_owned(),
            status: PlanStatus::Draft,
            created: Some(now),
            last_advanced: None,
            linked_task_ids: Vec::new(),
            supersedes: None,
            tags: Vec::new(),
        };

        let content = serialize_plan(&frontmatter, body);
        write_atomic_sync(&path, content.as_bytes())
            .with_context(|| format!("failed to write plan: {}", path.display()))?;

        let plan = Plan {
            path: path.clone(),
            frontmatter,
            body: body.to_owned(),
        };

        if let Ok(mut idx) = self.index.lock() {
            idx.push(plan.clone());
        }

        Ok(plan)
    }

    /// Get a plan by slug.
    pub fn get(&self, slug: &str) -> Option<Plan> {
        let idx = self.index.lock().ok()?;
        idx.iter().find(|p| p.frontmatter.slug == slug).cloned()
    }

    /// List plans, optionally filtered by status.
    pub fn list(&self, filter: Option<PlanStatus>) -> Vec<Plan> {
        let idx = self.index.lock().unwrap_or_else(|e| e.into_inner());
        match filter {
            Some(status) => idx
                .iter()
                .filter(|p| p.frontmatter.status == status)
                .cloned()
                .collect(),
            None => idx.clone(),
        }
    }

    /// Update a plan with a partial patch.
    pub fn update(&self, slug: &str, patch: PlanPatch) -> Result<Plan> {
        let mut idx = self.index.lock().unwrap_or_else(|e| e.into_inner());
        let plan = idx
            .iter_mut()
            .find(|p| p.frontmatter.slug == slug)
            .context(format!("plan '{slug}' not found"))?;

        if let Some(title) = patch.title {
            plan.frontmatter.title = title;
        }
        if let Some(status) = patch.status {
            plan.frontmatter.status = status;
        }
        if let Some(tags) = patch.tags {
            plan.frontmatter.tags = tags;
        }
        if let Some(body) = patch.body {
            plan.body = body;
        }
        if let Some(supersedes) = patch.supersedes {
            plan.frontmatter.supersedes = Some(supersedes);
        }
        if let Some(linked) = patch.linked_task_ids {
            plan.frontmatter.linked_task_ids = linked;
        }

        let content = serialize_plan(&plan.frontmatter, &plan.body);
        write_atomic_sync(&plan.path, content.as_bytes())
            .with_context(|| format!("failed to write plan: {}", plan.path.display()))?;

        Ok(plan.clone())
    }

    /// Advance a plan: append to the Progress Log and update last_advanced.
    pub fn advance(&self, slug: &str, summary: &str) -> Result<Plan> {
        let mut idx = self.index.lock().unwrap_or_else(|e| e.into_inner());
        let plan = idx
            .iter_mut()
            .find(|p| p.frontmatter.slug == slug)
            .context(format!("plan '{slug}' not found"))?;

        let now = Utc::now().to_rfc3339();
        plan.frontmatter.last_advanced = Some(now.clone());

        // Append to ## Progress Log section (create if absent)
        let log_entry = format!("- {} — {}\n", now, summary);
        if let Some(pos) = plan.body.find("## Progress Log") {
            // Find the end of the heading line
            let after_heading = pos + "## Progress Log".len();
            let insert_pos = plan.body[after_heading..]
                .find('\n')
                .map(|i| after_heading + i + 1)
                .unwrap_or(plan.body.len());
            // Find where to insert (after existing entries or after newline)
            // We insert at the end of the section — find next ## or end
            let section_end = plan.body[insert_pos..]
                .find("\n## ")
                .map(|i| insert_pos + i)
                .unwrap_or(plan.body.len());
            // Ensure there's a newline before our entry
            let needs_newline = !plan.body[..section_end].ends_with('\n');
            let mut new_body = plan.body[..section_end].to_owned();
            if needs_newline {
                new_body.push('\n');
            }
            new_body.push_str(&log_entry);
            new_body.push_str(&plan.body[section_end..]);
            plan.body = new_body;
        } else {
            // Create the section at the end
            if !plan.body.ends_with('\n') {
                plan.body.push('\n');
            }
            plan.body.push_str("\n## Progress Log\n");
            plan.body.push_str(&log_entry);
        }

        let content = serialize_plan(&plan.frontmatter, &plan.body);
        write_atomic_sync(&plan.path, content.as_bytes())
            .with_context(|| format!("failed to write plan: {}", plan.path.display()))?;

        Ok(plan.clone())
    }

    /// Archive a plan with a reason.
    pub fn archive(&self, slug: &str, reason: &str) -> Result<()> {
        self.update(
            slug,
            PlanPatch {
                status: Some(PlanStatus::Archived),
                ..Default::default()
            },
        )?;
        // Append archive reason to progress log
        self.advance(slug, &format!("Archived: {reason}"))?;
        Ok(())
    }

    /// Reload plans from disk if any files have changed.
    /// Returns true if any changes were detected.
    pub fn reload_if_changed(&self) -> bool {
        let old_count = self.index.lock().map(|idx| idx.len()).unwrap_or(0);
        self.load_all();
        let new_count = self.index.lock().map(|idx| idx.len()).unwrap_or(0);
        old_count != new_count
    }

    /// Parse `## TODOs` section for `- [ ] N. <title>` lines and create
    /// tasks via TaskStore. Store IDs in `linked_task_ids`. Idempotent —
    /// skip already-linked tasks.
    pub fn materialize_tasks(
        &self,
        slug: &str,
        task_store: &jfc_session::TaskStore,
    ) -> Result<Vec<String>> {
        let plan = self.get(slug).context(format!("plan '{slug}' not found"))?;

        // Parse ## TODOs section
        let todos = parse_todo_items(&plan.body);
        if todos.is_empty() {
            return Ok(Vec::new());
        }

        let existing_ids: Vec<String> = plan.frontmatter.linked_task_ids.clone();
        let mut new_ids = Vec::new();

        for todo_title in &todos {
            // Check if this task title already exists in linked tasks
            let already_linked = existing_ids.iter().any(|id| {
                task_store
                    .get(id)
                    .map(|t| t.subject == *todo_title)
                    .unwrap_or(false)
            });
            if already_linked {
                continue;
            }

            // Create the task
            let blocked_by: Vec<String> = Vec::new();
            match task_store.create(
                todo_title.clone(),
                format!("From plan: {}", plan.frontmatter.title),
                None,
                blocked_by,
            ) {
                Ok(task) => {
                    new_ids.push(task.id.to_string());
                }
                Err(e) => {
                    tracing::warn!(
                        target: "jfc::plan",
                        error = %e,
                        title = %todo_title,
                        "failed to create task from plan TODO"
                    );
                }
            }
        }

        // Update linked_task_ids
        if !new_ids.is_empty() {
            let mut all_ids = existing_ids;
            all_ids.extend(new_ids.clone());
            self.update(
                slug,
                PlanPatch {
                    linked_task_ids: Some(all_ids),
                    ..Default::default()
                },
            )?;
        }

        Ok(new_ids)
    }

    /// Handle task completion: find plans with this task_id in linked_task_ids,
    /// call advance() with summary. If ALL linked tasks are done, flip status to Done.
    pub fn on_task_done(
        &self,
        task_id: &str,
        summary: &str,
        task_store: &jfc_session::TaskStore,
    ) -> Result<Vec<String>> {
        let idx = self.index.lock().unwrap_or_else(|e| e.into_inner());
        let matching_slugs: Vec<String> = idx
            .iter()
            .filter(|p| p.frontmatter.linked_task_ids.iter().any(|id| id == task_id))
            .map(|p| p.frontmatter.slug.clone())
            .collect();
        drop(idx);

        let mut advanced_slugs = Vec::new();
        for slug in &matching_slugs {
            self.advance(slug, summary)?;
            advanced_slugs.push(slug.clone());

            // Check if all linked tasks are done
            if let Some(plan) = self.get(slug) {
                let all_done = plan.frontmatter.linked_task_ids.iter().all(|id| {
                    task_store
                        .get(id)
                        .map(|t| t.status == jfc_session::TaskStatus::Completed)
                        .unwrap_or(false)
                });
                if all_done && !plan.frontmatter.linked_task_ids.is_empty() {
                    let _ = self.update(
                        slug,
                        PlanPatch {
                            status: Some(PlanStatus::Done),
                            ..Default::default()
                        },
                    );
                }
            }
        }

        Ok(advanced_slugs)
    }

    /// Get the root directory of this store.
    pub fn root(&self) -> &Path {
        &self.root
    }
}

// ─── TODO parsing ────────────────────────────────────────────────────────────

/// Parse `## TODOs` section for lines matching `- [ ] N. <title>`.
fn parse_todo_items(body: &str) -> Vec<String> {
    let mut in_todos = false;
    let mut items = Vec::new();

    for line in body.lines() {
        if line.starts_with("## TODOs") {
            in_todos = true;
            continue;
        }
        if in_todos && line.starts_with("## ") {
            break; // next section
        }
        if in_todos {
            // Match `- [ ] N. <title>` pattern
            let trimmed = line.trim();
            if let Some(rest) = trimmed.strip_prefix("- [ ] ") {
                // Check for `N. <title>` pattern
                if let Some(dot_pos) = rest.find(". ") {
                    let num_part = &rest[..dot_pos];
                    if num_part.chars().all(|c| c.is_ascii_digit()) {
                        let title = rest[dot_pos + 2..].trim().to_owned();
                        if !title.is_empty() {
                            items.push(title);
                        }
                    }
                }
            }
        }
    }

    items
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn setup_store() -> (TempDir, Arc<PlanStore>) {
        let dir = TempDir::new().unwrap();
        let plans_dir = dir.path().join("plans");
        let store = PlanStore::open_at(&plans_dir).unwrap();
        (dir, store)
    }

    #[test]
    fn plan_store_create_and_get_normal() {
        let (_dir, store) = setup_store();

        let plan = store
            .create(
                "My Test Plan",
                "This is the body.\n\n## TODOs\n- [ ] 1. Do something\n",
            )
            .unwrap();

        assert_eq!(plan.frontmatter.slug, "my-test-plan");
        assert_eq!(plan.frontmatter.title, "My Test Plan");
        assert_eq!(plan.frontmatter.status, PlanStatus::Draft);
        assert!(plan.frontmatter.created.is_some());
        assert_eq!(
            plan.body,
            "This is the body.\n\n## TODOs\n- [ ] 1. Do something\n"
        );

        // Get by slug
        let fetched = store.get("my-test-plan").unwrap();
        assert_eq!(fetched.frontmatter.title, "My Test Plan");
        assert_eq!(fetched.body, plan.body);

        // List
        let all = store.list(None);
        assert_eq!(all.len(), 1);

        let drafts = store.list(Some(PlanStatus::Draft));
        assert_eq!(drafts.len(), 1);

        let active = store.list(Some(PlanStatus::Active));
        assert_eq!(active.len(), 0);
    }

    #[test]
    fn plan_store_corrupt_frontmatter_robust() {
        let dir = TempDir::new().unwrap();
        let plans_dir = dir.path().join("plans");
        std::fs::create_dir_all(&plans_dir).unwrap();

        // Write a corrupt plan file
        let corrupt_path = plans_dir.join("corrupt.md");
        std::fs::write(&corrupt_path, "---\nthis is not: [valid yaml\n---\nbody").unwrap();

        // Write a valid plan file
        let valid_content = "---\nslug: valid\ntitle: Valid Plan\nstatus: active\n---\nBody here\n";
        std::fs::write(plans_dir.join("valid.md"), valid_content).unwrap();

        // Store should load the valid one and skip the corrupt one
        let store = PlanStore::open_at(&plans_dir).unwrap();
        let plans = store.list(None);
        assert_eq!(plans.len(), 1);
        assert_eq!(plans[0].frontmatter.slug, "valid");
    }

    #[test]
    fn plan_store_advance_appends_log_normal() {
        let (_dir, store) = setup_store();

        store.create("Advance Test", "Some content.\n").unwrap();

        // First advance — creates ## Progress Log
        let plan = store.advance("advance-test", "First progress").unwrap();
        assert!(plan.body.contains("## Progress Log"));
        assert!(plan.body.contains("First progress"));
        assert!(plan.frontmatter.last_advanced.is_some());

        // Second advance — appends to existing log
        let plan = store.advance("advance-test", "Second progress").unwrap();
        assert!(plan.body.contains("First progress"));
        assert!(plan.body.contains("Second progress"));

        // Verify the file on disk is consistent
        let reloaded = store.get("advance-test").unwrap();
        assert!(reloaded.body.contains("Second progress"));
    }

    #[test]
    fn plan_store_slugify_normal() {
        assert_eq!(slugify("Hello World"), "hello-world");
        assert_eq!(slugify("My Plan (v2)"), "my-plan-v2");
        assert_eq!(slugify("  spaces  "), "spaces");
        assert_eq!(slugify("UPPER CASE"), "upper-case");
        // Truncation
        let long_title = "a".repeat(100);
        let slug = slugify(&long_title);
        assert!(slug.len() <= 60);
    }

    #[test]
    fn materialize_creates_tasks_normal() {
        let (_dir, store) = setup_store();
        let task_store = jfc_session::TaskStore::in_memory();

        store
            .create(
                "Task Plan",
                "## TODOs\n- [ ] 1. First task\n- [ ] 2. Second task\n",
            )
            .unwrap();

        let ids = store.materialize_tasks("task-plan", &task_store).unwrap();
        assert_eq!(ids.len(), 2);

        // Verify tasks were created
        let tasks = task_store.list(jfc_session::DeletedFilter::Exclude);
        assert_eq!(tasks.len(), 2);
        assert!(tasks.iter().any(|t| t.subject == "First task"));
        assert!(tasks.iter().any(|t| t.subject == "Second task"));

        // Idempotent — running again should not create duplicates
        let ids2 = store.materialize_tasks("task-plan", &task_store).unwrap();
        assert_eq!(ids2.len(), 0);
        let tasks2 = task_store.list(jfc_session::DeletedFilter::Exclude);
        assert_eq!(tasks2.len(), 2);
    }

    #[test]
    fn on_task_done_advances_plan_normal() {
        let (_dir, store) = setup_store();
        let task_store = jfc_session::TaskStore::in_memory();

        store
            .create("Done Plan", "## TODOs\n- [ ] 1. Only task\n")
            .unwrap();

        let ids = store.materialize_tasks("done-plan", &task_store).unwrap();
        assert_eq!(ids.len(), 1);

        // Complete the task
        let task_id = &ids[0];
        task_store
            .update(
                task_id,
                jfc_session::TaskPatch {
                    status: Some(jfc_session::TaskStatus::Completed),
                    ..Default::default()
                },
            )
            .unwrap();

        let advanced = store
            .on_task_done(task_id, "Task completed successfully", &task_store)
            .unwrap();
        assert_eq!(advanced.len(), 1);
        assert_eq!(advanced[0], "done-plan");

        // Check plan was advanced
        let plan = store.get("done-plan").unwrap();
        assert!(plan.body.contains("Task completed successfully"));
    }

    #[test]
    fn all_tasks_done_flips_plan_to_done_normal() {
        let (_dir, store) = setup_store();
        let task_store = jfc_session::TaskStore::in_memory();

        store
            .create(
                "All Done Plan",
                "## TODOs\n- [ ] 1. Task A\n- [ ] 2. Task B\n",
            )
            .unwrap();

        let ids = store
            .materialize_tasks("all-done-plan", &task_store)
            .unwrap();
        assert_eq!(ids.len(), 2);

        // Complete first task
        task_store
            .update(
                &ids[0],
                jfc_session::TaskPatch {
                    status: Some(jfc_session::TaskStatus::Completed),
                    ..Default::default()
                },
            )
            .unwrap();
        store.on_task_done(&ids[0], "A done", &task_store).unwrap();

        // Plan should still be Draft (not all done yet)
        let plan = store.get("all-done-plan").unwrap();
        assert_ne!(plan.frontmatter.status, PlanStatus::Done);

        // Complete second task
        task_store
            .update(
                &ids[1],
                jfc_session::TaskPatch {
                    status: Some(jfc_session::TaskStatus::Completed),
                    ..Default::default()
                },
            )
            .unwrap();
        store.on_task_done(&ids[1], "B done", &task_store).unwrap();

        // Now plan should be Done
        let plan = store.get("all-done-plan").unwrap();
        assert_eq!(plan.frontmatter.status, PlanStatus::Done);
    }

    #[test]
    fn parse_todo_items_normal() {
        let body = "## TODOs\n- [ ] 1. First\n- [ ] 2. Second\n- [x] 3. Done\n## Other\n";
        let items = parse_todo_items(body);
        assert_eq!(items, vec!["First", "Second"]);
    }

    #[test]
    fn parse_todo_items_no_section_robust() {
        let body = "No todos section here.\n";
        let items = parse_todo_items(body);
        assert!(items.is_empty());
    }
}
