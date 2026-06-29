use super::*;

#[test]
fn plugin_doctor_reports_reload_digest_and_descriptor_counts_normal() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let plugin = tmp.path().join("acme");
    std::fs::create_dir_all(plugin.join("skills/audit")).expect("create skills");
    std::fs::create_dir_all(plugin.join("agents")).expect("create agents");
    std::fs::create_dir_all(plugin.join("flows")).expect("create workflows");
    std::fs::create_dir_all(plugin.join("commands")).expect("create commands");
    std::fs::write(
        plugin.join(".jfc-plugin.toml"),
        "[plugin]\nname = \"acme-tools\"\nworkflows_dir = \"flows\"\n\n[[metrics]]\nid = \"cache.hit_rate\"\nlabel = \"Cache hit rate\"\ndescription = \"Cache hit rate\"\nunit = \"percent\"\nsurfaces = [\"sidebar\", \"panel\"]\npriority = 42\n\n[[ui_panels]]\nscope = \"info_sidebar\"\nid = \"review.summary\"\ntitle = \"Review Summary\"\nbody = \"3 open reviews\\n1 blocking approval\"\nruntime_action_id = \"metrics.refresh\"\nrefresh = { kind = \"process_bridge\", handler = \"bin/review-panel\", min_interval_ms = 5000, auto_refresh_ms = 60000 }\npriority = 44\n\n[[ui_widgets]]\nscope = \"info_sidebar\"\nid = \"review.queue\"\nlabel = \"Review Queue\"\nkind = \"text\"\nbody = \"3 open reviews\"\nruntime_action_id = \"metrics.refresh\"\nrefresh = { kind = \"process_bridge\", handler = \"bin/review-widget\", min_interval_ms = 5000, auto_refresh_ms = 60000 }\npriority = 43\n\n[[runtime_actions]]\nid = \"metrics.refresh\"\nlabel = \"Refresh Metrics\"\ndescription = \"Refresh metric snapshots\"\nkind = \"refresh_metrics\"\npriority = 41\n\n[[runtime_extensions]]\ntarget = \"prompt_context\"\nid = \"context.review-rules\"\nlabel = \"Review Rules\"\npriority = 39\n\n[runtime_extensions.executor]\nkind = \"static_text\"\nhandler = \"Always include plugin review rules.\"\n",
    )
    .expect("write manifest");

    let output = plugin_doctor_in(tmp.path(), Some("old-digest")).expect("doctor output");

    assert!(output.contains("reload: changed"));
    assert!(output.contains("previous_descriptor_digest: old-digest"));
    assert!(output.contains("health: total=1 active=1 disabled=0 failed=0 errors=0"));
    assert!(output.contains(
        "descriptors: resources=3 commands=1 tools=0 providers=0 services=0 ui_slots=0 ui_panels=1 ui_widgets=1 runtime_actions=1 runtime_extensions=1 agent_launches=0 metrics=1 hooks=0"
    ));
    assert!(output.contains("services:"));
    assert!(
        output.contains(
            "- builtin.plugin-management plugin_installer jfc plugin install [available]"
        )
    );
    assert!(output.contains(
        "- builtin.plugin-management plugin_template_catalog jfc plugin templates [available]"
    ));
    assert!(
        output.contains("- builtin.plugin-management plugin_smoke jfc plugin smoke [available]")
    );
    assert!(output.contains("metrics:"));
    assert!(output.contains(
        "- acme-tools cache.hit_rate Cache hit rate [percent; surfaces: sidebar,panel; priority=42]"
    ));
    assert!(output.contains("ui_panels:"));
    assert!(output.contains(
        "- acme-tools review.summary Review Summary [scope: info_sidebar; priority=44; refresh=process_bridge; min_interval_ms=5000; auto_refresh_ms=60000]"
    ));
    assert!(output.contains("ui_widgets:"));
    assert!(output.contains(
        "- acme-tools review.queue Review Queue [text; scope: info_sidebar; priority=43; refresh=process_bridge; min_interval_ms=5000; auto_refresh_ms=60000]"
    ));
    assert!(output.contains("runtime_actions:"));
    assert!(
        output.contains(
            "- acme-tools metrics.refresh Refresh Metrics [refresh_metrics; priority=41]"
        )
    );
    assert!(output.contains("runtime_extensions:"));
    assert!(output.contains(
        "- acme-tools context.review-rules Review Rules [prompt_context; executor=static_text; priority=39]"
    ));
    assert!(output.contains("- acme-tools"));
}

#[test]
fn plugin_doctor_reports_descriptor_issues_normal() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let plugin_id = jfc_plugin_sdk::PluginId::new("acme-tools");
    let report = PluginReloadReport {
        diagnostics: jfc_plugin_host::PluginHostDiagnostics {
            health: jfc_plugin_host::PluginHealthSummary {
                total: 1,
                active: 1,
                registered: 0,
                disabled: 0,
                failed: 0,
                error_count: 0,
            },
            counts: jfc_plugin_host::PluginDescriptorCounts::default(),
            descriptor_digest: "bbbbbbbbbbbbbbbb".to_owned(),
            descriptor_issues: vec![jfc_plugin_host::PluginDescriptorIssue {
                kind: jfc_plugin_host::PluginDescriptorIssueKind::MissingRuntimeAction,
                severity: jfc_plugin_host::PluginDescriptorIssueSeverity::Error,
                actionability:
                    jfc_plugin_host::PluginDescriptorIssueActionability::AddRuntimeAction,
                plugin_id: plugin_id.clone(),
                descriptor_kind: jfc_plugin_host::PluginDescriptorKind::UiWidget,
                descriptor_id: "review.queue".to_owned(),
                target_plugin_id: plugin_id.clone(),
                target_kind: jfc_plugin_host::PluginDescriptorTargetKind::RuntimeAction,
                target_id: "review.queue.run".to_owned(),
                message: "descriptor references a missing runtime action".to_owned(),
                repair_action: jfc_plugin_host::PluginDescriptorRepairAction::AddRuntimeAction {
                    plugin_id: plugin_id.clone(),
                    action_id: "review.queue.run".to_owned(),
                },
                repair_hint: "Add runtime action 'review.queue.run' to plugin 'acme-tools', or point UI widget 'review.queue' in plugin 'acme-tools' at an existing runtime action.".to_owned(),
            }],
            active_plugins: Vec::new(),
            failed_plugins: Vec::new(),
        },
        previous_descriptor_digest: None,
        changed: None,
    };

    let output = render_plugin_doctor(tmp.path(), &report, &[], &[], &[], &[], &[], &[], &[], &[]);

    assert!(output.contains("descriptor_issues:"));
    assert!(output.contains(
        "- acme-tools ui_widget:review.queue -> acme-tools:runtime_action:review.queue.run [error; add_runtime_action; missing_runtime_action] hint: Add runtime action 'review.queue.run' to plugin 'acme-tools'"
    ));
}
