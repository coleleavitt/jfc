//! Managed Agents webhook verification.
//!
//! Anthropic's Go SDK exposes this as `Beta.Webhooks.Unwrap`: verify the
//! Standard Webhooks headers, then decode the managed-agents event payload.

use crate::error::{Error, Result};
use base64::Engine as _;
use base64::engine::general_purpose::{STANDARD, URL_SAFE, URL_SAFE_NO_PAD};
use hmac::{Hmac, KeyInit, Mac};
use reqwest::header::HeaderMap;
use serde::Deserialize;
use sha2::Sha256;
use std::collections::BTreeMap;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

type HmacSha256 = Hmac<Sha256>;

const DEFAULT_TOLERANCE: Duration = Duration::from_secs(5 * 60);

#[derive(Debug, Clone)]
pub struct WebhookService {
    webhook_key: String,
    tolerance: Duration,
}

impl WebhookService {
    pub fn new(webhook_key: impl Into<String>) -> Self {
        Self {
            webhook_key: webhook_key.into(),
            tolerance: DEFAULT_TOLERANCE,
        }
    }

    pub fn with_tolerance(mut self, tolerance: Duration) -> Self {
        self.tolerance = tolerance;
        self
    }

    pub fn verify(&self, payload: &[u8], headers: &HeaderMap) -> Result<()> {
        verify_standard_webhook(&self.webhook_key, payload, headers, self.tolerance)
    }

    pub fn unwrap(&self, payload: &[u8], headers: &HeaderMap) -> Result<WebhookEvent> {
        self.verify(payload, headers)?;
        Ok(serde_json::from_slice(payload)?)
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct WebhookEvent {
    pub id: String,
    #[serde(rename = "type")]
    pub type_: String,
    pub created_at: String,
    pub data: WebhookEventData,
}

#[derive(Debug, Clone, Deserialize)]
pub struct WebhookEventData {
    #[serde(default)]
    pub id: Option<String>,
    #[serde(rename = "type")]
    pub type_: String,
    #[serde(default)]
    pub organization_id: Option<String>,
    #[serde(default)]
    pub workspace_id: Option<String>,
    #[serde(default)]
    pub vault_id: Option<String>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, serde_json::Value>,
}

impl WebhookEventData {
    pub fn is_session_outcome_evaluation_ended(&self) -> bool {
        self.type_ == "session.outcome_evaluation_ended"
    }
}

fn verify_standard_webhook(
    webhook_key: &str,
    payload: &[u8],
    headers: &HeaderMap,
    tolerance: Duration,
) -> Result<()> {
    let msg_id = header_str(headers, "webhook-id")?;
    let timestamp = header_str(headers, "webhook-timestamp")?;
    let signature = header_str(headers, "webhook-signature")?;
    verify_timestamp(timestamp, tolerance)?;

    let secret = decode_secret(webhook_key)?;
    let mut signed = Vec::with_capacity(msg_id.len() + timestamp.len() + payload.len() + 2);
    signed.extend_from_slice(msg_id.as_bytes());
    signed.push(b'.');
    signed.extend_from_slice(timestamp.as_bytes());
    signed.push(b'.');
    signed.extend_from_slice(payload);

    for sig in signature_candidates(signature) {
        if let Ok(decoded) = decode_signature(sig) {
            let mut mac = HmacSha256::new_from_slice(&secret)
                .map_err(|_| Error::Authentication("invalid webhook key".to_owned()))?;
            mac.update(&signed);
            if mac.verify_slice(&decoded).is_ok() {
                return Ok(());
            }
        }
    }

    Err(Error::Authentication(
        "webhook signature verification failed".to_owned(),
    ))
}

fn header_str<'a>(headers: &'a HeaderMap, name: &str) -> Result<&'a str> {
    headers
        .get(name)
        .and_then(|v| v.to_str().ok())
        .filter(|v| !v.is_empty())
        .ok_or_else(|| Error::Authentication(format!("missing {name} webhook header")))
}

fn verify_timestamp(timestamp: &str, tolerance: Duration) -> Result<()> {
    let ts = timestamp
        .parse::<u64>()
        .map_err(|_| Error::Authentication("invalid webhook timestamp".to_owned()))?;
    let event_time = UNIX_EPOCH + Duration::from_secs(ts);
    let now = SystemTime::now();
    let age = if now >= event_time {
        now.duration_since(event_time).unwrap_or_default()
    } else {
        event_time.duration_since(now).unwrap_or_default()
    };
    if age > tolerance {
        return Err(Error::Authentication(
            "webhook timestamp outside tolerance".to_owned(),
        ));
    }
    Ok(())
}

fn decode_secret(webhook_key: &str) -> Result<Vec<u8>> {
    let raw = webhook_key.strip_prefix("whsec_").unwrap_or(webhook_key);
    decode_base64(raw).map_err(|_| Error::Authentication("invalid webhook key".to_owned()))
}

fn decode_signature(signature: &str) -> std::result::Result<Vec<u8>, base64::DecodeError> {
    decode_base64(signature)
}

fn decode_base64(input: &str) -> std::result::Result<Vec<u8>, base64::DecodeError> {
    STANDARD
        .decode(input)
        .or_else(|_| URL_SAFE.decode(input))
        .or_else(|_| URL_SAFE_NO_PAD.decode(input))
}

fn signature_candidates(header: &str) -> impl Iterator<Item = &str> {
    header
        .split_whitespace()
        .flat_map(|chunk| chunk.split(','))
        .filter_map(|part| {
            let part = part.trim();
            if part.is_empty() {
                return None;
            }
            Some(
                part.strip_prefix("v1=")
                    .unwrap_or(part.strip_prefix("v1,").unwrap_or(part)),
            )
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use reqwest::header::{HeaderMap, HeaderValue};

    #[test]
    fn unwrap_accepts_valid_standard_webhook_signature() {
        let key = "whsec_c2VjcmV0Cg==";
        let payload = br#"{"id":"wevt_1","created_at":"2026-03-15T10:00:00Z","data":{"id":"sesn_1","organization_id":"org_1","type":"session.status_idled","workspace_id":"wrk_1"},"type":"event"}"#;
        let msg_id = "msg_1";
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs()
            .to_string();
        let secret = decode_secret(key).unwrap();
        let mut mac = HmacSha256::new_from_slice(&secret).unwrap();
        mac.update(format!("{msg_id}.{timestamp}.").as_bytes());
        mac.update(payload);
        let sig = STANDARD.encode(mac.finalize().into_bytes());

        let mut headers = HeaderMap::new();
        headers.insert("webhook-id", HeaderValue::from_static("msg_1"));
        headers.insert(
            "webhook-timestamp",
            HeaderValue::from_str(&timestamp).unwrap(),
        );
        headers.insert(
            "webhook-signature",
            HeaderValue::from_str(&format!("v1,{sig}")).unwrap(),
        );

        let service = WebhookService::new(key);
        let event = service.unwrap(payload, &headers).unwrap();
        assert_eq!(event.data.type_, "session.status_idled");
    }
}
