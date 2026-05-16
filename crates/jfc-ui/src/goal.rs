//! `/goal <condition>` — session-scoped stop condition.
//!
//! Models Claude Code v2.1.139+'s `/goal` as a **managed stop hook**: the
//! user sets a natural-language condition; every time the agent tries to
//! stop (`StopReason::EndTurn` with no pending tools / approvals), we
//! ask a separate evaluator call whether the transcript proves the
//! condition is satisfied. If not, we inject the evaluator's "what's
//! missing" reasoning as a `<system-reminder>` and let the agentic loop
//! continue. If yes, we clear the goal and stamp a visible
//! `goal_status` marker into the transcript.
//!
//! Why a separate module: the evaluator is provider-call-shaped (non-
//! streaming `complete()` returning structured JSON), and the loop
//! continuation logic is the same shape `continue_agentic_loop` already
//! takes — so the goal layer just decides "continue" vs "stop" and
//! mutates `app.messages` accordingly.

use anyhow::{Result, anyhow};
use serde::{Deserialize, Serialize};

use crate::types::{ChatMessage, MessagePart, Role};
use jfc_provider::{
    ModelId, Provider, ProviderContent, ProviderMessage, ProviderRole, StreamOptions,
};

/// Hard cap on the condition body to mirror Claude Code's 4000-char ceiling.
/// Prevents the user from accidentally pasting a 100KB prompt and
/// breaking every subsequent evaluator call.
pub const MAX_CONDITION_LEN: usize = 4_000;

/// Hard cap on iterations so a buggy evaluator can't drive the agent
/// forever. Mirrors the `max_turns` agent contract — after this many
/// consecutive "not yet met" verdicts, the goal auto-clears with a
/// failure marker.
pub const MAX_ITERATIONS: u32 = 50;

/// Hard cap on the transcript snapshot we send to the evaluator. Same
/// rationale as `advisor::MAX_SNAPSHOT_CHARS` — context blow-out
/// destroys the budget on a single call.
const MAX_SNAPSHOT_CHARS: usize = 40_000;

/// Active goal state. Persisted on `App` while the condition is live.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActiveGoal {
    /// The natural-language condition the user set.
    pub condition: String,
    /// Number of "not yet met" verdicts the evaluator has returned this
    /// session. Capped at [`MAX_ITERATIONS`].
    pub iterations: u32,
    /// Wall-clock instant the goal was set (millis since UNIX epoch).
    /// Used for the "Goal achieved in 2m 14s" success banner.
    pub set_at_ms: u64,
    /// The evaluator's last "what's missing" string. Surfaces in the UI
    /// so the user can see why the loop is still iterating.
    pub last_unmet_reason: Option<String>,
}

impl ActiveGoal {
    pub fn new(condition: String) -> Self {
        Self {
            condition,
            iterations: 0,
            set_at_ms: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis() as u64)
                .unwrap_or(0),
            last_unmet_reason: None,
        }
    }

    /// True once we've burned the iteration budget. Caller should clear
    /// the goal and emit a failure marker rather than evaluate again.
    pub fn is_exhausted(&self) -> bool {
        self.iterations >= MAX_ITERATIONS
    }

    /// Elapsed wall-clock time since the goal was set.
    pub fn elapsed(&self) -> std::time::Duration {
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(self.set_at_ms);
        std::time::Duration::from_millis(now_ms.saturating_sub(self.set_at_ms))
    }
}

/// The aliases that clear an active goal. Mirrors Claude Code's
/// `clear / stop / off / reset / none / cancel` set.
pub fn is_clear_arg(arg: &str) -> bool {
    matches!(
        arg.trim().to_ascii_lowercase().as_str(),
        "clear" | "stop" | "off" | "reset" | "none" | "cancel"
    )
}

/// Validate a candidate condition. Returns `Err` with a user-facing
/// message when the condition is empty, too long, or a control verb.
pub fn validate_condition(raw: &str) -> Result<String, &'static str> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err("Goal condition must not be empty. Usage: `/goal <condition>`");
    }
    if trimmed.len() > MAX_CONDITION_LEN {
        return Err("Goal condition is too long (max 4000 chars)");
    }
    if is_clear_arg(trimmed) {
        return Err("Use `/goal clear` (no quotes) to remove an active goal");
    }
    Ok(trimmed.to_owned())
}

/// Structured evaluator response. The evaluator MUST emit this as the
/// first JSON object in its reply; we parse it with serde_json and
/// fall back to "unmet" if parsing fails so a malformed evaluator
/// reply never silently terminates the loop.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GoalVerdict {
    /// True when the transcript proves the condition holds.
    pub ok: bool,
    /// One-line justification — surfaces in the system-reminder if
    /// `ok=false`, or in the "Goal achieved" banner if `ok=true`.
    #[serde(default)]
    pub reason: String,
}

/// System prompt prepended to every evaluator call. Carefully phrased
/// so the evaluator only judges from transcript evidence — no
/// extrapolation, no "looks plausible," no charity. Mirrors the
/// Claude Code stop-hook evaluator's tone.
pub const GOAL_EVALUATOR_SYSTEM_PROMPT: &str = "\
You are evaluating a stop-condition hook for a coding agent. The user \
set a goal; the transcript below is what the agent has done so far. \
Decide whether the transcript provides DIRECT EVIDENCE that the \
condition is satisfied. Evidence must be concrete (file written, \
command output, verified test result) — not promises, not plans, not \
\"will do next.\" If the agent has only described what it intends to \
do, the condition is NOT yet met.

Reply with EXACTLY one JSON object as the first thing in your \
response, no markdown fences, no prose preface:

{\"ok\": true, \"reason\": \"<evidence quote / file:line>\"}

or

{\"ok\": false, \"reason\": \"<what is still missing>\"}";

/// Render the main transcript into a single user-message string for
/// the evaluator. Same shape as `advisor::render_snapshot` — tool
/// calls collapse to `[Tool: <kind>]`, tool results truncate to keep
/// the call cheap.
pub fn render_snapshot(history: &[ChatMessage]) -> String {
    let mut out = String::new();
    for msg in history {
        let role = match msg.role {
            Role::User => "USER",
            Role::Assistant => "ASSISTANT",
        };
        out.push_str(&format!("---\n{role}:\n"));
        for part in &msg.parts {
            render_part_into(part, &mut out);
        }
        out.push('\n');
    }
    truncate_snapshot_tail(out)
}

/// Render one `MessagePart` into the snapshot accumulator. Pulled out
/// of `render_snapshot` so the outer function stays a flat loop.
fn render_part_into(part: &MessagePart, out: &mut String) {
    match part {
        MessagePart::Text(t) => out.push_str(t),
        // Reasoning is private to the model; the evaluator should
        // judge from public output only.
        MessagePart::Reasoning(_) => {}
        MessagePart::Tool(tc) => {
            // Show kind + input summary + a short slice of the output so
            // the evaluator can spot "the agent already ran cargo test
            // and it passed" without scrolling thousands of bytes.
            let output_preview = match &tc.output {
                crate::types::ToolOutput::Text(t) => truncate(t, 400),
                crate::types::ToolOutput::LargeText(lt) => truncate(&lt.content, 400),
                crate::types::ToolOutput::Diff(_) => "[diff]".to_owned(),
                crate::types::ToolOutput::FileContent { path, .. } => {
                    format!("[file: {path}]")
                }
                crate::types::ToolOutput::Command {
                    stdout, exit_code, ..
                } => format!(
                    "[exit={}] {}",
                    exit_code
                        .map(|c| c.to_string())
                        .unwrap_or_else(|| "?".into()),
                    truncate(stdout, 300)
                ),
                crate::types::ToolOutput::FileList(v) => {
                    format!("[{} entries]", v.len())
                }
                crate::types::ToolOutput::ServerToolResult { tool_kind, content } => {
                    let preview = serde_json::to_string(content).unwrap_or_default();
                    format!(
                        "[{wire}] {}",
                        truncate(&preview, 400),
                        wire = tool_kind.wire_type()
                    )
                }
                crate::types::ToolOutput::Empty => String::new(),
            };
            out.push_str(&format!(
                "\n[Tool: {} ({}) → {}]",
                tc.kind.label(),
                truncate(&tc.input.summary(), 200),
                output_preview,
            ));
        }
        MessagePart::TaskStatus(ts) => {
            out.push_str(&format!(
                "\n[Task: {} status={}]",
                ts.description,
                ts.status.label()
            ));
        }
        MessagePart::Advisor(a) => {
            out.push_str(&format!("\n[Advisor: {}]", truncate(a, 200)));
        }
        // Compaction marker — irrelevant for the evaluator's verdict.
        MessagePart::CompactBoundary { .. } => {}
        // Redacted thinking is opaque; nothing to render.
        MessagePart::RedactedThinking(_) => {}
    }
}

/// Tail-truncate the snapshot so the most recent (and most relevant)
/// context wins. The evaluator usually cares about the LAST few turns,
/// not the prologue.
///
/// Walks forward from the byte-offset cut point to the next char
/// boundary instead of slicing at an arbitrary byte. Without this,
/// a transcript with non-ASCII content (emoji, accented chars, CJK,
/// any code identifier with `é`) would panic the evaluator call with
/// "byte index N is not a char boundary." `floor_char_boundary` is
/// nightly-only so we hand-roll the equivalent using `char_indices`.
fn truncate_snapshot_tail(out: String) -> String {
    if out.len() <= MAX_SNAPSHOT_CHARS {
        return out;
    }
    let desired_cut = out.len() - MAX_SNAPSHOT_CHARS;
    let safe_cut = out
        .char_indices()
        .map(|(i, _)| i)
        .find(|&i| i >= desired_cut)
        .unwrap_or(out.len());
    format!(
        "[... transcript truncated to last {MAX_SNAPSHOT_CHARS} chars ...]\n{}",
        &out[safe_cut..]
    )
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        return s.to_owned();
    }
    let cut = s
        .char_indices()
        .map(|(i, _)| i)
        .find(|&i| i >= max)
        .unwrap_or(max);
    format!("{}…", &s[..cut])
}

/// Run the evaluator. Returns `Ok(verdict)` when the provider call
/// succeeded; `Err` on transport failure or unparseable reply.
pub async fn evaluate(
    provider: &dyn Provider,
    model: ModelId,
    condition: &str,
    history: &[ChatMessage],
) -> Result<GoalVerdict> {
    let snapshot = render_snapshot(history);
    let user_body = format!("Transcript:\n{snapshot}\n\nCondition to verify:\n{condition}");
    let messages = vec![ProviderMessage {
        role: ProviderRole::User,
        content: vec![ProviderContent::Text(user_body)],
    }];
    let opts = StreamOptions::new(model)
        .system(GOAL_EVALUATOR_SYSTEM_PROMPT)
        .max_tokens(512);
    let resp = provider
        .complete(messages, &opts)
        .await
        .map_err(|e| anyhow!("goal evaluator call failed: {e}"))?;
    parse_verdict(&resp.content)
}

/// Best-effort verdict parser. Looks for the first balanced `{...}`
/// JSON object in the reply and deserialises it. If parsing fails, we
/// default to `ok=false` with the raw reply as the reason — the loop
/// continues, the user sees the malformed reply, and a future iteration
/// can recover. Never returns Err on bad shape so the loop can't die.
pub fn parse_verdict(reply: &str) -> Result<GoalVerdict> {
    let trimmed = reply.trim();
    if trimmed.is_empty() {
        return Ok(GoalVerdict {
            ok: false,
            reason: "evaluator returned empty reply".to_owned(),
        });
    }
    if let Some(json) = extract_first_json_object(trimmed) {
        if let Ok(v) = serde_json::from_str::<GoalVerdict>(&json) {
            return Ok(v);
        }
    }
    Ok(GoalVerdict {
        ok: false,
        reason: format!(
            "evaluator reply was not parseable JSON: {}",
            truncate(trimmed, 200)
        ),
    })
}

/// Pull the first balanced JSON object out of `s`. Handles markdown
/// fences and prose preludes (some models ignore the "no preface"
/// instruction). Returns None if there's no `{` at all.
///
/// Implemented as a small state machine so the brace tracker isn't
/// fooled by `}` characters inside a quoted string.
fn extract_first_json_object(s: &str) -> Option<String> {
    let start = s.find('{')?;
    let bytes = s.as_bytes();
    let mut state = JsonScanState::default();
    for (i, &b) in bytes[start..].iter().enumerate() {
        if state.step(b) {
            return Some(s[start..start + i + 1].to_owned());
        }
    }
    None
}

#[derive(Default)]
struct JsonScanState {
    depth: i32,
    in_string: bool,
    escape: bool,
}

impl JsonScanState {
    /// Feed one byte. Returns `true` exactly once: when the byte closes
    /// the outermost `{...}` and the object is balanced.
    fn step(&mut self, b: u8) -> bool {
        if self.in_string {
            self.step_in_string(b);
            return false;
        }
        match b {
            b'"' => self.in_string = true,
            b'{' => self.depth += 1,
            b'}' => {
                self.depth -= 1;
                if self.depth == 0 {
                    return true;
                }
            }
            _ => {}
        }
        false
    }

    fn step_in_string(&mut self, b: u8) {
        if self.escape {
            self.escape = false;
        } else if b == b'\\' {
            self.escape = true;
        } else if b == b'"' {
            self.in_string = false;
        }
    }
}

/// Format the success banner shown when the goal is met. Surfaced as a
/// visible assistant message AND as a structured marker the resume
/// path can scan for.
pub fn format_success_banner(goal: &ActiveGoal, reason: &str) -> String {
    let elapsed = goal.elapsed();
    let mins = elapsed.as_secs() / 60;
    let secs = elapsed.as_secs() % 60;
    format!(
        "✓ Goal achieved in {mins}m {secs}s after {} iterations.\n\
         Condition: {}\n\
         Evidence: {reason}\n\
         [goal_status: met]",
        goal.iterations + 1,
        goal.condition,
    )
}

/// Format the failure banner shown when iterations are exhausted.
pub fn format_exhaustion_banner(goal: &ActiveGoal) -> String {
    let elapsed = goal.elapsed();
    let mins = elapsed.as_secs() / 60;
    let secs = elapsed.as_secs() % 60;
    let last = goal
        .last_unmet_reason
        .as_deref()
        .unwrap_or("(no diagnostic)");
    format!(
        "✗ Goal abandoned after {} iterations ({mins}m {secs}s).\n\
         Condition: {}\n\
         Last unmet reason: {last}\n\
         [goal_status: exhausted]",
        goal.iterations, goal.condition,
    )
}

/// Path to the goal sidecar JSON for a given session id. Lives next
/// to the session file under `~/.config/jfc/sessions/`. A sidecar
/// (rather than a new field on `SerializedSession`) keeps the
/// 28+ `save_session` call sites untouched and lets the goal layer
/// own its own persistence lifecycle. Missing file = no active goal.
pub fn sidecar_path(session_id: &str) -> std::path::PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join("jfc")
        .join("sessions")
        .join(format!("{session_id}.goal.json"))
}

/// Persist the active goal beside the session journal so resume can
/// rebuild it. `None` deletes any prior sidecar (the goal cleared,
/// or completed, or exhausted — we don't want resume to revive a
/// stale one). Best-effort: failures are logged, never propagated.
pub fn save_sidecar(session_id: &str, goal: Option<&ActiveGoal>) {
    let path = sidecar_path(session_id);
    match goal {
        Some(g) => {
            if let Some(parent) = path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            match serde_json::to_string_pretty(g) {
                Ok(json) => {
                    let tmp = path.with_extension("tmp");
                    if std::fs::write(&tmp, json).is_ok() {
                        let _ = std::fs::rename(&tmp, &path);
                        tracing::debug!(
                            target: "jfc::goal",
                            session_id,
                            iterations = g.iterations,
                            "goal sidecar saved"
                        );
                    }
                }
                Err(e) => {
                    tracing::warn!(
                        target: "jfc::goal",
                        session_id,
                        error = %e,
                        "failed to serialize goal sidecar"
                    );
                }
            }
        }
        None => {
            let _ = std::fs::remove_file(&path);
            tracing::debug!(
                target: "jfc::goal",
                session_id,
                "goal sidecar removed"
            );
        }
    }
}

/// Load any persisted goal for `session_id`. Returns `None` when the
/// sidecar is absent, unreadable, or malformed — those all read as
/// "no active goal." We don't surface load errors to the user
/// because a corrupted sidecar shouldn't block session resume.
pub fn load_sidecar(session_id: &str) -> Option<ActiveGoal> {
    let path = sidecar_path(session_id);
    let raw = std::fs::read_to_string(&path).ok()?;
    serde_json::from_str::<ActiveGoal>(&raw).ok()
}

/// System-reminder body injected into the user-side of the conversation
/// when the evaluator says "not yet met." Tells the agent what's
/// missing so the next sub-stream can address it directly.
pub fn format_unmet_reminder(condition: &str, reason: &str, iteration: u32) -> String {
    format!(
        "Stop-condition hook ({}/{} iterations):\n\
         Condition: {condition}\n\
         Verdict: not yet met\n\
         Missing: {reason}\n\n\
         Continue working toward the condition. Do not stop until it is \
         satisfied or the user clears the goal.",
        iteration, MAX_ITERATIONS,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    // Normal: validate_condition accepts a plain string and trims.
    #[test]
    fn validate_condition_accepts_normal() {
        assert_eq!(
            validate_condition("  build passes  ").unwrap(),
            "build passes"
        );
    }

    // Robust: empty / whitespace-only / clear-aliases / oversize bodies
    // all surface a structured error rather than silently activating.
    #[test]
    fn validate_condition_rejects_bad_inputs_robust() {
        assert!(validate_condition("").is_err());
        assert!(validate_condition("   ").is_err());
        assert!(validate_condition("clear").is_err());
        assert!(validate_condition("cancel").is_err());
        let long = "x".repeat(MAX_CONDITION_LEN + 1);
        assert!(validate_condition(&long).is_err());
    }

    // Normal: the clear-arg set matches Claude Code's vocabulary.
    #[test]
    fn is_clear_arg_recognises_aliases_normal() {
        for s in ["clear", "stop", "off", "reset", "none", "cancel"] {
            assert!(is_clear_arg(s), "{s} should clear");
            assert!(is_clear_arg(&s.to_uppercase()), "{s} upper");
        }
        assert!(!is_clear_arg("yes"));
        assert!(!is_clear_arg("done"));
    }

    // Normal: a well-formed JSON reply yields a real verdict.
    #[test]
    fn parse_verdict_parses_clean_json_normal() {
        let reply = "{\"ok\": true, \"reason\": \"PR merged at 1234abc\"}";
        let v = parse_verdict(reply).unwrap();
        assert!(v.ok);
        assert!(v.reason.contains("PR merged"));
    }

    // Robust: a model that emits a prose preface plus JSON still parses
    // because we extract the first balanced `{...}` block.
    #[test]
    fn parse_verdict_strips_prose_preface_robust() {
        let reply =
            "Here is the verdict:\n```json\n{\"ok\": false, \"reason\": \"build still red\"}\n```";
        let v = parse_verdict(reply).unwrap();
        assert!(!v.ok);
        assert!(v.reason.contains("build still red"));
    }

    // Robust: empty / unparseable replies default to "not met" — the loop
    // stays alive and reports the failure rather than silently stopping.
    #[test]
    fn parse_verdict_falls_back_to_unmet_on_garbage_robust() {
        let v = parse_verdict("").unwrap();
        assert!(!v.ok);
        let v = parse_verdict("totally not json").unwrap();
        assert!(!v.ok);
    }

    // Robust: nested JSON-with-strings doesn't trip the brace tracker
    // (e.g. when the reason itself contains escaped braces).
    #[test]
    fn parse_verdict_handles_nested_braces_robust() {
        let reply = r#"{"ok": false, "reason": "missing \"}\" in output"}"#;
        let v = parse_verdict(reply).unwrap();
        assert!(!v.ok);
        assert!(v.reason.contains("missing"));
    }

    // Normal: ActiveGoal::is_exhausted flips after MAX_ITERATIONS.
    #[test]
    fn active_goal_exhaustion_normal() {
        let mut g = ActiveGoal::new("x".into());
        assert!(!g.is_exhausted());
        g.iterations = MAX_ITERATIONS - 1;
        assert!(!g.is_exhausted());
        g.iterations = MAX_ITERATIONS;
        assert!(g.is_exhausted());
    }

    // Normal: success banner carries the marker tag the resume scan
    // looks for.
    #[test]
    fn success_banner_carries_marker_normal() {
        let g = ActiveGoal::new("ship it".into());
        let s = format_success_banner(&g, "PR merged");
        assert!(s.contains("[goal_status: met]"));
        assert!(s.contains("ship it"));
        assert!(s.contains("PR merged"));
    }

    // Robust: unmet reminder includes both the iteration counter and
    // the missing-piece narrative so the next turn knows what to do.
    #[test]
    fn unmet_reminder_includes_counter_and_missing_robust() {
        let s = format_unmet_reminder("tests pass", "cargo test still failing", 3);
        assert!(s.contains("3/"));
        assert!(s.contains("tests pass"));
        assert!(s.contains("cargo test still failing"));
    }

    // Robust: render_snapshot caps at MAX_SNAPSHOT_CHARS so a 2MB
    // transcript can't blow up the evaluator call.
    #[test]
    fn render_snapshot_caps_size_robust() {
        let huge = "x".repeat(MAX_SNAPSHOT_CHARS * 3);
        let msg = ChatMessage::user(huge);
        let snap = render_snapshot(&[msg]);
        assert!(snap.len() <= MAX_SNAPSHOT_CHARS + 200);
        assert!(snap.contains("transcript truncated"));
    }

    // Robust: truncate_snapshot_tail never panics on non-ASCII content.
    // The naive slice (out[cut..]) would panic if `cut` landed inside
    // a multi-byte UTF-8 sequence. We hand-roll a char-boundary walk
    // — this test would have caught the original byte-slice bug on a
    // transcript with emoji, accented identifiers, or CJK.
    #[test]
    fn render_snapshot_handles_non_ascii_at_truncation_boundary_robust() {
        // Each "é" is 2 bytes. We size the body so that the naive
        // cut would land inside one of them.
        let body = "é".repeat(MAX_SNAPSHOT_CHARS);
        let msg = ChatMessage::user(body);
        let snap = render_snapshot(&[msg]);
        // Round-trip parse — would panic if the slice was invalid UTF-8.
        let _ = snap.chars().count();
        assert!(snap.contains("transcript truncated"));
    }

    /// Test-local guard that restores `XDG_CONFIG_HOME` on drop.
    /// Avoids leaking a tmp path into the next test in the same
    /// process. `serial_test::serial` ensures these tests don't race
    /// each other, since env vars are process-global.
    struct XdgGuard {
        prev: Option<String>,
    }
    impl XdgGuard {
        fn set(path: &std::path::Path) -> Self {
            let prev = std::env::var("XDG_CONFIG_HOME").ok();
            unsafe { std::env::set_var("XDG_CONFIG_HOME", path) };
            Self { prev }
        }
    }
    impl Drop for XdgGuard {
        fn drop(&mut self) {
            unsafe {
                match self.prev.take() {
                    Some(v) => std::env::set_var("XDG_CONFIG_HOME", v),
                    None => std::env::remove_var("XDG_CONFIG_HOME"),
                }
            }
        }
    }

    // Normal: the sidecar round-trips an ActiveGoal through disk so
    // /continue can rebuild app.goal on resume.
    #[serial_test::serial]
    #[test]
    fn sidecar_round_trips_goal_normal() {
        let session_id = format!("ses_goal_sidecar_test_{}", std::process::id());
        let tmp = tempfile::tempdir().unwrap();
        let _guard = XdgGuard::set(tmp.path());

        let mut goal = ActiveGoal::new("ship it".into());
        goal.iterations = 7;
        goal.last_unmet_reason = Some("tests still failing".into());

        save_sidecar(&session_id, Some(&goal));
        let loaded = load_sidecar(&session_id).expect("sidecar present");
        assert_eq!(loaded.condition, "ship it");
        assert_eq!(loaded.iterations, 7);
        assert_eq!(
            loaded.last_unmet_reason.as_deref(),
            Some("tests still failing")
        );

        // Clearing with None removes the file.
        save_sidecar(&session_id, None);
        assert!(load_sidecar(&session_id).is_none());
    }

    // Robust: load_sidecar treats missing / corrupt files as "no goal"
    // rather than panicking — a busted sidecar shouldn't block resume.
    #[serial_test::serial]
    #[test]
    fn sidecar_corrupt_file_loads_as_none_robust() {
        let session_id = format!("ses_goal_corrupt_test_{}", std::process::id());
        let tmp = tempfile::tempdir().unwrap();
        let _guard = XdgGuard::set(tmp.path());

        let path = sidecar_path(&session_id);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "{ not valid json at all").unwrap();
        assert!(
            load_sidecar(&session_id).is_none(),
            "corrupt sidecar must not panic, must read as None"
        );
    }
}
