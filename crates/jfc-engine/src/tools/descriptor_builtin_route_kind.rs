use crate::types::{ReplacementMode, ToolInput, ToolKind};

use super::descriptor_filesystem_defs::{
    EDIT_HANDLER, MULTI_EDIT_HANDLER, NOTEBOOK_EDIT_HANDLER, NOTEBOOK_READ_HANDLER, READ_HANDLER,
    WRITE_HANDLER,
};
use super::descriptor_search_defs::{GLOB_HANDLER, GREP_HANDLER};
use super::descriptor_shell_defs::{BASH_HANDLER, BASH_OUTPUT_HANDLER};

pub(crate) enum BuiltinToolRoute<'a> {
    Bash {
        command: &'a str,
        timeout: Option<u64>,
        workdir: Option<&'a str>,
        run_in_background: Option<bool>,
        suppress_output: Option<bool>,
    },
    BashOutput {
        task_id: &'a str,
        offset: Option<u64>,
        limit: Option<u64>,
        block: Option<bool>,
        timeout: Option<u64>,
        wait_up_to: Option<u64>,
    },
    Edit {
        file_path: &'a str,
        old_string: &'a str,
        new_string: &'a str,
        replacement: ReplacementMode,
    },
    Glob {
        pattern: &'a str,
        path: Option<&'a str>,
    },
    Grep {
        pattern: &'a str,
        path: Option<&'a str>,
        glob: Option<&'a str>,
        output_mode: Option<&'a str>,
    },
    MultiEdit {
        file_path: &'a str,
        edits: &'a serde_json::Value,
    },
    NotebookEdit {
        path: &'a str,
        cell_id: &'a str,
        new_source: &'a str,
        edit_mode: Option<&'a str>,
    },
    NotebookRead {
        path: &'a str,
    },
    Read {
        file_path: &'a str,
        offset: Option<u64>,
        limit: Option<u64>,
    },
    Write {
        file_path: &'a str,
        content: &'a str,
    },
}

impl<'a> BuiltinToolRoute<'a> {
    pub(crate) fn from_tool(kind: &'a ToolKind, input: &'a ToolInput) -> Option<Self> {
        match (kind, input) {
            (
                ToolKind::Bash,
                ToolInput::Bash {
                    command,
                    timeout,
                    workdir,
                    run_in_background,
                    suppress_output,
                },
            ) => Some(Self::Bash {
                command,
                timeout: *timeout,
                workdir: workdir.as_deref(),
                run_in_background: *run_in_background,
                suppress_output: *suppress_output,
            }),
            (
                ToolKind::BashOutput,
                ToolInput::BashOutput {
                    task_id,
                    offset,
                    limit,
                    block,
                    timeout,
                    wait_up_to,
                },
            ) => Some(Self::BashOutput {
                task_id,
                offset: *offset,
                limit: *limit,
                block: *block,
                timeout: *timeout,
                wait_up_to: *wait_up_to,
            }),
            (
                ToolKind::Edit,
                ToolInput::Edit {
                    file_path,
                    old_string,
                    new_string,
                    replacement,
                },
            ) => Some(Self::Edit {
                file_path,
                old_string,
                new_string,
                replacement: *replacement,
            }),
            (ToolKind::Glob, ToolInput::Glob { pattern, path }) => Some(Self::Glob {
                pattern,
                path: path.as_deref(),
            }),
            (
                ToolKind::Grep,
                ToolInput::Grep {
                    pattern,
                    path,
                    glob,
                    output_mode,
                },
            ) => Some(Self::Grep {
                pattern,
                path: path.as_deref(),
                glob: glob.as_deref(),
                output_mode: output_mode.as_deref(),
            }),
            (ToolKind::MultiEdit, ToolInput::MultiEdit { file_path, edits }) => {
                Some(Self::MultiEdit { file_path, edits })
            }
            (
                ToolKind::NotebookEdit,
                ToolInput::NotebookEdit {
                    path,
                    cell_id,
                    new_source,
                    edit_mode,
                },
            ) => Some(Self::NotebookEdit {
                path,
                cell_id,
                new_source,
                edit_mode: edit_mode.as_deref(),
            }),
            (ToolKind::NotebookRead, ToolInput::NotebookRead { path }) => {
                Some(Self::NotebookRead { path })
            }
            (
                ToolKind::Read,
                ToolInput::Read {
                    file_path,
                    offset,
                    limit,
                },
            ) => Some(Self::Read {
                file_path,
                offset: *offset,
                limit: *limit,
            }),
            (ToolKind::Write, ToolInput::Write { file_path, content }) => {
                Some(Self::Write { file_path, content })
            }
            _ => None,
        }
    }

    pub(crate) const fn handler(&self) -> &'static str {
        match self {
            Self::Bash { .. } => BASH_HANDLER,
            Self::BashOutput { .. } => BASH_OUTPUT_HANDLER,
            Self::Edit { .. } => EDIT_HANDLER,
            Self::Glob { .. } => GLOB_HANDLER,
            Self::Grep { .. } => GREP_HANDLER,
            Self::MultiEdit { .. } => MULTI_EDIT_HANDLER,
            Self::NotebookEdit { .. } => NOTEBOOK_EDIT_HANDLER,
            Self::NotebookRead { .. } => NOTEBOOK_READ_HANDLER,
            Self::Read { .. } => READ_HANDLER,
            Self::Write { .. } => WRITE_HANDLER,
        }
    }
}
