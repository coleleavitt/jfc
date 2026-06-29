//! Tool execution result types.
//!
//! These live in `jfc-core` so both `jfc-tools` and `jfc` can produce
//! and consume them without circular dependencies.

use std::path::PathBuf;

use crate::{Attachment, DiffView};

#[derive(Debug, Clone)]
pub struct ExecutionResult {
    pub output: String,
    pub outcome: ToolOutcome,
    pub diagnostics: Vec<ToolDiagnostic>,
    pub provenance: Option<ToolProvenance>,
    pub diff: Option<DiffView>,
    pub attachments: Vec<Attachment>,
}

impl ExecutionResult {
    pub fn success(output: impl Into<String>) -> Self {
        Self {
            output: output.into(),
            outcome: ToolOutcome::Success,
            diagnostics: Vec::new(),
            provenance: None,
            diff: None,
            attachments: Vec::new(),
        }
    }

    pub fn failure(output: impl Into<String>) -> Self {
        let output = output.into();
        Self {
            diagnostics: vec![ToolDiagnostic::error(output.clone())],
            output,
            outcome: ToolOutcome::Failed,
            provenance: None,
            diff: None,
            attachments: Vec::new(),
        }
    }

    pub fn structured_failure(
        output: impl Into<String>,
        error_category: ToolErrorCategory,
        retryable: bool,
    ) -> Self {
        let output = output.into();
        Self {
            diagnostics: vec![ToolDiagnostic::structured_error(
                output.clone(),
                error_category,
                retryable,
            )],
            output,
            outcome: ToolOutcome::Failed,
            provenance: None,
            diff: None,
            attachments: Vec::new(),
        }
    }

    pub fn with_provenance(mut self, provenance: ToolProvenance) -> Self {
        self.provenance = Some(provenance);
        self
    }

    pub fn with_diff(mut self, diff: DiffView) -> Self {
        self.diff = Some(diff);
        self
    }

    pub fn with_attachments(mut self, atts: Vec<Attachment>) -> Self {
        self.attachments = atts;
        self
    }

    pub fn is_error(&self) -> bool {
        matches!(self.outcome, ToolOutcome::Failed)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolOutcome {
    Success,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolDiagnostic {
    pub level: DiagnosticLevel,
    pub message: String,
    pub help: Option<String>,
    pub error_category: Option<ToolErrorCategory>,
    pub retryable: Option<bool>,
}

impl ToolDiagnostic {
    pub fn error(message: impl Into<String>) -> Self {
        Self {
            level: DiagnosticLevel::Error,
            message: message.into(),
            help: None,
            error_category: None,
            retryable: None,
        }
    }

    pub fn structured_error(
        message: impl Into<String>,
        error_category: ToolErrorCategory,
        retryable: bool,
    ) -> Self {
        Self {
            level: DiagnosticLevel::Error,
            message: message.into(),
            help: None,
            error_category: Some(error_category),
            retryable: Some(retryable),
        }
    }

    pub fn with_help(mut self, help: impl Into<String>) -> Self {
        self.help = Some(help.into());
        self
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiagnosticLevel {
    Error,
    Warning,
    Help,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolErrorCategory {
    Validation,
    Permission,
    Transient,
    Business,
    Configuration,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolProvenance {
    pub cwd: PathBuf,
    pub source: ToolSource,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToolSource {
    ModelRequested,
    LocalExecutor,
    Plugin { plugin_id: String },
}
