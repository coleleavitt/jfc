//! Slash handlers: thin adapters to the cohesive `*_commands` modules.

use super::*;

pub(super) async fn cmd_worktree(
    app: &mut App,
    parts: &[&str],
    _text: &str,
    _tx: Option<&mpsc::Sender<AppEvent>>,
) {
    handle_worktree_command(app, parts.get(1).copied().unwrap_or("").trim()).await;
}

pub(super) async fn cmd_mcp(
    app: &mut App,
    parts: &[&str],
    _text: &str,
    _tx: Option<&mpsc::Sender<AppEvent>>,
) {
    handle_mcp_command(app, parts.get(1).copied().unwrap_or("").trim()).await;
}

pub(super) async fn cmd_theme(
    app: &mut App,
    parts: &[&str],
    _text: &str,
    _tx: Option<&mpsc::Sender<AppEvent>>,
) {
    handle_theme_command(app, parts.get(1).copied().unwrap_or("").trim());
}

pub(super) async fn cmd_fleet(
    app: &mut App,
    _parts: &[&str],
    _text: &str,
    _tx: Option<&mpsc::Sender<AppEvent>>,
) {
    handle_fleet_command(app);
}

pub(super) async fn cmd_teleport(
    app: &mut App,
    parts: &[&str],
    _text: &str,
    _tx: Option<&mpsc::Sender<AppEvent>>,
) {
    handle_teleport_command(app, parts.get(1).copied().unwrap_or("").trim()).await;
}

pub(super) async fn cmd_init(
    app: &mut App,
    _parts: &[&str],
    _text: &str,
    _tx: Option<&mpsc::Sender<AppEvent>>,
) {
    handle_init_command(app).await;
}

pub(super) async fn cmd_plan(
    app: &mut App,
    _parts: &[&str],
    _text: &str,
    tx: Option<&mpsc::Sender<AppEvent>>,
) {
    handle_doc_command(app, crate::document_formats::DocKind::Plan, tx).await;
}

pub(super) async fn cmd_roadmap(
    app: &mut App,
    _parts: &[&str],
    _text: &str,
    tx: Option<&mpsc::Sender<AppEvent>>,
) {
    handle_doc_command(app, crate::document_formats::DocKind::Roadmap, tx).await;
}

pub(super) async fn cmd_parity(
    app: &mut App,
    _parts: &[&str],
    _text: &str,
    tx: Option<&mpsc::Sender<AppEvent>>,
) {
    handle_doc_command(app, crate::document_formats::DocKind::Parity, tx).await;
}

pub(super) async fn cmd_philosophy(
    app: &mut App,
    _parts: &[&str],
    _text: &str,
    tx: Option<&mpsc::Sender<AppEvent>>,
) {
    handle_doc_command(app, crate::document_formats::DocKind::Philosophy, tx).await;
}

pub(super) async fn cmd_usage(
    app: &mut App,
    _parts: &[&str],
    _text: &str,
    tx: Option<&mpsc::Sender<AppEvent>>,
) {
    handle_doc_command(app, crate::document_formats::DocKind::Usage, tx).await;
}

pub(super) async fn cmd_cost(
    app: &mut App,
    _parts: &[&str],
    _text: &str,
    _tx: Option<&mpsc::Sender<AppEvent>>,
) {
    handle_cost_command(app);
}

pub(super) async fn cmd_status(
    app: &mut App,
    _parts: &[&str],
    _text: &str,
    _tx: Option<&mpsc::Sender<AppEvent>>,
) {
    handle_status_command(app);
}

pub(super) async fn cmd_bug(
    app: &mut App,
    parts: &[&str],
    _text: &str,
    _tx: Option<&mpsc::Sender<AppEvent>>,
) {
    handle_bug_command(app, parts.get(1..).map(|r| r.join(" ")).unwrap_or_default());
}

pub(super) async fn cmd_rewind(
    app: &mut App,
    parts: &[&str],
    _text: &str,
    _tx: Option<&mpsc::Sender<AppEvent>>,
) {
    handle_rewind_command(app, parts.get(1).copied().unwrap_or("").trim());
}

pub(super) async fn cmd_output_style(
    app: &mut App,
    parts: &[&str],
    _text: &str,
    _tx: Option<&mpsc::Sender<AppEvent>>,
) {
    // `/brief` is shorthand for `/output-style brief`. v132
    // exposes the same alias via `tengu_brief_mode_toggled`.
    let alias_brief = parts[0] == "/brief";
    let arg = if alias_brief {
        "brief".to_string()
    } else {
        parts.get(1).copied().unwrap_or("").trim().to_string()
    };
    handle_output_style_command(app, &arg);
}

pub(super) async fn cmd_dump_context(
    app: &mut App,
    _parts: &[&str],
    _text: &str,
    _tx: Option<&mpsc::Sender<AppEvent>>,
) {
    handle_dump_context_command(app).await;
}

pub(super) async fn cmd_install_github_app(
    app: &mut App,
    _parts: &[&str],
    _text: &str,
    _tx: Option<&mpsc::Sender<AppEvent>>,
) {
    handle_install_github_app(app).await;
}

pub(super) async fn cmd_pr(
    app: &mut App,
    parts: &[&str],
    _text: &str,
    _tx: Option<&mpsc::Sender<AppEvent>>,
) {
    handle_pr_view(app, parts.get(1).copied().unwrap_or("").trim()).await;
}

pub(super) async fn cmd_pr_autofix(
    app: &mut App,
    parts: &[&str],
    _text: &str,
    tx: Option<&mpsc::Sender<AppEvent>>,
) {
    handle_pr_autofix(app, parts.get(1).copied().unwrap_or("").trim(), tx).await;
}

pub(super) async fn cmd_setup_github_actions(
    app: &mut App,
    parts: &[&str],
    _text: &str,
    _tx: Option<&mpsc::Sender<AppEvent>>,
) {
    handle_setup_github_actions(app, parts.get(1).copied().unwrap_or("").trim()).await;
}

pub(super) async fn cmd_dream(
    app: &mut App,
    parts: &[&str],
    _text: &str,
    tx: Option<&mpsc::Sender<AppEvent>>,
) {
    handle_dream_command(app, parts.get(1).copied().unwrap_or("").trim(), tx).await;
}

pub(super) async fn cmd_loop(
    app: &mut App,
    parts: &[&str],
    _text: &str,
    tx: Option<&mpsc::Sender<AppEvent>>,
) {
    handle_loop_command(app, parts.get(1).copied().unwrap_or("").trim(), tx).await;
}

pub(super) async fn cmd_schedule(
    app: &mut App,
    parts: &[&str],
    _text: &str,
    tx: Option<&mpsc::Sender<AppEvent>>,
) {
    handle_schedule_command(app, parts.get(1).copied().unwrap_or("").trim(), tx).await;
}
