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
    /// Ed25519 signature over the canonical length-prefixed signing message.
    pub signature: Signature,
    /// The public key presented by the remote agent.
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
/// 2. The presented public key matches the trusted key for this agent.
/// 3. The Ed25519 signature is valid over the canonical message.
pub fn verify_attestation(
    token: &AgentSessionToken,
    trusted_public_key: &PublicKey,
    now_unix: u64,
) -> AttestationResult {
    if now_unix > token.expires_at {
        return AttestationResult::Expired;
    }

    if &token.public_key != trusted_public_key {
        return AttestationResult::InvalidSignature;
    }

    let message = build_signing_message(
        &token.agent_id,
        token.issued_at,
        token.expires_at,
        &token.payload,
    );
    if verify_ed25519_signature(trusted_public_key, &message, &token.signature) {
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
    let agent_id_bytes = agent_id.as_bytes();
    extend_len_prefixed(&mut message, agent_id_bytes);
    message.extend_from_slice(&issued_at.to_le_bytes());
    message.extend_from_slice(&expires_at.to_le_bytes());
    extend_len_prefixed(&mut message, payload);
    message
}

fn extend_len_prefixed(message: &mut Vec<u8>, bytes: &[u8]) {
    let len = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
    message.extend_from_slice(&len.to_le_bytes());
    message.extend_from_slice(bytes);
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
        assert_eq!(
            verify_attestation(&token, &token.public_key, 3000),
            AttestationResult::Expired
        );
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
            verify_attestation(&token, &token.public_key, 1500),
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
        assert_eq!(
            verify_attestation(&token, &PublicKey(verifying_key.to_bytes()), 1500),
            AttestationResult::Valid
        );
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
            verify_attestation(&token, &token.public_key, 1500),
            AttestationResult::InvalidSignature
        );
    }

    #[test]
    fn build_signing_message_is_deterministic() {
        let msg1 = build_signing_message("agent", 100, 200, b"data");
        let msg2 = build_signing_message("agent", 100, 200, b"data");
        assert_eq!(msg1, msg2);
    }

    #[test]
    fn self_signed_token_with_untrusted_presented_key_is_rejected_regression() {
        let attacker_key = SigningKey::from_bytes(&[11u8; 32]);
        let trusted_key = SigningKey::from_bytes(&[12u8; 32]).verifying_key();
        let message = build_signing_message("test-agent", 1000, 5000, b"hello");
        let signature = attacker_key.sign(&message);
        let token = AgentSessionToken {
            agent_id: "test-agent".to_string(),
            issued_at: 1000,
            expires_at: 5000,
            payload: b"hello".to_vec(),
            signature: Signature(signature.to_bytes()),
            public_key: PublicKey(attacker_key.verifying_key().to_bytes()),
        };

        assert_eq!(
            verify_attestation(&token, &PublicKey(trusted_key.to_bytes()), 1500),
            AttestationResult::InvalidSignature
        );
    }

    #[test]
    fn signing_message_separates_agent_id_from_timestamps_regression() {
        let first = build_signing_message("a", 0x6201, 0, b"");
        let second = build_signing_message("ab", 0x62, 0, b"");

        assert_ne!(first, second);
    }
}
