//! Interactive `/login vertex` setup, mirroring v2.1.132's
//! `tengu_oauth_vertex_wizard`.
//!
//! Same shape as [`super::bedrock_wizard`]: pure state machine driven by
//! [`VertexWizard::advance`] so the slash-command flow and a future TUI panel
//! can both reuse it.

use std::path::PathBuf;

use super::vertex::{
    DEFAULT_REGION, VertexConfig, default_config_path, fetch_gcloud_default_project,
    fetch_gcloud_token, gcloud_cli_available, save_config,
};

#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(dead_code)]
pub enum WizardStep {
    /// Probe whether `gcloud` is on `$PATH`. When false we exit with a clear
    /// message — there's no SDK fallback.
    DetectGcloud,
    /// Prompt for the GCP project id. Wizard pre-fills with
    /// `gcloud config get-value project` when available.
    PromptProject {
        suggested: Option<String>,
    },
    /// Prompt for the region. Empty ⇒ accept [`DEFAULT_REGION`].
    PromptRegion,
    /// Verify by acquiring an access token via `gcloud auth print-access-token`.
    VerifyToken,
    /// Persist the config to `~/.config/jfc/vertex.toml`.
    Persist,
    Done {
        config_path: PathBuf,
    },
    Failed {
        reason: String,
    },
}

#[allow(dead_code)]
pub struct VertexWizard {
    config: VertexConfig,
    config_path: PathBuf,
    step: WizardStep,
}

impl VertexWizard {
    #[allow(dead_code)]
    pub fn new() -> Self {
        Self {
            config: VertexConfig::default(),
            config_path: default_config_path(),
            step: WizardStep::DetectGcloud,
        }
    }

    #[allow(dead_code)]
    pub fn with_config_path(mut self, path: PathBuf) -> Self {
        self.config_path = path;
        self
    }

    #[allow(dead_code)]
    pub fn current_step(&self) -> &WizardStep {
        &self.step
    }

    #[allow(dead_code)]
    pub fn config(&self) -> &VertexConfig {
        &self.config
    }

    /// Tests bypass the gcloud probe by stepping directly into a prompt.
    #[allow(dead_code)]
    pub fn force_step(&mut self, step: WizardStep) {
        self.step = step;
    }

    #[allow(dead_code)]
    pub fn advance(&mut self, input: &str) -> WizardStep {
        let next = match &self.step {
            WizardStep::DetectGcloud => {
                if gcloud_cli_available() {
                    let suggested = fetch_gcloud_default_project();
                    WizardStep::PromptProject { suggested }
                } else {
                    WizardStep::Failed {
                        reason: "The Google Cloud CLI (`gcloud`) was not found on $PATH. \
                                 Install it from https://cloud.google.com/sdk/docs/install \
                                 and try again."
                            .to_owned(),
                    }
                }
            }
            WizardStep::PromptProject { suggested } => {
                let trimmed = input.trim();
                let chosen = if trimmed.is_empty() {
                    suggested.clone()
                } else {
                    Some(trimmed.to_owned())
                };
                match chosen {
                    Some(p) if !p.is_empty() => {
                        self.config.project = Some(p);
                        WizardStep::PromptRegion
                    }
                    _ => WizardStep::Failed {
                        reason: "GCP project id is required. Set one with \
                                 `gcloud config set project <id>` or pass it explicitly."
                            .to_owned(),
                    },
                }
            }
            WizardStep::PromptRegion => {
                let trimmed = input.trim();
                self.config.region = if trimmed.is_empty() {
                    Some(DEFAULT_REGION.to_owned())
                } else {
                    Some(trimmed.to_owned())
                };
                WizardStep::VerifyToken
            }
            WizardStep::VerifyToken => match fetch_gcloud_token() {
                Ok(_) => WizardStep::Persist,
                Err(e) => WizardStep::Failed {
                    reason: format!(
                        "Could not fetch a gcloud access token: {e}. Try `gcloud auth login` \
                         (user creds) or `gcloud auth activate-service-account` (CI)."
                    ),
                },
            },
            WizardStep::Persist => match save_config(&self.config_path, &self.config) {
                Ok(()) => WizardStep::Done {
                    config_path: self.config_path.clone(),
                },
                Err(e) => WizardStep::Failed {
                    reason: format!("Failed to write {}: {e}", self.config_path.display()),
                },
            },
            WizardStep::Done { .. } | WizardStep::Failed { .. } => self.step.clone(),
        };
        self.step = next.clone();
        next
    }
}

impl Default for VertexWizard {
    fn default() -> Self {
        Self::new()
    }
}

#[allow(dead_code)]
pub fn render_step(step: &WizardStep) -> String {
    match step {
        WizardStep::DetectGcloud => "Checking for the Google Cloud CLI…".to_owned(),
        WizardStep::PromptProject { suggested } => match suggested {
            Some(p) => format!("Enter GCP project id [default: {p}]:"),
            None => "Enter GCP project id:".to_owned(),
        },
        WizardStep::PromptRegion => format!("Enter Vertex region [default: {DEFAULT_REGION}]:"),
        WizardStep::VerifyToken => "Verifying gcloud credentials…".to_owned(),
        WizardStep::Persist => "Saving configuration…".to_owned(),
        WizardStep::Done { config_path } => {
            format!("Vertex configured. Wrote {}", config_path.display())
        }
        WizardStep::Failed { reason } => format!("Vertex setup failed: {reason}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    // Normal: rendering covers every step variant.
    #[test]
    fn render_every_step_non_empty_normal() {
        let steps = [
            WizardStep::DetectGcloud,
            WizardStep::PromptProject { suggested: None },
            WizardStep::PromptProject {
                suggested: Some("acme".into()),
            },
            WizardStep::PromptRegion,
            WizardStep::VerifyToken,
            WizardStep::Persist,
            WizardStep::Done {
                config_path: PathBuf::from("/tmp/v.toml"),
            },
            WizardStep::Failed {
                reason: "boom".into(),
            },
        ];
        for s in steps {
            assert!(!render_step(&s).is_empty());
        }
    }

    // Normal: an explicit project id flows into the config and advances.
    #[test]
    fn project_prompt_explicit_advances_normal() {
        let dir = tempdir().unwrap();
        let mut w = VertexWizard::new().with_config_path(dir.path().join("v.toml"));
        w.force_step(WizardStep::PromptProject { suggested: None });
        let next = w.advance("my-project-123");
        assert_eq!(next, WizardStep::PromptRegion);
        assert_eq!(w.config().project.as_deref(), Some("my-project-123"));
    }

    // Normal: a blank reply uses the suggested project from gcloud.
    #[test]
    fn project_prompt_uses_suggestion_normal() {
        let dir = tempdir().unwrap();
        let mut w = VertexWizard::new().with_config_path(dir.path().join("v.toml"));
        w.force_step(WizardStep::PromptProject {
            suggested: Some("auto-detected".into()),
        });
        let next = w.advance("");
        assert_eq!(next, WizardStep::PromptRegion);
        assert_eq!(w.config().project.as_deref(), Some("auto-detected"));
    }

    // Robust: a blank reply with no suggestion fails — project is mandatory.
    #[test]
    fn project_prompt_blank_with_no_suggestion_fails_robust() {
        let dir = tempdir().unwrap();
        let mut w = VertexWizard::new().with_config_path(dir.path().join("v.toml"));
        w.force_step(WizardStep::PromptProject { suggested: None });
        let next = w.advance("");
        assert!(matches!(next, WizardStep::Failed { .. }));
    }

    // Normal: blank region uses the default; non-blank overrides.
    #[test]
    fn region_prompt_default_or_custom_normal() {
        let dir = tempdir().unwrap();
        let mut w = VertexWizard::new().with_config_path(dir.path().join("v.toml"));
        w.force_step(WizardStep::PromptRegion);
        w.advance("");
        assert_eq!(w.config().region.as_deref(), Some(DEFAULT_REGION));

        let mut w2 = VertexWizard::new().with_config_path(dir.path().join("v2.toml"));
        w2.force_step(WizardStep::PromptRegion);
        w2.advance("europe-west4");
        assert_eq!(w2.config().region.as_deref(), Some("europe-west4"));
    }

    // Robust: terminal states are idempotent — doesn't accidentally rewind.
    #[test]
    fn terminal_states_idempotent_robust() {
        let mut w = VertexWizard::new();
        w.force_step(WizardStep::Done {
            config_path: PathBuf::from("/tmp/x.toml"),
        });
        assert!(matches!(w.advance(""), WizardStep::Done { .. }));

        let mut w2 = VertexWizard::new();
        w2.force_step(WizardStep::Failed {
            reason: "no gcloud".into(),
        });
        assert!(matches!(w2.advance(""), WizardStep::Failed { .. }));
    }
}
