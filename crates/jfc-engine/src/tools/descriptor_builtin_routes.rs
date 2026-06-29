use std::path::Path;
use std::sync::Arc;

use tokio::sync::Mutex;

pub(crate) use super::descriptor_builtin_route_kind::BuiltinToolRoute;
use super::descriptor_filesystem_routes::{
    execute_edit_route, execute_multi_edit_route, execute_notebook_edit_route,
    execute_notebook_read_route, execute_read_route, execute_write_route,
};
use super::descriptor_shell_routes::{execute_bash_output_route, execute_bash_route};
use super::search::{execute_glob, execute_grep};
use crate::context::ReadDedupCache;
use crate::runtime::ExecutionResult;

pub(crate) struct DescriptorExecutionContext<'a> {
    pub(crate) cwd: &'a Path,
    pub(crate) dedup: Option<&'a Arc<Mutex<ReadDedupCache>>>,
    pub(crate) runtime_tool_id: Option<&'a str>,
}

impl<'a> DescriptorExecutionContext<'a> {
    pub(crate) const fn new(
        cwd: &'a Path,
        dedup: Option<&'a Arc<Mutex<ReadDedupCache>>>,
        runtime_tool_id: Option<&'a str>,
    ) -> Self {
        Self {
            cwd,
            dedup,
            runtime_tool_id,
        }
    }
}

impl<'a> BuiltinToolRoute<'a> {
    pub(crate) async fn execute(self, context: DescriptorExecutionContext<'_>) -> ExecutionResult {
        match self {
            Self::Bash {
                command,
                timeout,
                workdir,
                run_in_background,
                suppress_output,
            } => {
                execute_bash_route(
                    command,
                    timeout,
                    workdir,
                    run_in_background,
                    suppress_output,
                    context.cwd,
                    context.runtime_tool_id,
                )
                .await
            }
            Self::BashOutput {
                task_id,
                offset,
                limit,
                block,
                timeout,
                wait_up_to,
            } => {
                execute_bash_output_route(task_id, offset, limit, block, timeout, wait_up_to).await
            }
            Self::Edit {
                file_path,
                old_string,
                new_string,
                replacement,
            } => {
                execute_edit_route(
                    file_path,
                    old_string,
                    new_string,
                    replacement,
                    context.cwd,
                    context.dedup,
                )
                .await
            }
            Self::Glob { pattern, path } => execute_glob(pattern, path, context.cwd).await,
            Self::Grep {
                pattern,
                path,
                glob,
                output_mode,
            } => execute_grep(pattern, path, glob, output_mode, context.cwd).await,
            Self::MultiEdit { file_path, edits } => {
                execute_multi_edit_route(file_path, edits, context.cwd, context.dedup).await
            }
            Self::NotebookEdit {
                path,
                cell_id,
                new_source,
                edit_mode,
            } => execute_notebook_edit_route(path, cell_id, new_source, edit_mode).await,
            Self::NotebookRead { path } => execute_notebook_read_route(path).await,
            Self::Read {
                file_path,
                offset,
                limit,
            } => execute_read_route(file_path, offset, limit, context.dedup).await,
            Self::Write { file_path, content } => {
                execute_write_route(file_path, content, context.cwd, context.dedup).await
            }
        }
    }
}
