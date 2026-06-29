use super::trace::{self, BackendResultTrace, BackendStart, PageRequest};
use crate::backend::{BackendId, BackendResult, SearchBackend, SearchResult};
use async_trait::async_trait;
use reqwest::header::{ACCEPT, ORIGIN, REFERER, USER_AGENT};
use serde::Deserialize;

const HOST: &str = "cmu.primo.exlibrisgroup.com";
const VID: &str = "01CMU_INST:01CMU";
const INST: &str = "01CMU_INST";
const TAB: &str = "Everything";
const SCOPE: &str = "MyInst_and_CI";
const PRIMO_USER_AGENT: &str = "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/124.0 Safari/537.36";

pub struct PrimoBackend;

#[async_trait]
impl SearchBackend for PrimoBackend {
    fn id(&self) -> BackendId {
        BackendId::Primo
    }

    fn is_available(&self) -> bool {
        true
    }

    async fn search(&self, query: &str, max_results: usize) -> BackendResult {
        let _linkscope_search = linkscope::phase("web.backend.primo.search");
        trace::backend_start(BackendStart {
            backend: "primo",
            query,
            max_results,
        });
        search_primo_structured(query, max_results).await
    }
}

async fn search_primo_structured(query: &str, max_results: usize) -> BackendResult {
    let _linkscope_search = linkscope::phase("web.backend.primo.structured");
    let q = query.trim();
    let target = max_results.clamp(1, 100);
    let mut results = Vec::new();
    let mut offset = 0;

    while results.len() < target {
        let remaining = target - results.len();
        let page = search_primo_page(q, offset, remaining.min(50)).await?;
        if page.is_empty() {
            linkscope::record_items("web.backend.primo.empty_page", 1);
            break;
        }
        offset += page.len();
        results.extend(page);
    }

    trace::backend_result(BackendResultTrace {
        backend: "primo",
        status: "ok",
        results: results.len(),
    });
    Ok(results)
}

async fn search_primo_page(query: &str, offset: usize, limit: usize) -> BackendResult {
    let _linkscope_page = linkscope::phase("web.backend.primo.page");
    let page_limit = limit.clamp(1, 50);
    trace::page_request(PageRequest {
        backend: "primo",
        query,
        offset,
        limit: page_limit,
    });
    let limit = page_limit.to_string();
    let offset_start = offset;
    let offset = offset_start.to_string();
    let client = crate::http_client()?;
    let q_encoded = urlencoding_minimal(query);
    let referer_query = urlencoding_minimal(&format!("any,contains,{query}"));
    let url = format!(
        "https://{HOST}/primaws/rest/pub/pnxs\
         ?acTriggered=false\
         &blendFacetsSeparately=false\
         &citationTrailFilterByAvailability=true\
         &disableCache=false\
         &getMore=0\
         &inst={INST}\
         &isCDSearch=false\
         &lang=en\
         &limit={limit}\
         &mode=Basic\
         &newspapersActive=false\
         &newspapersSearch=false\
         &offset={offset}\
         &otbRanking=false\
         &pcAvailability=false\
         &q=any,contains,{q_encoded}\
         &qExclude=\
         &qInclude=\
         &rapido=false\
         &refEntryActive=false\
         &rtaLinks=true\
         &scope={SCOPE}\
         &searchInFulltextUserSelection=false\
         &skipDelivery=Y\
         &sort=rank\
         &tab={TAB}\
         &vid={VID}"
    );
    let response = client
        .get(url)
        .header(ACCEPT, "application/json, text/plain, */*")
        .header(ORIGIN, format!("https://{HOST}"))
        .header(REFERER, format!("https://{HOST}/discovery/search?institution={INST}&vid={VID}&tab={TAB}&search_scope={SCOPE}&query={referer_query}"))
        .header(USER_AGENT, PRIMO_USER_AGENT)
        .send()
        .await
        .map_err(|e| format!("Primo request failed: {e}"))?;
    if !response.status().is_success() {
        let status = response.status();
        linkscope::record_items("web.backend.primo.http_error", u64::from(status.as_u16()));
        return Err(format!("Primo returned HTTP {status}"));
    }
    linkscope::record_items("web.backend.primo.http_ok", 1);
    let parsed: PrimoResponse = response
        .json()
        .await
        .map_err(|e| format!("Primo JSON parse failed: {e}"))?;

    Ok(parsed.into_results(page_limit, offset_start))
}

fn urlencoding_minimal(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                (b as char).to_string()
            }
            b' ' => "%20".to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect()
}

#[derive(Deserialize)]
struct PrimoResponse {
    #[serde(default)]
    docs: Vec<PrimoDoc>,
}

impl PrimoResponse {
    fn into_results(self, max_results: usize, offset: usize) -> Vec<SearchResult> {
        self.docs
            .into_iter()
            .take(max_results)
            .enumerate()
            .filter_map(|(rank, doc)| doc.into_result(offset + rank + 1))
            .collect()
    }
}

#[derive(Deserialize)]
struct PrimoDoc {
    pnx: Option<PrimoPnx>,
}

impl PrimoDoc {
    fn into_result(self, rank: usize) -> Option<SearchResult> {
        let pnx = self.pnx?;
        let display = pnx.display?;
        let title = first_string(&display.title)?.trim().to_owned();
        if title.is_empty() {
            return None;
        }
        let record_id = pnx.control.and_then(|c| first_string(&c.recordid));
        let url = record_id
            .map(|id| format!("https://{HOST}/discovery/fulldisplay?docid={id}&vid={VID}"))
            .unwrap_or_else(|| format!("https://{HOST}/discovery/search?vid={VID}"));
        let snippet = first_string(&display.description).unwrap_or_default();

        Some(SearchResult {
            title,
            url,
            snippet,
            doi: None,
            arxiv_id: None,
            source: BackendId::Primo,
            rank,
        })
    }
}

#[derive(Deserialize)]
struct PrimoPnx {
    display: Option<PrimoDisplay>,
    control: Option<PrimoControl>,
}

#[derive(Deserialize)]
struct PrimoDisplay {
    #[serde(default)]
    title: serde_json::Value,
    #[serde(default)]
    description: serde_json::Value,
}

#[derive(Deserialize)]
struct PrimoControl {
    #[serde(default)]
    recordid: serde_json::Value,
}

fn first_string(value: &serde_json::Value) -> Option<String> {
    value
        .as_str()
        .map(ToOwned::to_owned)
        .or_else(|| value.as_array()?.first()?.as_str().map(ToOwned::to_owned))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn minimal_url_encoding_preserves_primo_commas_normal() {
        assert_eq!(
            urlencoding_minimal("any,contains,rust traits"),
            "any%2Ccontains%2Crust%20traits"
        );
    }

    #[test]
    fn parse_primo_response_maps_docs_normal() {
        let response: PrimoResponse = serde_json::from_str(
            r#"{
                "docs":[{"pnx":{"display":{"title":["Rust async traits"],"description":["Library discovery result"]},"control":{"recordid":["alma123"]}}}]
            }"#,
        )
        .expect("valid primo response");

        let results = response.into_results(5, 0);

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].source, BackendId::Primo);
        assert_eq!(results[0].title, "Rust async traits");
        assert!(results[0].url.contains("alma123"));
    }

    #[test]
    fn parse_primo_response_accepts_string_fields_robust() {
        let response: PrimoResponse = serde_json::from_str(
            r#"{
                "docs":[{"pnx":{"display":{"title":"Rust async traits","description":"Library discovery result"},"control":{"recordid":"alma123"}}}]
            }"#,
        )
        .expect("valid primo response");

        let results = response.into_results(5, 0);

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].title, "Rust async traits");
        assert_eq!(results[0].snippet, "Library discovery result");
    }
}
