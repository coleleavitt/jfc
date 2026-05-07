//! Thin bridge between jfc-ui and the `jfc-anthropic-sdk` crate.
//!
//! Mostly a single source of truth for constructing an SDK `Client` from
//! whatever credentials the user has configured (env, OAuth, vault),
//! plus opt-in helpers that swap inline attachments for FileID
//! references when payload size makes that worthwhile.
//!
//! Most jfc workflows go through the existing `provider.rs` abstraction
//! (which talks the Anthropic Messages API directly via reqwest). This
//! module is for the *managed-agents* / *files* / *batches* / *skills*
//! surfaces — features that v132 has but jfc-ui's main streaming path
//! doesn't need.

use jfc_anthropic_sdk::Client;

/// Above this size (in bytes), inline attachments should be uploaded
/// via the Files API and referenced by FileID instead. Saves prompt
/// tokens and lets payloads exceed the inline base64 ceiling. v132 uses
/// 100KB as the same threshold per the SDK audit.
pub const FILE_UPLOAD_THRESHOLD_BYTES: usize = 100 * 1024;

/// Build an SDK Client from the active credential profile. Returns
/// `None` when no Anthropic API key is configured (the SDK requires
/// one — OAuth-only sessions go through `provider.rs` instead).
pub fn build_client() -> Option<Client> {
    let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
    let profile = crate::credential_vault::active_profile(&cwd);
    let api_key = crate::credential_vault::api_key("anthropic", profile.as_deref())?;
    Some(Client::with_api_key(api_key))
}

/// Decide whether an attachment should go through the Files API instead
/// of being inlined. Currently a simple size check.
pub fn should_upload(att: &crate::attachments::Attachment) -> bool {
    att.bytes.len() >= FILE_UPLOAD_THRESHOLD_BYTES
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn upload_threshold_is_100kb_normal() {
        assert_eq!(FILE_UPLOAD_THRESHOLD_BYTES, 100 * 1024);
    }
}
