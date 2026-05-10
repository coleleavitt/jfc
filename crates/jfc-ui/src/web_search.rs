//! Web search backends for the `WebSearch` tool.
//!
//! Supports four search backends, selected by query prefix:
//!
//! | Prefix | Backend | Example |
//! |--------|---------|---------|
//! | *(none)* | Google CSE | `rust async traits` |
//! | `arxiv:` | arXiv API | `arxiv: transformer attention mechanism` |
//! | `scholar:` | Semantic Scholar | `scholar: attention is all you need` |
//! | `papers:` | arXiv + S2 BFF in parallel, deduped | `papers: graph neural networks` |
//!
//! ## Google CSE
//! Reads API keys from `~/.config/google-search-mcp/config.toml` (shared
//! with the standalone google-cse-mcp-rs server). Falls back to
//! `GOOGLE_CSE_API_KEY` + `GOOGLE_CSE_CX` env vars.
//!
//! ## arXiv
//! Uses the public Atom feed API at `export.arxiv.org`. No API key needed.
//!
//! ## Semantic Scholar
//! Uses the public Graph API. Optional `SEMANTIC_SCHOLAR_API_KEY` env var
//! or `~/.config/academic-papers-mcp/config.toml` for higher rate limits.

use std::path::PathBuf;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicUsize, Ordering};

use serde::Deserialize;

// ── Public entry point ──────────────────────────────────────────────────────

/// Route a search query to the appropriate backend based on prefix.
pub async fn search(query: &str, max_results: usize) -> Result<String, String> {
    if let Some(q) = query
        .strip_prefix("arxiv:")
        .or_else(|| query.strip_prefix("arxiv "))
    {
        search_arxiv(q.trim(), max_results).await
    } else if let Some(q) = query
        .strip_prefix("scholar:")
        .or_else(|| query.strip_prefix("scholar "))
    {
        search_semantic_scholar(q.trim(), max_results).await
    } else if let Some(q) = query
        .strip_prefix("papers:")
        .or_else(|| query.strip_prefix("papers "))
    {
        search_papers_combined(q.trim(), max_results).await
    } else {
        search_google(query, max_results).await
    }
}

/// Run arXiv and Semantic Scholar (BFF) searches concurrently and merge
/// results, deduplicating by arXiv ID / DOI / normalized title.
async fn search_papers_combined(query: &str, max_results: usize) -> Result<String, String> {
    let per_backend = max_results.max(3); // each backend returns up to N, then we cap
    let (arxiv_res, s2_res) = tokio::join!(
        search_arxiv_entries(query, per_backend),
        search_s2_via_bff_entries(query, per_backend),
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

    entries.truncate(max_results);

    let mut out = format!(
        "Papers: \"{query}\" — {} unique results (arXiv: {arxiv_count}, S2: {s2_count})\n",
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
                format!("{}...", &e.abstract_text[..200])
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
    if let Some(keys) = load_google_keys_from_config() {
        if !keys.is_empty() {
            tracing::info!(count = keys.len(), "Google CSE keys loaded from config");
            return keys;
        }
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
    let key = pool.next().ok_or_else(|| {
        "No Google CSE API keys configured. Set GOOGLE_CSE_API_KEY + GOOGLE_CSE_CX \
         env vars, or create ~/.config/google-search-mcp/config.toml"
            .to_string()
    })?;

    let num = max_results.min(10).max(1);
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
        return Err(format!(
            "Google CSE API error {}: {}",
            err.code, err.message
        ));
    }
    if !status.is_success() {
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

/// Run an arXiv search and return structured entries (used by combined search).
async fn search_arxiv_entries(query: &str, max_results: usize) -> Result<Vec<PaperEntry>, String> {
    let num = max_results.min(20).max(1);
    let client = http_client()?;
    let num_str = num.to_string();

    let resp = client
        .get("https://export.arxiv.org/api/query")
        .query(&[
            ("search_query", &format!("all:{query}")),
            ("max_results", &num_str),
            ("start", &"0".to_string()),
            ("sortBy", &"relevance".to_string()),
            ("sortOrder", &"descending".to_string()),
        ])
        .send()
        .await
        .map_err(|e| format!("arXiv request failed: {e}"))?;

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
    let num = max_results.min(20).max(1);
    let client = http_client()?;
    let num_str = num.to_string();

    let resp = client
        .get("https://export.arxiv.org/api/query")
        .query(&[
            ("search_query", &format!("all:{query}")),
            ("max_results", &num_str),
            ("start", &"0".to_string()),
            ("sortBy", &"relevance".to_string()),
            ("sortOrder", &"descending".to_string()),
        ])
        .send()
        .await
        .map_err(|e| format!("arXiv request failed: {e}"))?;

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
                format!("{}...", &entry.summary[..200])
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

    loop {
        let start = match xml[search_from..].find("<entry>") {
            Some(pos) => search_from + pos,
            None => break,
        };
        let end = match xml[start..].find("</entry>") {
            Some(pos) => start + pos + "</entry>".len(),
            None => break,
        };
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
        if let Ok(k) = std::env::var("SEMANTIC_SCHOLAR_API_KEY") {
            if !k.is_empty() {
                return Some(k);
            }
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
    let limit = max_results.min(20).max(1);

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
                        format!("{}...", &abs[..200])
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
                format!("{}...", &e.abstract_text[..200])
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
