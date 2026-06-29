// Deprecated: merged into commands/inbox.rs to avoid split handlers.
// This module remains to preserve the public path but forwards to the new location.
use crate::commands::prelude::*;

pub async fn inject_inbox_reminder(state: &mut EngineState, session_id: &str) {
    super::inbox::inject_inbox_reminder(state, session_id).await;
}
