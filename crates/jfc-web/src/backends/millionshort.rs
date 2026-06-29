use async_trait::async_trait;
use reqwest::header::{ACCEPT, USER_AGENT};
use serde::Deserialize;
use std::path::PathBuf;
use std::time::Duration;

use super::trace::{self, BackendResultTrace, BackendStart};
use crate::backend::{BackendId, BackendResult, SearchBackend, SearchResult};

const MILLIONSHORT_BASE: &str = "https://millionshort.com";
const MILLIONSHORT_USER_AGENT: &str =
    "Mozilla/5.0 (compatible; jfc-web/1.0; +https://github.com/coleleavitt/jfc)";

pub struct MillionShortBackend;

#[async_trait]
impl SearchBackend for MillionShortBackend {
    fn id(&self) -> BackendId {
        BackendId::MillionShort
    }

    fn is_available(&self) -> bool {
        true
    }

    async fn search(&self, query: &str, max_results: usize) -> BackendResult {
        let _linkscope_search = linkscope::phase("web.backend.millionshort.search");
        trace::backend_start(BackendStart {
            backend: "millionshort",
            query,
            max_results,
        });
        search_millionshort_structured(query, max_results).await
    }

    fn timeout(&self) -> Duration {
        trace::timeout("millionshort", Duration::from_secs(15));
        Duration::from_secs(15)
    }
}

#[derive(Deserialize)]
struct CredentialsFile {
    millionshort: Option<MillionShortCredentials>,
}

#[derive(Deserialize)]
struct MillionShortCredentials {
    username: String,
    password: String,
}

pub async fn search_millionshort_structured(query: &str, max_results: usize) -> BackendResult {
    let _linkscope_search = linkscope::phase("web.backend.millionshort.structured");
    let client = reqwest::Client::builder()
        .user_agent(MILLIONSHORT_USER_AGENT)
        .timeout(Duration::from_secs(15))
        .cookie_store(true)
        .build()
        .map_err(|e| format!("Million Short client init failed: {e}"))?;
    let search_path = format!(
        "/search?keywords={}&remove=",
        urlencoding::encode(query).into_owned()
    );

    if let Some(creds) = load_credentials() {
        linkscope::record_items("web.backend.millionshort.credentials", 1);
        let _ = client
            .post(format!("{MILLIONSHORT_BASE}/_login"))
            .header(ACCEPT, "text/html,application/xhtml+xml")
            .form(&[
                ("email", creds.username.as_str()),
                ("password", creds.password.as_str()),
                ("redirect", search_path.as_str()),
            ])
            .send()
            .await;
    }

    let resp = client
        .get(format!("{MILLIONSHORT_BASE}{search_path}"))
        .header(USER_AGENT, MILLIONSHORT_USER_AGENT)
        .header(ACCEPT, "text/html,application/xhtml+xml")
        .send()
        .await
        .map_err(|e| format!("Million Short request failed: {e}"))?;
    let status = resp.status();
    linkscope::record_items(
        "web.backend.millionshort.http_status",
        u64::from(status.as_u16()),
    );
    let body = resp
        .text()
        .await
        .map_err(|e| format!("Million Short response read failed: {e}"))?;
    if !status.is_success() {
        return Err(format!("Million Short returned HTTP {status}"));
    }
    if is_login_page(&body) {
        return Err(
            "Million Short returned its login page; verify ~/.config/jfc/credentials.toml [millionshort] credentials"
                .to_owned(),
        );
    }

    trace::bytes("web.backend.millionshort.body", body.len());
    let results = parse_millionshort_results(&body, max_results);
    trace::backend_result(BackendResultTrace {
        backend: "millionshort",
        status: "parsed",
        results: results.len(),
    });
    Ok(results)
}

fn load_credentials() -> Option<MillionShortCredentials> {
    let home = std::env::var("HOME").ok()?;
    let path = PathBuf::from(home).join(".config/jfc/credentials.toml");
    let content = std::fs::read_to_string(path).ok()?;
    let parsed: CredentialsFile = toml::from_str(&content).ok()?;
    parsed
        .millionshort
        .filter(|c| !c.username.is_empty() && !c.password.is_empty())
}

fn is_login_page(html: &str) -> bool {
    html.contains("Login | Million Short") || html.contains("Login to continue")
}

fn parse_millionshort_results(html: &str, max_results: usize) -> Vec<SearchResult> {
    if is_login_page(html) {
        return Vec::new();
    }

    let limit = max_results.clamp(1, 20);
    let mut results = Vec::new();
    let mut rest = html;

    while results.len() < limit {
        let Some(title_class) = rest.find("class=\"resultsTitle\"") else {
            break;
        };
        let title_segment = &rest[title_class..];
        let Some(anchor_start) = title_segment.find("<a") else {
            break;
        };
        let anchor = &title_segment[anchor_start..];
        let Some(body_start) = anchor.find('>') else {
            break;
        };
        let body = &anchor[body_start + 1..];
        let Some(body_end) = body.find("</a>") else {
            break;
        };

        let title = html_fragment_to_text(&body[..body_end]);
        let url = attr_value(anchor, "href").unwrap_or_default().to_owned();
        let after_title = &body[body_end + "</a>".len()..];
        let snippet = description_text(after_title);

        if !title.is_empty() && !url.is_empty() {
            results.push(SearchResult {
                title,
                url,
                snippet,
                doi: None,
                arxiv_id: None,
                source: BackendId::MillionShort,
                rank: results.len() + 1,
            });
        }
        rest = after_title;
    }

    results
}

fn description_text(segment_after_title: &str) -> String {
    let Some(class_idx) = segment_after_title.find("class=\"resultsDescription\"") else {
        return String::new();
    };
    let description = &segment_after_title[class_idx..];
    let Some(body_start) = description.find('>') else {
        return String::new();
    };
    let body = &description[body_start + 1..];
    let body_end = body.find("</div>").unwrap_or(body.len());
    html_fragment_to_text(&body[..body_end])
}

fn attr_value<'a>(html: &'a str, attr: &str) -> Option<&'a str> {
    let needle = format!("{attr}=\"");
    let value = html.split_once(&needle)?.1;
    let end = value.find('"')?;
    Some(&value[..end])
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
    fn parse_millionshort_results_reads_result_cards_normal() {
        let html = r#"
            <ul class="resultsList">
              <li>
                <div class="resultsTitle"><a href="https://example.com/article">Example &amp; Result</a></div>
                <div class="siteUrl"><a href="https://example.com/article">example.com/article</a></div>
                <div class="resultsDescription">Useful <b>snippet</b> text.</div>
              </li>
            </ul>
        "#;

        let results = parse_millionshort_results(html, 5);

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].title, "Example & Result");
        assert_eq!(results[0].url, "https://example.com/article");
        assert_eq!(results[0].snippet, "Useful snippet text.");
        assert_eq!(results[0].source, BackendId::MillionShort);
        assert_eq!(results[0].rank, 1);
    }

    #[test]
    fn parse_millionshort_login_page_returns_empty_robust() {
        let results = parse_millionshort_results(
            r#"<html><title>Login | Million Short</title><h6>Login to continue</h6></html>"#,
            5,
        );

        assert!(results.is_empty());
    }
}
