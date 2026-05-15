use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::provider::{
    CompletionResponse, EventStream, ModelId, ModelInfo, Provider, ProviderId, ProviderMessage,
    StreamConvention, StreamOptions,
};

use super::oauth_core::{
    AuthMethod, TokenStore, generate_pkce, generate_state, jwt_claim, now_secs,
};
use super::openai;

const PROVIDER_ID: &str = "codex";
const CLIENT_ID: &str = "app_EMoamEEZ73f0CkXaXp7hrann";
const ISSUER: &str = "https://auth.openai.com";
const CODEX_API_ENDPOINT: &str = "https://chatgpt.com/backend-api/codex/responses";
const CODEX_DEVICE_URL: &str = "https://auth.openai.com/codex/device";
const CODEX_ORIGINATOR: &str = "opencode";
const DEFAULT_EXPIRES_IN: u64 = 3600;

#[derive(Clone)]
pub struct CodexOAuthProvider {
    client: reqwest::Client,
    store: TokenStore,
}

#[derive(Debug, Clone)]
pub struct CodexAuthorizeRequest {
    pub url: String,
    pub verifier: String,
    pub state: String,
    pub redirect_uri: String,
}

#[derive(Debug, Clone)]
pub struct CodexDeviceCode {
    pub verification_url: String,
    pub user_code: String,
    pub device_auth_id: String,
    pub interval: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CodexTokenSet {
    pub access_token: String,
    pub refresh_token: String,
    pub expires_at: u64,
    pub account_id: Option<String>,
}

impl CodexOAuthProvider {
    pub fn new() -> Self {
        Self {
            client: super::http::streaming_client(),
            store: TokenStore::default_path().into(),
        }
    }

    pub fn with_store(path: impl Into<std::path::PathBuf>) -> Self {
        Self {
            client: super::http::streaming_client(),
            store: TokenStore::new(path),
        }
    }

    pub fn has_usable_config(&self) -> bool {
        matches!(
            self.store.get(PROVIDER_ID),
            Ok(Some(AuthMethod::OAuth { .. }))
        )
    }

    pub fn store_path(&self) -> &std::path::Path {
        self.store.path()
    }

    pub fn authorize_url(redirect_uri: &str) -> CodexAuthorizeRequest {
        let pkce = generate_pkce();
        let state = generate_state();
        let scope = "openid profile email offline_access";
        let url = format!(
            "{ISSUER}/oauth/authorize?response_type=code&client_id={}&redirect_uri={}&scope={}&code_challenge={}&code_challenge_method=S256&state={}&originator={}",
            url_encode(CLIENT_ID),
            url_encode(redirect_uri),
            url_encode(scope),
            url_encode(&pkce.code_challenge),
            url_encode(&state),
            url_encode(CODEX_ORIGINATOR),
        );
        CodexAuthorizeRequest {
            url,
            verifier: pkce.code_verifier,
            state,
            redirect_uri: redirect_uri.to_owned(),
        }
    }

    pub async fn request_device_code(&self) -> anyhow::Result<CodexDeviceCode> {
        #[derive(Serialize)]
        struct Req<'a> {
            client_id: &'a str,
        }
        #[derive(Deserialize)]
        struct Resp {
            device_auth_id: String,
            #[serde(alias = "usercode")]
            user_code: String,
            #[serde(default, deserialize_with = "deserialize_interval")]
            interval: u64,
        }

        let resp = self
            .client
            .post(format!("{ISSUER}/api/accounts/deviceauth/usercode"))
            .json(&Req {
                client_id: CLIENT_ID,
            })
            .send()
            .await?
            .error_for_status()?;
        let body: Resp = resp.json().await?;
        Ok(CodexDeviceCode {
            verification_url: CODEX_DEVICE_URL.to_owned(),
            user_code: body.user_code,
            device_auth_id: body.device_auth_id,
            interval: body.interval.max(1),
        })
    }

    pub async fn poll_device_code(
        &self,
        device: &CodexDeviceCode,
    ) -> anyhow::Result<CodexTokenSet> {
        #[derive(Serialize)]
        struct PollReq<'a> {
            device_auth_id: &'a str,
            user_code: &'a str,
        }
        #[derive(Deserialize)]
        struct CodeResp {
            authorization_code: String,
            code_verifier: String,
        }

        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15 * 60);
        loop {
            let resp = self
                .client
                .post(format!("{ISSUER}/api/accounts/deviceauth/token"))
                .json(&PollReq {
                    device_auth_id: &device.device_auth_id,
                    user_code: &device.user_code,
                })
                .send()
                .await?;
            if resp.status().is_success() {
                let code: CodeResp = resp.json().await?;
                return self
                    .exchange_code_with_verifier(
                        &code.authorization_code,
                        &code.code_verifier,
                        &format!("{ISSUER}/deviceauth/callback"),
                    )
                    .await;
            }
            if std::time::Instant::now() >= deadline {
                anyhow::bail!("device auth timed out after 15 minutes");
            }
            tokio::time::sleep(std::time::Duration::from_secs(device.interval.max(1))).await;
        }
    }

    pub async fn exchange_code(
        &self,
        code: &str,
        req: &CodexAuthorizeRequest,
    ) -> anyhow::Result<CodexTokenSet> {
        self.exchange_code_with_verifier(code, &req.verifier, &req.redirect_uri)
            .await
    }

    async fn exchange_code_with_verifier(
        &self,
        code: &str,
        verifier: &str,
        redirect_uri: &str,
    ) -> anyhow::Result<CodexTokenSet> {
        #[derive(Deserialize)]
        struct TokenResp {
            access_token: String,
            refresh_token: String,
            #[serde(default)]
            expires_in: Option<u64>,
            #[serde(default)]
            id_token: Option<String>,
        }
        let body = form_body(&[
            ("grant_type", "authorization_code"),
            ("code", code),
            ("redirect_uri", redirect_uri),
            ("client_id", CLIENT_ID),
            ("code_verifier", verifier),
        ]);
        let resp = self
            .client
            .post(format!("{ISSUER}/oauth/token"))
            .header("Content-Type", "application/x-www-form-urlencoded")
            .body(body)
            .send()
            .await?
            .error_for_status()?;
        let body: TokenResp = resp.json().await?;
        let account_id = body
            .id_token
            .as_deref()
            .and_then(extract_account_id_from_jwt)
            .or_else(|| extract_account_id_from_jwt(&body.access_token));
        let tokens = CodexTokenSet {
            access_token: body.access_token,
            refresh_token: body.refresh_token,
            expires_at: now_secs() + body.expires_in.unwrap_or(DEFAULT_EXPIRES_IN),
            account_id,
        };
        self.persist_tokens(&tokens)?;
        Ok(tokens)
    }

    pub async fn refresh_tokens(&self, refresh_token: &str) -> anyhow::Result<CodexTokenSet> {
        #[derive(Deserialize)]
        struct TokenResp {
            access_token: String,
            #[serde(default)]
            refresh_token: Option<String>,
            #[serde(default)]
            expires_in: Option<u64>,
            #[serde(default)]
            id_token: Option<String>,
        }
        let body = form_body(&[
            ("grant_type", "refresh_token"),
            ("refresh_token", refresh_token),
            ("client_id", CLIENT_ID),
        ]);
        let resp = self
            .client
            .post(format!("{ISSUER}/oauth/token"))
            .header("Content-Type", "application/x-www-form-urlencoded")
            .body(body)
            .send()
            .await?
            .error_for_status()?;
        let body: TokenResp = resp.json().await?;
        let account_id = body
            .id_token
            .as_deref()
            .and_then(extract_account_id_from_jwt)
            .or_else(|| extract_account_id_from_jwt(&body.access_token));
        let tokens = CodexTokenSet {
            access_token: body.access_token,
            refresh_token: body
                .refresh_token
                .unwrap_or_else(|| refresh_token.to_owned()),
            expires_at: now_secs() + body.expires_in.unwrap_or(DEFAULT_EXPIRES_IN),
            account_id,
        };
        self.persist_tokens(&tokens)?;
        Ok(tokens)
    }

    fn persist_tokens(&self, tokens: &CodexTokenSet) -> std::io::Result<()> {
        self.store.set(
            PROVIDER_ID,
            AuthMethod::OAuth {
                access_token: tokens.access_token.clone(),
                refresh_token: tokens.refresh_token.clone(),
                expires_at: tokens.expires_at,
                account_id: tokens.account_id.clone(),
            },
        )
    }

    async fn ensure_tokens(&self) -> anyhow::Result<CodexTokenSet> {
        match self.store.get(PROVIDER_ID)? {
            Some(AuthMethod::OAuth {
                access_token,
                refresh_token,
                expires_at,
                account_id,
            }) => {
                if (AuthMethod::OAuth {
                    access_token: access_token.clone(),
                    refresh_token: refresh_token.clone(),
                    expires_at,
                    account_id: account_id.clone(),
                })
                .is_expired_or_expiring(now_secs())
                {
                    self.refresh_tokens(&refresh_token).await
                } else {
                    Ok(CodexTokenSet {
                        access_token,
                        refresh_token,
                        expires_at,
                        account_id,
                    })
                }
            }
            _ => anyhow::bail!(
                "Codex OAuth is not configured. Run `jfc auth codex login` or `jfc auth codex device`."
            ),
        }
    }

    fn apply_codex_headers(
        builder: reqwest::RequestBuilder,
        tokens: &CodexTokenSet,
    ) -> reqwest::RequestBuilder {
        let builder = builder
            .bearer_auth(&tokens.access_token)
            .header("originator", CODEX_ORIGINATOR)
            .header("User-Agent", "jfc-codex-oauth");
        if let Some(account_id) = tokens.account_id.as_deref() {
            builder.header("ChatGPT-Account-Id", account_id)
        } else {
            builder
        }
    }

    pub fn codex_models() -> Vec<ModelInfo> {
        [
            "gpt-5.1-codex",
            "gpt-5.1-codex-max",
            "gpt-5.1-codex-mini",
            "gpt-5.2-codex",
            "gpt-5.3-codex",
            "gpt-5.4",
            "gpt-5.4-mini",
        ]
        .into_iter()
        .map(|id| {
            ModelInfo::new(ModelId::new(id), id, ProviderId::new(PROVIDER_ID))
                .with_context_window_tokens(Some(400_000))
                .with_max_output_tokens(Some(128_000))
                .with_costs(Some(0.0), Some(0.0))
        })
        .collect()
    }
}

impl crate::provider::seal::Sealed for CodexOAuthProvider {}

#[async_trait]
impl Provider for CodexOAuthProvider {
    fn name(&self) -> &str {
        PROVIDER_ID
    }

    fn available_models(&self) -> Vec<ModelInfo> {
        Self::codex_models()
    }

    fn stream_convention(&self) -> StreamConvention {
        StreamConvention::OpenAiNative
    }

    async fn fetch_models(&self) -> anyhow::Result<Vec<ModelInfo>> {
        Ok(Self::codex_models())
    }

    async fn ensure_auth(&self) -> anyhow::Result<()> {
        let _ = self.ensure_tokens().await?;
        Ok(())
    }

    fn auth_headers(&self) -> reqwest::header::HeaderMap {
        use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
        let mut headers = HeaderMap::new();
        if let Ok(Some(AuthMethod::OAuth {
            access_token,
            account_id,
            ..
        })) = self.store.get(PROVIDER_ID)
        {
            if let Ok(value) = HeaderValue::from_str(&format!("Bearer {access_token}")) {
                headers.insert(reqwest::header::AUTHORIZATION, value);
            }
            if let Some(account_id) = account_id {
                if let Ok(value) = HeaderValue::from_str(&account_id) {
                    headers.insert(HeaderName::from_static("chatgpt-account-id"), value);
                }
            }
            headers.insert(
                HeaderName::from_static("originator"),
                HeaderValue::from_static(CODEX_ORIGINATOR),
            );
        }
        headers
    }

    fn rewrite_url(&self, original: &str) -> Option<String> {
        if original.ends_with("/responses") || original.ends_with("/chat/completions") {
            Some(CODEX_API_ENDPOINT.to_owned())
        } else {
            None
        }
    }

    async fn stream(
        &self,
        messages: Vec<ProviderMessage>,
        options: &StreamOptions,
    ) -> anyhow::Result<EventStream> {
        let tokens = self.ensure_tokens().await?;
        let body = openai::build_responses_body(messages, options, true);
        let resp =
            Self::apply_codex_headers(self.client.post(CODEX_API_ENDPOINT).json(&body), &tokens)
                .send()
                .await?;
        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            anyhow::bail!("Codex API error {status}: {text}");
        }
        Ok(openai::responses_event_stream(resp))
    }

    async fn complete(
        &self,
        messages: Vec<ProviderMessage>,
        options: &StreamOptions,
    ) -> anyhow::Result<CompletionResponse> {
        let tokens = self.ensure_tokens().await?;
        let resp = Self::apply_codex_headers(
            self.client
                .post(CODEX_API_ENDPOINT)
                .json(&openai::build_responses_body(messages, options, false)),
            &tokens,
        )
        .send()
        .await?
        .error_for_status()?;
        let body: Value = resp.json().await?;
        Ok(CompletionResponse {
            content: openai::response_output_text(&body),
            usage: openai::response_usage(&body).unwrap_or_default(),
        })
    }
}

fn deserialize_interval<'de, D>(deserializer: D) -> Result<u64, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = serde_json::Value::deserialize(deserializer)?;
    match value {
        serde_json::Value::Number(n) => Ok(n.as_u64().unwrap_or(5)),
        serde_json::Value::String(s) => s.parse::<u64>().map_err(serde::de::Error::custom),
        _ => Ok(5),
    }
}

pub fn extract_account_id_from_jwt(jwt: &str) -> Option<String> {
    jwt_claim(
        jwt,
        &[
            "chatgpt_account_id",
            "https://api.openai.com/auth.chatgpt_account_id",
            "account_id",
        ],
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine;

    fn jwt_with_claims(claims: serde_json::Value) -> String {
        let header = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(br#"{"alg":"none"}"#);
        let payload = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(serde_json::to_vec(&claims).unwrap());
        format!("{header}.{payload}.")
    }

    #[test]
    fn extracts_chatgpt_account_id_from_jwt() {
        let jwt = jwt_with_claims(serde_json::json!({"chatgpt_account_id":"acct-123"}));
        assert_eq!(
            extract_account_id_from_jwt(&jwt).as_deref(),
            Some("acct-123")
        );
    }

    #[test]
    fn authorize_url_contains_codex_oauth_params() {
        let req = CodexOAuthProvider::authorize_url("http://localhost:1455/auth/callback");
        assert!(
            req.url
                .starts_with("https://auth.openai.com/oauth/authorize?")
        );
        assert!(req.url.contains("client_id=app_EMoamEEZ73f0CkXaXp7hrann"));
        assert!(req.url.contains("code_challenge="));
        assert!(req.url.contains("originator=opencode"));
    }

    #[test]
    fn codex_models_are_zero_cost_unknowns() {
        let models = CodexOAuthProvider::codex_models();
        assert!(models.iter().any(|m| m.id.as_str() == "gpt-5.1-codex"));
        let usage = crate::types::ModelUsage {
            input_tokens: 1_000_000,
            output_tokens: 1_000_000,
            ..Default::default()
        };
        assert_eq!(crate::cost::cost_for("codex/gpt-5.1-codex", &usage), 0.0);
    }
}

fn form_body(params: &[(&str, &str)]) -> String {
    params
        .iter()
        .map(|(k, v)| format!("{}={}", url_encode(k), url_encode(v)))
        .collect::<Vec<_>>()
        .join("&")
}

fn url_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + s.len() / 4);
    for b in s.as_bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(*b as char)
            }
            _ => out.push_str(&format!("%{:02X}", b)),
        }
    }
    out
}
