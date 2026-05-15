//! Session catalog and path helpers.
//!
//! Full transcript serialization still lives in `jfc-ui` while message/tool
//! types are being untangled. This crate owns the provider-neutral session
//! index surface: paths, IDs, metadata listing, and picker helpers.

use std::path::PathBuf;

use jfc_core::SessionId;
use tracing::debug;

mod catalog;

pub use catalog::{
    SessionMetadata, cwd_mismatch_message, format_session_id_timestamp, group_by_cwd,
    list_session_ids_only, list_sessions, list_sessions_filtered, list_sessions_with_metadata,
    load_session_metadata, most_recent_session, most_recent_session_for_cwd, relative_time,
    shorten_cwd,
};

pub fn sessions_dir() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("jfc")
        .join("sessions")
}

pub fn generate_session_id() -> SessionId {
    let now = chrono::Utc::now();
    let id = SessionId::new(format!("ses_{}", now.format("%Y%m%d_%H%M%S")));
    debug!(target: "jfc::session", %id, "generated session id");
    id
}
