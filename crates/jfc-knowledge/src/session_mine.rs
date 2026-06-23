//! Stage 1 — mine the user's own session history into candidate lessons.
//!
//! Reads `~/.config/jfc/sessions/*.json` (verified shape:
//! `{id, cwd, messages:[{role, parts:[{type:"tool", kind, status, input,
//! output} | {type:"text"|"reasoning", content}]}]}`) and extracts two lesson
//! kinds, **deterministically** (no LLM):
//!
//! 1. **Error patterns** — a `status:"failed"` tool part followed by a later
//!    *successful* part of the same `kind` (the "recovery window"). The
//!    failed→succeeded pair is the **verifier**: we only record the lesson as
//!    [`Outcome::Verified`] when the transcript proves the fix worked. An
//!    unrecovered failure is recorded `Unverified` (lower rank) — it's a known
//!    rough edge, not a confirmed lesson.
//! 2. **Preferences** — a user `text` turn that *corrects* the immediately
//!    preceding assistant turn (negation cues: "no,", "don't", "actually",
//!    "stop", "instead").
//!
//! All extracted text is redacted (Stage 0) first. Lessons are project-scoped
//! candidates keyed by a `norm_key`, so the same lesson seen across many sessions
//! **compounds** (`support_count`) rather than duplicating. Nothing here promotes
//! to cross-project scope — that stays human-gated.

use std::path::Path;

use serde::Deserialize;

use crate::record::{Kind, Outcome};
use crate::redact::redact;

/// A mined lesson, ready to be folded into the candidate store.
#[derive(Debug, Clone, PartialEq)]
pub struct MinedLesson {
    pub kind: Kind,
    /// What triggers/contextualizes the lesson (e.g. the failing tool + message).
    pub trigger: String,
    /// The lesson claim (what to do / what the user prefers).
    pub claim: String,
    /// Verified iff backed by a failed→succeeded recovery in-transcript.
    pub outcome: Outcome,
    /// Normalized dedup key — identical lessons across sessions share it.
    pub norm_key: String,
    /// Source session id (provenance).
    pub session_id: String,
}

#[derive(Debug, Default)]
pub struct MineReport {
    pub sessions_scanned: usize,
    pub error_lessons: usize,
    pub preference_lessons: usize,
    pub verified: usize,
}

// ── Session JSON model (only the fields we need) ─────────────────────────────

#[derive(Debug, Deserialize)]
struct Session {
    #[serde(default)]
    id: String,
    #[serde(default)]
    messages: Vec<Message>,
}

#[derive(Debug, Deserialize)]
struct Message {
    #[serde(default)]
    role: String,
    #[serde(default)]
    parts: Vec<Part>,
}

#[derive(Debug, Deserialize)]
struct Part {
    #[serde(rename = "type", default)]
    ptype: String,
    // tool parts
    #[serde(default)]
    kind: Option<String>,
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    output: Option<serde_json::Value>,
    // text/reasoning parts
    #[serde(default)]
    content: Option<String>,
}

impl Part {
    fn text(&self) -> Option<&str> {
        self.content.as_deref()
    }

    fn output_text(&self) -> String {
        match &self.output {
            Some(serde_json::Value::String(s)) => s.clone(),
            Some(serde_json::Value::Object(map)) => map
                .get("content")
                .and_then(|v| v.as_str())
                .map(str::to_owned)
                .unwrap_or_default(),
            _ => String::new(),
        }
    }
}

/// Parse + mine a single session's raw JSON. `project_key` scopes the lessons.
/// Returns `None` if the JSON isn't a recognizable session.
pub fn mine_session_json(raw: &str) -> Vec<MinedLesson> {
    let Ok(session) = serde_json::from_str::<Session>(raw) else {
        return Vec::new();
    };
    let mut lessons = Vec::new();
    mine_errors(&session, &mut lessons);
    mine_preferences(&session, &mut lessons);
    lessons
}

/// Error patterns: each failed tool part, verified if a later same-kind part
/// succeeded (recovery window). Flatten parts in order so "later" is well-defined.
fn mine_errors(session: &Session, out: &mut Vec<MinedLesson>) {
    let parts: Vec<&Part> = session
        .messages
        .iter()
        .flat_map(|m| m.parts.iter())
        .collect();

    for (i, p) in parts.iter().enumerate() {
        if p.ptype != "tool" || p.status.as_deref() != Some("failed") {
            continue;
        }
        let kind = p.kind.as_deref().unwrap_or("tool");
        let msg_class = classify_error(&redact(&p.output_text(), true));
        if msg_class.is_empty() {
            continue;
        }
        // Recovery window: a later part of the same kind that completed.
        let recovered = parts[i + 1..].iter().take(12).any(|q| {
            q.ptype == "tool"
                && q.kind.as_deref() == Some(kind)
                && q.status.as_deref() == Some("complete")
        });

        let outcome = if recovered {
            Outcome::Verified
        } else {
            Outcome::Unverified
        };
        let trigger = format!("{kind} failed: {msg_class}");
        let claim = recovery_claim(kind, &msg_class, recovered);
        let norm_key = format!("err:{}:{}", kind.to_lowercase(), msg_class.to_lowercase());
        out.push(MinedLesson {
            kind: Kind::Finding,
            trigger,
            claim,
            outcome,
            norm_key,
            session_id: session.id.clone(),
        });
    }
}

/// Preferences: a user turn that negates the preceding assistant turn.
fn mine_preferences(session: &Session, out: &mut Vec<MinedLesson>) {
    for w in session.messages.windows(2) {
        let (prev, cur) = (&w[0], &w[1]);
        if prev.role != "assistant" || cur.role != "user" {
            continue;
        }
        let Some(user_text) = cur
            .parts
            .iter()
            .filter(|p| p.ptype == "text")
            .find_map(|p| p.text())
        else {
            continue;
        };
        let trimmed = user_text.trim();
        if !is_correction(trimmed) {
            continue;
        }
        let redacted = redact(trimmed, false);
        let claim = redacted.chars().take(280).collect::<String>();
        let norm_key = format!("pref:{}", normalize_claim(&claim));
        out.push(MinedLesson {
            kind: Kind::Preference,
            trigger: "user corrected the assistant".to_owned(),
            claim,
            // Preferences aren't verified by a recovery pair; they're observed.
            outcome: Outcome::Unverified,
            norm_key,
            session_id: session.id.clone(),
        });
    }
}

/// Normalize a tool's error output into a stable class string for dedup.
fn classify_error(output: &str) -> String {
    let o = output.to_lowercase();
    const CLASSES: &[(&str, &str)] = &[
        ("old_string not found", "old_string-not-found"),
        ("no such file", "no-such-file"),
        ("command not found", "command-not-found"),
        ("permission denied", "permission-denied"),
        ("modulenotfounderror", "module-not-found"),
        ("cannot find", "cannot-find"),
        ("timed out", "timeout"),
        ("did not match", "no-match"),
        ("expected", "parse-error"),
    ];
    for (needle, class) in CLASSES {
        if o.contains(needle) {
            return (*class).to_owned();
        }
    }
    String::new()
}

fn recovery_claim(kind: &str, class: &str, recovered: bool) -> String {
    let base = match class {
        "old_string-not-found" => {
            "When an Edit's old_string isn't found, re-read the exact current \
             bytes (including indentation/line-number gutters) before retrying."
        }
        "no-such-file" => "Verify the path exists (and the cwd) before operating on a file.",
        "command-not-found" => "Check the tool is installed / on PATH before invoking it.",
        "module-not-found" => "Install/declare the dependency before importing it.",
        "no-match" | "parse-error" => "Confirm the target text exactly before a search/replace.",
        _ => "Recheck preconditions before retrying this operation.",
    };
    if recovered {
        format!("{kind}: {base} (a later attempt succeeded in the same session.)")
    } else {
        format!("{kind}: {base}")
    }
}

/// Does a user message read as a correction of the preceding assistant turn?
fn is_correction(text: &str) -> bool {
    let t = text.trim_start().to_lowercase();
    const CUES: &[&str] = &[
        "no,",
        "no.",
        "no ",
        "don't",
        "dont",
        "do not",
        "actually",
        "stop",
        "instead",
        "that's wrong",
        "thats wrong",
        "incorrect",
        "not what i",
        "you should have",
        "why did you",
        "revert",
        "undo",
    ];
    // Keep it tight: short-ish messages that lead with a negation cue.
    text.len() < 400 && CUES.iter().any(|c| t.starts_with(c) || t.contains(c))
}

fn normalize_claim(s: &str) -> String {
    s.to_lowercase()
        .split_whitespace()
        .take(10)
        .collect::<Vec<_>>()
        .join(" ")
}

/// Mine every `ses_*.json` in a directory. Sidecar files (`.goal.json`, `.bak`)
/// are skipped. Pure parse+extract — the caller folds the results into the store.
pub fn mine_dir(dir: &Path) -> (Vec<MinedLesson>, MineReport) {
    let mut lessons = Vec::new();
    let mut report = MineReport::default();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return (lessons, report);
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = path.file_name().and_then(|s| s.to_str()).unwrap_or("");
        if !name.starts_with("ses_") || !name.ends_with(".json") || name.contains("goal") {
            continue;
        }
        let Ok(raw) = std::fs::read_to_string(&path) else {
            continue;
        };
        report.sessions_scanned += 1;
        for l in mine_session_json(&raw) {
            match l.kind {
                Kind::Preference => report.preference_lessons += 1,
                _ => report.error_lessons += 1,
            }
            if l.outcome == Outcome::Verified {
                report.verified += 1;
            }
            lessons.push(l);
        }
    }
    (lessons, report)
}

#[cfg(test)]
mod tests {
    use super::*;

    // A failed Edit later recovered by a successful Edit → one VERIFIED finding.
    #[test]
    fn failed_then_succeeded_edit_yields_verified_lesson_normal() {
        let raw = r#"{
          "id":"ses_test",
          "messages":[
            {"role":"assistant","parts":[
              {"type":"tool","kind":"Edit","status":"failed","output":{"type":"text","content":"old_string not found in /home/cole/x.rs"}}
            ]},
            {"role":"assistant","parts":[
              {"type":"tool","kind":"Edit","status":"complete","output":{"type":"text","content":"ok"}}
            ]}
          ]
        }"#;
        let lessons = mine_session_json(raw);
        let errs: Vec<_> = lessons.iter().filter(|l| l.kind == Kind::Finding).collect();
        assert_eq!(errs.len(), 1, "{lessons:?}");
        assert_eq!(errs[0].outcome, Outcome::Verified);
        assert_eq!(errs[0].norm_key, "err:edit:old_string-not-found");
    }

    // A failed tool with NO later recovery → an UNVERIFIED finding (not gated in).
    #[test]
    fn unrecovered_failure_is_unverified_robust() {
        let raw = r#"{
          "id":"s",
          "messages":[{"role":"assistant","parts":[
            {"type":"tool","kind":"Bash","status":"failed","output":{"type":"text","content":"command not found: foo"}}
          ]}]
        }"#;
        let lessons = mine_session_json(raw);
        assert_eq!(lessons.len(), 1);
        assert_eq!(lessons[0].outcome, Outcome::Unverified);
    }

    // A user negation after an assistant turn → a preference lesson.
    #[test]
    fn user_correction_yields_preference_normal() {
        let raw = r#"{
          "id":"s",
          "messages":[
            {"role":"assistant","parts":[{"type":"text","content":"I'll use tabs."}]},
            {"role":"user","parts":[{"type":"text","content":"No, always use spaces, not tabs."}]}
          ]
        }"#;
        let lessons = mine_session_json(raw);
        let prefs: Vec<_> = lessons
            .iter()
            .filter(|l| l.kind == Kind::Preference)
            .collect();
        assert_eq!(prefs.len(), 1, "{lessons:?}");
        assert!(prefs[0].claim.to_lowercase().contains("spaces"));
    }

    // A normal (non-correcting) user reply → no preference.
    #[test]
    fn non_correction_user_reply_is_ignored_robust() {
        let raw = r#"{
          "id":"s",
          "messages":[
            {"role":"assistant","parts":[{"type":"text","content":"Done."}]},
            {"role":"user","parts":[{"type":"text","content":"Great, thanks! Now add a test."}]}
          ]
        }"#;
        let prefs: Vec<_> = mine_session_json(raw)
            .into_iter()
            .filter(|l| l.kind == Kind::Preference)
            .collect();
        assert!(prefs.is_empty(), "{prefs:?}");
    }

    // Secrets in tool output must be redacted before they reach a lesson.
    #[test]
    fn mined_lesson_text_is_redacted_regression() {
        let raw = r#"{
          "id":"s",
          "messages":[{"role":"assistant","parts":[
            {"type":"tool","kind":"Bash","status":"failed","output":{"type":"text","content":"old_string not found token=ghp_0123456789abcdefghij"}}
          ]}]
        }"#;
        let lessons = mine_session_json(raw);
        // The classifier keys off "old_string not found"; the trigger carries the
        // redacted class, and no raw secret survives anywhere in the lesson.
        for l in &lessons {
            let blob = format!("{} {} {}", l.trigger, l.claim, l.norm_key);
            assert!(!blob.contains("ghp_0123456789"), "secret leaked: {blob}");
        }
    }

    #[test]
    fn malformed_json_is_skipped_robust() {
        assert!(mine_session_json("not json at all").is_empty());
        assert!(mine_session_json("{}").is_empty());
    }
}
