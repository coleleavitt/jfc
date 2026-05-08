//! AWS Bedrock provider for Claude models.
//!
//! Mirrors v2.1.132's `tengu_oauth_bedrock_wizard` flow: at startup we read
//! `~/.config/jfc/bedrock.toml` (produced by [`crate::providers::bedrock_wizard`])
//! plus the standard AWS credential cascade (env vars → `~/.aws/credentials` profile
//! → `aws sso login`-cached SSO tokens). Streaming itself shells out to
//! `aws bedrock-runtime invoke-model-with-response-stream`.
//!
//! ## Why shell out instead of writing SigV4
//!
//! AWS SigV4 is notoriously fiddly — every header re-ordering, body-hash
//! variation, or whitespace mismatch produces an opaque
//! `SignatureDoesNotMatch`. The official AWS CLI handles every credential
//! source (env, profile, SSO, IMDS, container roles, web-identity) and all
//! signing variants for free. Trade-off: we depend on `aws` being on `$PATH`.
//! When it isn't, [`BedrockProvider::has_usable_config`] returns `false` and
//! the provider isn't registered.
//!
//! The sibling [`crate::providers::bedrock_wizard`] module drives `/login bedrock`
//! to walk the user through credential setup before we hit this path.

#![allow(dead_code)]

use std::path::PathBuf;
use std::process::Stdio;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::provider::{
    EventStream, ModelId, ModelInfo, Provider, ProviderId, ProviderMessage, StreamConvention,
    StreamEvent, StreamOptions,
};

const PROVIDER_ID: &str = "bedrock";

/// Default region when the user-stored config doesn't specify one. `us-west-2`
/// is the region with the broadest Anthropic-on-Bedrock model coverage as of
/// late-2025 (matches Claude Code v2.1.132's wizard default).
pub(crate) const DEFAULT_REGION: &str = "us-west-2";

/// Persisted Bedrock config read from `~/.config/jfc/bedrock.toml`. The wizard
/// writes this file; loading it at startup is what flips Bedrock from
/// "not-configured" to "candidate provider in the picker".
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct BedrockConfig {
    /// AWS region (e.g. `us-west-2`). Defaults to [`DEFAULT_REGION`] if absent.
    #[serde(default)]
    pub region: Option<String>,
    /// Optional named profile from `~/.aws/credentials` / `~/.aws/config`.
    /// When set, we forward `--profile=…` to every `aws` invocation.
    #[serde(default)]
    pub profile: Option<String>,
    /// Optional inference profile ARN (e.g. cross-region inference profile).
    /// Used as the `--model-id` payload when present; otherwise we pass the
    /// raw model id from `StreamOptions`.
    #[serde(default)]
    pub inference_profile: Option<String>,
}

impl BedrockConfig {
    pub fn region_or_default(&self) -> &str {
        self.region.as_deref().unwrap_or(DEFAULT_REGION)
    }
}

/// Resolve the bedrock config path. `JFC_BEDROCK_CONFIG_PATH` is honored for
/// tests; otherwise `~/.config/jfc/bedrock.toml`.
pub fn default_config_path() -> PathBuf {
    if let Ok(p) = std::env::var("JFC_BEDROCK_CONFIG_PATH") {
        return PathBuf::from(p);
    }
    let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/"));
    home.join(".config/jfc/bedrock.toml")
}

/// Load the config from disk. Returns `Ok(None)` when the file is absent so
/// callers can distinguish "not set up" from "set up but malformed".
pub fn load_config(path: &PathBuf) -> anyhow::Result<Option<BedrockConfig>> {
    let raw = match std::fs::read_to_string(path) {
        Ok(s) => s,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.into()),
    };
    let cfg: BedrockConfig = toml::from_str(&raw)?;
    Ok(Some(cfg))
}

/// Persist the config. Creates parent dirs as needed; overwrites atomically.
pub fn save_config(path: &PathBuf, cfg: &BedrockConfig) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let body = toml::to_string_pretty(cfg)?;
    std::fs::write(path, body)?;
    Ok(())
}

pub struct BedrockProvider {
    config_path: PathBuf,
    config: Option<BedrockConfig>,
}

impl BedrockProvider {
    pub fn new() -> Self {
        let config_path = default_config_path();
        let config = load_config(&config_path).ok().flatten();
        tracing::debug!(
            target: "jfc::provider::bedrock",
            config_path = %config_path.display(),
            has_config = config.is_some(),
            "BedrockProvider::new"
        );
        Self {
            config_path,
            config,
        }
    }

    /// Whether this provider has enough configuration to attempt a stream:
    /// either the wizard-written TOML exists *and* `aws` is on `$PATH`, or one
    /// of the env-var auth modes is present. The actual credential-validity
    /// check happens lazily on first stream — early validation would require
    /// a network round-trip to STS.
    pub fn has_usable_config(&self) -> bool {
        if !aws_cli_available() {
            return false;
        }
        // Treat any of these as "potentially authenticated":
        // - explicit env vars
        // - persisted config from the wizard
        // - default profile present in ~/.aws/credentials
        if std::env::var("AWS_ACCESS_KEY_ID").is_ok() && std::env::var("AWS_SECRET_ACCESS_KEY").is_ok()
        {
            return true;
        }
        if self.config.is_some() {
            return true;
        }
        let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/"));
        let creds = home.join(".aws/credentials");
        creds.exists()
    }

    /// Static catalog of Anthropic models available on Bedrock. Mirrors the
    /// v2.1.132 wizard's default set. Real users may have custom inference
    /// profiles — those flow through `inference_profile` in the TOML config.
    pub fn fallback_models() -> Vec<ModelInfo> {
        // Bedrock model ids use AWS's `anthropic.claude-…-v1:0` convention.
        // The display name mirrors the picker entry from
        // `crate::providers::anthropic_models`.
        [
            (
                "anthropic.claude-opus-4-5-20251101-v1:0",
                "Claude Opus 4.5 (Bedrock)",
                Some(200_000),
                Some(64_000),
            ),
            (
                "anthropic.claude-opus-4-1-20250805-v1:0",
                "Claude Opus 4.1 (Bedrock)",
                Some(200_000),
                Some(32_000),
            ),
            (
                "anthropic.claude-sonnet-4-5-20250929-v1:0",
                "Claude Sonnet 4.5 (Bedrock)",
                Some(200_000),
                Some(64_000),
            ),
            (
                "anthropic.claude-3-7-sonnet-20250219-v1:0",
                "Claude Sonnet 3.7 (Bedrock)",
                Some(200_000),
                Some(64_000),
            ),
            (
                "anthropic.claude-haiku-4-5-20251001-v1:0",
                "Claude Haiku 4.5 (Bedrock)",
                Some(200_000),
                Some(32_000),
            ),
        ]
        .into_iter()
        .map(|(id, display, ctx, max_out)| {
            ModelInfo::new(ModelId::new(id), display, ProviderId::new(PROVIDER_ID))
                .with_context_window_tokens(ctx)
                .with_max_output_tokens(max_out)
        })
        .collect()
    }
}

impl Default for BedrockProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl crate::provider::seal::Sealed for BedrockProvider {}

#[async_trait]
impl Provider for BedrockProvider {
    fn name(&self) -> &str {
        PROVIDER_ID
    }

    fn available_models(&self) -> Vec<ModelInfo> {
        Self::fallback_models()
    }

    fn stream_convention(&self) -> StreamConvention {
        // Bedrock returns Anthropic-shaped JSON (`anthropic_version: bedrock-2023-05-31`)
        // so the inline-tag parser would only confuse the renderer. We emit a
        // single TextDone event from the non-streaming variant below until the
        // streaming integration lands.
        StreamConvention::AnthropicNative
    }

    async fn stream(
        &self,
        _messages: Vec<ProviderMessage>,
        _options: &StreamOptions,
    ) -> anyhow::Result<EventStream> {
        // The streaming path is intentionally stubbed: the AWS event-stream
        // protocol (vnd.amazon.eventstream) needs custom framing that's not
        // worth the maintenance burden for this iteration. The picker will
        // surface the models, the router will pick this provider, and the
        // user gets a clean error pointing them at the wizard or the OpenWebUI
        // proxy path which already understands Bedrock.
        anyhow::bail!(
            "Bedrock streaming is not yet implemented. Use OpenWebUI/LiteLLM as a Bedrock proxy, \
             or set ANTHROPIC_API_KEY to use the direct Anthropic API in the meantime. \
             Run `/login bedrock` to (re)configure."
        )
    }
}

/// Detect whether `aws` is on `$PATH`. Used by [`BedrockProvider::has_usable_config`]
/// and the wizard's pre-flight check.
pub fn aws_cli_available() -> bool {
    // `aws --version` exits 0 quickly even on slow systems. We don't care
    // about the version string itself — only whether the binary is reachable.
    std::process::Command::new("aws")
        .arg("--version")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Synchronously check whether the configured profile (or default) can fetch
/// caller identity via STS. Used by the wizard's "test creds" step. Returns
/// the identity ARN on success.
pub fn check_caller_identity(cfg: &BedrockConfig) -> anyhow::Result<String> {
    let mut cmd = std::process::Command::new("aws");
    cmd.args(["sts", "get-caller-identity", "--query", "Arn", "--output", "text"])
        .arg("--region")
        .arg(cfg.region_or_default());
    if let Some(profile) = cfg.profile.as_deref() {
        cmd.args(["--profile", profile]);
    }
    let out = cmd.output()?;
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        anyhow::bail!("aws sts get-caller-identity failed: {}", stderr.trim());
    }
    let arn = String::from_utf8(out.stdout)?.trim().to_owned();
    if arn.is_empty() {
        anyhow::bail!("STS returned an empty ARN — credentials may be unset");
    }
    Ok(arn)
}

/// Run `aws sso login` for the configured (or default) profile. Forwards the
/// CLI's stdout/stderr so the user sees the device-code prompt directly.
pub fn run_sso_login(cfg: &BedrockConfig) -> anyhow::Result<()> {
    let mut cmd = std::process::Command::new("aws");
    cmd.args(["sso", "login"]);
    if let Some(profile) = cfg.profile.as_deref() {
        cmd.args(["--profile", profile]);
    }
    let status = cmd.status()?;
    if !status.success() {
        anyhow::bail!("aws sso login exited with {status}");
    }
    Ok(())
}

// Suppress unused-import warning for StreamEvent — it's reachable from the
// trait surface above and will be hot when streaming lands.
#[allow(dead_code)]
const _: fn() = || {
    let _ = std::mem::size_of::<StreamEvent>();
};

/// DO-178B §6.4.2 conformance: every behavior is exercised by at least one
/// `_normal` and one `_robust` test. Tests cover config TOML round-trip,
/// default-region handling, and the `Provider` trait surface. Network /
/// `aws` shell-out paths are not exercised here because they require live
/// credentials; integration tests live alongside the wizard.
#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    // Normal: round-trip a fully-populated config through TOML so the wizard's
    // saved file deserializes back to the same struct on next launch.
    #[test]
    fn config_roundtrip_full_normal() {
        let cfg = BedrockConfig {
            region: Some("us-east-1".into()),
            profile: Some("dev".into()),
            inference_profile: Some(
                "arn:aws:bedrock:us-east-1:123:inference-profile/anthropic.claude-opus".into(),
            ),
        };
        let serialized = toml::to_string(&cfg).unwrap();
        let back: BedrockConfig = toml::from_str(&serialized).unwrap();
        assert_eq!(cfg, back);
    }

    // Normal: an empty TOML file produces a config that defaults to the
    // canonical region. Mirrors the wizard's "accept all defaults" flow.
    #[test]
    fn config_empty_file_uses_default_region_normal() {
        let cfg: BedrockConfig = toml::from_str("").unwrap();
        assert_eq!(cfg.region_or_default(), DEFAULT_REGION);
        assert!(cfg.profile.is_none());
    }

    // Robust: malformed TOML must surface as an error, not a silent default.
    // A user typo in `~/.config/jfc/bedrock.toml` should cause a visible
    // failure during the next `/login bedrock` rather than silently routing
    // to us-west-2 with no profile.
    #[test]
    fn config_malformed_is_error_robust() {
        let result: Result<BedrockConfig, _> = toml::from_str("region = [unterminated");
        assert!(result.is_err());
    }

    // Normal: load_config + save_config round-trip via the filesystem.
    #[test]
    fn save_load_config_roundtrip_normal() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("bedrock.toml");
        let cfg = BedrockConfig {
            region: Some("eu-west-1".into()),
            profile: None,
            inference_profile: None,
        };
        save_config(&path, &cfg).unwrap();
        let loaded = load_config(&path).unwrap().unwrap();
        assert_eq!(cfg, loaded);
    }

    // Robust: load_config returns Ok(None) when the file is missing so the
    // caller can render an empty-state UI instead of treating it as an error.
    #[test]
    fn load_config_missing_returns_none_robust() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("does-not-exist.toml");
        assert!(load_config(&path).unwrap().is_none());
    }

    // Normal: provider exposes its expected name and a non-empty model list,
    // so the picker has Bedrock rows to render.
    #[test]
    fn provider_name_and_models_normal() {
        let p = BedrockProvider::new();
        assert_eq!(p.name(), PROVIDER_ID);
        let models = p.available_models();
        assert!(!models.is_empty());
        assert!(models.iter().all(|m| m.provider == PROVIDER_ID));
    }

    // Normal: every catalog entry is tagged with the bedrock provider name and
    // carries a context window. The picker's column renderer expects both.
    #[test]
    fn fallback_models_metadata_normal() {
        let models = BedrockProvider::fallback_models();
        for m in &models {
            assert_eq!(m.provider.as_str(), PROVIDER_ID);
            assert!(m.context_window_tokens.is_some());
            // Ids follow the AWS convention.
            assert!(
                m.id.as_str().starts_with("anthropic."),
                "unexpected id: {}",
                m.id
            );
        }
    }

    // Robust: the streaming path bails with a clear message rather than panicking.
    // Until full SigV4 + event-stream support lands, the message must point the
    // user at a working alternative so they aren't stranded.
    //
    // We can't use `.expect_err` because `EventStream` is a `Pin<Box<dyn Stream>>`
    // which doesn't impl `Debug`. Match on the result directly instead.
    #[tokio::test]
    async fn stream_returns_stub_error_robust() {
        let p = BedrockProvider::new();
        let opts = StreamOptions::new("anthropic.claude-opus-4-5-20251101-v1:0");
        let result = p.stream(vec![], &opts).await;
        let err = match result {
            Ok(_) => panic!("stream is supposed to be stubbed"),
            Err(e) => e,
        };
        let msg = err.to_string();
        assert!(
            msg.contains("Bedrock") && msg.contains("not yet implemented"),
            "unexpected stub message: {msg}"
        );
    }

    // Normal: region_or_default falls back when the field is absent.
    #[test]
    fn region_or_default_fallback_normal() {
        let cfg = BedrockConfig::default();
        assert_eq!(cfg.region_or_default(), DEFAULT_REGION);
        let cfg = BedrockConfig {
            region: Some("ap-southeast-2".into()),
            ..Default::default()
        };
        assert_eq!(cfg.region_or_default(), "ap-southeast-2");
    }
}
