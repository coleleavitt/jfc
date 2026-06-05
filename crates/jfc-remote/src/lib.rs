//! Remote-control protocol crate for jfc.
//!
//! Provides the wire protocol (`RemoteEnvelope` / `RemoteFrame`), pairing
//! token + HMAC frame attestation, a pluggable transport abstraction, and
//! a `tokio-tungstenite` WebSocket transport (server + client).
//!
//! The protocol is transport-agnostic: the same framing works over
//! localhost WebSocket, SSH tunnel, or a relay. The WS server binds to
//! `127.0.0.1` by default and relies on an external encrypted tunnel
//! (Tailscale, `ssh -L`, or `cloudflared`) for remote access.

pub mod auth;
pub mod protocol;
pub mod transport;
pub mod ws;
