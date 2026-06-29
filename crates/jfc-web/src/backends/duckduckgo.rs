use crate::backend::{BackendId, BackendResult, SearchBackend, SearchResult};
use async_trait::async_trait;
use reqwest::header::{ACCEPT, USER_AGENT};

use super::trace::{self, BackendResultTrace, BackendStart};

const DUCKDUCKGO_HTML_URL: &str = "https://html.duckduckgo.com/html/";
const DUCKDUCKGO_USER_AGENT: &str =
    "Mozilla/5.0 (compatible; jfc-web/1.0; +https://github.com/coleleavitt/jfc)";

pub struct DuckDuckGoBackend;

#[async_trait]
impl SearchBackend for DuckDuckGoBackend {
    fn id(&self) -> BackendId {
        BackendId::DuckDuckGo
    }

    fn is_available(&self) -> bool {
        true
    }

    async fn search(&self, query: &str, max_results: usize) -> BackendResult {
        let _linkscope_search = linkscope::phase("web.backend.duckduckgo.search");
        trace::backend_start(BackendStart {
            backend: "duckduckgo",
            query,
            max_results,
        });
        match search_duckduckgo_html(query, max_results).await {
            Ok(results) if !results.is_empty() => {
                trace::backend_result(BackendResultTrace {
                    backend: "duckduckgo",
                    status: "html_ok",
                    results: results.len(),
                });
                Ok(results)
            }
            Ok(_) => {
                linkscope::record_items("web.backend.duckduckgo.html_empty", 1);
                crate::search_duckduckgo_structured(query, max_results).await
            }
            Err(html_err) => match crate::search_duckduckgo_structured(query, max_results).await {
                Ok(results) => Ok(results),
                Err(instant_err) => Err(format!(
                    "DuckDuckGo HTML search failed: {html_err}; Instant Answer failed: {instant_err}"
                )),
            },
        }
    }
}

async fn search_duckduckgo_html(query: &str, max_results: usize) -> BackendResult {
    let _linkscope_html = linkscope::phase("web.backend.duckduckgo.html");
    let client = crate::http_client()?;
    let resp = client
        .get(DUCKDUCKGO_HTML_URL)
        .header(USER_AGENT, DUCKDUCKGO_USER_AGENT)
        .header(ACCEPT, "text/html,application/xhtml+xml")
        .query(&[("q", query)])
        .send()
        .await
        .map_err(|e| format!("DuckDuckGo HTML request failed: {e}"))?;

    let status = resp.status();
    linkscope::record_items("web.backend.duckduckgo.http", u64::from(status.as_u16()));
    let body = resp
        .text()
        .await
        .map_err(|e| format!("DuckDuckGo HTML response read failed: {e}"))?;
    if !status.is_success() {
        return Err(format!("DuckDuckGo HTML returned HTTP {status}"));
    }

    trace::bytes("web.backend.duckduckgo.body", body.len());
    let results = parse_duckduckgo_html_results(&body, max_results);
    trace::backend_result(BackendResultTrace {
        backend: "duckduckgo",
        status: "html_parsed",
        results: results.len(),
    });
    Ok(results)
}

fn parse_duckduckgo_html_results(html: &str, max_results: usize) -> Vec<SearchResult> {
    let limit = max_results.clamp(1, 20);
    let mut results = Vec::new();
    let mut rest = html;

    while results.len() < limit {
        let Some(class_idx) = rest.find("class=\"result__a\"") else {
            break;
        };
        let anchor = &rest[class_idx..];
        let Some(title_start) = anchor.find('>') else {
            break;
        };
        let title_body = &anchor[title_start + 1..];
        let Some(title_end) = title_body.find("</a>") else {
            break;
        };

        let href = attr_value(anchor, "href")
            .and_then(decode_duckduckgo_result_url)
            .unwrap_or_default();
        let title = html_fragment_to_text(&title_body[..title_end]);
        let after_title = &title_body[title_end + "</a>".len()..];
        let snippet = snippet_text(after_title);

        if !title.is_empty() && !href.is_empty() {
            results.push(SearchResult {
                title,
                url: href,
                snippet,
                doi: None,
                arxiv_id: None,
                source: BackendId::DuckDuckGo,
                rank: results.len() + 1,
            });
        }
        rest = after_title;
    }

    results
}

fn snippet_text(segment_after_title: &str) -> String {
    let Some(class_idx) = segment_after_title.find("class=\"result__snippet\"") else {
        return String::new();
    };
    let snippet = &segment_after_title[class_idx..];
    let Some(body_start) = snippet.find('>') else {
        return String::new();
    };
    let body = &snippet[body_start + 1..];
    let Some(body_end) = body.find("</a>") else {
        return String::new();
    };
    html_fragment_to_text(&body[..body_end])
}

fn attr_value<'a>(html: &'a str, attr: &str) -> Option<&'a str> {
    let needle = format!("{attr}=\"");
    let value = html.split_once(&needle)?.1;
    let end = value.find('"')?;
    Some(&value[..end])
}

fn decode_duckduckgo_result_url(href: &str) -> Option<String> {
    let href = crate::decode_basic_entities(href);
    if let Some(query) = href.split_once('?').map(|(_, q)| q) {
        for pair in query.split('&') {
            if let Some(raw) = pair.strip_prefix("uddg=") {
                return urlencoding::decode(raw).ok().map(|url| url.into_owned());
            }
        }
    }

    if href.starts_with("//") {
        Some(format!("https:{href}"))
    } else if href.starts_with("http://") || href.starts_with("https://") {
        Some(href)
    } else {
        None
    }
}

fn html_fragment_to_text(html: &str) -> String {
    let stripped = crate::strip_html_tags(html);
    let decoded = crate::decode_basic_entities(&stripped);
    decoded.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_duckduckgo_html_results_decodes_redirects_normal() {
        let html = r#"
            <div class="result results_links results_links_deep web-result ">
              <a rel="nofollow" class="result__a" href="//duckduckgo.com/l/?uddg=https%3A%2F%2Fdocs.rs%2Fasync%2Dtrait%2Flatest%2Fasync_trait%2F&amp;rut=abc">async_trait - Rust - Docs.rs</a>
              <a class="result__snippet" href="//duckduckgo.com/l/?uddg=https%3A%2F%2Fdocs.rs%2Fasync%2Dtrait%2Flatest%2Fasync_trait%2F&amp;rut=abc">Type erasure for <b>async</b> <b>trait</b> methods.</a>
            </div>
        "#;

        let results = parse_duckduckgo_html_results(html, 5);

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].title, "async_trait - Rust - Docs.rs");
        assert_eq!(
            results[0].url,
            "https://docs.rs/async-trait/latest/async_trait/"
        );
        assert_eq!(results[0].snippet, "Type erasure for async trait methods.");
        assert_eq!(results[0].source, BackendId::DuckDuckGo);
        assert_eq!(results[0].rank, 1);
    }
}
