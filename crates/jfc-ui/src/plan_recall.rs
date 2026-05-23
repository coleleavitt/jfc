//! Two-phase LLM-driven plan recall — mirrors `memory_recall.rs` but for plans.
//!
//! When the user asks a question, this module selects relevant plans and
//! synthesizes context from them to inject into the system prompt.
//!
//! ## Pipeline
//!
//! 1. **Select** — given a query and plan slugs/titles, pick up to 3 relevant.
//! 2. **Synthesize** — read the selected plans and extract up to 5 context items.
//!
//! The recall block is wrapped in `<system-reminder>` tags.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::Mutex;

use anyhow::Result;
use serde_json::{Value, json};

use crate::plan::Plan;

// ─── Tunables ────────────────────────────────────────────────────────────────

/// Maximum number of plans the select pass may return.
const MAX_SELECTED_PLANS: usize = 3;

/// Maximum number of context items the synthesize pass may return.
const MAX_CONTEXT_ITEMS: usize = 5;

/// Token cap for the structured tool response.
const RECALL_MAX_TOKENS: u32 = 1024;

const SELECT_TOOL_NAME: &str = "select_plans";
const SYNTHESIZE_TOOL_NAME: &str = "extract_plan_context";

// ─── System prompts ──────────────────────────────────────────────────────────

fn select_system_prompt() -> String {
    "Select plans relevant to the user's current message.\n\
     \n\
     You will be given:\n\
     - The user's current message.\n\
     - A list of plan slugs with their titles and statuses.\n\
     \n\
     Return ONLY slugs of plans that are relevant to the user's current work.\n\
     A plan is relevant when it describes an initiative, feature, or goal that\n\
     the user's message is working toward or affects.\n\
     \n\
     Constraints:\n\
     - Return at most 3 slugs.\n\
     - If nothing is clearly relevant, return an empty list.\n\
     - Prefer Active plans over Draft/Paused.\n\
     - Ignore Archived plans unless explicitly asked about.\n\
     \n\
     Output: call the `select_plans` tool with your decision."
        .to_owned()
}

fn synthesize_system_prompt() -> String {
    "Extract context from the provided plans relevant to the user's message.\n\
     \n\
     You will be given:\n\
     - The user's current message.\n\
     - The full content of one or more plans (title, status, body).\n\
     \n\
     Distill these into short, actionable context items the assistant should\n\
     keep in mind while answering. Each item must:\n\
     - Be 1–2 sentences. No prose, no preamble.\n\
     - Reference which plan it came from.\n\
     - Be directly relevant to the user's message.\n\
     \n\
     Constraints:\n\
     - Return at most 5 context items.\n\
     - If nothing is relevant, return an empty list.\n\
     - Focus on: current status, next steps, blockers, linked tasks.\n\
     \n\
     Output: call the `extract_plan_context` tool."
        .to_owned()
}

// ─── Tool schemas ────────────────────────────────────────────────────────────

fn select_tool_def() -> jfc_provider::ToolDef {
    jfc_provider::ToolDef {
        name: SELECT_TOOL_NAME.into(),
        description: "Return the slugs of plans relevant to the user's current message.".into(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "selected_plans": {
                    "type": "array",
                    "description": "Slugs of plans relevant to the user's message. At most 3.",
                    "items": { "type": "string" },
                    "maxItems": MAX_SELECTED_PLANS,
                }
            },
            "required": ["selected_plans"]
        }),
    }
}

fn synthesize_tool_def() -> jfc_provider::ToolDef {
    jfc_provider::ToolDef {
        name: SYNTHESIZE_TOOL_NAME.into(),
        description: "Return context items extracted from the provided plans.".into(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "context_items": {
                    "type": "array",
                    "description": "Up to 5 short context items (1–2 sentences each).",
                    "items": {
                        "type": "object",
                        "properties": {
                            "context": {
                                "type": "string",
                                "description": "The context item, 1–2 sentences."
                            },
                            "plan_slug": {
                                "type": "string",
                                "description": "Slug of the plan this came from."
                            }
                        },
                        "required": ["context", "plan_slug"]
                    },
                    "maxItems": MAX_CONTEXT_ITEMS,
                }
            },
            "required": ["context_items"]
        }),
    }
}

// ─── Cache ───────────────────────────────────────────────────────────────────

/// Process-wide cache: `(hash(query), result)`.
static LAST_PLAN_RECALL: Mutex<Option<(u64, Option<String>)>> = Mutex::new(None);

/// Runtime toggle. `None` ⇒ defer to persisted config.
static RUNTIME_OVERRIDE: Mutex<Option<bool>> = Mutex::new(None);

/// Returns the effective enabled-state.
pub fn is_enabled(persisted: bool) -> bool {
    if let Ok(g) = RUNTIME_OVERRIDE.lock()
        && let Some(b) = *g
    {
        return b;
    }
    persisted
}

/// Set the runtime override.
pub fn set_runtime_override(value: Option<bool>) {
    if let Ok(mut g) = RUNTIME_OVERRIDE.lock() {
        *g = value;
    }
    if let Ok(mut g) = LAST_PLAN_RECALL.lock() {
        *g = None;
    }
}

fn hash_query(query: &str) -> u64 {
    let mut h = DefaultHasher::new();
    query.trim().hash(&mut h);
    h.finish()
}

fn cached_recall(query: &str) -> Option<Option<String>> {
    let guard = LAST_PLAN_RECALL.lock().ok()?;
    let (h, block) = guard.as_ref()?;
    if *h == hash_query(query) {
        Some(block.clone())
    } else {
        None
    }
}

fn cache_recall(query: &str, block: Option<String>) {
    if let Ok(mut guard) = LAST_PLAN_RECALL.lock() {
        *guard = Some((hash_query(query), block));
    }
}

#[cfg(test)]
pub fn clear_cache() {
    if let Ok(mut guard) = LAST_PLAN_RECALL.lock() {
        *guard = None;
    }
}

// ─── Phase 1: select ────────────────────────────────────────────────────────

/// Ask the model which plans are relevant to `query`. Returns chosen slugs.
pub async fn select_relevant_plans(
    query: &str,
    available_plans: &[Plan],
    provider: std::sync::Arc<dyn jfc_provider::Provider>,
    model: jfc_provider::ModelId,
) -> Result<Vec<String>> {
    use jfc_provider::{ProviderContent, ProviderMessage, ProviderRole, StreamOptions};

    if available_plans.is_empty() {
        return Ok(Vec::new());
    }

    let listing = render_plan_listing(available_plans);
    let user_msg = format!(
        "# User message\n\n{query}\n\n# Available plans\n\n{listing}\n\n\
         Call `{SELECT_TOOL_NAME}` with the slugs you want to surface."
    );

    let messages = vec![ProviderMessage {
        role: ProviderRole::User,
        content: vec![ProviderContent::Text(user_msg)],
    }];

    let opts = StreamOptions::new(model)
        .system(select_system_prompt())
        .max_tokens(RECALL_MAX_TOKENS)
        .tools(vec![select_tool_def()]);

    let resp = match call_provider(&*provider, messages, &opts).await {
        Ok(r) => r,
        Err(e) => {
            tracing::warn!(
                target: "jfc::plan_recall",
                error = %e,
                "select pass failed"
            );
            return Ok(Vec::new());
        }
    };

    let selected = parse_selection(&resp).unwrap_or_default();

    // Validate slugs exist
    let known: std::collections::HashSet<&str> = available_plans
        .iter()
        .map(|p| p.frontmatter.slug.as_str())
        .collect();

    let filtered: Vec<String> = selected
        .into_iter()
        .filter(|s| known.contains(s.as_str()))
        .take(MAX_SELECTED_PLANS)
        .collect();

    Ok(filtered)
}

fn render_plan_listing(plans: &[Plan]) -> String {
    let mut out = String::new();
    for plan in plans {
        let preview = plan.body.lines().next().unwrap_or("(empty)").trim();
        let preview = if preview.len() > 120 {
            format!("{}…", &preview[..preview.floor_char_boundary(120)])
        } else {
            preview.to_owned()
        };
        out.push_str(&format!(
            "- `{}` [{}] \"{}\": {}\n",
            plan.frontmatter.slug, plan.frontmatter.status, plan.frontmatter.title, preview
        ));
    }
    out
}

fn parse_selection(content: &str) -> Option<Vec<String>> {
    let v = first_json_object(content)?;
    let input = pick_input(&v);
    let arr = input.get("selected_plans")?.as_array()?;
    Some(
        arr.iter()
            .filter_map(Value::as_str)
            .map(str::to_owned)
            .collect(),
    )
}

// ─── Phase 2: synthesize ────────────────────────────────────────────────────

/// Synthesize context from selected plans into a system-reminder block.
pub async fn synthesize_plan_context(
    query: &str,
    selected_plans: &[Plan],
    provider: std::sync::Arc<dyn jfc_provider::Provider>,
    model: jfc_provider::ModelId,
) -> Result<Option<String>> {
    use jfc_provider::{ProviderContent, ProviderMessage, ProviderRole, StreamOptions};

    if selected_plans.is_empty() {
        return Ok(None);
    }

    let bodies = render_plan_bodies(selected_plans);
    let user_msg = format!(
        "# User message\n\n{query}\n\n# Plan contents\n\n{bodies}\n\n\
         Call `{SYNTHESIZE_TOOL_NAME}` with context items relevant to this message."
    );

    let messages = vec![ProviderMessage {
        role: ProviderRole::User,
        content: vec![ProviderContent::Text(user_msg)],
    }];

    let opts = StreamOptions::new(model)
        .system(synthesize_system_prompt())
        .max_tokens(RECALL_MAX_TOKENS)
        .tools(vec![synthesize_tool_def()]);

    let resp = match call_provider(&*provider, messages, &opts).await {
        Ok(r) => r,
        Err(e) => {
            tracing::warn!(
                target: "jfc::plan_recall",
                error = %e,
                "synthesize pass failed"
            );
            return Ok(None);
        }
    };

    let items = match parse_context_items(&resp) {
        Some(items) if !items.is_empty() => items,
        _ => return Ok(None),
    };

    Ok(Some(format_plan_recall_block(&items)))
}

fn render_plan_bodies(plans: &[Plan]) -> String {
    let mut out = String::new();
    for plan in plans {
        out.push_str(&format!(
            "## {} ({})\nStatus: {}\n\n{}\n\n",
            plan.frontmatter.title,
            plan.frontmatter.slug,
            plan.frontmatter.status,
            plan.body.trim()
        ));
    }
    out
}

#[derive(Debug, Clone)]
pub struct PlanContextItem {
    pub context: String,
    pub plan_slug: String,
}

fn parse_context_items(content: &str) -> Option<Vec<PlanContextItem>> {
    let v = first_json_object(content)?;
    let input = pick_input(&v);
    let arr = input.get("context_items")?.as_array()?;
    let mut out = Vec::new();
    for item in arr.iter().take(MAX_CONTEXT_ITEMS) {
        let context = item
            .get("context")
            .and_then(Value::as_str)?
            .trim()
            .to_owned();
        let plan_slug = item
            .get("plan_slug")
            .and_then(Value::as_str)
            .unwrap_or("plan")
            .trim()
            .to_owned();
        if !context.is_empty() {
            out.push(PlanContextItem { context, plan_slug });
        }
    }
    Some(out)
}

fn format_plan_recall_block(items: &[PlanContextItem]) -> String {
    let mut body = String::from(
        "The following context was recalled from active project plans. \
         They provide background on ongoing initiatives — apply when relevant.\n\n",
    );
    for PlanContextItem { context, plan_slug } in items {
        body.push_str(&format!("- {context} _(plan: {plan_slug})_\n"));
    }
    format!("\n\n<system-reminder>\n{body}</system-reminder>\n")
}

// ─── Orchestrator ────────────────────────────────────────────────────────────

/// Run the full select → synthesize pipeline. Returns the recall block or None.
pub async fn run_plan_recall(
    query: &str,
    plans: &[Plan],
    provider: std::sync::Arc<dyn jfc_provider::Provider>,
    model: jfc_provider::ModelId,
) -> Option<String> {
    if let Some(cached) = cached_recall(query) {
        return cached;
    }

    if plans.is_empty() {
        cache_recall(query, None);
        return None;
    }

    let selected_slugs =
        match select_relevant_plans(query, plans, provider.clone(), model.clone()).await {
            Ok(slugs) if !slugs.is_empty() => slugs,
            Ok(_) => {
                cache_recall(query, None);
                return None;
            }
            Err(_) => {
                cache_recall(query, None);
                return None;
            }
        };

    let selected: Vec<Plan> = plans
        .iter()
        .filter(|p| selected_slugs.contains(&p.frontmatter.slug))
        .cloned()
        .collect();

    let block = match synthesize_plan_context(query, &selected, provider, model).await {
        Ok(b) => b,
        Err(_) => None,
    };

    cache_recall(query, block.clone());
    block
}

// ─── Provider helpers ────────────────────────────────────────────────────────

async fn call_provider(
    provider: &dyn jfc_provider::Provider,
    messages: Vec<jfc_provider::ProviderMessage>,
    options: &jfc_provider::StreamOptions,
) -> Result<String> {
    use futures::StreamExt;

    match provider.complete(messages.clone(), options).await {
        Ok(resp) => Ok(resp.content),
        Err(e) => {
            let err_msg = e.to_string().to_lowercase();
            if !(err_msg.contains("not support") || err_msg.contains("unsupported")) {
                return Err(e);
            }
            let mut stream = provider.stream(messages, options).await?;
            let mut text = String::new();
            let mut tool_input = String::new();
            while let Some(event) = stream.next().await {
                match event? {
                    jfc_provider::StreamEvent::TextDelta { delta, .. } => {
                        text.push_str(&delta);
                    }
                    jfc_provider::StreamEvent::ToolDelta { delta, .. } => {
                        tool_input.push_str(&delta);
                    }
                    jfc_provider::StreamEvent::ToolDone { input_json, .. } => {
                        tool_input = input_json;
                    }
                    jfc_provider::StreamEvent::Done { .. } => break,
                    jfc_provider::StreamEvent::Error { message } => {
                        anyhow::bail!("{message}");
                    }
                    _ => {}
                }
            }
            if !tool_input.is_empty() {
                Ok(tool_input)
            } else {
                Ok(text)
            }
        }
    }
}

fn first_json_object(s: &str) -> Option<Value> {
    let s = s.trim();
    if let Ok(v) = serde_json::from_str::<Value>(s) {
        return Some(v);
    }
    let start = s.find('{')?;
    let end = s.rfind('}')?;
    if start >= end {
        return None;
    }
    serde_json::from_str(&s[start..=end]).ok()
}

fn pick_input(v: &Value) -> &Value {
    if let Some(input) = v.get("input") {
        return input;
    }
    if let Some(args) = v.get("arguments") {
        return args;
    }
    v
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plan::{PlanFrontmatter, PlanStatus};
    use async_trait::async_trait;
    use jfc_provider::{
        CompletionResponse, EventStream, ModelInfo, StreamConvention, StreamOptions, TokenUsage,
    };
    use std::path::PathBuf;
    use std::sync::{Arc, Mutex as StdMutex};

    struct MockProvider {
        responses: StdMutex<Vec<String>>,
        calls: StdMutex<Vec<String>>,
    }

    impl MockProvider {
        fn with_responses<I: IntoIterator<Item = String>>(items: I) -> Arc<Self> {
            Arc::new(Self {
                responses: StdMutex::new(items.into_iter().collect()),
                calls: StdMutex::new(Vec::new()),
            })
        }

        fn call_count(&self) -> usize {
            self.calls.lock().unwrap().len()
        }
    }

    #[async_trait]
    impl jfc_provider::Provider for MockProvider {
        fn name(&self) -> &str {
            "mock"
        }
        fn available_models(&self) -> Vec<ModelInfo> {
            Vec::new()
        }
        fn stream_convention(&self) -> StreamConvention {
            StreamConvention::AnthropicNative
        }
        async fn stream(
            &self,
            _: Vec<jfc_provider::ProviderMessage>,
            _: &StreamOptions,
        ) -> anyhow::Result<EventStream> {
            anyhow::bail!("stream not supported in mock");
        }
        async fn complete(
            &self,
            messages: Vec<jfc_provider::ProviderMessage>,
            _: &StreamOptions,
        ) -> anyhow::Result<CompletionResponse> {
            let last = messages
                .last()
                .and_then(|m| m.content.first())
                .and_then(|c| match c {
                    jfc_provider::ProviderContent::Text(t) => Some(t.clone()),
                    _ => None,
                })
                .unwrap_or_default();
            self.calls.lock().unwrap().push(last);

            let mut q = self.responses.lock().unwrap();
            let next = q.first().cloned().unwrap_or_else(|| "{}".to_owned());
            if q.len() > 1 {
                q.remove(0);
            }
            Ok(CompletionResponse {
                content: next,
                usage: TokenUsage::default(),
            })
        }
    }
    impl jfc_provider::seal::Sealed for MockProvider {}

    fn make_plan(slug: &str, title: &str, body: &str) -> Plan {
        Plan {
            path: PathBuf::from(format!("/fake/plans/{slug}.md")),
            frontmatter: PlanFrontmatter {
                slug: slug.to_owned(),
                title: title.to_owned(),
                status: PlanStatus::Active,
                created: None,
                last_advanced: None,
                linked_task_ids: Vec::new(),
                supersedes: None,
                tags: Vec::new(),
            },
            body: body.to_owned(),
        }
    }

    #[tokio::test]
    async fn select_returns_relevant_plans_normal() {
        clear_cache();
        let provider =
            MockProvider::with_responses([json!({"selected_plans": ["auth-system"]}).to_string()]);
        let plans = vec![
            make_plan("auth-system", "Auth System", "Build the auth layer"),
            make_plan("ui-redesign", "UI Redesign", "New component library"),
        ];

        let slugs = select_relevant_plans(
            "implement login endpoint",
            &plans,
            provider.clone(),
            jfc_provider::ModelId::new("haiku"),
        )
        .await
        .unwrap();

        assert_eq!(slugs, vec!["auth-system".to_owned()]);
        assert_eq!(provider.call_count(), 1);
    }

    #[tokio::test]
    async fn synthesize_emits_system_reminder_normal() {
        clear_cache();
        let provider = MockProvider::with_responses([json!({
            "context_items": [
                {"context": "Auth system uses JWT tokens.", "plan_slug": "auth-system"},
                {"context": "Next step is the login endpoint.", "plan_slug": "auth-system"},
            ]
        })
        .to_string()]);

        let plans = vec![make_plan(
            "auth-system",
            "Auth System",
            "Build JWT-based auth",
        )];

        let block = synthesize_plan_context(
            "implement login",
            &plans,
            provider,
            jfc_provider::ModelId::new("haiku"),
        )
        .await
        .unwrap()
        .expect("expected Some");

        assert!(block.contains("<system-reminder>"));
        assert!(block.contains("</system-reminder>"));
        assert!(block.contains("Auth system uses JWT tokens."));
        assert!(block.contains("_(plan: auth-system)_"));
    }

    #[tokio::test]
    async fn malformed_response_returns_empty_robust() {
        clear_cache();
        let provider = MockProvider::with_responses(["totally not json".to_owned()]);
        let plans = vec![make_plan("test", "Test", "body")];

        let slugs = select_relevant_plans(
            "query",
            &plans,
            provider,
            jfc_provider::ModelId::new("haiku"),
        )
        .await
        .unwrap();

        assert!(slugs.is_empty());
    }

    #[tokio::test]
    async fn empty_plans_skips_provider_robust() {
        clear_cache();
        let provider = MockProvider::with_responses([]);
        let plans: Vec<Plan> = Vec::new();

        let slugs = select_relevant_plans(
            "query",
            &plans,
            provider.clone(),
            jfc_provider::ModelId::new("haiku"),
        )
        .await
        .unwrap();

        assert!(slugs.is_empty());
        assert_eq!(provider.call_count(), 0);
    }

    #[tokio::test]
    async fn run_plan_recall_caches_result_normal() {
        clear_cache();
        let provider = MockProvider::with_responses([
            json!({"selected_plans": ["p1"]}).to_string(),
            json!({"context_items": [{"context": "Fact.", "plan_slug": "p1"}]}).to_string(),
        ]);
        let plans = vec![make_plan("p1", "Plan 1", "body")];

        let block = run_plan_recall(
            "query",
            &plans,
            provider.clone(),
            jfc_provider::ModelId::new("haiku"),
        )
        .await;
        assert!(block.is_some());

        // Second call should hit cache
        let block2 = run_plan_recall(
            "query",
            &plans,
            provider.clone(),
            jfc_provider::ModelId::new("haiku"),
        )
        .await;
        assert_eq!(block, block2);
        assert_eq!(provider.call_count(), 2); // only the original 2 calls
    }
}
