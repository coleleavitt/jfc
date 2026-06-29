use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use hmac::{Hmac, KeyInit, Mac};
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use thiserror::Error;

use crate::model::TokenClaims;
use crate::time::now_ms;

type HmacSha256 = Hmac<Sha256>;

#[derive(Debug, Error)]
pub enum TokenError {
    #[error("invalid token")]
    Invalid,
    #[error("invalid token signature")]
    BadSignature,
    #[error("token expired")]
    Expired,
    #[error("token json error: {0}")]
    Json(#[from] serde_json::Error),
}

#[derive(Debug, Serialize)]
struct Header<'a> {
    alg: &'a str,
    typ: &'a str,
}

#[derive(Debug, Serialize, Deserialize)]
struct WireClaims {
    session_id: String,
    worker_id: String,
    exp_ms: u64,
}

pub fn mint_worker_token(secret: &[u8], claims: &TokenClaims) -> Result<String, TokenError> {
    let header = Header {
        alg: "HS256",
        typ: "JWT",
    };
    let wire = WireClaims {
        session_id: claims.session_id.clone(),
        worker_id: claims.worker_id.clone(),
        exp_ms: claims.exp_ms,
    };
    let header = URL_SAFE_NO_PAD.encode(serde_json::to_vec(&header)?);
    let payload = URL_SAFE_NO_PAD.encode(serde_json::to_vec(&wire)?);
    let signing_input = format!("{header}.{payload}");
    let sig = sign(secret, signing_input.as_bytes());
    Ok(format!(
        "{signing_input}.{}",
        URL_SAFE_NO_PAD.encode(sig.as_slice())
    ))
}

pub fn verify_worker_token(secret: &[u8], token: &str) -> Result<TokenClaims, TokenError> {
    let mut parts = token.split('.');
    let header = parts.next().ok_or(TokenError::Invalid)?;
    let payload = parts.next().ok_or(TokenError::Invalid)?;
    let sig = parts.next().ok_or(TokenError::Invalid)?;
    if parts.next().is_some() {
        return Err(TokenError::Invalid);
    }

    let signing_input = format!("{header}.{payload}");
    let expected = sign(secret, signing_input.as_bytes());
    let sig = URL_SAFE_NO_PAD
        .decode(sig.as_bytes())
        .map_err(|_| TokenError::Invalid)?;
    if sig != expected {
        return Err(TokenError::BadSignature);
    }

    let payload = URL_SAFE_NO_PAD
        .decode(payload.as_bytes())
        .map_err(|_| TokenError::Invalid)?;
    let wire: WireClaims = serde_json::from_slice(&payload)?;
    if wire.exp_ms <= now_ms() {
        return Err(TokenError::Expired);
    }
    Ok(TokenClaims {
        session_id: wire.session_id,
        worker_id: wire.worker_id,
        exp_ms: wire.exp_ms,
    })
}

fn sign(secret: &[u8], bytes: &[u8]) -> Vec<u8> {
    let mut mac = HmacSha256::new_from_slice(secret).expect("HMAC accepts any key length");
    mac.update(bytes);
    mac.finalize().into_bytes().to_vec()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn worker_token_roundtrip() {
        let claims = TokenClaims {
            session_id: "ses_1".to_owned(),
            worker_id: "wrk_1".to_owned(),
            exp_ms: now_ms() + 60_000,
        };
        let token = mint_worker_token(b"secret", &claims).unwrap();
        let decoded = verify_worker_token(b"secret", &token).unwrap();
        assert_eq!(decoded.session_id, "ses_1");
        assert_eq!(decoded.worker_id, "wrk_1");
    }

    #[test]
    fn worker_token_rejects_wrong_secret() {
        let claims = TokenClaims {
            session_id: "ses_1".to_owned(),
            worker_id: "wrk_1".to_owned(),
            exp_ms: now_ms() + 60_000,
        };
        let token = mint_worker_token(b"secret", &claims).unwrap();
        assert!(verify_worker_token(b"other", &token).is_err());
    }
}
