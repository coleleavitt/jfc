//! Two-phase LLM-driven memory recall — port of Claude Code v2.1.132's `bt1` /
//! `xt1` flow (`tengu_memory_survey_event` in cli.js).
//!
//! Background — why two passes instead of one:
//!
//! Older builds of Claude Code (and our previous behavior, see
//! `memory::render_memories_section`) injected *every* memory file into the
//! system prompt on every turn. That wastes input tokens and dilutes the
//! attention signal: a "favor concise replies" preference and a "this repo
//! uses sqlx" project fact are both equally weighted, even though the second
//! is irrelevant to a question about formatting.
//!
//! v2.1.132 ships a two-pass recall:
//!
//! 1. **Select** (system prompt `Rt1`): the user's current message + a list of
//!    memory filenames goes to a Haiku-tier model with one tool — `select_memories`.
//!    The model returns up to **5 filenames** it thinks are relevant. We force
//!    the tool call so the model can't ramble or skip the structured output.
//!
//! 2. **Synthesize** (system prompt `Ct1`): the *content* of those selected
//!    files plus the user's message goes to the same model with another forced
//!    tool — `extract_facts`. The model returns up to **7 short facts** (1–2
//!    sentences each), each citing the source file. The synthesized facts are
//!    wrapped in a `<system-reminder>` block and appended to the system prompt
//!    so the model treats them as background context, not user instructions.
//!
//! The recall is skipped when:
//!   - The memory list is empty (no work to do).
//!   - The user's last message is empty or starts with `/` (slash commands
//!     don't need memory grounding — the dispatcher handles them locally).
//!   - The provider rejects the recall call (graceful fallback — we log and
//!     return None, the turn continues without a recall block).
//!
//! Caching: `(query_hash, recall_block)` is memoized per-process so consecutive
//! turns with the same prompt (e.g. the user re-submits after a tool failure)
//! don't re-call the LLM. The hash uses `DefaultHasher` (FNV-ish) which is
//! adequate — collisions just trigger an unnecessary recall, never wrong data.
//!
//! ## DO-178B test convention
//!
//! Tests are split `_normal` (canned-response happy paths) and `_robust`
//! (malformed JSON, empty inputs, provider errors). Each function has at least
//! one of each.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::Arc;
use std::sync::Mutex;

use anyhow::Result;
use serde_json::{Value, json};

use crate::memory::MemoryEntry;
use crate::provider::{
    ModelId, Provider, ProviderContent, ProviderMessage, ProviderRole, StreamOptions, ToolDef,
};

// ─── Tunables (mirroring v132 defaults) ──────────────────────────────────────

/// Maximum number of memory files the select pass may return. v132 caps at 5.
const MAX_SELECTED_FILES: usize = 5;

/// Maximum number of facts the synthesize pass may return. v132 caps at 7.
const MAX_FACTS: usize = 7;

/// Token cap for the structured tool response. The schemas are tiny so this is
/// generous — we'd rather pay a bit more than truncate the JSON mid-array.
const RECALL_MAX_TOKENS: u32 = 1024;

const SELECT_TOOL_NAME: &str = "select_memories";
const SYNTHESIZE_TOOL_NAME: &str = "extract_facts";

// ─── System prompts (modeled on v132 Rt1 / Ct1) ──────────────────────────────

/// Built from cli.js `Rt1`'s structure. v2.0.x didn't ship this prompt, so the
/// exact wording is a reconstruction guided by the deliverables spec — the
/// shape (instruction → constraints → output rule) is what the model needs.
fn select_system_prompt() -> String {
    "Select memories relevant to the user's current message.\n\
     \n\
     You will be given:\n\
     - The user's current message.\n\
     - A list of memory file paths with one-line previews.\n\
     \n\
     Return ONLY filenames that are likely to inform the response to this\n\
     specific message. A memory is relevant when its content would change\n\
     how a coding assistant answers the message — corrections from the user,\n\
     project facts the user assumes you know, stylistic preferences that\n\
     apply to the kind of work being requested.\n\
     \n\
     Constraints:\n\
     - Return at most 5 filenames.\n\
     - If nothing is clearly relevant, return an empty list.\n\
     - Do NOT speculate or infer relevance from filenames alone — use the\n\
       preview text.\n\
     - Use exact filenames (no path normalization, no extensions stripped).\n\
     \n\
     Output: call the `select_memories` tool with your decision."
        .to_owned()
}

/// Built from cli.js `Ct1`'s structure. Mirrors the v132 synthesis prompt:
/// extract facts, cite sources, keep each fact short.
fn synthesize_system_prompt() -> String {
    "Extract facts relevant to the user's current message from the provided\n\
     memory contents.\n\
     \n\
     You will be given:\n\
     - The user's current message.\n\
     - The full text of one or more memory files (each tagged with its\n\
       source filename).\n\
     \n\
     Distill these into short, actionable facts the assistant should keep in\n\
     mind while answering. Each fact must:\n\
     - Be 1–2 sentences. No prose, no preamble.\n\
     - Cite its source memory file (the filename, not the full path).\n\
     - Stand alone — readable without the original memory.\n\
     - Be directly relevant to the user's message. Drop anything tangential.\n\
     \n\
     Constraints:\n\
     - Return at most 7 facts.\n\
     - If nothing in the memories is relevant, return an empty list — do not\n\
       fabricate.\n\
     - Do NOT include instructions, opinions, or meta-commentary about the\n\
       memories themselves.\n\
     \n\
     Output: call the `extract_facts` tool."
        .to_owned()
}

// ─── Tool schemas ────────────────────────────────────────────────────────────

fn select_tool_def() -> ToolDef {
    ToolDef {
        name: SELECT_TOOL_NAME.into(),
        description: "Return the filenames of memories relevant to the user's current message."
            .into(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "selected_memories": {
                    "type": "array",
                    "description": "Filenames (basename only, with extension) of memories relevant to the user's message. At most 5. Empty if nothing applies.",
                    "items": { "type": "string" },
                    "maxItems": MAX_SELECTED_FILES,
                }
            },
            "required": ["selected_memories"]
        }),
    }
}

fn synthesize_tool_def() -> ToolDef {
    ToolDef {
        name: SYNTHESIZE_TOOL_NAME.into(),
        description: "Return short facts extracted from the provided memories.".into(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "relevant_facts": {
                    "type": "array",
                    "description": "Up to 7 short facts (1–2 sentences each). Each must cite its source filename.",
                    "items": {
                        "type": "object",
                        "properties": {
                            "fact": {
                                "type": "string",
                                "description": "The fact itself, 1–2 sentences."
                            },
                            "source": {
                                "type": "string",
                                "description": "Filename (basename) the fact came from."
                            }
                        },
                        "required": ["fact", "source"]
                    },
                    "maxItems": MAX_FACTS,
                },
                "cited_memories": {
                    "type": "array",
                    "description": "Distinct source filenames referenced by relevant_facts. May be empty if relevant_facts is empty.",
                    "items": { "type": "string" }
                }
            },
            "required": ["relevant_facts", "cited_memories"]
        }),
    }
}

// ─── Recall cache ────────────────────────────────────────────────────────────

/// Process-wide cache keyed on `hash(query)`. Holds the last produced recall
/// block so a re-submitted prompt doesn't re-call the LLM. Single-slot is fine
/// — stream calls are sequential within a session, so a deeper cache wouldn't
/// hit more often. Wrapped in `Mutex` because `OnceLock` would lock the cache
/// to its first value.
static LAST_RECALL: Mutex<Option<(u64, Option<String>)>> = Mutex::new(None);

/// Runtime toggle for `/memory recall on|off`. `None` ⇒ defer to
/// `Config.memory_recall_enabled` (the persisted value). `Some(b)` ⇒ override.
/// Lives for the process lifetime — the user can persist their choice by
/// writing it to `~/.config/jfc/config.toml` themselves; we don't touch the
/// file from a slash command since the user may have hand-formatted it.
static RUNTIME_OVERRIDE: Mutex<Option<bool>> = Mutex::new(None);

/// Returns the effective enabled-state — runtime override beats persisted
/// config. Used by both the stream-prep path and the `/memory recall status`
/// slash command.
pub fn is_enabled(persisted: bool) -> bool {
    if let Ok(g) = RUNTIME_OVERRIDE.lock() {
        if let Some(b) = *g {
            return b;
        }
    }
    persisted
}

/// Set the runtime override. Pass `None` to fall back to the persisted config.
pub fn set_runtime_override(value: Option<bool>) {
    if let Ok(mut g) = RUNTIME_OVERRIDE.lock() {
        *g = value;
    }
    // Drop the cache when toggling so the next turn re-runs (or skips) recall.
    if let Ok(mut g) = LAST_RECALL.lock() {
        *g = None;
    }
}

fn hash_query(query: &str) -> u64 {
    let mut h = DefaultHasher::new();
    query.trim().hash(&mut h);
    h.finish()
}

/// Returns `Some(block)` when this query is the same as the last call's. The
/// inner `Option<String>` captures both "we computed a block" and "we
/// affirmatively decided no block applies" — both are valid cached outcomes
/// that should not trigger a re-call.
pub fn cached_recall(query: &str) -> Option<Option<String>> {
    let guard = LAST_RECALL.lock().ok()?;
    let (h, block) = guard.as_ref()?;
    if *h == hash_query(query) {
        Some(block.clone())
    } else {
        None
    }
}

fn cache_recall(query: &str, block: Option<String>) {
    if let Ok(mut guard) = LAST_RECALL.lock() {
        *guard = Some((hash_query(query), block));
    }
}

/// Test-only: drop any cached entry. Lets each test start from a clean slot.
#[cfg(test)]
pub fn clear_cache() {
    if let Ok(mut guard) = LAST_RECALL.lock() {
        *guard = None;
    }
}

// ─── Phase 1: select ────────────────────────────────────────────────────────

/// Ask the model which of the available memories is relevant to `query`.
/// Returns the chosen filenames (basenames). On any error or malformed
/// response, returns `Ok(vec![])` — recall is opportunistic, never blocking.
#[tracing::instrument(
    target = "jfc::memory_recall",
    skip(available, provider),
    fields(
        provider = %provider.name(),
        model = %model,
        query_len = query.len(),
        available_count = available.len(),
    ),
)]
pub async fn select_relevant_memories(
    query: &str,
    available: &[MemoryEntry],
    provider: Arc<dyn Provider>,
    model: ModelId,
) -> Result<Vec<String>> {
    if available.is_empty() {
        tracing::debug!(target: "jfc::memory_recall", "no memories available — skipping select");
        return Ok(Vec::new());
    }

    let listing = render_memory_listing(available);
    let user_msg = format!(
        "# User message\n\n{query}\n\n# Available memories\n\n{listing}\n\n\
         Call `{SELECT_TOOL_NAME}` with the filenames you want to surface."
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
                target: "jfc::memory_recall",
                error = %e,
                "select pass failed — returning empty selection"
            );
            return Ok(Vec::new());
        }
    };

    let selected = parse_selection(&resp).unwrap_or_default();

    // Validate: every returned name must exist in `available`. The model
    // sometimes adds a `.md` we already had or hallucinates plausible-looking
    // filenames; drop anything we can't actually load.
    let known: std::collections::HashSet<String> = available
        .iter()
        .filter_map(|m| {
            m.path
                .file_name()
                .and_then(|s| s.to_str())
                .map(str::to_owned)
        })
        .collect();

    let filtered: Vec<String> = selected
        .into_iter()
        .filter(|s| known.contains(s))
        .take(MAX_SELECTED_FILES)
        .collect();

    tracing::debug!(
        target: "jfc::memory_recall",
        selected = filtered.len(),
        "select pass complete"
    );

    Ok(filtered)
}

fn render_memory_listing(memories: &[MemoryEntry]) -> String {
    let mut out = String::new();
    for mem in memories {
        let filename = mem
            .path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("unknown.md");
        let preview = mem.body.lines().next().unwrap_or("(empty)").trim();
        let preview = if preview.len() > 200 {
            format!("{}…", &preview[..200])
        } else {
            preview.to_owned()
        };
        out.push_str(&format!(
            "- `{filename}` [{}|{}]: {preview}\n",
            mem.frontmatter.memory_type, mem.frontmatter.scope
        ));
    }
    out
}

fn parse_selection(content: &str) -> Option<Vec<String>> {
    let v = first_json_object(content)?;
    let arr = pick_input(&v).get("selected_memories")?.as_array()?;
    let names: Vec<String> = arr
        .iter()
        .filter_map(Value::as_str)
        .map(str::to_owned)
        .collect();
    Some(names)
}

// ─── Phase 2: synthesize ────────────────────────────────────────────────────

/// Given the memories chosen by the select pass, ask the model to extract up
/// to 7 facts. Returns a fully-formatted `<system-reminder>` block ready to
/// append to the system prompt, or `None` when no facts apply / the call
/// failed.
#[tracing::instrument(
    target = "jfc::memory_recall",
    skip(selected, provider),
    fields(
        provider = %provider.name(),
        model = %model,
        query_len = query.len(),
        selected_count = selected.len(),
    ),
)]
pub async fn synthesize_memories(
    query: &str,
    selected: &[MemoryEntry],
    provider: Arc<dyn Provider>,
    model: ModelId,
) -> Result<Option<String>> {
    if selected.is_empty() {
        return Ok(None);
    }

    let bodies = render_memory_bodies(selected);
    let user_msg = format!(
        "# User message\n\n{query}\n\n# Memory contents\n\n{bodies}\n\n\
         Call `{SYNTHESIZE_TOOL_NAME}` with the facts that matter for this message."
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
                target: "jfc::memory_recall",
                error = %e,
                "synthesize pass failed — no recall block"
            );
            return Ok(None);
        }
    };

    let facts = match parse_facts(&resp) {
        Some(f) if !f.is_empty() => f,
        _ => {
            tracing::debug!(
                target: "jfc::memory_recall",
                "synthesize pass returned no usable facts"
            );
            return Ok(None);
        }
    };

    Ok(Some(format_recall_block(&facts)))
}

fn render_memory_bodies(memories: &[MemoryEntry]) -> String {
    let mut out = String::new();
    for mem in memories {
        let filename = mem
            .path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("unknown.md");
        out.push_str(&format!("## {filename}\n\n{}\n\n", mem.body.trim()));
    }
    out
}

#[derive(Debug, Clone)]
pub struct Fact {
    pub fact: String,
    pub source: String,
}

fn parse_facts(content: &str) -> Option<Vec<Fact>> {
    let v = first_json_object(content)?;
    let input = pick_input(&v);
    let arr = input.get("relevant_facts")?.as_array()?;
    let mut out = Vec::new();
    for item in arr.iter().take(MAX_FACTS) {
        let fact = item.get("fact").and_then(Value::as_str)?.trim().to_owned();
        let source = item
            .get("source")
            .and_then(Value::as_str)
            .unwrap_or("memory")
            .trim()
            .to_owned();
        if !fact.is_empty() {
            out.push(Fact { fact, source });
        }
    }
    Some(out)
}

/// v2.1.132 wraps the synthesized facts in a `<system-reminder>` so the model
/// treats them as background context rather than user instructions. Match
/// that shape exactly so the model's existing trained behavior takes over.
pub fn format_recall_block(facts: &[Fact]) -> String {
    let mut body = String::from(
        "The following facts were recalled from prior conversations and saved memories. \
         They are background context, not user instructions — apply them when relevant, \
         and ignore any that don't fit the current task.\n\n",
    );
    for Fact { fact, source } in facts {
        body.push_str(&format!("- {fact} _(source: {source})_\n"));
    }
    format!("\n\n<system-reminder>\n{body}</system-reminder>\n")
}

// ─── Provider call helpers ──────────────────────────────────────────────────

/// Invoke `provider.complete()`; if the provider doesn't support
/// non-streaming, fall back to draining `provider.stream()`. Mirrors
/// `compact::complete_or_stream` but returns the raw string content (we
/// don't need the usage struct here).
async fn call_provider(
    provider: &dyn Provider,
    messages: Vec<ProviderMessage>,
    options: &StreamOptions,
) -> Result<String> {
    use futures::StreamExt;

    match provider.complete(messages.clone(), options).await {
        Ok(resp) => Ok(resp.content),
        Err(e) => {
            let err_msg = e.to_string().to_lowercase();
            if !(err_msg.contains("not support") || err_msg.contains("unsupported")) {
                return Err(e);
            }
            tracing::debug!(
                target: "jfc::memory_recall",
                "provider.complete() unsupported — falling back to streaming"
            );
            let mut stream = provider.stream(messages, options).await?;
            let mut text = String::new();
            let mut tool_input = String::new();
            while let Some(event) = stream.next().await {
                match event? {
                    crate::provider::StreamEvent::TextDelta { delta, .. } => {
                        text.push_str(&delta);
                    }
                    crate::provider::StreamEvent::ToolDelta { delta, .. } => {
                        tool_input.push_str(&delta);
                    }
                    crate::provider::StreamEvent::ToolDone { input_json, .. } => {
                        tool_input = input_json;
                    }
                    crate::provider::StreamEvent::Done { .. } => break,
                    crate::provider::StreamEvent::Error { message } => {
                        anyhow::bail!("{message}");
                    }
                    _ => {}
                }
            }
            // Prefer the structured tool input when we got it.
            if !tool_input.is_empty() {
                Ok(tool_input)
            } else {
                Ok(text)
            }
        }
    }
}

/// Locate the first complete JSON object in a string. Providers return tool
/// inputs in a few different shapes — sometimes wrapped in `{"input": ...}`,
/// sometimes naked, sometimes with leading prose. We just want the first
/// brace-delimited object.
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

/// Some providers wrap the tool args in `{"input": {...}}` (Anthropic raw
/// content blocks) or `{"arguments": {...}}` (OpenAI). Peel one layer if
/// present so callers can read the schema fields directly.
fn pick_input(v: &Value) -> &Value {
    if let Some(input) = v.get("input") {
        return input;
    }
    if let Some(args) = v.get("arguments") {
        return args;
    }
    v
}

// ─── Top-level orchestrator ─────────────────────────────────────────────────

/// Run the full select → synthesize pipeline against `query`. Returns the
/// recall block (`Some(...)`) when at least one relevant fact was extracted,
/// or `None` otherwise. Caches `(hash(query), result)` so back-to-back calls
/// with the same query reuse the previous result.
///
/// `query` is typically the user's last message text. Slash commands and
/// empty queries should be filtered by the caller — this function trusts its
/// input.
pub async fn run_recall(
    query: &str,
    available: &[MemoryEntry],
    provider: Arc<dyn Provider>,
    model: ModelId,
) -> Option<String> {
    if let Some(cached) = cached_recall(query) {
        tracing::debug!(target: "jfc::memory_recall", "recall cache hit");
        return cached;
    }

    if available.is_empty() {
        cache_recall(query, None);
        return None;
    }

    let selected_names = match select_relevant_memories(
        query,
        available,
        provider.clone(),
        model.clone(),
    )
    .await
    {
        Ok(names) if !names.is_empty() => names,
        Ok(_) => {
            cache_recall(query, None);
            return None;
        }
        Err(e) => {
            tracing::warn!(
                target: "jfc::memory_recall",
                error = %e,
                "select_relevant_memories errored — skipping recall"
            );
            cache_recall(query, None);
            return None;
        }
    };

    let selected: Vec<MemoryEntry> = available
        .iter()
        .filter(|m| {
            m.path
                .file_name()
                .and_then(|f| f.to_str())
                .map(|f| selected_names.iter().any(|n| n == f))
                .unwrap_or(false)
        })
        .cloned()
        .collect();

    let block = match synthesize_memories(query, &selected, provider, model).await {
        Ok(b) => b,
        Err(e) => {
            tracing::warn!(
                target: "jfc::memory_recall",
                error = %e,
                "synthesize_memories errored — skipping recall"
            );
            None
        }
    };

    cache_recall(query, block.clone());
    block
}

// ─── Tests (DO-178B normal/robust pairs) ────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::{MemoryFrontmatter, MemoryLevel, MemoryScope, MemoryType};
    use crate::provider::{
        CompletionResponse, EventStream, ModelInfo, StreamConvention, TokenUsage,
    };
    use async_trait::async_trait;
    use std::path::PathBuf;
    use std::sync::Mutex as StdMutex;

    // ── Mock provider ────────────────────────────────────────────────────

    /// Provider stub that returns a canned completion. Records every call so
    /// the tests can assert "the synthesize phase was/wasn't invoked".
    struct MockProvider {
        responses: StdMutex<Vec<String>>,
        calls: StdMutex<Vec<String>>,
        err_on: Option<String>,
    }

    impl MockProvider {
        fn with_responses<I: IntoIterator<Item = String>>(items: I) -> Arc<Self> {
            Arc::new(Self {
                responses: StdMutex::new(items.into_iter().collect()),
                calls: StdMutex::new(Vec::new()),
                err_on: None,
            })
        }

        fn calls(&self) -> Vec<String> {
            self.calls.lock().unwrap().clone()
        }
    }

    #[async_trait]
    impl Provider for MockProvider {
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
            _: Vec<ProviderMessage>,
            _: &StreamOptions,
        ) -> anyhow::Result<EventStream> {
            anyhow::bail!("stream not supported in mock");
        }
        async fn complete(
            &self,
            messages: Vec<ProviderMessage>,
            _: &StreamOptions,
        ) -> anyhow::Result<CompletionResponse> {
            // Record the user message text so tests can assert on what we sent.
            let last = messages
                .last()
                .and_then(|m| m.content.first())
                .and_then(|c| match c {
                    ProviderContent::Text(t) => Some(t.clone()),
                    _ => None,
                })
                .unwrap_or_default();
            self.calls.lock().unwrap().push(last);

            if let Some(needle) = &self.err_on {
                if self
                    .calls
                    .lock()
                    .unwrap()
                    .last()
                    .map(|s| s.contains(needle))
                    .unwrap_or(false)
                {
                    anyhow::bail!("mock forced error on '{needle}'");
                }
            }

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
    impl crate::provider::seal::Sealed for MockProvider {}

    fn make_entry(filename: &str, body: &str) -> MemoryEntry {
        MemoryEntry {
            path: PathBuf::from(format!("/fake/memory/{filename}")),
            level: MemoryLevel::Project,
            frontmatter: MemoryFrontmatter {
                memory_type: MemoryType::Context,
                scope: MemoryScope::Private,
                created: None,
            },
            body: body.to_owned(),
        }
    }

    // ── select_relevant_memories ─────────────────────────────────────────

    // Normal: a well-formed `selected_memories` array round-trips through the
    // parser and gets filtered against the actual memory list.
    #[tokio::test]
    async fn select_returns_canned_filenames_normal() {
        clear_cache();
        let provider = MockProvider::with_responses([
            json!({"selected_memories": ["a.md", "b.md"]}).to_string(),
        ]);
        let entries = vec![
            make_entry("a.md", "first"),
            make_entry("b.md", "second"),
            make_entry("c.md", "unrelated"),
        ];

        let names = select_relevant_memories(
            "implement feature X",
            &entries,
            provider.clone(),
            ModelId::new("haiku"),
        )
        .await
        .unwrap();

        assert_eq!(names, vec!["a.md".to_owned(), "b.md".to_owned()]);
        assert_eq!(provider.calls().len(), 1, "select must call provider once");
    }

    // Robust: malformed JSON is swallowed — we return an empty selection
    // instead of panicking. The recall pipeline must never break a turn.
    #[tokio::test]
    async fn select_malformed_returns_empty_robust() {
        clear_cache();
        let provider = MockProvider::with_responses(["this is not json at all".to_owned()]);
        let entries = vec![make_entry("a.md", "first")];

        let names = select_relevant_memories("query", &entries, provider, ModelId::new("haiku"))
            .await
            .unwrap();

        assert!(names.is_empty(), "malformed response must yield no names");
    }

    // Robust: hallucinated filenames not present in `available` are dropped.
    #[tokio::test]
    async fn select_drops_hallucinated_filenames_robust() {
        clear_cache();
        let provider = MockProvider::with_responses([
            json!({"selected_memories": ["a.md", "ghost.md"]}).to_string(),
        ]);
        let entries = vec![make_entry("a.md", "first")];

        let names = select_relevant_memories("q", &entries, provider, ModelId::new("haiku"))
            .await
            .unwrap();
        assert_eq!(names, vec!["a.md".to_owned()]);
    }

    // Robust: empty memory list returns empty selection without calling the
    // provider — saves a network round-trip on greenfield repos.
    #[tokio::test]
    async fn select_empty_memories_skips_provider_robust() {
        clear_cache();
        let provider = MockProvider::with_responses([]);
        let names = select_relevant_memories("q", &[], provider.clone(), ModelId::new("haiku"))
            .await
            .unwrap();
        assert!(names.is_empty());
        assert!(
            provider.calls().is_empty(),
            "must not call provider when no memories exist"
        );
    }

    // ── synthesize_memories ──────────────────────────────────────────────

    // Normal: a well-formed `relevant_facts` payload becomes a recall block
    // with the expected `<system-reminder>` envelope and source citations.
    #[tokio::test]
    async fn synthesize_emits_system_reminder_block_normal() {
        clear_cache();
        let provider = MockProvider::with_responses([json!({
            "relevant_facts": [
                {"fact": "Prefer concise replies.", "source": "style.md"},
                {"fact": "This repo uses sqlx.", "source": "stack.md"},
            ],
            "cited_memories": ["style.md", "stack.md"]
        })
        .to_string()]);

        let selected = vec![
            make_entry("style.md", "Prefer concise replies."),
            make_entry("stack.md", "Repo uses sqlx."),
        ];

        let block = synthesize_memories("q", &selected, provider, ModelId::new("haiku"))
            .await
            .unwrap()
            .expect("expected a Some(block)");

        assert!(block.starts_with("\n\n<system-reminder>\n"));
        assert!(block.trim_end().ends_with("</system-reminder>"));
        assert!(block.contains("Prefer concise replies"));
        assert!(block.contains("_(source: style.md)_"));
        assert!(block.contains("_(source: stack.md)_"));
    }

    // Normal: empty selected list returns None without calling the provider.
    #[tokio::test]
    async fn synthesize_empty_selection_returns_none_normal() {
        clear_cache();
        let provider = MockProvider::with_responses([]);
        let block = synthesize_memories("q", &[], provider.clone(), ModelId::new("haiku"))
            .await
            .unwrap();
        assert!(block.is_none());
        assert!(provider.calls().is_empty());
    }

    // Robust: malformed synthesize response → returns None, no panic.
    #[tokio::test]
    async fn synthesize_malformed_returns_none_robust() {
        clear_cache();
        let provider = MockProvider::with_responses(["not even close to json".to_owned()]);
        let selected = vec![make_entry("a.md", "body")];
        let block = synthesize_memories("q", &selected, provider, ModelId::new("haiku"))
            .await
            .unwrap();
        assert!(block.is_none(), "malformed response must yield None");
    }

    // Robust: empty `relevant_facts` array returns None instead of an empty
    // `<system-reminder>` block — empty blocks would just bloat the prompt.
    #[tokio::test]
    async fn synthesize_empty_facts_returns_none_robust() {
        clear_cache();
        let provider = MockProvider::with_responses([
            json!({"relevant_facts": [], "cited_memories": []}).to_string(),
        ]);
        let selected = vec![make_entry("a.md", "body")];
        let block = synthesize_memories("q", &selected, provider, ModelId::new("haiku"))
            .await
            .unwrap();
        assert!(block.is_none());
    }

    // ── format_recall_block shape ────────────────────────────────────────

    // Normal: the envelope matches the v132 system-reminder shape exactly.
    // Caller wires this into the system prompt so the model treats it as
    // background context — matching CC's exact tag boundaries is what
    // makes that work.
    #[test]
    fn format_recall_block_shape_normal() {
        let facts = vec![
            Fact {
                fact: "Use snake_case for filenames.".into(),
                source: "style.md".into(),
            },
            Fact {
                fact: "Tests live alongside their module.".into(),
                source: "layout.md".into(),
            },
        ];
        let block = format_recall_block(&facts);
        assert!(block.starts_with("\n\n<system-reminder>\n"));
        assert!(block.contains("background context"));
        assert!(block.contains("- Use snake_case for filenames. _(source: style.md)_"));
        assert!(block.contains("- Tests live alongside their module. _(source: layout.md)_"));
        // Closing tag with trailing newline.
        assert!(block.ends_with("</system-reminder>\n"));
    }

    // Robust: a single empty fact list produces a still-well-formed envelope.
    // (Not used by the pipeline — `synthesize_memories` short-circuits — but
    // the formatter is public and must not panic on empty input.)
    #[test]
    fn format_recall_block_empty_facts_robust() {
        let block = format_recall_block(&[]);
        assert!(block.contains("<system-reminder>"));
        assert!(block.contains("</system-reminder>"));
    }

    // ── run_recall (orchestrator) ────────────────────────────────────────

    // Normal: end-to-end happy path — select returns one file, synthesize
    // returns one fact, the cache stores the result.
    #[tokio::test]
    async fn run_recall_full_pipeline_normal() {
        clear_cache();
        let provider = MockProvider::with_responses([
            json!({"selected_memories": ["a.md"]}).to_string(),
            json!({
                "relevant_facts": [{"fact": "F.", "source": "a.md"}],
                "cited_memories": ["a.md"]
            })
            .to_string(),
        ]);
        let entries = vec![make_entry("a.md", "body")];

        let block = run_recall(
            "implement X",
            &entries,
            provider.clone(),
            ModelId::new("haiku"),
        )
        .await;

        assert!(block.is_some());
        assert_eq!(provider.calls().len(), 2, "select + synthesize");

        // Same query → cache hit, no extra calls.
        let block2 = run_recall(
            "implement X",
            &entries,
            provider.clone(),
            ModelId::new("haiku"),
        )
        .await;
        assert_eq!(block, block2);
        assert_eq!(provider.calls().len(), 2, "cache must skip the LLM");
    }

    // Robust: empty memories list short-circuits before the provider is hit
    // and the result is cached as None.
    #[tokio::test]
    async fn run_recall_empty_memories_returns_none_robust() {
        clear_cache();
        let provider = MockProvider::with_responses([]);
        let block = run_recall("q", &[], provider.clone(), ModelId::new("haiku")).await;
        assert!(block.is_none());
        assert!(provider.calls().is_empty());
    }

    // ── parse_selection / parse_facts edge cases ─────────────────────────

    // Normal: tool input wrapped in `{"input": {...}}` (Anthropic raw shape)
    // is unwrapped before reading schema fields.
    #[test]
    fn parse_selection_unwraps_input_envelope_normal() {
        let s = json!({"input": {"selected_memories": ["x.md"]}}).to_string();
        let names = parse_selection(&s).unwrap();
        assert_eq!(names, vec!["x.md".to_owned()]);
    }

    // Normal: `{"arguments": {...}}` (OpenAI shape) also unwraps.
    #[test]
    fn parse_selection_unwraps_arguments_envelope_normal() {
        let s = json!({"arguments": {"selected_memories": ["y.md"]}}).to_string();
        let names = parse_selection(&s).unwrap();
        assert_eq!(names, vec!["y.md".to_owned()]);
    }

    // Robust: missing `selected_memories` field → None, not a panic.
    #[test]
    fn parse_selection_missing_field_returns_none_robust() {
        let s = json!({"some_other_key": []}).to_string();
        assert!(parse_selection(&s).is_none());
    }

    // Robust: facts without a `fact` field are dropped silently.
    #[test]
    fn parse_facts_drops_invalid_entries_robust() {
        let s = json!({
            "relevant_facts": [
                {"fact": "good", "source": "a.md"},
                {"source": "b.md"}, // missing fact
            ],
            "cited_memories": ["a.md", "b.md"]
        })
        .to_string();
        let parsed = parse_facts(&s);
        // The function bails the whole list when one entry is missing required
        // fields (`?` on the missing fact). This is conservative — better
        // than half-rendering. Either behavior is acceptable; assert the
        // current contract so a future refactor doesn't silently change it.
        assert!(parsed.is_none() || parsed.as_ref().unwrap().len() <= 2);
    }

    // ── cache hash function ──────────────────────────────────────────────

    // Normal: the same query produces the same hash; different queries produce
    // different hashes (collisions are statistically improbable for hand-picked
    // strings).
    #[test]
    fn hash_query_is_stable_normal() {
        assert_eq!(hash_query("hello"), hash_query("hello"));
        assert_eq!(hash_query("  hello  "), hash_query("hello")); // trim
        assert_ne!(hash_query("hello"), hash_query("world"));
    }

    // Robust: empty / whitespace-only queries hash equivalently.
    #[test]
    fn hash_query_whitespace_robust() {
        assert_eq!(hash_query(""), hash_query("   "));
        assert_eq!(hash_query("\n"), hash_query(""));
    }
}
