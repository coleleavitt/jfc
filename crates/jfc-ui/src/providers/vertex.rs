//! GCP Vertex AI provider for Claude models.
//!
//! Mirrors v2.1.132's `tengu_oauth_vertex_wizard` flow: the wizard writes
//! `~/.config/jfc/vertex.toml`; at runtime we shell out to
//! `gcloud auth print-access-token` for an ephemeral OAuth bearer and POST it
//! to the Vertex Anthropic endpoint:
//!
//! ```text
//! https://{region}-aiplatform.googleapis.com/v1/projects/{project}/locations/{region}/publishers/anthropic/models/{model}:streamRawPredict
//! ```
//!
//! `streamRawPredict` accepts (and returns) the standard Anthropic Messages
//! API shape, so the existing [`super::sse`] helpers handle decoding without
//! a Vertex-specific code path.
//!
//! ## Why shell out to gcloud
//!
//! gcloud handles every credential variant (user OAuth, service account,
//! workload identity, impersonation, MFA-gated org policies) and refreshes
//! tokens automatically. Re-implementing that surface in Rust is far outside
//! the scope of this provider, and the wizard already detects gcloud as a
//! prerequisite. Trade-off: we depend on `gcloud` being on `$PATH`.

use std::path::PathBuf;
use std::process::Stdio;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::json;
use tokio::sync::Mutex;

use jfc_provider::{
    EventStream, ModelId, ModelInfo, Provider, ProviderId, ProviderMessage, StreamConvention,
    StreamOptions,
};

const PROVIDER_ID: &str = "vertex";
const ANTHROPIC_VERSION: &str = "vertex-2023-10-16";

/// Default region for Anthropic-on-Vertex. Mirrors the wizard's default.
pub(crate) const DEFAULT_REGION: &str = "us-central1";

/// Persisted Vertex config from `~/.config/jfc/vertex.toml`.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct VertexConfig {
    /// GCP project id (required for any real call). The wizard refuses to
    /// write the file without one.
    pub project: Option<String>,
    /// Vertex region (defaults to [`DEFAULT_REGION`]).
    #[serde(default)]
    pub region: Option<String>,
}

impl VertexConfig {
    pub fn region_or_default(&self) -> &str {
        self.region.as_deref().unwrap_or(DEFAULT_REGION)
    }

    /// True only when project is set — the wizard validates this, but we
    /// re-check at startup so a manually-truncated file degrades cleanly.
    pub fn is_complete(&self) -> bool {
        self.project
            .as_deref()
            .is_some_and(|p| !p.trim().is_empty())
    }
}

pub fn default_config_path() -> PathBuf {
    if let Ok(p) = std::env::var("JFC_VERTEX_CONFIG_PATH") {
        return PathBuf::from(p);
    }
    let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/"));
    home.join(".config/jfc/vertex.toml")
}

pub fn load_config(path: &PathBuf) -> anyhow::Result<Option<VertexConfig>> {
    let raw = match std::fs::read_to_string(path) {
        Ok(s) => s,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.into()),
    };
    let cfg: VertexConfig = toml::from_str(&raw)?;
    Ok(Some(cfg))
}

#[allow(dead_code)]
pub fn save_config(path: &PathBuf, cfg: &VertexConfig) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let body = toml::to_string_pretty(cfg)?;
    std::fs::write(path, body)?;
    Ok(())
}

/// Cached gcloud access token. Vertex tokens last ~1h; we cache for 50min so
/// a long stream doesn't 401 mid-response when the token rotates under us.
struct TokenCache {
    token: String,
    fetched_at: Instant,
}

impl TokenCache {
    fn is_fresh(&self) -> bool {
        self.fetched_at.elapsed() < Duration::from_secs(50 * 60)
    }
}

pub struct VertexProvider {
    client: reqwest::Client,
    #[allow(dead_code)]
    config_path: PathBuf,
    config: Option<VertexConfig>,
    token_cache: Mutex<Option<TokenCache>>,
    /// Override for tests — when set we return this string instead of shelling out.
    /// Production code never sets this.
    test_token_override: Option<String>,
}

impl VertexProvider {
    pub fn new() -> Self {
        let config_path = default_config_path();
        let config = load_config(&config_path).ok().flatten();
        tracing::debug!(
            target: "jfc::provider::vertex",
            config_path = %config_path.display(),
            has_config = config.is_some(),
            "VertexProvider::new"
        );
        Self {
            client: jfc_provider::http::streaming_client(),
            config_path,
            config,
            token_cache: Mutex::new(None),
            test_token_override: None,
        }
    }

    /// True when (a) gcloud is on PATH and (b) the wizard has stored a config
    /// with a project id. We don't pre-flight a real token here — the user
    /// might be temporarily offline and we should still surface Vertex models
    /// in the picker for cosmetic continuity.
    pub fn has_usable_config(&self) -> bool {
        if !gcloud_cli_available() {
            return false;
        }
        match &self.config {
            Some(cfg) => cfg.is_complete(),
            None => false,
        }
    }

    /// Resolve an access token, using the in-memory cache when fresh.
    async fn access_token(&self) -> anyhow::Result<String> {
        if let Some(t) = &self.test_token_override {
            return Ok(t.clone());
        }
        {
            let guard = self.token_cache.lock().await;
            if let Some(c) = guard.as_ref() {
                if c.is_fresh() {
                    return Ok(c.token.clone());
                }
            }
        }
        let fresh = fetch_gcloud_token()?;
        let mut guard = self.token_cache.lock().await;
        *guard = Some(TokenCache {
            token: fresh.clone(),
            fetched_at: Instant::now(),
        });
        Ok(fresh)
    }

    pub fn fallback_models() -> Vec<ModelInfo> {
        // Vertex model ids strip the AWS-style version suffix and use the
        // bare anthropic name (e.g. `claude-opus-4-5@20251101`).
        [
            (
                "claude-opus-4-5@20251101",
                "Claude Opus 4.5 (Vertex)",
                Some(1_000_000),
                Some(64_000),
            ),
            (
                "claude-opus-4-1@20250805",
                "Claude Opus 4.1 (Vertex)",
                Some(200_000),
                Some(32_000),
            ),
            (
                "claude-sonnet-4-5@20250929",
                "Claude Sonnet 4.5 (Vertex)",
                Some(200_000),
                Some(64_000),
            ),
            (
                "claude-3-7-sonnet@20250219",
                "Claude Sonnet 3.7 (Vertex)",
                Some(200_000),
                Some(64_000),
            ),
            (
                "claude-haiku-4-5@20251001",
                "Claude Haiku 4.5 (Vertex)",
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

    fn endpoint_url(&self, model: &str) -> anyhow::Result<String> {
        let cfg = self
            .config
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("Vertex is not configured. Run `/login vertex`."))?;
        let project = cfg
            .project
            .as_deref()
            .ok_or_else(|| anyhow::anyhow!("Vertex config is missing the GCP project id."))?;
        let region = cfg.region_or_default();
        Ok(format!(
            "https://{region}-aiplatform.googleapis.com/v1/projects/{project}/locations/{region}/publishers/anthropic/models/{model}:streamRawPredict"
        ))
    }
}

impl Default for VertexProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl jfc_provider::seal::Sealed for VertexProvider {}

#[async_trait]
impl Provider for VertexProvider {
    fn name(&self) -> &str {
        PROVIDER_ID
    }

    fn available_models(&self) -> Vec<ModelInfo> {
        Self::fallback_models()
    }

    fn stream_convention(&self) -> StreamConvention {
        StreamConvention::AnthropicNative
    }

    async fn stream(
        &self,
        messages: Vec<ProviderMessage>,
        options: &StreamOptions,
    ) -> anyhow::Result<EventStream> {
        let token = self.access_token().await?;
        let url = self.endpoint_url(options.model.as_str())?;

        // Vertex requires `anthropic_version: vertex-2023-10-16` and forbids
        // the regular `model` field in the body (the model is in the URL).
        let mut body = json!({
            "anthropic_version": ANTHROPIC_VERSION,
            "max_tokens": options.max_tokens,
            "stream": true,
            "messages": super::sse::build_messages(&messages),
        });
        if let Some(sys) = &options.system {
            body["system"] = json!(sys);
        }
        if let Some(temp) = options.temperature {
            body["temperature"] = serde_json::Value::from(temp);
        }
        if let Some(top_p) = options.top_p {
            body["top_p"] = serde_json::Value::from(top_p);
        }
        if !options.tools.is_empty() {
            body["tools"] = super::sse::build_tools(&options.tools);
        }
        if options.adaptive_thinking {
            let mut thinking = json!({ "type": "adaptive" });
            if let Some(display) = options.thinking_display.as_deref() {
                thinking["display"] = json!(display);
            }
            body["thinking"] = thinking;
        } else if let Some(budget) = options.thinking_budget {
            body["thinking"] = json!({ "type": "enabled", "budget_tokens": budget });
        }
        for (key, value) in &options.provider_options {
            body[key] = value.clone();
        }

        tracing::debug!(
            target: "jfc::provider::vertex",
            url = %url,
            model = %options.model,
            messages = messages.len(),
            "POST streamRawPredict"
        );
        let resp = self
            .client
            .post(&url)
            .bearer_auth(token)
            .header("content-type", "application/json")
            .header("accept", "text/event-stream")
            .json(&body)
            .send()
            .await?;
        let status = resp.status();
        if !status.is_success() {
            let text = resp.text().await.unwrap_or_default();
            anyhow::bail!("Vertex API error {status}: {text}");
        }
        Ok(super::sse::into_event_stream(resp))
    }
}

/// Detect whether `gcloud` is on `$PATH`. Used by [`VertexProvider::has_usable_config`]
/// and the wizard's pre-flight check.
pub fn gcloud_cli_available() -> bool {
    std::process::Command::new("gcloud")
        .arg("--version")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Synchronously fetch a fresh access token from gcloud. Errors carry the
/// gcloud stderr verbatim so the user can act on `Reauth required` etc.
pub fn fetch_gcloud_token() -> anyhow::Result<String> {
    let out = std::process::Command::new("gcloud")
        .args(["auth", "print-access-token"])
        .output()?;
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        anyhow::bail!("`gcloud auth print-access-token` failed: {}", stderr.trim());
    }
    let token = String::from_utf8(out.stdout)?.trim().to_owned();
    if token.is_empty() {
        anyhow::bail!("gcloud returned an empty access token");
    }
    Ok(token)
}

/// Read the active gcloud project (`gcloud config get-value project`). Used
/// by the wizard to pre-fill the prompt with what the user likely wants.
#[allow(dead_code)]
pub fn fetch_gcloud_default_project() -> Option<String> {
    let out = std::process::Command::new("gcloud")
        .args(["config", "get-value", "project"])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8(out.stdout).ok()?.trim().to_owned();
    if s.is_empty() || s == "(unset)" {
        None
    } else {
        Some(s)
    }
}

/// DO-178B §6.4.2 conformance: `_normal` and `_robust` cases for config
/// round-trip, endpoint construction, gcloud token shell-out via PATH override.
#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::os::unix::fs::PermissionsExt;
    use tempfile::tempdir;

    // Normal: config TOML round-trips.
    #[test]
    fn config_roundtrip_full_normal() {
        let cfg = VertexConfig {
            project: Some("my-proj".into()),
            region: Some("europe-west1".into()),
        };
        let s = toml::to_string(&cfg).unwrap();
        let back: VertexConfig = toml::from_str(&s).unwrap();
        assert_eq!(cfg, back);
    }

    // Normal: empty config falls back to the default region but is incomplete.
    #[test]
    fn config_empty_uses_default_region_normal() {
        let cfg: VertexConfig = toml::from_str("").unwrap();
        assert_eq!(cfg.region_or_default(), DEFAULT_REGION);
        assert!(!cfg.is_complete());
    }

    // Robust: malformed TOML errors out instead of silently defaulting.
    #[test]
    fn config_malformed_is_error_robust() {
        let result: Result<VertexConfig, _> = toml::from_str("project = [unterminated");
        assert!(result.is_err());
    }

    // Normal: filesystem round-trip.
    #[test]
    fn save_load_roundtrip_normal() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("vertex.toml");
        let cfg = VertexConfig {
            project: Some("p".into()),
            region: Some("us-east5".into()),
        };
        save_config(&path, &cfg).unwrap();
        let loaded = load_config(&path).unwrap().unwrap();
        assert_eq!(cfg, loaded);
    }

    // Robust: load_config returns Ok(None) for a missing file.
    #[test]
    fn load_missing_is_none_robust() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("nope.toml");
        assert!(load_config(&path).unwrap().is_none());
    }

    // Normal: provider trait surface — name + non-empty model catalog.
    #[test]
    fn provider_name_and_models_normal() {
        let p = VertexProvider::new();
        assert_eq!(p.name(), PROVIDER_ID);
        let models = p.available_models();
        assert!(!models.is_empty());
        assert!(models.iter().all(|m| m.provider == PROVIDER_ID));
    }

    // Normal: every catalog entry uses the Vertex `name@version` convention.
    #[test]
    fn fallback_models_use_vertex_id_format_normal() {
        let models = VertexProvider::fallback_models();
        for m in &models {
            assert!(
                m.id.as_str().contains('@'),
                "Vertex ids carry a `@version` suffix; got {}",
                m.id
            );
        }
    }

    // Normal: endpoint_url interpolates project, region, and model.
    #[test]
    fn endpoint_url_format_normal() {
        let mut p = VertexProvider::new();
        p.config = Some(VertexConfig {
            project: Some("acme-prod".into()),
            region: Some("us-central1".into()),
        });
        let url = p.endpoint_url("claude-opus-4-5@20251101").unwrap();
        assert_eq!(
            url,
            "https://us-central1-aiplatform.googleapis.com/v1/projects/acme-prod/locations/us-central1/publishers/anthropic/models/claude-opus-4-5@20251101:streamRawPredict"
        );
    }

    // Robust: building the URL with no config produces a clear error.
    #[test]
    fn endpoint_url_without_config_is_error_robust() {
        let p = VertexProvider::new();
        // We unconditionally clear config in case the test machine has one.
        let mut p2 = p;
        p2.config = None;
        let err = p2.endpoint_url("any").unwrap_err();
        assert!(
            err.to_string().contains("not configured"),
            "unexpected error: {err}"
        );
    }

    // Robust: building the URL when project is missing fails with a clear message.
    #[test]
    fn endpoint_url_without_project_is_error_robust() {
        let mut p = VertexProvider::new();
        p.config = Some(VertexConfig {
            project: None,
            region: Some("us-central1".into()),
        });
        let err = p.endpoint_url("any").unwrap_err();
        assert!(
            err.to_string().contains("project"),
            "unexpected error: {err}"
        );
    }

    // Normal: fetch_gcloud_token shells out to a stub binary on the PATH and
    // returns the trimmed stdout. We stage a fake `gcloud` script in a tempdir
    // and prepend it to $PATH so the call hits ours rather than the real CLI.
    //
    // This test is platform-conditional: we use a `#!/bin/sh` shebang which
    // requires a Unix shell. CI on macOS/Linux runs it; Windows skips.
    #[serial_test::serial]
    #[test]
    #[cfg(unix)]
    fn fetch_gcloud_token_shells_out_normal() {
        let dir = tempdir().unwrap();
        let stub = dir.path().join("gcloud");
        let mut f = std::fs::File::create(&stub).unwrap();
        // The stub echoes a fake token only for `auth print-access-token`.
        // For `--version` etc we exit success silently.
        writeln!(
            f,
            "#!/bin/sh\nif [ \"$1\" = \"auth\" ] && [ \"$2\" = \"print-access-token\" ]; then echo 'fake-test-token-abc'; exit 0; fi\nif [ \"$1\" = \"--version\" ]; then exit 0; fi\nexit 1"
        )
        .unwrap();
        drop(f);
        let mut perms = std::fs::metadata(&stub).unwrap().permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&stub, perms).unwrap();

        // Save and override PATH.
        let old_path = std::env::var("PATH").unwrap_or_default();
        let new_path = format!("{}:{}", dir.path().display(), old_path);
        // SAFETY: tests run sequentially within this module; we restore PATH
        // before returning. The harness is single-threaded for env-mutation
        // tests by default in cargo's test runner unless --test-threads is
        // increased — when it is, this test is the only one that mutates PATH
        // so the worst case is racey reads, not aliasing. The set_var call is
        // unsafe in 2024 edition because std cannot prove other threads
        // aren't reading $PATH simultaneously.
        unsafe {
            std::env::set_var("PATH", &new_path);
        }

        let result = fetch_gcloud_token();

        unsafe {
            std::env::set_var("PATH", &old_path);
        }

        let token = result.unwrap();
        assert_eq!(token, "fake-test-token-abc");
    }

    // Robust: when gcloud exits non-zero, fetch_gcloud_token surfaces stderr
    // so the user learns why (typically "Reauth required").
    #[serial_test::serial]
    #[test]
    #[cfg(unix)]
    fn fetch_gcloud_token_failure_propagates_stderr_robust() {
        let dir = tempdir().unwrap();
        let stub = dir.path().join("gcloud");
        let mut f = std::fs::File::create(&stub).unwrap();
        writeln!(f, "#!/bin/sh\necho 'ERROR: Reauth required.' 1>&2\nexit 1").unwrap();
        drop(f);
        let mut perms = std::fs::metadata(&stub).unwrap().permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&stub, perms).unwrap();

        let old_path = std::env::var("PATH").unwrap_or_default();
        let new_path = format!("{}:{}", dir.path().display(), old_path);
        unsafe {
            std::env::set_var("PATH", &new_path);
        }

        let result = fetch_gcloud_token();

        unsafe {
            std::env::set_var("PATH", &old_path);
        }

        let err = result.expect_err("stub exits non-zero");
        let msg = err.to_string();
        assert!(
            msg.contains("Reauth required") || msg.contains("failed"),
            "expected stderr to flow through; got: {msg}"
        );
    }
}
