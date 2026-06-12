use std::path::Path;

use super::ExecutionResult;

pub async fn execute_team_member_mode(
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

pub async fn execute_team_create(
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

pub async fn execute_team_delete(active_team_name: Option<&str>) -> ExecutionResult {
    use crate::swarm::team_helpers;

    let team_name = match active_team_name {
        Some(name) => name,
        None => {
            return ExecutionResult::failure(
                "No active team. Use TeamCreate first to establish a team.",
            );
        }
    };

    // The tool contract says "must terminate all teammates first" — enforce
    // it. Deleting the team dirs under live teammates left them polling
    // deleted mailboxes (orphaned workers, mailbox NotFound noise) while a
    // subsequent TeamCreate with the same name silently adopted the debris.
    let active = team_helpers::get_active_teammates(team_name).await;
    if !active.is_empty() {
        let names: Vec<&str> = active.iter().map(|m| m.name.as_str()).collect();
        return ExecutionResult::failure(format!(
            "Cannot delete team \"{team_name}\": {} teammate(s) still active ({}). \
             Terminate them first, then retry TeamDelete.",
            active.len(),
            names.join(", ")
        ));
    }

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

tokio::task_local! {
    /// Identity of the agent currently executing a tool. Set by the
    /// teammate runner around its `execute_tool` call so `SendMessage`
    /// writes `from = <teammate-name>` instead of the hardcoded
    /// `team-lead`. Unset on the leader's own tool dispatch path, where
    /// `team-lead` IS the correct sender.
    pub static CURRENT_AGENT_NAME: String;
}

/// Public accessor for the task-local current-agent name. Returns the
/// stored name if a teammate runner set it for this task scope, else
/// `None`. Lives in `tools::swarm` because `CURRENT_AGENT_NAME` is the
/// only piece of teammate identity the tool layer ever needs to read.
pub fn current_agent_name() -> Option<String> {
    CURRENT_AGENT_NAME.try_with(|name| name.clone()).ok()
}

pub async fn execute_send_message(
    to: &str,
    message: &str,
    summary: Option<&str>,
    active_team_name: Option<&str>,
) -> ExecutionResult {
    use crate::swarm::mailbox;
    use crate::swarm::types::MailboxMessage;

    let team_name = active_team_name.unwrap_or(crate::swarm::DEFAULT_TEAM_NAME);
    // Use the task-local identity if a teammate runner set it; otherwise
    // fall back to the leader. Without this, every teammate's
    // `SendMessage` (which goes through `execute_tool` like any other
    // tool) wrote `from = team-lead`, so the leader couldn't tell which
    // teammate actually messaged it and inbox routing collapsed.
    let from = current_agent_name().unwrap_or_else(|| crate::swarm::TEAM_LEAD_NAME.to_owned());

    // Validate the recipient against the team roster before writing.
    // Without this, a message to an exited (or mistyped) teammate is
    // persisted to a mailbox nobody will ever read and the sender is
    // told "success" — silent message loss.
    if to != crate::swarm::TEAM_LEAD_NAME
        && let Some(team_file) = crate::swarm::team_helpers::read_team_file(team_name).await
    {
        let known = team_file.members.iter().any(|m| m.name == to);
        let active = team_file
            .members
            .iter()
            .any(|m| m.name == to && m.is_active != Some(false));
        if !known {
            let roster: Vec<&str> = team_file.members.iter().map(|m| m.name.as_str()).collect();
            return ExecutionResult::failure(format!(
                "Unknown recipient '{to}' — no such teammate in team '{team_name}'. \
                 Known members: {}",
                roster.join(", ")
            ));
        }
        if !active {
            return ExecutionResult::failure(format!(
                "Teammate '{to}' has exited; the message would never be read. \
                 Spawn a new teammate or message the team lead instead."
            ));
        }
    }

    let msg = MailboxMessage {
        from: from.clone(),
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
                    "sender": from,
                    "target": format!("@{to}"),
                    "summary": summary,
                }
            });
            ExecutionResult::success(serde_json::to_string_pretty(&result).unwrap_or_default())
        }
        Err(e) => ExecutionResult::failure(format!("Failed to send message: {e}")),
    }
}
