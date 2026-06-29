//! HTTP client wrapper. Holds shared state — auth, base URL, default
//! headers — and exposes per-service handles via builder methods.

use crate::error::{Error, Result};
use crate::retry;
use reqwest::{Method, Response, StatusCode};
use std::sync::Arc;
use std::time::Duration;

mod trace;
use trace::{auth_label, trace_request_attempt, trace_request_status};

pub const DEFAULT_BASE_URL: &str = "https://api.anthropic.com";
pub const ANTHROPIC_VERSION: &str = "2023-06-01";

#[derive(Clone)]
pub struct Client {
    inner: Arc<ClientInner>,
}

struct ClientInner {
    http: reqwest::Client,
    base_url: String,
    auth: Auth,
    user_agent: String,
}

#[derive(Clone)]
enum Auth {
    ApiKey(String),
    Bearer(String),
}

impl Client {
    pub fn with_api_key(api_key: impl Into<String>) -> Self {
        Self::build(Auth::ApiKey(api_key.into()))
    }

    pub fn with_bearer(token: impl Into<String>) -> Self {
        Self::build(Auth::Bearer(token.into()))
    }

    fn build(auth: Auth) -> Self {
        linkscope::record_items("sdk.client.build", 1);
        if linkscope::trace_detail_enabled() {
            linkscope::detail_event_fields(
                "sdk.client.build.detail",
                [linkscope::TraceField::text("auth", auth_label(&auth))],
            );
        }
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(60))
            .build()
            .expect("reqwest client builder always succeeds with default config");
        Self {
            inner: Arc::new(ClientInner {
                http,
                base_url: DEFAULT_BASE_URL.to_owned(),
                auth,
                user_agent: format!("jfc-anthropic-sdk/{}", env!("CARGO_PKG_VERSION")),
            }),
        }
    }

    pub fn with_base_url(self, base_url: impl Into<String>) -> Self {
        let mut url = base_url.into();
        while url.ends_with('/') {
            url.pop();
        }
        Self {
            inner: Arc::new(ClientInner {
                http: self.inner.http.clone(),
                base_url: url,
                auth: self.inner.auth.clone(),
                user_agent: self.inner.user_agent.clone(),
            }),
        }
    }

    pub(crate) fn base_url(&self) -> &str {
        &self.inner.base_url
    }

    /// Build a request with auth + standard headers applied. Adds `path` to
    /// `base_url`. The optional `beta` value is set as `anthropic-beta`.
    pub(crate) fn request(
        &self,
        method: Method,
        path: &str,
        beta: Option<&str>,
    ) -> reqwest::RequestBuilder {
        if linkscope::is_enabled() {
            linkscope::detail_event_fields(
                "sdk.request.build",
                [
                    linkscope::TraceField::text("method", method.as_str()),
                    linkscope::TraceField::text("path", path),
                    linkscope::TraceField::count("has_beta", bool_to_u64(beta.is_some())),
                ],
            );
        }
        let url = format!("{}{}", self.inner.base_url, path);
        self.request_url(method, url, beta)
    }

    pub(crate) fn request_url(
        &self,
        method: Method,
        url: impl reqwest::IntoUrl,
        beta: Option<&str>,
    ) -> reqwest::RequestBuilder {
        if linkscope::trace_detail_enabled() {
            linkscope::detail_event_fields(
                "sdk.request.url",
                [
                    linkscope::TraceField::text("method", method.as_str()),
                    linkscope::TraceField::text("auth", auth_label(&self.inner.auth)),
                    linkscope::TraceField::count("has_beta", bool_to_u64(beta.is_some())),
                ],
            );
        }
        let mut req = self
            .inner
            .http
            .request(method, url)
            .header("anthropic-version", ANTHROPIC_VERSION)
            .header("user-agent", &self.inner.user_agent);
        match &self.inner.auth {
            Auth::ApiKey(k) => req = req.header("x-api-key", k),
            Auth::Bearer(t) => req = req.header("authorization", format!("Bearer {t}")),
        }
        if let Some(b) = beta {
            req = req.header("anthropic-beta", b);
        }
        req
    }

    /// Execute a request with the SDK's retry policy. Returns the raw
    /// response — callers parse the body. Failures past the retry budget
    /// surface as `Error::Api` with the final status + message.
    pub(crate) async fn execute_with_retry(
        &self,
        build: impl Fn() -> reqwest::RequestBuilder,
    ) -> Result<Response> {
        let _linkscope_retry = linkscope::phase("sdk.execute_with_retry");
        let mut attempt: u32 = 0;
        loop {
            linkscope::record_items("sdk.request.attempt", 1);
            let req = build();
            trace_request_attempt("sdk.request.send.start", attempt);
            let resp_result = {
                let _linkscope_send = linkscope::phase("sdk.request.send");
                req.send().await
            };
            match resp_result {
                Ok(resp) if resp.status().is_success() => {
                    trace_request_status("sdk.request.send.ok", attempt, resp.status());
                    if attempt > 0 {
                        linkscope::record_items("sdk.request.succeeded_after_retry", 1);
                    }
                    return Ok(resp);
                }
                Ok(resp) => {
                    let status = resp.status();
                    trace_request_status("sdk.request.send.status", attempt, status);
                    if linkscope::is_enabled() {
                        linkscope::detail_event_fields(
                            "sdk.request.status",
                            [
                                linkscope::TraceField::count("attempt", u64::from(attempt + 1)),
                                linkscope::TraceField::count("status", u64::from(status.as_u16())),
                                linkscope::TraceField::count(
                                    "will_retry",
                                    bool_to_u64(
                                        attempt + 1 < retry::MAX_ATTEMPTS
                                            && retry::should_retry_status(status.as_u16()),
                                    ),
                                ),
                            ],
                        );
                    }
                    if attempt + 1 >= retry::MAX_ATTEMPTS
                        || !retry::should_retry_status(status.as_u16())
                    {
                        linkscope::record_items("sdk.request.api_error", 1);
                        return Err(into_api_error(resp).await);
                    }
                    linkscope::record_items("sdk.request.retry_status", 1);
                    let delay = retry::parse_retry_after(resp.headers())
                        .unwrap_or_else(|| retry::delay_for(attempt));
                    tracing::debug!(
                        target: "jfc_anthropic_sdk::retry",
                        attempt,
                        status = %status,
                        wait_ms = delay.as_millis() as u64,
                        "retrying after API error"
                    );
                    tokio::time::sleep(delay).await;
                    attempt += 1;
                }
                Err(e) => {
                    trace_request_attempt("sdk.request.send.transport_error", attempt);
                    if attempt + 1 >= retry::MAX_ATTEMPTS {
                        linkscope::record_items("sdk.request.transport_error", 1);
                        return Err(Error::Transport(e));
                    }
                    linkscope::record_items("sdk.request.retry_transport_error", 1);
                    let delay = retry::delay_for(attempt);
                    tracing::debug!(
                        target: "jfc_anthropic_sdk::retry",
                        attempt,
                        error = %e,
                        wait_ms = delay.as_millis() as u64,
                        "retrying after transport error"
                    );
                    tokio::time::sleep(delay).await;
                    attempt += 1;
                }
            }
        }
    }
}

fn bool_to_u64(value: bool) -> u64 {
    if value { 1 } else { 0 }
}

pub(crate) async fn into_api_error(resp: Response) -> Error {
    let status = resp.status();
    let request_id = resp
        .headers()
        .get("request-id")
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);
    let body_text = resp.text().await.unwrap_or_default();
    let body_json = serde_json::from_str::<serde_json::Value>(&body_text).ok();
    let message = body_json
        .as_ref()
        .and_then(|v| v.get("error"))
        .and_then(|e| e.get("message"))
        .and_then(|m| m.as_str())
        .unwrap_or(&body_text)
        .to_owned();
    if status == StatusCode::UNAUTHORIZED || status == StatusCode::FORBIDDEN {
        return Error::Authentication(message);
    }
    Error::Api {
        status,
        message,
        request_id,
        body: body_json,
    }
}
