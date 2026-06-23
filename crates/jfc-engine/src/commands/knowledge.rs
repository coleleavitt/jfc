//! `/knowledge` — manage the unified cross-project knowledge store
//! (`jfc-knowledge`). All store access is synchronous SQLite, so each handler
//! runs it on a blocking thread.
//!
//! Subcommands: `import`, `mine`, `list`, `show <id>`, `forget <id>`,
//! `promote <id>` (the human cross-project gate), `review`, `gaps`,
//! `consolidate`, `gc-legacy --confirm` (archive — never deletes without
//! explicit confirmation).

use crate::commands::prelude::*;
use jfc_knowledge::{KnowledgeStore, RecallFilter, RelKind, Scope};

/// Render a help/usage message.
fn usage() -> String {
    "Usage: /knowledge <subcommand>\n\
     - import            import legacy .md memories into the store (idempotent)\n\
     - mine              mine your session history into project lessons (offline)\n\
     - list              show recent stored knowledge\n\
     - gaps              ranked 'what to learn next' (unresolved references)\n\
     - promote <id>      promote a lesson to cross-project (global) scope\n\
     - forget <id>       delete one record\n\
     - consolidate       dedup/forget pass (offline maintenance)\n\
     - status            row counts\n\
     - gc-legacy --confirm   archive (move, not delete) the old .md memory files"
        .to_owned()
}

pub(super) async fn handle_knowledge_command(state: &mut EngineState, arg: &str) {
    let mut parts = arg.split_whitespace();
    let sub = parts.next().unwrap_or("");
    let rest: Vec<String> = parts.map(str::to_owned).collect();
    let cwd = PathBuf::from(&state.cwd);

    let msg = match sub {
        "" | "help" => usage(),
        "status" => run_status(&cwd).await,
        "list" => run_list(&cwd).await,
        "gaps" => run_gaps(&cwd).await,
        "import" => run_import(&cwd).await,
        "mine" => run_mine(&cwd).await,
        "consolidate" => run_consolidate(&cwd).await,
        "promote" => run_promote(&cwd, rest.first().map(String::as_str)).await,
        "forget" => run_forget(&cwd, rest.first().map(String::as_str)).await,
        "gc-legacy" => run_gc_legacy(&cwd, rest.iter().any(|a| a == "--confirm")).await,
        other => format!("Unknown /knowledge subcommand `{other}`.\n\n{}", usage()),
    };
    state.messages.push(ChatMessage::assistant(msg));
}

async fn run_status(cwd: &std::path::Path) -> String {
    let cwd = cwd.to_path_buf();
    blocking(move || {
        let store = KnowledgeStore::open_default()?;
        let live = store.live_count()?;
        let project = jfc_knowledge::project_key(&cwd);
        let gaps = store.gaps(1000)?.len();
        Ok(format!(
            "Knowledge store: {live} live record(s); {gaps} open gap(s). This project's key: {project}.\n\
             DB: ~/.local/share/jfc/knowledge.db (delete it to fully reset)."
        ))
    })
    .await
}

async fn run_list(cwd: &std::path::Path) -> String {
    let cwd = cwd.to_path_buf();
    blocking(move || {
        let store = KnowledgeStore::open_default()?;
        let project = jfc_knowledge::project_key(&cwd);
        let hits = store.recall(
            "",
            &RecallFilter {
                project_key: Some(&project),
                limit: 20,
            },
        )?;
        if hits.is_empty() {
            return Ok("No knowledge stored yet. Try `/knowledge import` or `/knowledge mine`.".to_owned());
        }
        let mut out = String::from("Recent knowledge (top-ranked):\n");
        for h in hits {
            let v = if h.outcome == jfc_knowledge::Outcome::Verified { " ✓" } else { "" };
            out.push_str(&format!("- [{}] {} ({}){v}\n  id: {}\n", h.scope.slug(), h.title, h.kind.slug(), h.id));
        }
        Ok(out)
    })
    .await
}

async fn run_gaps(cwd: &std::path::Path) -> String {
    let cwd = cwd.to_path_buf();
    blocking(move || {
        let _ = cwd;
        let store = KnowledgeStore::open_default()?;
        let gaps = store.gaps(20)?;
        if gaps.is_empty() {
            return Ok("No open knowledge gaps.".to_owned());
        }
        let mut out = String::from("Knowledge gaps (what to learn next, by reference count):\n");
        for g in gaps {
            out.push_str(&format!("- ×{} {} — {}\n", g.ref_count, g.label, g.reason));
        }
        Ok(out)
    })
    .await
}

async fn run_import(cwd: &std::path::Path) -> String {
    let cwd = cwd.to_path_buf();
    blocking(move || {
        let store = KnowledgeStore::open_default()?;
        let project = jfc_knowledge::project_key(&cwd);
        let mut items = Vec::new();
        // User-level memories.
        if let Some(cfg) = dirs::config_dir() {
            let user_dir = cfg.join("jfc").join("memory");
            items.extend(jfc_knowledge::import::scan_markdown_dir(&user_dir, Scope::User, None));
        }
        // Project-level memories.
        let proj_dir = cwd.join(".jfc").join("memory");
        items.extend(jfc_knowledge::import::scan_markdown_dir(&proj_dir, Scope::Project, Some(project)));

        let report = store.import_memories(&items)?;
        Ok(format!(
            "Imported {} new memory record(s), skipped {} already present, {} error(s). \
             Source .md files were left untouched.",
            report.imported,
            report.skipped,
            report.errors.len()
        ))
    })
    .await
}

async fn run_mine(cwd: &std::path::Path) -> String {
    let cwd = cwd.to_path_buf();
    blocking(move || {
        let store = KnowledgeStore::open_default()?;
        let project = jfc_knowledge::project_key(&cwd);
        let Some(sessions_dir) = dirs::config_dir().map(|c| c.join("jfc").join("sessions")) else {
            return Ok("Could not locate ~/.config/jfc/sessions.".to_owned());
        };
        let (lessons, report) = jfc_knowledge::session_mine::mine_dir(&sessions_dir);
        let (inserted, compounded) = store.ingest_mined(&project, &lessons)?;
        Ok(format!(
            "Mined {} session(s): {} error-lesson(s) ({} verified) + {} preference(s). \
             Stored {} new, compounded {} existing — all project-scoped. \
             Use `/knowledge promote <id>` to share one across projects.",
            report.sessions_scanned,
            report.error_lessons,
            report.verified,
            report.preference_lessons,
            inserted,
            compounded
        ))
    })
    .await
}

async fn run_consolidate(cwd: &std::path::Path) -> String {
    let cwd = cwd.to_path_buf();
    blocking(move || {
        let _ = cwd;
        let mut store = KnowledgeStore::open_default()?;
        let superseded = store.consolidate()?;
        let removed = store.decay(jfc_knowledge::DEFAULT_MAX_AGE_MS, jfc_knowledge::DEFAULT_MAX_ROWS_PER_SCOPE)?;
        Ok(format!("Consolidated: {superseded} duplicate(s) superseded, {removed} stale row(s) pruned."))
    })
    .await
}

async fn run_promote(cwd: &std::path::Path, id: Option<&str>) -> String {
    let Some(id) = id.map(str::to_owned) else {
        return "Usage: /knowledge promote <id>  (see ids in /knowledge list)".to_owned();
    };
    let cwd = cwd.to_path_buf();
    blocking(move || {
        let _ = cwd;
        let store = KnowledgeStore::open_default()?;
        if store.promote(&id)? {
            Ok(format!("Promoted {id} to cross-project (global) scope. It will now be recalled in every project."))
        } else {
            Ok(format!("No live record with id {id} (already promoted, superseded, or unknown)."))
        }
    })
    .await
}

async fn run_forget(cwd: &std::path::Path, id: Option<&str>) -> String {
    let Some(id) = id.map(str::to_owned) else {
        return "Usage: /knowledge forget <id>".to_owned();
    };
    let cwd = cwd.to_path_buf();
    blocking(move || {
        let _ = cwd;
        let store = KnowledgeStore::open_default()?;
        let n = store.forget(&id)?;
        Ok(if n > 0 {
            format!("Forgot record {id}.")
        } else {
            format!("No record with id {id}.")
        })
    })
    .await
}

/// Archive — never deletes. Moves the legacy `.md` memory dirs to a timestamped
/// backup under the same parent, so the cutover is reversible. Requires
/// `--confirm`.
async fn run_gc_legacy(cwd: &std::path::Path, confirmed: bool) -> String {
    if !confirmed {
        return "This archives (moves, does not delete) your legacy .md memory files.\n\
                Re-run as `/knowledge gc-legacy --confirm` to proceed. The files are \
                moved to a timestamped `memory.archived-<ts>` dir and can be moved back."
            .to_owned();
    }
    let cwd = cwd.to_path_buf();
    blocking(move || {
        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let mut moved = Vec::new();
        let proj_mem = cwd.join(".jfc").join("memory");
        if proj_mem.is_dir() {
            let dest = cwd.join(".jfc").join(format!("memory.archived-{ts}"));
            std::fs::rename(&proj_mem, &dest)
                .map_err(jfc_knowledge::KnowledgeError::from)?;
            moved.push(dest.display().to_string());
        }
        if moved.is_empty() {
            Ok("No legacy project .md memory dir to archive.".to_owned())
        } else {
            Ok(format!("Archived (moved, not deleted): {}. Move it back to restore.", moved.join(", ")))
        }
    })
    .await
}

/// Run a blocking store closure and format any error into the reply.
async fn blocking<F>(f: F) -> String
where
    F: FnOnce() -> jfc_knowledge::Result<String> + Send + 'static,
{
    match tokio::task::spawn_blocking(f).await {
        Ok(Ok(msg)) => msg,
        Ok(Err(e)) => format!("Knowledge store error: {e}"),
        Err(e) => format!("Knowledge task failed: {e}"),
    }
}

/// Link two records (used by mining/consolidation hooks; exposed for tests).
#[allow(dead_code)]
pub(crate) fn link_records(
    store: &KnowledgeStore,
    from: &str,
    to: &str,
    rel: RelKind,
) -> jfc_knowledge::Result<()> {
    store.link(from, to, rel)
}
