//! Interactive `/login bedrock` setup, mirroring the shape of v2.1.132's
//! `tengu_oauth_bedrock_wizard`.
//!
//! Splitting this wizard into its own module keeps [`super::bedrock`] focused
//! on the runtime [`crate::provider::Provider`] surface and concentrates all
//! the user-facing prose / pre-flight checks here.
//!
//! The wizard is implemented as a *non-interactive* state machine: the
//! [`BedrockWizard`] struct is fed plain input strings via
//! [`BedrockWizard::advance`]. That choice is deliberate — we plan to drive
//! it from both the slash-command console (`/login bedrock`) and a future
//! TUI panel; both surfaces share these step transitions.

#![allow(dead_code)]

use std::path::PathBuf;

use super::bedrock::{
    BedrockConfig, DEFAULT_REGION, aws_cli_available, check_caller_identity, default_config_path,
    save_config,
};

/// Each step matches a discrete user prompt the wizard surfaces.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WizardStep {
    /// First check: is the AWS CLI on `$PATH`? When false we exit early
    /// pointing the user at the install docs.
    DetectAwsCli,
    /// Prompt for region. Empty string ⇒ accept [`DEFAULT_REGION`].
    PromptRegion,
    /// Prompt for the named profile (optional). Empty ⇒ rely on default.
    PromptProfile,
    /// Prompt for an inference-profile ARN (optional).
    PromptInferenceProfile,
    /// Run `aws sts get-caller-identity` to confirm credentials.
    VerifyCredentials,
    /// Persist [`BedrockConfig`] to `~/.config/jfc/bedrock.toml`.
    Persist,
    /// Terminal: success.
    Done {
        config_path: PathBuf,
        identity: String,
    },
    /// Terminal: failure.
    Failed { reason: String },
}

pub struct BedrockWizard {
    config: BedrockConfig,
    config_path: PathBuf,
    step: WizardStep,
    identity: Option<String>,
}

impl BedrockWizard {
    pub fn new() -> Self {
        Self {
            config: BedrockConfig::default(),
            config_path: default_config_path(),
            step: WizardStep::DetectAwsCli,
            identity: None,
        }
    }

    /// For tests: override the destination path so we don't clobber a real
    /// `~/.config/jfc/bedrock.toml` during `cargo test`.
    pub fn with_config_path(mut self, path: PathBuf) -> Self {
        self.config_path = path;
        self
    }

    pub fn current_step(&self) -> &WizardStep {
        &self.step
    }

    pub fn config(&self) -> &BedrockConfig {
        &self.config
    }

    /// Force the wizard into an arbitrary step. Tests use this to bypass the
    /// CLI-detect probe so they don't depend on whether `aws` is installed.
    /// Production code never calls this directly.
    pub fn force_step(&mut self, step: WizardStep) {
        self.step = step;
    }

    /// Advance one step. `input` is the user's reply for prompts (empty =
    /// accept default). For non-prompt steps the input is ignored.
    pub fn advance(&mut self, input: &str) -> WizardStep {
        let next = match &self.step {
            WizardStep::DetectAwsCli => {
                if aws_cli_available() {
                    WizardStep::PromptRegion
                } else {
                    WizardStep::Failed {
                        reason: "The AWS CLI (`aws`) was not found on $PATH. Install it from \
                                 https://aws.amazon.com/cli/ and try again."
                            .to_owned(),
                    }
                }
            }
            WizardStep::PromptRegion => {
                let trimmed = input.trim();
                self.config.region = if trimmed.is_empty() {
                    Some(DEFAULT_REGION.to_owned())
                } else {
                    Some(trimmed.to_owned())
                };
                WizardStep::PromptProfile
            }
            WizardStep::PromptProfile => {
                let trimmed = input.trim();
                self.config.profile = if trimmed.is_empty() {
                    None
                } else {
                    Some(trimmed.to_owned())
                };
                WizardStep::PromptInferenceProfile
            }
            WizardStep::PromptInferenceProfile => {
                let trimmed = input.trim();
                self.config.inference_profile = if trimmed.is_empty() {
                    None
                } else {
                    Some(trimmed.to_owned())
                };
                WizardStep::VerifyCredentials
            }
            WizardStep::VerifyCredentials => match check_caller_identity(&self.config) {
                Ok(arn) => {
                    self.identity = Some(arn);
                    WizardStep::Persist
                }
                Err(e) => WizardStep::Failed {
                    reason: format!(
                        "Could not verify AWS credentials: {e}. Try `aws sso login` first, \
                         or check that AWS_ACCESS_KEY_ID/AWS_SECRET_ACCESS_KEY are exported."
                    ),
                },
            },
            WizardStep::Persist => match save_config(&self.config_path, &self.config) {
                Ok(()) => WizardStep::Done {
                    config_path: self.config_path.clone(),
                    identity: self.identity.clone().unwrap_or_default(),
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

impl Default for BedrockWizard {
    fn default() -> Self {
        Self::new()
    }
}

/// Terminal-friendly summary for the slash-command response panel.
pub fn render_step(step: &WizardStep) -> String {
    match step {
        WizardStep::DetectAwsCli => "Checking for the AWS CLI…".to_owned(),
        WizardStep::PromptRegion => {
            format!("Enter AWS region [default: {DEFAULT_REGION}]:")
        }
        WizardStep::PromptProfile => "Enter named profile (blank for default):".to_owned(),
        WizardStep::PromptInferenceProfile => {
            "Enter inference-profile ARN (blank to skip):".to_owned()
        }
        WizardStep::VerifyCredentials => "Verifying credentials with STS…".to_owned(),
        WizardStep::Persist => "Saving configuration…".to_owned(),
        WizardStep::Done {
            config_path,
            identity,
        } => format!(
            "Bedrock configured. Identity: {}. Wrote {}",
            if identity.is_empty() {
                "(unknown)"
            } else {
                identity.as_str()
            },
            config_path.display()
        ),
        WizardStep::Failed { reason } => format!("Bedrock setup failed: {reason}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    // Normal: every step variant renders to a non-empty string. The slash
    // command UI relies on this to never blank out mid-wizard.
    #[test]
    fn render_every_step_non_empty_normal() {
        let steps = [
            WizardStep::DetectAwsCli,
            WizardStep::PromptRegion,
            WizardStep::PromptProfile,
            WizardStep::PromptInferenceProfile,
            WizardStep::VerifyCredentials,
            WizardStep::Persist,
            WizardStep::Done {
                config_path: PathBuf::from("/tmp/x.toml"),
                identity: "arn:aws:iam::123:user/dev".into(),
            },
            WizardStep::Failed {
                reason: "boom".into(),
            },
        ];
        for s in steps {
            assert!(!render_step(&s).is_empty());
        }
    }

    // Normal: a blank region prompt fills in the default. Mirrors the
    // "press enter to accept default" UX.
    #[test]
    fn region_prompt_uses_default_when_empty_normal() {
        let dir = tempdir().unwrap();
        let mut w = BedrockWizard::new().with_config_path(dir.path().join("bedrock.toml"));
        w.force_step(WizardStep::PromptRegion);
        let next = w.advance("");
        assert_eq!(next, WizardStep::PromptProfile);
        assert_eq!(w.config().region.as_deref(), Some(DEFAULT_REGION));
    }

    // Normal: a non-empty region overrides the default.
    #[test]
    fn region_prompt_accepts_custom_normal() {
        let dir = tempdir().unwrap();
        let mut w = BedrockWizard::new().with_config_path(dir.path().join("bedrock.toml"));
        w.force_step(WizardStep::PromptRegion);
        w.advance("eu-central-1");
        assert_eq!(w.config().region.as_deref(), Some("eu-central-1"));
    }

    // Normal: profile is optional — set when provided, omitted when blank.
    #[test]
    fn profile_prompt_optional_normal() {
        let dir = tempdir().unwrap();
        let mut w = BedrockWizard::new().with_config_path(dir.path().join("bedrock.toml"));
        w.force_step(WizardStep::PromptProfile);
        w.advance("dev");
        assert_eq!(w.config().profile.as_deref(), Some("dev"));

        let mut w2 = BedrockWizard::new().with_config_path(dir.path().join("b2.toml"));
        w2.force_step(WizardStep::PromptProfile);
        w2.advance("");
        assert!(w2.config().profile.is_none());
    }

    // Normal: inference profile prompt threads through to VerifyCredentials.
    #[test]
    fn inference_profile_prompt_advances_normal() {
        let dir = tempdir().unwrap();
        let mut w = BedrockWizard::new().with_config_path(dir.path().join("bedrock.toml"));
        w.force_step(WizardStep::PromptInferenceProfile);
        let next = w.advance("arn:aws:bedrock:us-west-2:1:inference-profile/x");
        assert_eq!(next, WizardStep::VerifyCredentials);
        assert_eq!(
            w.config().inference_profile.as_deref(),
            Some("arn:aws:bedrock:us-west-2:1:inference-profile/x")
        );
    }

    // Robust: terminal Done/Failed steps refuse to rewind on further
    // advance() calls. Prevents UI bugs where double-clicking re-enters
    // a stale wizard.
    #[test]
    fn terminal_step_is_idempotent_robust() {
        let mut w = BedrockWizard::new();
        w.force_step(WizardStep::Done {
            config_path: PathBuf::from("/tmp/x.toml"),
            identity: "arn".into(),
        });
        let next = w.advance("ignored input");
        assert!(matches!(next, WizardStep::Done { .. }));
    }

    // Robust: failed step is also terminal — extra input doesn't accidentally
    // step back into the prompts.
    #[test]
    fn failed_step_is_idempotent_robust() {
        let mut w = BedrockWizard::new();
        w.force_step(WizardStep::Failed {
            reason: "no aws cli".into(),
        });
        let next = w.advance("");
        assert!(matches!(next, WizardStep::Failed { .. }));
    }
}
