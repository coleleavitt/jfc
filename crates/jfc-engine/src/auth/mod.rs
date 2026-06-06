//! Auth module — authentication helpers for various providers.
//!
//! Re-export shim: the implementations moved to the `jfc-auth` crate during
//! the engine extraction; these module paths are preserved until the final
//! shim-removal stage.

pub use jfc_auth::{device_flow, sts};
