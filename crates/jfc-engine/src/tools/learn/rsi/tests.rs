use jfc_learn::rsi_curator::ApplyToStore;

#[tokio::test(flavor = "multi_thread")]
#[serial_test::serial]
async fn learn_rsi_promote_and_rollback_drive_tool_surface_normal() {
    let temp = tempfile::tempdir().unwrap();
    let _env = EnvGuard::set("JFC_KNOWLEDGE_DB", temp.path().join("knowledge.db"));
    let project = jfc_knowledge::project_key(temp.path());
    let store = jfc_knowledge::KnowledgeStore::open_default().await.unwrap();
    store
        .upsert_definition(&existing_tool_definition(&project))
        .await
        .unwrap();
    let traces = recovered_tool_traces();
    let curator = jfc_learn::RsiCurator::new(
        jfc_learn::RsiCuratorConfig::default(),
        jfc_learn::RsiPromotionPolicy::default(),
    );
    let mut report = curator.run(&traces).unwrap();
    let sandbox = crate::sandbox::rsi_external_worker_sandbox(temp.path());
    let external_status = sandbox.status.slug();
    let external_egress = sandbox.egress_isolated.to_string();
    report.experiment_job.external_worker_sandbox = sandbox;
    report.apply_to_store(&store, &project).await.unwrap();
    let candidate = report
        .candidates
        .iter()
        .find(|candidate| candidate.kind == jfc_learn::CandidateKind::ToolDefinitionPatch)
        .unwrap();

    let listed = crate::tools::execute_tool(
        jfc_core::ToolKind::LearnRsiList,
        jfc_core::ToolInput::LearnRsiList {
            status: Some("candidate".to_owned()),
            limit: Some(5),
        },
        temp.path().to_path_buf(),
        None,
        None,
        None,
    )
    .await;

    assert!(!listed.is_error(), "{}", listed.output);
    assert!(listed.output.contains(&candidate.definition_name()));
    assert!(
        listed
            .output
            .contains("research=tool_definition_control verified=true")
    );
    assert!(
        listed
            .output
            .contains("control=tool_definition_write/verified")
    );
    assert!(
        listed
            .output
            .contains("thinking=private_reasoning_derived raw_stored=false")
    );
    assert!(
        listed
            .output
            .contains("promote: learn_rsi_promote kind=tool_definition")
    );
    assert!(listed.output.contains("Experiment dashboard:"));
    assert!(listed.output.contains("hidden_validation=required"));
    assert!(listed.output.contains("anti_cheat=protected"));
    assert!(listed.output.contains("sandbox=network_blocked"));
    assert!(listed.output.contains("next_action=branch_out"));
    assert!(listed.output.contains("Experiment loop:"));
    assert!(listed.output.contains("phase=branch"));
    assert!(listed.output.contains("timeout_seconds=300"));
    assert!(listed.output.contains("commit_required=true"));
    assert!(listed.output.contains("egress=deny_by_default"));
    assert!(listed.output.contains("Experiment job:"));
    assert!(listed.output.contains("name=rsi-experiment-iteration"));
    assert!(listed.output.contains("cadence_seconds=900"));
    assert!(listed.output.contains("max_iterations_per_cycle=1"));
    assert!(listed.output.contains("preflight=ready"));
    assert!(listed.output.contains("hidden_validation=4/4"));
    assert!(listed.output.contains("reject_metric_mutation=true"));
    assert!(
        listed
            .output
            .contains("sandbox_enforcement=in_process_only")
    );
    assert!(listed.output.contains("execution_mode=in_process_curator"));
    assert!(listed.output.contains("kernel_enforced=false"));
    assert!(listed.output.contains("sandbox_backend=none"));
    assert!(
        listed
            .output
            .contains(&format!("external_worker_sandbox={external_status}"))
    );
    assert!(
        listed
            .output
            .contains("external_backend=bubblewrap_unshare_net")
    );
    assert!(
        listed
            .output
            .contains(&format!("external_egress_isolated={external_egress}"))
    );
    assert!(listed.output.contains("fresh_worktree=true"));
    assert!(listed.output.contains("Experiment loop state:"));
    assert!(listed.output.contains("runs=1"));
    assert!(listed.output.contains("preflight=ready"));
    assert!(listed.output.contains("candidate_actions="));

    let promoted = crate::tools::execute_tool(
        jfc_core::ToolKind::LearnRsiPromote,
        jfc_core::ToolInput::LearnRsiPromote {
            kind: "tool_definition".to_owned(),
            name: candidate.definition_name(),
        },
        temp.path().to_path_buf(),
        None,
        None,
        None,
    )
    .await;

    assert!(!promoted.is_error(), "{}", promoted.output);
    assert!(promoted.output.contains("RSI definition promoted"));
    let active = definition(&store, &project, "Edit").await;
    assert_eq!(active.body, candidate.body);

    let active_audit = crate::tools::execute_tool(
        jfc_core::ToolKind::LearnRsiList,
        jfc_core::ToolInput::LearnRsiList {
            status: Some("active".to_owned()),
            limit: Some(5),
        },
        temp.path().to_path_buf(),
        None,
        None,
        None,
    )
    .await;

    assert!(!active_audit.is_error(), "{}", active_audit.output);
    assert!(active_audit.output.contains("health=healthy"));
    assert!(
        active_audit
            .output
            .contains("rollback: learn_rsi_rollback kind=tool_definition name=Edit")
    );

    let rolled_back = crate::tools::execute_tool(
        jfc_core::ToolKind::LearnRsiRollback,
        jfc_core::ToolInput::LearnRsiRollback {
            kind: "tool_definition".to_owned(),
            name: "Edit".to_owned(),
        },
        temp.path().to_path_buf(),
        None,
        None,
        None,
    )
    .await;

    assert!(!rolled_back.is_error(), "{}", rolled_back.output);
    assert!(rolled_back.output.contains("RSI definition rolled back"));
    let restored = definition(&store, &project, "Edit").await;
    assert_eq!(restored.body, "old body");
}

fn existing_tool_definition(project: &str) -> jfc_knowledge::NewDefinition {
    jfc_knowledge::NewDefinition {
        kind: "tool_definition".to_owned(),
        scope: jfc_knowledge::DefinitionScope::Project,
        project_key: Some(project.to_owned()),
        namespace: None,
        name: "Edit".to_owned(),
        title: Some("Edit".to_owned()),
        description: Some("old".to_owned()),
        body: "old body".to_owned(),
        metadata_json: "{}".to_owned(),
        source_path: Some("rust:tool:Edit".to_owned()),
        source_hash: Some("oldhash".to_owned()),
        status: jfc_knowledge::DefinitionStatus::Active,
        created_by: "test".to_owned(),
    }
}

fn recovered_tool_traces() -> Vec<jfc_learn::RsiTrace> {
    ["s1", "s2", "s3", "s4"]
        .into_iter()
        .map(recovered_tool_trace)
        .collect()
}

fn recovered_tool_trace(session_id: &str) -> jfc_learn::RsiTrace {
    let mut trace = jfc_learn::RsiTrace::new(session_id);
    trace.outcome = Some(jfc_learn::RsiOutcome::Succeeded);
    trace.tool_steps = vec![
        jfc_learn::RsiToolStep::new("Edit", false),
        jfc_learn::RsiToolStep::new("Read", true),
        jfc_learn::RsiToolStep::new("Edit", true),
    ];
    trace.verifications = vec![jfc_learn::RsiVerification::new("hidden cargo test", true)];
    trace.thinking_tokens = 1_024;
    trace.thinking_blocks = vec!["private thinking must not be stored".to_owned()];
    trace
}

async fn definition(
    store: &jfc_knowledge::KnowledgeStore,
    project: &str,
    name: &str,
) -> jfc_knowledge::DefinitionRecord {
    store
        .get_definition_by_name(
            "tool_definition",
            jfc_knowledge::DefinitionScope::Project,
            Some(project),
            None,
            name,
        )
        .await
        .unwrap()
        .unwrap()
}

struct EnvGuard {
    key: &'static str,
    previous: Option<std::ffi::OsString>,
}

impl EnvGuard {
    fn set(key: &'static str, value: impl AsRef<std::path::Path>) -> Self {
        let previous = std::env::var_os(key);
        // SAFETY: this test is serial and restores the process-wide variable
        // before any following test can observe it.
        unsafe { std::env::set_var(key, value.as_ref()) };
        Self { key, previous }
    }
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        // SAFETY: this test is serial and restores the process-wide variable
        // before any following test can observe it.
        unsafe {
            if let Some(previous) = &self.previous {
                std::env::set_var(self.key, previous);
            } else {
                std::env::remove_var(self.key);
            }
        }
    }
}
