use crate::backend::{BackendId, SearchResult};
use serde::Deserialize;

#[derive(Deserialize)]
pub struct FourGetResponse {
    pub status: String,
    pub npt: Option<String>,
    #[serde(default)]
    web: Vec<FourGetWebResult>,
}

impl FourGetResponse {
    pub fn into_results(self, max_results: usize, rank_offset: usize) -> Vec<SearchResult> {
        self.web
            .into_iter()
            .take(max_results)
            .enumerate()
            .filter_map(|(rank, item)| item.into_search_result(rank_offset + rank + 1))
            .collect()
    }
}

#[derive(Deserialize)]
struct FourGetWebResult {
    title: Option<String>,
    description: Option<String>,
    url: Option<String>,
}

impl FourGetWebResult {
    fn into_search_result(self, rank: usize) -> Option<SearchResult> {
        let title = self.title?.trim().to_owned();
        let url = self.url?.trim().to_owned();
        if title.is_empty() || url.is_empty() {
            return None;
        }
        let description = self.description.unwrap_or_default();
        Some(SearchResult {
            title: crate::decode_basic_entities(&crate::strip_html_tags(&title)),
            url,
            snippet: crate::decode_basic_entities(&crate::strip_html_tags(&description)),
            doi: None,
            arxiv_id: None,
            source: BackendId::FourGet,
            rank,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_response_maps_web_results_normal() {
        let response: FourGetResponse = serde_json::from_str(
            r#"{
                "status":"ok",
                "npt":"yandex1.key",
                "web":[{"title":"async_trait - Rust - Docs.rs","description":"Type erasure for <b>async</b> methods","url":"https://docs.rs/async-trait/latest/async_trait/"}]
            }"#,
        )
        .expect("valid 4get response");

        let results = response.into_results(5, 10);

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].source, BackendId::FourGet);
        assert_eq!(results[0].title, "async_trait - Rust - Docs.rs");
        assert_eq!(results[0].snippet, "Type erasure for async methods");
        assert_eq!(results[0].rank, 11);
    }
}
