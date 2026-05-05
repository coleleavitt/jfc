//! Cargo-check diagnostic producer.
//!
//! Spawns `cargo check --message-format=json` as a background watcher and
//! converts each compiler-message JSON line into a `DiagnosticEntry`.
//! Emits `AppEvent::DiagnosticsUpdated { entries }` whenever the build
//! completes (one snapshot per check run, replacing whatever was there).
//!
//! This is the most useful "LSP-style" producer for a Rust project —
//! it's what `rust-analyzer`'s `cargo.checkOnSave` hook actually does
//! under the hood, and it's testable without standing up a JSON-RPC
//! server. A full LSP client (initialize handshake, didChange,
//! publishDiagnostics over stdio) would be a meaningful next layer; this
//! covers the common case.
//!
//! ## Triggering
//!
//! The watcher runs once on startup, then re-runs whenever the user
//! invokes the `/check` slash command (or on a periodic interval — both
//! orchestrations call into `run_once`). Calling re-entrantly is safe:
//! each call spawns a fresh `cargo` process.
//!
//! ## JSON shape we parse
//!
//! ```text
//! {"reason":"compiler-message","message":{
//!   "level":"error",
//!   "message":"unresolved import",
//!   "code":{"code":"E0432","explanation":null},
//!   "spans":[{"file_name":"src/main.rs","line_start":12,"column_start":5,
//!             "is_primary":true,...}],
//!   ...
//! }}
//! ```

use crate::app::AppEvent;
use crate::diagnostics::{DiagnosticEntry, Severity};
use serde::Deserialize;
use std::path::PathBuf;
use std::process::Stdio;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tokio::sync::mpsc::UnboundedSender;

/// Spawn `cargo check --message-format=json` from `cwd`. Streams each
/// line through `parse_cargo_message`; on process exit, sends one
/// `AppEvent::DiagnosticsUpdated` carrying the accumulated set. Errors
/// (cargo missing, non-cargo project) silently no-op — better to leave
/// the row blank than spam the user.
pub async fn run_once(cwd: PathBuf, tx: UnboundedSender<AppEvent>) {
    tracing::info!(
        target: "jfc::diagnostics",
        ?cwd,
        "cargo check starting"
    );
    let mut child = match Command::new("cargo")
        .args(["check", "--message-format=json", "--quiet"])
        .current_dir(&cwd)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
    {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!(
                target: "jfc::diagnostics",
                error = %e,
                "failed to spawn cargo check"
            );
            return;
        }
    };
    let Some(stdout) = child.stdout.take() else {
        return;
    };
    let mut reader = BufReader::new(stdout).lines();
    let mut entries: Vec<DiagnosticEntry> = Vec::new();
    while let Ok(Some(line)) = reader.next_line().await {
        if let Some(entry) = parse_cargo_message(&line) {
            entries.push(entry);
        }
    }
    let _ = child.wait().await;
    tracing::info!(
        target: "jfc::diagnostics",
        count = entries.len(),
        "cargo check complete"
    );
    let _ = tx.send(AppEvent::DiagnosticsUpdated { entries });
}

#[derive(Deserialize)]
struct CargoLine<'a> {
    reason: &'a str,
    #[serde(default)]
    message: Option<CargoMessage<'a>>,
}

#[derive(Deserialize)]
struct CargoMessage<'a> {
    level: &'a str,
    message: String,
    #[serde(default)]
    code: Option<CargoCode>,
    #[serde(default)]
    spans: Vec<CargoSpan>,
}

#[derive(Deserialize)]
struct CargoCode {
    code: String,
}

#[derive(Deserialize)]
struct CargoSpan {
    file_name: String,
    line_start: u32,
    column_start: u32,
    #[serde(default)]
    is_primary: bool,
}

/// Pure parser. Returns `Some(entry)` for `compiler-message` lines that
/// carry at least one primary span. Skips non-compiler lines (build
/// progress, warnings without spans, internal compiler chatter).
pub fn parse_cargo_message(line: &str) -> Option<DiagnosticEntry> {
    let parsed: CargoLine = serde_json::from_str(line).ok()?;
    if parsed.reason != "compiler-message" {
        return None;
    }
    let msg = parsed.message?;
    let severity = match msg.level {
        "error" | "error: internal compiler error" => Severity::Error,
        "warning" => Severity::Warning,
        "note" => Severity::Info,
        "help" => Severity::Hint,
        _ => return None,
    };
    // The "primary" span is the one cargo points an arrow at. Spans
    // without a primary usually mean a multi-file diagnostic where the
    // root cause was in a different file we already reported — skip.
    let primary = msg.spans.iter().find(|s| s.is_primary)?;
    let entry = DiagnosticEntry {
        file: primary.file_name.clone(),
        line: primary.line_start,
        col: primary.column_start,
        message: msg.message,
        code: msg.code.map(|c| c.code),
        source: Some("cargo".into()),
        severity,
    };
    tracing::trace!(
        target: "jfc::diagnostics",
        file = %entry.file,
        line = entry.line,
        severity = ?entry.severity,
        "parsed diagnostic"
    );
    Some(entry)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_error_with_code_and_span_normal() {
        let line = r#"{"reason":"compiler-message","message":{
            "level":"error","message":"unresolved import",
            "code":{"code":"E0432","explanation":null},
            "spans":[{"file_name":"src/main.rs","line_start":12,"line_end":12,
                      "column_start":5,"column_end":15,"is_primary":true,
                      "byte_start":0,"byte_end":0}]
        }}"#;
        let entry = parse_cargo_message(line).expect("must parse");
        assert_eq!(entry.severity, Severity::Error);
        assert_eq!(entry.file, "src/main.rs");
        assert_eq!(entry.line, 12);
        assert_eq!(entry.col, 5);
        assert_eq!(entry.message, "unresolved import");
        assert_eq!(entry.code.as_deref(), Some("E0432"));
        assert_eq!(entry.source.as_deref(), Some("cargo"));
    }

    #[test]
    fn parse_warning_normal() {
        let line = r#"{"reason":"compiler-message","message":{
            "level":"warning","message":"unused variable",
            "spans":[{"file_name":"a.rs","line_start":3,"line_end":3,
                      "column_start":1,"column_end":2,"is_primary":true,
                      "byte_start":0,"byte_end":0}]
        }}"#;
        let entry = parse_cargo_message(line).expect("must parse");
        assert_eq!(entry.severity, Severity::Warning);
        assert!(entry.code.is_none());
    }

    #[test]
    fn skip_non_compiler_message_normal() {
        // Build progress, artifact, build-finished — all non-compiler.
        for line in [
            r#"{"reason":"build-script-executed","package_id":"x"}"#,
            r#"{"reason":"compiler-artifact","package_id":"x"}"#,
            r#"{"reason":"build-finished","success":true}"#,
        ] {
            assert!(parse_cargo_message(line).is_none(), "should skip {line}");
        }
    }

    #[test]
    fn skip_message_without_primary_span_robust() {
        // A diagnostic with spans but none flagged primary — cargo
        // sometimes attaches secondary-only spans (e.g. "previously
        // defined here"). We surface only the primary in our row.
        let line = r#"{"reason":"compiler-message","message":{
            "level":"error","message":"shadowed",
            "spans":[{"file_name":"a.rs","line_start":1,"line_end":1,
                      "column_start":1,"column_end":2,"is_primary":false,
                      "byte_start":0,"byte_end":0}]
        }}"#;
        assert!(parse_cargo_message(line).is_none());
    }

    #[test]
    fn skip_message_without_spans_robust() {
        // Some compiler chatter has no spans at all (linker errors).
        let line = r#"{"reason":"compiler-message","message":{
            "level":"error","message":"linking failed","spans":[]
        }}"#;
        assert!(parse_cargo_message(line).is_none());
    }

    #[test]
    fn skip_unknown_severity_robust() {
        // Future cargo versions might add new levels — don't panic;
        // skip them silently rather than guess.
        let line = r#"{"reason":"compiler-message","message":{
            "level":"trace","message":"x",
            "spans":[{"file_name":"a.rs","line_start":1,"line_end":1,
                      "column_start":1,"column_end":2,"is_primary":true,
                      "byte_start":0,"byte_end":0}]
        }}"#;
        assert!(parse_cargo_message(line).is_none());
    }

    #[test]
    fn malformed_json_no_panic_robust() {
        // Truncated/corrupt JSON should yield None, never panic.
        for line in ["", "{", r#"{"reason":"compiler-message"}"#, "not-json"] {
            assert!(parse_cargo_message(line).is_none());
        }
    }

    #[test]
    fn picks_primary_among_many_spans_normal() {
        // Multi-span errors: the *primary* span is what we attribute
        // the diagnostic to. Cargo emits non-primary spans for
        // related code locations that helped produce the error.
        let line = r#"{"reason":"compiler-message","message":{
            "level":"error","message":"mismatched types",
            "spans":[
                {"file_name":"helper.rs","line_start":3,"line_end":3,
                 "column_start":1,"column_end":2,"is_primary":false,
                 "byte_start":0,"byte_end":0},
                {"file_name":"main.rs","line_start":42,"line_end":42,
                 "column_start":17,"column_end":25,"is_primary":true,
                 "byte_start":0,"byte_end":0},
                {"file_name":"types.rs","line_start":10,"line_end":10,
                 "column_start":5,"column_end":12,"is_primary":false,
                 "byte_start":0,"byte_end":0}
            ]
        }}"#;
        let entry = parse_cargo_message(line).expect("must parse");
        assert_eq!(entry.file, "main.rs");
        assert_eq!(entry.line, 42);
        assert_eq!(entry.col, 17);
    }
}
