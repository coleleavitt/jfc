use reqwest::header::{AUTHORIZATION, HeaderMap, HeaderValue};

use crate::model::{
    BridgeRequest, BridgeResponse, DeliveryAckRequest, EventList, EventUploadRequest,
    HeartbeatRequest, SessionRecord, WorkerRecord, WorkerUpdateRequest,
};

#[derive(Clone)]
pub struct BridgeClient {
    base_url: String,
    http: reqwest::Client,
    bearer: Option<String>,
}

impl BridgeClient {
    pub fn new(base_url: impl Into<String>) -> Self {
        Self {
            base_url: base_url.into().trim_end_matches('/').to_owned(),
            http: reqwest::Client::new(),
            bearer: None,
        }
    }

    pub fn with_bearer(mut self, bearer: impl Into<String>) -> Self {
        self.bearer = Some(bearer.into());
        self
    }

    pub async fn bridge(&self, req: BridgeRequest) -> anyhow::Result<BridgeResponse> {
        self.post_json("/bridge", &req).await
    }

    pub async fn get_worker(&self) -> anyhow::Result<WorkerRecord> {
        self.get_json("/worker").await
    }

    pub async fn update_worker(&self, req: WorkerUpdateRequest) -> anyhow::Result<WorkerRecord> {
        self.put_json("/worker", &req).await
    }

    pub async fn heartbeat(&self, req: HeartbeatRequest) -> anyhow::Result<WorkerRecord> {
        self.post_json("/worker/heartbeat", &req).await
    }

    pub async fn post_events(&self, req: EventUploadRequest) -> anyhow::Result<EventList> {
        self.post_json("/worker/events", &req).await
    }

    pub async fn post_internal_events(&self, req: EventUploadRequest) -> anyhow::Result<EventList> {
        self.post_json("/worker/internal-events", &req).await
    }

    pub async fn get_internal_events(&self, after: Option<&str>) -> anyhow::Result<EventList> {
        let path = match after {
            Some(after) => format!("/worker/internal-events?after={after}"),
            None => "/worker/internal-events".to_owned(),
        };
        self.get_json(&path).await
    }

    pub async fn ack_delivery(
        &self,
        req: DeliveryAckRequest,
    ) -> anyhow::Result<crate::model::BridgeEvent> {
        self.post_json("/worker/events/delivery", &req).await
    }

    pub async fn get_session(&self, session_id: &str) -> anyhow::Result<SessionRecord> {
        self.get_json(&format!("/sessions/{session_id}")).await
    }

    fn headers(&self) -> anyhow::Result<HeaderMap> {
        let mut headers = HeaderMap::new();
        if let Some(token) = self.bearer.as_deref() {
            headers.insert(
                AUTHORIZATION,
                HeaderValue::from_str(&format!("Bearer {token}"))?,
            );
        }
        Ok(headers)
    }

    async fn get_json<T>(&self, path: &str) -> anyhow::Result<T>
    where
        T: serde::de::DeserializeOwned,
    {
        let resp = self
            .http
            .get(format!("{}{}", self.base_url, path))
            .headers(self.headers()?)
            .send()
            .await?;
        decode(resp).await
    }

    async fn post_json<T, B>(&self, path: &str, body: &B) -> anyhow::Result<T>
    where
        T: serde::de::DeserializeOwned,
        B: serde::Serialize + ?Sized,
    {
        let resp = self
            .http
            .post(format!("{}{}", self.base_url, path))
            .headers(self.headers()?)
            .json(body)
            .send()
            .await?;
        decode(resp).await
    }

    async fn put_json<T, B>(&self, path: &str, body: &B) -> anyhow::Result<T>
    where
        T: serde::de::DeserializeOwned,
        B: serde::Serialize + ?Sized,
    {
        let resp = self
            .http
            .put(format!("{}{}", self.base_url, path))
            .headers(self.headers()?)
            .json(body)
            .send()
            .await?;
        decode(resp).await
    }
}

async fn decode<T>(resp: reqwest::Response) -> anyhow::Result<T>
where
    T: serde::de::DeserializeOwned,
{
    let status = resp.status();
    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        anyhow::bail!("bridge request failed ({status}): {body}");
    }
    Ok(resp.json::<T>().await?)
}
