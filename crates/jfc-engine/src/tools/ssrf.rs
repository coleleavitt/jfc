//! SSRF guard for model-controlled outbound HTTP (CS-JFC-002).
//!
//! `WebFetch` lets the model choose an arbitrary URL whose response body is
//! returned into agent context. Without a destination guard that is a
//! server-side request forgery primitive: the model (or injected content) can
//! read `http://localhost`, LAN hosts, or the cloud metadata endpoint
//! (`169.254.169.254`) and exfiltrate the response.
//!
//! This module parses + classifies the destination before the request and
//! re-validates every redirect hop. DNS rebinding (resolver returning a public
//! address at check time and a private one at connect time) is a residual risk
//! not fully closed here.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

/// Maximum redirect hops a single `WebFetch` will follow. Each hop is
/// re-validated, so this only bounds work, not safety.
pub const MAX_REDIRECTS: usize = 5;

/// True when `ip` must never be reached by a model-controlled fetch.
pub fn is_disallowed_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => is_disallowed_ipv4(v4),
        IpAddr::V6(v6) => is_disallowed_ipv6(v6),
    }
}

fn is_disallowed_ipv4(ip: Ipv4Addr) -> bool {
    let o = ip.octets();
    ip.is_loopback()            // 127.0.0.0/8
        || ip.is_private()      // 10/8, 172.16/12, 192.168/16
        || ip.is_link_local()   // 169.254.0.0/16 (incl. 169.254.169.254 metadata)
        || ip.is_unspecified()  // 0.0.0.0
        || ip.is_broadcast()    // 255.255.255.255
        || ip.is_documentation()
        || ip.is_multicast()    // 224.0.0.0/4
        || (o[0] == 100 && (o[1] & 0xc0) == 64)        // 100.64.0.0/10 CGNAT
        || (o[0] == 198 && (o[1] == 18 || o[1] == 19)) // 198.18.0.0/15 benchmarking
}

fn is_disallowed_ipv6(ip: Ipv6Addr) -> bool {
    if ip.is_loopback() || ip.is_unspecified() || ip.is_multicast() {
        return true;
    }
    // IPv4-mapped / IPv4-compatible addresses: classify the embedded v4 so a
    // mapped private address can't slip through.
    if let Some(v4) = ip.to_ipv4() {
        return is_disallowed_ipv4(v4);
    }
    let seg = ip.segments();
    (seg[0] & 0xfe00) == 0xfc00 // fc00::/7 unique-local (incl. metadata fd00:ec2::254)
        || (seg[0] & 0xffc0) == 0xfe80 // fe80::/10 link-local
}

/// Parse, scheme-check, resolve, and classify a model-supplied URL. Returns the
/// parsed [`reqwest::Url`] when the destination is a public host, or a
/// human-readable reason when it must be blocked.
pub async fn validate_public_url(raw: &str) -> Result<reqwest::Url, String> {
    let url = reqwest::Url::parse(raw).map_err(|e| format!("invalid URL: {e}"))?;

    let scheme = url.scheme();
    if scheme != "http" && scheme != "https" {
        return Err(format!(
            "scheme `{scheme}` is not allowed (only http/https)"
        ));
    }

    let host = url
        .host_str()
        .ok_or_else(|| "URL has no host".to_string())?
        .to_owned();

    // IP literal: classify directly, no DNS.
    if let Ok(ip) = host.parse::<IpAddr>() {
        if is_disallowed_ip(ip) {
            return Err(format!(
                "destination {ip} is a private/loopback/link-local/metadata address"
            ));
        }
        return Ok(url);
    }

    // Reject localhost aliases before resolution (resolvers may map them
    // inconsistently or not at all).
    let host_lc = host.to_ascii_lowercase();
    if host_lc == "localhost" || host_lc.ends_with(".localhost") {
        return Err("destination host resolves to localhost".to_string());
    }

    let port = url.port_or_known_default().unwrap_or(0);
    let mut resolved = false;
    let addrs = tokio::net::lookup_host((host.as_str(), port))
        .await
        .map_err(|e| format!("DNS resolution failed for {host}: {e}"))?;
    for addr in addrs {
        resolved = true;
        if is_disallowed_ip(addr.ip()) {
            return Err(format!(
                "{host} resolves to disallowed address {}",
                addr.ip()
            ));
        }
    }
    if !resolved {
        return Err(format!("{host} did not resolve to any address"));
    }
    Ok(url)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn metadata_and_private_ipv4_are_disallowed_normal() {
        for ip in [
            "169.254.169.254", // cloud metadata
            "127.0.0.1",
            "10.1.2.3",
            "192.168.0.1",
            "172.16.5.5",
            "0.0.0.0",
            "100.64.1.1", // CGNAT
            "198.18.0.1", // benchmarking
        ] {
            let ip: IpAddr = ip.parse().unwrap();
            assert!(is_disallowed_ip(ip), "{ip} should be disallowed");
        }
    }

    #[test]
    fn public_ipv4_is_allowed_normal() {
        for ip in ["1.1.1.1", "8.8.8.8", "93.184.216.34"] {
            let ip: IpAddr = ip.parse().unwrap();
            assert!(!is_disallowed_ip(ip), "{ip} should be allowed");
        }
    }

    #[test]
    fn ipv6_loopback_unique_local_and_mapped_private_disallowed_robust() {
        for ip in [
            "::1",                    // loopback
            "fc00::1",                // unique local
            "fe80::1",                // link local
            "::ffff:127.0.0.1",       // ipv4-mapped loopback
            "::ffff:169.254.169.254", // ipv4-mapped metadata
        ] {
            let ip: IpAddr = ip.parse().unwrap();
            assert!(is_disallowed_ip(ip), "{ip} should be disallowed");
        }
        let public: IpAddr = "2606:4700:4700::1111".parse().unwrap();
        assert!(!is_disallowed_ip(public));
    }

    #[tokio::test]
    async fn validate_rejects_metadata_localhost_and_bad_scheme() {
        assert!(
            validate_public_url("http://169.254.169.254/latest/meta-data/")
                .await
                .is_err()
        );
        assert!(validate_public_url("http://127.0.0.1:8080/").await.is_err());
        assert!(validate_public_url("http://localhost/admin").await.is_err());
        assert!(validate_public_url("file:///etc/passwd").await.is_err());
        assert!(validate_public_url("ftp://example.com/x").await.is_err());
        assert!(validate_public_url("http://[::1]/").await.is_err());
        assert!(validate_public_url("http://10.0.0.5/").await.is_err());
    }
}
