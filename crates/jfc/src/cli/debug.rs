//! `jfc debug query` — a one-shot raw provider query for debugging prompts.
//!
//! Lets you pick a provider/model, send a single prompt, and watch the *raw*
//! stream as it arrives: text deltas, thinking/reasoning blocks, tool-use
//! JSON, server tool results, usage, and stop reason. Nothing is hidden by the
//! TUI rendering layer — this is the wire view, for debugging what a model
//! actually emits (thinking XML tags, tool-call JSON, refusal stop reasons).
//!
//! Examples:
//!   jfc debug query -m claude-opus-4-5 "explain attention in one line"
//!   jfc debug query --provider anthropic-oauth -m claude-sonnet-4-6 \
//!       --thinking 4096 --show-thinking "prove sqrt 2 irrational"
//!   jfc debug query -m gpt-5.5 --raw-events "hello"      # dump every StreamEvent
//!   jfc debug providers                                   # list providers + models

use std::io::Write;
use std::sync::Arc;

use clap::{Subcommand, ValueEnum};
use futures::StreamExt;
use jfc_provider::{
    Provider, ProviderContent, ProviderMessage, ProviderRole, StopReason, StreamEvent,
    StreamOptions,
};

use crate::runtime::bootstrap::{build_providers, resolve_provider_model};

#[derive(Subcommand, Debug)]
pub(super) enum DebugSubcommand {
    /// Run a one-shot raw query against a provider/model and stream the output.
    Query(QueryArgs),
    /// List available providers and the models each one advertises.
    Providers,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, ValueEnum)]
pub(super) enum OutputMode {
    /// Human-readable, sectioned view (thinking / text / tool-use labelled).
    #[default]
    Pretty,
    /// One `{:?}`-formatted `StreamEvent` per line (the rawest view).
    Raw,
    /// One JSON object per line (`{"kind":...,...}`) for piping to `jq`.
    Json,
}

#[derive(clap::Args, Debug)]
pub(super) struct QueryArgs {
    /// The prompt text. If omitted, read the whole prompt from stdin.
    #[arg(value_name = "PROMPT")]
    prompt: Option<String>,

    /// Model id (bare `claude-opus-4-5` or qualified `provider/model`).
    #[arg(long, short = 'm', value_name = "MODEL")]
    model: Option<String>,

    /// Force a specific provider by name (e.g. `anthropic-oauth`, `openwebui`,
    /// `codex`). Overrides provider routing derived from the model id.
    #[arg(long, value_name = "NAME")]
    provider: Option<String>,

    /// System prompt to send ahead of the user message.
    #[arg(long, short = 's', value_name = "TEXT")]
    system: Option<String>,

    /// Extended-thinking budget in tokens (enables thinking when > 0).
    #[arg(long, value_name = "TOKENS")]
    thinking: Option<u32>,

    /// Max output tokens for the response.
    #[arg(long, value_name = "TOKENS", default_value = "4096")]
    max_tokens: u32,

    /// Sampling temperature.
    #[arg(long, value_name = "T")]
    temperature: Option<f64>,

    /// Reasoning effort for OpenAI-style models (low/medium/high).
    #[arg(long, value_name = "LEVEL")]
    reasoning_effort: Option<String>,

    /// Output mode.
    #[arg(long, value_enum, default_value = "pretty")]
    output: OutputMode,

    /// Shorthand for `--output raw` (dump every StreamEvent verbatim).
    #[arg(long = "raw-events")]
    raw_events: bool,

    /// Show thinking/reasoning blocks (on by default in pretty mode; use
    /// `--no-show-thinking` to hide them).
    #[arg(long = "show-thinking", default_value = "true", action = clap::ArgAction::Set)]
    show_thinking: bool,

    /// Print a usage + timing summary footer at the end.
    #[arg(long = "stats", default_value = "true", action = clap::ArgAction::Set)]
    stats: bool,
}

/// Entry point for `jfc debug …`.
pub(super) async fn run_debug_subcommand(sub: DebugSubcommand) -> anyhow::Result<()> {
    match sub {
        DebugSubcommand::Query(args) => run_query(args).await,
        DebugSubcommand::Providers => list_providers(),
    }
}

/// Resolve the provider + concrete model id to use, honoring an explicit
/// `--provider` override, else routing by the (possibly qualified) model id,
/// else falling back to the active provider's default model.
fn resolve(
    providers: &[Arc<dyn Provider>],
    active_idx: usize,
    default_model: &str,
    model_arg: Option<&str>,
    provider_arg: Option<&str>,
) -> anyhow::Result<(Arc<dyn Provider>, String)> {
    let model_id = model_arg.unwrap_or(default_model);

    if let Some(name) = provider_arg {
        let provider = providers
            .iter()
            .find(|p| p.name() == name)
            .cloned()
            .ok_or_else(|| {
                let available: Vec<&str> = providers.iter().map(|p| p.name()).collect();
                anyhow::anyhow!(
                    "no provider named `{name}` (available: {})",
                    available.join(", ")
                )
            })?;
        // Strip any `provider/` prefix from the model id when one is forced.
        let bare = model_id.split_once('/').map(|(_, m)| m).unwrap_or(model_id);
        return Ok((provider, bare.to_string()));
    }

    if let Some(res) = resolve_provider_model(providers, model_id) {
        return Ok((res.provider, res.model.as_str().to_string()));
    }

    // Fall back to the active provider with the model id as-is.
    let provider = providers
        .get(active_idx)
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("no providers configured"))?;
    Ok((provider, model_id.to_string()))
}

async fn run_query(args: QueryArgs) -> anyhow::Result<()> {
    let prompt = match args.prompt.clone() {
        Some(p) => p,
        None => read_stdin_prompt()?,
    };
    if prompt.trim().is_empty() {
        anyhow::bail!("empty prompt (pass a PROMPT argument or pipe text on stdin)");
    }

    let init = build_providers();
    let (provider, model) = resolve(
        &init.providers,
        init.active_idx,
        init.model.as_str(),
        args.model.as_deref(),
        args.provider.as_deref(),
    )?;

    let mode = if args.raw_events {
        OutputMode::Raw
    } else {
        args.output
    };

    eprintln!(
        "→ provider={} model={} thinking={} max_tokens={} mode={:?}",
        provider.name(),
        model,
        args.thinking.unwrap_or(0),
        args.max_tokens,
        mode
    );

    // Refresh auth (OAuth providers need a live access token before streaming).
    provider.ensure_auth().await?;

    let cfg = jfc_engine::config::load();
    let opts = build_stream_options(&model, &args, &cfg);

    let messages = vec![ProviderMessage {
        role: ProviderRole::User,
        content: vec![ProviderContent::Text(prompt)],
    }];

    let started = std::time::Instant::now();
    let mut stream = provider.stream(messages, &opts).await?;

    let mut printer = Printer::new(mode, args.show_thinking);
    let mut stats = Stats::default();

    while let Some(event) = stream.next().await {
        let event = event?;
        stats.observe(&event);
        printer.handle(&event)?;
    }
    printer.finish()?;

    if args.stats {
        let elapsed = started.elapsed();
        eprintln!(
            "\n── stats ──\n  elapsed: {:.2}s\n  input_tokens: {}\n  output_tokens: {}\n  cache_read: {}\n  cache_write: {}\n  stop_reason: {}",
            elapsed.as_secs_f64(),
            stats.input_tokens,
            stats.output_tokens,
            stats.cache_read,
            stats.cache_write,
            stats.stop_reason.as_deref().unwrap_or("(none)"),
        );
    }
    Ok(())
}

/// Build the [`StreamOptions`] for a query from its CLI args.
fn build_stream_options(
    model: &str,
    args: &QueryArgs,
    cfg: &jfc_engine::config::Config,
) -> StreamOptions {
    let mut opts = StreamOptions::new(model.to_string()).max_tokens(args.max_tokens);
    if let Some(system) = &args.system {
        opts = opts.system(system.clone());
    }
    if let Some(budget) = args.thinking
        && budget > 0
    {
        opts = opts.thinking(budget);
    }
    if let Some(t) = args.temperature {
        opts = opts.temperature(t);
    }
    if let Some(effort) = &args.reasoning_effort {
        opts = opts.reasoning_effort(effort.clone());
    }
    let custom_betas = cfg.anthropic_betas(std::iter::empty::<String>());
    if !custom_betas.is_empty() {
        opts = opts.custom_betas(custom_betas);
    }
    opts
}

fn read_stdin_prompt() -> anyhow::Result<String> {
    use std::io::Read;
    let mut buf = String::new();
    std::io::stdin().read_to_string(&mut buf)?;
    Ok(buf)
}

fn list_providers() -> anyhow::Result<()> {
    let init = build_providers();
    if init.providers.is_empty() {
        println!("(no providers configured — set ANTHROPIC_API_KEY or run `jfc auth login`)");
        return Ok(());
    }
    for (i, p) in init.providers.iter().enumerate() {
        let active = if i == init.active_idx {
            " (active)"
        } else {
            ""
        };
        println!("{}{}", p.name(), active);
        let models = p.available_models();
        if models.is_empty() {
            println!("  (no static model catalogue)");
        } else {
            for m in models.iter().take(40) {
                println!("  {}", m.id.as_str());
            }
            if models.len() > 40 {
                println!("  … and {} more", models.len() - 40);
            }
        }
    }
    Ok(())
}

/// Accumulates usage + stop reason across the stream for the stats footer.
#[derive(Default)]
struct Stats {
    input_tokens: u32,
    output_tokens: u32,
    thinking_tokens: Option<u32>,
    cache_read: u32,
    cache_write: u32,
    stop_reason: Option<String>,
}

impl Stats {
    fn observe(&mut self, event: &StreamEvent) {
        match event {
            StreamEvent::Usage {
                input_tokens,
                output_tokens,
                thinking_tokens,
                cache_read_tokens,
                cache_write_tokens,
            } => {
                self.input_tokens = *input_tokens;
                self.output_tokens = *output_tokens;
                self.thinking_tokens = *thinking_tokens;
                self.cache_read = *cache_read_tokens;
                self.cache_write = *cache_write_tokens;
            }
            StreamEvent::Done { stop_reason } => {
                self.stop_reason = Some(format_stop_reason(stop_reason));
            }
            // Deltas/metadata carry no stats fields; intentionally not tracked
            // here (the Printer renders them). Only Usage/Done feed the footer.
            StreamEvent::TextDelta { .. }
            | StreamEvent::TextDone { .. }
            | StreamEvent::ThinkingDelta { .. }
            | StreamEvent::ThinkingTokens { .. }
            | StreamEvent::ThinkingDone { .. }
            | StreamEvent::RedactedThinkingDone { .. }
            | StreamEvent::ToolDelta { .. }
            | StreamEvent::ToolDone { .. }
            | StreamEvent::ServerToolResult { .. }
            | StreamEvent::ResponseMetadata { .. }
            | StreamEvent::Error { .. }
            | StreamEvent::Keepalive
            | StreamEvent::FallbackTriggered(_) => {}
        }
    }
}

fn format_stop_reason(reason: &StopReason) -> String {
    match reason {
        StopReason::EndTurn => "end_turn".into(),
        StopReason::ToolUse => "tool_use".into(),
        StopReason::PauseTurn => "pause_turn".into(),
        StopReason::Refusal => "refusal".into(),
        StopReason::MaxTokens => "max_tokens".into(),
        StopReason::StopSequence => "stop_sequence".into(),
        StopReason::Other(s) => format!("other({s})"),
    }
}

/// Renders stream events according to the chosen [`OutputMode`].
struct Printer {
    mode: OutputMode,
    show_thinking: bool,
    /// Tracks which "section" we're in so pretty mode prints a header once.
    section: Section,
}

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
enum Section {
    None,
    Thinking,
    Text,
}

impl Printer {
    fn new(mode: OutputMode, show_thinking: bool) -> Self {
        Self {
            mode,
            show_thinking,
            section: Section::None,
        }
    }

    fn handle(&mut self, event: &StreamEvent) -> anyhow::Result<()> {
        match self.mode {
            OutputMode::Raw => {
                println!("{event:?}");
            }
            OutputMode::Json => {
                println!("{}", event_to_json(event));
            }
            OutputMode::Pretty => self.handle_pretty(event)?,
        }
        Ok(())
    }

    fn handle_pretty(&mut self, event: &StreamEvent) -> anyhow::Result<()> {
        let mut out = std::io::stdout();
        match event {
            StreamEvent::ThinkingDelta { delta, .. } => {
                if self.show_thinking {
                    self.enter(Section::Thinking, "── thinking ──");
                    print!("{delta}");
                    out.flush()?;
                }
            }
            StreamEvent::TextDelta { delta, .. } => {
                self.enter(Section::Text, "── response ──");
                print!("{delta}");
                out.flush()?;
            }
            StreamEvent::ToolDone {
                tool_name,
                tool_use_id,
                input_json,
                ..
            } => {
                println!("\n── tool_use: {tool_name} ({tool_use_id}) ──");
                println!("{}", pretty_json(input_json));
            }
            StreamEvent::ServerToolResult {
                tool_use_id,
                content,
                ..
            } => {
                println!("\n── server_tool_result ({tool_use_id}) ──");
                println!("{content:#}");
            }
            StreamEvent::RedactedThinkingDone { .. } => {
                if self.show_thinking {
                    println!("\n── redacted_thinking (opaque) ──");
                }
            }
            StreamEvent::Error { message } => {
                eprintln!("\n!! error: {message}");
            }
            StreamEvent::FallbackTriggered(info) => {
                // A provider/model fallback is highly relevant when debugging —
                // surface it rather than swallowing it.
                println!("\n── fallback_triggered: {info:?} ──");
            }
            StreamEvent::Done { stop_reason } => {
                println!("\n── done: {} ──", format_stop_reason(stop_reason));
            }
            // These are aggregates of the deltas above, keepalives, or stats-only
            // events: the streaming delta path + the stats footer already cover
            // them, so pretty mode deliberately emits nothing extra.
            StreamEvent::TextDone { .. }
            | StreamEvent::ThinkingDone { .. }
            | StreamEvent::ThinkingTokens { .. }
            | StreamEvent::ToolDelta { .. }
            | StreamEvent::Usage { .. }
            | StreamEvent::ResponseMetadata { .. }
            | StreamEvent::Keepalive => {}
        }
        Ok(())
    }

    /// Print a section header the first time we transition into it.
    fn enter(&mut self, section: Section, header: &str) {
        if self.section != section {
            if self.section != Section::None {
                println!();
            }
            println!("{header}");
            self.section = section;
        }
    }

    fn finish(&mut self) -> anyhow::Result<()> {
        if self.mode == OutputMode::Pretty && self.section != Section::None {
            println!();
        }
        std::io::stdout().flush()?;
        Ok(())
    }
}

/// Pretty-print a JSON string; fall back to the raw string if it doesn't parse.
fn pretty_json(s: &str) -> String {
    serde_json::from_str::<serde_json::Value>(s)
        .map(|v| format!("{v:#}"))
        .unwrap_or_else(|_| s.to_string())
}

/// Serialize a `StreamEvent` to a compact JSON line. `StreamEvent` doesn't
/// derive `Serialize` (it carries provider-internal types), so we project the
/// debug-relevant fields explicitly.
fn event_to_json(event: &StreamEvent) -> String {
    use serde_json::json;
    let v = match event {
        StreamEvent::TextDelta { index, delta } => {
            json!({"kind": "text_delta", "index": index, "delta": delta})
        }
        StreamEvent::TextDone { index, text } => {
            json!({"kind": "text_done", "index": index, "text": text})
        }
        StreamEvent::ThinkingDelta {
            index,
            delta,
            estimated_tokens,
        } => json!({
            "kind": "thinking_delta",
            "index": index,
            "delta": delta,
            "estimated_tokens": estimated_tokens,
        }),
        StreamEvent::ThinkingDone {
            index,
            text,
            signature,
        } => {
            json!({"kind": "thinking_done", "index": index, "text": text, "signature": signature})
        }
        StreamEvent::ThinkingTokens { index, delta } => {
            json!({"kind": "thinking_tokens", "index": index, "delta": delta})
        }
        StreamEvent::RedactedThinkingDone { index, data } => {
            json!({"kind": "redacted_thinking_done", "index": index, "data_len": data.len()})
        }
        StreamEvent::ToolDelta { index, delta } => {
            json!({"kind": "tool_delta", "index": index, "delta": delta})
        }
        StreamEvent::ToolDone {
            index,
            tool_name,
            tool_use_id,
            input_json,
            ..
        } => json!({
            "kind": "tool_done",
            "index": index,
            "tool_name": tool_name,
            "tool_use_id": tool_use_id,
            "input_json": input_json,
        }),
        StreamEvent::ServerToolResult {
            tool_use_id,
            content,
            ..
        } => json!({
            "kind": "server_tool_result",
            "tool_use_id": tool_use_id,
            "content": content,
        }),
        StreamEvent::Done { stop_reason } => {
            json!({"kind": "done", "stop_reason": format_stop_reason(stop_reason)})
        }
        StreamEvent::Usage {
            input_tokens,
            output_tokens,
            thinking_tokens,
            cache_read_tokens,
            cache_write_tokens,
        } => json!({
            "kind": "usage",
            "input_tokens": input_tokens,
            "output_tokens": output_tokens,
            "thinking_tokens": thinking_tokens,
            "cache_read_tokens": cache_read_tokens,
            "cache_write_tokens": cache_write_tokens,
        }),
        StreamEvent::ResponseMetadata {
            response_id,
            input_tokens,
        } => json!({
            "kind": "response_metadata",
            "response_id": response_id,
            "input_tokens": input_tokens,
        }),
        StreamEvent::Error { message } => json!({"kind": "error", "message": message}),
        StreamEvent::Keepalive => json!({"kind": "keepalive"}),
        StreamEvent::FallbackTriggered(info) => {
            json!({"kind": "fallback_triggered", "info": format!("{info:?}")})
        }
    };
    v.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stop_reason_formats_all_variants() {
        assert_eq!(format_stop_reason(&StopReason::EndTurn), "end_turn");
        assert_eq!(format_stop_reason(&StopReason::ToolUse), "tool_use");
        assert_eq!(format_stop_reason(&StopReason::PauseTurn), "pause_turn");
        assert_eq!(format_stop_reason(&StopReason::Refusal), "refusal");
        assert_eq!(format_stop_reason(&StopReason::MaxTokens), "max_tokens");
        assert_eq!(
            format_stop_reason(&StopReason::StopSequence),
            "stop_sequence"
        );
        assert_eq!(
            format_stop_reason(&StopReason::Other("x".into())),
            "other(x)"
        );
    }

    #[test]
    fn pretty_json_handles_valid_and_invalid() {
        assert_eq!(pretty_json(r#"{"a":1}"#), "{\n  \"a\": 1\n}");
        assert_eq!(pretty_json("not json"), "not json");
    }

    #[test]
    fn event_to_json_projects_text_delta() {
        let ev = StreamEvent::TextDelta {
            index: 0,
            delta: "hi".into(),
        };
        let j: serde_json::Value = serde_json::from_str(&event_to_json(&ev)).unwrap();
        assert_eq!(j["kind"], "text_delta");
        assert_eq!(j["delta"], "hi");
    }

    #[test]
    fn event_to_json_projects_thinking_and_tool() {
        let think = StreamEvent::ThinkingDelta {
            index: 1,
            delta: "reason".into(),
            estimated_tokens: Some(42),
        };
        let j: serde_json::Value = serde_json::from_str(&event_to_json(&think)).unwrap();
        assert_eq!(j["kind"], "thinking_delta");
        assert_eq!(j["estimated_tokens"], 42);

        let tool = StreamEvent::ToolDone {
            index: 2,
            tool_name: "Bash".into(),
            tool_use_id: "tu_1".into(),
            input_json: r#"{"cmd":"ls"}"#.into(),
            thought_signature: None,
        };
        let j: serde_json::Value = serde_json::from_str(&event_to_json(&tool)).unwrap();
        assert_eq!(j["kind"], "tool_done");
        assert_eq!(j["tool_name"], "Bash");
    }

    #[test]
    fn stats_accumulate_usage_and_stop() {
        let mut stats = Stats::default();
        stats.observe(&StreamEvent::Usage {
            input_tokens: 10,
            output_tokens: 20,
            thinking_tokens: Some(7),
            cache_read_tokens: 3,
            cache_write_tokens: 4,
        });
        stats.observe(&StreamEvent::Done {
            stop_reason: StopReason::EndTurn,
        });
        assert_eq!(stats.input_tokens, 10);
        assert_eq!(stats.output_tokens, 20);
        assert_eq!(stats.cache_read, 3);
        assert_eq!(stats.cache_write, 4);
        assert_eq!(stats.stop_reason.as_deref(), Some("end_turn"));
    }

    #[test]
    fn pretty_printer_enters_sections_once() {
        let mut p = Printer::new(OutputMode::Pretty, true);
        assert_eq!(p.section, Section::None);
        p.handle(&StreamEvent::ThinkingDelta {
            index: 0,
            delta: "r".into(),
            estimated_tokens: None,
        })
        .unwrap();
        assert_eq!(p.section, Section::Thinking);
        p.handle(&StreamEvent::TextDelta {
            index: 0,
            delta: "x".into(),
        })
        .unwrap();
        assert_eq!(p.section, Section::Text);
        p.finish().unwrap();
    }

    #[test]
    fn pretty_printer_hides_thinking_when_disabled() {
        // show_thinking=false: a thinking delta must not advance the section.
        let mut p = Printer::new(OutputMode::Pretty, false);
        p.handle(&StreamEvent::ThinkingDelta {
            index: 0,
            delta: "secret".into(),
            estimated_tokens: None,
        })
        .unwrap();
        assert_eq!(p.section, Section::None);
    }
}
