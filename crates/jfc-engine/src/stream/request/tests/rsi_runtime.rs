use std::sync::Arc;

use jfc_provider::{ModelId, Provider, StreamConvention};

use super::super::prepare_stream_request;
use super::{TestProvider, user_text};

#[tokio::test]
#[serial_test::serial]
async fn prepare_consumes_active_rsi_runtime_definitions_normal() {
    let _env = KnowledgeDbEnvGuard::set();
    let cwd = std::env::current_dir().unwrap();
    let project_key = jfc_knowledge::project_key(&cwd);
    let store = jfc_knowledge::KnowledgeStore::open_default().await.unwrap();
    for definition in active_rsi_definitions(&project_key) {
        store.upsert_definition(&definition).await.unwrap();
    }
    let provider: Arc<dyn Provider> = Arc::new(TestProvider {
        name: "openai-test",
        convention: StreamConvention::OpenAiNative,
    });

    let request = prepare_stream_request(
        provider,
        &[user_text("run cargo test and fix failures")],
        &ModelId::new("test-model"),
        Default::default(),
    )
    .await;

    let system = request.opts.system.as_deref().unwrap_or_default();
    assert!(
        system.contains("## Active RSI Runtime Guidance"),
        "{system}"
    );
    assert!(system.contains("RSI_PROMPT_SENTINEL"), "{system}");
    assert!(system.contains("RSI_SKILL_SENTINEL"), "{system}");
    assert!(system.contains("RSI_HARNESS_SENTINEL"), "{system}");
    assert!(system.contains("RSI_CONTEXT_SENTINEL"), "{system}");
    assert!(system.contains("RSI_BUDGET_SENTINEL"), "{system}");
    assert!(system.contains("RSI_REASONING_SENTINEL"), "{system}");
    assert!(system.contains("show earlier: `Bash`"), "{system}");

    let bash = request
        .opts
        .tools
        .iter()
        .find(|tool| tool.name == "Bash")
        .expect("Bash should be advertised for a cargo test request");
    assert!(
        bash.description.contains("RSI_TOOL_SENTINEL"),
        "{}",
        bash.description
    );
}

fn active_rsi_definitions(project_key: &str) -> Vec<jfc_knowledge::NewDefinition> {
    vec![
        active_rsi_definition(
            "system_prompt",
            project_key,
            "rsi-trace-correction-guard",
            "Prompt patch for correction recovery",
            "RSI_PROMPT_SENTINEL verify corrected assumptions before proceeding.",
            serde_json::json!({"rsi":{"candidate_kind":"system_prompt_patch"}}),
        ),
        active_rsi_definition(
            "skill",
            project_key,
            "rsi-verified-runbook",
            "RSI verified runbook",
            "---\nname: rsi-verified-runbook\ndescription: 'RSI_SKILL_SENTINEL reusable verified workflow.'\ncreated-by: rsi-curator\n---\nRSI_SKILL_SENTINEL verify the final observable outcome before reusing this workflow.\n",
            serde_json::json!({"rsi":{"candidate_kind":"skill_draft"}}),
        ),
        active_rsi_definition(
            "harness_patch",
            project_key,
            "claude-test-bash",
            "Harness patch from RSI trace",
            "RSI_HARNESS_SENTINEL snapshot state, verify inputs, and rollback if success or cost regresses.",
            serde_json::json!({"rsi":{"candidate_kind":"harness_patch"}}),
        ),
        active_rsi_definition(
            "context_playbook",
            project_key,
            "tool-recovery",
            "Context playbook: tool recovery",
            "RSI_CONTEXT_SENTINEL retrieve this playbook for similar tool-recovery traces.",
            serde_json::json!({"rsi":{"candidate_kind":"context_playbook_patch"}}),
        ),
        active_rsi_definition(
            "budget_policy",
            project_key,
            "test-model-default-effort",
            "Reasoning budget recommendation",
            "RSI_BUDGET_SENTINEL prefer lower effort after verified low-token traces.",
            serde_json::json!({
                "rsi": {
                    "candidate_kind": "budget_policy",
                    "budget": {
                        "tool_visibility": [
                            {
                                "tool_name": "Bash",
                                "action": "show_earlier",
                                "reason": "verified traces needed command execution"
                            }
                        ]
                    }
                }
            }),
        ),
        active_rsi_definition(
            "reasoning_policy",
            project_key,
            "verified-efficient",
            "Reasoning process policy",
            "RSI_REASONING_SENTINEL require observable verification and never copy private reasoning into prompts, skills, memory, or tool definitions.",
            serde_json::json!({"rsi":{"candidate_kind":"reasoning_policy"}}),
        ),
        active_rsi_definition(
            "tool_definition",
            project_key,
            "Bash",
            "Tool definition patch for Bash recovery",
            "RSI_TOOL_SENTINEL verify cwd and command before retry.",
            serde_json::json!({"rsi":{"candidate_kind":"tool_definition_patch"}}),
        ),
    ]
}

fn active_rsi_definition(
    kind: &str,
    project_key: &str,
    name: &str,
    title: &str,
    body: &str,
    metadata: serde_json::Value,
) -> jfc_knowledge::NewDefinition {
    jfc_knowledge::NewDefinition {
        kind: kind.to_owned(),
        scope: jfc_knowledge::DefinitionScope::Project,
        project_key: Some(project_key.to_owned()),
        namespace: None,
        name: name.to_owned(),
        title: Some(title.to_owned()),
        description: Some("active RSI test definition".to_owned()),
        body: body.to_owned(),
        metadata_json: metadata.to_string(),
        source_path: Some(format!("rsi:definition:test:{kind}:{name}")),
        source_hash: Some(format!("test-{kind}-{name}")),
        status: jfc_knowledge::DefinitionStatus::Active,
        created_by: "test".to_owned(),
    }
}

struct KnowledgeDbEnvGuard {
    prior: Option<std::ffi::OsString>,
    _temp: tempfile::TempDir,
}

impl KnowledgeDbEnvGuard {
    fn set() -> Self {
        let temp = tempfile::TempDir::new().unwrap();
        let prior = std::env::var_os("JFC_KNOWLEDGE_DB");
        // SAFETY: this serial test owns the process-wide knowledge DB variable
        // for its duration and restores it in Drop before any parallel use.
        unsafe { std::env::set_var("JFC_KNOWLEDGE_DB", temp.path().join("knowledge.db")) };
        Self { prior, _temp: temp }
    }
}

impl Drop for KnowledgeDbEnvGuard {
    fn drop(&mut self) {
        match self.prior.take() {
            Some(value) => {
                // SAFETY: see KnowledgeDbEnvGuard::set; restoration is protected
                // by the same serial test boundary.
                unsafe { std::env::set_var("JFC_KNOWLEDGE_DB", value) };
            }
            None => {
                // SAFETY: see KnowledgeDbEnvGuard::set; restoration is protected
                // by the same serial test boundary.
                unsafe { std::env::remove_var("JFC_KNOWLEDGE_DB") };
            }
        }
    }
}
