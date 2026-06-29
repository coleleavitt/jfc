use anyhow::{Context, Result};

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct AuthEndpointPolicy {
    pub allow_custom: bool,
    pub allow_insecure: bool,
}

pub(crate) fn validate_auth_base_url_with_policy(
    base: &str,
    policy: AuthEndpointPolicy,
) -> Result<()> {
    let url = reqwest::Url::parse(base).context("invalid voice auth endpoint URL")?;
    let scheme = url.scheme();
    let host = url.host_str().unwrap_or("");

    if scheme != "wss" && !(policy.allow_custom && policy.allow_insecure && scheme == "ws") {
        anyhow::bail!(
            "refusing voice auth endpoint with scheme `{scheme}`; use wss:// or set voice.allow_insecure_auth_endpoint = true with voice.allow_custom_auth_endpoint = true"
        );
    }
    if !is_anthropic_host(host) && !policy.allow_custom {
        anyhow::bail!(
            "refusing non-Anthropic voice auth endpoint `{host}`; set voice.allow_custom_auth_endpoint = true for trusted local testing"
        );
    }
    Ok(())
}

pub(crate) fn validate_auth_http_base_url_with_policy(
    base: &str,
    policy: AuthEndpointPolicy,
) -> Result<()> {
    let url = reqwest::Url::parse(base).context("invalid voice auth endpoint URL")?;
    let scheme = url.scheme();
    let host = url.host_str().unwrap_or("");

    if scheme != "https" && !(policy.allow_custom && policy.allow_insecure && scheme == "http") {
        anyhow::bail!(
            "refusing voice auth endpoint with scheme `{scheme}`; use https:// or set voice.allow_insecure_auth_endpoint = true with voice.allow_custom_auth_endpoint = true"
        );
    }
    if !is_anthropic_host(host) && !policy.allow_custom {
        anyhow::bail!(
            "refusing non-Anthropic voice auth endpoint `{host}`; set voice.allow_custom_auth_endpoint = true for trusted local testing"
        );
    }
    Ok(())
}

fn is_anthropic_host(host: &str) -> bool {
    host == "api.anthropic.com" || host.ends_with(".anthropic.com")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_auth_base_url_accepts_anthropic_wss_normal() {
        validate_auth_base_url_with_policy(
            "wss://api.anthropic.com",
            AuthEndpointPolicy::default(),
        )
        .unwrap();
    }

    #[test]
    fn validate_auth_base_url_rejects_custom_without_opt_in_robust() {
        let err = validate_auth_base_url_with_policy(
            "wss://example.invalid",
            AuthEndpointPolicy::default(),
        )
        .unwrap_err();

        assert!(err.to_string().contains("non-Anthropic"));
    }

    #[test]
    fn validate_auth_base_url_rejects_cleartext_without_opt_in_robust() {
        let err = validate_auth_base_url_with_policy(
            "ws://api.anthropic.com",
            AuthEndpointPolicy::default(),
        )
        .unwrap_err();

        assert!(err.to_string().contains("scheme"));
    }

    #[test]
    fn validate_auth_http_base_url_accepts_anthropic_https_normal() {
        validate_auth_http_base_url_with_policy(
            "https://api.anthropic.com",
            AuthEndpointPolicy::default(),
        )
        .unwrap();
    }

    #[test]
    fn validate_auth_http_base_url_rejects_custom_without_opt_in_robust() {
        let err = validate_auth_http_base_url_with_policy(
            "https://example.invalid",
            AuthEndpointPolicy::default(),
        )
        .unwrap_err();

        assert!(err.to_string().contains("non-Anthropic"));
    }

    #[test]
    fn validate_auth_http_base_url_rejects_cleartext_without_opt_in_robust() {
        let err = validate_auth_http_base_url_with_policy(
            "http://api.anthropic.com",
            AuthEndpointPolicy::default(),
        )
        .unwrap_err();

        assert!(err.to_string().contains("scheme"));
    }

    #[test]
    fn policy_allows_trusted_custom_endpoint_normal() {
        let policy = AuthEndpointPolicy {
            allow_custom: true,
            allow_insecure: false,
        };

        validate_auth_base_url_with_policy("wss://example.invalid", policy).unwrap();
        validate_auth_http_base_url_with_policy("https://example.invalid", policy).unwrap();
    }
}
