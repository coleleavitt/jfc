//! OpenWebUI OIDC + Duo automated login flow.
//!
//! Direct port of `opencode-openwebui-auth/src/oauth/oidc-login.ts`.
//! Reverse-engineered from a Burp capture of chat.ai2s.org → shibboleth.arizona.edu
//! → api-*.duosecurity.com. The 6-step chain:
//!
//!   1. GET  /oauth/oidc/login              → capture owui-session, follow redirect to Shibboleth
//!   2. POST execution=e1s1 → e1s2          → submit NetID + password
//!   3. Navigate Spring Web Flow             → Duo handoff
//!   4. Duo Universal Prompt v4 (frameless) → push or passcode
//!   5. duo-callback → e1s3 → e1s4           → Shibboleth issues OIDC code
//!   6. /oauth/oidc/callback                 → Open WebUI sets `token` cookie
//!
//! Cookies are persisted to `~/.config/opencode/openwebui-cookies.json` so a
//! warm session can skip credentials and only re-do Duo (matches the plugin).

#![allow(dead_code)]

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};

use super::openwebui_jwt::token_expires_at_ms;

const UA: &str = "Mozilla/5.0 (X11; Linux x86_64; rv:149.0) Gecko/20100101 Firefox/149.0";

const DUO_BROWSER_FEATURES: &str = r#"{"touch_supported":false,"platform_authenticator_status":"unavailable","webauthn_supported":true,"screen_resolution_height":1200,"screen_resolution_width":1920,"screen_color_depth":24,"is_uvpa_available":false,"client_capabilities_uvpa":false}"#;

/// Result of a successful OIDC login.
#[derive(Debug, Clone)]
pub struct OidcLoginResult {
    /// HS256 JWT issued by Open WebUI.
    pub token: String,
    /// Underlying Shibboleth id_token (RS256). Empty string if absent.
    pub oauth_id_token: String,
    /// Session UUID. Empty string if absent.
    pub oauth_session_id: String,
    /// Unix milliseconds when the JWT expires.
    pub expires_at: i64,
}

/// Inputs for `oidc_login`.
#[derive(Debug, Clone)]
pub struct OidcLoginOptions {
    /// e.g. "https://chat.ai2s.org" — no trailing slash required.
    pub base_url: String,
    /// NetID / username.
    pub username: String,
    /// NetID password.
    pub password: String,
    /// Optional 6-digit Duo Mobile passcode. When set, used instead of push.
    pub duo_passcode: Option<String>,
    /// "push" (default) or "passcode".
    pub duo_method: DuoMethod,
    /// Status-poll interval. Default 2 s.
    pub poll_interval: Duration,
    /// Total deadline for Duo approval. Default 60 s.
    pub poll_timeout: Duration,
}

impl OidcLoginOptions {
    pub fn new(base_url: impl Into<String>, username: impl Into<String>, password: impl Into<String>) -> Self {
        Self {
            base_url: base_url.into(),
            username: username.into(),
            password: password.into(),
            duo_passcode: None,
            duo_method: DuoMethod::Push,
            poll_interval: Duration::from_secs(2),
            poll_timeout: Duration::from_secs(60),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DuoMethod {
    Push,
    Passcode,
}

/// Cookie jar — minimal per-domain storage that mirrors the plugin's CookieJar.
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct CookieJar {
    /// domain → name → value
    cookies: HashMap<String, HashMap<String, String>>,
}

impl CookieJar {
    pub fn new() -> Self {
        Self::default()
    }

    /// Capture all `Set-Cookie` headers from a response into the jar.
    pub fn capture(&mut self, url: &str, headers: &reqwest::header::HeaderMap) {
        let Ok(parsed) = url::Url::parse(url) else { return };
        let Some(domain) = parsed.host_str().map(|s| s.to_owned()) else { return };
        let jar = self.cookies.entry(domain).or_default();

        for raw in headers.get_all(reqwest::header::SET_COOKIE).iter() {
            let Ok(s) = raw.to_str() else { continue };
            let pair = s.split(';').next().unwrap_or("").trim();
            let Some(eq) = pair.find('=') else { continue };
            let name = pair[..eq].trim().to_string();
            let value = pair[eq + 1..].trim().to_string();
            // null value or expired cookie → delete
            if value == "null" || s.contains("expires=Thu, 01 Jan 1970") {
                jar.remove(&name);
            } else {
                jar.insert(name, value);
            }
        }
    }

    /// Build the `Cookie:` header for a given URL. Includes parent domains.
    pub fn header_for(&self, url: &str) -> String {
        let Ok(parsed) = url::Url::parse(url) else { return String::new() };
        let Some(domain) = parsed.host_str() else { return String::new() };
        let mut parts = Vec::new();
        for (d, jar) in &self.cookies {
            if domain == d || domain.ends_with(&format!(".{d}")) {
                for (k, v) in jar {
                    parts.push(format!("{k}={v}"));
                }
            }
        }
        parts.join("; ")
    }

    /// Get a specific cookie value.
    pub fn get(&self, domain: &str, name: &str) -> Option<&str> {
        self.cookies.get(domain).and_then(|m| m.get(name).map(|s| s.as_str()))
    }
}

/// Cookie jar persisted at `~/.config/opencode/openwebui-cookies.json`.
fn cookie_jar_path() -> PathBuf {
    let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/"));
    home.join(".config/opencode/openwebui-cookies.json")
}

#[derive(Serialize, Deserialize)]
struct PersistedJar {
    v: u32,
    ts: u64,
    cookies: HashMap<String, HashMap<String, String>>,
}

/// Persist the jar so a warm session can skip credentials. Best-effort.
pub fn save_cookie_jar(jar: &CookieJar) {
    let path = cookie_jar_path();
    let Some(parent) = path.parent() else { return };
    if std::fs::create_dir_all(parent).is_err() {
        return;
    }
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    let payload = PersistedJar { v: 1, ts, cookies: jar.cookies.clone() };
    let Ok(json) = serde_json::to_string(&payload) else { return };
    let pid = std::process::id();
    let tmp = path.with_extension(format!("json.tmp-{pid}"));
    if std::fs::write(&tmp, json).is_ok() {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600));
        }
        let _ = std::fs::rename(&tmp, &path);
    }
}

/// Load the persisted jar if it exists and is < 24h old.
pub fn load_cookie_jar() -> Option<CookieJar> {
    let raw = std::fs::read_to_string(cookie_jar_path()).ok()?;
    let parsed: PersistedJar = serde_json::from_str(&raw).ok()?;
    if parsed.v != 1 {
        return None;
    }
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    if now_ms.saturating_sub(parsed.ts) > 24 * 60 * 60 * 1000 {
        return None;
    }
    Some(CookieJar { cookies: parsed.cookies })
}

/// Build a Reqwest client configured for the OIDC flow: 30 s timeout, no auto
/// redirect (we follow them manually so we can capture cookies at every hop).
fn build_client() -> anyhow::Result<reqwest::Client> {
    Ok(reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(30))
        .user_agent(UA)
        .build()?)
}

/// Make a single HTTP request, threading the jar.
async fn request(
    client: &reqwest::Client,
    jar: &mut CookieJar,
    method: reqwest::Method,
    url: &str,
    body: Option<&str>,
    extra: &[(&str, &str)],
) -> anyhow::Result<reqwest::Response> {
    let mut req = client.request(method, url).header("User-Agent", UA);
    let cookie = jar.header_for(url);
    if !cookie.is_empty() {
        req = req.header("Cookie", cookie);
    }
    if body.is_some() && !extra.iter().any(|(k, _)| k.eq_ignore_ascii_case("content-type")) {
        req = req.header("Content-Type", "application/x-www-form-urlencoded");
    }
    for (k, v) in extra {
        req = req.header(*k, *v);
    }
    if let Some(b) = body {
        req = req.body(b.to_owned());
    }
    let res = req.send().await?;
    jar.capture(url, res.headers());
    Ok(res)
}

/// Final result of `follow_redirects`: the response body, the final URL, and
/// the HTTP status of the terminal response.
struct FollowResult {
    body: String,
    url: String,
    status: u16,
}

/// Follow up to `max_hops` 3xx redirects, capturing cookies at every hop.
async fn follow_redirects(
    client: &reqwest::Client,
    jar: &mut CookieJar,
    url: &str,
    method: reqwest::Method,
    body: Option<&str>,
    extra: &[(&str, &str)],
    max_hops: u32,
) -> anyhow::Result<FollowResult> {
    let mut current_url = url.to_string();
    let mut current_method = method.clone();
    let mut current_body = body.map(|s| s.to_string());

    let mut res = request(client, jar, current_method.clone(), &current_url, current_body.as_deref(), extra).await?;

    for _ in 0..max_hops {
        let status = res.status().as_u16();
        if !(300..400).contains(&status) {
            break;
        }
        let location = res
            .headers()
            .get(reqwest::header::LOCATION)
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_owned());
        let Some(loc) = location else { break };
        let _ = res.text().await;
        let abs = url::Url::parse(&current_url)
            .ok()
            .and_then(|base| base.join(&loc).ok())
            .map(|u| u.to_string())
            .unwrap_or(loc);
        // 303 always becomes GET. 307/308 preserve method+body. 301/302 also typically GET.
        let new_method = if status == 307 || status == 308 {
            current_method.clone()
        } else {
            reqwest::Method::GET
        };
        let new_body = if matches!(new_method, reqwest::Method::GET) {
            None
        } else {
            current_body.clone()
        };
        current_url = abs;
        current_method = new_method;
        current_body = new_body;
        res = request(client, jar, current_method.clone(), &current_url, current_body.as_deref(), extra).await?;
    }

    let final_status = res.status().as_u16();
    let body = res.text().await.unwrap_or_default();
    Ok(FollowResult { body, url: current_url, status: final_status })
}

/* ------------------------- HTML helpers ----------------------- */

fn extract_form_action(html: &str, base_url: &str) -> Option<String> {
    let re = regex::Regex::new(r#"<form[^>]*action="([^"]+)""#).ok()?;
    let m = re.captures(html)?;
    let action = m.get(1)?.as_str().replace("&amp;", "&");
    let base = url::Url::parse(base_url).ok()?;
    Some(base.join(&action).ok()?.to_string())
}

fn extract_hidden_fields(html: &str) -> HashMap<String, String> {
    let mut fields = HashMap::new();
    let re = regex::Regex::new(r#"<input[^>]*type="hidden"[^>]*>"#).unwrap();
    for m in re.find_iter(html) {
        let tag = m.as_str();
        let name = regex::Regex::new(r#"name="([^"]+)""#)
            .unwrap()
            .captures(tag)
            .and_then(|c| c.get(1))
            .map(|m| m.as_str().to_owned());
        let value = regex::Regex::new(r#"value="([^"]*)""#)
            .unwrap()
            .captures(tag)
            .and_then(|c| c.get(1))
            .map(|m| m.as_str().replace("&amp;", "&"))
            .unwrap_or_default();
        if let Some(n) = name {
            fields.insert(n, value);
        }
    }
    fields
}

fn url_encode(pairs: &HashMap<String, String>) -> String {
    let mut parts: Vec<(String, String)> = pairs.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
    parts.sort_by(|a, b| a.0.cmp(&b.0));
    serde_urlencoded::to_string(&parts).unwrap_or_default()
}

/* ------------------------- Step implementations ----------------------- */

async fn step1_initiate_oidc(client: &reqwest::Client, jar: &mut CookieJar, base_url: &str) -> anyhow::Result<String> {
    tracing::info!(target: "jfc::oidc", "Step 1: Initiating OIDC login");
    let url = format!("{base_url}/oauth/oidc/login");
    let r = follow_redirects(client, jar, &url, reqwest::Method::GET, None, &[], 10).await?;
    if !r.url.contains("shibboleth.arizona.edu") && !r.url.contains("webauth.arizona.edu") {
        anyhow::bail!("Step 1: Expected redirect to Shibboleth, got {}", r.url);
    }
    tracing::info!(target: "jfc::oidc", url = %r.url, "Step 1: Landed on Shibboleth");
    Ok(r.url)
}

struct Step2Out {
    url: String,
    body: String,
    skipped_credentials: bool,
}

async fn step2_submit_credentials(
    client: &reqwest::Client,
    jar: &mut CookieJar,
    shib_url: &str,
    username: &str,
    password: &str,
) -> anyhow::Result<Step2Out> {
    tracing::info!(target: "jfc::oidc", "Step 2a: Fetching localStorage probe");
    let probe = follow_redirects(client, jar, shib_url, reqwest::Method::GET, None, &[], 10).await?;
    let probe_action = extract_form_action(&probe.body, &probe.url)
        .ok_or_else(|| anyhow::anyhow!("Step 2a: Could not find e1s1 form action"))?;
    let mut probe_fields = extract_hidden_fields(&probe.body);
    probe_fields.insert("shib_idp_ls_supported".into(), "true".into());
    probe_fields.insert("shib_idp_ls_success.shib_idp_session_ss".into(), "true".into());
    probe_fields.insert("shib_idp_ls_success.shib_idp_persistent_ss".into(), "true".into());
    probe_fields.entry("_eventId_proceed".into()).or_insert_with(String::new);

    tracing::info!(target: "jfc::oidc", action = %probe_action, "Step 2a: Submitting localStorage probe");
    let probe_body = url_encode(&probe_fields);
    let probe_res = follow_redirects(
        client, jar, &probe_action, reqwest::Method::POST, Some(&probe_body), &[], 10,
    )
    .await?;

    let login_html = probe_res.body;
    let login_url = probe_res.url;
    tracing::info!(target: "jfc::oidc", url = %login_url, "Step 2a: Advanced");

    if !login_html.contains("j_username") || !login_html.contains("j_password") {
        if login_url.contains("Duo")
            || login_url.contains("duo")
            || login_url.contains("execution=e1s3")
            || login_url.contains("oauth/oidc/callback")
            || login_html.contains("duo_form")
            || login_html.contains("duosecurity.com")
        {
            tracing::info!(target: "jfc::oidc", "Step 2: Shibboleth session alive — skipping credentials");
            return Ok(Step2Out { url: login_url, body: login_html, skipped_credentials: true });
        }
        anyhow::bail!("Step 2a: Expected login form on {}", login_url);
    }

    let login_action = extract_form_action(&login_html, &login_url)
        .ok_or_else(|| anyhow::anyhow!("Step 2b: Could not find e1s2 login form action"))?;
    tracing::info!(target: "jfc::oidc", action = %login_action, "Step 2b: Submitting credentials");

    let mut login_fields = extract_hidden_fields(&login_html);
    login_fields.insert("j_username".into(), username.to_owned());
    login_fields.insert("j_password".into(), password.to_owned());
    login_fields.insert("_eventId_proceed".into(), String::new());

    let login_body = url_encode(&login_fields);
    let r = follow_redirects(
        client, jar, &login_action, reqwest::Method::POST, Some(&login_body), &[], 10,
    )
    .await?;
    tracing::info!(target: "jfc::oidc", url = %r.url, "Step 2b: After credential submit");

    let bounced_back = r.url.contains("execution=e1s2") && r.body.contains("j_password");
    let has_error = r.body.contains("credentials you provided cannot be determined to be authentic")
        || r.body.contains("login-error");
    if bounced_back || has_error {
        anyhow::bail!("Step 2b: Login failed — invalid NetID or password");
    }

    Ok(Step2Out { url: r.url, body: r.body, skipped_credentials: false })
}

async fn step3_navigate_to_duo(
    client: &reqwest::Client,
    jar: &mut CookieJar,
    current_url: &str,
    current_body: &str,
) -> anyhow::Result<(String, String)> {
    tracing::info!(target: "jfc::oidc", "Step 3: Navigating to Duo 2FA");
    let mut url = current_url.to_string();
    let mut body = current_body.to_string();

    let duo_auth_re = regex::Regex::new(r#"/idp/profile/Authn/Duo/2FA/authorize[^"'\s]*"#).unwrap();
    let duo_embedded_re = regex::Regex::new(r#"https://api-[a-f0-9]+\.duosecurity\.com/[^"'\s]+"#).unwrap();
    let auto_redirect_re = regex::Regex::new(r#"window\.location\s*(?:\.href\s*)?=\s*['"]([^'"]+)"#).unwrap();
    let meta_refresh_re = regex::Regex::new(r#"http-equiv="refresh"\s+content="\d+;\s*url=([^"]+)""#).unwrap();
    let exec_re = regex::Regex::new(r"execution=e(\d+)s(\d+)").unwrap();

    for _ in 0..8 {
        if body.contains("duosecurity.com") || url.contains("duosecurity.com") {
            break;
        }

        if let Some(m) = duo_auth_re.find(&body) {
            let path = m.as_str().replace("&amp;", "&");
            let abs = url::Url::parse(&url)?.join(&path)?.to_string();
            tracing::info!(target: "jfc::oidc", url = %abs, "Step 3: Found Duo authorize link");
            let r = follow_redirects(client, jar, &abs, reqwest::Method::GET, None, &[], 10).await?;
            url = r.url;
            body = r.body;
            continue;
        }

        if duo_embedded_re.find(&body).is_some() {
            break;
        }

        if let Some(m) = auto_redirect_re.captures(&body).or_else(|| meta_refresh_re.captures(&body)) {
            let next = m.get(1).unwrap().as_str().replace("&amp;", "&");
            let abs = url::Url::parse(&url)?.join(&next)?.to_string();
            tracing::info!(target: "jfc::oidc", url = %abs, "Step 3: Following auto-redirect");
            let r = follow_redirects(client, jar, &abs, reqwest::Method::GET, None, &[], 10).await?;
            url = r.url;
            body = r.body;
            continue;
        }

        let action = extract_form_action(&body, &url);
        let has_event_proceed = body.contains("_eventId_proceed") || body.contains("_eventId=proceed");
        if let (Some(a), true) = (action, has_event_proceed) {
            let mut hidden = extract_hidden_fields(&body);
            hidden.entry("_eventId_proceed".into()).or_insert_with(String::new);
            let post_body = url_encode(&hidden);
            tracing::info!(target: "jfc::oidc", url = %a, "Step 3: Submitting auto-proceed form");
            let r = follow_redirects(client, jar, &a, reqwest::Method::POST, Some(&post_body), &[], 10).await?;
            url = r.url;
            body = r.body;
            continue;
        }

        if let Some(m) = exec_re.captures(&url) {
            let flow = m.get(1).unwrap().as_str().to_string();
            let step: u32 = m.get(2).unwrap().as_str().parse::<u32>().unwrap_or(0) + 1;
            let next_url = exec_re.replace(&url, format!("execution=e{flow}s{step}").as_str()).into_owned();
            tracing::info!(target: "jfc::oidc", "Step 3: Advancing to execution=e{flow}s{step}");
            let r = follow_redirects(client, jar, &next_url, reqwest::Method::GET, None, &[], 10).await?;
            url = r.url;
            body = r.body;
            continue;
        }

        break;
    }

    let mut duo_url = if url.contains("duosecurity.com") { Some(url.clone()) } else { None };
    if duo_url.is_none() {
        if let Some(m) = duo_embedded_re.find(&body) {
            duo_url = Some(m.as_str().replace("&amp;", "&"));
        }
    }
    if duo_url.is_none() {
        if let Some(m) = duo_auth_re.find(&body) {
            let path = m.as_str().replace("&amp;", "&");
            let abs = url::Url::parse(&url)?.join(&path)?.to_string();
            tracing::info!(target: "jfc::oidc", url = %abs, "Step 3: Following late-discovered Duo authorize");
            let r = follow_redirects(client, jar, &abs, reqwest::Method::GET, None, &[], 10).await?;
            if r.url.contains("duosecurity.com") {
                duo_url = Some(r.url.clone());
            }
            url = r.url;
            body = r.body;
        }
    }

    let duo_url = duo_url.ok_or_else(|| {
        anyhow::anyhow!("Step 3: Could not find Duo authorize URL. Current URL: {url}")
    })?;
    Ok((duo_url, body))
}

async fn post_expect_redirect(
    client: &reqwest::Client,
    jar: &mut CookieJar,
    url: &str,
    body: &str,
    headers: &[(&str, &str)],
    location_must_include: &str,
    label: &str,
) -> anyhow::Result<String> {
    let res = request(client, jar, reqwest::Method::POST, url, Some(body), headers).await?;
    let status = res.status().as_u16();
    let loc = res
        .headers()
        .get(reqwest::header::LOCATION)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_owned());
    let _ = res.text().await;
    if !(300..400).contains(&status) {
        anyhow::bail!("{label}: expected 3xx, got {status}");
    }
    let loc = loc.ok_or_else(|| anyhow::anyhow!("{label}: missing Location header"))?;
    if !loc.contains(location_must_include) {
        anyhow::bail!("{label}: expected Location containing \"{location_must_include}\", got \"{loc}\"");
    }
    let abs = url::Url::parse(url)?.join(&loc)?.to_string();
    Ok(abs)
}

const DUO_PLUGIN_FIELD_OVERRIDES: &[(&str, &str)] = &[
    ("screen_resolution_width", "1920"),
    ("screen_resolution_height", "1200"),
    ("color_depth", "24"),
    ("has_touch_capability", "false"),
    ("is_cef_browser", "false"),
    ("is_ipad_os", "false"),
    ("is_user_verifying_platform_authenticator_available", "false"),
    ("react_support", "true"),
    ("java_version", ""),
    ("flash_version", ""),
    ("ch_ua_error", ""),
    ("client_hints", ""),
    ("is_ie_compatibility_mode", ""),
    ("user_verifying_platform_authenticator_available_error", ""),
    ("acting_ie_version", ""),
    ("react_support_error_message", ""),
    ("extension_instance_key", ""),
    ("session_trust_extension_id", ""),
];

fn build_plugin_form_body(html: &str) -> String {
    let mut fields = extract_hidden_fields(html);
    for (k, v) in DUO_PLUGIN_FIELD_OVERRIDES {
        fields.insert((*k).to_owned(), (*v).to_owned());
    }
    url_encode(&fields)
}

#[derive(Deserialize)]
struct DuoStatusResponse {
    response: DuoStatusInner,
    #[serde(default)]
    stat: Option<String>,
}

#[derive(Deserialize)]
struct DuoStatusInner {
    #[serde(default)]
    status_code: Option<String>,
    #[serde(default)]
    result: Option<String>,
    #[serde(default)]
    reason: Option<String>,
}

#[derive(Deserialize)]
struct DuoPromptDataResponse {
    stat: String,
    #[serde(default)]
    message_enum: Option<i64>,
    response: DuoPromptDataInner,
}

#[derive(Deserialize, Default)]
struct DuoPromptDataInner {
    #[serde(default)]
    phones: Vec<DuoPhone>,
    #[serde(default)]
    auth_method_order: Vec<DuoAuthMethod>,
}

#[derive(Deserialize)]
struct DuoPhone {
    key: String,
    index: String,
    #[serde(default)]
    name: Option<String>,
}

#[derive(Deserialize)]
struct DuoAuthMethod {
    factor: String,
    #[serde(default)]
    device_key: Option<String>,
}

#[derive(Deserialize)]
struct DuoFactorResponse {
    stat: String,
    #[serde(default)]
    message_enum: Option<i64>,
    #[serde(default)]
    response: Option<DuoFactorInner>,
}

#[derive(Deserialize)]
struct DuoFactorInner {
    txid: String,
}

async fn step4_complete_duo(
    client: &reqwest::Client,
    jar: &mut CookieJar,
    frameless_url: &str,
    frameless_body: &str,
    opts: &OidcLoginOptions,
) -> anyhow::Result<String> {
    tracing::info!(target: "jfc::oidc", "Step 4: Starting Duo 2FA");
    let parsed = url::Url::parse(frameless_url)?;
    let duo_host = format!("{}://{}", parsed.scheme(), parsed.host_str().unwrap_or(""));
    let sid = parsed
        .query_pairs()
        .find(|(k, _)| k == "sid")
        .map(|(_, v)| v.to_string())
        .ok_or_else(|| anyhow::anyhow!("Step 4: Could not extract sid from frameless URL"))?;

    let form_fields0 = extract_hidden_fields(frameless_body);
    let xsrf_from_form = form_fields0
        .get("_xsrf")
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("Step 4: missing _xsrf hidden field"))?;

    let plugin_body1 = build_plugin_form_body(frameless_body);
    let post_headers: Vec<(&str, &str)> = vec![
        ("Content-Type", "application/x-www-form-urlencoded"),
        ("Origin", &duo_host),
        ("Referer", frameless_url),
        ("Accept", "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8"),
        ("Sec-Fetch-Site", "same-origin"),
        ("Sec-Fetch-Mode", "navigate"),
        ("Sec-Fetch-Dest", "document"),
        ("Upgrade-Insecure-Requests", "1"),
    ];

    // 4a: 1st plugin_form POST → 303 → /preauth/healthcheck
    tracing::info!(target: "jfc::oidc", "Step 4a: POST #1 plugin_form");
    let healthcheck_url = post_expect_redirect(
        client, jar, frameless_url, &plugin_body1, &post_headers,
        "/preauth/healthcheck", "Step 4a",
    )
    .await?;

    // 4b: GET healthcheck shell
    let hc_page = follow_redirects(
        client, jar, &healthcheck_url, reqwest::Method::GET, None,
        &[
            ("Accept", "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8"),
            ("Referer", frameless_url),
        ],
        5,
    )
    .await?;
    let xsrf_re = regex::Regex::new(r#""xsrf_token":\s*"([^"]+)""#).unwrap();
    let xsrf = xsrf_re
        .captures(&hc_page.body)
        .and_then(|c| c.get(1))
        .map(|m| m.as_str().to_owned())
        .unwrap_or_else(|| xsrf_from_form.clone());

    // 4c: AJAX healthcheck/data
    let hc_data_url = format!(
        "{duo_host}/frame/v4/preauth/healthcheck/data?sid={}",
        urlencoding::encode(&sid)
    );
    let hc_data = request(
        client, jar, reqwest::Method::GET, &hc_data_url, None,
        &[
            ("Accept", "*/*"),
            ("X-Xsrftoken", xsrf.as_str()),
            ("Content-Type", "application/x-www-form-urlencoded;charset=UTF-8"),
            ("Origin", &duo_host),
            ("Referer", &healthcheck_url),
            ("Sec-Fetch-Site", "same-origin"),
            ("Sec-Fetch-Mode", "cors"),
            ("Sec-Fetch-Dest", "empty"),
        ],
    )
    .await?;
    if !hc_data.status().is_success() {
        anyhow::bail!("Step 4c: /preauth/healthcheck/data returned {}", hc_data.status());
    }
    let _ = hc_data.text().await;

    // 4d: GET /frame/v4/return → 303 → frameless (2nd visit)
    let return_url = format!("{duo_host}/frame/v4/return?sid={}", urlencoding::encode(&sid));
    let return_res = request(
        client, jar, reqwest::Method::GET, &return_url, None,
        &[
            ("Accept", "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8"),
            ("Referer", &healthcheck_url),
            ("Sec-Fetch-Site", "same-origin"),
            ("Sec-Fetch-Mode", "navigate"),
            ("Sec-Fetch-Dest", "document"),
            ("Upgrade-Insecure-Requests", "1"),
        ],
    )
    .await?;
    let return_status = return_res.status().as_u16();
    let return_loc = return_res.headers()
        .get(reqwest::header::LOCATION)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_owned());
    let _ = return_res.text().await;
    let return_loc = return_loc.unwrap_or_default();
    if !(300..400).contains(&return_status) || !return_loc.contains("/frame/frameless/v4/auth") {
        anyhow::bail!("Step 4d: /return expected 303 → frameless, got {return_status} {return_loc}");
    }
    let frameless_url2 = url::Url::parse(&duo_host)?.join(&return_loc)?.to_string();
    let frameless2 = follow_redirects(
        client, jar, &frameless_url2, reqwest::Method::GET, None,
        &[
            ("Accept", "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8"),
            ("Referer", &healthcheck_url),
        ],
        5,
    )
    .await?;

    // 4e: 2nd plugin_form POST → 302 → /auth/prompt
    let plugin_body2 = build_plugin_form_body(&frameless2.body);
    let post_headers2: Vec<(&str, &str)> = vec![
        ("Content-Type", "application/x-www-form-urlencoded"),
        ("Origin", &duo_host),
        ("Referer", &frameless_url2),
        ("Accept", "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8"),
        ("Sec-Fetch-Site", "same-origin"),
        ("Sec-Fetch-Mode", "navigate"),
        ("Sec-Fetch-Dest", "document"),
        ("Upgrade-Insecure-Requests", "1"),
    ];
    tracing::info!(target: "jfc::oidc", "Step 4e: POST #2 plugin_form");
    let prompt_url = post_expect_redirect(
        client, jar, &frameless_url2, &plugin_body2, &post_headers2,
        "/auth/prompt", "Step 4e",
    )
    .await?;

    // 4f: GET /auth/prompt shell
    let prompt_page = follow_redirects(
        client, jar, &prompt_url, reqwest::Method::GET, None,
        &[
            ("Accept", "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8"),
            ("Referer", &frameless_url2),
        ],
        5,
    )
    .await?;
    let xsrf_prompt = xsrf_re
        .captures(&prompt_page.body)
        .and_then(|c| c.get(1))
        .map(|m| m.as_str().to_owned())
        .unwrap_or(xsrf.clone());

    // 4g: GET /auth/prompt/data
    let prompt_data_url = format!(
        "{duo_host}/frame/v4/auth/prompt/data?post_auth_action=OIDC_EXIT&browser_features={}&sid={}",
        urlencoding::encode(DUO_BROWSER_FEATURES),
        urlencoding::encode(&sid)
    );
    let prompt_data_res = request(
        client, jar, reqwest::Method::GET, &prompt_data_url, None,
        &[
            ("Accept", "*/*"),
            ("X-Xsrftoken", xsrf_prompt.as_str()),
            ("Content-Type", "application/x-www-form-urlencoded;charset=UTF-8"),
            ("Origin", &duo_host),
            ("Referer", &prompt_url),
            ("Sec-Fetch-Site", "same-origin"),
            ("Sec-Fetch-Mode", "cors"),
            ("Sec-Fetch-Dest", "empty"),
        ],
    )
    .await?;
    let prompt_data_text = prompt_data_res.text().await?;
    let prompt_data: DuoPromptDataResponse = serde_json::from_str(&prompt_data_text)
        .map_err(|e| anyhow::anyhow!("Step 4g: parse /prompt/data: {e}\n{}", &prompt_data_text[..prompt_data_text.len().min(400)]))?;
    if prompt_data.stat != "OK" {
        anyhow::bail!("Step 4g: /prompt/data FAIL (message_enum={:?})", prompt_data.message_enum);
    }
    let phones = &prompt_data.response.phones;
    tracing::info!(
        target: "jfc::oidc",
        phones = phones.len(),
        methods = prompt_data.response.auth_method_order.len(),
        "Step 4g: prompt/data OK"
    );

    // 4h: POST /frame/v4/prompt
    let use_passcode = matches!(opts.duo_method, DuoMethod::Passcode)
        || (opts.duo_passcode.is_some() && !matches!(opts.duo_method, DuoMethod::Push));
    let device_key = phones.first().map(|p| p.key.clone()).unwrap_or_default();
    let (factor, device, passcode): (String, String, Option<String>) = if use_passcode && opts.duo_passcode.is_some() {
        ("Passcode".into(), "null".into(), opts.duo_passcode.clone())
    } else {
        let device = phones.first().map(|p| p.index.clone()).unwrap_or_else(|| "phone1".into());
        ("Duo Push".into(), device, None)
    };

    let mut prompt_params: Vec<(&str, String)> = Vec::new();
    if let Some(p) = passcode.as_ref() {
        prompt_params.push(("passcode", p.clone()));
    }
    prompt_params.push(("device", device.clone()));
    prompt_params.push(("factor", factor.clone()));
    prompt_params.push(("postAuthDestination", "OIDC_EXIT".into()));
    prompt_params.push(("browser_features", DUO_BROWSER_FEATURES.into()));
    prompt_params.push(("sid", sid.clone()));
    let prompt_body = serde_urlencoded::to_string(&prompt_params).unwrap_or_default();

    let factor_res = request(
        client, jar, reqwest::Method::POST,
        &format!("{duo_host}/frame/v4/prompt"),
        Some(&prompt_body),
        &[
            ("Accept", "*/*"),
            ("X-Xsrftoken", xsrf_prompt.as_str()),
            ("Content-Type", "application/x-www-form-urlencoded;charset=UTF-8"),
            ("Origin", &duo_host),
            ("Referer", &prompt_url),
            ("Sec-Fetch-Site", "same-origin"),
            ("Sec-Fetch-Mode", "cors"),
            ("Sec-Fetch-Dest", "empty"),
        ],
    )
    .await?;
    let factor_text = factor_res.text().await?;
    let factor_data: DuoFactorResponse = serde_json::from_str(&factor_text).map_err(|e| {
        anyhow::anyhow!("Step 4h: parse /prompt: {e}\n{}", &factor_text[..factor_text.len().min(400)])
    })?;
    if factor_data.stat != "OK" {
        anyhow::bail!("Step 4h: /prompt FAIL (message_enum={:?})", factor_data.message_enum);
    }
    let txid = factor_data
        .response
        .ok_or_else(|| anyhow::anyhow!("Step 4h: /prompt missing txid response"))?
        .txid;

    // 4i: poll /frame/v4/status
    let deadline = std::time::Instant::now() + opts.poll_timeout;
    loop {
        if std::time::Instant::now() >= deadline {
            anyhow::bail!("Step 4i: Duo approval timed out after {:?}", opts.poll_timeout);
        }
        let status_body = serde_urlencoded::to_string([("txid", &txid), ("sid", &sid)]).unwrap_or_default();
        let res = request(
            client, jar, reqwest::Method::POST,
            &format!("{duo_host}/frame/v4/status"),
            Some(&status_body),
            &[
                ("Accept", "*/*"),
                ("X-Xsrftoken", xsrf_prompt.as_str()),
                ("Content-Type", "application/x-www-form-urlencoded;charset=UTF-8"),
                ("Origin", &duo_host),
                ("Referer", &prompt_url),
                ("Sec-Fetch-Site", "same-origin"),
                ("Sec-Fetch-Mode", "cors"),
                ("Sec-Fetch-Dest", "empty"),
            ],
        )
        .await?;
        let body_text = res.text().await?;
        let parsed: DuoStatusResponse = serde_json::from_str(&body_text).map_err(|e| {
            anyhow::anyhow!("Step 4i: parse /status: {e}\n{}", &body_text[..body_text.len().min(300)])
        })?;
        if parsed.response.result.as_deref() == Some("SUCCESS")
            || parsed.response.status_code.as_deref() == Some("allow")
        {
            tracing::info!(
                target: "jfc::oidc",
                reason = ?parsed.response.reason,
                "Step 4i: Duo approved"
            );
            break;
        }
        if parsed.response.status_code.as_deref() == Some("deny") {
            anyhow::bail!("Step 4i: Duo denied — {}", parsed.response.reason.unwrap_or_default());
        }
        tokio::time::sleep(opts.poll_interval).await;
    }

    // 4j: POST /oidc/exit → 303 → shibboleth duo-callback
    let exit_pairs = [
        ("sid", sid.as_str()),
        ("txid", txid.as_str()),
        ("factor", factor.as_str()),
        ("device_key", device_key.as_str()),
        ("_xsrf", xsrf_prompt.as_str()),
        ("dampen_choice", "true"),
    ];
    let exit_body = serde_urlencoded::to_string(exit_pairs).unwrap_or_default();
    let exit_res = request(
        client, jar, reqwest::Method::POST,
        &format!("{duo_host}/frame/v4/oidc/exit"),
        Some(&exit_body),
        &[
            ("Accept", "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8"),
            ("Content-Type", "application/x-www-form-urlencoded"),
            ("Origin", &duo_host),
            ("Referer", &prompt_url),
            ("Sec-Fetch-Site", "same-origin"),
            ("Sec-Fetch-Mode", "navigate"),
            ("Sec-Fetch-Dest", "document"),
            ("Upgrade-Insecure-Requests", "1"),
        ],
    )
    .await?;
    let exit_status = exit_res.status().as_u16();
    let exit_loc = exit_res
        .headers()
        .get(reqwest::header::LOCATION)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_owned());
    let _ = exit_res.text().await;
    let exit_loc = exit_loc.unwrap_or_default();
    if !exit_loc.contains("duo-callback") {
        anyhow::bail!("Step 4j: expected duo-callback, got status={exit_status} loc={exit_loc}");
    }
    tracing::info!(target: "jfc::oidc", "Step 4j: Duo OIDC exit → Shibboleth duo-callback");
    Ok(exit_loc)
}

async fn step5_and_6_extract_token(
    client: &reqwest::Client,
    jar: &mut CookieJar,
    duo_callback_url: &str,
    base_url: &str,
) -> anyhow::Result<OidcLoginResult> {
    tracing::info!(target: "jfc::oidc", "Step 5: Following Shibboleth post-Duo redirects");
    let host = url::Url::parse(base_url)?.host_str().unwrap_or("").to_owned();
    let mut r = follow_redirects(client, jar, duo_callback_url, reqwest::Method::GET, None, &[], 10).await?;
    tracing::info!(target: "jfc::oidc", url = %r.url, "Step 5: After duo-callback");

    for _ in 0..8 {
        if jar.get(&host, "token").is_some() {
            break;
        }
        if r.body.contains("shib_idp_ls_success") || r.body.contains("_eventId_proceed") {
            let action = extract_form_action(&r.body, &r.url).unwrap_or_else(|| r.url.clone());
            let mut hidden = extract_hidden_fields(&r.body);
            hidden.entry("_eventId_proceed".into()).or_insert_with(String::new);
            hidden
                .entry("shib_idp_ls_success.shib_idp_session_ss".into())
                .or_insert_with(|| "true".into());
            hidden
                .entry("shib_idp_ls_exception.shib_idp_session_ss".into())
                .or_insert_with(String::new);
            let body_str = url_encode(&hidden);
            r = follow_redirects(
                client, jar, &action, reqwest::Method::POST, Some(&body_str), &[], 10,
            )
            .await?;
            continue;
        }
        let next = regex::Regex::new(r#"window\.location\s*=\s*['"]([^'"]+)"#)
            .unwrap()
            .captures(&r.body)
            .and_then(|c| c.get(1).map(|m| m.as_str().to_owned()))
            .or_else(|| {
                regex::Regex::new(r#"http-equiv="refresh"\s+content="\d+;url=([^"]+)""#)
                    .unwrap()
                    .captures(&r.body)
                    .and_then(|c| c.get(1).map(|m| m.as_str().to_owned()))
            });
        if let Some(next_url) = next {
            let abs = url::Url::parse(&r.url)?.join(&next_url)?.to_string();
            r = follow_redirects(client, jar, &abs, reqwest::Method::GET, None, &[], 10).await?;
            continue;
        }
        if r.url.contains("execution=") && !r.url.contains("_eventId_proceed") {
            let sep = if r.url.contains('?') { "&" } else { "?" };
            let proceed = format!("{}{sep}_eventId_proceed=1", r.url);
            r = follow_redirects(client, jar, &proceed, reqwest::Method::GET, None, &[], 10).await?;
            continue;
        }
        break;
    }

    let token = jar
        .get(&host, "token")
        .map(str::to_owned)
        .ok_or_else(|| {
            anyhow::anyhow!(
                "Step 5: No token cookie received; ended at {}. OpenWebUI didn't complete the OIDC exchange.",
                r.url
            )
        })?;
    let oauth_id_token = jar.get(&host, "oauth_id_token").unwrap_or("").to_owned();
    let oauth_session_id = jar.get(&host, "oauth_session_id").unwrap_or("").to_owned();

    // Default to 28-day expiry when JWT can't be parsed.
    let default_expiry_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64 + 28 * 24 * 60 * 60 * 1000)
        .unwrap_or(0);
    let expires_at = token_expires_at_ms(&token).unwrap_or(default_expiry_ms);

    tracing::info!(
        target: "jfc::oidc",
        expires_at,
        "Step 5: Got token"
    );
    Ok(OidcLoginResult {
        token,
        oauth_id_token,
        oauth_session_id,
        expires_at,
    })
}

/// Run the complete 6-step OIDC + Duo login.
pub async fn oidc_login(opts: OidcLoginOptions) -> anyhow::Result<OidcLoginResult> {
    let client = build_client()?;
    let mut jar = load_cookie_jar().unwrap_or_default();
    let base_url = opts.base_url.trim_end_matches('/').to_owned();

    let shib_url = step1_initiate_oidc(&client, &mut jar, &base_url).await?;
    let step2 = step2_submit_credentials(&client, &mut jar, &shib_url, &opts.username, &opts.password).await?;

    if step2.url.contains("oauth/oidc/callback") || step2.url.contains(&format!("{base_url}/auth")) {
        tracing::info!(target: "jfc::oidc", "Session fully alive — skipped to OIDC callback");
        let result = step5_and_6_extract_token(&client, &mut jar, &step2.url, &base_url).await?;
        save_cookie_jar(&jar);
        return Ok(result);
    }

    let (duo_url, duo_body) = step3_navigate_to_duo(&client, &mut jar, &step2.url, &step2.body).await?;
    let duo_callback = step4_complete_duo(&client, &mut jar, &duo_url, &duo_body, &opts).await?;
    let result = step5_and_6_extract_token(&client, &mut jar, &duo_callback, &base_url).await?;
    save_cookie_jar(&jar);
    if step2.skipped_credentials {
        tracing::info!(target: "jfc::oidc", "Shibboleth session was reused; Duo still required");
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cookie_jar_capture_basic_normal() {
        let mut jar = CookieJar::new();
        let mut headers = reqwest::header::HeaderMap::new();
        headers.append(
            reqwest::header::SET_COOKIE,
            "token=abc123; Path=/; HttpOnly".parse().unwrap(),
        );
        jar.capture("https://chat.ai2s.org/auth", &headers);
        assert_eq!(jar.get("chat.ai2s.org", "token"), Some("abc123"));
    }

    #[test]
    fn cookie_jar_clears_null_robust() {
        let mut jar = CookieJar::new();
        let mut h1 = reqwest::header::HeaderMap::new();
        h1.append(reqwest::header::SET_COOKIE, "k=v".parse().unwrap());
        jar.capture("https://x.example.com/", &h1);
        let mut h2 = reqwest::header::HeaderMap::new();
        h2.append(reqwest::header::SET_COOKIE, "k=null".parse().unwrap());
        jar.capture("https://x.example.com/", &h2);
        assert!(jar.get("x.example.com", "k").is_none());
    }

    #[test]
    fn extract_hidden_fields_parses_basic_form_normal() {
        let html = r#"<form><input type="hidden" name="csrf" value="abc"/><input type="hidden" name="x" value=""/></form>"#;
        let fields = extract_hidden_fields(html);
        assert_eq!(fields.get("csrf").map(String::as_str), Some("abc"));
        assert_eq!(fields.get("x").map(String::as_str), Some(""));
    }

    #[test]
    fn extract_form_action_resolves_relative_normal() {
        let html = r#"<form action="login?x=1"></form>"#;
        let action = extract_form_action(html, "https://idp.example/auth/").unwrap();
        assert_eq!(action, "https://idp.example/auth/login?x=1");
    }
}
