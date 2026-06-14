//! Fallback sandbox for non-Linux platforms (no-op).

use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Debug, Clone, Default)]
pub struct SandboxPolicy;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SandboxResult {
    Applied,
    Unsupported,
    Failed(String),
}

impl SandboxPolicy {
    pub fn new() -> Self {
        Self
    }

    pub fn allow_write(self, _: impl Into<PathBuf>) -> Self {
        self
    }

    pub fn allow_read(self, _: impl Into<PathBuf>) -> Self {
        self
    }

    pub fn block_network(self) -> Self {
        self
    }

    pub fn allow_exec(self, _: impl Into<PathBuf>) -> Self {
        self
    }

    pub fn is_path_allowed(&self, _: &Path, _: bool) -> bool {
        true
    }

    pub fn is_network_blocked(&self) -> bool {
        false
    }

    pub fn apply_to_command(&self, _: &mut Command) -> SandboxResult {
        SandboxResult::Unsupported
    }

    pub fn economy_solver(_: &Path) -> Self {
        Self
    }
}
