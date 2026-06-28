use jfc_plugin_sdk::HookName;
use thiserror::Error;

#[derive(Debug, Error)]
#[non_exhaustive]
pub enum PluginHostError {
    #[error("duplicate plugin id: {plugin_id}")]
    DuplicatePluginId { plugin_id: String },
    #[error(
        "duplicate {descriptor_kind} descriptor id {descriptor_id}: {first_plugin_id} conflicts with {duplicate_plugin_id}"
    )]
    DuplicateDescriptorId {
        descriptor_kind: String,
        descriptor_id: String,
        first_plugin_id: String,
        duplicate_plugin_id: String,
    },
    #[error("plugin not found: {plugin_id}")]
    PluginNotFound { plugin_id: String },
    #[error("plugin activation failed for {plugin_id}: {message}")]
    ActivationFailed { plugin_id: String, message: String },
    #[error("hook {hook} failed for {plugin_id}: {message}")]
    HookFailed {
        plugin_id: String,
        hook: HookName,
        message: String,
    },
    #[error("finalizer failed for {plugin_id}: {message}")]
    FinalizerFailed { plugin_id: String, message: String },
    #[error("plugin reported error: {message}")]
    PluginReported { message: String },
}

impl PluginHostError {
    pub fn plugin(message: impl Into<String>) -> Self {
        Self::PluginReported {
            message: message.into(),
        }
    }
}
