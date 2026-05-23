const SERVER_TOOL_USE_PREFIX: &str = "server_tool_use:";
const MCP_TOOL_PREFIX: &str = "mcp__";

#[derive(Clone, Debug, PartialEq)]
pub enum ToolKind {
    Edit,
    Write,
    Read,
    Bash,
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
    CodeIndex,
    GraphQuery,
    GraphContext,
    GraphSearch,
    GraphCallers,
    GraphCallees,
    GraphImpact,
    GraphNode,
    GraphExplore,
    GraphStatus,
    GraphFiles,
    RunCoverage,
    SymbolEdit,
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
    ServerWebSearch,
    ServerCodeExecution,
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

impl ToolKind {
    pub fn from_name(name: &str) -> Self {
        return_tool_kind!(name,
            Self::Edit => ["edit", "str_replace_based_edit_tool"],
            Self::Write => ["write", "write_file"],
            Self::Read => ["read", "read_file"],
            Self::Bash => ["bash", "run_bash"],
            Self::Glob => ["glob"],
            Self::Grep => ["grep"],
            Self::Search => ["codebase_search", "search"],
            Self::ApplyPatch => ["apply_patch"],
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
            Self::CodeIndex => ["code_index"],
            Self::GraphQuery => ["graph_query"],
            Self::GraphContext => ["graph_context"],
            Self::GraphSearch => ["graph_search"],
            Self::GraphCallers => ["graph_callers"],
            Self::GraphCallees => ["graph_callees"],
            Self::GraphImpact => ["graph_impact"],
            Self::GraphNode => ["graph_node"],
            Self::GraphExplore => ["graph_explore"],
            Self::GraphStatus => ["graph_status"],
            Self::GraphFiles => ["graph_files"],
            Self::RunCoverage => ["run_coverage"],
            Self::SymbolEdit => ["symbol_edit"],
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
            Self::MultiEdit => ["multi_edit"],
            Self::AskUserQuestion => ["ask_user_question"],
            Self::WebFetch => ["web_fetch"],
            Self::WebSearch => ["web_search"],
            Self::PostBounty => ["post_bounty"],
            Self::MarketStatus => ["market_status"],
            Self::RunBounty => ["run_bounty"],
            Self::CronCreate => ["cron_create"],
            Self::CronList => ["cron_list"],
            Self::CronDelete => ["cron_delete"],
            Self::ScheduleWakeup => ["schedule_wakeup"],
            Self::Monitor => ["monitor"],
            Self::Lsp => ["lsp"],
            Self::PushNotification => ["push_notification"],
            Self::RemoteTrigger => ["remote_trigger"],
            Self::EnterPlanMode => ["enter_plan_mode"],
            Self::EnterWorktree => ["enter_worktree"],
            Self::ExitWorktree => ["exit_worktree"],
            Self::NotebookRead => ["notebook_read"],
            Self::NotebookEdit => ["notebook_edit"],
            Self::ScratchpadRead => ["scratchpad_read"],
            Self::ScratchpadWrite => ["scratchpad_write"],
            Self::Workflow => ["workflow", "run_workflow"],
        );

        if let Some(inner) = name.strip_prefix(SERVER_TOOL_USE_PREFIX) {
            return_tool_kind!(inner,
                Self::ServerWebSearch => ["web_search", "web_search_tool"],
                Self::ServerCodeExecution => ["code_execution"],
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

    pub fn label(&self) -> &str {
        match self {
            Self::Edit => "Edit",
            Self::Write => "Write",
            Self::Read => "Read",
            Self::Bash => "Bash",
            Self::Glob => "Glob",
            Self::Grep => "Grep",
            Self::Search => "Search",
            Self::ApplyPatch => "Patch",
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
            Self::CodeIndex => "CodeIndex",
            Self::GraphQuery => "GraphQuery",
            Self::GraphContext => "GraphContext",
            Self::GraphSearch => "GraphSearch",
            Self::GraphCallers => "GraphCallers",
            Self::GraphCallees => "GraphCallees",
            Self::GraphImpact => "GraphImpact",
            Self::GraphNode => "GraphNode",
            Self::GraphExplore => "GraphExplore",
            Self::GraphStatus => "GraphStatus",
            Self::GraphFiles => "GraphFiles",
            Self::RunCoverage => "RunCoverage",
            Self::SymbolEdit => "SymbolEdit",
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
            Self::ExitPlanMode => "ExitPlanMode",
            Self::MultiEdit => "MultiEdit",
            Self::AskUserQuestion => "AskUserQuestion",
            Self::WebFetch => "WebFetch",
            Self::WebSearch => "WebSearch",
            Self::PostBounty => "PostBounty",
            Self::RunBounty => "RunBounty",
            Self::MarketStatus => "MarketStatus",
            Self::Mcp(name) => name.as_str(),
            Self::CronCreate => "CronCreate",
            Self::CronList => "CronList",
            Self::CronDelete => "CronDelete",
            Self::ScheduleWakeup => "ScheduleWakeup",
            Self::Monitor => "Monitor",
            Self::Lsp => "LSP",
            Self::PushNotification => "PushNotification",
            Self::RemoteTrigger => "RemoteTrigger",
            Self::EnterPlanMode => "EnterPlanMode",
            Self::EnterWorktree => "EnterWorktree",
            Self::ExitWorktree => "ExitWorktree",
            Self::NotebookRead => "NotebookRead",
            Self::NotebookEdit => "NotebookEdit",
            Self::ScratchpadRead => "ScratchpadRead",
            Self::ScratchpadWrite => "ScratchpadWrite",
            Self::Workflow => "Workflow",
            Self::ServerWebSearch => "ServerWebSearch",
            Self::ServerCodeExecution => "ServerCodeExecution",
            Self::Generic(name) => name.as_str(),
            Self::UnknownTool { advertised_name } => advertised_name.as_str(),
        }
    }

    pub fn api_name(&self) -> &str {
        match self {
            Self::Edit => "Edit",
            Self::Write => "Write",
            Self::Read => "Read",
            Self::Bash => "Bash",
            Self::Glob => "Glob",
            Self::Grep => "Grep",
            Self::Search => "codebase_search",
            Self::ApplyPatch => "apply_patch",
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
            Self::CodeIndex => "code_index",
            Self::GraphQuery => "graph_query",
            Self::GraphContext => "graph_context",
            Self::GraphSearch => "graph_search",
            Self::GraphCallers => "graph_callers",
            Self::GraphCallees => "graph_callees",
            Self::GraphImpact => "graph_impact",
            Self::GraphNode => "graph_node",
            Self::GraphExplore => "graph_explore",
            Self::GraphStatus => "graph_status",
            Self::GraphFiles => "graph_files",
            Self::RunCoverage => "run_coverage",
            Self::SymbolEdit => "symbol_edit",
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
            Self::ExitPlanMode => "ExitPlanMode",
            Self::MultiEdit => "MultiEdit",
            Self::AskUserQuestion => "AskUserQuestion",
            Self::WebFetch => "WebFetch",
            Self::WebSearch => "WebSearch",
            Self::PostBounty => "post_bounty",
            Self::RunBounty => "run_bounty",
            Self::MarketStatus => "market_status",
            Self::Mcp(name) => name.as_str(),
            Self::CronCreate => "CronCreate",
            Self::CronList => "CronList",
            Self::CronDelete => "CronDelete",
            Self::ScheduleWakeup => "ScheduleWakeup",
            Self::Monitor => "Monitor",
            Self::Lsp => "LSP",
            Self::PushNotification => "PushNotification",
            Self::RemoteTrigger => "RemoteTrigger",
            Self::EnterPlanMode => "EnterPlanMode",
            Self::EnterWorktree => "EnterWorktree",
            Self::ExitWorktree => "ExitWorktree",
            Self::NotebookRead => "NotebookRead",
            Self::NotebookEdit => "NotebookEdit",
            Self::ScratchpadRead => "ScratchpadRead",
            Self::ScratchpadWrite => "ScratchpadWrite",
            Self::Workflow => "Workflow",
            Self::ServerWebSearch => "server_tool_use:web_search",
            Self::ServerCodeExecution => "server_tool_use:code_execution",
            Self::Generic(name) => name.as_str(),
            Self::UnknownTool { advertised_name } => advertised_name.as_str(),
        }
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
