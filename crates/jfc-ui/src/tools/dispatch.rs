/// Main tool dispatcher for jfc.
/// Contains the `execute_tool` async fn and all its match arms.
/// State helpers live in `registry`; synchronous helpers in `safe_tools`.
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::Mutex;

use crate::context::ReadDedupCache;
use crate::runtime::ExecutionResult;
use crate::types::{ToolInput, ToolKind};
use jfc_session::TaskStore;

use super::bash::{execute_bash_output, execute_bash_with_options};
use super::daemon::{
    execute_cron_create, execute_cron_delete, execute_cron_list, execute_monitor,
    execute_schedule_wakeup,
};
use super::design::{
    execute_design_bundle_html, execute_design_capabilities, execute_design_check_system,
    execute_design_copy_file, execute_design_delete_file, execute_design_handoff,
    execute_design_list_files, execute_design_project_create, execute_design_project_list,
    execute_design_project_set_meta, execute_design_read_file, execute_design_register_asset,
    execute_design_serve, execute_design_unregister_asset, execute_design_write_file,
};
use super::dispatch_heavy;
use super::economy::strip_html_tags;
use super::filesystem::{execute_edit, execute_read, execute_write};
use super::lsp::execute_lsp;
use super::memory::{execute_memory_create, execute_memory_delete};
use super::notebook::{execute_notebook_edit, execute_notebook_read};
use super::notifications::{execute_push_notification, execute_remote_trigger};
use super::scratchpad::{execute_scratchpad_read, execute_scratchpad_write};
use super::search::{execute_glob, execute_grep};
use super::swarm::{
    execute_send_message, execute_team_create, execute_team_delete, execute_team_member_mode,
};
use super::tasks::{
    TaskCreateRequest, TaskUpdateRequest, execute_skill, execute_task_create, execute_task_done,
    execute_task_get, execute_task_list, execute_task_stop, execute_task_update,
    execute_task_validate,
};
use super::worktree::{execute_enter_plan_mode, execute_enter_worktree, execute_exit_worktree};

use super::registry::{
    collusion_detector, invalidate_graph_session_cache, market_orchestrator, record_edited_file,
    snapshot_event_sender, snapshot_mcp_registry,
};
#[cfg(feature = "permission-automation")]
use super::safe_tools::tool_permission_path;
use super::safe_tools::{
    execute_code_index, execute_tool_search, execute_tool_suggest, maybe_run_slop_guard,
};

pub(super) fn resolve_bash_workdir(cwd: &Path, workdir: Option<&str>) -> PathBuf {
    let Some(workdir) = workdir.map(str::trim).filter(|workdir| !workdir.is_empty()) else {
        return cwd.to_path_buf();
    };
    let path = Path::new(workdir);
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        cwd.join(path)
    }
}

#[tracing::instrument(target = "jfc::tools", skip(input, cwd, dedup, task_store), fields(kind = ?kind))]
pub async fn execute_tool(
    kind: ToolKind,
    input: ToolInput,
    cwd: std::path::PathBuf,
    dedup: Option<Arc<Mutex<ReadDedupCache>>>,
    task_store: Option<Arc<TaskStore>>,
    active_team_name: Option<&str>,
) -> ExecutionResult {
    #[cfg(feature = "hooks")]
    {
        // Hook integration point: BeforeToolDispatch
        // When fully wired, this will:
        // 1. Build HookContext from tool name + input
        // 2. Fire BeforeToolDispatch hooks
        // 3. If Abort → return error
        // 4. If Skip → return empty result
        // 5. If Replace → use replacement input
        tracing::trace!(target: "jfc::hooks", "hook integration point: BeforeToolDispatch");
    }

    #[cfg(feature = "permission-automation")]
    {
        use crate::permissions::{PermissionAction, check_tool_permission};

        let config = crate::config::feature_config::FeatureConfig::load(&cwd);
        let rules = crate::permissions::RuleSet::from_config(&config);
        let decision = check_tool_permission(&rules, kind.api_name(), tool_permission_path(&input));

        if matches!(decision.action, PermissionAction::Deny) {
            let reason = decision
                .reason
                .as_deref()
                .unwrap_or("permission rule denied tool invocation");
            return ExecutionResult::failure(format!(
                "Permission denied for {}: {reason}",
                kind.api_name()
            ));
        }
    }

    // Audit ledger: record mutating tool calls as immutable facts so "what did
    // this agent do, when" is answerable. The mutating/read-only classification
    // is derived from the unified CommandSpec metadata (one source), not a
    // hand-maintained match here. Read-only tools are skipped to keep ledger
    // volume meaningful. Offloaded to a blocking thread so the locked
    // line-append never stalls dispatch.
    if crate::command_spec::tool_is_mutating(kind.clone()) {
        let tool = kind.api_name().to_string();
        let detail = crate::changeset::ledger_detail_for(&kind, &input);
        tokio::task::spawn_blocking(move || {
            crate::changeset::record_tool_call(&tool, detail, None, None);
        });
    }

    // For task tools in team mode, prefer the caller-supplied store if
    // one was passed (the UI keeps `app.task_store` pointing at the
    // team's `tasks.json` once team mode is active, and the event-loop
    // migration runs at TeammateSpawned). Only fall back to
    // `TaskStore::open_team` when the caller didn't thread a store —
    // e.g. the swarm runner's tool path. Without this guard, every
    // concurrent task tool would `open_team` its own fresh
    // `Arc<TaskStore>`, each with its own private `Mutex<TaskStoreInner>`,
    // and last-write-wins on `tasks.json` would silently drop sibling
    // task creates — the "unknown task id t35..t46" symptom.
    let task_store = match (active_team_name, &kind, task_store.clone()) {
        (
            Some(team_name),
            ToolKind::TaskCreate
            | ToolKind::TaskUpdate
            | ToolKind::TaskList
            | ToolKind::TaskDone
            | ToolKind::TaskStop
            | ToolKind::TaskGet,
            None,
        ) => Some(TaskStore::open_team(team_name)),
        (_, _, Some(store)) => Some(store),
        _ => task_store,
    };

    match (kind, input) {
        (
            ToolKind::Bash,
            ToolInput::Bash {
                command,
                timeout,
                workdir,
                run_in_background,
            },
        ) => {
            let effective_cwd = resolve_bash_workdir(&cwd, workdir.as_deref());
            execute_bash_with_options(
                &command,
                timeout,
                &effective_cwd,
                None,
                run_in_background.unwrap_or(false),
            )
            .await
        }
        (
            ToolKind::BashOutput,
            ToolInput::BashOutput {
                task_id,
                offset,
                limit,
            },
        ) => execute_bash_output(&task_id, offset, limit).await,
        (
            ToolKind::Read,
            ToolInput::Read {
                file_path,
                offset,
                limit,
            },
        ) => execute_read(&file_path, offset, limit, dedup.as_ref()).await,
        (ToolKind::Write, ToolInput::Write { file_path, content }) => {
            // Speculation: route Write through overlay when speculating.
            let target_path = match crate::speculation::overlay_path_for(Path::new(&file_path)) {
                Some(overlay) => {
                    if let Some(parent) = overlay.parent() {
                        let _ = std::fs::create_dir_all(parent);
                    }
                    overlay.display().to_string()
                }
                None => file_path.clone(),
            };
            let result = execute_write(&target_path, &content).await;
            if !result.is_error() {
                if let Some(cache) = &dedup {
                    cache.lock().await.invalidate(Path::new(&file_path));
                }
                // Drop the cached graph for this workspace so the next
                // graph_query reflects the new file content.
                invalidate_graph_session_cache(Some(&cwd));
                record_edited_file(Path::new(&file_path));
                // Slop guard: check the written content for quality issues.
                return maybe_run_slop_guard(result, Path::new(&file_path), &content, &cwd).await;
            }
            result
        }
        (
            ToolKind::Edit,
            ToolInput::Edit {
                file_path,
                old_string,
                new_string,
                replacement,
            },
        ) => {
            let result = execute_edit(&file_path, &old_string, &new_string, replacement).await;
            if !result.is_error() {
                if let Some(cache) = &dedup {
                    cache.lock().await.invalidate(Path::new(&file_path));
                }
                invalidate_graph_session_cache(Some(&cwd));
                record_edited_file(Path::new(&file_path));
                // Slop guard: read the post-edit content and check for quality issues.
                let post_content = tokio::fs::read_to_string(&file_path)
                    .await
                    .unwrap_or_default();
                return maybe_run_slop_guard(result, Path::new(&file_path), &post_content, &cwd)
                    .await;
            }
            result
        }
        (ToolKind::Glob, ToolInput::Glob { pattern, path }) => {
            execute_glob(&pattern, path.as_deref(), &cwd).await
        }
        (
            ToolKind::Grep,
            ToolInput::Grep {
                pattern,
                path,
                glob,
                output_mode,
            },
        ) => {
            execute_grep(
                &pattern,
                path.as_deref(),
                glob.as_deref(),
                output_mode.as_deref(),
                &cwd,
            )
            .await
        }
        (
            ToolKind::TaskCreate,
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
                tags,
                priority,
                effort,
                model,
            },
        ) => execute_task_create(
            task_store,
            TaskCreateRequest {
                subject,
                description,
                active_form,
                blocked_by,
                acceptance_criteria,
                verification_command,
                risk,
                parent_id,
                kind,
                tags,
                priority,
                effort,
                model,
            },
        ),
        (
            ToolKind::TaskUpdate,
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
                blocked_by,
                tags,
                priority,
                effort,
                model,
            },
        ) => execute_task_update(
            task_store,
            TaskUpdateRequest {
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
                blocked_by,
                tags,
                priority,
                effort,
                model,
            },
        ),
        (
            ToolKind::TaskList,
            ToolInput::TaskList {
                status_filter,
                owner_filter,
                include_history,
                history_query,
            },
        ) => execute_task_list(
            task_store,
            status_filter.as_deref(),
            owner_filter.as_deref(),
            include_history.unwrap_or(false),
            history_query.as_deref(),
        ),
        (ToolKind::TaskDone, ToolInput::TaskDone { task_id }) => {
            let task_store_for_task = task_store.clone();
            match tokio::task::spawn_blocking(move || {
                let result = execute_task_done(task_store_for_task.clone(), &task_id);
                // Plan↔task reverse linkage: when a task that was materialized
                // from a plan is marked done, advance the linked plan (and flip
                // it to Done when every linked task is complete).
                if !result.is_error()
                    && let Some(store) = task_store_for_task.as_ref()
                {
                    advance_linked_plans(store, &task_id);
                }
                result
            })
            .await
            {
                Ok(result) => result,
                Err(err) => ExecutionResult::failure(format!("TaskDone worker panicked: {err}")),
            }
        }
        (ToolKind::TaskStop, ToolInput::TaskStop { task_id }) => execute_task_stop("", &task_id),
        (ToolKind::TaskGet, ToolInput::TaskGet { task_id }) => {
            execute_task_get(task_store, &task_id)
        }
        (ToolKind::TaskValidate, ToolInput::TaskValidate) => execute_task_validate(task_store),
        (ToolKind::Task, ToolInput::Task(_)) => {
            ExecutionResult::failure("Task tool must be dispatched via the streaming executor")
        }
        (ToolKind::Workflow, ToolInput::Workflow { .. }) => ExecutionResult::failure(
            "Workflow tool must be dispatched via the streaming executor (background task)",
        ),
        (ToolKind::Skill, ToolInput::Skill { name, args }) => {
            execute_skill(&name, args.as_deref()).await
        }
        (ToolKind::ToolSearch, ToolInput::ToolSearch { query, limit }) => {
            execute_tool_search(&query, limit, &cwd).await
        }
        (ToolKind::ToolSuggest, ToolInput::ToolSuggest { intent, limit }) => {
            execute_tool_suggest(&intent, limit, &cwd).await
        }
        (
            ToolKind::MemoryCreate,
            ToolInput::MemoryCreate {
                level,
                memory_type,
                scope,
                body,
            },
        ) => execute_memory_create(&level, &memory_type, &scope, &body, &cwd),
        (ToolKind::MemoryDelete, ToolInput::MemoryDelete { path }) => execute_memory_delete(&path),
        (
            ToolKind::TeamCreate,
            ToolInput::TeamCreate {
                team_name,
                description,
            },
        ) => execute_team_create(&team_name, description.as_deref(), &cwd).await,
        (ToolKind::TeamDelete, ToolInput::TeamDelete) => {
            execute_team_delete(active_team_name).await
        }
        (
            ToolKind::SendMessage,
            ToolInput::SendMessage {
                to,
                message,
                summary,
            },
        ) => execute_send_message(&to, &message, summary.as_deref(), active_team_name).await,
        (ToolKind::TeamMemberMode, ToolInput::TeamMemberMode { member_name, mode }) => {
            execute_team_member_mode(&member_name, &mode, active_team_name).await
        }
        (
            ToolKind::CodeIndex,
            ToolInput::CodeIndex {
                path,
                query,
                kind,
                max_entries,
            },
        ) => execute_code_index(
            &cwd,
            path.as_deref(),
            query.as_deref(),
            kind.as_deref(),
            max_entries,
        ),
        (
            ToolKind::GraphQuery,
            ToolInput::GraphQuery {
                query,
                max_tokens,
                include_handles,
                format,
            },
        ) => dispatch_heavy::execute_graph_query(
            query,
            max_tokens,
            include_handles,
            format.as_deref(),
            &cwd,
        ),
        (
            ToolKind::GraphContext,
            ToolInput::GraphContext {
                task,
                max_nodes,
                include_code,
                format,
            },
        ) => dispatch_heavy::execute_graph_context(
            task,
            max_nodes,
            include_code,
            format.as_deref(),
            &cwd,
        ),
        (
            ToolKind::GraphSearch,
            ToolInput::GraphSearch {
                query,
                limit,
                include_code,
                format,
            },
        ) => dispatch_heavy::execute_graph_search(
            query,
            limit,
            include_code,
            format.as_deref(),
            &cwd,
        ),
        (
            ToolKind::GraphCallers,
            ToolInput::GraphCallers {
                symbol,
                limit,
                format,
            },
        ) => dispatch_heavy::execute_graph_callers(symbol, limit, format.as_deref(), &cwd),
        (
            ToolKind::GraphCallees,
            ToolInput::GraphCallees {
                symbol,
                limit,
                format,
            },
        ) => dispatch_heavy::execute_graph_callees(symbol, limit, format.as_deref(), &cwd),
        (
            ToolKind::GraphImpact,
            ToolInput::GraphImpact {
                symbol,
                depth,
                format,
            },
        ) => dispatch_heavy::execute_graph_impact(symbol, depth, format.as_deref(), &cwd),
        (
            ToolKind::GraphNode,
            ToolInput::GraphNode {
                symbol,
                include_code,
            },
        ) => dispatch_heavy::execute_graph_node(symbol, include_code, &cwd),
        (ToolKind::GraphExplore, ToolInput::GraphExplore { query, max_files }) => {
            dispatch_heavy::execute_graph_explore(query, max_files, &cwd)
        }
        (ToolKind::GraphOutline, ToolInput::GraphOutline { file }) => {
            dispatch_heavy::execute_graph_outline(file, &cwd)
        }
        (
            ToolKind::GraphGrep,
            ToolInput::GraphGrep {
                pattern,
                glob,
                limit,
            },
        ) => dispatch_heavy::execute_graph_grep(pattern, glob.as_deref(), limit, &cwd),
        (ToolKind::GraphStatus, ToolInput::GraphStatus {}) => {
            dispatch_heavy::execute_graph_status(&cwd)
        }
        (ToolKind::GraphFiles, ToolInput::GraphFiles { path }) => {
            dispatch_heavy::execute_graph_files(path.as_deref(), &cwd)
        }
        (
            ToolKind::GetProgramSlice,
            ToolInput::GetProgramSlice {
                symbol,
                backward,
                max_nodes,
            },
        ) => dispatch_heavy::execute_get_program_slice(symbol, backward, max_nodes, &cwd),
        (ToolKind::GetDataDependencies, ToolInput::GetDataDependencies { symbol, max_nodes }) => {
            dispatch_heavy::execute_get_data_dependencies(symbol, max_nodes, &cwd)
        }
        (
            ToolKind::TaintFlow,
            ToolInput::TaintFlow {
                sources,
                sinks,
                sanitizers,
                max_paths,
            },
        ) => dispatch_heavy::execute_taint_flow(sources, sinks, sanitizers, max_paths, &cwd),
        (ToolKind::PlanCreate, ToolInput::PlanCreate { title, body }) => {
            crate::tools::plans::execute_plan_create(&title, body.as_deref())
        }
        (ToolKind::PlanList, ToolInput::PlanList { status }) => {
            crate::tools::plans::execute_plan_list(status.as_deref())
        }
        (ToolKind::PlanShow, ToolInput::PlanShow { slug }) => {
            crate::tools::plans::execute_plan_show(&slug)
        }
        (ToolKind::PlanAdvance, ToolInput::PlanAdvance { slug, summary }) => {
            crate::tools::plans::execute_plan_advance(&slug, &summary)
        }
        (ToolKind::PlanArchive, ToolInput::PlanArchive { slug, reason }) => {
            crate::tools::plans::execute_plan_archive(&slug, reason.as_deref())
        }
        (ToolKind::PlanMaterialize, ToolInput::PlanMaterialize { slug }) => {
            crate::tools::plans::execute_plan_materialize(&slug)
        }
        (ToolKind::LearnStatus, ToolInput::LearnStatus {}) => {
            crate::tools::learn::execute_learn_status()
        }
        (ToolKind::LearnHistorize, ToolInput::LearnHistorize {}) => {
            crate::tools::learn::execute_learn_historize()
        }
        (ToolKind::LearnDream, ToolInput::LearnDream {}) => {
            crate::tools::learn::execute_learn_dream()
        }
        (ToolKind::LearnKeyFilesList, ToolInput::LearnKeyFilesList {}) => {
            crate::tools::learn::execute_learn_key_files_list(std::path::Path::new(&cwd))
        }
        (ToolKind::LearnUserProfileShow, ToolInput::LearnUserProfileShow {}) => {
            crate::tools::learn::execute_learn_user_profile_show()
        }
        (
            ToolKind::RunCoverage,
            ToolInput::RunCoverage {
                lcov_path,
                include_untested_list,
            },
        ) => dispatch_heavy::execute_run_coverage(lcov_path, include_untested_list, &cwd),
        (
            ToolKind::SymbolEdit,
            ToolInput::SymbolEdit {
                handle,
                new_content,
                validate,
                dispatch_cascade,
            },
        ) => {
            dispatch_heavy::execute_symbol_edit(
                handle,
                new_content,
                validate,
                dispatch_cascade,
                &cwd,
                task_store,
            )
            .await
        }
        (
            ToolKind::PostBounty,
            ToolInput::PostBounty {
                description,
                budget,
                acceptance_criteria,
                max_solvers,
                auto_dispatch,
            },
        ) => {
            dispatch_heavy::execute_post_bounty(
                description,
                budget,
                acceptance_criteria,
                max_solvers,
                auto_dispatch,
                &cwd,
            )
            .await
        }
        (
            ToolKind::RunBounty,
            ToolInput::RunBounty {
                bounty_id,
                max_solvers,
            },
        ) => dispatch_heavy::execute_run_bounty(bounty_id, max_solvers, &cwd).await,
        (ToolKind::MarketStatus, ToolInput::MarketStatus { bounty_id }) => {
            // Try-lock: a bounty cycle in flight holds this mutex for
            // minutes. Returning a "busy" message lets the model
            // continue and retry instead of stalling its turn on the
            // orchestrator. See `economy::market_report_string` for the
            // same pattern.
            let Ok(orch) = market_orchestrator().try_lock() else {
                return ExecutionResult::success(
                    "Market is busy executing a bounty cycle. \
                     Re-run market_status once the cycle completes.",
                );
            };
            let detector = match collusion_detector().lock() {
                Ok(g) => g,
                Err(e) => {
                    return ExecutionResult::failure(format!(
                        "collusion detector mutex poisoned: {e}"
                    ));
                }
            };
            let report = jfc_economy::reporting::MarketReport::generate(&orch, &detector, 0, 0);
            let critical = report.health.is_critical();
            let mut body = format!(
                "Market: {} bounties total ({} active) · spent {} / remaining {} tok\n\
                 Health: composite={:.2} (eff={:.2}, fair={:.2}, trust={:.2}, budget={:.2})",
                report.total_bounties,
                report.active_bounties,
                report.total_spent,
                report.remaining_budget,
                report.health.composite,
                report.health.efficiency,
                report.health.fairness,
                report.health.trust,
                report.health.budget_adherence,
            );
            if critical {
                body.push_str(" [CRITICAL]");
            }
            if !report.flagged_agents.is_empty() {
                body.push_str("\nFlagged agents:");
                for f in &report.flagged_agents {
                    body.push_str(&format!("\n  - {f}"));
                }
            }
            if let Some(id) = bounty_id
                && let Some(state) = orch.bounty_state(&id)
            {
                body.push_str(&format!("\nBounty `{id}` state: {state:?}"));
                if matches!(state, jfc_economy::types::MarketState::Open) {
                    body.push_str(" — call run_bounty to drive Solve→Validate→Settle.");
                }
            }
            ExecutionResult::success(body)
        }
        (ToolKind::MultiEdit, ToolInput::MultiEdit { file_path, edits }) => {
            // Serialize on the same per-file lock used by Edit/Write so
            // MultiEdit and parallel Edit calls don't race on the same file.
            let _guard_lock = crate::tools::filesystem::acquire_file_lock(&file_path).await;
            let _guard = _guard_lock.lock().await;
            // Apply each edit in order. Each edit sees the previous
            // edit's output, so later edits can reference text that
            // earlier edits introduced. Bails on the first edit that
            // doesn't match — partial application would leave the
            // file in a half-edited state the model has to recover
            // from. Same contract as v132.
            let path = std::path::PathBuf::from(&file_path);
            let mut content = match tokio::fs::read_to_string(&path).await {
                Ok(s) => s,
                Err(e) => {
                    return ExecutionResult::failure(format!(
                        "MultiEdit: cannot read {file_path}: {e}"
                    ));
                }
            };
            let edit_array =
                match edits.as_array() {
                    Some(a) => a,
                    None => return ExecutionResult::failure(
                        "MultiEdit: `edits` must be an array of {old_string, new_string} objects"
                            .to_string(),
                    ),
                };
            let mut applied = 0usize;
            for (i, edit) in edit_array.iter().enumerate() {
                let old = edit
                    .get("old_string")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let new_s = edit
                    .get("new_string")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let replace_all = edit
                    .get("replace_all")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                if old.is_empty() {
                    return ExecutionResult::failure(format!(
                        "MultiEdit: edit {} has empty old_string",
                        i + 1
                    ));
                }
                if !content.contains(old) {
                    return ExecutionResult::failure(format!(
                        "MultiEdit: edit {} of {} — old_string not found. \
                         Earlier edits applied: {applied}. \
                         Read the file and retry with the current contents.",
                        i + 1,
                        edit_array.len()
                    ));
                }
                content = if replace_all {
                    content.replace(old, new_s)
                } else {
                    let occurrences = content.matches(old).count();
                    if occurrences > 1 {
                        return ExecutionResult::failure(format!(
                            "MultiEdit: edit {} matched {occurrences} times — \
                             pass `replace_all: true` or include more context to disambiguate.",
                            i + 1
                        ));
                    }
                    content.replacen(old, new_s, 1)
                };
                applied += 1;
            }
            if let Err(e) = tokio::fs::write(&path, &content).await {
                return ExecutionResult::failure(format!("MultiEdit: write {file_path}: {e}"));
            }
            tracing::info!(
                target: "jfc::tools::multi_edit",
                file_path = %file_path,
                applied,
                bytes = content.len(),
                "MultiEdit applied"
            );
            invalidate_graph_session_cache(Some(&cwd));
            record_edited_file(Path::new(&file_path));
            let result =
                ExecutionResult::success(format!("Applied {applied} edits to {file_path}."));
            // Slop guard: check the final content for quality issues.
            maybe_run_slop_guard(result, Path::new(&file_path), &content, &cwd).await
        }
        (ToolKind::AskUserQuestion, ToolInput::AskUserQuestion { questions }) => {
            // FALLBACK PATH ONLY. The normal route diverts AskUserQuestion
            // into the interactive modal in `handle_stream_tool` (see
            // `app.pending_question` / `input/question.rs`) before it ever
            // reaches dispatch, so this arm is effectively unreachable. It
            // remains as a defensive degrade-to-text path in case a future
            // code path dispatches the tool directly: surface the prompt(s) as
            // a transcript entry and treat the user's next message as the answer.
            let mut blocks: Vec<String> = Vec::new();
            for q in questions.as_array().into_iter().flatten() {
                let prompt = q.get("question").and_then(|v| v.as_str()).unwrap_or("");
                let multi = q
                    .get("multiSelect")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                let opts: Vec<String> = q
                    .get("options")
                    .and_then(|v| v.as_array())
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|opt| {
                                let label = opt.get("label").and_then(|v| v.as_str())?;
                                let desc = opt
                                    .get("description")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("");
                                if desc.is_empty() {
                                    Some(format!("- {label}"))
                                } else {
                                    Some(format!("- {label} — {desc}"))
                                }
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                blocks.push(format!(
                    "**Question for you:** {prompt}\n\n{}\n\n_(Reply with your choice{} as your next message.)_",
                    opts.join("\n"),
                    if multi { "(s)" } else { "" }
                ));
            }
            let body = blocks.join("\n\n");
            tracing::info!(
                target: "jfc::tools::ask",
                question_count = questions.as_array().map(|a| a.len()).unwrap_or(0),
                "AskUserQuestion surfaced (fallback text path)"
            );
            ExecutionResult::success(format!(
                "{body}\n\n(The user's next message is your tool result.)"
            ))
        }
        (ToolKind::WebFetch, ToolInput::WebFetch { url, prompt }) => {
            // v132 caches WebFetch results per-URL with a 15-minute TTL so
            // the model can iterate on a document it just fetched without
            // re-downloading. Cache HIT returns immediately with a
            // `<system-reminder>` flag so the model knows the body is from
            // a previous fetch (matters if the URL was a live endpoint).
            if let Some(cached) = crate::web_cache::get(&url) {
                let prompt_hint = prompt
                    .as_ref()
                    .map(|p| format!("Focus: {p}\n\n"))
                    .unwrap_or_default();
                tracing::debug!(
                    target: "jfc::tools::webfetch",
                    %url,
                    cached_bytes = cached.len(),
                    "WebFetch cache HIT"
                );
                return ExecutionResult::success(format!(
                    "{}\n\nGET {url} → 200 (cached)\n\n{prompt_hint}{cached}",
                    crate::system_reminder::format(
                        "WebFetch result served from cache (last fetch <15min ago). \
                         If you need fresh content, re-issue with a cache-busting query \
                         parameter."
                    ),
                ));
            }

            // Use reqwest with a short timeout. Strips HTML to text
            // when content-type indicates HTML; otherwise returns
            // the body as-is. The optional `prompt` is *not* applied
            // here (we don't run a second LLM pass) — it's surfaced
            // verbatim in the tool result so the model sees its own
            // intent and can summarize during the next turn.
            let client = match reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(15))
                .user_agent("jfc/0.1 (https://github.com/coleleavitt/jfc)")
                .build()
            {
                Ok(c) => c,
                Err(e) => return ExecutionResult::failure(format!("WebFetch: client init: {e}")),
            };
            let resp = match client.get(&url).send().await {
                Ok(r) => r,
                Err(e) => return ExecutionResult::failure(format!("WebFetch: {url}: {e}")),
            };
            let status = resp.status();
            let content_type = resp
                .headers()
                .get("content-type")
                .and_then(|v| v.to_str().ok())
                .unwrap_or("")
                .to_owned();
            let body = resp.text().await.unwrap_or_default();
            let body = if content_type.contains("html") {
                // Cheap HTML→text: strip tags. A real impl would use
                // scraper/html5ever; this is an MVP.
                strip_html_tags(&body)
            } else {
                body
            };
            // Cap to ~50 KB so the tool result doesn't blow context.
            let truncated = if body.len() > 50_000 {
                // Find a char boundary at or before 50_000 to avoid panicking
                // on multi-byte UTF-8 sequences.
                let end = body.floor_char_boundary(50_000);
                format!(
                    "{}\n\n[...truncated, full {} bytes]",
                    &body[..end],
                    body.len()
                )
            } else {
                body
            };
            // Cache successful 2xx responses only — caching errors would
            // mask transient outages on retry.
            if status.is_success() {
                crate::web_cache::put(&url, truncated.clone());
            }
            let prompt_hint = prompt
                .as_ref()
                .map(|p| format!("Focus: {p}\n\n"))
                .unwrap_or_default();
            ExecutionResult::success(format!("GET {url} → {status}\n\n{prompt_hint}{truncated}"))
        }
        (ToolKind::WebSearch, ToolInput::WebSearch { query, max_results }) => {
            let num = max_results.unwrap_or(5) as usize;
            match crate::web_search::search(&query, num).await {
                Ok(results) => ExecutionResult::success(results),
                Err(e) => ExecutionResult::failure(e),
            }
        }
        (ToolKind::ExitPlanMode, ToolInput::ExitPlanMode { plan }) => {
            // Ultraplan: if there's an active ultraplan session, complete it
            // and teleport the plan back to the parent rather than entering
            // the standard plan-mode UI flow.
            let active = crate::ultraplan::list_sessions();
            if let Some(s) = active
                .iter()
                .find(|s| matches!(s.phase, crate::ultraplan::UltraplanPhase::Exploring))
            {
                crate::ultraplan::complete_session(&s.id, plan.clone());
                return ExecutionResult::success(format!(
                    "Ultraplan session `{}` complete. Plan ready ({} bytes).\n\n{plan}",
                    s.id,
                    plan.len()
                ));
            }
            // Hand the plan off to the UI thread so all permission-mode
            // mutations stay on a single task. The model's tool result
            // is the success acknowledgment — the actual mode flip
            // happens when the main loop drains `UiEvent::ExitPlanModeRequested`.
            if let Some(tx) = snapshot_event_sender() {
                _ = tx
                    .send(crate::runtime::AppEvent::Ui(
                        crate::runtime::UiEvent::ExitPlanModeRequested { plan: plan.clone() },
                    ))
                    .await;
                tracing::info!(
                    target: "jfc::tools::plan_mode",
                    plan_bytes = plan.len(),
                    "ExitPlanMode dispatched to UI thread"
                );
                ExecutionResult::success(
                    "Plan presented to user. Permission mode transitions \
                     from Plan to AcceptEdits — you may now perform the \
                     destructive operations described in the plan."
                        .to_string(),
                )
            } else {
                tracing::warn!(
                    target: "jfc::tools::plan_mode",
                    "ExitPlanMode called but no AppEvent sender registered"
                );
                ExecutionResult::failure(
                    "ExitPlanMode failed: UI event channel unavailable.".to_string(),
                )
            }
        }
        (ToolKind::Mcp(advertised_name), ToolInput::Mcp { arguments, .. }) => {
            // Route through the global MCP registry. The registry is
            // populated at startup from `[mcp.<name>]` config blocks;
            // if it's missing, MCP isn't wired in this build (e.g.
            // headless test) — surface a clean failure so the model
            // can recover rather than thinking the call hung.
            let Some(registry) = snapshot_mcp_registry() else {
                return ExecutionResult::failure(
                    "MCP registry not initialized — restart jfc with the MCP module enabled."
                        .to_string(),
                );
            };
            match crate::mcp::dispatch_tool(&registry, &advertised_name, arguments).await {
                Ok(outcome) if outcome.is_error => ExecutionResult::failure(outcome.text),
                Ok(outcome) => ExecutionResult::success(outcome.text),
                Err(e) => ExecutionResult::failure(format!("MCP dispatch failed: {e}")),
            }
        }
        (
            ToolKind::CronCreate,
            ToolInput::CronCreate {
                schedule,
                command,
                description,
            },
        ) => execute_cron_create(&schedule, &command, &description),
        (ToolKind::CronList, ToolInput::CronList) => execute_cron_list(),
        (ToolKind::CronDelete, ToolInput::CronDelete { id }) => execute_cron_delete(&id),
        (
            ToolKind::ScheduleWakeup,
            ToolInput::ScheduleWakeup {
                delay_seconds,
                prompt,
                reason,
            },
        ) => execute_schedule_wakeup(delay_seconds, &prompt, &reason),
        (ToolKind::Monitor, ToolInput::Monitor { command, until }) => {
            execute_monitor(&command, &until, &cwd).await
        }
        (
            ToolKind::Lsp,
            ToolInput::Lsp {
                kind: req_kind,
                file,
                line,
                column,
            },
        ) => execute_lsp(&req_kind, &file, line, column, &cwd).await,
        (ToolKind::PushNotification, ToolInput::PushNotification { message, title }) => {
            execute_push_notification(&message, title.as_deref())
        }
        (
            ToolKind::RemoteTrigger,
            ToolInput::RemoteTrigger {
                trigger_id,
                payload,
            },
        ) => execute_remote_trigger(&trigger_id, payload.as_ref()).await,
        (ToolKind::EnterPlanMode, ToolInput::EnterPlanMode { reason }) => {
            execute_enter_plan_mode(&reason).await
        }
        (ToolKind::EnterWorktree, ToolInput::EnterWorktree { name, branch }) => {
            execute_enter_worktree(&name, branch.as_deref(), &cwd).await
        }
        (ToolKind::ExitWorktree, ToolInput::ExitWorktree) => execute_exit_worktree(&cwd).await,
        (ToolKind::NotebookRead, ToolInput::NotebookRead { path }) => {
            execute_notebook_read(&path).await
        }
        (
            ToolKind::NotebookEdit,
            ToolInput::NotebookEdit {
                path,
                cell_id,
                new_source,
                edit_mode,
            },
        ) => execute_notebook_edit(&path, &cell_id, &new_source, edit_mode.as_deref()).await,
        (ToolKind::ScratchpadRead, ToolInput::ScratchpadRead { key }) => {
            // Blocking flock + file IO: move off the async reactor thread.
            tokio::task::spawn_blocking(move || execute_scratchpad_read(&key))
                .await
                .unwrap_or_else(|e| {
                    ExecutionResult::failure(format!("scratchpad read task failed: {e}"))
                })
        }
        (ToolKind::ScratchpadWrite, ToolInput::ScratchpadWrite { key, value }) => {
            tokio::task::spawn_blocking(move || execute_scratchpad_write(&key, &value))
                .await
                .unwrap_or_else(|e| {
                    ExecutionResult::failure(format!("scratchpad write task failed: {e}"))
                })
        }
        // ─── New tools (Phase 2-7 port from Claude Code 2.1.150) ───
        (
            ToolKind::SendUserMessage,
            ToolInput::SendUserMessage {
                message,
                summary,
                status,
                ..
            },
        ) => {
            let _label = summary.as_deref().unwrap_or("message");
            let status_str = status.as_deref().unwrap_or("normal");
            // In brief mode, this is the ONLY output the user sees.
            // The tool result is marked with a special prefix so the renderer
            // can distinguish it from normal tool output.
            // Push notification for proactive messages when user may be away.
            if status_str == "proactive" {
                let push_text = summary
                    .as_deref()
                    .unwrap_or(&message[..message.len().min(100)]);
                let _ =
                    crate::tools::notifications::execute_push_notification(push_text, Some("jfc"));
            }
            // The message itself — rendered as markdown to the user.
            ExecutionResult::success(message)
        }
        (
            ToolKind::SendUserFile,
            ToolInput::SendUserFile {
                files,
                caption,
                status,
            },
        ) => {
            let status_str = status.as_deref().unwrap_or("normal");
            let cap = caption.as_deref().unwrap_or("");
            // Extract file path list from the value (accepts array of strings).
            let paths: Vec<String> = match &files {
                serde_json::Value::Array(arr) => arr
                    .iter()
                    .filter_map(|v| v.as_str().map(|s| s.to_string()))
                    .collect(),
                serde_json::Value::String(s) => vec![s.clone()],
                _ => Vec::new(),
            };
            if paths.is_empty() {
                ExecutionResult::failure(
                    "SendUserFile requires a non-empty `files` array of paths.".to_string(),
                )
            } else {
                // Resolve + validate each file exists + collect size info.
                let mut delivered = Vec::with_capacity(paths.len());
                let mut errors = Vec::new();
                for p in &paths {
                    let path = std::path::Path::new(p);
                    let abs = if path.is_absolute() {
                        path.to_path_buf()
                    } else {
                        cwd.join(path)
                    };
                    match std::fs::metadata(&abs) {
                        Ok(meta) if meta.is_file() => {
                            delivered.push(format!("{} ({} bytes)", abs.display(), meta.len()));
                        }
                        Ok(_) => errors.push(format!("{}: not a regular file", abs.display())),
                        Err(e) => errors.push(format!("{}: {e}", abs.display())),
                    }
                }
                // Fire push notification when proactive + sandboxed/away.
                if status_str == "proactive" && !delivered.is_empty() {
                    let body = if cap.is_empty() {
                        format!("{} file(s) delivered", delivered.len())
                    } else {
                        format!("{}: {} file(s)", cap, delivered.len())
                    };
                    let _ = crate::tools::notifications::execute_push_notification(
                        &body,
                        Some("jfc — files ready"),
                    );
                }
                let mut out = format!("[SendUserFile status={status_str}]");
                if !cap.is_empty() {
                    out.push_str(&format!(" {cap}"));
                }
                out.push('\n');
                if !delivered.is_empty() {
                    out.push_str(&format!("Delivered:\n  {}", delivered.join("\n  ")));
                }
                if !errors.is_empty() {
                    if !delivered.is_empty() {
                        out.push('\n');
                    }
                    out.push_str(&format!("Errors:\n  {}", errors.join("\n  ")));
                }
                // Return success if at least one file was delivered, even if some failed
                if !delivered.is_empty() {
                    ExecutionResult::success(out)
                } else if errors.is_empty() {
                    // No files delivered and no errors = caller provided empty paths
                    ExecutionResult::success(out)
                } else {
                    // No files delivered but errors occurred = all files failed
                    ExecutionResult::failure(out)
                }
            }
        }
        (ToolKind::StructuredOutput, ToolInput::StructuredOutput { data }) => {
            // DSPy Assertions on the retry path: classify the payload as an
            // AssertionOutcome and, on a hard violation, return *actionable*
            // feedback (which field failed + re-emit instruction) so the agent's
            // next-turn retry converges instead of seeing a bare error.
            use crate::tools::structured_output::{format_retry_feedback, schema_outcome};
            match format_retry_feedback(&schema_outcome(&data)) {
                None => ExecutionResult::success(format!(
                    "Structured output provided successfully.\n{}",
                    serde_json::to_string_pretty(&data).unwrap_or_else(|_| data.to_string())
                )),
                Some(feedback) => ExecutionResult::failure(feedback),
            }
        }
        (ToolKind::WaitForMcpServers, ToolInput::WaitForMcpServers { timeout_ms }) => {
            let timeout = std::time::Duration::from_millis(timeout_ms.unwrap_or(30_000));
            let registry = snapshot_mcp_registry();
            match registry {
                Some(r) => {
                    let all_servers = r.list().await;
                    if all_servers.is_empty() {
                        return ExecutionResult::success("No MCP servers configured.".to_string());
                    }
                    let total = all_servers.len();
                    let all_names: Vec<String> =
                        all_servers.iter().map(|s| s.name.clone()).collect();
                    // Poll until all servers are active or timeout.
                    let start = std::time::Instant::now();
                    loop {
                        let active = r.list_active().await;
                        if active.len() >= total {
                            let names: Vec<&str> = active.iter().map(|s| s.name.as_str()).collect();
                            break ExecutionResult::success(format!(
                                "All {} MCP servers ready: {}",
                                total,
                                names.join(", ")
                            ));
                        }
                        if start.elapsed() >= timeout {
                            let active_names: std::collections::HashSet<String> =
                                active.iter().map(|s| s.name.clone()).collect();
                            let timed_out: Vec<&str> = all_names
                                .iter()
                                .filter(|n| !active_names.contains(n.as_str()))
                                .map(|n| n.as_str())
                                .collect();
                            let ready: Vec<&str> = active.iter().map(|s| s.name.as_str()).collect();
                            break ExecutionResult::success(format!(
                                "Timeout after {}ms. Ready: [{}]. Timed out: [{}]",
                                timeout.as_millis(),
                                ready.join(", "),
                                timed_out.join(", ")
                            ));
                        }
                        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
                    }
                }
                None => ExecutionResult::success("No MCP registry available.".to_string()),
            }
        }
        (ToolKind::ListMcpResources, ToolInput::ListMcpResources { server }) => {
            let Some(registry) = snapshot_mcp_registry() else {
                return ExecutionResult::failure("MCP registry not initialized.".to_string());
            };
            let servers = registry.list().await;
            let mut result = String::new();
            for s in &servers {
                if let Some(ref filter) = server
                    && s.name != *filter
                {
                    continue;
                }
                result.push_str(&format!("## {}\n", s.name));
                for res in &s.resources {
                    result.push_str(&format!("  - {} ({})\n", res.name, res.uri));
                }
            }
            if result.is_empty() {
                ExecutionResult::success("No MCP resources found.".to_string())
            } else {
                ExecutionResult::success(result)
            }
        }
        (ToolKind::ReadMcpResource, ToolInput::ReadMcpResource { server, uri }) => {
            let Some(registry) = snapshot_mcp_registry() else {
                return ExecutionResult::failure("MCP registry not initialized.".to_string());
            };
            match registry.read_resource(&server, &uri).await {
                Ok(content) => ExecutionResult::success(content),
                Err(e) => ExecutionResult::failure(format!("Failed to read MCP resource: {e}")),
            }
        }
        (ToolKind::DesignProjectCreate, ToolInput::DesignProjectCreate { title }) => {
            execute_design_project_create(&cwd, &title)
        }
        (ToolKind::DesignProjectList, ToolInput::DesignProjectList {}) => {
            execute_design_project_list(&cwd)
        }
        (
            ToolKind::DesignProjectSetMeta,
            ToolInput::DesignProjectSetMeta {
                project_id,
                title,
                is_design_system,
            },
        ) => execute_design_project_set_meta(&cwd, &project_id, title.as_deref(), is_design_system),
        (ToolKind::DesignListFiles, ToolInput::DesignListFiles { project_id }) => {
            execute_design_list_files(&cwd, &project_id)
        }
        (ToolKind::DesignReadFile, ToolInput::DesignReadFile { project_id, path }) => {
            execute_design_read_file(&cwd, &project_id, &path)
        }
        (
            ToolKind::DesignWriteFile,
            ToolInput::DesignWriteFile {
                project_id,
                path,
                content,
                asset_name,
            },
        ) => execute_design_write_file(&cwd, &project_id, &path, &content, asset_name.as_deref()),
        (ToolKind::DesignDeleteFile, ToolInput::DesignDeleteFile { project_id, path }) => {
            execute_design_delete_file(&cwd, &project_id, &path)
        }
        (
            ToolKind::DesignCopyFile,
            ToolInput::DesignCopyFile {
                project_id,
                from_path,
                to_path,
            },
        ) => execute_design_copy_file(&cwd, &project_id, &from_path, &to_path),
        (
            ToolKind::DesignRegisterAsset,
            ToolInput::DesignRegisterAsset {
                project_id,
                name,
                path,
            },
        ) => execute_design_register_asset(&cwd, &project_id, &name, &path),
        (
            ToolKind::DesignUnregisterAsset,
            ToolInput::DesignUnregisterAsset { project_id, path },
        ) => execute_design_unregister_asset(&cwd, &project_id, &path),
        (
            ToolKind::DesignBundleHtml,
            ToolInput::DesignBundleHtml {
                input,
                output,
                require_thumbnail,
            },
        ) => execute_design_bundle_html(&input, output.as_deref(), require_thumbnail),
        (
            ToolKind::DesignHandoff,
            ToolInput::DesignHandoff {
                project_dir,
                feature,
                files,
            },
        ) => execute_design_handoff(&project_dir, &feature, &files),
        (ToolKind::DesignCheckSystem, ToolInput::DesignCheckSystem { project_dir }) => {
            execute_design_check_system(&project_dir)
        }
        (ToolKind::DesignCapabilities, ToolInput::DesignCapabilities { format }) => {
            execute_design_capabilities(format.as_deref())
        }
        (
            ToolKind::DesignServe,
            ToolInput::DesignServe {
                project_dir,
                port,
                file,
            },
        ) => execute_design_serve(&project_dir, port, file.as_deref()),
        (ToolKind::Advisor, ToolInput::Advisor {}) => ExecutionResult::failure(
            "Advisor must be executed through the stream dispatcher so JFC can \
                 attach the current transcript snapshot. Use `/advisor <question>` \
                 for a direct manual query."
                .to_string(),
        ),
        (ToolKind::ConnectGitHub, ToolInput::ConnectGitHub {}) => ExecutionResult::failure(
            "ConnectGitHub is not supported in this environment. \
                 Use `gh auth login` via the Bash tool instead."
                .to_string(),
        ),
        (kind, input) => ExecutionResult::failure(format!(
            "tool input mismatch: {kind:?} was paired with an incompatible \
             ToolInput variant ({}). This is a routing bug — the tool's \
             implementation exists but the parsed input didn't match its \
             expected shape.",
            input.summary()
        )),
    }
}

/// Post-TaskDone hook: open the project PlanStore and advance any plan whose
/// `linked_task_ids` contains `task_id`. If every linked task is complete,
/// the plan's status flips to `Done`. Errors are logged-and-swallowed —
/// plan bookkeeping is best-effort and must never fail the underlying
/// TaskDone tool call.
pub(crate) fn advance_linked_plans(task_store: &TaskStore, task_id: &str) {
    let git_root = crate::context::discover_git_root();
    let plan_store = match crate::plan::PlanStore::open_project(git_root.as_deref()) {
        Ok(s) => s,
        Err(e) => {
            tracing::debug!(
                target: "jfc::plan",
                error = %e,
                "advance_linked_plans: could not open PlanStore (skipping)"
            );
            return;
        }
    };

    // Build a summary from the task's subject when available; fall back to
    // a generic message so the progress-log entry still records the event.
    let summary = task_store
        .get(task_id)
        .map(|t| {
            if t.subject.is_empty() {
                format!("Task {task_id} completed")
            } else {
                format!("Task {task_id} done: {}", t.subject)
            }
        })
        .unwrap_or_else(|| format!("Task {task_id} completed"));

    match plan_store.on_task_done(task_id, &summary, task_store) {
        Ok(advanced) if !advanced.is_empty() => {
            tracing::debug!(
                target: "jfc::plan",
                task_id,
                plans = ?advanced,
                "advanced linked plans on TaskDone"
            );
        }
        Ok(_) => {}
        Err(e) => {
            tracing::warn!(
                target: "jfc::plan",
                task_id,
                error = %e,
                "advance_linked_plans: on_task_done failed"
            );
        }
    }
}
