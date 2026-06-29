//! `jfc-anthropic-sdk` — a Rust client for Anthropic's Beta APIs, mirroring
//! the public surface of the official `anthropic-sdk-go` and `ant` CLI.
//!
//! The crate is laid out one module per service so callers can import only
//! what they need:
//!
//! - [`messages`] — `POST /v1/messages` (streaming chat completion)
//! - [`agents`] — `BetaAgentService` (managed-agents create / list / update / archive)
//! - [`sessions`] — `BetaSessionService` + resources + events
//! - [`skills`] — `BetaSkillService` + version management + multipart upload
//! - [`environments`] — `BetaEnvironmentService` (isolated execution contexts)
//! - [`vaults`] — `BetaVaultService` + `BetaVaultCredentialService`
//! - [`memory_stores`] — `BetaMemoryStoreService` + memories + versions
//! - [`webhooks`] — Standard Webhooks verification + Managed Agents event unwrap
//! - [`files`] — `BetaFileService` (upload / download / metadata)
//! - [`batches`] — `BetaMessageBatchService` (async batch processing)
//! - [`models`] — `BetaModelService` (capability discovery)
//! - [`user_profiles`] — `BetaUserProfileService` (multi-user enrollment)
//!
//! All Beta endpoints require an `anthropic-beta` header pinning the feature
//! version (e.g. `managed-agents-2026-04-01`). Versions are exposed as
//! constants on each service so callers can pick or override.
//!
//! ## Auth
//!
//! Two schemes are supported:
//! - `Client::with_api_key(key)` — sets the `x-api-key` header (default).
//! - `Client::with_bearer(token)` — sets `Authorization: Bearer <token>`
//!   for OAuth-managed deployments.
//!
//! ## Retry
//!
//! Mirrors the SDK exactly: status codes 408/409/429 + 5xx retry with
//! exponential backoff `min(0.5s * 2^attempt, 8s)` plus quarter-jitter,
//! capped at 5 attempts. `Retry-After` / `Retry-After-Ms` headers override.
//! See [`retry`] for the policy.

pub mod agents;
pub mod batches;
pub mod client;
pub mod environments;
pub mod error;
pub mod files;
pub mod managed_agents;
pub mod memory_stores;
pub mod messages;
pub mod models;
pub mod pagination;
pub mod retry;
pub mod sessions;
pub mod skills;
pub mod sse;
pub mod user_profiles;
pub mod vaults;
pub mod webhooks;

pub use client::Client;
pub use error::{Error, Result};

/// Beta header values pinned per feature surface. Mirrored from the Go SDK
/// constants — bump these when Anthropic publishes a new revision.
pub mod beta {
    pub const MANAGED_AGENTS: &str = "managed-agents-2026-04-01";
    pub const FILES: &str = "files-api-2025-04-14";
    pub const MESSAGE_BATCHES: &str = "message-batches-2024-09-24";
    pub const SKILLS: &str = "skills-2025-10-02";
    pub const USER_PROFILES: &str = "user-profiles-2026-03-24";
    pub const TOKEN_COUNTING: &str = "token-counting-2024-11-01";
    pub const STRUCTURED_OUTPUTS: &str = "structured-outputs-2025-12-15";
    pub const CONTEXT_MANAGEMENT: &str = "context-management-2025-06-27";
    pub const NARRATION_SUMMARIES: &str = "summarize-connector-text-2026-03-13";
}
