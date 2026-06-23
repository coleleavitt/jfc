//! Slash handlers: thin adapters to the cohesive `*_commands` modules.

use super::knowledge::handle_knowledge_command;
use super::{automation::*, github::*, local::*, mcp::*, worktree::*};
use crate::commands::prelude::*;
use crate::runtime::EngineEvent;

pub(super) async fn cmd_worktree(
    state: &mut EngineState,
    parts: &[&str],
    _text: &str,
    _tx: Option<&mpsc::Sender<EngineEvent>>,
) {
    handle_worktree_command(state, parts.get(1).copied().unwrap_or("").trim()).await;
}

pub(super) async fn cmd_knowledge(
    state: &mut EngineState,
    _parts: &[&str],
    text: &str,
    _tx: Option<&mpsc::Sender<EngineEvent>>,
) {
    // Pass the full argument string (everything after `/knowledge`) so
    // multi-word subcommands like `gc-legacy --confirm` arrive intact.
    let arg = text
        .trim()
        .strip_prefix("/knowledge")
        .unwrap_or("")
        .trim();
    handle_knowledge_command(state, arg).await;
}

pub(super) async fn cmd_mcp(
    state: &mut EngineState,
    parts: &[&str],
    _text: &str,
    _tx: Option<&mpsc::Sender<EngineEvent>>,
) {
    handle_mcp_command(state, parts.get(1).copied().unwrap_or("").trim()).await;
}
pub(super) async fn cmd_teleport(
    state: &mut EngineState,
    parts: &[&str],
    _text: &str,
    _tx: Option<&mpsc::Sender<EngineEvent>>,
) {
    handle_teleport_command(state, parts.get(1).copied().unwrap_or("").trim()).await;
}

pub(super) async fn cmd_init(
    state: &mut EngineState,
    _parts: &[&str],
    _text: &str,
    _tx: Option<&mpsc::Sender<EngineEvent>>,
) {
    handle_init_command(state).await;
}

pub(super) async fn cmd_plan(
    state: &mut EngineState,
    _parts: &[&str],
    _text: &str,
    tx: Option<&mpsc::Sender<EngineEvent>>,
) {
    handle_doc_command(state, crate::document_formats::DocKind::Plan, tx).await;
}

pub(super) async fn cmd_roadmap(
    state: &mut EngineState,
    _parts: &[&str],
    _text: &str,
    tx: Option<&mpsc::Sender<EngineEvent>>,
) {
    handle_doc_command(state, crate::document_formats::DocKind::Roadmap, tx).await;
}

pub(super) async fn cmd_parity(
    state: &mut EngineState,
    _parts: &[&str],
    _text: &str,
    tx: Option<&mpsc::Sender<EngineEvent>>,
) {
    handle_doc_command(state, crate::document_formats::DocKind::Parity, tx).await;
}

pub(super) async fn cmd_philosophy(
    state: &mut EngineState,
    _parts: &[&str],
    _text: &str,
    tx: Option<&mpsc::Sender<EngineEvent>>,
) {
    handle_doc_command(state, crate::document_formats::DocKind::Philosophy, tx).await;
}

pub(super) async fn cmd_usage(
    state: &mut EngineState,
    _parts: &[&str],
    _text: &str,
    tx: Option<&mpsc::Sender<EngineEvent>>,
) {
    handle_doc_command(state, crate::document_formats::DocKind::Usage, tx).await;
}

pub(super) async fn cmd_cost(
    state: &mut EngineState,
    _parts: &[&str],
    _text: &str,
    _tx: Option<&mpsc::Sender<EngineEvent>>,
) {
    handle_cost_command(state);
}

pub(super) async fn cmd_usage_report(
    state: &mut EngineState,
    _parts: &[&str],
    _text: &str,
    _tx: Option<&mpsc::Sender<EngineEvent>>,
) {
    super::local::handle_usage_report_command(state);
}

pub(super) async fn cmd_status(
    state: &mut EngineState,
    _parts: &[&str],
    _text: &str,
    _tx: Option<&mpsc::Sender<EngineEvent>>,
) {
    handle_status_command(state);
}

/// `/audit [change <id>]` — show the runtime audit ledger (agent actions).
/// With `change <id>`, scope to a single change-set's events.
pub(super) async fn cmd_audit(
    state: &mut EngineState,
    parts: &[&str],
    _text: &str,
    _tx: Option<&mpsc::Sender<EngineEvent>>,
) {
    let mut filter = jfc_changeset::LedgerFilter::default();
    if let (Some(&"change"), Some(id)) = (parts.get(1), parts.get(2)) {
        filter.change_id = Some((*id).to_string());
    }
    let root = std::path::PathBuf::from(&state.cwd);
    let events = crate::changeset::query_ledger_in(&root, &filter);
    let body = format!(
        "Audit ledger ({} event{}):\n{}",
        events.len(),
        if events.len() == 1 { "" } else { "s" },
        crate::changeset::render_ledger(&events)
    );
    state.messages.push(ChatMessage::assistant(body));
}

/// `/commands [manifest|completions]` — the unified command/tool list,
/// generated from the single CommandSpec metadata layer across CLI, slash, and
/// tool surfaces. `manifest` emits the machine-readable JSON-lines manifest;
/// `completions` emits the deduped completion words.
pub(super) async fn cmd_commands(
    state: &mut EngineState,
    parts: &[&str],
    _text: &str,
    _tx: Option<&mpsc::Sender<EngineEvent>>,
) {
    let body = match parts.get(1) {
        Some(&"manifest") => format!(
            "Command manifest (single-source):\n{}",
            crate::command_spec::render_manifest_all()
        ),
        Some(&"completions") => {
            let specs = crate::command_spec::all_specs();
            let refs: Vec<&dyn crate::command_spec::CommandSpec> = specs
                .iter()
                .map(|s| s as &dyn crate::command_spec::CommandSpec)
                .collect();
            format!(
                "Completion words:\n{}",
                crate::command_spec::render_completions(&refs)
            )
        }
        _ => format!(
            "Commands across all surfaces:\n{}",
            crate::command_spec::render_all()
        ),
    };
    state.messages.push(ChatMessage::assistant(body));
}

/// `/changes [show|test|apply|revert <id> [-- <cmd>]]` — review surface for
/// agent change-sets. Bare `/changes` lists; subcommands operate on one id.
pub(super) async fn cmd_changes(
    state: &mut EngineState,
    parts: &[&str],
    text: &str,
    _tx: Option<&mpsc::Sender<EngineEvent>>,
) {
    let root = std::path::PathBuf::from(&state.cwd);
    let body = match (parts.get(1), parts.get(2)) {
        (Some(&"show"), Some(id)) => crate::changeset::show_change(&root, id),
        (Some(&"apply"), Some(id)) => crate::changeset::apply_change(&root, id).await,
        (Some(&"revert"), Some(id)) => crate::changeset::revert_change(&root, id).await,
        (Some(&"test"), Some(id)) => {
            // Everything after `-- ` is the test command.
            let cmd = text.split_once(" -- ").map(|(_, c)| c.trim()).unwrap_or("");
            if cmd.is_empty() {
                "usage: /changes test <id> -- <command>".to_string()
            } else {
                crate::changeset::test_change(&root, id, cmd).await
            }
        }
        _ => crate::changeset::list_changes(&root),
    };
    state.messages.push(ChatMessage::assistant(body));
}

pub(super) async fn cmd_bug(
    state: &mut EngineState,
    parts: &[&str],
    _text: &str,
    _tx: Option<&mpsc::Sender<EngineEvent>>,
) {
    handle_bug_command(
        state,
        parts.get(1..).map(|r| r.join(" ")).unwrap_or_default(),
    );
}

pub(super) async fn cmd_rewind(
    state: &mut EngineState,
    parts: &[&str],
    _text: &str,
    _tx: Option<&mpsc::Sender<EngineEvent>>,
) {
    handle_rewind_command(state, parts.get(1).copied().unwrap_or("").trim());
}

pub(super) async fn cmd_output_style(
    state: &mut EngineState,
    parts: &[&str],
    _text: &str,
    _tx: Option<&mpsc::Sender<EngineEvent>>,
) {
    // `/brief` is shorthand for `/output-style brief`. v132
    // exposes the same alias via `tengu_brief_mode_toggled`.
    let alias_brief = parts[0] == "/brief";
    let arg = if alias_brief {
        "brief".to_string()
    } else {
        parts.get(1).copied().unwrap_or("").trim().to_string()
    };
    handle_output_style_command(state, &arg);
}

pub(super) async fn cmd_dump_context(
    state: &mut EngineState,
    _parts: &[&str],
    _text: &str,
    _tx: Option<&mpsc::Sender<EngineEvent>>,
) {
    handle_dump_context_command(state).await;
}

pub(super) async fn cmd_install_github_app(
    state: &mut EngineState,
    _parts: &[&str],
    _text: &str,
    _tx: Option<&mpsc::Sender<EngineEvent>>,
) {
    handle_install_github_app(state).await;
}

pub(super) async fn cmd_pr(
    state: &mut EngineState,
    parts: &[&str],
    _text: &str,
    _tx: Option<&mpsc::Sender<EngineEvent>>,
) {
    handle_pr_view(state, parts.get(1).copied().unwrap_or("").trim()).await;
}

pub(super) async fn cmd_pr_autofix(
    state: &mut EngineState,
    parts: &[&str],
    _text: &str,
    tx: Option<&mpsc::Sender<EngineEvent>>,
) {
    handle_pr_autofix(state, parts.get(1).copied().unwrap_or("").trim(), tx).await;
}

pub(super) async fn cmd_setup_github_actions(
    state: &mut EngineState,
    parts: &[&str],
    _text: &str,
    _tx: Option<&mpsc::Sender<EngineEvent>>,
) {
    handle_setup_github_actions(state, parts.get(1).copied().unwrap_or("").trim()).await;
}

pub(super) async fn cmd_dream(
    state: &mut EngineState,
    parts: &[&str],
    _text: &str,
    tx: Option<&mpsc::Sender<EngineEvent>>,
) {
    handle_dream_command(state, parts.get(1).copied().unwrap_or("").trim(), tx).await;
}

pub(super) async fn cmd_loop(
    state: &mut EngineState,
    parts: &[&str],
    _text: &str,
    tx: Option<&mpsc::Sender<EngineEvent>>,
) {
    handle_loop_command(state, parts.get(1).copied().unwrap_or("").trim(), tx).await;
}

pub(super) async fn cmd_schedule(
    state: &mut EngineState,
    parts: &[&str],
    _text: &str,
    tx: Option<&mpsc::Sender<EngineEvent>>,
) {
    handle_schedule_command(state, parts.get(1).copied().unwrap_or("").trim(), tx).await;
}

pub(super) async fn cmd_fleet(
    state: &mut EngineState,
    _parts: &[&str],
    _text: &str,
    _tx: Option<&mpsc::Sender<EngineEvent>>,
) {
    handle_fleet_command(state);
}
