use std::{fmt, str::FromStr};

use serde::{Deserialize, Serialize};

use crate::{PluginId, PluginSdkError};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HookName {
    PreToolUse,
    PostToolUse,
    PostToolUseFailure,
    UserPromptSubmit,
    SessionStart,
    SessionEnd,
    Stop,
    Setup,
    UserPromptExpansion,
    FileChanged,
    CwdChanged,
    Notification,
    SubagentStart,
    SubagentStop,
    UserInterrupt,
    ModelResponseChunk,
    UserInputRequired,
    PostToolBatch,
    PostCompact,
    BeforeStream,
    AfterStream,
    BeforeCompact,
    AfterCompact,
    OnPermissionRequest,
    OnPermissionDenied,
    OnMessageDisplay,
    OnElicitation,
    OnElicitationResult,
    OnTaskCreated,
    OnTaskCompleted,
    WorktreeCreate,
    WorktreeRemove,
    ConfigChange,
    OnInstructionsLoaded,
    OnTeammateIdle,
    StopFailure,
    CommandExecuteBefore,
    ToolDefinition,
}

impl HookName {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::PreToolUse => "pre_tool_use",
            Self::PostToolUse => "post_tool_use",
            Self::PostToolUseFailure => "post_tool_use_failure",
            Self::UserPromptSubmit => "user_prompt_submit",
            Self::SessionStart => "session_start",
            Self::SessionEnd => "session_end",
            Self::Stop => "stop",
            Self::Setup => "setup",
            Self::UserPromptExpansion => "user_prompt_expansion",
            Self::FileChanged => "file_changed",
            Self::CwdChanged => "cwd_changed",
            Self::Notification => "notification",
            Self::SubagentStart => "subagent_start",
            Self::SubagentStop => "subagent_stop",
            Self::UserInterrupt => "user_interrupt",
            Self::ModelResponseChunk => "model_response_chunk",
            Self::UserInputRequired => "user_input_required",
            Self::PostToolBatch => "post_tool_batch",
            Self::PostCompact => "post_compact",
            Self::BeforeStream => "before_stream",
            Self::AfterStream => "after_stream",
            Self::BeforeCompact => "before_compact",
            Self::AfterCompact => "after_compact",
            Self::OnPermissionRequest => "on_permission_request",
            Self::OnPermissionDenied => "on_permission_denied",
            Self::OnMessageDisplay => "on_message_display",
            Self::OnElicitation => "on_elicitation",
            Self::OnElicitationResult => "on_elicitation_result",
            Self::OnTaskCreated => "on_task_created",
            Self::OnTaskCompleted => "on_task_completed",
            Self::WorktreeCreate => "worktree_create",
            Self::WorktreeRemove => "worktree_remove",
            Self::ConfigChange => "config_change",
            Self::OnInstructionsLoaded => "on_instructions_loaded",
            Self::OnTeammateIdle => "on_teammate_idle",
            Self::StopFailure => "stop_failure",
            Self::CommandExecuteBefore => "command_execute_before",
            Self::ToolDefinition => "tool_definition",
        }
    }

    pub const fn script_name(self) -> &'static str {
        match self {
            Self::PreToolUse => "pre-tool-use",
            Self::PostToolUse => "post-tool-use",
            Self::PostToolUseFailure => "post-tool-use-failure",
            Self::UserPromptSubmit => "user-prompt-submit",
            Self::SessionStart => "session-start",
            Self::SessionEnd => "session-end",
            Self::Stop => "stop",
            Self::Setup => "setup",
            Self::UserPromptExpansion => "user-prompt-expansion",
            Self::FileChanged => "file-changed",
            Self::CwdChanged => "cwd-changed",
            Self::Notification => "notification",
            Self::SubagentStart => "subagent-start",
            Self::SubagentStop => "subagent-stop",
            Self::UserInterrupt => "user-interrupt",
            Self::ModelResponseChunk => "model-response-chunk",
            Self::UserInputRequired => "user-input-required",
            Self::PostToolBatch => "post-tool-batch",
            Self::PostCompact => "post-compact",
            Self::BeforeStream => "before-stream",
            Self::AfterStream => "after-stream",
            Self::BeforeCompact => "before-compact",
            Self::AfterCompact => "after-compact",
            Self::OnPermissionRequest => "on-permission-request",
            Self::OnPermissionDenied => "on-permission-denied",
            Self::OnMessageDisplay => "on-message-display",
            Self::OnElicitation => "on-elicitation",
            Self::OnElicitationResult => "on-elicitation-result",
            Self::OnTaskCreated => "on-task-created",
            Self::OnTaskCompleted => "on-task-completed",
            Self::WorktreeCreate => "worktree-create",
            Self::WorktreeRemove => "worktree-remove",
            Self::ConfigChange => "config-change",
            Self::OnInstructionsLoaded => "on-instructions-loaded",
            Self::OnTeammateIdle => "on-teammate-idle",
            Self::StopFailure => "stop-failure",
            Self::CommandExecuteBefore => "command-execute-before",
            Self::ToolDefinition => "tool-definition",
        }
    }
}

impl FromStr for HookName {
    type Err = PluginSdkError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "pre_tool_use" => Ok(Self::PreToolUse),
            "post_tool_use" => Ok(Self::PostToolUse),
            "post_tool_use_failure" => Ok(Self::PostToolUseFailure),
            "user_prompt_submit" => Ok(Self::UserPromptSubmit),
            "session_start" => Ok(Self::SessionStart),
            "session_end" => Ok(Self::SessionEnd),
            "stop" => Ok(Self::Stop),
            "setup" => Ok(Self::Setup),
            "user_prompt_expansion" => Ok(Self::UserPromptExpansion),
            "file_changed" => Ok(Self::FileChanged),
            "cwd_changed" => Ok(Self::CwdChanged),
            "notification" => Ok(Self::Notification),
            "subagent_start" => Ok(Self::SubagentStart),
            "subagent_stop" => Ok(Self::SubagentStop),
            "user_interrupt" => Ok(Self::UserInterrupt),
            "model_response_chunk" => Ok(Self::ModelResponseChunk),
            "user_input_required" => Ok(Self::UserInputRequired),
            "post_tool_batch" => Ok(Self::PostToolBatch),
            "post_compact" => Ok(Self::PostCompact),
            "before_stream" => Ok(Self::BeforeStream),
            "after_stream" => Ok(Self::AfterStream),
            "before_compact" => Ok(Self::BeforeCompact),
            "after_compact" => Ok(Self::AfterCompact),
            "on_permission_request" => Ok(Self::OnPermissionRequest),
            "on_permission_denied" => Ok(Self::OnPermissionDenied),
            "on_message_display" => Ok(Self::OnMessageDisplay),
            "on_elicitation" => Ok(Self::OnElicitation),
            "on_elicitation_result" => Ok(Self::OnElicitationResult),
            "on_task_created" => Ok(Self::OnTaskCreated),
            "on_task_completed" => Ok(Self::OnTaskCompleted),
            "worktree_create" => Ok(Self::WorktreeCreate),
            "worktree_remove" => Ok(Self::WorktreeRemove),
            "config_change" => Ok(Self::ConfigChange),
            "on_instructions_loaded" => Ok(Self::OnInstructionsLoaded),
            "on_teammate_idle" => Ok(Self::OnTeammateIdle),
            "stop_failure" => Ok(Self::StopFailure),
            "command_execute_before" => Ok(Self::CommandExecuteBefore),
            "tool_definition" => Ok(Self::ToolDefinition),
            other => Err(PluginSdkError::UnknownHookName(other.to_owned())),
        }
    }
}

impl fmt::Display for HookName {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct HookDescriptor {
    pub plugin_id: PluginId,
    pub name: HookName,
    #[serde(default)]
    pub priority: i32,
}

impl HookDescriptor {
    pub fn new(plugin_id: PluginId, name: HookName) -> Self {
        Self {
            plugin_id,
            name,
            priority: 0,
        }
    }

    pub fn name(&self) -> HookName {
        self.name
    }

    pub fn with_priority(mut self, priority: i32) -> Self {
        self.priority = priority;
        self
    }
}
