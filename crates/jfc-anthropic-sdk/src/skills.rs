//! `BetaSkillService` + version management. Skills are uploaded as
//! multipart/form-data; each version is immutable.
//!
//! Endpoints (`anthropic-beta: skills-2025-10-02`):
//! - `POST /v1/beta/skills` — create (multipart)
//! - `GET /v1/beta/skills` — list
//! - `GET /v1/beta/skills/{id}` — retrieve
//! - `DELETE /v1/beta/skills/{id}` — delete
//! - `POST /v1/beta/skills/{id}/versions` — new version (multipart)
//! - `GET /v1/beta/skills/{id}/versions` — list versions

use crate::beta;
use crate::client::Client;
use crate::error::Result;
use reqwest::Method;
use reqwest::multipart::{Form, Part};
use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
pub struct Skill {
    pub id: String,
    pub display_title: String,
    pub created_at: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SkillVersion {
    pub id: String,
    pub skill_id: String,
    pub version_number: u32,
    pub created_at: String,
}

pub struct SkillService {
    client: Client,
}

impl SkillService {
    pub fn new(client: Client) -> Self {
        Self { client }
    }

    pub async fn list(&self) -> Result<Vec<Skill>> {
        let resp = self
            .client
            .execute_with_retry(|| {
                self.client
                    .request(Method::GET, "/v1/beta/skills", Some(beta::SKILLS))
            })
            .await?;
        let body: serde_json::Value = resp.json().await?;
        let data = body
            .get("data")
            .cloned()
            .unwrap_or(serde_json::Value::Array(Vec::new()));
        Ok(serde_json::from_value(data)?)
    }

    pub async fn get(&self, skill_id: &str) -> Result<Skill> {
        let path = format!("/v1/beta/skills/{skill_id}");
        let resp = self
            .client
            .execute_with_retry(|| self.client.request(Method::GET, &path, Some(beta::SKILLS)))
            .await?;
        Ok(resp.json().await?)
    }

    pub async fn delete(&self, skill_id: &str) -> Result<()> {
        let path = format!("/v1/beta/skills/{skill_id}");
        self.client
            .execute_with_retry(|| {
                self.client
                    .request(Method::DELETE, &path, Some(beta::SKILLS))
            })
            .await?;
        Ok(())
    }

    /// Create a new skill. v132's `BetaSkillService.New` mirror —
    /// multipart upload with a `display_title` text field plus one
    /// `files` Part per attached file. Returns the freshly-minted
    /// Skill record.
    pub async fn create(
        &self,
        display_title: &str,
        files: Vec<SkillFile>,
    ) -> Result<Skill> {
        let resp = self
            .client
            .http()
            .request(Method::POST, format!("{}/v1/beta/skills", self.client.base_url()))
            .header("anthropic-beta", beta::SKILLS)
            .multipart(build_skill_form(display_title, files)?)
            .send()
            .await?;
        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(crate::error::Error::Api {
                status,
                message: body,
                request_id: None,
                body: None,
            });
        }
        Ok(resp.json().await?)
    }

    /// Upload a new version of an existing skill.
    pub async fn create_version(
        &self,
        skill_id: &str,
        files: Vec<SkillFile>,
    ) -> Result<SkillVersion> {
        let url = format!(
            "{}/v1/beta/skills/{skill_id}/versions",
            self.client.base_url()
        );
        let resp = self
            .client
            .http()
            .request(Method::POST, url)
            .header("anthropic-beta", beta::SKILLS)
            .multipart(build_skill_form(skill_id, files)?)
            .send()
            .await?;
        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(crate::error::Error::Api {
                status,
                message: body,
                request_id: None,
                body: None,
            });
        }
        Ok(resp.json().await?)
    }
}

/// One file attached to a skill upload. `bytes` is the in-memory body;
/// callers reading from disk should `tokio::fs::read` first.
pub struct SkillFile {
    pub filename: String,
    pub mime_type: String,
    pub bytes: Vec<u8>,
}

fn build_skill_form(label: &str, files: Vec<SkillFile>) -> Result<Form> {
    let mut form = Form::new().text("display_title", label.to_owned());
    for f in files {
        let part = Part::bytes(f.bytes)
            .file_name(f.filename.clone())
            .mime_str(&f.mime_type)
            .map_err(|e| crate::error::Error::Other(format!("multipart mime: {e}")))?;
        form = form.part("files", part);
    }
    Ok(form)
}

/// Anthropic's built-in skills, identified by name. Reference these by
/// name in `agents::SkillRef::Anthropic { skill_id: BUILTIN_WEB_SEARCH, .. }`.
pub mod builtin {
    pub const WEB_SEARCH: &str = "web_search";
    pub const COMPUTER: &str = "computer";
}
