use std::time::{Duration, Instant};

use serde::Deserialize;
use tokio::sync::mpsc;

use crate::runtime::{AppEvent, ProviderEvent};

const SUMMARY_URL: &str = "https://status.claude.com/api/v2/summary.json";
const POLL_INTERVAL: Duration = Duration::from_secs(60);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(8);

#[derive(Debug, Clone)]
pub struct ClaudeStatusSnapshot {
    pub indicator: String,
    pub description: String,
    pub components: Vec<ClaudeComponentStatus>,
    pub incidents: Vec<ClaudeIncidentStatus>,
    pub fetched_at: Instant,
    pub bytes_in: u64,
    pub bytes_out: u64,
}

#[derive(Debug, Clone)]
pub struct ClaudeComponentStatus {
    pub name: String,
    pub status: String,
}

#[derive(Debug, Clone)]
pub struct ClaudeIncidentStatus {
    pub name: String,
    pub status: String,
    pub impact: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ClaudeStatusUpdate {
    pub snapshot: Option<ClaudeStatusSnapshot>,
    pub error: Option<String>,
}

impl ClaudeStatusSnapshot {
    pub fn is_degraded(&self) -> bool {
        self.indicator != "none" || self.components.iter().any(|c| c.status != "operational")
    }

    pub fn anthropic_api_or_code_degraded(&self) -> bool {
        self.components.iter().any(|component| {
            is_api_or_code_component(component)
                && !matches!(component.status.as_str(), "operational" | "")
        })
    }

    pub fn short_badge(&self) -> String {
        if let Some(component) = self
            .components
            .iter()
            .find(|c| is_api_or_code_component(c) && c.status != "operational")
        {
            format!(
                "status {} {}",
                concise_component_name(&component.name),
                component.status.replace('_', " ")
            )
        } else if self.is_degraded() {
            format!("status {}", self.description.to_ascii_lowercase())
        } else {
            "status ok".to_owned()
        }
    }

    pub fn outage_context(&self) -> Option<String> {
        if !self.anthropic_api_or_code_degraded() && self.indicator == "none" {
            return None;
        }

        let incident = self
            .incidents
            .iter()
            .find(|incident| incident.status != "resolved")
            .map(|incident| match incident.impact.as_deref() {
                Some(impact) if !impact.is_empty() => format!("{impact} · {}", incident.name),
                _ => incident.name.clone(),
            });
        let component = self
            .components
            .iter()
            .find(|c| is_api_or_code_component(c) && c.status != "operational");

        match (component, incident) {
            (Some(component), Some(incident)) => Some(format!(
                "{} {} · {}",
                concise_component_name(&component.name),
                component.status.replace('_', " "),
                incident
            )),
            (Some(component), None) => Some(format!(
                "{} {}",
                concise_component_name(&component.name),
                component.status.replace('_', " ")
            )),
            (None, Some(incident)) => Some(incident),
            (None, None) if self.indicator != "none" => Some(self.description.clone()),
            (None, None) => None,
        }
    }

    pub fn age_secs(&self) -> u64 {
        self.fetched_at.elapsed().as_secs()
    }
}

pub fn spawn_status_poll(tx: mpsc::Sender<AppEvent>) {
    if matches!(
        std::env::var("JFC_DISABLE_CLAUDE_STATUS").as_deref(),
        Ok("1") | Ok("true")
    ) {
        return;
    }

    tokio::spawn(async move {
        let client = match reqwest::Client::builder()
            .timeout(REQUEST_TIMEOUT)
            .user_agent("jfc-ui claude-status heartbeat")
            .build()
        {
            Ok(client) => client,
            Err(err) => {
                let _ = tx
                    .send(AppEvent::Provider(ProviderEvent::ClaudeStatusUpdated(
                        ClaudeStatusUpdate {
                            snapshot: None,
                            error: Some(format!("status client: {err}")),
                        },
                    )))
                    .await;
                return;
            }
        };

        loop {
            let update = match fetch_summary(&client).await {
                Ok(snapshot) => ClaudeStatusUpdate {
                    snapshot: Some(snapshot),
                    error: None,
                },
                Err(err) => ClaudeStatusUpdate {
                    snapshot: None,
                    error: Some(err.to_string()),
                },
            };

            if tx
                .send(AppEvent::Provider(ProviderEvent::ClaudeStatusUpdated(
                    update,
                )))
                .await
                .is_err()
            {
                break;
            }

            tokio::time::sleep(POLL_INTERVAL).await;
        }
    });
}

async fn fetch_summary(client: &reqwest::Client) -> anyhow::Result<ClaudeStatusSnapshot> {
    let response = client.get(SUMMARY_URL).send().await?;
    let status = response.status();
    let bytes = response.bytes().await?;
    if !status.is_success() {
        anyhow::bail!("status.claude.com returned HTTP {status}");
    }

    let bytes_in = bytes.len() as u64;
    let summary: SummaryResponse = serde_json::from_slice(&bytes)?;
    Ok(summary.into_snapshot(bytes_in, estimate_outbound_bytes(SUMMARY_URL)))
}

fn estimate_outbound_bytes(url: &str) -> u64 {
    // Approximate one small HTTPS GET: request line + Host/User-Agent/Accept headers.
    url.len() as u64 + 96
}

fn is_api_or_code_component(component: &ClaudeComponentStatus) -> bool {
    let name = component.name.to_ascii_lowercase();
    name.contains("claude api") || name.contains("claude code")
}

fn concise_component_name(name: &str) -> &'static str {
    let lower = name.to_ascii_lowercase();
    if lower.contains("claude code") {
        "Code"
    } else if lower.contains("claude api") {
        "API"
    } else {
        "Claude"
    }
}

#[derive(Debug, Deserialize)]
struct SummaryResponse {
    status: SummaryStatus,
    #[serde(default)]
    components: Vec<Component>,
    #[serde(default)]
    incidents: Vec<Incident>,
}

impl SummaryResponse {
    fn into_snapshot(self, bytes_in: u64, bytes_out: u64) -> ClaudeStatusSnapshot {
        ClaudeStatusSnapshot {
            indicator: self.status.indicator,
            description: self.status.description,
            components: self
                .components
                .into_iter()
                .map(|component| ClaudeComponentStatus {
                    name: component.name,
                    status: component.status,
                })
                .collect(),
            incidents: self
                .incidents
                .into_iter()
                .map(|incident| ClaudeIncidentStatus {
                    name: incident.name,
                    status: incident.status,
                    impact: incident.impact,
                })
                .collect(),
            fetched_at: Instant::now(),
            bytes_in,
            bytes_out,
        }
    }
}

#[derive(Debug, Deserialize)]
struct SummaryStatus {
    indicator: String,
    description: String,
}

#[derive(Debug, Deserialize)]
struct Component {
    name: String,
    status: String,
}

#[derive(Debug, Deserialize)]
struct Incident {
    name: String,
    status: String,
    impact: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshot_badge_prefers_api_component_degradation_normal() {
        let snapshot = ClaudeStatusSnapshot {
            indicator: "minor".to_owned(),
            description: "Minor Service Outage".to_owned(),
            components: vec![ClaudeComponentStatus {
                name: "Claude API".to_owned(),
                status: "partial_outage".to_owned(),
            }],
            incidents: vec![ClaudeIncidentStatus {
                name: "Elevated error rates on Opus 4.6 and 4.7".to_owned(),
                status: "investigating".to_owned(),
                impact: Some("major".to_owned()),
            }],
            fetched_at: Instant::now(),
            bytes_in: 1024,
            bytes_out: 128,
        };

        assert_eq!(snapshot.short_badge(), "status API partial outage");
        assert_eq!(
            snapshot.outage_context().as_deref(),
            Some("API partial outage · major · Elevated error rates on Opus 4.6 and 4.7")
        );
    }
}
