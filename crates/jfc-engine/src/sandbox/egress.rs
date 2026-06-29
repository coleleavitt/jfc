//! Network-egress allowlist for sandboxed agentic runs.
//!
//! Mirrors Perplexity Computer's sandbox network controls found in the
//! 2026-06-11 mindemon dump: "Allow outbound internet from sandboxes. Disable
//! to block all outbound traffic" and "Domains that sandboxes can access.
//! Supports wildcards (e.g. `*.pypi.org`)".
//!
//! [`NetworkSandboxConfig`](super::NetworkSandboxConfig) already carries the
//! allow/deny domain lists; this module adds the missing *decision* logic:
//! default-deny resolution, wildcard matching, and a denied-takes-precedence
//! rule. It is a pure, host-level policy function so it can back either a bwrap
//! egress proxy or an in-process HTTP guard, and it is fully unit-testable
//! without spawning anything.

use super::NetworkSandboxConfig;

/// The effective egress decision for a destination host.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EgressDecision {
    Allow,
    Deny,
}

impl EgressDecision {
    pub fn is_allowed(self) -> bool {
        matches!(self, EgressDecision::Allow)
    }
}

/// Whether outbound networking is permitted at all, and to which hosts.
///
/// Resolution order for a host:
/// 1. If outbound is fully disabled → `Deny`.
/// 2. If any denied pattern matches → `Deny` (deny takes precedence).
/// 3. If any allowed pattern matches → `Allow`.
/// 4. Otherwise → `Deny` (default-deny).
#[derive(Debug, Clone)]
pub struct EgressPolicy {
    /// When false, all outbound traffic is blocked regardless of the lists
    /// ("Disable to block all outbound traffic").
    pub outbound_enabled: bool,
    pub allowed: Vec<String>,
    pub denied: Vec<String>,
}

impl EgressPolicy {
    /// Block-everything policy (the safe default for an untrusted agentic run).
    pub fn block_all() -> Self {
        let _linkscope_policy = linkscope::phase("engine.sandbox.egress.block_all");
        Self {
            outbound_enabled: false,
            allowed: Vec::new(),
            denied: Vec::new(),
        }
    }

    /// Build a policy from a [`NetworkSandboxConfig`]. Outbound is considered
    /// enabled when there is at least one allowed domain (mirrors the existing
    /// `--unshare-net` heuristic: an empty allowlist means no network).
    pub fn from_network_config(cfg: &NetworkSandboxConfig) -> Self {
        let _linkscope_policy = linkscope::phase("engine.sandbox.egress.from_network_config");
        linkscope::event_fields(
            "engine.sandbox.egress.from_network_config",
            [
                linkscope::TraceField::count(
                    "allowed",
                    u64::try_from(cfg.allowed_domains.len()).unwrap_or(u64::MAX),
                ),
                linkscope::TraceField::count(
                    "denied",
                    u64::try_from(cfg.denied_domains.len()).unwrap_or(u64::MAX),
                ),
                linkscope::TraceField::count("has_proxy", u64::from(cfg.has_egress_proxy())),
            ],
        );
        Self {
            outbound_enabled: !cfg.allowed_domains.is_empty(),
            allowed: cfg.allowed_domains.clone(),
            denied: cfg.denied_domains.clone(),
        }
    }

    /// Decide whether egress to `host` is permitted. `host` is a bare hostname
    /// (e.g. `files.pypi.org`); a leading scheme/port is not expected.
    pub fn decide(&self, host: &str) -> EgressDecision {
        let _linkscope_decide = linkscope::phase("engine.sandbox.egress.decide");
        if !self.outbound_enabled {
            linkscope::detail_event_fields(
                "engine.sandbox.egress.decision",
                [
                    linkscope::TraceField::text("host", host.to_owned()),
                    linkscope::TraceField::text("decision", "deny_disabled"),
                ],
            );
            return EgressDecision::Deny;
        }
        let host = normalize_host(host);
        if host.is_empty() {
            linkscope::detail_event_fields(
                "engine.sandbox.egress.decision",
                [linkscope::TraceField::text("decision", "deny_empty_host")],
            );
            return EgressDecision::Deny;
        }
        if self.denied.iter().any(|p| matches_domain(p, host)) {
            linkscope::detail_event_fields(
                "engine.sandbox.egress.decision",
                [
                    linkscope::TraceField::text("host", host.to_owned()),
                    linkscope::TraceField::text("decision", "deny_pattern"),
                ],
            );
            return EgressDecision::Deny;
        }
        if self.allowed.iter().any(|p| matches_domain(p, host)) {
            linkscope::detail_event_fields(
                "engine.sandbox.egress.decision",
                [
                    linkscope::TraceField::text("host", host.to_owned()),
                    linkscope::TraceField::text("decision", "allow"),
                ],
            );
            return EgressDecision::Allow;
        }
        linkscope::detail_event_fields(
            "engine.sandbox.egress.decision",
            [
                linkscope::TraceField::text("host", host.to_owned()),
                linkscope::TraceField::text("decision", "deny_default"),
            ],
        );
        EgressDecision::Deny
    }

    pub fn allows(&self, host: &str) -> bool {
        self.decide(host).is_allowed()
    }

    /// Decide for a URL by extracting its host component. A URL we can't parse a
    /// host from is denied.
    pub fn decide_url(&self, url: &str) -> EgressDecision {
        let _linkscope_url = linkscope::phase("engine.sandbox.egress.decide_url");
        linkscope::detail_event_fields(
            "engine.sandbox.egress.decide_url",
            [linkscope::TraceField::bytes(
                "url_bytes",
                u64::try_from(url.len()).unwrap_or(u64::MAX),
            )],
        );
        match host_from_url(url) {
            Some(host) => self.decide(&host),
            None => EgressDecision::Deny,
        }
    }
}

/// Lowercase a host and strip a trailing dot / surrounding whitespace.
fn normalize_host(host: &str) -> &str {
    host.trim().trim_end_matches('.')
}

/// Match a domain `pattern` against a `host`.
///
/// - `*.example.com` matches any subdomain (`a.example.com`, `a.b.example.com`)
///   but NOT the apex `example.com`.
/// - `example.com` matches the apex AND any subdomain (`a.example.com`) — the
///   common "this domain and below" intent.
/// - matching is case-insensitive.
fn matches_domain(pattern: &str, host: &str) -> bool {
    let pattern = pattern.trim().trim_end_matches('.');
    if pattern.is_empty() {
        return false;
    }
    let host = host.to_ascii_lowercase();

    if let Some(suffix) = pattern.strip_prefix("*.") {
        let suffix = suffix.to_ascii_lowercase();
        // Wildcard matches strict subdomains only.
        return host.ends_with(&format!(".{suffix}"));
    }

    let pattern = pattern.to_ascii_lowercase();
    // Bare domain matches the apex or any subdomain.
    host == pattern || host.ends_with(&format!(".{pattern}"))
}

/// Extract the host from a URL without pulling in a URL-parsing dependency.
/// Handles `scheme://host[:port]/...`, `host:port`, and bare `host`.
fn host_from_url(url: &str) -> Option<String> {
    let after_scheme = match url.split_once("://") {
        Some((_, rest)) => rest,
        None => url,
    };
    // Strip userinfo (user:pass@host), then path/query, then port.
    let authority = after_scheme
        .split(['/', '?', '#'])
        .next()
        .unwrap_or(after_scheme);
    let authority = authority.rsplit('@').next().unwrap_or(authority);
    let host = authority.split(':').next().unwrap_or(authority);
    let host = normalize_host(host);
    if host.is_empty() {
        None
    } else {
        Some(host.to_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn policy(allowed: &[&str], denied: &[&str]) -> EgressPolicy {
        EgressPolicy {
            outbound_enabled: true,
            allowed: allowed.iter().map(|s| s.to_string()).collect(),
            denied: denied.iter().map(|s| s.to_string()).collect(),
        }
    }

    // ── Default-deny ──────────────────────────────────────────────────────────

    #[test]
    fn block_all_denies_everything_normal() {
        let p = EgressPolicy::block_all();
        assert!(!p.allows("pypi.org"));
        assert_eq!(p.decide("anything.com"), EgressDecision::Deny);
    }

    #[test]
    fn empty_allowlist_denies_by_default_normal() {
        let p = policy(&[], &[]);
        assert!(!p.allows("example.com"));
    }

    #[test]
    fn from_config_with_no_domains_blocks_outbound_normal() {
        let cfg = NetworkSandboxConfig::default();
        let p = EgressPolicy::from_network_config(&cfg);
        assert!(!p.outbound_enabled);
        assert!(!p.allows("pypi.org"));
    }

    // ── Wildcard matching ───────────────────────────────────────────────────────

    #[test]
    fn wildcard_matches_subdomains_normal() {
        let p = policy(&["*.pypi.org"], &[]);
        assert!(p.allows("files.pypi.org"));
        assert!(p.allows("a.b.pypi.org"));
    }

    #[test]
    fn wildcard_does_not_match_apex_robust() {
        let p = policy(&["*.pypi.org"], &[]);
        assert!(!p.allows("pypi.org"));
    }

    #[test]
    fn bare_domain_matches_apex_and_subdomains_normal() {
        let p = policy(&["pypi.org"], &[]);
        assert!(p.allows("pypi.org"));
        assert!(p.allows("files.pypi.org"));
    }

    #[test]
    fn unrelated_host_is_denied_robust() {
        let p = policy(&["pypi.org"], &[]);
        assert!(!p.allows("evil.com"));
        // Suffix-confusion guard: notpypi.org must not match pypi.org.
        assert!(!p.allows("notpypi.org"));
    }

    // ── Deny precedence ───────────────────────────────────────────────────────

    #[test]
    fn denied_takes_precedence_over_allowed_normal() {
        let p = policy(&["*.example.com"], &["secret.example.com"]);
        assert!(p.allows("public.example.com"));
        assert!(!p.allows("secret.example.com"));
    }

    #[test]
    fn deny_wildcard_blocks_a_whole_subtree_robust() {
        let p = policy(&["example.com"], &["*.internal.example.com"]);
        assert!(p.allows("api.example.com"));
        assert!(!p.allows("db.internal.example.com"));
    }

    // ── Host / URL normalisation ──────────────────────────────────────────────

    #[test]
    fn case_and_trailing_dot_are_normalised_robust() {
        let p = policy(&["pypi.org"], &[]);
        assert!(p.allows("FILES.PyPI.ORG"));
        assert!(p.allows("files.pypi.org."));
    }

    #[test]
    fn decide_url_extracts_host_normal() {
        let p = policy(&["*.pypi.org"], &[]);
        assert!(
            p.decide_url("https://files.pypi.org/packages/x.whl")
                .is_allowed()
        );
        assert!(
            p.decide_url("https://user:pass@files.pypi.org:443/a")
                .is_allowed()
        );
        assert!(!p.decide_url("https://evil.com/p").is_allowed());
    }

    #[test]
    fn decide_url_denies_unparseable_robust() {
        let p = policy(&["pypi.org"], &[]);
        assert!(!p.decide_url("not a url").is_allowed());
        assert!(!p.decide_url("https://").is_allowed());
    }

    #[test]
    fn from_config_roundtrip_enforces_lists_normal() {
        let cfg = NetworkSandboxConfig {
            allowed_domains: vec!["*.pypi.org".into(), "crates.io".into()],
            denied_domains: vec!["evil.pypi.org".into()],
            allow_managed_domains_only: false,
            ..Default::default()
        };
        let p = EgressPolicy::from_network_config(&cfg);
        assert!(p.outbound_enabled);
        assert!(p.allows("files.pypi.org"));
        assert!(p.allows("crates.io"));
        assert!(!p.allows("evil.pypi.org"));
        assert!(!p.allows("example.com"));
    }
}
