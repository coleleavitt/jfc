//! Bridge attestation — Ed25519 signature verification for remote agent identity.
//!
//! When jfc communicates with remote agents (via the SDK bridge or
//! teammate protocol), each agent presents a session token signed with
//! its Ed25519 private key. This module verifies those signatures to
//! confirm the remote agent's identity hasn't been spoofed.

use ed25519_dalek::{Signature as DalekSignature, Verifier, VerifyingKey};

/// An Ed25519 public key (32 bytes).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublicKey(pub [u8; 32]);

/// An Ed25519 signature (64 bytes).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Signature(pub [u8; 64]);

/// A session token presented by a remote agent for attestation.
#[derive(Debug, Clone)]
pub struct AgentSessionToken {
    /// The agent's claimed identity (e.g. session ID or agent name).
    pub agent_id: String,
    /// Unix timestamp when the token was issued.
    pub issued_at: u64,
    /// Unix timestamp when the token expires.
    pub expires_at: u64,
    /// Arbitrary payload (e.g. serialized session metadata).
    pub payload: Vec<u8>,
    /// Ed25519 signature over `agent_id || issued_at || expires_at || payload`.
    pub signature: Signature,
    /// The public key that should verify this token.
    pub public_key: PublicKey,
}

/// Result of attestation verification.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AttestationResult {
    /// Signature is valid and token is not expired.
    Valid,
    /// Signature verification failed.
    InvalidSignature,
    /// Token has expired.
    Expired,
    /// Token is malformed (missing fields, bad lengths).
    Malformed(String),
}

/// Verify an agent session token.
///
/// Checks:
/// 1. Token is not expired (against `now_unix`).
/// 2. The Ed25519 signature is valid over the canonical message.
pub fn verify_attestation(token: &AgentSessionToken, now_unix: u64) -> AttestationResult {
    // Check expiry
    if now_unix > token.expires_at {
        return AttestationResult::Expired;
    }

    // Build the canonical message to verify:
    // agent_id bytes || issued_at (8 bytes LE) || expires_at (8 bytes LE) || payload
    let mut message = Vec::new();
    message.extend_from_slice(token.agent_id.as_bytes());
    message.extend_from_slice(&token.issued_at.to_le_bytes());
    message.extend_from_slice(&token.expires_at.to_le_bytes());
    message.extend_from_slice(&token.payload);

    if verify_ed25519_signature(&token.public_key, &message, &token.signature) {
        AttestationResult::Valid
    } else {
        AttestationResult::InvalidSignature
    }
}

fn verify_ed25519_signature(public_key: &PublicKey, message: &[u8], signature: &Signature) -> bool {
    let Ok(key) = VerifyingKey::from_bytes(&public_key.0) else {
        return false;
    };
    let sig = DalekSignature::from_bytes(&signature.0);
    key.verify(message, &sig).is_ok()
}

/// Create a signing message from token fields (for the signing side).
pub fn build_signing_message(
    agent_id: &str,
    issued_at: u64,
    expires_at: u64,
    payload: &[u8],
) -> Vec<u8> {
    let mut message = Vec::new();
    message.extend_from_slice(agent_id.as_bytes());
    message.extend_from_slice(&issued_at.to_le_bytes());
    message.extend_from_slice(&expires_at.to_le_bytes());
    message.extend_from_slice(payload);
    message
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::{Signer, SigningKey};

    #[test]
    fn expired_token_is_rejected() {
        let token = AgentSessionToken {
            agent_id: "test-agent".to_string(),
            issued_at: 1000,
            expires_at: 2000,
            payload: b"hello".to_vec(),
            signature: Signature([1u8; 64]),
            public_key: PublicKey([2u8; 32]),
        };
        assert_eq!(verify_attestation(&token, 3000), AttestationResult::Expired);
    }

    #[test]
    fn zero_signature_is_invalid() {
        let token = AgentSessionToken {
            agent_id: "test-agent".to_string(),
            issued_at: 1000,
            expires_at: 5000,
            payload: b"hello".to_vec(),
            signature: Signature([0u8; 64]),
            public_key: PublicKey([2u8; 32]),
        };
        assert_eq!(
            verify_attestation(&token, 1500),
            AttestationResult::InvalidSignature
        );
    }

    #[test]
    fn signed_token_passes_normal() {
        let signing_key = SigningKey::from_bytes(&[7u8; 32]);
        let verifying_key = signing_key.verifying_key();
        let message = build_signing_message("test-agent", 1000, 5000, b"hello");
        let signature = signing_key.sign(&message);
        let token = AgentSessionToken {
            agent_id: "test-agent".to_string(),
            issued_at: 1000,
            expires_at: 5000,
            payload: b"hello".to_vec(),
            signature: Signature(signature.to_bytes()),
            public_key: PublicKey(verifying_key.to_bytes()),
        };
        assert_eq!(verify_attestation(&token, 1500), AttestationResult::Valid);
    }

    #[test]
    fn tampered_signed_token_is_rejected_robust() {
        let signing_key = SigningKey::from_bytes(&[9u8; 32]);
        let verifying_key = signing_key.verifying_key();
        let message = build_signing_message("test-agent", 1000, 5000, b"hello");
        let signature = signing_key.sign(&message);
        let token = AgentSessionToken {
            agent_id: "test-agent".to_string(),
            issued_at: 1000,
            expires_at: 5000,
            payload: b"tampered".to_vec(),
            signature: Signature(signature.to_bytes()),
            public_key: PublicKey(verifying_key.to_bytes()),
        };
        assert_eq!(
            verify_attestation(&token, 1500),
            AttestationResult::InvalidSignature
        );
    }

    #[test]
    fn build_signing_message_is_deterministic() {
        let msg1 = build_signing_message("agent", 100, 200, b"data");
        let msg2 = build_signing_message("agent", 100, 200, b"data");
        assert_eq!(msg1, msg2);
    }
}
