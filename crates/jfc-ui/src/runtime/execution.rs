use std::path::PathBuf;

use crate::{attachments::Attachment, types::DiffView};

#[derive(Debug, Clone)]
pub struct ExecutionResult {
    pub output: String,
    pub outcome: ToolOutcome,
    pub diagnostics: Vec<ToolDiagnostic>,
    pub provenance: Option<ToolProvenance>,
    /// When set, the renderer prefers this structured diff over
    /// `output`/`Text`. Used by Edit (and Write-as-overwrite) to surface
    /// a colorized diff in the transcript instead of a flat
    /// "file updated successfully" string.
    pub diff: Option<DiffView>,
    /// Binary attachments produced by this tool (e.g. the PDF the
    /// Read tool just loaded). The event-loop `ToolResult` handler
    /// promotes these onto the owning assistant message's
    /// `.attachments` field so the per-message ownership model carries
    /// them to the provider. Replaces the previous
    /// `push_pending_tool_attachment` global queue — per-message
    /// ownership means cross-session/cross-stream leaks are impossible
    /// by construction.
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

    pub fn with_provenance(mut self, provenance: ToolProvenance) -> Self {
        self.provenance = Some(provenance);
        self
    }

    pub fn with_diff(mut self, diff: DiffView) -> Self {
        self.diff = Some(diff);
        self
    }

    /// Attach one or more binary attachments to this tool result.
    /// The event-loop handler will move them onto the owning assistant
    /// message at ToolResult time so they're serialized as
    /// `ProviderContent::Attachment` blocks on the next request.
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
}

impl ToolDiagnostic {
    fn error(message: impl Into<String>) -> Self {
        Self {
            level: DiagnosticLevel::Error,
            message: message.into(),
            help: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
pub enum DiagnosticLevel {
    Error,
    Warning,
    Help,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolProvenance {
    pub cwd: PathBuf,
    pub source: ToolSource,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
pub enum ToolSource {
    ModelRequested,
    LocalExecutor,
}
