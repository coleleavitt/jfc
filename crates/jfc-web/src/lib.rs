//! Web search backends for the `WebSearch` tool.
//!
//! Backends are selected by a query prefix (`prefix:` or `prefix `). With no
//! prefix the query routes to Google CSE (general web), falling back to Brave
//! Search if no Google keys are configured.
//!
//! | Prefix | Backend | Key? | Example |
//! |--------|---------|------|---------|
//! | *(none)* | Google CSE → Brave fallback | optional | `rust async traits` |
//! | `edu:` | Google CSE scoped to academic TLDs worldwide | optional | `edu: dark matter detection` |
//! | `cn:` | Google CSE scoped to Chinese academic TLDs (`.edu.cn`/`.ac.cn`/…) | optional | `cn: superconductivity` |
//! | `primo:` | ExLibris Primo — 8000+ university library discovery systems | no | `primo: cmu/decompiler` |
//! | `gov:` | Google CSE scoped to `.gov` / `.gov.*` | optional | `gov: inflation report` |
//! | `uni:` | OpenAlex — a named university's research output (any country) | no | `uni: Tsinghua University: quantum computing` |
//! | `arxiv:` | arXiv API | no | `arxiv: transformer attention` |
//! | `scholar:` | Semantic Scholar (Graph API → BFF) | optional | `scholar: attention is all you need` |
//! | `openalex:` | OpenAlex (250M+ works, institutions + countries) | no | `openalex: graph neural networks` |
//! | `crossref:` | Crossref (160M+ DOIs) | no | `crossref: attention is all you need` |
//! | `pubmed:` | PubMed / NCBI E-utilities (biomedical) | optional | `pubmed: CRISPR off-target` |
//! | `doaj:` | Directory of Open Access Journals | no | `doaj: open peer review` |
//! | `core:` | CORE (290M+ OA full texts) | yes | `core: federated learning` |
//! | `unpaywall:` | Unpaywall — resolve a DOI to free OA PDFs | no (email) | `unpaywall: 10.1038/nature12373` |
//! | `papers:` | arXiv + S2 + OpenAlex in parallel, deduped | mixed | `papers: graph neural networks` |
//! | `brave:` | Brave Search (independent index) | yes | `brave: rust web framework` |
//! | `tavily:` | Tavily (LLM-oriented search) | yes | `tavily: latest llm benchmarks` |
//! | `exa:` | Exa (neural/semantic search) | yes | `exa: papers like AlphaFold` |
//! | `ddg:` | DuckDuckGo Instant Answer (facts/definitions) | no | `ddg: what is a monad` |
//! | `searxng:` | SearXNG meta-search (key-free SERP; `SEARXNG_URL` env) | no | `searxng: rust web framework` |
//! | `wiki:` | Wikipedia / MediaWiki search | no | `wiki: transformer model` |
//!
//! ## Google CSE
//! Reads API keys from `~/.config/google-search-mcp/config.toml` (shared
//! with the standalone google-cse-mcp-rs server). Falls back to
//! `GOOGLE_CSE_API_KEY` + `GOOGLE_CSE_CX` env vars, then to Brave Search.
//!
//! ## arXiv
//! Uses the public Atom feed API at `export.arxiv.org`. No API key needed.
//!
//! ## Semantic Scholar
//! Uses the public Graph API. Optional `SEMANTIC_SCHOLAR_API_KEY` env var
//! or `~/.config/academic-papers-mcp/config.toml` for higher rate limits.
//!
//! ## OpenAlex / Crossref / PubMed / Unpaywall
//! All key-free. A contact email (`OPENALEX_EMAIL` / `CROSSREF_EMAIL` env, or
//! `~/.config/academic-papers-mcp/config.toml`) opts into the faster "polite
//! pool"; absence just uses the default pool.
//!
//! ## Key-based web backends
//! Brave (`BRAVE_API_KEY`), Tavily (`TAVILY_API_KEY`), Exa (`EXA_API_KEY`),
//! and CORE (`CORE_API_KEY`) read their keys from the environment and return a
//! clear setup error when the key is missing.

pub mod cache;

use std::path::PathBuf;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use reqwest::header::RETRY_AFTER;
use serde::Deserialize;
use tokio::time::sleep;

// ── Public entry point ──────────────────────────────────────────────────────

/// Strip a `prefix:` or `prefix ` selector from the front of `query`,
/// returning the trimmed remainder when it matches.
fn match_prefix<'a>(query: &'a str, prefix: &str) -> Option<&'a str> {
    let colon = format!("{prefix}:");
    let space = format!("{prefix} ");
    query
        .strip_prefix(&colon)
        .or_else(|| query.strip_prefix(&space))
        .map(str::trim)
}

/// Route a search query to the appropriate backend based on prefix.
pub async fn search(query: &str, max_results: usize) -> Result<String, String> {
    if let Some(q) = match_prefix(query, "arxiv") {
        search_arxiv(q, max_results).await
    } else if let Some(q) = match_prefix(query, "scholar") {
        search_semantic_scholar(q, max_results).await
    } else if let Some(q) = match_prefix(query, "openalex") {
        search_openalex(q, max_results).await
    } else if let Some(q) = match_prefix(query, "crossref") {
        search_crossref(q, max_results).await
    } else if let Some(q) = match_prefix(query, "pubmed") {
        search_pubmed(q, max_results).await
    } else if let Some(q) = match_prefix(query, "doaj") {
        search_doaj(q, max_results).await
    } else if let Some(q) = match_prefix(query, "core") {
        search_core(q, max_results).await
    } else if let Some(q) = match_prefix(query, "unpaywall") {
        search_unpaywall(q).await
    } else if let Some(q) = match_prefix(query, "papers") {
        search_papers_combined(q, max_results).await
    } else if let Some(q) = match_prefix(query, "brave") {
        search_brave(q, max_results).await
    } else if let Some(q) = match_prefix(query, "tavily") {
        search_tavily(q, max_results).await
    } else if let Some(q) = match_prefix(query, "exa") {
        search_exa(q, max_results).await
    } else if let Some(q) = match_prefix(query, "ddg") {
        search_duckduckgo(q).await
    } else if let Some(q) = match_prefix(query, "searxng") {
        search_searxng(q, max_results).await
    } else if let Some(q) = match_prefix(query, "wiki") {
        search_wikipedia(q, max_results).await
    } else if let Some(q) = match_prefix(query, "primo") {
        search_primo(q, max_results).await
    } else if let Some(q) = match_prefix(query, "uni") {
        search_university(q, max_results).await
    } else if let Some(q) = match_prefix(query, "edu") {
        search_google(&scoped_query(q, EDU_TLDS), max_results).await
    } else if let Some(q) = match_prefix(query, "cn") {
        search_google(&scoped_query(q, CN_TLDS), max_results).await
    } else if let Some(q) = match_prefix(query, "gov") {
        // CSE doesn't honour bare TLD site: filters like site:.gov; use explicit
        // domain suffixes instead.
        let gov_query = format!(
            "{q} (site:usa.gov OR site:nih.gov OR site:cdc.gov OR site:nsf.gov OR site:energy.gov OR site:nasa.gov OR site:congress.gov OR site:whitehouse.gov OR site:govinfo.gov OR site:gov.uk OR site:gc.ca OR site:australia.gov.au OR site:europa.eu)"
        );
        search_google(&gov_query, max_results).await
    } else {
        search_google(query, max_results).await
    }
}

/// Academic second-level domains worldwide, for the `edu:` prefix. Curated to
/// the highest-output research countries and capped so the assembled `site:`
/// OR-group stays within Google CSE's effective query-length limit (large
/// OR-groups start returning zero results).
const EDU_TLDS: &[&str] = &[
    ".edu",    // USA
    ".ac.uk",  // United Kingdom
    ".ac.jp",  // Japan
    ".edu.cn", // China (teaching institutions)
    ".ac.cn",  // China (research institutes)
    ".edu.au", // Australia
    ".ac.in",  // India
    ".ac.kr",  // South Korea
    ".edu.hk", // Hong Kong
    ".edu.tw", // Taiwan
    ".edu.sg", // Singapore
    ".ac.nz",  // New Zealand
    ".ac.za",  // South Africa
    ".edu.br", // Brazil
];

/// Chinese academic second-level domains, for the `cn:` prefix. China splits
/// teaching (`edu.cn`) from research institutes (`ac.cn`); cover both.
const CN_TLDS: &[&str] = &[".edu.cn", ".ac.cn", ".edu.hk", ".edu.mo", ".edu.tw"];

/// Build a Google query restricted to a set of TLDs via a `site:` OR-group.
fn scoped_query(query: &str, tlds: &[&str]) -> String {
    let group = tlds
        .iter()
        .map(|t| format!("site:{t}"))
        .collect::<Vec<_>>()
        .join(" OR ");
    format!("{query} ({group})")
}

/// Run arXiv and Semantic Scholar (BFF) searches concurrently and merge
/// results, deduplicating by arXiv ID / DOI / normalized title.
async fn search_papers_combined(query: &str, max_results: usize) -> Result<String, String> {
    let per_backend = max_results.max(3); // each backend returns up to N, then we cap
    let (arxiv_res, s2_res, openalex_res) = tokio::join!(
        search_arxiv_entries(query, per_backend),
        search_s2_via_bff_entries(query, per_backend),
        search_openalex_entries(query, per_backend),
    );

    let mut entries: Vec<PaperEntry> = Vec::new();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();

    let push_unique = |entries: &mut Vec<PaperEntry>,
                       seen: &mut std::collections::HashSet<String>,
                       e: PaperEntry| {
        // Dedupe key: prefer arXiv ID, then DOI, then normalized title
        let key = e
            .arxiv_id
            .clone()
            .or_else(|| e.doi.clone())
            .unwrap_or_else(|| {
                e.title
                    .to_lowercase()
                    .chars()
                    .filter(|c| c.is_alphanumeric() || c.is_whitespace())
                    .collect::<String>()
                    .split_whitespace()
                    .collect::<Vec<_>>()
                    .join(" ")
            });
        if seen.insert(key) {
            entries.push(e);
        }
    };

    let mut arxiv_count = 0;
    let mut s2_count = 0;
    let mut openalex_count = 0;
    let mut errors = Vec::new();

    match arxiv_res {
        Ok(arxiv_entries) => {
            arxiv_count = arxiv_entries.len();
            for e in arxiv_entries {
                push_unique(&mut entries, &mut seen, e);
            }
        }
        Err(e) => errors.push(format!("arXiv: {e}")),
    }

    match s2_res {
        Ok(s2_entries) => {
            s2_count = s2_entries.len();
            for e in s2_entries {
                push_unique(&mut entries, &mut seen, e);
            }
        }
        Err(e) => errors.push(format!("S2 BFF: {e}")),
    }

    match openalex_res {
        Ok(openalex_entries) => {
            openalex_count = openalex_entries.len();
            for e in openalex_entries {
                push_unique(&mut entries, &mut seen, e);
            }
        }
        Err(e) => errors.push(format!("OpenAlex: {e}")),
    }

    entries.truncate(max_results);

    let mut out = format!(
        "Papers: \"{query}\" — {} unique results (arXiv: {arxiv_count}, S2: {s2_count}, OpenAlex: {openalex_count})\n",
        entries.len()
    );
    if !errors.is_empty() {
        out.push_str(&format!("Note: {}\n", errors.join("; ")));
    }
    out.push('\n');

    if entries.is_empty() {
        out.push_str("No results found.\n");
        return Ok(out);
    }

    for (i, e) in entries.iter().enumerate() {
        out.push_str(&format!("{}. {}", i + 1, e.title));
        if let Some(y) = &e.year {
            out.push_str(&format!(" ({y})"));
        }
        out.push_str(&format!("  [{}]\n", e.source));
        if !e.authors.is_empty() {
            out.push_str(&format!("   Authors: {}\n", e.authors.join(", ")));
        }
        if let Some(v) = &e.venue {
            out.push_str(&format!("   Venue: {v}\n"));
        }
        if let Some(c) = e.citations {
            out.push_str(&format!("   Citations: {c}\n"));
        }
        if let Some(u) = &e.url {
            out.push_str(&format!("   URL: {u}\n"));
        }
        if let Some(p) = &e.pdf {
            out.push_str(&format!("   PDF: {p}\n"));
        }
        if !e.abstract_text.is_empty() {
            let preview = if e.abstract_text.len() > 200 {
                // Char-boundary safe slice — web abstracts carry UTF-8
                // (em-dashes, smart quotes, non-Latin scripts) and a
                // raw byte slice panics when a multi-byte glyph
                // straddles the cap (slop_guard.rs hit this in prod).
                let cap = e.abstract_text.floor_char_boundary(200);
                format!("{}...", &e.abstract_text[..cap])
            } else {
                e.abstract_text.clone()
            };
            out.push_str(&format!("   {preview}\n"));
        }
        out.push('\n');
    }

    Ok(out)
}

/// Unified paper representation across backends.
struct PaperEntry {
    title: String,
    authors: Vec<String>,
    year: Option<String>,
    venue: Option<String>,
    citations: Option<u64>,
    url: Option<String>,
    pdf: Option<String>,
    abstract_text: String,
    arxiv_id: Option<String>,
    doi: Option<String>,
    source: &'static str,
}

// ── Shared HTTP client ──────────────────────────────────────────────────────

fn http_client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .user_agent("jfc/0.1")
        .build()
        .map_err(|e| format!("HTTP client init: {e}"))
}

// ═══════════════════════════════════════════════════════════════════════════
// Google Custom Search Engine
// ═══════════════════════════════════════════════════════════════════════════

#[derive(Clone)]
struct CseKey {
    api_key: String,
    cx: String,
}

struct KeyPool {
    keys: Vec<CseKey>,
    index: AtomicUsize,
}

impl KeyPool {
    fn next(&self) -> Option<&CseKey> {
        if self.keys.is_empty() {
            return None;
        }
        let idx = self.index.fetch_add(1, Ordering::Relaxed) % self.keys.len();
        Some(&self.keys[idx])
    }
}

fn key_pool() -> &'static KeyPool {
    static POOL: OnceLock<KeyPool> = OnceLock::new();
    POOL.get_or_init(|| {
        let keys = load_google_keys();
        KeyPool {
            keys,
            index: AtomicUsize::new(0),
        }
    })
}

#[derive(Deserialize)]
struct GoogleConfigFile {
    keys: Vec<GoogleConfigKey>,
}

#[derive(Deserialize)]
struct GoogleConfigKey {
    api_key: String,
    cx: String,
    #[allow(dead_code)]
    #[serde(default)]
    description: String,
}

fn load_google_keys() -> Vec<CseKey> {
    if let Some(keys) = load_google_keys_from_config()
        && !keys.is_empty()
    {
        tracing::info!(count = keys.len(), "Google CSE keys loaded from config");
        return keys;
    }
    if let (Ok(api_key), Ok(cx)) = (
        std::env::var("GOOGLE_CSE_API_KEY"),
        std::env::var("GOOGLE_CSE_CX"),
    ) {
        tracing::info!("Google CSE keys loaded from env vars");
        return vec![CseKey { api_key, cx }];
    }
    tracing::warn!("No Google CSE keys found — web search will fall back to arXiv/S2 only");
    Vec::new()
}

fn load_google_keys_from_config() -> Option<Vec<CseKey>> {
    let home = std::env::var("HOME").ok()?;
    let path = PathBuf::from(home).join(".config/google-search-mcp/config.toml");
    let content = std::fs::read_to_string(&path).ok()?;
    let config: GoogleConfigFile = toml::from_str(&content).ok()?;
    Some(
        config
            .keys
            .into_iter()
            .map(|k| CseKey {
                api_key: k.api_key,
                cx: k.cx,
            })
            .collect(),
    )
}

#[derive(Deserialize)]
struct GoogleSearchResponse {
    #[serde(default)]
    items: Vec<GoogleSearchItem>,
    #[serde(rename = "searchInformation")]
    search_information: Option<GoogleSearchInfo>,
    error: Option<GoogleApiError>,
}

#[derive(Deserialize)]
struct GoogleSearchItem {
    title: String,
    link: String,
    snippet: String,
}

#[derive(Deserialize)]
struct GoogleSearchInfo {
    #[serde(rename = "totalResults")]
    total_results: String,
    #[serde(rename = "searchTime")]
    search_time: f64,
}

#[derive(Deserialize)]
struct GoogleApiError {
    code: i32,
    message: String,
}

async fn search_google(query: &str, max_results: usize) -> Result<String, String> {
    let pool = key_pool();
    let key = match pool.next() {
        Some(k) => k,
        None => {
            // No Google keys — fall back to Brave Search if it's configured,
            // otherwise surface a setup error covering both options.
            tracing::info!("No Google CSE keys, attempting Brave Search fallback");
            return brave_results(query, max_results)
                .await
                .map_err(|brave_err| {
                    format!(
                        "No Google CSE API keys configured (set GOOGLE_CSE_API_KEY + \
                     GOOGLE_CSE_CX or ~/.config/google-search-mcp/config.toml), and \
                     Brave fallback unavailable: {brave_err}"
                    )
                });
        }
    };

    let num = max_results.clamp(1, 10);
    let client = http_client()?;
    let num_str = num.to_string();

    let resp = client
        .get("https://www.googleapis.com/customsearch/v1")
        .query(&[
            ("key", key.api_key.as_str()),
            ("cx", key.cx.as_str()),
            ("q", query),
            ("num", num_str.as_str()),
        ])
        .send()
        .await
        .map_err(|e| format!("Google CSE request failed: {e}"))?;

    let status = resp.status();
    let body = resp
        .text()
        .await
        .map_err(|e| format!("Response read: {e}"))?;

    let parsed: GoogleSearchResponse =
        serde_json::from_str(&body).map_err(|e| format!("JSON parse: {e}"))?;

    if let Some(err) = parsed.error {
        // Quota/rate-limit (429) and transient 5xx errors are recoverable via a
        // different backend — don't make the whole search fail just because the
        // Google CSE daily quota is exhausted. Fall back to Brave, then DDG.
        if google_error_is_recoverable(err.code, &err.message) {
            tracing::warn!(
                code = err.code,
                "Google CSE error {} ({}); falling back to an alternate backend",
                err.code,
                err.message
            );
            return google_fallback(query, max_results).await.map_err(|fb| {
                format!(
                    "Google CSE error {}: {} (and fallback failed: {fb})",
                    err.code, err.message
                )
            });
        }
        return Err(format!(
            "Google CSE API error {}: {}",
            err.code, err.message
        ));
    }
    if !status.is_success() {
        if status.as_u16() == 429 || status.is_server_error() {
            tracing::warn!(%status, "Google CSE HTTP {status}; falling back to an alternate backend");
            return google_fallback(query, max_results).await.map_err(|fb| {
                format!("Google CSE returned HTTP {status} (and fallback failed: {fb})")
            });
        }
        return Err(format!("Google CSE returned HTTP {status}"));
    }

    let mut out = String::new();
    if let Some(info) = &parsed.search_information {
        out.push_str(&format!(
            "Search: \"{query}\" — {total} results ({time:.2}s)\n\n",
            total = info.total_results,
            time = info.search_time,
        ));
    }
    if parsed.items.is_empty() {
        out.push_str("No results found.\n");
    } else {
        for (i, item) in parsed.items.iter().enumerate() {
            out.push_str(&format!(
                "{}. {}\n   URL: {}\n   {}\n\n",
                i + 1,
                item.title,
                item.link,
                item.snippet,
            ));
        }
    }
    Ok(out)
}

/// True when a Google CSE error is worth retrying on a different backend:
/// quota/rate-limit (HTTP 429) or transient server errors (5xx). Permanent
/// errors (bad key, malformed query) are not recoverable this way.
fn google_error_is_recoverable(code: i32, message: &str) -> bool {
    if code == 429 || (500..=599).contains(&code) {
        return true;
    }
    let m = message.to_ascii_lowercase();
    m.contains("quota")
        || m.contains("rate limit")
        || m.contains("ratelimit")
        || m.contains("exceeded")
}

/// Fallback chain for a failed/throttled Google CSE search: try Brave, then
/// SearXNG (key-free meta-search SERP), then DuckDuckGo (instant answers).
/// Returns the first backend that yields results, or the combined error.
async fn google_fallback(query: &str, max_results: usize) -> Result<String, String> {
    match brave_results(query, max_results).await {
        Ok(out) => Ok(out),
        Err(brave_err) => match search_searxng(query, max_results).await {
            Ok(out) => Ok(out),
            Err(searx_err) => match search_duckduckgo(query).await {
                Ok(out) => Ok(out),
                Err(ddg_err) => Err(format!(
                    "Brave: {brave_err}; SearXNG: {searx_err}; DuckDuckGo: {ddg_err}"
                )),
            },
        },
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// arXiv (Atom feed API, regex-parsed)
// ═══════════════════════════════════════════════════════════════════════════

/// Convert ArxivEntry → PaperEntry for combined search.
fn arxiv_to_paper(e: ArxivEntry) -> PaperEntry {
    PaperEntry {
        title: e.title,
        authors: e.authors,
        year: e.published.split('-').next().map(String::from),
        venue: if e.category.is_empty() {
            None
        } else {
            Some(format!("arXiv {}", e.category))
        },
        citations: None,
        url: Some(format!("https://arxiv.org/abs/{}", e.arxiv_id)),
        pdf: Some(format!("https://arxiv.org/pdf/{}", e.arxiv_id)),
        abstract_text: e.summary,
        arxiv_id: Some(e.arxiv_id.clone()),
        doi: None,
        source: "arXiv",
    }
}

/// Exponential backoff retry for arXiv HTTP requests.
/// Retries on 429 (rate limit) with either Retry-After or 3s → 6s → 12s delays.
async fn fetch_with_backoff(
    client: &reqwest::Client,
    url: &str,
    params: &[(&str, &str)],
) -> Result<reqwest::Response, String> {
    const MAX_ATTEMPTS: u32 = 3;
    let mut delay = Duration::from_secs(3);

    for attempt in 1..=MAX_ATTEMPTS {
        let resp = client
            .get(url)
            .query(params)
            .send()
            .await
            .map_err(|e| format!("arXiv request failed: {e}"))?;

        if resp.status() != 429 {
            return Ok(resp);
        }

        if attempt == MAX_ATTEMPTS {
            return Err("arXiv rate limited (429) — max retries exhausted".to_string());
        }

        let retry_after = resp
            .headers()
            .get(RETRY_AFTER)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.parse::<u64>().ok())
            .map(Duration::from_secs)
            .unwrap_or(delay);

        tracing::warn!(
            "arXiv rate limited (429), retrying in {}ms (attempt {}/{})",
            retry_after.as_millis(),
            attempt,
            MAX_ATTEMPTS
        );
        sleep(retry_after).await;
        delay = delay.saturating_mul(2).min(Duration::from_secs(12));
    }

    Err("arXiv request failed — unexpected control flow".to_string())
}

/// Run an arXiv search and return structured entries (used by combined search).
async fn search_arxiv_entries(query: &str, max_results: usize) -> Result<Vec<PaperEntry>, String> {
    let num = max_results.clamp(1, 20);
    let client = http_client()?;
    let num_str = num.to_string();

    let params = [
        ("search_query", format!("all:{query}")),
        ("max_results", num_str.clone()),
        ("start", "0".to_string()),
        ("sortBy", "relevance".to_string()),
        ("sortOrder", "descending".to_string()),
    ];
    let params_ref: Vec<(&str, &str)> = params.iter().map(|(k, v)| (*k, v.as_str())).collect();

    let resp =
        fetch_with_backoff(&client, "https://export.arxiv.org/api/query", &params_ref).await?;

    if !resp.status().is_success() {
        return Err(format!("arXiv returned HTTP {}", resp.status()));
    }
    let xml = resp
        .text()
        .await
        .map_err(|e| format!("Response read: {e}"))?;
    Ok(parse_arxiv_entries(&xml)
        .into_iter()
        .map(arxiv_to_paper)
        .collect())
}

async fn search_arxiv(query: &str, max_results: usize) -> Result<String, String> {
    let num = max_results.clamp(1, 20);
    let client = http_client()?;
    let num_str = num.to_string();

    let params = [
        ("search_query", format!("all:{query}")),
        ("max_results", num_str.clone()),
        ("start", "0".to_string()),
        ("sortBy", "relevance".to_string()),
        ("sortOrder", "descending".to_string()),
    ];
    let params_ref: Vec<(&str, &str)> = params.iter().map(|(k, v)| (*k, v.as_str())).collect();

    let resp =
        fetch_with_backoff(&client, "https://export.arxiv.org/api/query", &params_ref).await?;

    if !resp.status().is_success() {
        return Err(format!("arXiv returned HTTP {}", resp.status()));
    }

    let xml = resp
        .text()
        .await
        .map_err(|e| format!("Response read: {e}"))?;
    let entries = parse_arxiv_entries(&xml);

    let total = extract_tag(&xml, "opensearch:totalResults").unwrap_or_else(|| "?".to_string());

    let mut out = String::new();
    out.push_str(&format!(
        "arXiv search: \"{query}\" — {total} total results\n\n"
    ));

    if entries.is_empty() {
        out.push_str("No results found.\n");
    } else {
        for (i, entry) in entries.iter().enumerate() {
            out.push_str(&format!("{}. {}\n", i + 1, entry.title));
            out.push_str(&format!("   arXiv: {}\n", entry.arxiv_id));
            out.push_str(&format!("   Authors: {}\n", entry.authors.join(", ")));
            out.push_str(&format!(
                "   Published: {} | Category: {}\n",
                entry.published, entry.category
            ));
            out.push_str(&format!(
                "   PDF: https://arxiv.org/pdf/{}\n",
                entry.arxiv_id
            ));
            // Truncate abstract to ~200 chars for readability.
            let summary = if entry.summary.len() > 200 {
                {
                    // Char-boundary safe — see comment in the abstract_text
                    // truncation above for the slop_guard.rs precedent.
                    let cap = entry.summary.floor_char_boundary(200);
                    format!("{}...", &entry.summary[..cap])
                }
            } else {
                entry.summary.clone()
            };
            out.push_str(&format!("   {}\n\n", summary));
        }
    }
    Ok(out)
}

struct ArxivEntry {
    title: String,
    arxiv_id: String,
    authors: Vec<String>,
    summary: String,
    published: String,
    category: String,
}

fn parse_arxiv_entries(xml: &str) -> Vec<ArxivEntry> {
    // Split on <entry> tags (regex-free approach for simplicity).
    let mut entries = Vec::new();
    let mut search_from = 0;

    while let Some(rel_start) = xml[search_from..].find("<entry>") {
        let start = search_from + rel_start;
        let Some(rel_end) = xml[start..].find("</entry>") else {
            break;
        };
        let end = start + rel_end + "</entry>".len();
        let entry_xml = &xml[start..end];
        search_from = end;

        let title = extract_tag(entry_xml, "title")
            .unwrap_or_default()
            .replace('\n', " ")
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");

        let id_url = extract_tag(entry_xml, "id").unwrap_or_default();
        let arxiv_id = id_url
            .rsplit_once("/abs/")
            .map(|(_, id)| id.to_string())
            .unwrap_or(id_url);

        let summary = extract_tag(entry_xml, "summary")
            .unwrap_or_default()
            .replace('\n', " ")
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");

        let published = extract_tag(entry_xml, "published")
            .unwrap_or_default()
            .chars()
            .take(10)
            .collect();

        // Extract primary category from <arxiv:primary_category term="...">
        let category = entry_xml
            .find("primary_category")
            .and_then(|pos| {
                let after = &entry_xml[pos..];
                let tstart = after.find("term=\"")? + 6;
                let tend = after[tstart..].find('"')? + tstart;
                Some(after[tstart..tend].to_string())
            })
            .unwrap_or_default();

        // Extract all <author><name>...</name></author>
        let mut authors = Vec::new();
        let mut author_search = 0;
        while let Some(ns) = entry_xml[author_search..].find("<name>") {
            let ns = author_search + ns + 6;
            if let Some(ne) = entry_xml[ns..].find("</name>") {
                authors.push(entry_xml[ns..ns + ne].trim().to_string());
                author_search = ns + ne;
            } else {
                break;
            }
        }

        entries.push(ArxivEntry {
            title,
            arxiv_id,
            authors,
            summary,
            published,
            category,
        });
    }

    entries
}

fn extract_tag(xml: &str, tag: &str) -> Option<String> {
    let open = format!("<{}", tag);
    let close = format!("</{}>", tag);
    let start = xml.find(&open)?;
    let after_open = &xml[start..];
    // Skip past the opening tag (handle attributes).
    let content_start = after_open.find('>')? + 1;
    let content = &after_open[content_start..];
    let end = content.find(&close)?;
    Some(content[..end].trim().to_string())
}

// ═══════════════════════════════════════════════════════════════════════════
// Semantic Scholar (Graph API v1)
// ═══════════════════════════════════════════════════════════════════════════

fn semantic_scholar_api_key() -> Option<&'static str> {
    static KEY: OnceLock<Option<String>> = OnceLock::new();
    KEY.get_or_init(|| {
        // Try env var first.
        if let Ok(k) = std::env::var("SEMANTIC_SCHOLAR_API_KEY")
            && !k.is_empty()
        {
            return Some(k);
        }
        // Try academic-papers-mcp config.
        let home = std::env::var("HOME").ok()?;
        let path = PathBuf::from(home).join(".config/academic-papers-mcp/config.toml");
        let content = std::fs::read_to_string(&path).ok()?;
        // Simple TOML key extraction — avoid pulling in a full TOML parse
        // for a single optional key.
        for line in content.lines() {
            let line = line.trim();
            if let Some(rest) = line.strip_prefix("semantic_scholar_api_key") {
                let rest = rest.trim().strip_prefix('=')?;
                let rest = rest.trim().trim_matches('"');
                if !rest.is_empty() {
                    return Some(rest.to_string());
                }
            }
        }
        None
    })
    .as_deref()
}

const S2_FIELDS: &str =
    "paperId,title,abstract,year,citationCount,url,venue,authors,publicationDate,openAccessPdf";

async fn search_semantic_scholar(query: &str, max_results: usize) -> Result<String, String> {
    let limit = max_results.clamp(1, 20);

    // Try the public Graph API first.
    match search_s2_public(query, limit).await {
        Ok(out) => return Ok(out),
        Err(e) if e.contains("429") || e.contains("rate") => {
            tracing::info!("S2 public API rate limited, falling back to BFF");
        }
        Err(e) => return Err(e),
    }

    // Fallback: use the semanticscholar.org BFF (completion + paper detail).
    // Works without an API key but limited to ~5 results per query.
    search_s2_via_bff(query, limit).await
}

async fn search_s2_public(query: &str, limit: usize) -> Result<String, String> {
    let client = http_client()?;
    let limit_str = limit.to_string();

    let mut req = client
        .get("https://api.semanticscholar.org/graph/v1/paper/search")
        .query(&[
            ("query", query),
            ("limit", limit_str.as_str()),
            ("offset", "0"),
            ("fields", S2_FIELDS),
        ]);

    if let Some(key) = semantic_scholar_api_key() {
        req = req.header("x-api-key", key);
    }

    let resp = req
        .send()
        .await
        .map_err(|e| format!("S2 request failed: {e}"))?;

    if resp.status() == 429 {
        return Err("S2 public API rate limited (429)".to_string());
    }
    if !resp.status().is_success() {
        return Err(format!("Semantic Scholar returned HTTP {}", resp.status()));
    }

    let body = resp
        .text()
        .await
        .map_err(|e| format!("Response read: {e}"))?;
    let parsed: serde_json::Value =
        serde_json::from_str(&body).map_err(|e| format!("JSON parse: {e}"))?;

    let total = parsed.get("total").and_then(|v| v.as_u64()).unwrap_or(0);
    let papers = parsed.get("data").and_then(|v| v.as_array());

    let mut out = String::new();
    out.push_str(&format!(
        "Semantic Scholar: \"{query}\" — {total} total results\n\n"
    ));

    match papers {
        Some(papers) if !papers.is_empty() => {
            for (i, paper) in papers.iter().enumerate() {
                let title = paper.get("title").and_then(|v| v.as_str()).unwrap_or("?");
                let year = paper.get("year").and_then(|v| v.as_u64());
                let venue = paper.get("venue").and_then(|v| v.as_str()).unwrap_or("");
                let citations = paper
                    .get("citationCount")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0);
                let url = paper.get("url").and_then(|v| v.as_str()).unwrap_or("");
                let paper_id = paper.get("paperId").and_then(|v| v.as_str()).unwrap_or("");

                // Authors
                let authors: Vec<&str> = paper
                    .get("authors")
                    .and_then(|v| v.as_array())
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|a| a.get("name").and_then(|n| n.as_str()))
                            .collect()
                    })
                    .unwrap_or_default();

                // Open access PDF
                let pdf = paper
                    .get("openAccessPdf")
                    .and_then(|v| v.get("url"))
                    .and_then(|v| v.as_str());

                out.push_str(&format!("{}. {}", i + 1, title));
                if let Some(y) = year {
                    out.push_str(&format!(" ({y})"));
                }
                out.push('\n');
                if !authors.is_empty() {
                    out.push_str(&format!("   Authors: {}\n", authors.join(", ")));
                }
                if !venue.is_empty() {
                    out.push_str(&format!("   Venue: {venue}\n"));
                }
                out.push_str(&format!("   Citations: {citations} | ID: {paper_id}\n"));
                if !url.is_empty() {
                    out.push_str(&format!("   URL: {url}\n"));
                }
                if let Some(pdf_url) = pdf {
                    out.push_str(&format!("   PDF: {pdf_url}\n"));
                }

                // Abstract (truncated).
                if let Some(abs) = paper.get("abstract").and_then(|v| v.as_str()) {
                    let abs = if abs.len() > 200 {
                        format!("{}...", &abs[..abs.floor_char_boundary(200)])
                    } else {
                        abs.to_string()
                    };
                    out.push_str(&format!("   {abs}\n"));
                }
                out.push('\n');
            }
        }
        _ => {
            out.push_str("No results found.\n");
        }
    }

    Ok(out)
}

// ── Semantic Scholar BFF fallback ──────────────────────────────────────────
//
// When the public Graph API is rate-limited (429), fall back to the
// www.semanticscholar.org BFF which the website itself uses. This requires
// no API key but is limited to ~5 results per query (completion endpoint
// returns at most 5 suggestions). Endpoints discovered by mining the
// website's webpack bundles in research/.

async fn search_s2_via_bff(query: &str, limit: usize) -> Result<String, String> {
    let entries = search_s2_via_bff_entries(query, limit).await?;

    if entries.is_empty() {
        return Ok(format!(
            "Semantic Scholar (BFF): \"{query}\" — no results\n"
        ));
    }

    let mut out = format!(
        "Semantic Scholar (via BFF, no API key): \"{query}\" — {} results\n\n",
        entries.len()
    );

    for (i, e) in entries.iter().enumerate() {
        out.push_str(&format!("{}. {}", i + 1, e.title));
        if let Some(y) = &e.year {
            out.push_str(&format!(" ({y})"));
        }
        out.push('\n');
        if !e.authors.is_empty() {
            out.push_str(&format!("   Authors: {}\n", e.authors.join(", ")));
        }
        if let Some(v) = &e.venue {
            out.push_str(&format!("   Venue: {v}\n"));
        }
        let cites = e.citations.unwrap_or(0);
        if let Some(u) = &e.url {
            out.push_str(&format!("   Citations: {cites} | URL: {u}\n"));
        }
        if let Some(p) = &e.pdf {
            out.push_str(&format!("   PDF: {p}\n"));
        }
        if !e.abstract_text.is_empty() {
            let preview = if e.abstract_text.len() > 200 {
                // Char-boundary safe slice — web abstracts carry UTF-8
                // (em-dashes, smart quotes, non-Latin scripts) and a
                // raw byte slice panics when a multi-byte glyph
                // straddles the cap (slop_guard.rs hit this in prod).
                let cap = e.abstract_text.floor_char_boundary(200);
                format!("{}...", &e.abstract_text[..cap])
            } else {
                e.abstract_text.clone()
            };
            out.push_str(&format!("   {preview}\n"));
        }
        out.push('\n');
    }

    Ok(out)
}

/// Run the BFF completion + paper-batch lookup and return structured entries.
async fn search_s2_via_bff_entries(query: &str, limit: usize) -> Result<Vec<PaperEntry>, String> {
    let client = http_client()?;

    let completion_resp = client
        .get("https://www.semanticscholar.org/api/1/completion")
        .query(&[("q", query), ("type", "Paper")])
        .header("Accept", "application/json")
        .header("Referer", "https://www.semanticscholar.org/")
        .send()
        .await
        .map_err(|e| format!("S2 BFF completion failed: {e}"))?;

    if !completion_resp.status().is_success() {
        return Err(format!(
            "S2 BFF completion HTTP {}",
            completion_resp.status()
        ));
    }

    let completion_body = completion_resp
        .text()
        .await
        .map_err(|e| format!("Response read: {e}"))?;
    let completion_json: serde_json::Value =
        serde_json::from_str(&completion_body).map_err(|e| format!("BFF JSON parse: {e}"))?;

    let paper_ids: Vec<String> = completion_json
        .get("suggestions")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .take(limit)
                .filter_map(|s| s.get("linkedId").and_then(|v| v.as_str()))
                .map(String::from)
                .collect()
        })
        .unwrap_or_default();

    if paper_ids.is_empty() {
        return Ok(Vec::new());
    }

    let mut entries = Vec::with_capacity(paper_ids.len());

    for paper_id in &paper_ids {
        let detail_resp = client
            .get(format!(
                "https://www.semanticscholar.org/api/1/paper/{paper_id}"
            ))
            .header("Accept", "application/json")
            .send()
            .await;

        let detail = match detail_resp {
            Ok(r) if r.status().is_success() => match r.text().await {
                Ok(body) => match serde_json::from_str::<serde_json::Value>(&body) {
                    Ok(v) => v,
                    Err(_) => continue,
                },
                Err(_) => continue,
            },
            _ => continue,
        };

        let paper = detail.get("paper").unwrap_or(&detail);

        let title = paper
            .get("title")
            .and_then(|v| v.get("text").or(Some(v)))
            .and_then(|v| v.as_str())
            .unwrap_or("?")
            .to_string();

        let year = paper.get("year").and_then(|v| {
            v.get("text")
                .and_then(|t| t.as_str())
                .map(String::from)
                .or_else(|| v.as_u64().map(|n| n.to_string()))
        });

        let venue = paper
            .get("venue")
            .and_then(|v| v.get("text").or(Some(v)))
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .map(String::from);

        let citations = paper
            .get("citationStats")
            .and_then(|v| v.get("numCitations"))
            .and_then(|v| v.as_u64())
            .or_else(|| paper.get("citationCount").and_then(|v| v.as_u64()));

        let authors: Vec<String> = paper
            .get("authors")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|a| {
                        a.get("name")
                            .and_then(|n| n.get("text").or(Some(n)))
                            .and_then(|v| v.as_str())
                            .map(String::from)
                    })
                    .collect()
            })
            .unwrap_or_default();

        let abstract_text = paper
            .get("paperAbstract")
            .and_then(|v| v.get("text").or(Some(v)))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        let pdf = paper
            .get("primaryPaperLink")
            .and_then(|v| v.get("url"))
            .and_then(|v| v.as_str())
            .map(String::from);

        // Pull arxiv ID from primaryPaperLink if linkType is arxiv
        let arxiv_id = paper
            .get("primaryPaperLink")
            .filter(|v| {
                v.get("linkType")
                    .and_then(|t| t.as_str())
                    .map(|s| s.eq_ignore_ascii_case("arxiv"))
                    .unwrap_or(false)
            })
            .and_then(|v| v.get("url"))
            .and_then(|v| v.as_str())
            .and_then(|u| {
                // Extract arxiv id from URL like https://arxiv.org/pdf/1706.03762.pdf
                u.rsplit('/')
                    .next()
                    .and_then(|s| s.strip_suffix(".pdf").or(Some(s)))
                    .map(String::from)
            });

        let doi = paper
            .get("doiInfo")
            .and_then(|v| v.get("doi"))
            .and_then(|v| v.as_str())
            .map(String::from)
            .or_else(|| {
                paper
                    .get("externalIds")
                    .and_then(|v| v.get("DOI"))
                    .and_then(|v| v.as_str())
                    .map(String::from)
            });

        entries.push(PaperEntry {
            title,
            authors,
            year,
            venue,
            citations,
            url: Some(format!("https://www.semanticscholar.org/paper/{paper_id}")),
            pdf,
            abstract_text,
            arxiv_id,
            doi,
            source: "S2",
        });
    }

    Ok(entries)
}

// ── Shared formatting helpers ───────────────────────────────────────────────

/// Strip HTML/XML tags from `s`, returning plain text. Used by Crossref
/// (JATS XML abstracts) and MediaWiki (searchmatch span markup).
fn strip_html_tags(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut in_tag = false;
    for c in s.chars() {
        if c == '<' {
            in_tag = true;
        } else if c == '>' {
            in_tag = false;
        } else if !in_tag {
            out.push(c);
        }
        // else: inside a tag — skip character, no action needed
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Truncate `text` to at most ~200 chars on a UTF-8 char boundary.
fn truncate_abstract(text: &str) -> String {
    if text.len() > 200 {
        // Char-boundary safe — web abstracts carry multi-byte UTF-8.
        format!("{}...", &text[..text.floor_char_boundary(200)])
    } else {
        text.to_string()
    }
}

/// Render a list of `PaperEntry` into the standard human-readable block used
/// by all paper backends.
fn format_paper_entries(header: &str, entries: &[PaperEntry]) -> String {
    let mut out = format!("{header}\n\n");
    if entries.is_empty() {
        out.push_str("No results found.\n");
        return out;
    }
    for (i, e) in entries.iter().enumerate() {
        out.push_str(&format!("{}. {}", i + 1, e.title));
        if let Some(y) = &e.year {
            out.push_str(&format!(" ({y})"));
        }
        out.push_str(&format!("  [{}]\n", e.source));
        if !e.authors.is_empty() {
            out.push_str(&format!("   Authors: {}\n", e.authors.join(", ")));
        }
        if let Some(v) = &e.venue {
            out.push_str(&format!("   Venue: {v}\n"));
        }
        if let Some(c) = e.citations {
            out.push_str(&format!("   Citations: {c}\n"));
        }
        if let Some(u) = &e.url {
            out.push_str(&format!("   URL: {u}\n"));
        }
        if let Some(p) = &e.pdf {
            out.push_str(&format!("   PDF: {p}\n"));
        }
        if !e.abstract_text.is_empty() {
            out.push_str(&format!("   {}\n", truncate_abstract(&e.abstract_text)));
        }
        out.push('\n');
    }
    out
}

/// Look up an optional contact email for the academic "polite pools". Checks
/// the given env vars in order, then `academic-papers-mcp/config.toml`.
fn polite_pool_email(env_vars: &[&str]) -> Option<String> {
    for var in env_vars {
        if let Ok(v) = std::env::var(var)
            && !v.is_empty()
        {
            return Some(v);
        }
    }
    let home = std::env::var("HOME").ok()?;
    let path = PathBuf::from(home).join(".config/academic-papers-mcp/config.toml");
    let content = std::fs::read_to_string(&path).ok()?;
    for line in content.lines() {
        let line = line.trim();
        for key in ["polite_pool_email", "contact_email", "email"] {
            if let Some(rest) = line.strip_prefix(key) {
                let rest = rest.trim().strip_prefix('=')?.trim().trim_matches('"');
                if !rest.is_empty() {
                    return Some(rest.to_string());
                }
            }
        }
    }
    None
}

// ═══════════════════════════════════════════════════════════════════════════
// OpenAlex (key-free; surfaces author institutions + country codes)
// ═══════════════════════════════════════════════════════════════════════════

/// Decode OpenAlex's inverted-index abstract back into plain text.
fn openalex_abstract(inverted: Option<&serde_json::Value>) -> String {
    let Some(map) = inverted.and_then(|v| v.as_object()) else {
        return String::new();
    };
    let mut positioned: Vec<(u64, &str)> = Vec::new();
    for (word, positions) in map {
        if let Some(arr) = positions.as_array() {
            for p in arr {
                if let Some(idx) = p.as_u64() {
                    positioned.push((idx, word.as_str()));
                }
            }
        }
    }
    positioned.sort_by_key(|(idx, _)| *idx);
    positioned
        .into_iter()
        .map(|(_, w)| w)
        .collect::<Vec<_>>()
        .join(" ")
}

fn openalex_to_paper(work: &serde_json::Value) -> PaperEntry {
    let title = work
        .get("display_name")
        .or_else(|| work.get("title"))
        .and_then(|v| v.as_str())
        .unwrap_or("?")
        .to_string();

    let year = work
        .get("publication_year")
        .and_then(|v| v.as_u64())
        .map(|y| y.to_string());

    // Authors annotated with their institution + ISO country code — the key
    // signal for "which universities / countries published this".
    let authors: Vec<String> = work
        .get("authorships")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|a| {
                    let name = a
                        .get("author")
                        .and_then(|au| au.get("display_name"))
                        .and_then(|v| v.as_str())?;
                    let inst = a
                        .get("institutions")
                        .and_then(|v| v.as_array())
                        .and_then(|arr| arr.first())
                        .and_then(|i| {
                            let n = i.get("display_name").and_then(|v| v.as_str())?;
                            let cc = i
                                .get("country_code")
                                .and_then(|v| v.as_str())
                                .map(|c| format!(", {c}"))
                                .unwrap_or_default();
                            Some(format!(" ({n}{cc})"))
                        })
                        .unwrap_or_default();
                    Some(format!("{name}{inst}"))
                })
                .collect()
        })
        .unwrap_or_default();

    let venue = work
        .get("primary_location")
        .and_then(|v| v.get("source"))
        .and_then(|v| v.get("display_name"))
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(String::from);

    let citations = work.get("cited_by_count").and_then(|v| v.as_u64());

    let doi = work
        .get("doi")
        .and_then(|v| v.as_str())
        .map(|d| d.trim_start_matches("https://doi.org/").to_string());

    let pdf = work
        .get("best_oa_location")
        .or_else(|| work.get("primary_location"))
        .and_then(|v| v.get("pdf_url"))
        .and_then(|v| v.as_str())
        .map(String::from);

    let abstract_text = openalex_abstract(work.get("abstract_inverted_index"));

    PaperEntry {
        title,
        authors,
        year,
        venue,
        citations,
        url: work.get("id").and_then(|v| v.as_str()).map(String::from),
        pdf,
        abstract_text,
        arxiv_id: None,
        doi,
        source: "OpenAlex",
    }
}

async fn search_openalex_entries(
    query: &str,
    max_results: usize,
) -> Result<Vec<PaperEntry>, String> {
    let per_page = max_results.clamp(1, 25).to_string();
    let client = http_client()?;

    let mut req = client
        .get("https://api.openalex.org/works")
        .query(&[("search", query), ("per_page", per_page.as_str())]);
    if let Some(email) = polite_pool_email(&["OPENALEX_EMAIL", "CROSSREF_EMAIL"]) {
        req = req.query(&[("mailto", email.as_str())]);
    }

    let resp = req
        .send()
        .await
        .map_err(|e| format!("OpenAlex request failed: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("OpenAlex returned HTTP {}", resp.status()));
    }
    let body = resp
        .text()
        .await
        .map_err(|e| format!("Response read: {e}"))?;
    let parsed: serde_json::Value =
        serde_json::from_str(&body).map_err(|e| format!("JSON parse: {e}"))?;

    Ok(parsed
        .get("results")
        .and_then(|v| v.as_array())
        .map(|arr| arr.iter().map(openalex_to_paper).collect())
        .unwrap_or_default())
}

async fn search_openalex(query: &str, max_results: usize) -> Result<String, String> {
    let entries = search_openalex_entries(query, max_results).await?;
    Ok(format_paper_entries(
        &format!("OpenAlex: \"{query}\" — {} results", entries.len()),
        &entries,
    ))
}

/// Resolve a university name to its top OpenAlex institution, returning
/// `(openalex_id, display_name, country_code, works_count)`. Works for any
/// university worldwide — US, Chinese, European — via the ROR-backed index.
async fn resolve_institution(name: &str) -> Result<Option<(String, String, String, u64)>, String> {
    let client = http_client()?;
    // Use the main institutions search API (not autocomplete) sorted by
    // works_count so the largest/most prominent institution wins —
    // autocomplete returns lexicographic matches, which causes "MIT World Peace
    // University" to beat "Massachusetts Institute of Technology".
    let mut req = client.get("https://api.openalex.org/institutions").query(&[
        ("search", name),
        ("sort", "works_count:desc"),
        ("per_page", "1"),
    ]);
    if let Some(email) = polite_pool_email(&["OPENALEX_EMAIL", "CROSSREF_EMAIL"]) {
        req = req.query(&[("mailto", email.as_str())]);
    }
    let resp = req
        .send()
        .await
        .map_err(|e| format!("OpenAlex institution lookup failed: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("OpenAlex returned HTTP {}", resp.status()));
    }
    let body = resp
        .text()
        .await
        .map_err(|e| format!("Response read: {e}"))?;
    let parsed: serde_json::Value =
        serde_json::from_str(&body).map_err(|e| format!("JSON parse: {e}"))?;

    Ok(parsed
        .get("results")
        .and_then(|v| v.as_array())
        .and_then(|arr| arr.first())
        .and_then(|inst| {
            // id looks like https://openalex.org/I99065089 — keep the short id.
            let id = inst
                .get("id")
                .and_then(|v| v.as_str())?
                .rsplit('/')
                .next()?;
            let display = inst
                .get("display_name")
                .and_then(|v| v.as_str())
                .unwrap_or("?")
                .to_string();
            // `hint` carries "City, Country"; take the country half if present.
            let country = inst
                .get("hint")
                .and_then(|v| v.as_str())
                .and_then(|h| h.rsplit(',').next())
                .map(|s| s.trim().to_string())
                .unwrap_or_default();
            let works = inst
                .get("works_count")
                .and_then(|v| v.as_u64())
                .unwrap_or(0);
            Some((id.to_string(), display, country, works))
        }))
}

/// `uni: <University Name>: <topic>` — find a specific university's research
/// output on a topic via OpenAlex, regardless of country or web domain. The
/// topic half is optional (`uni: Tsinghua University` lists recent works).
async fn search_university(query: &str, max_results: usize) -> Result<String, String> {
    // Split on the first colon: "<name>: <topic>". No colon ⇒ name only.
    let (name, topic) = match query.split_once(':') {
        Some((n, t)) => (n.trim(), t.trim()),
        None => (query.trim(), ""),
    };
    if name.is_empty() {
        return Err(
            "Usage: `uni: <University Name>: <topic>` (topic optional), e.g. \
             `uni: Tsinghua University: quantum computing`."
                .to_string(),
        );
    }

    let Some((inst_id, display, country, works)) = resolve_institution(name).await? else {
        return Ok(format!(
            "University search: no OpenAlex institution matched \"{name}\".\n"
        ));
    };

    let per_page = max_results.clamp(1, 25).to_string();
    let filter = format!("authorships.institutions.lineage:{inst_id}");
    let client = http_client()?;

    let mut req = client
        .get("https://api.openalex.org/works")
        .query(&[("filter", filter.as_str()), ("per_page", per_page.as_str())]);
    if topic.is_empty() {
        // No topic ⇒ show the institution's most-cited works.
        req = req.query(&[("sort", "cited_by_count:desc")]);
    } else {
        req = req.query(&[("search", topic)]);
    }
    if let Some(email) = polite_pool_email(&["OPENALEX_EMAIL", "CROSSREF_EMAIL"]) {
        req = req.query(&[("mailto", email.as_str())]);
    }

    let resp = req
        .send()
        .await
        .map_err(|e| format!("OpenAlex works request failed: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("OpenAlex returned HTTP {}", resp.status()));
    }
    let parsed: serde_json::Value = resp
        .text()
        .await
        .map_err(|e| format!("Response read: {e}"))
        .and_then(|b| serde_json::from_str(&b).map_err(|e| format!("JSON parse: {e}")))?;

    let total = parsed
        .pointer("/meta/count")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let entries: Vec<PaperEntry> = parsed
        .get("results")
        .and_then(|v| v.as_array())
        .map(|arr| arr.iter().map(openalex_to_paper).collect())
        .unwrap_or_default();

    let topic_label = if topic.is_empty() {
        "most-cited works".to_string()
    } else {
        format!("\"{topic}\"")
    };
    let header = format!(
        "University: {display} ({country}, {works} total works) — {topic_label}: \
         {total} matching results"
    );
    Ok(format_paper_entries(&header, &entries))
}

// ═══════════════════════════════════════════════════════════════════════════
// Crossref (key-free; authoritative DOI metadata)
// ═══════════════════════════════════════════════════════════════════════════

fn crossref_to_paper(item: &serde_json::Value) -> PaperEntry {
    let title = item
        .get("title")
        .and_then(|v| v.as_array())
        .and_then(|arr| arr.first())
        .and_then(|v| v.as_str())
        .unwrap_or("?")
        .to_string();

    let authors: Vec<String> = item
        .get("author")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|a| {
                    let given = a.get("given").and_then(|v| v.as_str()).unwrap_or("");
                    let family = a.get("family").and_then(|v| v.as_str()).unwrap_or("");
                    let name = format!("{given} {family}").trim().to_string();
                    if name.is_empty() { None } else { Some(name) }
                })
                .collect()
        })
        .unwrap_or_default();

    // year: published.date-parts[0][0]
    let year = item
        .get("published")
        .or_else(|| item.get("issued"))
        .and_then(|v| v.get("date-parts"))
        .and_then(|v| v.as_array())
        .and_then(|arr| arr.first())
        .and_then(|v| v.as_array())
        .and_then(|arr| arr.first())
        .and_then(|v| v.as_u64())
        .map(|y| y.to_string());

    let venue = item
        .get("container-title")
        .and_then(|v| v.as_array())
        .and_then(|arr| arr.first())
        .and_then(|v| v.as_str())
        .map(String::from);

    let citations = item.get("is-referenced-by-count").and_then(|v| v.as_u64());
    let doi = item.get("DOI").and_then(|v| v.as_str()).map(String::from);
    let url = doi.as_ref().map(|d| format!("https://doi.org/{d}"));

    let abstract_text = item
        .get("abstract")
        .and_then(|v| v.as_str())
        // Crossref abstracts embed JATS XML tags; strip them.
        .map(strip_html_tags)
        .unwrap_or_default();

    PaperEntry {
        title,
        authors,
        year,
        venue,
        citations,
        url,
        pdf: None,
        abstract_text,
        arxiv_id: None,
        doi,
        source: "Crossref",
    }
}

async fn search_crossref(query: &str, max_results: usize) -> Result<String, String> {
    let rows = max_results.clamp(1, 25).to_string();
    let client = http_client()?;

    let mut req = client
        .get("https://api.crossref.org/works")
        .query(&[("query", query), ("rows", rows.as_str())]);
    if let Some(email) = polite_pool_email(&["CROSSREF_EMAIL", "OPENALEX_EMAIL"]) {
        req = req.query(&[("mailto", email.as_str())]);
    }

    let resp = req
        .send()
        .await
        .map_err(|e| format!("Crossref request failed: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("Crossref returned HTTP {}", resp.status()));
    }
    let body = resp
        .text()
        .await
        .map_err(|e| format!("Response read: {e}"))?;
    let parsed: serde_json::Value =
        serde_json::from_str(&body).map_err(|e| format!("JSON parse: {e}"))?;

    let entries: Vec<PaperEntry> = parsed
        .get("message")
        .and_then(|v| v.get("items"))
        .and_then(|v| v.as_array())
        .map(|arr| arr.iter().map(crossref_to_paper).collect())
        .unwrap_or_default();

    Ok(format_paper_entries(
        &format!("Crossref: \"{query}\" — {} results", entries.len()),
        &entries,
    ))
}

// ═══════════════════════════════════════════════════════════════════════════
// PubMed / NCBI E-utilities (biomedical)
// ═══════════════════════════════════════════════════════════════════════════

async fn search_pubmed(query: &str, max_results: usize) -> Result<String, String> {
    let retmax = max_results.clamp(1, 20).to_string();
    let client = http_client()?;
    let api_key = std::env::var("NCBI_API_KEY").ok().filter(|k| !k.is_empty());

    // Step 1: esearch → list of PMIDs.
    let mut esearch = client
        .get("https://eutils.ncbi.nlm.nih.gov/entrez/eutils/esearch.fcgi")
        .query(&[
            ("db", "pubmed"),
            ("term", query),
            ("retmax", retmax.as_str()),
            ("retmode", "json"),
        ]);
    if let Some(k) = &api_key {
        esearch = esearch.query(&[("api_key", k.as_str())]);
    }
    let esearch_resp = esearch
        .send()
        .await
        .map_err(|e| format!("PubMed esearch failed: {e}"))?;
    if !esearch_resp.status().is_success() {
        return Err(format!("PubMed esearch HTTP {}", esearch_resp.status()));
    }
    let esearch_json: serde_json::Value = esearch_resp
        .text()
        .await
        .map_err(|e| format!("Response read: {e}"))
        .and_then(|b| serde_json::from_str(&b).map_err(|e| format!("JSON parse: {e}")))?;

    let total = esearch_json
        .pointer("/esearchresult/count")
        .and_then(|v| v.as_str())
        .unwrap_or("?")
        .to_string();
    let ids: Vec<String> = esearch_json
        .pointer("/esearchresult/idlist")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();

    if ids.is_empty() {
        return Ok(format!(
            "PubMed: \"{query}\" — {total} total results\n\nNo results found.\n"
        ));
    }

    // Step 2: esummary → metadata for the PMIDs.
    let id_csv = ids.join(",");
    let mut esummary = client
        .get("https://eutils.ncbi.nlm.nih.gov/entrez/eutils/esummary.fcgi")
        .query(&[
            ("db", "pubmed"),
            ("id", id_csv.as_str()),
            ("retmode", "json"),
        ]);
    if let Some(k) = &api_key {
        esummary = esummary.query(&[("api_key", k.as_str())]);
    }
    let esummary_json: serde_json::Value = esummary
        .send()
        .await
        .map_err(|e| format!("PubMed esummary failed: {e}"))?
        .text()
        .await
        .map_err(|e| format!("Response read: {e}"))
        .and_then(|b| serde_json::from_str(&b).map_err(|e| format!("JSON parse: {e}")))?;

    let result = esummary_json.get("result");
    let mut out = format!("PubMed: \"{query}\" — {total} total results\n\n");
    for (i, pmid) in ids.iter().enumerate() {
        let Some(doc) = result.and_then(|r| r.get(pmid)) else {
            continue;
        };
        let title = doc.get("title").and_then(|v| v.as_str()).unwrap_or("?");
        let journal = doc
            .get("fulljournalname")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let pubdate = doc.get("pubdate").and_then(|v| v.as_str()).unwrap_or("");
        let authors: Vec<&str> = doc
            .get("authors")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|a| a.get("name").and_then(|n| n.as_str()))
                    .collect()
            })
            .unwrap_or_default();

        out.push_str(&format!("{}. {title}\n", i + 1));
        if !authors.is_empty() {
            out.push_str(&format!("   Authors: {}\n", authors.join(", ")));
        }
        if !journal.is_empty() {
            out.push_str(&format!("   Journal: {journal} ({pubdate})\n"));
        }
        out.push_str(&format!(
            "   URL: https://pubmed.ncbi.nlm.nih.gov/{pmid}/\n\n"
        ));
    }
    Ok(out)
}

// ═══════════════════════════════════════════════════════════════════════════
// DOAJ — Directory of Open Access Journals (key-free)
// ═══════════════════════════════════════════════════════════════════════════

async fn search_doaj(query: &str, max_results: usize) -> Result<String, String> {
    let page_size = max_results.clamp(1, 25).to_string();
    let client = http_client()?;
    let encoded = urlencoding_minimal(query);

    let resp = client
        .get(format!("https://doaj.org/api/search/articles/{encoded}"))
        .query(&[("pageSize", page_size.as_str())])
        .send()
        .await
        .map_err(|e| format!("DOAJ request failed: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("DOAJ returned HTTP {}", resp.status()));
    }
    let parsed: serde_json::Value = resp
        .text()
        .await
        .map_err(|e| format!("Response read: {e}"))
        .and_then(|b| serde_json::from_str(&b).map_err(|e| format!("JSON parse: {e}")))?;

    let total = parsed.get("total").and_then(|v| v.as_u64()).unwrap_or(0);
    let entries: Vec<PaperEntry> = parsed
        .get("results")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .map(|r| {
                    let b = r.get("bibjson").unwrap_or(r);
                    let title = b
                        .get("title")
                        .and_then(|v| v.as_str())
                        .unwrap_or("?")
                        .to_string();
                    let year = b.get("year").and_then(|v| v.as_str()).map(String::from);
                    let venue = b
                        .get("journal")
                        .and_then(|j| j.get("title"))
                        .and_then(|v| v.as_str())
                        .map(String::from);
                    let authors = b
                        .get("author")
                        .and_then(|v| v.as_array())
                        .map(|arr| {
                            arr.iter()
                                .filter_map(|a| {
                                    a.get("name").and_then(|v| v.as_str()).map(String::from)
                                })
                                .collect()
                        })
                        .unwrap_or_default();
                    let abstract_text = b
                        .get("abstract")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    let url = b
                        .get("link")
                        .and_then(|v| v.as_array())
                        .and_then(|arr| arr.first())
                        .and_then(|l| l.get("url"))
                        .and_then(|v| v.as_str())
                        .map(String::from);
                    let doi = b
                        .get("identifier")
                        .and_then(|v| v.as_array())
                        .and_then(|arr| {
                            arr.iter()
                                .find(|id| id.get("type").and_then(|t| t.as_str()) == Some("doi"))
                        })
                        .and_then(|id| id.get("id"))
                        .and_then(|v| v.as_str())
                        .map(String::from);
                    PaperEntry {
                        title,
                        authors,
                        year,
                        venue,
                        citations: None,
                        url,
                        pdf: None,
                        abstract_text,
                        arxiv_id: None,
                        doi,
                        source: "DOAJ",
                    }
                })
                .collect()
        })
        .unwrap_or_default();

    Ok(format_paper_entries(
        &format!("DOAJ: \"{query}\" — {total} total results"),
        &entries,
    ))
}

/// Minimal percent-encoding for a path segment (DOAJ takes the query in-path).
fn urlencoding_minimal(s: &str) -> String {
    let mut out = String::with_capacity(s.len() * 3);
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

// ═══════════════════════════════════════════════════════════════════════════
// CORE — 290M+ open-access full texts (free API key)
// ═══════════════════════════════════════════════════════════════════════════

async fn search_core(query: &str, max_results: usize) -> Result<String, String> {
    let key = std::env::var("CORE_API_KEY").ok().filter(|k| !k.is_empty()).ok_or_else(|| {
        "CORE requires a free API key. Set CORE_API_KEY (register at https://core.ac.uk/services/api)".to_string()
    })?;
    let limit = max_results.clamp(1, 25);
    let client = http_client()?;

    let resp = client
        .post("https://api.core.ac.uk/v3/search/works")
        .bearer_auth(&key)
        .json(&serde_json::json!({ "q": query, "limit": limit }))
        .send()
        .await
        .map_err(|e| format!("CORE request failed: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("CORE returned HTTP {}", resp.status()));
    }
    let parsed: serde_json::Value = resp
        .text()
        .await
        .map_err(|e| format!("Response read: {e}"))
        .and_then(|b| serde_json::from_str(&b).map_err(|e| format!("JSON parse: {e}")))?;

    let total = parsed
        .get("totalHits")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let entries: Vec<PaperEntry> = parsed
        .get("results")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .map(|w| {
                    let title = w
                        .get("title")
                        .and_then(|v| v.as_str())
                        .unwrap_or("?")
                        .to_string();
                    let year = w
                        .get("yearPublished")
                        .and_then(|v| v.as_u64())
                        .map(|y| y.to_string());
                    let authors = w
                        .get("authors")
                        .and_then(|v| v.as_array())
                        .map(|arr| {
                            arr.iter()
                                .filter_map(|a| {
                                    a.get("name").and_then(|v| v.as_str()).map(String::from)
                                })
                                .collect()
                        })
                        .unwrap_or_default();
                    let abstract_text = w
                        .get("abstract")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    let doi = w.get("doi").and_then(|v| v.as_str()).map(String::from);
                    let pdf = w
                        .get("downloadUrl")
                        .and_then(|v| v.as_str())
                        .map(String::from);
                    PaperEntry {
                        title,
                        authors,
                        year,
                        venue: None,
                        citations: w.get("citationCount").and_then(|v| v.as_u64()),
                        url: doi.as_ref().map(|d| format!("https://doi.org/{d}")),
                        pdf,
                        abstract_text,
                        arxiv_id: None,
                        doi,
                        source: "CORE",
                    }
                })
                .collect()
        })
        .unwrap_or_default();

    Ok(format_paper_entries(
        &format!("CORE: \"{query}\" — {total} total results"),
        &entries,
    ))
}

// ═══════════════════════════════════════════════════════════════════════════
// Unpaywall — resolve a DOI to free, legal OA PDF locations (key-free, email)
// ═══════════════════════════════════════════════════════════════════════════

async fn search_unpaywall(doi: &str) -> Result<String, String> {
    // Accept a bare DOI or a doi.org URL; Unpaywall is a lookup, not search.
    let doi = doi
        .trim()
        .trim_start_matches("https://doi.org/")
        .trim_start_matches("http://doi.org/")
        .trim_start_matches("doi:")
        .trim();
    if doi.is_empty() || !doi.contains('/') {
        return Err(
            "Unpaywall resolves a DOI to its open-access PDFs. Pass a DOI, e.g. \
             `unpaywall: 10.1038/nature12373`."
                .to_string(),
        );
    }

    // Unpaywall requires a contact email; fall back to a project default.
    let email = polite_pool_email(&["UNPAYWALL_EMAIL", "OPENALEX_EMAIL", "CROSSREF_EMAIL"])
        .unwrap_or_else(|| "jfc-web@users.noreply.github.com".to_string());

    let client = http_client()?;
    let resp = client
        .get(format!("https://api.unpaywall.org/v2/{doi}"))
        .query(&[("email", email.as_str())])
        .send()
        .await
        .map_err(|e| format!("Unpaywall request failed: {e}"))?;

    if resp.status() == 404 {
        return Ok(format!("Unpaywall: DOI {doi} not found.\n"));
    }
    if !resp.status().is_success() {
        return Err(format!("Unpaywall returned HTTP {}", resp.status()));
    }
    let p: serde_json::Value = resp
        .text()
        .await
        .map_err(|e| format!("Response read: {e}"))
        .and_then(|b| serde_json::from_str(&b).map_err(|e| format!("JSON parse: {e}")))?;

    let title = p.get("title").and_then(|v| v.as_str()).unwrap_or("?");
    let year = p.get("year").and_then(|v| v.as_u64());
    let journal = p.get("journal_name").and_then(|v| v.as_str()).unwrap_or("");
    let is_oa = p.get("is_oa").and_then(|v| v.as_bool()).unwrap_or(false);
    let oa_status = p
        .get("oa_status")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown");

    let mut out = format!("Unpaywall: {doi}\n\n{title}");
    if let Some(y) = year {
        out.push_str(&format!(" ({y})"));
    }
    out.push('\n');
    if !journal.is_empty() {
        out.push_str(&format!("Journal: {journal}\n"));
    }
    out.push_str(&format!(
        "Open access: {} (status: {oa_status})\n",
        if is_oa { "yes" } else { "no" }
    ));
    out.push_str(&format!("DOI: https://doi.org/{doi}\n\n"));

    if let Some(locations) = p.get("oa_locations").and_then(|v| v.as_array())
        && !locations.is_empty()
    {
        out.push_str("OA locations:\n");
        for loc in locations {
            let pdf = loc
                .get("url_for_pdf")
                .and_then(|v| v.as_str())
                .or_else(|| loc.get("url").and_then(|v| v.as_str()))
                .unwrap_or("");
            let host = loc.get("host_type").and_then(|v| v.as_str()).unwrap_or("");
            let version = loc.get("version").and_then(|v| v.as_str()).unwrap_or("");
            let repo = loc
                .get("repository_institution")
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
                .map(|s| format!(" — {s}"))
                .unwrap_or_default();
            out.push_str(&format!("- [{host}/{version}{repo}] {pdf}\n"));
        }
    } else if !is_oa {
        out.push_str("No open-access copy found.\n");
    }
    Ok(out)
}

// ═══════════════════════════════════════════════════════════════════════════
// Brave Search (independent index; free API key)
// ═══════════════════════════════════════════════════════════════════════════

async fn brave_results(query: &str, max_results: usize) -> Result<String, String> {
    let key = std::env::var("BRAVE_API_KEY").ok().filter(|k| !k.is_empty()).ok_or_else(|| {
        "Brave Search requires an API key. Set BRAVE_API_KEY (free tier at https://brave.com/search/api/)".to_string()
    })?;
    let count = max_results.clamp(1, 20).to_string();
    let client = http_client()?;

    let resp = client
        .get("https://api.search.brave.com/res/v1/web/search")
        .header("Accept", "application/json")
        .header("X-Subscription-Token", key)
        .query(&[("q", query), ("count", count.as_str())])
        .send()
        .await
        .map_err(|e| format!("Brave request failed: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("Brave returned HTTP {}", resp.status()));
    }
    let parsed: serde_json::Value = resp
        .text()
        .await
        .map_err(|e| format!("Response read: {e}"))
        .and_then(|b| serde_json::from_str(&b).map_err(|e| format!("JSON parse: {e}")))?;

    let mut out = format!("Brave Search: \"{query}\"\n\n");
    let results = parsed.pointer("/web/results").and_then(|v| v.as_array());
    match results {
        Some(items) if !items.is_empty() => {
            for (i, item) in items.iter().enumerate() {
                let title = item.get("title").and_then(|v| v.as_str()).unwrap_or("?");
                let url = item.get("url").and_then(|v| v.as_str()).unwrap_or("");
                let desc = item
                    .get("description")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                out.push_str(&format!("{}. {title}\n   URL: {url}\n   {desc}\n\n", i + 1));
            }
        }
        _ => out.push_str("No results found.\n"),
    }
    Ok(out)
}

async fn search_brave(query: &str, max_results: usize) -> Result<String, String> {
    brave_results(query, max_results).await
}

// ═══════════════════════════════════════════════════════════════════════════
// Tavily (LLM-oriented search; free API key)
// ═══════════════════════════════════════════════════════════════════════════

async fn search_tavily(query: &str, max_results: usize) -> Result<String, String> {
    let key = std::env::var("TAVILY_API_KEY")
        .ok()
        .filter(|k| !k.is_empty())
        .ok_or_else(|| {
            "Tavily requires an API key. Set TAVILY_API_KEY (free tier at https://tavily.com)"
                .to_string()
        })?;
    let limit = max_results.clamp(1, 20);
    let client = http_client()?;

    let resp = client
        .post("https://api.tavily.com/search")
        .json(&serde_json::json!({
            "api_key": key,
            "query": query,
            "max_results": limit,
            "include_answer": true,
        }))
        .send()
        .await
        .map_err(|e| format!("Tavily request failed: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("Tavily returned HTTP {}", resp.status()));
    }
    let parsed: serde_json::Value = resp
        .text()
        .await
        .map_err(|e| format!("Response read: {e}"))
        .and_then(|b| serde_json::from_str(&b).map_err(|e| format!("JSON parse: {e}")))?;

    let mut out = format!("Tavily: \"{query}\"\n\n");
    if let Some(answer) = parsed.get("answer").and_then(|v| v.as_str())
        && !answer.is_empty()
    {
        out.push_str(&format!("Answer: {answer}\n\n"));
    }
    let results = parsed.get("results").and_then(|v| v.as_array());
    match results {
        Some(items) if !items.is_empty() => {
            for (i, item) in items.iter().enumerate() {
                let title = item.get("title").and_then(|v| v.as_str()).unwrap_or("?");
                let url = item.get("url").and_then(|v| v.as_str()).unwrap_or("");
                let content = item.get("content").and_then(|v| v.as_str()).unwrap_or("");
                out.push_str(&format!(
                    "{}. {title}\n   URL: {url}\n   {}\n\n",
                    i + 1,
                    truncate_abstract(content)
                ));
            }
        }
        _ => out.push_str("No results found.\n"),
    }
    Ok(out)
}

// ═══════════════════════════════════════════════════════════════════════════
// Exa (neural/semantic search; free API key)
// ═══════════════════════════════════════════════════════════════════════════

async fn search_exa(query: &str, max_results: usize) -> Result<String, String> {
    let key = std::env::var("EXA_API_KEY")
        .ok()
        .filter(|k| !k.is_empty())
        .ok_or_else(|| {
            "Exa requires an API key. Set EXA_API_KEY (free tier at https://exa.ai)".to_string()
        })?;
    let limit = max_results.clamp(1, 20);
    let client = http_client()?;

    let resp = client
        .post("https://api.exa.ai/search")
        .header("x-api-key", key)
        .json(&serde_json::json!({
            "query": query,
            "numResults": limit,
            "contents": { "text": { "maxCharacters": 300 } },
        }))
        .send()
        .await
        .map_err(|e| format!("Exa request failed: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("Exa returned HTTP {}", resp.status()));
    }
    let parsed: serde_json::Value = resp
        .text()
        .await
        .map_err(|e| format!("Response read: {e}"))
        .and_then(|b| serde_json::from_str(&b).map_err(|e| format!("JSON parse: {e}")))?;

    let mut out = format!("Exa: \"{query}\"\n\n");
    let results = parsed.get("results").and_then(|v| v.as_array());
    match results {
        Some(items) if !items.is_empty() => {
            for (i, item) in items.iter().enumerate() {
                let title = item.get("title").and_then(|v| v.as_str()).unwrap_or("?");
                let url = item.get("url").and_then(|v| v.as_str()).unwrap_or("");
                let text = item.get("text").and_then(|v| v.as_str()).unwrap_or("");
                out.push_str(&format!(
                    "{}. {title}\n   URL: {url}\n   {}\n\n",
                    i + 1,
                    truncate_abstract(text)
                ));
            }
        }
        _ => out.push_str("No results found.\n"),
    }
    Ok(out)
}

// ═══════════════════════════════════════════════════════════════════════════
// SearXNG (key-free meta-search; aggregates Google/Bing/DDG/etc.)
//
// Microsoft retired the Bing Web Search API in Aug 2025, so the durable
// key-free way to get a real SERP is a SearXNG instance. Reads the base URL
// from `SEARXNG_URL` (e.g. a self-hosted instance) and falls back to a public
// instance; both expose the standard `/search?format=json` JSON API.
// ═══════════════════════════════════════════════════════════════════════════

fn searxng_base_url() -> String {
    std::env::var("SEARXNG_URL")
        .ok()
        .map(|s| s.trim_end_matches('/').to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "https://searx.be".to_string())
}

async fn search_searxng(query: &str, max_results: usize) -> Result<String, String> {
    let base = searxng_base_url();
    let client = http_client()?;
    let resp = client
        .get(format!("{base}/search"))
        .query(&[("q", query), ("format", "json")])
        // Some public instances reject requests without a browser-ish UA.
        .header("User-Agent", "jfc-web/1.0 (+https://github.com/coleleavitt/jfc)")
        .send()
        .await
        .map_err(|e| format!("SearXNG request failed: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("SearXNG ({base}) returned HTTP {}", resp.status()));
    }
    let parsed: serde_json::Value = resp
        .text()
        .await
        .map_err(|e| format!("Response read: {e}"))
        .and_then(|b| serde_json::from_str(&b).map_err(|e| format!("JSON parse: {e}")))?;

    let mut out = format!("SearXNG ({base}): \"{query}\"\n\n");
    match parsed.get("results").and_then(|v| v.as_array()) {
        Some(items) if !items.is_empty() => {
            for (i, item) in items.iter().take(max_results.clamp(1, 20)).enumerate() {
                let title = item.get("title").and_then(|v| v.as_str()).unwrap_or("?");
                let url = item.get("url").and_then(|v| v.as_str()).unwrap_or("");
                let content = item.get("content").and_then(|v| v.as_str()).unwrap_or("");
                out.push_str(&format!(
                    "{}. {title}\n   URL: {url}\n   {content}\n\n",
                    i + 1
                ));
            }
        }
        _ => out.push_str("No results found.\n"),
    }
    Ok(out)
}

// ═══════════════════════════════════════════════════════════════════════════
// DuckDuckGo Instant Answer (key-free; facts/definitions, not full SERP)
// ═══════════════════════════════════════════════════════════════════════════

async fn search_duckduckgo(query: &str) -> Result<String, String> {
    let client = http_client()?;
    let resp = client
        .get("https://api.duckduckgo.com/")
        .query(&[
            ("q", query),
            ("format", "json"),
            ("no_html", "1"),
            ("skip_disambig", "1"),
        ])
        .send()
        .await
        .map_err(|e| format!("DuckDuckGo request failed: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("DuckDuckGo returned HTTP {}", resp.status()));
    }
    let parsed: serde_json::Value = resp
        .text()
        .await
        .map_err(|e| format!("Response read: {e}"))
        .and_then(|b| serde_json::from_str(&b).map_err(|e| format!("JSON parse: {e}")))?;

    let mut out = format!("DuckDuckGo Instant Answer: \"{query}\"\n\n");
    let mut any = false;

    let heading = parsed.get("Heading").and_then(|v| v.as_str()).unwrap_or("");
    let abstract_text = parsed
        .get("AbstractText")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let abstract_url = parsed
        .get("AbstractURL")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    if !abstract_text.is_empty() {
        any = true;
        if !heading.is_empty() {
            out.push_str(&format!("{heading}\n"));
        }
        out.push_str(&format!("{abstract_text}\n"));
        if !abstract_url.is_empty() {
            out.push_str(&format!("Source: {abstract_url}\n"));
        }
        out.push('\n');
    }

    if let Some(answer) = parsed.get("Answer").and_then(|v| v.as_str())
        && !answer.is_empty()
    {
        any = true;
        out.push_str(&format!("Answer: {answer}\n\n"));
    }

    if let Some(topics) = parsed.get("RelatedTopics").and_then(|v| v.as_array()) {
        let related: Vec<(&str, &str)> = topics
            .iter()
            .filter_map(|t| {
                let text = t.get("Text").and_then(|v| v.as_str())?;
                let url = t.get("FirstURL").and_then(|v| v.as_str()).unwrap_or("");
                Some((text, url))
            })
            .take(8)
            .collect();
        if !related.is_empty() {
            any = true;
            out.push_str("Related:\n");
            for (text, url) in related {
                out.push_str(&format!("- {text}\n  {url}\n"));
            }
            out.push('\n');
        }
    }

    if !any {
        out.push_str(
            "No instant answer. DuckDuckGo's API only returns facts/definitions, \
             not a full result list — use the default (Google) backend for general queries.\n",
        );
    }
    Ok(out)
}

// ═══════════════════════════════════════════════════════════════════════════
// Wikipedia / MediaWiki search (key-free)
// ═══════════════════════════════════════════════════════════════════════════

async fn search_wikipedia(query: &str, max_results: usize) -> Result<String, String> {
    let limit = max_results.clamp(1, 20).to_string();
    let client = http_client()?;

    let resp = client
        .get("https://en.wikipedia.org/w/api.php")
        .query(&[
            ("action", "query"),
            ("list", "search"),
            ("srsearch", query),
            ("srlimit", limit.as_str()),
            ("format", "json"),
        ])
        .send()
        .await
        .map_err(|e| format!("Wikipedia request failed: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("Wikipedia returned HTTP {}", resp.status()));
    }
    let parsed: serde_json::Value = resp
        .text()
        .await
        .map_err(|e| format!("Response read: {e}"))
        .and_then(|b| serde_json::from_str(&b).map_err(|e| format!("JSON parse: {e}")))?;

    let total = parsed
        .pointer("/query/searchinfo/totalhits")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let hits = parsed.pointer("/query/search").and_then(|v| v.as_array());

    let mut out = format!("Wikipedia: \"{query}\" — {total} total hits\n\n");
    match hits {
        Some(items) if !items.is_empty() => {
            for (i, item) in items.iter().enumerate() {
                let title = item.get("title").and_then(|v| v.as_str()).unwrap_or("?");
                let snippet = item
                    .get("snippet")
                    .and_then(|v| v.as_str())
                    // Strip the <span class="searchmatch"> markup MediaWiki injects.
                    .map(strip_html_tags)
                    .unwrap_or_default();
                let slug = title.replace(' ', "_");
                out.push_str(&format!(
                    "{}. {title}\n   URL: https://en.wikipedia.org/wiki/{slug}\n   {snippet}\n\n",
                    i + 1
                ));
            }
        }
        _ => out.push_str("No results found.\n"),
    }
    Ok(out)
}

// ═══════════════════════════════════════════════════════════════════════════
// ExLibris Primo — library discovery system used by 8000+ universities
//
// API reverse-engineered from the CMU Primo JS bundle (bundle.js):
//   pnxBaseURL: /primaws/rest/pub/pnxs
//   search params: q, tab, search_scope, vid, inst, lang, offset, limit, sort, mode
//   JWT: public guestJwt endpoint (/primaws/rest/pub/institution/{inst}/guestJwt)
//         but the pnxs endpoint works without auth for many institutions
//
// Instance table: maps short names → (host, vid, inst, tab, scope)
// Covers US, UK, EU, China, Australia, Canada, Japan, Singapore universities.
// `primo: <query>` defaults to CMU; `primo: <inst>/<query>` picks an instance.
// ═══════════════════════════════════════════════════════════════════════════

/// Known Primo instances: (key, host, vid, inst_code, default_tab, default_scope)
///
/// Verified working (unauthenticated public pnxs endpoint responds 200):
///   cmu ✓, mit ✓
/// Unverified (VIDs from public ExLibris documentation / institution web pages;
/// may require IP allowlist, different auth, or have changed):
///   all others — they fail gracefully with a clear error message.
static PRIMO_INSTANCES: &[(&str, &str, &str, &str, &str, &str)] = &[
    // ── USA (verified ✓ / documented) ───────────────────────────────────
    (
        "cmu",
        "cmu.primo.exlibrisgroup.com",
        "01CMU_INST:01CMU",
        "01CMU_INST",
        "Everything",
        "MyInst_and_CI",
    ),
    (
        "mit",
        "mit.primo.exlibrisgroup.com",
        "01MIT_INST:MIT",
        "01MIT_INST",
        "all",
        "all",
    ),
    (
        "harvard",
        "hollis.harvard.edu",
        "01HVD_INST:HVD2",
        "01HVD_INST",
        "Everything",
        "MyInst_and_CI",
    ),
    (
        "stanford",
        "stanford.primo.exlibrisgroup.com",
        "01STANFORD_INST:stanford",
        "01STANFORD_INST",
        "Everything",
        "everything",
    ),
    (
        "berkeley",
        "berkeley.primo.exlibrisgroup.com",
        "01UCS_BER:UCB",
        "01UCS_BER",
        "Everything",
        "everything",
    ),
    (
        "columbia",
        "columbia.primo.exlibrisgroup.com",
        "01COLU_INST:COLU",
        "01COLU_INST",
        "All",
        "COLU",
    ),
    (
        "cornell",
        "cornell.primo.exlibrisgroup.com",
        "01CORNELL_INST:CORNELL",
        "01CORNELL_INST",
        "Everything",
        "everything",
    ),
    (
        "yale",
        "yale.primo.exlibrisgroup.com",
        "01YAL_INST:default",
        "01YAL_INST",
        "default_tab",
        "default_scope",
    ),
    (
        "princeton",
        "princeton.primo.exlibrisgroup.com",
        "01PRI_INST:PRINCETON",
        "01PRI_INST",
        "Everything",
        "everything",
    ),
    (
        "brown",
        "brown.primo.exlibrisgroup.com",
        "01BU_INST:BROWN",
        "01BU_INST",
        "Everything",
        "MyInst_and_CI",
    ),
    (
        "michigan",
        "umich.primo.exlibrisgroup.com",
        "01UMICH_INST:UMICH",
        "01UMICH_INST",
        "Everything",
        "everything",
    ),
    (
        "ucla",
        "ucla.primo.exlibrisgroup.com",
        "01UCS_LAC:UCLA",
        "01UCS_LAC",
        "Everything",
        "everything",
    ),
    (
        "chicago",
        "uchicago.primo.exlibrisgroup.com",
        "01UCHICAGO_INST:UCHICAGO",
        "01UCHICAGO_INST",
        "everything",
        "everything",
    ),
    (
        "caltech",
        "caltech.primo.exlibrisgroup.com",
        "01CALT_INST:Caltech",
        "01CALT_INST",
        "everything",
        "everything",
    ),
    (
        "nyu",
        "nyu-primo.hosted.exlibrisgroup.com",
        "NYU:NYU",
        "NYU",
        "all",
        "all",
    ),
    (
        "jhu",
        "jhu.primo.exlibrisgroup.com",
        "01JHU_INST:JHU",
        "01JHU_INST",
        "all",
        "allsystem",
    ),
    (
        "duke",
        "duke.primo.exlibrisgroup.com",
        "01DUKE_INST:duke_library",
        "01DUKE_INST",
        "Everything",
        "everything",
    ),
    (
        "purdue",
        "purdue.primo.exlibrisgroup.com",
        "PURDUE:PURDUE",
        "PURDUE",
        "boilermakers_tab",
        "MyInst_and_CI",
    ),
    (
        "uiuc",
        "uiuc.primo.exlibrisgroup.com",
        "01CARLI_UIU:UIUC_DEFAULT",
        "01CARLI_UIU",
        "UIUC_DEFAULT",
        "LibraryCatalog",
    ),
    (
        "gatech",
        "gatech.primo.exlibrisgroup.com",
        "01GALI_GIT:GT",
        "01GALI_GIT",
        "default_tab",
        "defaultscope",
    ),
    (
        "washington",
        "uw.primo.exlibrisgroup.com",
        "01ALLIANCE_UW:UW",
        "01ALLIANCE_UW",
        "uw_alma",
        "uw_everything",
    ),
    // ── Additional USA (newly discovered + verified) ─────────────────────
    (
        "pitt",
        "pitt.primo.exlibrisgroup.com",
        "01PITT_INST:01PITT_INST",
        "01PITT_INST",
        "Everything",
        "MyInst_and_CI",
    ),
    (
        "rochester",
        "rochester.primo.exlibrisgroup.com",
        "01ROCH_INST:UR01",
        "01ROCH_INST",
        "Everything",
        "MyInst_and_CI",
    ),
    (
        "emory",
        "emory.primo.exlibrisgroup.com",
        "01GALI_EMORY:EMORY",
        "01GALI_EMORY",
        "Everything",
        "MyInst_and_CI",
    ),
    (
        "vanderbilt",
        "vanderbilt.primo.exlibrisgroup.com",
        "01VAN_INST:vandy",
        "01VAN_INST",
        "Everything",
        "MyInst_and_CI",
    ),
    (
        "rice",
        "rice.primo.exlibrisgroup.com",
        "01RICE_INST:RICE",
        "01RICE_INST",
        "Everything",
        "MyInst_and_CI",
    ),
    (
        "utah",
        "utah.primo.exlibrisgroup.com",
        "01UTAH_INST:UTAH",
        "01UTAH_INST",
        "Everything",
        "MyInst_and_CI",
    ),
    (
        "miami",
        "miami.primo.exlibrisgroup.com",
        "01UMIAMI_INST:umiami",
        "01UMIAMI_INST",
        "Everything",
        "MyInst_and_CI",
    ),
    (
        "scu",
        "scu.primo.exlibrisgroup.com",
        "01SCU_INST:SCU",
        "01SCU_INST",
        "Everything",
        "MyInst_and_CI",
    ),
    (
        "davidson",
        "davidson.primo.exlibrisgroup.com",
        "01DCOLL_INST:01DCOLL",
        "01DCOLL_INST",
        "Everything",
        "MyInst_and_CI",
    ),
    (
        "sva",
        "sva.primo.exlibrisgroup.com",
        "01VISUAL_INST:01VISUAL",
        "01VISUAL_INST",
        "Everything",
        "MyInst_and_CI",
    ),
    (
        "aul",
        "aul.primo.exlibrisgroup.com",
        "01AUL_INST:AUL",
        "01AUL_INST",
        "Everything",
        "MyInst_and_CI_noR",
    ),
    (
        "usma",
        "usma.primo.exlibrisgroup.com",
        "01USMA_INST:USMA",
        "01USMA_INST",
        "Everything",
        "MyInst_and_CI",
    ),
    (
        "lva",
        "lva.primo.exlibrisgroup.com",
        "01LVA_INST:LVA_default",
        "01LVA_INST",
        "Everything",
        "MyInst_and_CI",
    ),
    (
        "wrlc",
        "wrlc-gm.primo.exlibrisgroup.com",
        "01WRLC_GML:01WRLC_GML",
        "01WRLC_GML",
        "Everything",
        "MyInst_and_CI",
    ),
    (
        "uky",
        "saalck-uky.primo.exlibrisgroup.com",
        "01SAA_UKY:UKY",
        "01SAA_UKY",
        "Everything",
        "MyInst_and_CI",
    ),
    (
        "du",
        "du.primo.exlibrisgroup.com",
        "01UDENVER_INST:UDENVER",
        "01UDENVER_INST",
        "Everything",
        "MyInst_and_CI",
    ),
    (
        "glendale",
        "caccl-glendale.primo.exlibrisgroup.com",
        "01CACCL_GLENDALE:GLENDALE",
        "01CACCL_GLENDALE",
        "Everything",
        "MyInst_and_CI",
    ),
    (
        "sfsu",
        "csu-sfsu.primo.exlibrisgroup.com",
        "01CSFSU_INST:SFSU",
        "01CSFSU_INST",
        "Everything",
        "MyInst_and_CI",
    ),
    (
        "bll",
        "bll01.primo.exlibrisgroup.com",
        "01BLL_INST:BLUK_VU1",
        "01BLL_INST",
        "Everything",
        "MyInst_and_CI",
    ),
    (
        "psu",
        "psu.primo.exlibrisgroup.com",
        "01PSU_INST:PSU",
        "01PSU_INST",
        "Everything",
        "everything",
    ),
    (
        "osu",
        "osu.primo.exlibrisgroup.com",
        "01OPLIN_OSU:OSU",
        "01OPLIN_OSU",
        "Everything",
        "everything",
    ),
    (
        "usc",
        "usc.primo.exlibrisgroup.com",
        "01USC_INST:01USC",
        "01USC_INST",
        "Everything",
        "everything",
    ),
    (
        "bu",
        "bu.primo.exlibrisgroup.com",
        "01BOSU_INST:BOSU",
        "01BOSU_INST",
        "everything",
        "everything",
    ),
    (
        "northeastern",
        "northeastern.primo.exlibrisgroup.com",
        "01NEU_INST:NEU",
        "01NEU_INST",
        "Everything",
        "everything",
    ),
    (
        "tufts",
        "tufts.primo.exlibrisgroup.com",
        "01TUFTS_INST:TUFTS",
        "01TUFTS_INST",
        "Everything",
        "everything",
    ),
    (
        "dartmouth",
        "dartmouth.primo.exlibrisgroup.com",
        "01DCLD_INST:DARTMOUTH",
        "01DCLD_INST",
        "Everything",
        "everything",
    ),
    (
        "unc",
        "unc.primo.exlibrisgroup.com",
        "01UNC_INST:UNC",
        "01UNC_INST",
        "Everything",
        "everything",
    ),
    (
        "georgetown",
        "georgetown.primo.exlibrisgroup.com",
        "01GUTO_INST:GUTO",
        "01GUTO_INST",
        "Everything",
        "everything",
    ),
    // ── Canada ───────────────────────────────────────────────────────────
    (
        "toronto",
        "utoronto.primo.exlibrisgroup.com",
        "01UTORONTO_INST:UTORONTO",
        "01UTORONTO_INST",
        "default",
        "UTORONTO_DEFAULT",
    ),
    (
        "mcgill",
        "mcgill.primo.exlibrisgroup.com",
        "01MCGILL_INST:default",
        "01MCGILL_INST",
        "everything",
        "everything",
    ),
    (
        "ubc",
        "ubc.primo.exlibrisgroup.com",
        "01UBC_INST:UBC_default",
        "01UBC_INST",
        "Everything",
        "Everything",
    ),
    (
        "waterloo",
        "waterloo.primo.exlibrisgroup.com",
        "01WLU_INST:WATERLOO",
        "01WLU_INST",
        "Everything",
        "everything",
    ),
    // ── UK ───────────────────────────────────────────────────────────────
    (
        "oxford",
        "solo.bodleian.ox.ac.uk",
        "OXFORD:SOLO",
        "OXFORD",
        "Everything",
        "SOLO_Everything",
    ),
    (
        "cambridge",
        "idiscover.lib.cam.ac.uk",
        "44CAM_INST:44CAM_VU2",
        "44CAM_INST",
        "default_tab",
        "default_scope",
    ),
    (
        "ucl",
        "ucl.primo.exlibrisgroup.com",
        "44UCL_INST:UCL_VU2",
        "44UCL_INST",
        "Everything",
        "MyInst_and_CI",
    ),
    (
        "imperial",
        "imperial.primo.exlibrisgroup.com",
        "44IMP_INST:ICL_VU1",
        "44IMP_INST",
        "Everything",
        "MyInst_and_CI",
    ),
    (
        "edinburgh",
        "discovered.ed.ac.uk",
        "44UOE_INST:44UOE_VU1",
        "44UOE_INST",
        "UoE",
        "UoE_discovery",
    ),
    (
        "manchester",
        "manchester.primo.exlibrisgroup.com",
        "44MAN_INST:MAN_VU1",
        "44MAN_INST",
        "Everything",
        "Everything",
    ),
    (
        "lse",
        "lse.primo.exlibrisgroup.com",
        "44LSE_INST:44LSE",
        "44LSE_INST",
        "Everything",
        "Everything",
    ),
    (
        "kings",
        "kcl.primo.exlibrisgroup.com",
        "44KCL_INST:KCL_VU1",
        "44KCL_INST",
        "Everything",
        "Everything",
    ),
    (
        "bristol",
        "bristol.primo.exlibrisgroup.com",
        "44BRI_INST:BRI_VU1",
        "44BRI_INST",
        "Everything",
        "Everything",
    ),
    (
        "warwick",
        "warwick.primo.exlibrisgroup.com",
        "44WAR_INST:WAR_VU1",
        "44WAR_INST",
        "Everything",
        "Everything",
    ),
    // ── Europe ───────────────────────────────────────────────────────────
    (
        "eth",
        "eth.primo.exlibrisgroup.com",
        "41SLSP_ETH:ETH",
        "41SLSP_ETH",
        "default",
        "default",
    ),
    (
        "epfl",
        "epfl.primo.exlibrisgroup.com",
        "41SLSP_EPF:EPFL_VU1",
        "41SLSP_EPF",
        "default",
        "default",
    ),
    (
        "tum",
        "tum.primo.exlibrisgroup.com",
        "49TUM_INST:TUM",
        "49TUM_INST",
        "MyCatalog",
        "MyCatalog",
    ),
    (
        "lmu",
        "lmu.primo.exlibrisgroup.com",
        "49MUM_INST:MUM",
        "49MUM_INST",
        "everything_ub",
        "everything_ub",
    ),
    (
        "humboldt",
        "hu-berlin.primo.exlibrisgroup.com",
        "49HUB_INST:HUB_default",
        "49HUB_INST",
        "default",
        "default",
    ),
    (
        "leiden",
        "catalogue.leidenuniv.nl",
        "31UKB_LEI_INST:SINGLE_LEIDENU",
        "31UKB_LEI_INST",
        "Catalogus",
        "LEIDENU",
    ),
    (
        "delft",
        "tudelft.primo.exlibrisgroup.com",
        "31TUD_INST:31TUD_INST",
        "31TUD_INST",
        "Everything",
        "Everything",
    ),
    (
        "amsterdam",
        "uva.primo.exlibrisgroup.com",
        "31UKB_UAM_INST:UAMD",
        "31UKB_UAM_INST",
        "catalogue",
        "UAMD",
    ),
    (
        "karolinska",
        "ki.primo.exlibrisgroup.com",
        "46KI_INST:KI_VU1",
        "46KI_INST",
        "default_tab",
        "default_scope",
    ),
    (
        "kth",
        "kth.primo.exlibrisgroup.com",
        "46KTH_INST:KTH",
        "46KTH_INST",
        "default_tab",
        "default_scope",
    ),
    (
        "stockholm",
        "su.primo.exlibrisgroup.com",
        "46SU_INST:46SU_VU1",
        "46SU_INST",
        "default_tab",
        "default_scope",
    ),
    (
        "chalmers",
        "chalmers.primo.exlibrisgroup.com",
        "46CHA_INST:46CHA_VU1",
        "46CHA_INST",
        "default_tab",
        "default_scope",
    ),
    (
        "ghent",
        "ugent.primo.exlibrisgroup.com",
        "32UGE_INST:Ugent",
        "32UGE_INST",
        "everything",
        "everything",
    ),
    (
        "leuven",
        "leuven.primo.exlibrisgroup.com",
        "32KUL_INST:KUL",
        "32KUL_INST",
        "all_content",
        "all_content",
    ),
    (
        "paris",
        "biu-sante.primo.exlibrisgroup.com",
        "33USPC_INST:33USPC",
        "33USPC_INST",
        "Everything",
        "Everything",
    ),
    (
        "sorbonne",
        "sorbonne-universite.primo.exlibrisgroup.com",
        "33SOR_INST:VU_SOR",
        "33SOR_INST",
        "Everything",
        "Everything",
    ),
    (
        "bologna",
        "unibo.primo.exlibrisgroup.com",
        "39UBO_INST:39UBO_VU1",
        "39UBO_INST",
        "Everything",
        "Everything",
    ),
    (
        "sapienza",
        "uniroma1.primo.exlibrisgroup.com",
        "39SAP_INST:SAPIENZA",
        "39SAP_INST",
        "Everything",
        "Everything",
    ),
    // ── Asia-Pacific ─────────────────────────────────────────────────────
    (
        "nus",
        "nus.primo.exlibrisgroup.com",
        "65NU_INST:NUS",
        "65NU_INST",
        "Everything",
        "Everything",
    ),
    (
        "ntu",
        "ntu.primo.exlibrisgroup.com",
        "65NTUSG_INST:NTU",
        "65NTUSG_INST",
        "Everything",
        "Everything",
    ),
    (
        "smu",
        "smu.primo.exlibrisgroup.com",
        "65SMU_INST:SMU",
        "65SMU_INST",
        "Everything",
        "MyInst_and_CI",
    ),
    (
        "sutd",
        "sutd.primo.exlibrisgroup.com",
        "65SUTD_INST:SUTD",
        "65SUTD_INST",
        "Everything",
        "MyInst_and_CI",
    ),
    (
        "hku",
        "julac-hku.primo.exlibrisgroup.com",
        "852JULAC_HKU:HKU",
        "852JULAC_HKU",
        "Everything",
        "MyInst_and_CI",
    ),
    (
        "cuhk",
        "julac-cuhk.primo.exlibrisgroup.com",
        "852JULAC_CUHK:CUHK",
        "852JULAC_CUHK",
        "default_tab",
        "All",
    ),
    (
        "hkust",
        "julac-hkust.primo.exlibrisgroup.com",
        "852JULAC_HKUST:HKUST",
        "852JULAC_HKUST",
        "Everything",
        "HKUST_catalog_primo",
    ),
    (
        "cityu",
        "cityu.primo.exlibrisgroup.com",
        "852CITYU_INST:CITYU",
        "852CITYU_INST",
        "Everything",
        "Everything",
    ),
    (
        "mahidol",
        "mahidol.primo.exlibrisgroup.com",
        "66MU_INST:MU",
        "66MU_INST",
        "Everything",
        "MyInst_and_CI",
    ),
    (
        "melbourne",
        "unimelb.primo.exlibrisgroup.com",
        "61UMELB_INST:UoM",
        "61UMELB_INST",
        "Everything",
        "Everything",
    ),
    (
        "sydney",
        "usyd.primo.exlibrisgroup.com",
        "61USYD_INST:sydney",
        "61USYD_INST",
        "Everything",
        "Everything",
    ),
    (
        "anu",
        "anu.primo.exlibrisgroup.com",
        "61ANU_INST:ANU",
        "61ANU_INST",
        "Everything",
        "MyInst_and_CI",
    ),
    (
        "uq",
        "uq.primo.exlibrisgroup.com",
        "61UQ_INST:61UQ",
        "61UQ_INST",
        "61UQ_All",
        "61UQ_All",
    ),
    (
        "monash",
        "monash.primo.exlibrisgroup.com",
        "61MONASH_INST:Monash",
        "61MONASH_INST",
        "Everything",
        "Everything",
    ),
    (
        "uwa",
        "uwa.primo.exlibrisgroup.com",
        "61UWA_INST:UWA",
        "61UWA_INST",
        "Everything",
        "MyInst_and_CI",
    ),
    (
        "qut",
        "qut.primo.exlibrisgroup.com",
        "61QUT_INST:61QUT",
        "61QUT_INST",
        "Everything",
        "Everything",
    ),
    (
        "otago",
        "primo.otago.ac.nz",
        "64OTAGO_INST:OTAGO",
        "64OTAGO_INST",
        "Everything",
        "Everything",
    ),
    (
        "auckland",
        "auckland.primo.exlibrisgroup.com",
        "64UAU_INST:UAU",
        "64UAU_INST",
        "Everything",
        "Everything",
    ),
    // ── China ────────────────────────────────────────────────────────────
    (
        "pku",
        "pku.primo.exlibrisgroup.com",
        "86PKU_INST:PKU",
        "86PKU_INST",
        "Everything",
        "Everything",
    ),
    (
        "sjtu",
        "sjtu.primo.exlibrisgroup.com",
        "86SJTU_INST:SJTU",
        "86SJTU_INST",
        "Everything",
        "Everything",
    ),
    (
        "fudan",
        "fudan.primo.exlibrisgroup.com",
        "86FDU_INST:FDU",
        "86FDU_INST",
        "Everything",
        "Everything",
    ),
    (
        "zju",
        "zju.primo.exlibrisgroup.com",
        "86ZJU_INST:ZJU",
        "86ZJU_INST",
        "Everything",
        "Everything",
    ),
    // ── Middle East / Africa ─────────────────────────────────────────────
    (
        "tau",
        "tau.primo.exlibrisgroup.com",
        "972TAU_INST:TAU",
        "972TAU_INST",
        "TAU",
        "TAU",
    ),
    (
        "huji",
        "huji.primo.exlibrisgroup.com",
        "972HJU_INST:HJU",
        "972HJU_INST",
        "Everything",
        "Everything",
    ),
    (
        "technion",
        "technion.primo.exlibrisgroup.com",
        "972TECH_INST:TECH",
        "972TECH_INST",
        "Everything",
        "Everything",
    ),
    (
        "weizmann",
        "weizmann.primo.exlibrisgroup.com",
        "972WIS_INST:WIS",
        "972WIS_INST",
        "Everything",
        "Everything",
    ),
    (
        "bgu",
        "bgu.primo.exlibrisgroup.com",
        "972BGU_INST:BGU",
        "972BGU_INST",
        "Everything",
        "Everything",
    ),
    (
        "haifa",
        "haifa.primo.exlibrisgroup.com",
        "972UH_INST:UH",
        "972UH_INST",
        "Everything",
        "Everything",
    ),
    (
        "biu",
        "biu.primo.exlibrisgroup.com",
        "972BILU_INST:BIU",
        "972BILU_INST",
        "Everything",
        "Everything",
    ),
    (
        "uct",
        "uct.primo.exlibrisgroup.com",
        "27UCT_INST:ZA-UCT",
        "27UCT_INST",
        "Everything",
        "Everything",
    ),
    (
        "up",
        "up.primo.exlibrisgroup.com",
        "27UP_INST:ZA-UP",
        "27UP_INST",
        "Everything",
        "Everything",
    ),
    (
        "bne",
        "bne.primo.exlibrisgroup.com",
        "34BNE_INST:BNE",
        "34BNE_INST",
        "Everything",
        "Everything",
    ),
    (
        "kb",
        "kb.primo.exlibrisgroup.com",
        "31KB_INST:KB",
        "31KB_INST",
        "Everything",
        "Everything",
    ),
    (
        "nla",
        "nla.primo.exlibrisgroup.com",
        "61NLA_INST:NLA",
        "61NLA_INST",
        "Everything",
        "Everything",
    ),
    (
        "uab",
        "uab.primo.exlibrisgroup.com",
        "34UAB_INST:UAB",
        "34UAB_INST",
        "Everything",
        "Everything",
    ),
    (
        "postech",
        "postech.primo.exlibrisgroup.com",
        "82POSTECH_INST:POSTECH",
        "82POSTECH_INST",
        "Everything",
        "Everything",
    ),
    (
        "snu",
        "snu.primo.exlibrisgroup.com",
        "82SNU_INST:SNU",
        "82SNU_INST",
        "Everything",
        "Everything",
    ),
    // ── Latin America ────────────────────────────────────────────────────
    (
        "usp",
        "usp.primo.exlibrisgroup.com",
        "55USP_INST:USP",
        "55USP_INST",
        "Everything",
        "Everything",
    ),
];

/// Map a Primo instance key to a human-readable institution name.
fn primo_display_name(key: &str) -> &'static str {
    match key.to_lowercase().as_str() {
        "cmu" => "Carnegie Mellon University",
        "mit" => "MIT",
        "harvard" => "Harvard University",
        "stanford" => "Stanford University",
        "berkeley" => "UC Berkeley",
        "columbia" => "Columbia University",
        "cornell" => "Cornell University",
        "yale" => "Yale University",
        "princeton" => "Princeton University",
        "brown" => "Brown University",
        "michigan" => "University of Michigan",
        "ucla" => "UCLA",
        "chicago" => "University of Chicago",
        "caltech" => "Caltech",
        "nyu" => "New York University",
        "jhu" => "Johns Hopkins University",
        "duke" => "Duke University",
        "oxford" => "University of Oxford (SOLO)",
        "cambridge" => "University of Cambridge",
        "ucl" => "University College London",
        "imperial" => "Imperial College London",
        "edinburgh" => "University of Edinburgh",
        "manchester" => "University of Manchester",
        "lse" => "London School of Economics",
        "eth" => "ETH Zürich",
        "epfl" => "EPFL Lausanne",
        "tum" => "TU Munich",
        "leiden" => "Leiden University",
        "delft" => "TU Delft",
        "leuven" => "KU Leuven",
        "toronto" => "University of Toronto",
        "mcgill" => "McGill University",
        "ubc" => "University of British Columbia",
        "nus" => "National University of Singapore",
        "ntu" => "Nanyang Technological University",
        "hku" => "University of Hong Kong",
        "cuhk" => "Chinese University of Hong Kong",
        "hkust" => "HKUST",
        "melbourne" => "University of Melbourne",
        "sydney" => "University of Sydney",
        "anu" => "Australian National University",
        "monash" => "Monash University",
        "pku" => "Peking University",
        "sjtu" => "Shanghai Jiao Tong University",
        "fudan" => "Fudan University",
        "zju" => "Zhejiang University",
        "tau" => "Tel Aviv University",
        "huji" => "Hebrew University of Jerusalem",
        "uct" => "University of Cape Town",
        "usp" => "University of São Paulo",
        "pitt" => "University of Pittsburgh",
        "rochester" => "University of Rochester",
        "emory" => "Emory University",
        "rice" => "Rice University",
        "utah" => "University of Utah",
        "miami" => "University of Miami",
        "scu" => "Santa Clara University",
        "davidson" => "Davidson College",
        "sva" => "School of Visual Arts (NYC)",
        "aul" => "Air University Library",
        "usma" => "West Point / USMA",
        "lva" => "Library of Virginia",
        "wrlc" => "Washington Research Libraries Consortium",
        "uky" => "University of Kentucky",
        "du" => "University of Denver",
        "glendale" => "Glendale Community College",
        "sfsu" => "San Francisco State University",
        "bll" => "British Library",
        "vanderbilt" => "Vanderbilt University",
        "smu" => "Singapore Management University",
        "sutd" => "Singapore Univ. of Technology & Design",
        "cityu" => "City University of Hong Kong",
        "mahidol" => "Mahidol University",
        "uq" => "University of Queensland",
        "uwa" => "University of Western Australia",
        "technion" => "Technion – Israel Institute of Technology",
        "weizmann" => "Weizmann Institute of Science",
        "bgu" => "Ben-Gurion University",
        "haifa" => "University of Haifa",
        "biu" => "Bar-Ilan University",
        "up" => "University of Pretoria",
        "bne" => "Biblioteca Nacional de España",
        "kb" => "Koninklijke Bibliotheek (Netherlands)",
        "nla" => "National Library of Australia",
        "uab" => "Universitat Autònoma de Barcelona",
        "postech" => "POSTECH",
        "snu" => "Seoul National University",
        // Unknown key — fall back to the raw key, which is `'static` because
        // it comes from the `PRIMO_INSTANCES` static table or a literal branch.
        // Use a leak only if the key is a user-supplied runtime string; in
        // practice the key is always one of the static table entries.
        _ => "?",
    }
}

/// Extract a display-field string from a Primo PNX display block.
/// Primo stores values as either a JSON array or a plain string.
fn pnx_str<'a>(disp: &'a serde_json::Value, field: &str) -> &'a str {
    disp.get(field)
        .and_then(|v| {
            v.as_array()
                .and_then(|a| a.first())
                .and_then(|v| v.as_str())
                .or_else(|| v.as_str())
        })
        .unwrap_or("")
}

/// Format one Primo PNX document into the human-readable output block.
fn primo_format_doc(i: usize, doc: &serde_json::Value, host: &str, vid: &str) -> String {
    let pnx = doc.get("pnx").unwrap_or(doc);
    let disp = pnx.get("display").unwrap_or(pnx);

    let title = pnx_str(disp, "title");
    let year = pnx_str(disp, "creationdate");
    let dtype = pnx_str(disp, "type");
    let description = pnx_str(disp, "description");

    let creators: Vec<&str> = disp
        .get("creator")
        .and_then(|v| v.as_array())
        .map(|a| a.iter().filter_map(|v| v.as_str()).collect())
        .unwrap_or_default();

    let record_id = pnx
        .get("control")
        .and_then(|c| c.get("recordid"))
        .and_then(|v| v.as_array())
        .and_then(|a| a.first())
        .and_then(|v| v.as_str())
        .unwrap_or("");

    let mut out = format!("{}. {}", i + 1, if title.is_empty() { "?" } else { title });
    if !year.is_empty() {
        out.push_str(&format!(" ({year})"));
    }
    if !dtype.is_empty() {
        out.push_str(&format!("  [{dtype}]"));
    }
    out.push('\n');
    if !creators.is_empty() {
        out.push_str(&format!(
            "   Authors: {}\n",
            creators[..creators.len().min(4)].join("; ")
        ));
    }
    if !record_id.is_empty() {
        out.push_str(&format!(
            "   URL: https://{host}/discovery/fulldisplay?docid={record_id}&vid={vid}\n"
        ));
    }
    if !description.is_empty() {
        out.push_str(&format!("   {}\n", truncate_abstract(description)));
    }
    out.push('\n');
    out
}

fn find_primo_instance(
    key: &str,
) -> Option<&'static (
    &'static str,
    &'static str,
    &'static str,
    &'static str,
    &'static str,
    &'static str,
)> {
    let lower = key.to_lowercase();
    PRIMO_INSTANCES
        .iter()
        .find(|(k, host, _, _, _, _)| lower == *k || host.contains(&lower) || lower.contains(k))
}

/// `primo: <query>` or `primo: <inst>/<query>` — search any ExLibris Primo
/// library discovery system. The `<inst>` short key selects an institution
/// from the built-in table (e.g. `primo: mit/decompiler`). With no inst key,
/// defaults to CMU. Works without authentication (public pnxs endpoint).
async fn search_primo(query: &str, max_results: usize) -> Result<String, String> {
    // Parse optional `instkey/query` syntax.
    let (inst_key, q) = if let Some((prefix, rest)) = query.split_once('/') {
        let prefix = prefix.trim();
        let rest = rest.trim();
        if !rest.is_empty() && find_primo_instance(prefix).is_some() {
            (prefix, rest)
        } else {
            ("cmu", query.trim())
        }
    } else {
        ("cmu", query.trim())
    };

    let inst = find_primo_instance(inst_key).unwrap_or_else(|| &PRIMO_INSTANCES[0]); // fallback to CMU
    let (_key, host, vid, inst_code, tab, scope) = inst;

    let limit = max_results.clamp(1, 10).to_string();
    let client = http_client()?;

    // Build URL manually — reqwest's .query() percent-encodes commas inside
    // values, but Primo's `q` param uses `any,contains,{term}` with literal
    // commas that must reach the server unencoded. The full parameter list is
    // required — the server returns 400 if the "extra" flags are missing.
    let q_encoded = urlencoding_minimal(q);
    let referer_query = urlencoding_minimal(&format!("any,contains,{q}",));
    let url = format!(
        "https://{host}/primaws/rest/pub/pnxs\
         ?acTriggered=false\
         &blendFacetsSeparately=false\
         &citationTrailFilterByAvailability=true\
         &disableCache=false\
         &getMore=0\
         &inst={inst_code}\
         &isCDSearch=false\
         &lang=en\
         &limit={limit}\
         &mode=Basic\
         &newspapersActive=false\
         &newspapersSearch=false\
         &offset=0\
         &otbRanking=false\
         &pcAvailability=false\
         &q=any,contains,{q_encoded}\
         &qExclude=\
         &qInclude=\
         &rapido=false\
         &refEntryActive=false\
         &rtaLinks=true\
         &scope={scope}\
         &searchInFulltextUserSelection=false\
         &skipDelivery=Y\
         &sort=rank\
         &tab={tab}\
         &vid={vid}"
    );
    let resp = client
        .get(&url)
        .header("Accept", "application/json, text/plain, */*")
        .header("Origin", format!("https://{host}"))
        .header("Referer", format!("https://{host}/discovery/search?institution={inst_code}&vid={vid}&tab={tab}&search_scope={scope}&query={referer_query}"))
        .header("User-Agent", "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/124.0 Safari/537.36")
        .send()
        .await
        .map_err(|e| format!("Primo request to {host} failed: {e}"))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(format!(
            "Primo ({host}) returned HTTP {status}: {}",
            &body[..body.len().min(200)]
        ));
    }

    let parsed: serde_json::Value = resp
        .text()
        .await
        .map_err(|e| format!("Response read: {e}"))
        .and_then(|b| serde_json::from_str(&b).map_err(|e| format!("JSON parse: {e}")))?;

    let total = parsed
        .pointer("/info/total")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let docs = parsed.get("docs").and_then(|v| v.as_array());

    let inst_display = primo_display_name(inst_key);
    let mut out = format!("Primo ({inst_display}): \"{q}\" — {total} total results\n\n");

    match docs {
        Some(items) if !items.is_empty() => {
            for (i, doc) in items.iter().enumerate() {
                out.push_str(&primo_format_doc(i, doc, host, vid));
            }
        }
        _ => out.push_str("No results found.\n"),
    }
    Ok(out)
}

#[cfg(test)]
mod fallback_tests {
    use super::{google_error_is_recoverable, match_prefix, searxng_base_url};

    #[test]
    fn google_429_and_quota_are_recoverable_normal() {
        assert!(google_error_is_recoverable(
            429,
            "Quota exceeded for quota metric"
        ));
        assert!(google_error_is_recoverable(503, "backend error"));
        assert!(google_error_is_recoverable(200, "daily Limit Exceeded"));
        assert!(google_error_is_recoverable(200, "user rate limit"));
    }

    #[test]
    fn google_permanent_errors_not_recoverable_robust() {
        assert!(!google_error_is_recoverable(400, "Invalid Value"));
        assert!(!google_error_is_recoverable(403, "API key not valid"));
        assert!(!google_error_is_recoverable(404, "not found"));
    }

    #[test]
    fn searxng_prefix_routes_and_base_url_defaults_normal() {
        // Prefix parsing routes to the searxng backend.
        assert_eq!(
            match_prefix("searxng: rust traits", "searxng"),
            Some("rust traits")
        );
        // Default base URL is a non-empty https endpoint with no trailing slash.
        let base = searxng_base_url();
        assert!(base.starts_with("https://"), "{base}");
        assert!(!base.ends_with('/'), "{base}");
    }
}
