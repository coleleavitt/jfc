//! OpenWebUI JWT helpers — ports `opencode-openwebui-auth/src/oauth/jwt.ts`.
//!
//! The plugin uses the OpenWebUI server-issued HS256 JWT (set as the `token`
//! cookie after OIDC callback). Claims are checked client-side to detect
//! expiry before sending a request that would otherwise 401.

use serde::Deserialize;

/// Minimal JWT claim shape we care about. OpenWebUI tokens always include
/// `id` (the user UUID) and `exp` (Unix seconds). `jti` is optional.
#[derive(Debug, Clone, Deserialize)]
pub struct JwtClaims {
    pub id: String,
    pub exp: i64,
    #[serde(default)]
    pub jti: Option<String>,
}

/// Decode the JWT payload without verifying the signature (the OpenWebUI
/// server is the only thing that can verify HS256, and we only need the
/// claims for expiry/identity display). Returns None for non-JWT strings.
pub fn parse_jwt_claims(token: &str) -> Option<JwtClaims> {
    let parts: Vec<&str> = token.split('.').collect();
    if parts.len() != 3 {
        return None;
    }
    use base64::{Engine as _, engine::general_purpose};
    // JWT spec uses base64url WITHOUT padding.
    let payload = general_purpose::URL_SAFE_NO_PAD
        .decode(parts[1])
        .or_else(|_| general_purpose::URL_SAFE.decode(parts[1]))
        .ok()?;
    serde_json::from_slice(&payload).ok()
}

/// True when the token is invalid OR within `skew_ms` of expiring.
pub fn is_token_expired(token: &str, skew_ms: i64) -> bool {
    let Some(claims) = parse_jwt_claims(token) else {
        return true;
    };
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);
    now_ms + skew_ms >= claims.exp * 1000
}

/// Token's `exp` claim as Unix milliseconds, or None for invalid tokens.
pub fn token_expires_at_ms(token: &str) -> Option<i64> {
    parse_jwt_claims(token).map(|c| c.exp * 1000)
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::{Engine as _, engine::general_purpose};

    fn make_token(exp: i64) -> String {
        let header = general_purpose::URL_SAFE_NO_PAD
            .encode(br#"{"alg":"HS256","typ":"JWT"}"#);
        let payload_json = format!(r#"{{"id":"abc","exp":{exp}}}"#);
        let payload = general_purpose::URL_SAFE_NO_PAD.encode(payload_json.as_bytes());
        format!("{header}.{payload}.signature")
    }

    #[test]
    fn parse_jwt_claims_valid_normal() {
        let token = make_token(9_999_999_999);
        let claims = parse_jwt_claims(&token).expect("claims parse");
        assert_eq!(claims.id, "abc");
        assert_eq!(claims.exp, 9_999_999_999);
    }

    #[test]
    fn parse_jwt_claims_not_three_parts_returns_none_robust() {
        assert!(parse_jwt_claims("only.two").is_none());
        assert!(parse_jwt_claims("").is_none());
        assert!(parse_jwt_claims("a.b.c.d").is_none());
    }

    #[test]
    fn parse_jwt_claims_garbage_payload_returns_none_robust() {
        assert!(parse_jwt_claims("aaa.bbb.ccc").is_none());
    }

    #[test]
    fn is_token_expired_future_returns_false_normal() {
        let token = make_token(9_999_999_999);
        assert!(!is_token_expired(&token, 0));
    }

    #[test]
    fn is_token_expired_past_returns_true_normal() {
        let token = make_token(1);
        assert!(is_token_expired(&token, 0));
    }

    #[test]
    fn is_token_expired_invalid_returns_true_robust() {
        assert!(is_token_expired("nope", 0));
    }

    #[test]
    fn token_expires_at_ms_returns_milliseconds_normal() {
        let token = make_token(1_700_000_000);
        assert_eq!(token_expires_at_ms(&token), Some(1_700_000_000_000));
    }
}
