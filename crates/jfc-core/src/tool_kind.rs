const SERVER_TOOL_USE_PREFIX: &str = "server_tool_use:";
const MCP_TOOL_PREFIX: &str = "mcp__";

#[derive(Clone, Debug, PartialEq)]
pub enum ToolKind {
    Edit,
    Write,
    Read,
    Bash,
    BashOutput,
    Glob,
    Grep,
    Search,
    ApplyPatch,
    TaskCreate,
    TaskUpdate,
    TaskList,
    TaskDone,
    TaskStop,
    TaskGet,
    TaskValidate,
    Task,
    Skill,
    ToolSearch,
    ToolSuggest,
    MemoryCreate,
    MemoryDelete,
    TeamCreate,
    TeamDelete,
    SendMessage,
    TeamMemberMode,
    PostBounty,
    RunBounty,
    MarketStatus,
    PlanCreate,
    PlanList,
    PlanShow,
    PlanAdvance,
    PlanArchive,
    PlanMaterialize,
    LearnStatus,
    LearnHistorize,
    LearnDream,
    LearnKeyFilesList,
    LearnUserProfileShow,
    ExitPlanMode,
    MultiEdit,
    AskUserQuestion,
    WebFetch,
    WebSearch,
    Mcp(String),
    CronCreate,
    CronList,
    CronDelete,
    ScheduleWakeup,
    Monitor,
    Lsp,
    PushNotification,
    RemoteTrigger,
    EnterPlanMode,
    EnterWorktree,
    ExitWorktree,
    NotebookRead,
    NotebookEdit,
    ScratchpadRead,
    ScratchpadWrite,
    Workflow,
    SendUserMessage,
    SendUserFile,
    StructuredOutput,
    WaitForMcpServers,
    ListMcpResources,
    ReadMcpResource,
    Advisor,
    ConnectGitHub,
    DesignProjectCreate,
    DesignProjectList,
    DesignProjectSetMeta,
    DesignListFiles,
    DesignReadFile,
    DesignWriteFile,
    DesignDeleteFile,
    DesignCopyFile,
    DesignRegisterAsset,
    DesignUnregisterAsset,
    DesignBundleHtml,
    DesignHandoff,
    DesignCheckSystem,
    DesignCapabilities,
    DesignServe,
    SetGoal,
    Research,
    Council,
    ServerWebSearch,
    ServerCodeExecution,
    ServerAdvisor,
    Generic(String),
    UnknownTool { advertised_name: String },
}

fn tool_name_eq(candidate: &str, alias: &str) -> bool {
    let mut lhs = candidate.bytes().filter(|b| *b != b'_');
    let mut rhs = alias.bytes().filter(|b| *b != b'_');

    loop {
        match (lhs.next(), rhs.next()) {
            (Some(a), Some(b)) if a.eq_ignore_ascii_case(&b) => {}
            (None, None) => return true,
            _ => return false,
        }
    }
}

fn tool_name_matches(candidate: &str, aliases: &[&str]) -> bool {
    aliases.iter().any(|alias| tool_name_eq(candidate, alias))
}

macro_rules! return_tool_kind {
    ($name:expr, $( $kind:expr => [$($alias:literal),+ $(,)?] ),+ $(,)?) => {
        $(
            if tool_name_matches($name, &[$($alias),+]) {
                return $kind;
            }
        )+
    };
}

/// Like [`return_tool_kind!`] but yields `Some(kind)` on a match and falls
/// through to `None`, so name-matching can be split across helper functions
/// chained with `Option::or_else`.
macro_rules! match_tool_kind {
    ($name:expr, $( $kind:expr => [$($alias:literal),+ $(,)?] ),+ $(,)?) => {{
        $(
            if tool_name_matches($name, &[$($alias),+]) {
                return Some($kind);
            }
        )+
        None
    }};
}

impl ToolKind {
    pub fn from_name(name: &str) -> Self {
        if let Some(kind) = Self::match_io_name(name)
            .or_else(|| Self::match_control_name(name))
            .or_else(|| Self::match_design_name(name))
            .or_else(|| Self::match_generic_name(name))
        {
            return kind;
        }

        if let Some(inner) = name.strip_prefix(SERVER_TOOL_USE_PREFIX) {
            return_tool_kind!(inner,
                Self::ServerWebSearch => ["web_search", "web_search_tool"],
                Self::ServerCodeExecution => ["code_execution"],
                Self::ServerAdvisor => ["advisor"],
            );
            return Self::Generic(name.to_owned());
        }

        if name.starts_with(MCP_TOOL_PREFIX) {
            return Self::Mcp(name.to_owned());
        }

        Self::UnknownTool {
            advertised_name: name.to_owned(),
        }
    }

    /// Filesystem, shell, search, task, team, and web tool names.
    fn match_io_name(name: &str) -> Option<Self> {
        match_tool_kind!(name,
            Self::Edit => ["edit", "str_replace_based_edit_tool"],
            Self::Write => ["write", "write_file"],
            Self::Read => ["read", "read_file"],
            Self::Bash => ["bash", "run_bash"],
            Self::BashOutput => [
                "bash_output",
                "bashoutput",
                "bash_output_tool",
                "task_output",
                "task_output_tool",
                "agent_output",
                "agent_output_tool",
            ],
            Self::Glob => ["glob"],
            Self::Grep => ["grep"],
            Self::Search => ["codebase_search", "search"],
            Self::ApplyPatch => ["apply_patch"],
            Self::MultiEdit => ["multi_edit"],
            Self::NotebookRead => ["notebook_read"],
            Self::NotebookEdit => ["notebook_edit"],
            Self::TaskCreate => ["task_create"],
            Self::TaskUpdate => ["task_update"],
            Self::TaskList => ["task_list"],
            Self::TaskDone => ["task_done"],
            Self::TaskStop => ["task_stop"],
            Self::TaskGet => ["task_get"],
            Self::TaskValidate => ["task_validate"],
            Self::Task => ["task"],
            Self::Skill => ["skill"],
            Self::ToolSearch => ["tool_search", "tool_search_tool"],
            Self::ToolSuggest => ["tool_suggest", "tool_suggest_tool"],
            Self::MemoryCreate => ["memory_create"],
            Self::MemoryDelete => ["memory_delete"],
            Self::TeamCreate => ["team_create"],
            Self::TeamDelete => ["team_delete"],
            Self::SendMessage => ["send_message"],
            Self::TeamMemberMode => ["team_member_mode"],
            Self::WebFetch => ["web_fetch"],
            Self::WebSearch => ["web_search"],
        )
    }

    /// Planning, learning, scheduling, market, MCP-control, and session-flow
    /// tool names.
    fn match_control_name(name: &str) -> Option<Self> {
        match_tool_kind!(name,
            Self::PlanCreate => ["plan_create"],
            Self::PlanList => ["plan_list"],
            Self::PlanShow => ["plan_show"],
            Self::PlanAdvance => ["plan_advance"],
            Self::PlanArchive => ["plan_archive"],
            Self::PlanMaterialize => ["plan_materialize"],
            Self::LearnStatus => ["learn_status"],
            Self::LearnHistorize => ["learn_historize"],
            Self::LearnDream => ["learn_dream"],
            Self::LearnKeyFilesList => ["learn_key_files_list"],
            Self::LearnUserProfileShow => ["learn_user_profile_show"],
            Self::ExitPlanMode => ["exit_plan_mode"],
            Self::EnterPlanMode => ["enter_plan_mode"],
            Self::AskUserQuestion => ["ask_user_question"],
            Self::PostBounty => ["post_bounty"],
            Self::MarketStatus => ["market_status"],
            Self::RunBounty => ["run_bounty"],
            Self::CronCreate => ["cron_create"],
            Self::CronList => ["cron_list"],
            Self::CronDelete => ["cron_delete"],
            Self::ScheduleWakeup => ["schedule_wakeup"],
            Self::Monitor => ["monitor"],
            Self::Lsp => ["lsp", "lsp_tool"],
            Self::PushNotification => ["push_notification"],
            Self::RemoteTrigger => ["remote_trigger"],
            Self::EnterWorktree => ["enter_worktree"],
            Self::ExitWorktree => ["exit_worktree"],
            Self::ScratchpadRead => ["scratchpad_read"],
            Self::ScratchpadWrite => ["scratchpad_write"],
            Self::Workflow => ["workflow", "run_workflow"],
            Self::SendUserMessage => ["send_user_message"],
            Self::SendUserFile => ["send_user_file"],
            Self::StructuredOutput => ["structured_output"],
            Self::WaitForMcpServers => ["wait_for_mcp_servers"],
            Self::ListMcpResources => ["list_mcp_resources", "list_mcp_resources_tool"],
            Self::ReadMcpResource => ["read_mcp_resource", "read_mcp_resource_tool"],
            Self::Advisor => ["advisor"],
            Self::ConnectGitHub => ["connect_github"],
            Self::SetGoal => ["set_goal", "setgoal"],
            Self::Research => ["research", "deep_research"],
            Self::Council => ["council", "model_council"],
        )
    }

    /// Design-tool names.
    fn match_design_name(name: &str) -> Option<Self> {
        match_tool_kind!(name,
            Self::DesignProjectCreate => ["design_project_create", "create_design_project"],
            Self::DesignProjectList => ["design_project_list", "list_design_projects"],
            Self::DesignProjectSetMeta => ["design_project_set_meta", "set_design_project_meta"],
            Self::DesignListFiles => ["design_list_files", "list_design_files"],
            Self::DesignReadFile => ["design_read_file", "read_design_file"],
            Self::DesignWriteFile => ["design_write_file", "write_design_file"],
            Self::DesignDeleteFile => ["design_delete_file", "delete_design_file"],
            Self::DesignCopyFile => ["design_copy_file", "copy_design_file"],
            Self::DesignRegisterAsset => ["design_register_asset", "register_assets", "register_asset"],
            Self::DesignUnregisterAsset => ["design_unregister_asset", "unregister_assets", "unregister_asset"],
            Self::DesignBundleHtml => ["design_bundle_html", "super_inline_html", "bundle_project", "save_standalone_html"],
            Self::DesignHandoff => ["design_handoff", "handoff_to_claude_code"],
            Self::DesignCheckSystem => ["design_check_system", "check_design_system"],
            Self::DesignCapabilities => ["design_capabilities", "jfc_design_capabilities"],
            Self::DesignServe => ["design_serve", "show_html", "show_to_user", "open_preview"],
        )
    }

    /// Named graph/index helper tools carried as `Generic`.
    fn match_generic_name(name: &str) -> Option<Self> {
        match_tool_kind!(name,
            Self::Generic("code_index".to_owned()) => ["code_index"],
            Self::Generic("graph_query".to_owned()) => ["graph_query"],
            Self::Generic("run_coverage".to_owned()) => ["run_coverage"],
            Self::Generic("symbol_edit".to_owned()) => ["symbol_edit"],
        )
    }

    pub fn label(&self) -> &str {
        // Dynamic variants (carry their own name) are handled first; the rest
        // are static strings split across helpers to keep each match small.
        match self {
            Self::Mcp(name) => name.as_str(),
            Self::Generic(name) => name.as_str(),
            Self::UnknownTool { advertised_name } => advertised_name.as_str(),
            other => other
                .core_label()
                .or_else(|| other.aux_label())
                .or_else(|| other.design_label())
                .unwrap_or("UnknownTool"),
        }
    }

    /// Labels for filesystem, shell, task, team, plan, and learn tools.
    fn core_label(&self) -> Option<&'static str> {
        Some(match self {
            Self::Edit => "Edit",
            Self::Write => "Write",
            Self::Read => "Read",
            Self::Bash => "Bash",
            Self::BashOutput => "BashOutput",
            Self::Glob => "Glob",
            Self::Grep => "Grep",
            Self::Search => "Search",
            Self::ApplyPatch => "Patch",
            Self::MultiEdit => "MultiEdit",
            Self::NotebookRead => "NotebookRead",
            Self::NotebookEdit => "NotebookEdit",
            Self::TaskCreate => "TaskCreate",
            Self::TaskUpdate => "TaskUpdate",
            Self::TaskList => "TaskList",
            Self::TaskDone => "TaskDone",
            Self::TaskStop => "TaskStop",
            Self::TaskGet => "TaskGet",
            Self::TaskValidate => "TaskValidate",
            Self::Task => "Task",
            Self::Skill => "Skill",
            Self::ToolSearch => "ToolSearch",
            Self::ToolSuggest => "ToolSuggest",
            Self::MemoryCreate => "MemoryCreate",
            Self::MemoryDelete => "MemoryDelete",
            Self::TeamCreate => "TeamCreate",
            Self::TeamDelete => "TeamDelete",
            Self::SendMessage => "SendMessage",
            Self::TeamMemberMode => "TeamMemberMode",
            Self::PlanCreate => "PlanCreate",
            Self::PlanList => "PlanList",
            Self::PlanShow => "PlanShow",
            Self::PlanAdvance => "PlanAdvance",
            Self::PlanArchive => "PlanArchive",
            Self::PlanMaterialize => "PlanMaterialize",
            Self::LearnStatus => "LearnStatus",
            Self::LearnHistorize => "LearnHistorize",
            Self::LearnDream => "LearnDream",
            Self::LearnKeyFilesList => "LearnKeyFilesList",
            Self::LearnUserProfileShow => "LearnUserProfileShow",
            _ => return None,
        })
    }

    /// Labels for web, market, cron, mcp-control, scratchpad, and session-flow
    /// tools.
    fn aux_label(&self) -> Option<&'static str> {
        Some(match self {
            Self::ExitPlanMode => "ExitPlanMode",
            Self::EnterPlanMode => "EnterPlanMode",
            Self::AskUserQuestion => "AskUserQuestion",
            Self::WebFetch => "WebFetch",
            Self::WebSearch => "WebSearch",
            Self::PostBounty => "PostBounty",
            Self::RunBounty => "RunBounty",
            Self::MarketStatus => "MarketStatus",
            Self::CronCreate => "CronCreate",
            Self::CronList => "CronList",
            Self::CronDelete => "CronDelete",
            Self::ScheduleWakeup => "ScheduleWakeup",
            Self::Monitor => "Monitor",
            Self::Lsp => "LSP",
            Self::PushNotification => "PushNotification",
            Self::RemoteTrigger => "RemoteTrigger",
            Self::EnterWorktree => "EnterWorktree",
            Self::ExitWorktree => "ExitWorktree",
            Self::ScratchpadRead => "ScratchpadRead",
            Self::ScratchpadWrite => "ScratchpadWrite",
            Self::Workflow => "Workflow",
            Self::SendUserMessage => "SendUserMessage",
            Self::SendUserFile => "SendUserFile",
            Self::StructuredOutput => "StructuredOutput",
            Self::WaitForMcpServers => "WaitForMcpServers",
            Self::ListMcpResources => "ListMcpResources",
            Self::ReadMcpResource => "ReadMcpResource",
            Self::Advisor => "Advisor",
            Self::ConnectGitHub => "ConnectGitHub",
            Self::SetGoal => "SetGoal",
            Self::Research => "Research",
            Self::Council => "Council",
            Self::ServerWebSearch => "ServerWebSearch",
            Self::ServerCodeExecution => "ServerCodeExecution",
            Self::ServerAdvisor => "ServerAdvisor",
            _ => return None,
        })
    }

    /// Labels for design tools.
    fn design_label(&self) -> Option<&'static str> {
        Some(match self {
            Self::DesignProjectCreate => "DesignProjectCreate",
            Self::DesignProjectList => "DesignProjectList",
            Self::DesignProjectSetMeta => "DesignProjectSetMeta",
            Self::DesignListFiles => "DesignListFiles",
            Self::DesignReadFile => "DesignReadFile",
            Self::DesignWriteFile => "DesignWriteFile",
            Self::DesignDeleteFile => "DesignDeleteFile",
            Self::DesignCopyFile => "DesignCopyFile",
            Self::DesignRegisterAsset => "DesignRegisterAsset",
            Self::DesignUnregisterAsset => "DesignUnregisterAsset",
            Self::DesignBundleHtml => "DesignBundleHtml",
            Self::DesignHandoff => "DesignHandoff",
            Self::DesignCheckSystem => "DesignCheckSystem",
            Self::DesignCapabilities => "DesignCapabilities",
            Self::DesignServe => "DesignServe",
            _ => return None,
        })
    }

    pub fn api_name(&self) -> &str {
        // Dynamic variants carry their own name; static names are split across
        // helpers to keep each match under the complexity budget.
        match self {
            Self::Mcp(name) => name.as_str(),
            Self::Generic(name) => name.as_str(),
            Self::UnknownTool { advertised_name } => advertised_name.as_str(),
            other => other
                .core_api_name()
                .or_else(|| other.aux_api_name())
                .or_else(|| other.design_api_name())
                .unwrap_or("UnknownTool"),
        }
    }

    /// API names for filesystem, shell, task, team, plan, and learn tools.
    fn core_api_name(&self) -> Option<&'static str> {
        Some(match self {
            Self::Edit => "Edit",
            Self::Write => "Write",
            Self::Read => "Read",
            Self::Bash => "Bash",
            Self::BashOutput => "BashOutput",
            Self::Glob => "Glob",
            Self::Grep => "Grep",
            Self::Search => "codebase_search",
            Self::ApplyPatch => "apply_patch",
            Self::MultiEdit => "MultiEdit",
            Self::NotebookRead => "NotebookRead",
            Self::NotebookEdit => "NotebookEdit",
            Self::TaskCreate => "TaskCreate",
            Self::TaskUpdate => "TaskUpdate",
            Self::TaskList => "TaskList",
            Self::TaskDone => "TaskDone",
            Self::TaskStop => "TaskStop",
            Self::TaskGet => "TaskGet",
            Self::TaskValidate => "TaskValidate",
            Self::Task => "Task",
            Self::Skill => "Skill",
            Self::ToolSearch => "ToolSearch",
            Self::ToolSuggest => "ToolSuggest",
            Self::MemoryCreate => "MemoryCreate",
            Self::MemoryDelete => "MemoryDelete",
            Self::TeamCreate => "TeamCreate",
            Self::TeamDelete => "TeamDelete",
            Self::SendMessage => "SendMessage",
            Self::TeamMemberMode => "TeamMemberMode",
            Self::PlanCreate => "plan_create",
            Self::PlanList => "plan_list",
            Self::PlanShow => "plan_show",
            Self::PlanAdvance => "plan_advance",
            Self::PlanArchive => "plan_archive",
            Self::PlanMaterialize => "plan_materialize",
            Self::LearnStatus => "learn_status",
            Self::LearnHistorize => "learn_historize",
            Self::LearnDream => "learn_dream",
            Self::LearnKeyFilesList => "learn_key_files_list",
            Self::LearnUserProfileShow => "learn_user_profile_show",
            _ => return None,
        })
    }

    /// API names for web, market, cron, mcp-control, scratchpad, and
    /// session-flow tools.
    fn aux_api_name(&self) -> Option<&'static str> {
        Some(match self {
            Self::ExitPlanMode => "ExitPlanMode",
            Self::EnterPlanMode => "EnterPlanMode",
            Self::AskUserQuestion => "AskUserQuestion",
            Self::WebFetch => "WebFetch",
            Self::WebSearch => "WebSearch",
            Self::PostBounty => "post_bounty",
            Self::RunBounty => "run_bounty",
            Self::MarketStatus => "market_status",
            Self::CronCreate => "CronCreate",
            Self::CronList => "CronList",
            Self::CronDelete => "CronDelete",
            Self::ScheduleWakeup => "ScheduleWakeup",
            Self::Monitor => "Monitor",
            Self::Lsp => "LSP",
            Self::PushNotification => "PushNotification",
            Self::RemoteTrigger => "RemoteTrigger",
            Self::EnterWorktree => "EnterWorktree",
            Self::ExitWorktree => "ExitWorktree",
            Self::ScratchpadRead => "ScratchpadRead",
            Self::ScratchpadWrite => "ScratchpadWrite",
            Self::Workflow => "Workflow",
            Self::SendUserMessage => "SendUserMessage",
            Self::SendUserFile => "SendUserFile",
            Self::StructuredOutput => "StructuredOutput",
            Self::WaitForMcpServers => "WaitForMcpServers",
            Self::ListMcpResources => "ListMcpResources",
            Self::ReadMcpResource => "ReadMcpResource",
            Self::Advisor => "Advisor",
            Self::ConnectGitHub => "ConnectGitHub",
            Self::SetGoal => "set_goal",
            Self::Research => "research",
            Self::Council => "council",
            Self::ServerWebSearch => "server_tool_use:web_search",
            Self::ServerCodeExecution => "server_tool_use:code_execution",
            Self::ServerAdvisor => "server_tool_use:advisor",
            _ => return None,
        })
    }

    /// API names for design tools.
    fn design_api_name(&self) -> Option<&'static str> {
        Some(match self {
            Self::DesignProjectCreate => "DesignProjectCreate",
            Self::DesignProjectList => "DesignProjectList",
            Self::DesignProjectSetMeta => "DesignProjectSetMeta",
            Self::DesignListFiles => "DesignListFiles",
            Self::DesignReadFile => "DesignReadFile",
            Self::DesignWriteFile => "DesignWriteFile",
            Self::DesignDeleteFile => "DesignDeleteFile",
            Self::DesignCopyFile => "DesignCopyFile",
            Self::DesignRegisterAsset => "DesignRegisterAsset",
            Self::DesignUnregisterAsset => "DesignUnregisterAsset",
            Self::DesignBundleHtml => "DesignBundleHtml",
            Self::DesignHandoff => "DesignHandoff",
            Self::DesignCheckSystem => "DesignCheckSystem",
            Self::DesignCapabilities => "DesignCapabilities",
            Self::DesignServe => "DesignServe",
            _ => return None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_name_normalizes_across_separators_normal() {
        for n in ["TaskCreate", "task_create", "taskcreate", "TASKCREATE"] {
            assert!(matches!(ToolKind::from_name(n), ToolKind::TaskCreate));
        }
    }

    #[test]
    fn from_name_unknown_falls_through_to_unknown_tool_robust() {
        match ToolKind::from_name("not_a_real_tool") {
            ToolKind::UnknownTool { advertised_name } => {
                assert_eq!(advertised_name, "not_a_real_tool")
            }
            other => panic!("expected UnknownTool, got {other:?}"),
        }
    }

    #[test]
    fn from_name_mcp_prefixed_routes_to_mcp_variant_normal() {
        match ToolKind::from_name("mcp__filesystem__read_file") {
            ToolKind::Mcp(s) => assert_eq!(s, "mcp__filesystem__read_file"),
            other => panic!("expected Mcp, got {other:?}"),
        }
    }
}
