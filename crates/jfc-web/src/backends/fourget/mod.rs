mod api;
mod config;
mod response;

use std::collections::HashSet;
use std::time::Duration;

use crate::backend::{BackendId, BackendResult, SearchBackend, SearchResult};
use async_trait::async_trait;

use self::api::{discover_instances, fetch_page};
use self::config::{FourGetRequest, configured_instances, configured_scrapers, instance_limit};

pub struct FourGetBackend;

#[async_trait]
impl SearchBackend for FourGetBackend {
    fn id(&self) -> BackendId {
        BackendId::FourGet
    }

    fn is_available(&self) -> bool {
        true
    }

    async fn search(&self, query: &str, max_results: usize) -> BackendResult {
        search_fourget_structured(query, max_results).await
    }

    fn timeout(&self) -> Duration {
        Duration::from_secs(45)
    }
}

pub async fn search_fourget_structured(query: &str, max_results: usize) -> BackendResult {
    let target = max_results.clamp(1, 100);
    let request = FourGetRequest::parse(query);
    let client = crate::http_client()?;
    let instances = discover_instances(&client, configured_instances(), instance_limit()).await;
    let scrapers = request
        .scraper
        .map(|scraper| vec![scraper.to_owned()])
        .unwrap_or_else(configured_scrapers);
    let mut states = SearchStates::new(instances, scrapers);

    states.collect(&client, request.query, target).await
}

struct SearchState {
    instance: String,
    scraper: String,
    next_page: Option<String>,
    exhausted: bool,
}

struct SearchStates {
    states: Vec<SearchState>,
    errors: Vec<String>,
}

impl SearchStates {
    fn new(instances: Vec<String>, scrapers: Vec<String>) -> Self {
        let states = instances
            .into_iter()
            .flat_map(|instance| {
                scrapers.iter().map(move |scraper| SearchState {
                    instance: instance.clone(),
                    scraper: scraper.clone(),
                    next_page: None,
                    exhausted: false,
                })
            })
            .collect();

        Self {
            states,
            errors: Vec::new(),
        }
    }

    async fn collect(
        &mut self,
        client: &reqwest::Client,
        query: &str,
        target: usize,
    ) -> BackendResult {
        let mut results = Vec::new();
        let mut seen = HashSet::new();

        while results.len() < target && self.states.iter().any(|state| !state.exhausted) {
            let mut progressed = false;
            let active: Vec<_> = self
                .states
                .iter()
                .enumerate()
                .filter(|(_, state)| !state.exhausted)
                .map(|(index, state)| {
                    (
                        index,
                        state.instance.clone(),
                        state.scraper.clone(),
                        state.next_page.clone(),
                    )
                })
                .collect();
            let pages = futures::future::join_all(active.iter().map(
                |(_, instance, scraper, next_page)| {
                    fetch_page(client, instance, query, scraper, next_page.as_deref())
                },
            ))
            .await;

            for ((index, instance, scraper, _), page) in active.into_iter().zip(pages) {
                let state = &mut self.states[index];
                let before = results.len();
                match page {
                    Ok(page) if page.status == "ok" => {
                        state.next_page = page.npt.clone();
                        let page_results = page.into_results(target - results.len(), results.len());
                        let page_had_results = !page_results.is_empty();
                        push_unique(page_results, &mut seen, &mut results);
                        progressed |= results.len() > before;
                        if state.next_page.is_none() || !page_had_results {
                            state.exhausted = true;
                        }
                    }
                    Ok(page) => {
                        self.errors
                            .push(format!("{} scraper {}: {}", instance, scraper, page.status));
                        state.exhausted = true;
                    }
                    Err(error) => {
                        self.errors
                            .push(format!("{} scraper {}: {error}", instance, scraper));
                        state.exhausted = true;
                    }
                }
                if results.len() >= target {
                    break;
                }
            }
            if !progressed && self.states.iter().all(|state| state.exhausted) {
                break;
            }
        }

        if results.is_empty() {
            Err(format!(
                "4get failed: {}",
                self.errors
                    .iter()
                    .take(8)
                    .cloned()
                    .collect::<Vec<_>>()
                    .join("; ")
            ))
        } else {
            Ok(results)
        }
    }
}

fn push_unique(
    page: Vec<SearchResult>,
    seen: &mut HashSet<String>,
    results: &mut Vec<SearchResult>,
) {
    for result in page {
        let keys = result.dedup_keys();
        if keys.iter().any(|key| seen.contains(key)) {
            continue;
        }
        for key in keys {
            seen.insert(key);
        }
        results.push(result);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn push_unique_skips_duplicate_url_robust() {
        let result = |title: &str| SearchResult {
            title: title.into(),
            url: "https://example.com/result".into(),
            snippet: String::new(),
            doi: None,
            arxiv_id: None,
            source: BackendId::FourGet,
            rank: 1,
        };
        let mut seen = HashSet::new();
        let mut results = Vec::new();

        push_unique(
            vec![result("first"), result("second")],
            &mut seen,
            &mut results,
        );

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].title, "first");
    }
}
