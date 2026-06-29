//! External ACP-agent profiles.
//!
//! Air treats Codex/Gemini/Junie as external ACP processes with their own
//! executable, auth, MCP gateway setup, model/mode state, and session lifecycle.
//! That is different from JFC's `jfc_provider::Provider`, which is a direct LLM
//! request client. Keep the distinction explicit so integrations like Junie do
//! not get modeled as ordinary model endpoints.

use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExternalAgentKind {
    GenericAcp,
    Codex,
    Gemini,
    Junie,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExternalAgentAuthMode {
    None,
    ApiKey,
    OAuth,
    JetBrainsLocalProxy,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExternalAgentEndpointKind {
    DirectLlmApi,
    LocalProxyToVendorBackend,
    LocalProcessOnly,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExternalAgentEndpointEvidence {
    pub kind: ExternalAgentEndpointKind,
    pub public_llm_endpoint_seen: bool,
    pub free_tier_evidence_seen: bool,
    pub notes: &'static str,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExternalAgentDistribution {
    pub stable_update_feed: Option<&'static str>,
    pub nightly_update_feed: Option<&'static str>,
    pub release_url_template: Option<&'static str>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExternalAgentProfile {
    pub kind: ExternalAgentKind,
    pub display_name: &'static str,
    pub executable_hint: &'static str,
    pub default_args: Vec<&'static str>,
    pub default_env: BTreeMap<&'static str, &'static str>,
    pub auth_mode: ExternalAgentAuthMode,
    pub endpoint_evidence: ExternalAgentEndpointEvidence,
    pub distribution: Option<ExternalAgentDistribution>,
    pub requires_mcp_gateway: bool,
    pub persists_model_and_mode: bool,
}

impl ExternalAgentProfile {
    pub fn junie_from_air() -> Self {
        let mut default_env = BTreeMap::new();
        default_env.insert("JUNIE_HOME", "<jfc-managed-junie-home>");
        default_env.insert("INGRAZZIO_URL", "<local-junie-api-proxy-url>");

        Self {
            kind: ExternalAgentKind::Junie,
            display_name: "Junie",
            executable_hint: "junie",
            default_args: vec![
                "--acp=true",
                "--mcp-default-locations=false",
                "--auth=<empty>",
            ],
            default_env,
            auth_mode: ExternalAgentAuthMode::JetBrainsLocalProxy,
            endpoint_evidence: ExternalAgentEndpointEvidence {
                kind: ExternalAgentEndpointKind::LocalProxyToVendorBackend,
                public_llm_endpoint_seen: false,
                free_tier_evidence_seen: false,
                notes: "Air starts Junie as an ACP process, sets INGRAZZIO_URL to a local proxy, and obtains JetBrains/JBA auth through the proxy path. The decompiled code shows distribution/auth plumbing, not a public Junie LLM API or free-tier contract.",
            },
            distribution: Some(ExternalAgentDistribution {
                stable_update_feed: Some(
                    "https://raw.githubusercontent.com/jetbrains-junie/junie/main/update-info.jsonl",
                ),
                nightly_update_feed: Some(
                    "https://raw.githubusercontent.com/jetbrains-junie/junie/main/update-info-nightly.jsonl",
                ),
                release_url_template: Some(
                    "https://github.com/JetBrains/junie/releases/download/{version}/junie-{channel}-{version}-{platform}.zip",
                ),
            }),
            requires_mcp_gateway: true,
            persists_model_and_mode: true,
        }
    }

    pub fn generic_acp(
        executable_hint: &'static str,
        default_args: Vec<&'static str>,
        default_env: BTreeMap<&'static str, &'static str>,
    ) -> Self {
        Self {
            kind: ExternalAgentKind::GenericAcp,
            display_name: "Generic ACP",
            executable_hint,
            default_args,
            default_env,
            auth_mode: ExternalAgentAuthMode::None,
            endpoint_evidence: ExternalAgentEndpointEvidence {
                kind: ExternalAgentEndpointKind::LocalProcessOnly,
                public_llm_endpoint_seen: false,
                free_tier_evidence_seen: false,
                notes: "Generic ACP launches a configured local process and exposes JFC MCP servers to it.",
            },
            distribution: None,
            requires_mcp_gateway: true,
            persists_model_and_mode: true,
        }
    }

    pub fn should_be_llm_provider(&self) -> bool {
        matches!(
            self.endpoint_evidence.kind,
            ExternalAgentEndpointKind::DirectLlmApi
        )
    }
}

pub mod launcher;
pub use launcher::{
    ExternalAgentHandle, ExternalAgentLaunchError, ExternalAgentSession, ExternalAgentSpec,
    ExternalAgentStatus, LaunchContext,
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn junie_profile_is_external_acp_not_llm_provider_normal() {
        let profile = ExternalAgentProfile::junie_from_air();
        assert_eq!(profile.kind, ExternalAgentKind::Junie);
        assert!(profile.requires_mcp_gateway);
        assert!(!profile.should_be_llm_provider());
        assert_eq!(
            profile.auth_mode,
            ExternalAgentAuthMode::JetBrainsLocalProxy
        );
        assert_eq!(
            profile.default_env.get("INGRAZZIO_URL"),
            Some(&"<local-junie-api-proxy-url>")
        );
        assert!(!profile.endpoint_evidence.free_tier_evidence_seen);
    }
}
