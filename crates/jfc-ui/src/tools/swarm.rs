use std::path::Path;

use super::ExecutionResult;

pub(super) async fn execute_team_member_mode(
    member_name: &str,
    mode: &str,
    active_team_name: Option<&str>,
) -> ExecutionResult {
    // Validate the mode string against the same vocabulary the leader's
    // `PermissionMode` understands. Reject anything else so a typo
    // doesn't silently leave the teammate in an undefined state.
    const VALID_MODES: &[&str] = &["plan", "default", "acceptEdits", "bypassPermissions"];
    if !VALID_MODES.iter().any(|v| v.eq_ignore_ascii_case(mode)) {
        return ExecutionResult::failure(format!(
            "Invalid mode '{mode}'. Must be one of: plan | default | acceptEdits | bypassPermissions"
        ));
    }
    let team_name = match active_team_name {
        Some(t) => t,
        None => {
            return ExecutionResult::failure(
                "No active team. Use TeamCreate first to establish a team.",
            );
        }
    };
    match crate::swarm::team_helpers::set_member_mode(team_name, member_name, mode).await {
        Ok(_) => ExecutionResult::success(format!("{member_name} mode set to {mode}")),
        Err(e) => ExecutionResult::failure(format!("Failed to update {member_name}'s mode: {e}")),
    }
}

// ─── LSP tool ──────────────────────────────────────────────────────────────
//
// Spawns a one-shot LSP client per request, fires the request, and shuts
// the client down. This is wasteful when the same workspace is queried
// repeatedly — a future iteration should pull from a global registry of
// already-spawned clients seeded by `lsp_client::maybe_spawn_lsp_clients`.
// For now the tool stays self-contained: the model can ask LSP questions

pub(super) async fn execute_team_create(
    team_name: &str,
    description: Option<&str>,
    cwd: &Path,
) -> ExecutionResult {
    use crate::swarm::{self, team_helpers, types::make_agent_id};

    let lead_id = make_agent_id(swarm::TEAM_LEAD_NAME, team_name);

    match team_helpers::create_team(
        team_name,
        description,
        &lead_id,
        None,
        &cwd.to_string_lossy(),
    )
    .await
    {
        Ok(_team_file) => {
            let file_path = team_helpers::team_file_path(team_name);
            let result = serde_json::json!({
                "team_name": team_name,
                "team_file_path": file_path.to_string_lossy(),
                "lead_agent_id": lead_id,
            });
            ExecutionResult::success(serde_json::to_string_pretty(&result).unwrap_or_default())
        }
        Err(e) => ExecutionResult::failure(format!("Failed to create team: {e}")),
    }
}

pub(super) async fn execute_team_delete(active_team_name: Option<&str>) -> ExecutionResult {
    use crate::swarm::team_helpers;

    let team_name = match active_team_name {
        Some(name) => name,
        None => {
            return ExecutionResult::failure(
                "No active team. Use TeamCreate first to establish a team.",
            );
        }
    };

    match team_helpers::delete_team(team_name).await {
        Ok(()) => {
            let result = serde_json::json!({
                "success": true,
                "message": format!("Cleaned up directories for team \"{team_name}\""),
                "team_name": team_name,
            });
            ExecutionResult::success(serde_json::to_string_pretty(&result).unwrap_or_default())
        }
        Err(e) => ExecutionResult::failure(format!("Failed to delete team: {e}")),
    }
}

pub(super) async fn execute_send_message(
    to: &str,
    message: &str,
    summary: Option<&str>,
    active_team_name: Option<&str>,
) -> ExecutionResult {
    use crate::swarm::mailbox;
    use crate::swarm::types::MailboxMessage;

    let team_name = active_team_name.unwrap_or(crate::swarm::DEFAULT_TEAM_NAME);

    let msg = MailboxMessage {
        from: crate::swarm::TEAM_LEAD_NAME.to_owned(),
        text: message.to_owned(),
        timestamp: chrono::Utc::now().to_rfc3339(),
        color: None,
        summary: summary.map(str::to_owned),
        read: false,
    };

    match mailbox::write_to_mailbox(to, msg, team_name).await {
        Ok(()) => {
            let result = serde_json::json!({
                "success": true,
                "message": format!("Message sent to {to}'s inbox"),
                "routing": {
                    "sender": crate::swarm::TEAM_LEAD_NAME,
                    "target": format!("@{to}"),
                    "summary": summary,
                }
            });
            ExecutionResult::success(serde_json::to_string_pretty(&result).unwrap_or_default())
        }
        Err(e) => ExecutionResult::failure(format!("Failed to send message: {e}")),
    }
}
