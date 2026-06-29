//! Structured review artifacts, Air-style review tools, and scoped prompt rules.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ReviewOutputEvent {
    pub schema_version: u8,
    pub run_id: String,
    pub created_at_ms: u128,
    pub source: String,
    pub target: Option<String>,
    pub files: Vec<String>,
    pub findings: Vec<ReviewFinding>,
    pub overall_correctness: String,
    pub overall_explanation: String,
    pub overall_confidence_score: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ReviewFinding {
    pub fingerprint: String,
    pub duplicate: bool,
    pub title: String,
    pub body: String,
    pub severity: Option<String>,
    pub category: Option<String>,
    pub confidence_score: Option<f64>,
    pub priority: Option<i32>,
    pub code_location: Option<ReviewCodeLocation>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ReviewCodeLocation {
    pub absolute_file_path: PathBuf,
    pub line_range: ReviewLineRange,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ReviewLineRange {
    pub start: u32,
    pub end: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ReviewComment {
    pub schema_version: u8,
    pub created_at_ms: u128,
    pub file_path: PathBuf,
    pub start_line: u32,
    pub end_line: u32,
    pub text: String,
    pub source: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SubmittedPlan {
    pub schema_version: u8,
    pub created_at_ms: u128,
    pub short_name: String,
    pub summary: String,
    pub plan: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CommitMessageSuggestion {
    pub schema_version: u8,
    pub created_at_ms: u128,
    pub scope: Option<String>,
    pub message: String,
}

pub fn normalize_review_output(
    cwd: &Path,
    run_id: &str,
    source: &str,
    args: &Value,
    result: &Value,
    existing_fingerprints: &HashSet<String>,
) -> ReviewOutputEvent {
    let created_at_ms = now_ms();
    let mut seen = HashSet::new();
    let findings = result
        .get("findings")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|item| normalize_finding(cwd, item, existing_fingerprints, &mut seen))
        .collect::<Vec<_>>();
    ReviewOutputEvent {
        schema_version: 1,
        run_id: run_id.to_owned(),
        created_at_ms,
        source: source.to_owned(),
        target: args
            .get("target")
            .and_then(Value::as_str)
            .map(str::to_owned),
        files: args
            .get("files")
            .and_then(Value::as_array)
            .map(|files| {
                files
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default(),
        findings,
        overall_correctness: result
            .get("overall_correctness")
            .and_then(Value::as_str)
            .unwrap_or_else(|| {
                if result
                    .get("findings")
                    .and_then(Value::as_array)
                    .is_some_and(Vec::is_empty)
                {
                    "patch is correct"
                } else {
                    "patch correctness requires review"
                }
            })
            .to_owned(),
        overall_explanation: result
            .get("overall_explanation")
            .or_else(|| result.get("final_report"))
            .or_else(|| result.get("summary"))
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_owned(),
        overall_confidence_score: result
            .get("overall_confidence_score")
            .or_else(|| result.get("confidence"))
            .and_then(Value::as_f64)
            .unwrap_or(0.0),
    }
}

fn normalize_finding(
    cwd: &Path,
    item: &Value,
    existing_fingerprints: &HashSet<String>,
    seen: &mut HashSet<String>,
) -> ReviewFinding {
    let title = string_field(item, "title")
        .or_else(|| string_field(item, "summary"))
        .unwrap_or_else(|| "Review finding".to_owned());
    let body = string_field(item, "body")
        .or_else(|| string_field(item, "evidence"))
        .or_else(|| string_field(item, "message"))
        .unwrap_or_default();
    let code_location = normalize_location(cwd, item);
    let fingerprint = finding_fingerprint(&title, &body, code_location.as_ref(), item);
    let duplicate =
        existing_fingerprints.contains(&fingerprint) || !seen.insert(fingerprint.clone());
    ReviewFinding {
        fingerprint,
        duplicate,
        title,
        body,
        severity: string_field(item, "severity"),
        category: string_field(item, "category"),
        confidence_score: item
            .get("confidence_score")
            .or_else(|| item.get("confidence"))
            .and_then(Value::as_f64),
        priority: item
            .get("priority")
            .and_then(Value::as_i64)
            .map(|value| value.clamp(i32::MIN as i64, i32::MAX as i64) as i32),
        code_location,
    }
}

fn normalize_location(cwd: &Path, item: &Value) -> Option<ReviewCodeLocation> {
    if let Some(location) = item.get("code_location") {
        let path = string_field(location, "absolute_file_path")
            .or_else(|| string_field(location, "file_path"))
            .or_else(|| string_field(location, "file"))?;
        let range = location.get("line_range").unwrap_or(location);
        let start = range
            .get("start")
            .or_else(|| range.get("start_line"))
            .or_else(|| item.get("line"))
            .and_then(Value::as_u64)
            .unwrap_or(1)
            .max(1) as u32;
        let end = range
            .get("end")
            .or_else(|| range.get("end_line"))
            .and_then(Value::as_u64)
            .unwrap_or(start as u64)
            .max(start as u64) as u32;
        return Some(ReviewCodeLocation {
            absolute_file_path: normalize_path(cwd, &path),
            line_range: ReviewLineRange { start, end },
        });
    }

    let path = string_field(item, "file").or_else(|| string_field(item, "file_path"))?;
    let start = item
        .get("line")
        .or_else(|| item.get("start_line"))
        .and_then(Value::as_u64)
        .unwrap_or(1)
        .max(1) as u32;
    let end = item
        .get("end_line")
        .and_then(Value::as_u64)
        .unwrap_or(start as u64)
        .max(start as u64) as u32;
    Some(ReviewCodeLocation {
        absolute_file_path: normalize_path(cwd, &path),
        line_range: ReviewLineRange { start, end },
    })
}

pub fn validate_review_comment(
    cwd: &Path,
    file_path: &str,
    start_line: u32,
    end_line: u32,
    text: &str,
) -> Result<ReviewComment, String> {
    let text = text.trim();
    if text.is_empty() {
        return Err("review comment text must not be empty".to_owned());
    }
    let start_line = start_line.max(1);
    let end_line = end_line.max(start_line);
    if end_line.saturating_sub(start_line) + 1 > 30 {
        return Err("review comment range must cover 30 lines or fewer".to_owned());
    }
    let path = normalize_path(cwd, file_path);
    if !path.exists() {
        return Err(format!(
            "review comment file does not exist: {}",
            path.display()
        ));
    }
    Ok(ReviewComment {
        schema_version: 1,
        created_at_ms: now_ms(),
        file_path: path,
        start_line,
        end_line,
        text: text.to_owned(),
        source: "tool".to_owned(),
    })
}

pub async fn persist_review_output(cwd: &Path, review: &ReviewOutputEvent) -> std::io::Result<()> {
    append_review_artifact(cwd, "review_events", &review.run_id, review).await
}

pub async fn persist_review_comment(cwd: &Path, comment: &ReviewComment) -> std::io::Result<()> {
    append_review_artifact(
        cwd,
        "comments",
        &format!(
            "{}:{}:{}",
            comment.file_path.display(),
            comment.start_line,
            comment.created_at_ms
        ),
        comment,
    )
    .await
}

pub async fn persist_submitted_plan(cwd: &Path, plan: &SubmittedPlan) -> std::io::Result<()> {
    append_review_artifact(cwd, "submitted_plans", &plan.short_name, plan).await
}

pub async fn persist_commit_message_suggestion(
    cwd: &Path,
    suggestion: &CommitMessageSuggestion,
) -> std::io::Result<()> {
    append_review_artifact(
        cwd,
        "commit_messages",
        &suggestion.created_at_ms.to_string(),
        suggestion,
    )
    .await
}

pub fn submitted_plan(short_name: String, summary: String, plan: String) -> SubmittedPlan {
    SubmittedPlan {
        schema_version: 1,
        created_at_ms: now_ms(),
        short_name,
        summary,
        plan,
    }
}

pub fn commit_message_suggestion(
    scope: Option<String>,
    message: String,
) -> CommitMessageSuggestion {
    CommitMessageSuggestion {
        schema_version: 1,
        created_at_ms: now_ms(),
        scope,
        message,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PromptRule {
    pub id: &'static str,
    pub required_tools: &'static [&'static str],
    pub body: &'static str,
}

impl PromptRule {
    pub fn applies_to(&self, tool_names: &HashSet<String>) -> bool {
        self.required_tools
            .iter()
            .any(|tool| tool_names.iter().any(|name| tool_name_matches(name, tool)))
    }
}

#[derive(Debug, Clone, Default)]
pub struct PromptRuleRegistry {
    rules: Vec<PromptRule>,
}

impl PromptRuleRegistry {
    pub fn air_style_defaults() -> Self {
        Self {
            rules: vec![
                PromptRule {
                    id: "review_comments",
                    required_tools: &[
                        "addreviewcomment",
                        "add_review_comment",
                        "add_comment",
                        "mcp__air__add_comment",
                    ],
                    body: "### Review comments\n\
                         When a review comment tool is available, use it only for concrete, \
                         actionable defects. Each comment must name one issue, cite a tight \
                         line range of 30 lines or fewer, and explain the required fix. Do \
                         not add compliments, style-only remarks, or vague concerns.",
                },
                PromptRule {
                    id: "submit_plan",
                    required_tools: &["submitplan", "submit_plan"],
                    body: "### Submit plan\n\
                         When the task is in planning/review mode and SubmitPlan is \
                         available, submit the implementation plan through the tool and \
                         stop. Keep the plan specific: files, intended changes, validation, \
                         and risks.",
                },
                PromptRule {
                    id: "commit_messages",
                    required_tools: &["suggestcommitmessage", "suggest_commit_message"],
                    body: "### Commit messages\n\
                         When suggesting a commit message, emit one concise conventional \
                         message through SuggestCommitMessage after inspecting the actual \
                         diff. Do not invent changes that are not in the diff.",
                },
                PromptRule {
                    id: "codegraph_priority",
                    required_tools: &["codegraph"],
                    body: "### CodeGraph priority\n\
                         For codebase structure questions, call one CodeGraph tool before \
                         broad Read/Grep unless the user named an exact file/path or the \
                         graph tool is unavailable. Use Read after CodeGraph identifies \
                         the relevant file or symbol.",
                },
            ],
        }
    }

    pub fn push(&mut self, rule: PromptRule) {
        self.rules.push(rule);
    }

    pub fn render_for_tools(&self, tool_names: &[String]) -> Option<String> {
        let names = tool_names
            .iter()
            .map(|name| name.to_ascii_lowercase())
            .collect::<HashSet<_>>();
        let sections = self
            .rules
            .iter()
            .filter(|rule| rule.applies_to(&names))
            .map(|rule| rule.body)
            .collect::<Vec<_>>();
        if sections.is_empty() {
            None
        } else {
            Some(format!(
                "## Tool-scoped prompt rules\n\n{}",
                sections.join("\n\n")
            ))
        }
    }
}

pub fn tool_scoped_prompt_rules(tool_names: &[String]) -> Option<String> {
    PromptRuleRegistry::air_style_defaults().render_for_tools(tool_names)
}

fn tool_name_matches(candidate: &str, expected: &str) -> bool {
    candidate == expected || candidate.contains(expected)
}

pub const REVIEW_ARTIFACT_SESSION_ID: &str = "__review__";
pub const REVIEW_ARTIFACT_KIND: &str = "review";

async fn append_review_artifact<T: Serialize>(
    cwd: &Path,
    stream: &str,
    key: &str,
    value: &T,
) -> std::io::Result<()> {
    let project_key = jfc_knowledge::project_key(cwd);
    let artifact_key = format!("{project_key}:{stream}:{key}");
    let value_json = serde_json::to_string(value).map_err(std::io::Error::other)?;
    tokio::task::spawn_blocking(move || {
        jfc_knowledge::block_on_knowledge(async {
            let store = jfc_knowledge::KnowledgeStore::open_default()
                .await
                .map_err(std::io::Error::other)?;
            store
                .append_session_artifact_event(
                    REVIEW_ARTIFACT_SESSION_ID,
                    REVIEW_ARTIFACT_KIND,
                    &artifact_key,
                    &value_json,
                )
                .await
                .map_err(std::io::Error::other)?;
            Ok(())
        })
    })
    .await
    .map_err(std::io::Error::other)?
}

fn normalize_path(cwd: &Path, path: &str) -> PathBuf {
    let path = PathBuf::from(path);
    if path.is_absolute() {
        path
    } else {
        cwd.join(path)
    }
}

fn finding_fingerprint(
    title: &str,
    body: &str,
    location: Option<&ReviewCodeLocation>,
    raw: &Value,
) -> String {
    let mut hasher = Sha256::new();
    hasher.update(title.trim().to_ascii_lowercase().as_bytes());
    hasher.update([0]);
    hasher.update(body.trim().to_ascii_lowercase().as_bytes());
    hasher.update([0]);
    if let Some(location) = location {
        hasher.update(location.absolute_file_path.to_string_lossy().as_bytes());
        hasher.update([0]);
        hasher.update(location.line_range.start.to_le_bytes());
        hasher.update(location.line_range.end.to_le_bytes());
    } else {
        hasher.update(raw.to_string().as_bytes());
    }
    hex::encode(&hasher.finalize()[..16])
}

fn string_field(value: &Value, key: &str) -> Option<String> {
    value.get(key).and_then(Value::as_str).map(str::to_owned)
}

pub fn now_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}

#[cfg(test)]
mod tests {
    use super::*;

    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn normalize_codex_style_finding_normal() {
        let raw = serde_json::json!({
            "findings": [{
                "title": "Missing validation",
                "body": "The branch accepts invalid input.",
                "confidence_score": 0.91,
                "priority": 1,
                "code_location": {
                    "absolute_file_path": "src/lib.rs",
                    "line_range": { "start": 10, "end": 12 }
                }
            }],
            "overall_correctness": "patch is incorrect",
            "overall_explanation": "There is one issue.",
            "overall_confidence_score": 0.8
        });
        let review = normalize_review_output(
            Path::new("/tmp/project"),
            "run_1",
            "auto",
            &serde_json::json!({"files": ["src/lib.rs"]}),
            &raw,
            &HashSet::new(),
        );
        assert_eq!(review.findings.len(), 1);
        assert_eq!(review.findings[0].title, "Missing validation");
        assert_eq!(
            review.findings[0]
                .code_location
                .as_ref()
                .unwrap()
                .absolute_file_path,
            PathBuf::from("/tmp/project/src/lib.rs")
        );
    }

    #[test]
    fn validate_review_comment_rejects_wide_range_robust() {
        let err = validate_review_comment(Path::new("."), "Cargo.toml", 1, 40, "fix")
            .expect_err("wide range should be rejected");
        assert!(err.contains("30 lines"));
    }

    #[test]
    fn prompt_rule_registry_filters_by_active_tools_normal() {
        let rules = PromptRuleRegistry::air_style_defaults();
        let none = rules.render_for_tools(&["Read".to_owned()]);
        assert!(none.is_none());

        let rendered = rules
            .render_for_tools(&[
                "AddReviewComment".to_owned(),
                "mcp__codegraph__codegraph_explore".to_owned(),
            ])
            .expect("matching tools should render scoped rules");
        assert!(rendered.contains("Review comments"));
        assert!(rendered.contains("CodeGraph priority"));
        assert!(!rendered.contains("Commit messages"));
    }

    #[tokio::test]
    async fn review_artifacts_persist_to_db_normal() {
        let tmp = tempfile::TempDir::new().unwrap();
        let _guard = db_env_guard(tmp.path());
        let cwd = tmp.path().join("repo");
        tokio::fs::create_dir_all(&cwd).await.unwrap();
        let review = ReviewOutputEvent {
            schema_version: 1,
            run_id: "run_1".to_owned(),
            created_at_ms: 1,
            source: "test".to_owned(),
            target: Some("diff".to_owned()),
            files: vec!["src/lib.rs".to_owned()],
            findings: Vec::new(),
            overall_correctness: "patch is correct".to_owned(),
            overall_explanation: "ok".to_owned(),
            overall_confidence_score: 0.9,
        };
        persist_review_output(&cwd, &review).await.unwrap();
        let comment_path = cwd.join("src/lib.rs");
        tokio::fs::create_dir_all(comment_path.parent().unwrap())
            .await
            .unwrap();
        tokio::fs::write(&comment_path, "fn main() {}\n")
            .await
            .unwrap();
        let comment = validate_review_comment(&cwd, "src/lib.rs", 1, 1, "tighten this").unwrap();
        persist_review_comment(&cwd, &comment).await.unwrap();

        let project_key = jfc_knowledge::project_key(&cwd);
        let rows = jfc_knowledge::KnowledgeStore::open_default()
            .await
            .unwrap()
            .list_recent_session_artifact_events(
                REVIEW_ARTIFACT_SESSION_ID,
                REVIEW_ARTIFACT_KIND,
                None,
                10,
            )
            .await
            .unwrap();
        assert!(
            rows.iter()
                .any(|row| row.key == format!("{project_key}:review_events:run_1"))
        );
        assert!(
            rows.iter()
                .any(|row| row.key.starts_with(&format!("{project_key}:comments:")))
        );
    }

    fn db_env_guard(root: &Path) -> DbEnvGuard {
        let guard = ENV_LOCK.lock().unwrap_or_else(|error| error.into_inner());
        let prior = std::env::var("JFC_KNOWLEDGE_DB").ok();
        unsafe {
            std::env::set_var("JFC_KNOWLEDGE_DB", root.join("knowledge.db"));
        }
        DbEnvGuard {
            prior,
            _guard: guard,
        }
    }

    struct DbEnvGuard {
        prior: Option<String>,
        _guard: std::sync::MutexGuard<'static, ()>,
    }

    impl Drop for DbEnvGuard {
        fn drop(&mut self) {
            unsafe {
                match self.prior.take() {
                    Some(prior) => std::env::set_var("JFC_KNOWLEDGE_DB", prior),
                    None => std::env::remove_var("JFC_KNOWLEDGE_DB"),
                }
            }
        }
    }
}
