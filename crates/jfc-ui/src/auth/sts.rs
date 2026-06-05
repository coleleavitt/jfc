//! AWS STS AssumeRoleWithWebIdentity — used for Bedrock auth when
//! `CLAUDE_CODE_USE_BEDROCK=1` is set.
//!
//! This implements the minimal STS call needed to exchange a web identity
//! token (e.g. from an OIDC provider) for temporary AWS credentials that
//! can sign Bedrock requests.
//!
//! Flow:
//! 1. Read `AWS_WEB_IDENTITY_TOKEN_FILE` (or accept token directly)
//! 2. POST to `https://sts.amazonaws.com/` with Action=AssumeRoleWithWebIdentity
//! 3. Parse XML response for AccessKeyId, SecretAccessKey, SessionToken, Expiration
//! 4. Return `Credentials` struct ready for SigV4 signing

use anyhow::{Context, Result, bail};
use chrono::{DateTime, Utc};
use serde::Serialize;
use std::time::Duration;

/// Temporary AWS credentials from an STS AssumeRole call.
#[derive(Debug, Clone, Serialize)]
pub struct Credentials {
    pub access_key_id: String,
    pub secret_access_key: String,
    pub session_token: String,
    pub expiration: DateTime<Utc>,
}

impl Credentials {
    /// Whether the credentials have expired (with a 5-minute buffer).
    pub fn is_expired(&self) -> bool {
        Utc::now() + chrono::Duration::minutes(5) >= self.expiration
    }

    /// Duration until expiration (0 if already expired).
    pub fn time_remaining(&self) -> Duration {
        let remaining = self.expiration - Utc::now();
        if remaining.num_seconds() <= 0 {
            Duration::ZERO
        } else {
            Duration::from_secs(remaining.num_seconds() as u64)
        }
    }
}

/// Default session duration in seconds (1 hour).
const DEFAULT_DURATION_SECONDS: u32 = 3600;

/// STS endpoint (global).
const STS_ENDPOINT: &str = "https://sts.amazonaws.com";

/// Perform AssumeRoleWithWebIdentity against AWS STS.
///
/// # Arguments
/// - `role_arn`: The ARN of the role to assume (e.g. `arn:aws:iam::123456789:role/BedrockAccess`)
/// - `web_identity_token`: The OIDC/OAuth token to exchange
/// - `session_name`: A name for the session (used in CloudTrail logs)
///
/// # Returns
/// Temporary `Credentials` valid for `DEFAULT_DURATION_SECONDS`.
pub async fn assume_role_with_web_identity(
    role_arn: &str,
    web_identity_token: &str,
    session_name: &str,
) -> Result<Credentials> {
    assume_role_with_web_identity_opts(
        role_arn,
        web_identity_token,
        session_name,
        DEFAULT_DURATION_SECONDS,
        None,
    )
    .await
}

/// Full-options variant with configurable duration and endpoint override.
pub async fn assume_role_with_web_identity_opts(
    role_arn: &str,
    web_identity_token: &str,
    session_name: &str,
    duration_seconds: u32,
    endpoint_override: Option<&str>,
) -> Result<Credentials> {
    let endpoint = endpoint_override.unwrap_or(STS_ENDPOINT);

    let params = [
        ("Action", "AssumeRoleWithWebIdentity"),
        ("Version", "2011-06-15"),
        ("RoleArn", role_arn),
        ("RoleSessionName", session_name),
        ("WebIdentityToken", web_identity_token),
        ("DurationSeconds", &duration_seconds.to_string()),
    ];

    // Manually encode form body since reqwest may not have the `form` feature
    let body = params
        .iter()
        .map(|(k, v)| format!("{}={}", urlencoded(k), urlencoded(v)))
        .collect::<Vec<_>>()
        .join("&");

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .context("build HTTP client for STS")?;

    let resp = client
        .post(endpoint)
        .header("content-type", "application/x-www-form-urlencoded")
        .body(body)
        .send()
        .await
        .context("STS AssumeRoleWithWebIdentity request")?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        bail!(
            "STS AssumeRoleWithWebIdentity failed (HTTP {}): {}",
            status,
            body
        );
    }

    let body = resp.text().await.context("read STS response body")?;
    parse_assume_role_response(&body)
}

/// Convenience: read the web identity token from the file path specified
/// by `AWS_WEB_IDENTITY_TOKEN_FILE` env var, then call assume_role.
pub async fn assume_role_from_env(role_arn: &str, session_name: &str) -> Result<Credentials> {
    let token_file = std::env::var("AWS_WEB_IDENTITY_TOKEN_FILE")
        .context("AWS_WEB_IDENTITY_TOKEN_FILE not set")?;
    let token = tokio::fs::read_to_string(&token_file)
        .await
        .with_context(|| format!("read web identity token from {token_file}"))?;
    assume_role_with_web_identity(role_arn, token.trim(), session_name).await
}

/// Parse the XML response from STS AssumeRoleWithWebIdentity.
///
/// Response shape (simplified):
/// ```xml
/// <AssumeRoleWithWebIdentityResponse>
///   <AssumeRoleWithWebIdentityResult>
///     <Credentials>
///       <AccessKeyId>...</AccessKeyId>
///       <SecretAccessKey>...</SecretAccessKey>
///       <SessionToken>...</SessionToken>
///       <Expiration>2023-01-01T00:00:00Z</Expiration>
///     </Credentials>
///   </AssumeRoleWithWebIdentityResult>
/// </AssumeRoleWithWebIdentityResponse>
/// ```
fn parse_assume_role_response(xml: &str) -> Result<Credentials> {
    // Minimal XML extraction — avoids pulling in a full XML parser dep.
    let access_key_id =
        extract_xml_value(xml, "AccessKeyId").context("missing AccessKeyId in STS response")?;
    let secret_access_key = extract_xml_value(xml, "SecretAccessKey")
        .context("missing SecretAccessKey in STS response")?;
    let session_token =
        extract_xml_value(xml, "SessionToken").context("missing SessionToken in STS response")?;
    let expiration_str =
        extract_xml_value(xml, "Expiration").context("missing Expiration in STS response")?;

    let expiration = expiration_str
        .parse::<DateTime<Utc>>()
        .with_context(|| format!("parse Expiration timestamp: {expiration_str}"))?;

    Ok(Credentials {
        access_key_id,
        secret_access_key,
        session_token,
        expiration,
    })
}

/// Extract the text content between `<tag>` and `</tag>` from XML.
fn extract_xml_value(xml: &str, tag: &str) -> Option<String> {
    let open = format!("<{tag}>");
    let close = format!("</{tag}>");
    let start = xml.find(&open)? + open.len();
    let end = xml[start..].find(&close)? + start;
    Some(xml[start..end].trim().to_string())
}

/// Minimal percent-encoding for application/x-www-form-urlencoded values.
fn urlencoded(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char);
            }
            b' ' => out.push('+'),
            _ => {
                out.push('%');
                out.push(char::from(HEX[(b >> 4) as usize]));
                out.push(char::from(HEX[(b & 0x0f) as usize]));
            }
        }
    }
    out
}

const HEX: &[u8; 16] = b"0123456789ABCDEF";

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Datelike;

    #[test]
    fn test_parse_assume_role_response() {
        let xml = r#"
<AssumeRoleWithWebIdentityResponse xmlns="https://sts.amazonaws.com/doc/2011-06-15/">
  <AssumeRoleWithWebIdentityResult>
    <Credentials>
      <AccessKeyId>ASIAXYZ123</AccessKeyId>
      <SecretAccessKey>wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY</SecretAccessKey>
      <SessionToken>FwoGZXIvYXdzEBYaDKq...</SessionToken>
      <Expiration>2025-01-15T12:00:00Z</Expiration>
    </Credentials>
  </AssumeRoleWithWebIdentityResult>
</AssumeRoleWithWebIdentityResponse>"#;

        let creds = parse_assume_role_response(xml).unwrap();
        assert_eq!(creds.access_key_id, "ASIAXYZ123");
        assert_eq!(
            creds.secret_access_key,
            "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY"
        );
        assert_eq!(creds.session_token, "FwoGZXIvYXdzEBYaDKq...");
        assert_eq!(creds.expiration.year(), 2025);
    }

    #[test]
    fn test_extract_xml_value() {
        let xml = "<Root><Foo>bar</Foo></Root>";
        assert_eq!(extract_xml_value(xml, "Foo"), Some("bar".to_string()));
        assert_eq!(extract_xml_value(xml, "Missing"), None);
    }

    #[test]
    fn test_credentials_expiration() {
        let creds = Credentials {
            access_key_id: "test".into(),
            secret_access_key: "test".into(),
            session_token: "test".into(),
            expiration: Utc::now() + chrono::Duration::hours(1),
        };
        assert!(!creds.is_expired());
        assert!(creds.time_remaining() > Duration::from_secs(3000));

        let expired = Credentials {
            expiration: Utc::now() - chrono::Duration::hours(1),
            ..creds
        };
        assert!(expired.is_expired());
        assert_eq!(expired.time_remaining(), Duration::ZERO);
    }
}
