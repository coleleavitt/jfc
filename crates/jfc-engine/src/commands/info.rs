//! Slash handlers: inspection, diagnostics & VCS review.

use crate::commands::prelude::*;
use crate::runtime::EngineEvent;

fn active_cwd(cwd: &str) -> std::path::PathBuf {
    std::path::PathBuf::from(cwd)
}

const REVIEW_DIFF_CAP: usize = 12_000;

fn cap_review_diff(diff_output: String) -> String {
    if diff_output.len() > REVIEW_DIFF_CAP {
        let cap = diff_output.floor_char_boundary(REVIEW_DIFF_CAP);
        format!(
            "{}\n\n[... diff truncated at 12000 chars ...]",
            &diff_output[..cap]
        )
    } else {
        diff_output
    }
}

pub(super) async fn cmd_diff(
    state: &mut EngineState,
    _parts: &[&str],
    text: &str,
    _tx: Option<&mpsc::Sender<EngineEvent>>,
) {
    // Show pending uncommitted + unstaged changes via `git diff
    // HEAD --stat`. Read-only; doesn't run unless we're in a
    // git repo. Surface in the transcript as an assistant
    // message (markdown code block) so the user — and the
    // model on the next turn — can see what's pending.
    state.messages.push(ChatMessage::user(text.to_owned()));
    let cwd = active_cwd(&state.cwd);
    let in_repo = std::process::Command::new("git")
        .args(["rev-parse", "--is-inside-work-tree"])
        .current_dir(&cwd)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);
    if !in_repo {
        state.messages.push(ChatMessage::assistant(
            "Not inside a git repository — `/diff` has nothing to show.".into(),
        ));
        return;
    }
    let stat = std::process::Command::new("git")
        .args(["diff", "HEAD", "--stat"])
        .current_dir(&cwd)
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .unwrap_or_default();
    let untracked = std::process::Command::new("git")
        .args(["ls-files", "--others", "--exclude-standard"])
        .current_dir(&cwd)
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .unwrap_or_default();
    if stat.trim().is_empty() && untracked.trim().is_empty() {
        state.messages.push(ChatMessage::assistant(
            "Working tree is clean — no pending changes.".into(),
        ));
    } else {
        let mut body = String::from("**Pending changes (`git diff HEAD`):**\n\n```\n");
        if !stat.trim().is_empty() {
            body.push_str(&stat);
        } else {
            body.push_str("(no tracked-file changes)\n");
        }
        if !untracked.trim().is_empty() {
            body.push_str("\n--- untracked ---\n");
            body.push_str(&untracked);
        }
        body.push_str("```\n");
        state.messages.push(ChatMessage::assistant(body));
    }
}

/// `/turn-diff` (`/td`) — show a `git diff` scoped to only the files the
/// assistant edited during the current user turn, so a single agentic step
/// can be reviewed without the noise of the whole working tree.
pub(super) async fn cmd_turn_diff(
    state: &mut EngineState,
    _parts: &[&str],
    text: &str,
    _tx: Option<&mpsc::Sender<EngineEvent>>,
) {
    state.messages.push(ChatMessage::user(text.to_owned()));
    if state.turn_edited_files.is_empty() {
        state.messages.push(ChatMessage::assistant(
            "No files edited this turn yet — `/turn-diff` has nothing to show.".into(),
        ));
        return;
    }
    let cwd = active_cwd(&state.cwd);
    let files: Vec<String> = state.turn_edited_files.iter().cloned().collect();
    // `git diff HEAD -- <files>` shows tracked-file changes; brand-new files
    // (created by Write) won't appear, so list those separately.
    let mut args: Vec<String> = vec!["diff".into(), "HEAD".into(), "--".into()];
    args.extend(files.iter().cloned());
    let diff = std::process::Command::new("git")
        .args(&args)
        .current_dir(&cwd)
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .unwrap_or_default();
    let new_files: Vec<&String> = files
        .iter()
        .filter(|f| {
            std::process::Command::new("git")
                .args(["ls-files", "--error-unmatch", f])
                .current_dir(&cwd)
                .output()
                .map(|o| !o.status.success())
                .unwrap_or(false)
        })
        .collect();

    let mut body = format!(
        "**Turn diff** — {} file{} edited this turn:\n\n```diff\n",
        files.len(),
        if files.len() == 1 { "" } else { "s" }
    );
    if diff.trim().is_empty() && new_files.is_empty() {
        body.push_str("(edits were reverted, or match HEAD — nothing to show)\n");
    } else {
        // Cap to keep a giant diff from flooding the transcript.
        const CAP: usize = 12_000;
        if diff.len() > CAP {
            body.push_str(&diff[..diff.floor_char_boundary(CAP)]);
            body.push_str("\n… (truncated; run `git diff HEAD` for the rest)\n");
        } else {
            body.push_str(&diff);
        }
    }
    body.push_str("```\n");
    if !new_files.is_empty() {
        body.push_str("\n_New files this turn:_ ");
        body.push_str(
            &new_files
                .iter()
                .map(|s| format!("`{s}`"))
                .collect::<Vec<_>>()
                .join(", "),
        );
        body.push('\n');
    }
    state.messages.push(ChatMessage::assistant(body));
}

pub(super) async fn cmd_timeline(
    state: &mut EngineState,
    _parts: &[&str],
    text: &str,
    _tx: Option<&mpsc::Sender<EngineEvent>>,
) {
    // Render a chronological tool-call timeline for the most
    // recent assistant turn. For each Tool part, emit one row
    // with "kind │ summary │ Δms" so the user can spot slow
    // tools at a glance.
    state.messages.push(ChatMessage::user(text.to_owned()));
    let last_assistant = state
        .messages
        .iter()
        .rposition(|m| matches!(m.role, jfc_core::Role::Assistant));
    let Some(idx) = last_assistant else {
        state.messages.push(ChatMessage::assistant(
            "No assistant turn yet — nothing to timeline.".into(),
        ));
        return;
    };
    let msg = &state.messages[idx];
    let mut rows: Vec<String> = Vec::new();
    for part in &msg.parts {
        if let jfc_core::MessagePart::Tool(tc) = part {
            let elapsed = tc
                .elapsed_ms
                .map(|ms| {
                    if ms >= 1_000 {
                        format!("{:.1}s", ms as f64 / 1000.0)
                    } else {
                        format!("{ms}ms")
                    }
                })
                .unwrap_or_else(|| "—".to_owned());
            let summary = tc.input.summary();
            let summary: String = summary.chars().take(60).collect();
            rows.push(format!(
                "  - **{}** · `{}` · {elapsed}",
                tc.kind.label(),
                summary,
            ));
        }
    }
    if rows.is_empty() {
        state.messages.push(ChatMessage::assistant(
            "Most recent assistant turn ran no tools.".into(),
        ));
    } else {
        state.messages.push(ChatMessage::assistant(format!(
            "**Tool timeline (last assistant turn, {} tools):**\n{}",
            rows.len(),
            rows.join("\n"),
        )));
    }
}

pub(super) async fn cmd_doctor(
    state: &mut EngineState,
    _parts: &[&str],
    text: &str,
    _tx: Option<&mpsc::Sender<EngineEvent>>,
) {
    // Mirrors Claude Code 2.1.139's /doctor command.
    // Health check: scan the most-likely failure modes for an
    // out-of-the-box jfc setup and surface a single status
    // block. Read-only; no fixes applied automatically — the
    // user opts in to remedies after seeing the report.
    state.messages.push(ChatMessage::user(text.to_owned()));

    let check = |ok: bool| if ok { "✓" } else { "✗" };

    let mut report = String::from("jfc doctor report\n─────────────────\n");

    // ── 1. Config file ────────────────────────────────────────────────
    {
        let cfg_path = crate::config::config_path();
        let cfg_display = cfg_path.display().to_string();
        // Tilde-shorten for readability
        let cfg_display = if let Some(home) = dirs::home_dir() {
            cfg_display.replacen(&home.display().to_string(), "~", 1)
        } else {
            cfg_display
        };
        let cfg_ok = cfg_path.exists() && {
            // Try a parse round-trip to catch TOML errors
            std::fs::read_to_string(&cfg_path)
                .ok()
                .and_then(|s| toml::from_str::<crate::config::Config>(&s).ok())
                .is_some()
        };
        report.push_str(&format!(
            "{} Config: {}{}\n",
            check(cfg_ok),
            cfg_display,
            if cfg_ok {
                ""
            } else if !cfg_path.exists() {
                " (not found)"
            } else {
                " (parse error)"
            },
        ));
    }

    // ── 2. Auth: ANTHROPIC_API_KEY env ───────────────────────────────
    {
        let api_key_set = std::env::var("ANTHROPIC_API_KEY").is_ok();
        report.push_str(&format!(
            "{} Auth: ANTHROPIC_API_KEY {}\n",
            check(api_key_set),
            if api_key_set { "set" } else { "not set" },
        ));
    }

    // ── 3. Auth: ~/.config/jfc/anthropic-accounts.json ───────────────
    {
        let accounts_path = dirs::config_dir()
            .unwrap_or_else(|| std::path::PathBuf::from("."))
            .join("jfc")
            .join("anthropic-accounts.json");
        let accounts_ok = accounts_path.exists();
        let accounts_display = {
            let s = accounts_path.display().to_string();
            if let Some(home) = dirs::home_dir() {
                s.replacen(&home.display().to_string(), "~", 1)
            } else {
                s
            }
        };
        report.push_str(&format!(
            "{} Auth: accounts file {} {}\n",
            check(accounts_ok),
            accounts_display,
            if accounts_ok {
                "(found)"
            } else {
                "(not found)"
            },
        ));
    }

    // ── 4. CLAUDE.md in project root ──────────────────────────────────
    {
        let project_root = std::path::PathBuf::from(&state.cwd);
        let claude_md = project_root.join("CLAUDE.md");
        let md_ok = claude_md.exists();
        let md_display = format!(
            "{}{}",
            "./",
            claude_md
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("CLAUDE.md")
        );
        report.push_str(&format!(
            "{} CLAUDE.md: {}\n",
            check(md_ok),
            if md_ok {
                md_display
            } else {
                format!("{} (not found)", md_display)
            },
        ));
    }

    // ── 5. MCP servers ────────────────────────────────────────────────
    {
        let cfg = crate::config::load_arc();
        if cfg.mcp.is_empty() {
            report.push_str("  MCP: no servers configured\n");
        } else {
            for (name, server) in &cfg.mcp {
                // Determine the binary to probe: use `command` if set,
                // otherwise the first element of `args` (e.g. npx), and
                // fall back to the server name itself.
                let probe_bin = server
                    .command
                    .as_deref()
                    .filter(|s| !s.is_empty())
                    .or_else(|| server.args.first().map(|s| s.as_str()))
                    .unwrap_or(name.as_str());
                let found = std::process::Command::new("which")
                    .arg(probe_bin)
                    .output()
                    .map(|o| o.status.success())
                    .unwrap_or(false);
                report.push_str(&format!(
                    "{} MCP: {} ({} {})\n",
                    check(found),
                    name,
                    probe_bin,
                    if found { "found" } else { "not found" },
                ));
            }
        }
    }

    // ── 6. Working directory + git repo ───────────────────────────────
    {
        let cwd = std::path::PathBuf::from(&state.cwd);
        let git_ok = std::process::Command::new("git")
            .args(["rev-parse", "--git-dir"])
            .current_dir(&cwd)
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);
        // Grab current branch name when inside a git repo
        let branch = if git_ok {
            std::process::Command::new("git")
                .args(["rev-parse", "--abbrev-ref", "HEAD"])
                .current_dir(&cwd)
                .output()
                .ok()
                .and_then(|o| {
                    if o.status.success() {
                        String::from_utf8(o.stdout)
                            .ok()
                            .map(|s| s.trim().to_owned())
                    } else {
                        None
                    }
                })
                .unwrap_or_else(|| "unknown".to_owned())
        } else {
            String::new()
        };
        let git_label = if git_ok {
            format!("yes (branch: {branch})")
        } else {
            "no".to_owned()
        };
        report.push_str(&format!("{} Git repo: {}\n", check(git_ok), git_label));
        report.push_str(&format!("  cwd: {}\n", cwd.display()));
    }

    // ── 7. Version ────────────────────────────────────────────────────
    report.push_str(&format!("  Version: {}\n", env!("CARGO_PKG_VERSION")));

    // ── 8. Bonus: active provider + permission mode ───────────────────
    report.push_str(&format!("  Provider: {}\n", state.provider.name()));
    report.push_str(&format!("  Permission mode: {:?}\n", state.permission_mode));

    // ── 9. Session cost so far ────────────────────────────────────────
    let total = crate::cost::total_cost(&state.usage_by_model);
    report.push_str(&format!(
        "  Session cost: {}\n",
        crate::cost::fmt_cost(total)
    ));

    state.messages.push(ChatMessage::assistant(report));
}
pub(super) async fn cmd_commit(
    state: &mut EngineState,
    _parts: &[&str],
    _text: &str,
    _tx: Option<&mpsc::Sender<EngineEvent>>,
) {
    // Generate a conventional commit message for staged changes.
    // 1. Check if anything is staged; bail early if not.
    // 2. Capture `git diff --cached` (capped at 8000 chars).
    // 3. Inject a user prompt so the model generates the message
    //    on the next turn — the user can then copy/run `git commit`.
    state.messages.push(ChatMessage::user("/commit".into()));
    let cwd = state.cwd.clone();
    let stat = tokio::process::Command::new("git")
        .args(["diff", "--cached", "--stat"])
        .current_dir(&cwd)
        .output()
        .await;
    match stat {
        Err(e) => {
            state.messages.push(ChatMessage::assistant(format!(
                "Could not run `git diff --cached --stat`: {e}"
            )));
        }
        Ok(out) => {
            let stat_str = String::from_utf8_lossy(&out.stdout);
            if stat_str.trim().is_empty() {
                state.messages.push(ChatMessage::assistant(
                    "Nothing staged. Stage changes first with `git add <file>` or `git add -p`."
                        .into(),
                ));
            } else {
                // Fetch the full diff, capped at 8000 chars to stay
                // well within any reasonable context window.
                let diff_output = tokio::process::Command::new("git")
                    .args(["diff", "--cached"])
                    .current_dir(&cwd)
                    .output()
                    .await
                    .ok();
                let diff_str = diff_output
                    .map(|o| {
                        let s = String::from_utf8_lossy(&o.stdout).into_owned();
                        if s.len() > 8000 {
                            // floor_char_boundary instead of a raw
                            // byte slice — git diff can carry
                            // non-ASCII filenames or content and
                            // a fixed-byte cap would panic if a
                            // multi-byte glyph straddled byte 8000.
                            let cap = s.floor_char_boundary(8000);
                            format!("{}\n\n[... diff truncated at 8000 chars ...]", &s[..cap])
                        } else {
                            s
                        }
                    })
                    .unwrap_or_default();
                let prompt = format!(
                    "Generate a conventional commit message for these staged changes.\n\
                             Format: `type(scope): description`\n\
                             Types: feat / fix / docs / style / refactor / test / chore\n\
                             Rules: imperative mood, ≤72 chars subject, no trailing period.\n\
                             Output ONLY the commit message — no explanation, no markdown fences.\n\n\
                             ```\n{diff_str}\n```"
                );
                state
                    .messages
                    .push(ChatMessage::assistant("Analyzing staged changes…".into()));
                state.queued_prompts.push(crate::runtime::QueuedPrompt {
                    text: prompt,
                    is_meta: false,
                    priority: crate::runtime::QueuePriority::Later,
                    attachments: Vec::new(),
                });
                state.push_effect(crate::app::EngineEffect::ScrollToBottom);
            }
        }
    }
}

pub(super) async fn cmd_review(
    state: &mut EngineState,
    parts: &[&str],
    text: &str,
    tx: Option<&mpsc::Sender<EngineEvent>>,
) {
    let req = parse_review_request(parts);
    state.messages.push(ChatMessage::user(text.to_owned()));

    if req.level.uses_workflow() {
        dispatch_code_review_workflow(state, &req, tx).await;
        return;
    }

    let cwd = state.cwd.clone();
    // Prefer staged diff; fall back to HEAD diff; fall back to
    // working-tree diff so /review always finds something useful.
    let diff_output = {
        let staged = tokio::process::Command::new("git")
            .args(["diff", "--cached"])
            .current_dir(&cwd)
            .output()
            .await
            .ok();
        let staged_str = staged
            .as_ref()
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_owned())
            .unwrap_or_default();
        if !staged_str.is_empty() {
            staged_str
        } else {
            tokio::process::Command::new("git")
                .args(["diff", "HEAD"])
                .current_dir(&cwd)
                .output()
                .await
                .ok()
                .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_owned())
                .unwrap_or_default()
        }
    };
    if diff_output.is_empty() {
        state.messages.push(ChatMessage::assistant(
            "No changes found (`git diff --cached` and `git diff HEAD` are both empty). \
                     Make some changes or stage files first."
                .into(),
        ));
    } else {
        let capped = cap_review_diff(diff_output);
        let target = req.target_or_default();
        let prompt = format!(
            "Review level: {}.\nTarget: {}.\n\n\
                     Review the following git diff for bugs, security issues, and code quality \
                     problems. Be specific — reference exact file names and line numbers where \
                     relevant. Organise findings by severity (Critical / High / Medium / Low). \
                     If there are no issues worth calling out, say so briefly.\n\n\
                     ```diff\n{capped}\n```",
            req.level.as_str(),
            target,
        );
        state
            .messages
            .push(ChatMessage::assistant("Reviewing changes…".into()));
        state.queued_prompts.push(crate::runtime::QueuedPrompt {
            text: prompt,
            is_meta: false,
            priority: crate::runtime::QueuePriority::Later,
            attachments: Vec::new(),
        });
        state.push_effect(crate::app::EngineEffect::ScrollToBottom);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ReviewLevel {
    Low,
    Medium,
    High,
    XHigh,
    Max,
    Ultra,
}

impl ReviewLevel {
    fn parse(value: &str) -> Option<Self> {
        match value.to_ascii_lowercase().as_str() {
            "low" => Some(Self::Low),
            "medium" | "med" => Some(Self::Medium),
            "high" => Some(Self::High),
            "xhigh" | "extra-high" | "extra_high" => Some(Self::XHigh),
            "max" => Some(Self::Max),
            "ultra" | "ultrareview" => Some(Self::Ultra),
            _ => None,
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
            Self::XHigh => "xhigh",
            Self::Max => "max",
            Self::Ultra => "ultra",
        }
    }

    fn workflow_level(self) -> &'static str {
        match self {
            Self::Ultra => "max",
            _ => self.as_str(),
        }
    }

    fn uses_workflow(self) -> bool {
        matches!(self, Self::High | Self::XHigh | Self::Max | Self::Ultra)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ReviewRequest {
    level: ReviewLevel,
    target: String,
}

impl ReviewRequest {
    fn target_or_default(&self) -> &str {
        if self.target.is_empty() {
            "current git diff"
        } else {
            &self.target
        }
    }
}

fn parse_review_request(parts: &[&str]) -> ReviewRequest {
    let rest = parts.get(1).copied().unwrap_or("").trim();
    let default_level = match parts.first().copied().unwrap_or("") {
        cmd if cmd.eq_ignore_ascii_case("/code-review") => ReviewLevel::High,
        cmd if cmd.eq_ignore_ascii_case("/ultrareview") => ReviewLevel::Ultra,
        _ => ReviewLevel::Medium,
    };
    if rest.is_empty() {
        return ReviewRequest {
            level: default_level,
            target: String::new(),
        };
    }

    let mut words = rest.splitn(2, char::is_whitespace);
    let first = words.next().unwrap_or("");
    let tail = words.next().unwrap_or("").trim();
    if let Some(level) = ReviewLevel::parse(first) {
        ReviewRequest {
            level,
            target: tail.to_owned(),
        }
    } else {
        ReviewRequest {
            level: default_level,
            target: rest.to_owned(),
        }
    }
}

async fn dispatch_code_review_workflow(
    state: &mut EngineState,
    req: &ReviewRequest,
    tx: Option<&mpsc::Sender<EngineEvent>>,
) {
    let cwd = state.cwd.clone();
    if crate::workflows::resolve(std::path::Path::new(&cwd), "code-review").is_none() {
        state.messages.push(ChatMessage::assistant(
            "Workflow `code-review` is not available. List workflows with `/workflow`.".into(),
        ));
        return;
    }

    if req.level == ReviewLevel::Ultra {
        state.messages.push(ChatMessage::assistant(
            "Cloud UltraReview is not implemented yet; dispatching local `code-review` at max effort.".into(),
        ));
    }

    let Some(tx) = tx else {
        state.messages.push(ChatMessage::assistant(
            "Code review workflow needs the event channel; called from a context without one."
                .into(),
        ));
        return;
    };

    let args = serde_json::json!({
        "level": req.level.workflow_level(),
        "target": req.target_or_default(),
    });
    let prompt = format!(
        "Run the saved workflow named \"code-review\" by calling the Workflow tool: \
         Workflow({{ name: \"code-review\", args: {} }}). Do not describe it — call the tool.",
        args
    );
    let _ = tx
        .send(crate::runtime::EngineEvent::Control(
            crate::runtime::ControlEvent::SubmitPrompt(prompt),
        ))
        .await;
    state.messages.push(ChatMessage::assistant(format!(
        "Dispatching `code-review` workflow at `{}` effort for `{}`…",
        req.level.workflow_level(),
        req.target_or_default(),
    )));
}

#[cfg(test)]
mod review_tests {
    use super::*;

    #[test]
    fn active_cwd_uses_engine_state_value_regression() {
        let cwd = active_cwd("/tmp/jfc-active-workspace");

        assert_eq!(cwd, std::path::PathBuf::from("/tmp/jfc-active-workspace"));
    }

    #[test]
    fn cap_review_diff_uses_utf8_boundary_regression() {
        let diff = format!("{}🙂tail", "a".repeat(REVIEW_DIFF_CAP - 1));

        let capped = cap_review_diff(diff);

        assert!(capped.contains("[... diff truncated at 12000 chars ...]"));
        assert!(!capped.contains("🙂"));
    }

    #[test]
    fn parse_review_request_defaults_to_medium_normal() {
        let req = parse_review_request(&["/review"]);
        assert_eq!(req.level, ReviewLevel::Medium);
        assert_eq!(req.target_or_default(), "current git diff");
    }

    #[test]
    fn parse_review_request_extracts_level_and_target_normal() {
        let req = parse_review_request(&["/code-review", "xhigh origin/main"]);
        assert_eq!(req.level, ReviewLevel::XHigh);
        assert_eq!(req.target, "origin/main");
    }

    #[test]
    fn parse_code_review_defaults_to_high_normal() {
        let req = parse_review_request(&["/code-review"]);
        assert_eq!(req.level, ReviewLevel::High);
        assert_eq!(req.target_or_default(), "current git diff");
    }

    #[test]
    fn parse_review_request_treats_unknown_first_word_as_target_robust() {
        let req = parse_review_request(&["/review", "feature/login"]);
        assert_eq!(req.level, ReviewLevel::Medium);
        assert_eq!(req.target, "feature/login");
    }

    #[test]
    fn parse_ultrareview_defaults_to_ultra_normal() {
        let req = parse_review_request(&["/ultrareview", "origin/main"]);
        assert_eq!(req.level, ReviewLevel::Ultra);
        assert_eq!(req.target, "origin/main");
    }

    #[test]
    fn review_level_routes_high_and_above_to_workflow_normal() {
        assert!(!ReviewLevel::Low.uses_workflow());
        assert!(!ReviewLevel::Medium.uses_workflow());
        assert!(ReviewLevel::High.uses_workflow());
        assert!(ReviewLevel::XHigh.uses_workflow());
        assert!(ReviewLevel::Max.uses_workflow());
        assert!(ReviewLevel::Ultra.uses_workflow());
        assert_eq!(ReviewLevel::Ultra.workflow_level(), "max");
    }
}

pub(super) async fn cmd_skills(
    state: &mut EngineState,
    _parts: &[&str],
    _text: &str,
    _tx: Option<&mpsc::Sender<EngineEvent>>,
) {
    let cwd = active_cwd(&state.cwd);
    let skills = crate::agents::load_skills(&cwd);
    let visible: Vec<_> = skills
        .iter()
        .filter(|skill| skill.is_discoverable())
        .collect();
    let body = if visible.is_empty() {
        "No user-invocable skills defined. Create or import DB-backed skill definitions.".to_owned()
    } else {
        // Compute column width for alignment
        let max_name_len = visible.iter().map(|s| s.name.len()).max().unwrap_or(10);
        let pad = max_name_len + 2;
        let mut s = String::from("Available Skills:\n");
        s.push_str(&"\u{2500}".repeat(pad + 40));
        s.push('\n');
        for sk in visible {
            let desc = sk.description.as_deref().unwrap_or("(no description)");
            // Truncate long descriptions for readability
            let desc_trunc: String = desc.chars().take(60).collect();
            let suffix = if desc.chars().count() > 60 {
                "\u{2026}"
            } else {
                ""
            };
            let mut meta = Vec::new();
            if sk.context.is_fork() {
                meta.push("fork".to_owned());
            }
            if !sk.files.is_empty() {
                meta.push(format!("{} files", sk.files.len()));
            }
            if let Some(schedule) = sk.schedule.as_deref().filter(|s| !s.trim().is_empty()) {
                meta.push(format!("schedule {schedule}"));
            }
            let meta = if meta.is_empty() {
                String::new()
            } else {
                format!(" [{}]", meta.join(", "))
            };
            s.push_str(&format!(
                "{:<width$}\u{2014} {}{suffix}{meta}\n",
                sk.name,
                desc_trunc,
                width = pad,
            ));
        }
        s
    };
    state.messages.push(ChatMessage::user("/skills".into()));
    state.messages.push(ChatMessage::assistant(body));
}

pub(super) async fn cmd_agents(
    state: &mut EngineState,
    _parts: &[&str],
    _text: &str,
    _tx: Option<&mpsc::Sender<EngineEvent>>,
) {
    let cwd = active_cwd(&state.cwd);
    let agents = crate::agents::load_agents(&cwd);
    let body = if agents.is_empty() {
        "No agent definitions found. Create or import DB-backed agent definitions \
                 with `name` plus optional `model`, `permissionMode`, `allowedTools`, \
                 `disallowedTools`, `skills`, `isolation`, and `forksParentContext`; \
                 the definition body becomes the system prompt for spawned subagents/teammates."
            .to_owned()
    } else {
        let mut s = format!("**{} agent(s) loaded:**\n\n", agents.len());
        for a in &agents {
            s.push_str(&format!(
                "- **{}** — model: {}, permission: {:?}, isolation: {}\n  \
                         tools: allowed={:?}, denied={:?}\n  source: `{}`\n",
                a.name,
                a.model.as_deref().unwrap_or("inherit"),
                a.permission_mode.unwrap_or_default(),
                a.isolation.as_deref().unwrap_or("none"),
                a.allowed_tools,
                a.disallowed_tools,
                a.source.display(),
            ));
        }
        s
    };
    state.messages.push(ChatMessage::user("/agents".into()));
    state.messages.push(ChatMessage::assistant(body));
}

pub(super) async fn cmd_market(
    state: &mut EngineState,
    _parts: &[&str],
    _text: &str,
    _tx: Option<&mpsc::Sender<EngineEvent>>,
) {
    // Surface the agent-economy snapshot — same data the
    // `market_status` tool returns, but framed for the user
    // rather than the model. No bounty_id filter for now.
    let cwd = active_cwd(&state.cwd);
    let report_str = match crate::tools::market_report_string_for_cwd(&cwd).await {
        Ok(s) => s,
        Err(e) => format!("Market unavailable: {e}"),
    };
    state.messages.push(ChatMessage::user("/market".into()));
    state.messages.push(ChatMessage::assistant(report_str));
}

/// `/hooks` — show registered hooks with per-rule activation metrics.
///
/// Reads the global `HookRegistry`'s metrics snapshot and renders a markdown
/// table of every registered handler. Columns: Hook Point, index, fire count,
/// last-fired timestamp (UTC), average execution ms.
///
/// If the registry is empty (headless tests, hooks feature disabled) it
/// shows a brief notice instead of an empty table.
pub(super) async fn cmd_hooks(
    state: &mut EngineState,
    _parts: &[&str],
    _text: &str,
    _tx: Option<&mpsc::Sender<EngineEvent>>,
) {
    state.messages.push(ChatMessage::user("/hooks".into()));

    let summary = crate::hooks::registered_hooks_summary();
    let metrics = crate::hooks::metrics_snapshot();

    if summary.is_empty() {
        state.messages.push(ChatMessage::assistant(
            "No hooks registered (global registry empty or `hooks` feature disabled).".into(),
        ));
        return;
    }

    let mut body = String::from("## Registered hooks\n\n");
    body.push_str("| Hook Point | # | Fires | Last Fired (UTC) | Avg ms |\n");
    body.push_str("|---|---|---|---|---|\n");

    for (point, idx) in &summary {
        let key = format!("{point:?}#{idx}");
        let (fires, last_str, avg_ms) = if let Some(m) = metrics.get(&key) {
            let last = m
                .last_fired_at
                .map(|t| {
                    // Format as YYYY-MM-DD HH:MM:SS UTC without chrono dep.
                    let secs = t
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_secs();
                    let s = secs % 60;
                    let mins = (secs / 60) % 60;
                    let h = (secs / 3600) % 24;
                    let days = secs / 86400;
                    let mut y = 1970u32;
                    let mut d = days as u32;
                    loop {
                        let ydays = if is_leap_year(y) { 366 } else { 365 };
                        if d < ydays {
                            break;
                        }
                        d -= ydays;
                        y += 1;
                    }
                    let months = [31u32, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
                    let leap = is_leap_year(y);
                    let mut mo = 1u32;
                    for &dim_base in &months {
                        let dim = if mo == 2 && leap {
                            dim_base + 1
                        } else {
                            dim_base
                        };
                        if d < dim {
                            break;
                        }
                        d -= dim;
                        mo += 1;
                    }
                    format!("{y}-{mo:02}-{:02} {h:02}:{mins:02}:{s:02}", d + 1)
                })
                .unwrap_or_else(|| "—".to_owned());
            let avg = if m.fire_count > 0 {
                format!("{:.1}", m.total_duration_ms as f64 / m.fire_count as f64)
            } else {
                "—".into()
            };
            (m.fire_count, last, avg)
        } else {
            (0, "—".into(), "—".into())
        };
        body.push_str(&format!(
            "| `{point:?}` | {idx} | {fires} | {last_str} | {avg_ms} |\n"
        ));
    }

    body.push_str(&format!("\n*{} handler(s) registered.*\n", summary.len()));

    state.messages.push(ChatMessage::assistant(body));
}

/// `/bashes` — list background shell tasks; `/bashes kill <id|all>` cancels them.
/// Surfaces the real engine roster (running + recently-settled) so the user can
/// see what's running and actually stop it, instead of the symbolic `N shells`
/// footer badge.
pub(super) async fn cmd_bashes(
    state: &mut EngineState,
    parts: &[&str],
    _text: &str,
    _tx: Option<&mpsc::Sender<EngineEvent>>,
) {
    state.messages.push(ChatMessage::user("/bashes".into()));

    // Sub-command: `/bashes kill <id>` or `/bashes kill all`.
    let sub = parts.get(1).map(|s| s.trim()).unwrap_or("");
    if let Some(rest) = sub
        .strip_prefix("kill")
        .or_else(|| sub.strip_prefix("cancel"))
    {
        let target = rest.trim();
        let body = if target.is_empty() {
            "Usage: `/bashes kill <task_id>` or `/bashes kill all`.".to_owned()
        } else if target == "all" {
            let running: Vec<String> = crate::tools::list_bash_tasks()
                .await
                .into_iter()
                .filter(|t| t.running)
                .map(|t| t.id)
                .collect();
            if running.is_empty() {
                "No running background shells to cancel.".to_owned()
            } else {
                let mut cancelled = 0usize;
                for id in &running {
                    if crate::tools::cancel_bash_task(id).await
                        == crate::tools::CancelOutcome::Cancelled
                    {
                        cancelled += 1;
                    }
                }
                format!("Cancelled {cancelled} running background shell(s).")
            }
        } else {
            match crate::tools::cancel_bash_task(target).await {
                crate::tools::CancelOutcome::Cancelled => {
                    format!(
                        "Cancelled background shell `{target}` (SIGKILL sent to its process tree)."
                    )
                }
                crate::tools::CancelOutcome::AlreadyFinished => {
                    format!("Background shell `{target}` had already finished.")
                }
                crate::tools::CancelOutcome::Unknown => {
                    format!("No background shell with id `{target}` is tracked in this session.")
                }
            }
        };
        state.messages.push(ChatMessage::assistant(body));
        return;
    }

    let tasks = crate::tools::list_bash_tasks().await;
    if tasks.is_empty() {
        state.messages.push(ChatMessage::assistant(
            "No background shells. A command runs in the background when invoked with \
             `run_in_background=true` or after it exceeds the foreground budget."
                .into(),
        ));
        return;
    }

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or_default();
    let running = tasks.iter().filter(|t| t.running).count();
    let mut body = format!(
        "**{} background shell(s)** ({running} running):\n\n",
        tasks.len()
    );
    for t in &tasks {
        let marker = if t.running { "▶" } else { "■" };
        let elapsed_ms = t
            .completed_at_ms
            .unwrap_or(now)
            .saturating_sub(t.started_at_ms);
        let cmd_preview: String = t.command.chars().take(80).collect();
        body.push_str(&format!(
            "- {marker} `{}` — {} · {}s · {} lines\n  {}\n",
            t.id,
            t.status,
            elapsed_ms / 1000,
            t.total_lines,
            cmd_preview,
        ));
    }
    if running > 0 {
        body.push_str("\n*Cancel with `/bashes kill <id>` or `/bashes kill all`.*\n");
    }
    state.messages.push(ChatMessage::assistant(body));
}

fn is_leap_year(y: u32) -> bool {
    y.is_multiple_of(4) && (!y.is_multiple_of(100) || y.is_multiple_of(400))
}

fn recall_query_text(text: &str) -> &str {
    let trimmed = text.trim();
    let Some(idx) = trimmed.find(char::is_whitespace) else {
        return "";
    };
    trimmed[idx..].trim()
}

fn try_render_compaction_archive_by_id(query: &str) -> Option<String> {
    if query.split_whitespace().count() != 1 {
        return None;
    }
    crate::compact_archive::render_archive_by_id(query)
        .or_else(|| crate::context_accounting::render_provider_history_archive_by_id(query))
}

/// `/expand <archive-id>` — reopen the exact raw messages that were replaced by
/// a compaction boundary. With no id, lists recent compaction archives.
pub(super) async fn cmd_expand(
    state: &mut EngineState,
    _parts: &[&str],
    text: &str,
    _tx: Option<&mpsc::Sender<EngineEvent>>,
) {
    state.messages.push(ChatMessage::user(text.to_owned()));
    let query = recall_query_text(text);

    let body = if query.is_empty() {
        let archives = crate::compact_archive::list_archives(10);
        let provider_archives = crate::context_accounting::list_provider_history_archives(10);
        if archives.is_empty() && provider_archives.is_empty() {
            "No context archives found. Archives are created the next time `/compact` or provider-history overflow handling replaces raw messages.".to_owned()
        } else {
            let mut s = String::from("Recent context archives (use `/expand <id>`):\n");
            for a in archives {
                s.push_str(&format!(
                    "  compact  {}  {}  ({} msgs)\n    ...{}\n",
                    a.id,
                    a.created_at.chars().take(19).collect::<String>(),
                    a.message_count,
                    a.snippet.chars().take(120).collect::<String>(),
                ));
            }
            for a in provider_archives {
                s.push_str(&format!(
                    "  provider-history  {}  {}  ({} msgs)\n    ...{}\n",
                    a.id,
                    a.created_at.chars().take(19).collect::<String>(),
                    a.message_count,
                    a.snippet.chars().take(120).collect::<String>(),
                ));
            }
            s
        }
    } else if let Some(rendered) = try_render_compaction_archive_by_id(query) {
        rendered
    } else {
        let archives = crate::compact_archive::search_archives(query, 5);
        let provider_archives =
            crate::context_accounting::search_provider_history_archives(query, 5);
        if archives.is_empty() && provider_archives.is_empty() {
            format!("No context archive matched `{query}`.")
        } else {
            let mut s = format!("Context archives matching `{query}`:\n");
            for a in archives {
                s.push_str(&format!(
                    "  compact  {}  {}  ({} msgs)\n    ...{}\n",
                    a.id,
                    a.created_at.chars().take(19).collect::<String>(),
                    a.message_count,
                    a.snippet.chars().take(120).collect::<String>(),
                ));
            }
            for a in provider_archives {
                s.push_str(&format!(
                    "  provider-history  {}  {}  ({} msgs)\n    ...{}\n",
                    a.id,
                    a.created_at.chars().take(19).collect::<String>(),
                    a.message_count,
                    a.snippet.chars().take(120).collect::<String>(),
                ));
            }
            s
        }
    };

    state.messages.push(ChatMessage::assistant(body));
}

/// `/reloadSettings` (`/reload-settings`) — bust the config cache so the next
/// `load_arc()` call re-reads `~/.config/jfc/config.toml` (and any project
/// overrides) from disk. Useful after editing the config file without wanting
/// to restart the session.
pub(super) async fn cmd_reload_settings(
    state: &mut EngineState,
    _parts: &[&str],
    text: &str,
    _tx: Option<&mpsc::Sender<EngineEvent>>,
) {
    state.messages.push(ChatMessage::user(text.to_owned()));
    crate::config::invalidate_cache();
    // Eagerly re-parse so the user sees the new values immediately (and any
    // parse errors surface in the response rather than silently deferring).
    let path = crate::config::config_path();
    let cfg = crate::config::load_arc();
    let preview = match toml::to_string_pretty(&cfg) {
        Ok(s) if s.trim().is_empty() => "(no local overrides — using defaults)".to_owned(),
        Ok(s) => format!("```toml\n{s}```"),
        Err(e) => format!("**Warning — could not serialize reloaded config:** {e}"),
    };
    state.messages.push(ChatMessage::assistant(format!(
        "**Settings reloaded** from `{}`.\n\n{preview}",
        path.display()
    )));
}

#[cfg(test)]
mod recall_command_tests {
    use super::*;

    #[test]
    fn command_query_preserves_spaces_normal() {
        assert_eq!(
            recall_query_text("/recall cache and resume"),
            "cache and resume"
        );
        assert_eq!(
            recall_query_text("  /search-sessions   claude cache  "),
            "claude cache"
        );
    }
}

#[cfg(test)]
mod bashes_tests {
    use super::*;
    use futures::stream::empty;
    use jfc_provider::TokenUsage;
    use jfc_provider::{CompletionResponse, ModelInfo, Provider, ProviderMessage, StreamOptions};
    use std::sync::Arc;

    struct NoopProvider;
    impl jfc_provider::seal::Sealed for NoopProvider {}
    #[async_trait::async_trait]
    impl Provider for NoopProvider {
        fn name(&self) -> &str {
            "noop"
        }
        fn available_models(&self) -> Vec<ModelInfo> {
            Vec::new()
        }
        async fn stream(
            &self,
            _m: Vec<ProviderMessage>,
            _o: &StreamOptions,
        ) -> anyhow::Result<jfc_provider::EventStream> {
            Ok(Box::pin(empty()))
        }
        async fn complete(
            &self,
            _m: Vec<ProviderMessage>,
            _o: &StreamOptions,
        ) -> anyhow::Result<CompletionResponse> {
            Ok(CompletionResponse {
                content: String::new(),
                usage: TokenUsage::default(),
                context_signals: None,
                reasoning: None,
            })
        }
    }

    fn test_state() -> EngineState {
        EngineState::new(Arc::new(NoopProvider), "test-model")
    }

    fn last_text(state: &EngineState) -> String {
        state.messages.last().unwrap().parts[0].text_only()
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn bashes_kill_unknown_id_reports_unknown_normal() {
        let mut state = test_state();
        cmd_bashes(
            &mut state,
            &["/bashes", "kill bash_not_a_real_task"],
            "/bashes kill bash_not_a_real_task",
            None,
        )
        .await;
        assert!(
            last_text(&state).contains("No background shell with id"),
            "got: {}",
            last_text(&state)
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn bashes_kill_without_target_shows_usage_normal() {
        let mut state = test_state();
        cmd_bashes(&mut state, &["/bashes", "kill"], "/bashes kill", None).await;
        assert!(last_text(&state).contains("Usage: `/bashes kill"));
    }
}
