use std::path::{Path, PathBuf};

use super::registry::record_background_agent_log;
use super::state::BackgroundAgentLaunch;

pub(super) type BackgroundWorktree = (crate::worktrees::WorktreeInfo, PathBuf, Option<String>);

pub(super) enum BackgroundIsolation {
    Proceed(Option<BackgroundWorktree>, Option<PathBuf>),
    FailClosed(String),
}

pub(super) async fn prepare_background_worktree(
    launch: &BackgroundAgentLaunch,
) -> BackgroundIsolation {
    let _linkscope_prepare = linkscope::phase("engine.worker.worktree.prepare");
    linkscope::event_fields(
        "engine.worker.worktree.prepare",
        [
            linkscope::TraceField::text("task_id", launch.task_id.clone()),
            linkscope::TraceField::text(
                "isolation",
                launch.task_input.isolation.clone().unwrap_or_default(),
            ),
            linkscope::TraceField::text("cwd", launch.cwd.display().to_string()),
        ],
    );
    if launch.task_input.isolation.as_deref() != Some("worktree") {
        linkscope::event_fields(
            "engine.worker.worktree.prepare.result",
            [linkscope::TraceField::text("status", "disabled")],
        );
        return BackgroundIsolation::Proceed(None, None);
    }

    let name = format!(
        "agent-{}",
        launch
            .task_id
            .replace("toolu_", "")
            .chars()
            .take(8)
            .collect::<String>()
    );
    let repo_root = match crate::worktrees::find_repo_root_async(&launch.cwd).await {
        Ok(root) => root,
        Err(e) => {
            linkscope::event_fields(
                "engine.worker.worktree.repo_root.result",
                [
                    linkscope::TraceField::text("status", "fallback_cwd"),
                    linkscope::TraceField::text("error", e.to_string()),
                ],
            );
            record_background_agent_log(
                &launch.task_id,
                &format!(
                    "[worktree] failed to resolve git root from {}: {e}; using cwd",
                    launch.cwd.display()
                ),
            );
            launch.cwd.clone()
        }
    };
    match crate::worktrees::create_worktree_async(&repo_root, &name).await {
        Ok(info) => {
            let path = PathBuf::from(&info.path);
            linkscope::event_fields(
                "engine.worker.worktree.prepare.result",
                [
                    linkscope::TraceField::text("status", "created"),
                    linkscope::TraceField::text("path", path.display().to_string()),
                    linkscope::TraceField::text("branch", info.branch.clone()),
                ],
            );
            record_background_agent_log(
                &launch.task_id,
                &format!("[worktree] created {}", path.display()),
            );
            let origin = crate::changeset::ChangeOrigin {
                task_id: Some(launch.task_id.clone()),
                agent_id: launch
                    .task_input
                    .subagent_type
                    .clone()
                    .or_else(|| Some("background".to_string())),
                session_id: launch.parent_session_id.clone(),
            };
            let change_id =
                crate::changeset::open_for_worktree(&repo_root, &info.path, &info.branch, &origin)
                    .await;
            BackgroundIsolation::Proceed(Some((info, repo_root, change_id)), Some(path))
        }
        Err(e) => match crate::changeset::isolation_fallback() {
            crate::changeset::IsolationFallback::FailClosed => {
                let msg = format!(
                    "[worktree] creation failed ({e}); isolation is fail-closed - \
                     refusing to run in the main checkout"
                );
                linkscope::event_fields(
                    "engine.worker.worktree.prepare.result",
                    [
                        linkscope::TraceField::text("status", "fail_closed"),
                        linkscope::TraceField::text("error", e),
                    ],
                );
                record_background_agent_log(&launch.task_id, &msg);
                BackgroundIsolation::FailClosed(msg)
            }
            crate::changeset::IsolationFallback::AllowCwd => {
                linkscope::event_fields(
                    "engine.worker.worktree.prepare.result",
                    [
                        linkscope::TraceField::text("status", "fail_open_cwd"),
                        linkscope::TraceField::text("error", e.to_string()),
                    ],
                );
                record_background_agent_log(
                    &launch.task_id,
                    &format!("[worktree] failed to create worktree: {e}; using cwd (fail-open)"),
                );
                BackgroundIsolation::Proceed(None, None)
            }
        },
    }
}

pub(super) async fn finish_background_worktree(
    task_id: &str,
    worktree_info: Option<BackgroundWorktree>,
) {
    let _linkscope_finish = linkscope::phase("engine.worker.worktree.finish");
    let Some((wt, repo_root, change_id)) = worktree_info else {
        linkscope::event_fields(
            "engine.worker.worktree.finish.result",
            [linkscope::TraceField::text("status", "none")],
        );
        return;
    };
    linkscope::event_fields(
        "engine.worker.worktree.finish",
        [
            linkscope::TraceField::text("task_id", task_id.to_owned()),
            linkscope::TraceField::text("path", wt.path.clone()),
            linkscope::TraceField::count("has_change_id", u64::from(change_id.is_some())),
        ],
    );
    if let Some(ref cid) = change_id {
        let _linkscope_changeset = linkscope::phase("engine.worker.worktree.finalize_changeset");
        crate::changeset::finalize_for_worktree(&repo_root, cid, &wt.path).await;
    }
    let _linkscope_status = linkscope::phase("engine.worker.worktree.git_status");
    let dirty = match tokio::process::Command::new("git")
        .arg("-C")
        .arg(&wt.path)
        .arg("status")
        .arg("--porcelain")
        .output()
        .await
    {
        Ok(out) if out.status.success() => !out.stdout.is_empty(),
        Ok(out) => {
            record_background_agent_log(
                task_id,
                &format!(
                    "[worktree] git status failed; preserving {}: {}",
                    wt.path,
                    String::from_utf8_lossy(&out.stderr)
                ),
            );
            true
        }
        Err(e) => {
            record_background_agent_log(
                task_id,
                &format!(
                    "[worktree] git status spawn failed; preserving {}: {e}",
                    wt.path
                ),
            );
            true
        }
    };
    if dirty {
        linkscope::event_fields(
            "engine.worker.worktree.finish.result",
            [linkscope::TraceField::text("status", "preserved_dirty")],
        );
        record_background_agent_log(
            task_id,
            &format!(
                "[worktree-preserved] path={} branch={} inspect=\"cd {} && git diff\"",
                wt.path, wt.branch, wt.path
            ),
        );
        return;
    }
    let wt_name = Path::new(&wt.path)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("");
    match crate::worktrees::remove_worktree_async(&repo_root, wt_name).await {
        Ok(_) => {
            linkscope::event_fields(
                "engine.worker.worktree.finish.result",
                [linkscope::TraceField::text("status", "removed")],
            );
            record_background_agent_log(task_id, &format!("[worktree-removed] path={}", wt.path))
        }
        Err(e) => {
            linkscope::event_fields(
                "engine.worker.worktree.finish.result",
                [
                    linkscope::TraceField::text("status", "cleanup_failed"),
                    linkscope::TraceField::text("error", e.to_string()),
                ],
            );
            record_background_agent_log(
                task_id,
                &format!("[worktree] cleanup failed for {}: {e}", wt.path),
            );
        }
    }
}
