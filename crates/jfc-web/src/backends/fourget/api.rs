use std::collections::HashSet;
use std::net::IpAddr;

use reqwest::header::{ACCEPT, USER_AGENT};
use serde::Deserialize;

use super::config::discovery_enabled;
use super::response::FourGetResponse;

const FOURGET_USER_AGENT: &str =
    "Mozilla/5.0 (compatible; jfc-web/1.0; +https://github.com/coleleavitt/jfc)";

pub async fn discover_instances(
    client: &reqwest::Client,
    seeds: Vec<String>,
    limit: usize,
) -> Vec<String> {
    let mut instances = Vec::new();
    let mut seen = HashSet::new();
    let mut fetched = HashSet::new();
    let mut frontier = push_instances(seeds, &mut seen, &mut instances, limit);
    if !discovery_enabled() {
        return instances;
    }

    while !frontier.is_empty() && instances.len() < limit {
        let current = frontier;
        frontier = Vec::new();
        let to_fetch: Vec<_> = current
            .into_iter()
            .filter(|instance| fetched.insert(instance.clone()))
            .collect();
        let responses = futures::future::join_all(
            to_fetch
                .iter()
                .map(|seed| fetch_ami4get(client, seed.as_str())),
        )
        .await;

        for response in responses.into_iter().flatten() {
            if response.status == "ok" && response.service == "4get" {
                let mut discovered = response.instances;
                if response.server.api_enabled {
                    discovered.insert(0, response.origin);
                }
                discovered.extend(response.server.alt_addresses);
                frontier.extend(push_instances(discovered, &mut seen, &mut instances, limit));
            }
        }
    }

    instances
}

pub async fn fetch_page(
    client: &reqwest::Client,
    instance: &str,
    query: &str,
    scraper: &str,
    next_page: Option<&str>,
) -> Result<FourGetResponse, String> {
    let url = format!("{}/api/v1/web", instance.trim_end_matches('/'));
    let request = client
        .get(url)
        .header(USER_AGENT, FOURGET_USER_AGENT)
        .header(ACCEPT, "application/json");
    let params = page_params(query, scraper, next_page);
    let request = request.query(&params);
    let response = request
        .send()
        .await
        .map_err(|e| format!("request failed: {e}"))?;
    let status = response.status();
    response
        .json()
        .await
        .map_err(|e| format!("JSON parse failed after HTTP {status}: {e}"))
}

fn page_params<'a>(
    query: &'a str,
    scraper: &'a str,
    next_page: Option<&'a str>,
) -> Vec<(&'a str, &'a str)> {
    next_page.map_or_else(
        || vec![("s", query), ("scraper", scraper)],
        |token| vec![("npt", token), ("scraper", scraper)],
    )
}

async fn fetch_ami4get(
    client: &reqwest::Client,
    instance: &str,
) -> Result<Ami4GetResponse, String> {
    let origin = instance.trim_end_matches('/').to_owned();
    let response = client
        .get(format!("{origin}/ami4get"))
        .header(USER_AGENT, FOURGET_USER_AGENT)
        .header(ACCEPT, "application/json")
        .send()
        .await
        .map_err(|e| format!("{origin}: /ami4get request failed: {e}"))?;
    let status = response.status();
    let mut parsed: Ami4GetResponse = response
        .json()
        .await
        .map_err(|e| format!("{origin}: /ami4get JSON parse failed after HTTP {status}: {e}"))?;
    parsed.origin = origin;
    Ok(parsed)
}

fn push_instances(
    candidates: Vec<String>,
    seen: &mut HashSet<String>,
    instances: &mut Vec<String>,
    limit: usize,
) -> Vec<String> {
    let mut added = Vec::new();
    for candidate in candidates {
        let Some(instance) = normalize_public_instance(&candidate) else {
            continue;
        };
        if seen.insert(instance.clone()) {
            added.push(instance.clone());
            instances.push(instance);
        }
        if instances.len() >= limit {
            break;
        }
    }
    added
}

fn normalize_public_instance(candidate: &str) -> Option<String> {
    let parsed = reqwest::Url::parse(candidate.trim()).ok()?;
    if parsed.scheme() != "https" || parsed.username() != "" || parsed.password().is_some() {
        return None;
    }
    if parsed.host_str().is_none()
        || parsed.query().is_some()
        || parsed.fragment().is_some()
        || !parsed.path().trim_matches('/').is_empty()
    {
        return None;
    }
    if let Some(host) = parsed.host_str()
        && is_blocked_host(host)
    {
        return None;
    }
    let host = parsed.host_str()?;
    let port = parsed
        .port()
        .map(|port| format!(":{port}"))
        .unwrap_or_default();
    Some(format!("https://{host}{port}"))
}

fn is_blocked_host(host: &str) -> bool {
    let lower = host.trim_matches('.').to_ascii_lowercase();
    if matches!(lower.as_str(), "localhost" | "localhost.localdomain")
        || lower.ends_with(".localhost")
        || lower.ends_with(".local")
        || lower.ends_with(".internal")
    {
        return true;
    }
    lower.parse::<IpAddr>().map(is_blocked_ip).unwrap_or(false)
}

fn is_blocked_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => {
            ip.is_private()
                || ip.is_loopback()
                || ip.is_link_local()
                || ip.is_broadcast()
                || ip.is_unspecified()
                || ip.is_documentation()
        }
        IpAddr::V6(ip) => {
            ip.is_loopback()
                || ip.is_unspecified()
                || ip.is_unique_local()
                || ip.is_unicast_link_local()
        }
    }
}

#[derive(Deserialize)]
struct Ami4GetResponse {
    status: String,
    service: String,
    server: Ami4GetServer,
    #[serde(default)]
    instances: Vec<String>,
    #[serde(skip)]
    origin: String,
}

#[derive(Deserialize)]
struct Ami4GetServer {
    api_enabled: bool,
    #[serde(default)]
    alt_addresses: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn push_instances_dedups_and_strips_trailing_slash_normal() {
        let mut seen = HashSet::new();
        let mut instances = Vec::new();

        let added = push_instances(
            vec![
                "https://4get.example/".into(),
                "https://4get.example".into(),
                "http://ignored.example".into(),
            ],
            &mut seen,
            &mut instances,
            10,
        );

        assert_eq!(instances, vec!["https://4get.example"]);
        assert_eq!(added, vec!["https://4get.example"]);
    }

    #[test]
    fn push_instances_rejects_non_public_roots_regression() {
        let mut seen = HashSet::new();
        let mut instances = Vec::new();

        let added = push_instances(
            vec![
                "http://4get.example".into(),
                "https://127.0.0.1".into(),
                "https://10.0.0.4".into(),
                "https://metadata.google.internal".into(),
                "https://4get.example/path".into(),
                "https://4get.example?q=x".into(),
                "https://user:pass@4get.example".into(),
                "https://4get.example:8443/".into(),
            ],
            &mut seen,
            &mut instances,
            10,
        );

        assert_eq!(instances, vec!["https://4get.example:8443"]);
        assert_eq!(added, vec!["https://4get.example:8443"]);
    }

    #[test]
    fn next_page_params_keep_original_scraper_normal() {
        let params = page_params("rust", "yandex", Some("yandex_w1.key"));

        assert_eq!(
            params,
            vec![("npt", "yandex_w1.key"), ("scraper", "yandex")]
        );
    }
}
