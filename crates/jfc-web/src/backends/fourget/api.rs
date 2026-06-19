use std::collections::HashSet;

use reqwest::header::{ACCEPT, USER_AGENT};
use serde::Deserialize;

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
        let instance = candidate.trim_end_matches('/').to_owned();
        if instance.starts_with("https://") && seen.insert(instance.clone()) {
            added.push(instance.clone());
            instances.push(instance);
        }
        if instances.len() >= limit {
            break;
        }
    }
    added
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
    fn next_page_params_keep_original_scraper_normal() {
        let params = page_params("rust", "yandex", Some("yandex_w1.key"));

        assert_eq!(
            params,
            vec![("npt", "yandex_w1.key"), ("scraper", "yandex")]
        );
    }
}
