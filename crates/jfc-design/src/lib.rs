//! `jfc-design` — the design-artifact product layer for JFC.
//!
//! This crate provides the native (pure-Rust) foundation that the `jfc-design`
//! skill pack drives:
//!
//! - [`project`] — design projects on disk (`.jfc/design/projects/<id>/`), a
//!   sandboxed file store, and an asset registry.
//! - [`inline`] — the standalone-HTML inliner (the native `super_inline_html`):
//!   inlines every referenced asset into a single offline-capable file.
//! - [`handoff`] — the developer-handoff package generator.
//! - [`design_system`] — the design-system indexer/compiler (`styles.css` closure,
//!   tokens, components, `@dsCard` / `@startingPoint` discovery + validation).
//! - [`capabilities`] — the Claude Design → JFC parity matrix, as data.
//! - [`server`] — a dependency-light localhost preview server (static files + a
//!   small JSON API) so HTML designs render with their relative asset paths intact.
//!
//! The browser-host capabilities (`eval_js`, `save_screenshot`, `gen_pptx`,
//! direct-edit inspection, verifier capture, and `.dc.html` streaming metadata)
//! live behind the `server-api` feature so the native core stays small for CLI
//! use and grows browser dependencies only for the design workspace server.

#[cfg(feature = "server-api")]
pub mod api;
#[cfg(feature = "server-api")]
pub mod browser;
pub mod capabilities;
pub mod design_system;
pub mod handoff;
pub mod inline;
pub mod media;
pub mod project;
pub mod server;
pub mod source_map;

mod mime;

/// Errors produced by the design layer.
#[derive(Debug, thiserror::Error)]
pub enum DesignError {
    #[error("io error at {path}: {source}")]
    Io {
        path: String,
        #[source]
        source: std::io::Error,
    },

    #[error("path escapes the project sandbox: {0}")]
    PathEscape(String),

    #[error("project not found: {0}")]
    ProjectNotFound(String),

    #[error("invalid project metadata: {0}")]
    BadMetadata(String),

    #[error("bundle error: {0}")]
    Bundle(String),

    #[error("browser host error: {0}")]
    Browser(String),

    #[error(transparent)]
    Json(#[from] serde_json::Error),
}

/// Convenience result alias for the crate.
pub type Result<T> = std::result::Result<T, DesignError>;

/// Helper for attaching a path to a raw [`std::io::Error`].
pub(crate) fn io_err(path: impl AsRef<std::path::Path>, source: std::io::Error) -> DesignError {
    DesignError::Io {
        path: path.as_ref().display().to_string(),
        source,
    }
}
