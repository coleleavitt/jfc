use std::sync::{Arc, Mutex};

use jfc_plugin_host::{
    HookInvocation, HookValue, PluginHost, PluginHostError, PluginRegistration, PluginStatusKind,
};
use jfc_plugin_sdk::{
    DescriptorVisibility, HookName, PluginId, PluginManifest, PluginSource, PluginVersion,
    ProviderDescriptor, ProviderExecutorKind, ToolApprovalPolicy, ToolDescriptor, ToolExecutorKind,
};
use serde_json::{Value, json};

#[test]
fn ordered_hook_mutation() {
    // Given: two active plugins with explicit activation order for the same typed hook.
    let mut host = PluginHost::new();
    host.register_internal(plugin("plugin.a").with_activation_order(10).with_hook(
        HookName::PreToolUse,
        0,
        append_step("a"),
    ))
    .expect("plugin A registers");
    host.register_internal(plugin("plugin.b").with_activation_order(20).with_hook(
        HookName::PreToolUse,
        0,
        append_step("b"),
    ))
    .expect("plugin B registers");

    // When: the hook is triggered with a mutable JSON payload.
    host.activate_all().expect("plugins activate");
    let result = host
        .trigger_hook(HookName::PreToolUse, HookValue::json(json!({"steps": []})))
        .expect("hook trigger succeeds");

    // Then: plugin A mutates before plugin B deterministically.
    assert_eq!(result.payload(), &json!({"steps": ["a", "b"]}));
}

#[test]
fn hook_trigger_until_stops_after_matching_payload() {
    let mut host = PluginHost::new();
    host.register_internal(plugin("plugin.a").with_activation_order(10).with_hook(
        HookName::PreToolUse,
        0,
        append_step("a"),
    ))
    .expect("plugin A registers");
    host.register_internal(plugin("plugin.b").with_activation_order(20).with_hook(
        HookName::PreToolUse,
        0,
        append_step("b"),
    ))
    .expect("plugin B registers");

    host.activate_all().expect("plugins activate");
    let result = host
        .trigger_hook_until(
            HookName::PreToolUse,
            HookValue::json(json!({"steps": []})),
            |value| {
                value
                    .payload()
                    .get("steps")
                    .and_then(Value::as_array)
                    .is_some_and(|steps| steps.len() == 1)
            },
        )
        .expect("hook trigger succeeds");

    assert_eq!(result.payload(), &json!({"steps": ["a"]}));
}

#[test]
fn duplicate_plugin_ids_are_rejected() {
    // Given: an existing plugin id in the registry.
    let mut host = PluginHost::new();
    host.register_internal(plugin("plugin.same"))
        .expect("first plugin registers");

    // When: another internal plugin uses the same id.
    let result = host.register_internal(plugin("plugin.same"));

    // Then: the duplicate is rejected before activation.
    assert!(matches!(
        result,
        Err(PluginHostError::DuplicatePluginId { plugin_id }) if plugin_id == "plugin.same"
    ));
}

#[test]
fn failed_activation_preserves_prior_plugin() {
    // Given: plugin A activates before plugin B, and B fails after registering cleanup.
    let cleanup = Arc::new(Mutex::new(Vec::<&'static str>::new()));
    let mut host = PluginHost::new();
    host.register_internal(plugin("plugin.a").with_activation_order(1).with_hook(
        HookName::PreToolUse,
        0,
        append_step("a"),
    ))
    .expect("plugin A registers");
    host.register_internal(
        plugin("plugin.b")
            .with_activation_order(2)
            .with_activation({
                let cleanup = Arc::clone(&cleanup);
                move |activation| {
                    activation.add_finalizer({
                        let cleanup = Arc::clone(&cleanup);
                        move || {
                            cleanup.lock().expect("cleanup lock").push("b");
                            Ok(())
                        }
                    });
                    Err(PluginHostError::plugin("activation exploded"))
                }
            }),
    )
    .expect("plugin B registers");

    // When: activation reaches the failing plugin.
    let result = host.activate_all();

    // Then: A remains active, B is failed, and B cleanup ran exactly once.
    assert!(matches!(
        result,
        Err(PluginHostError::ActivationFailed { plugin_id, .. }) if plugin_id == "plugin.b"
    ));
    assert_eq!(cleanup.lock().expect("cleanup lock").as_slice(), &["b"]);
    let hook_result = host
        .trigger_hook(HookName::PreToolUse, HookValue::json(json!({"steps": []})))
        .expect("remaining active hook succeeds");
    assert_eq!(hook_result.payload(), &json!({"steps": ["a"]}));
}

#[test]
fn finalizer_runs_exactly_once_on_shutdown() {
    // Given: an activated plugin with a lifecycle finalizer.
    let cleanup_count = Arc::new(Mutex::new(0_u32));
    let mut host = PluginHost::new();
    host.register_internal(plugin("plugin.cleanup").with_finalizer({
        let cleanup_count = Arc::clone(&cleanup_count);
        move || {
            *cleanup_count.lock().expect("cleanup count lock") += 1;
            Ok(())
        }
    }))
    .expect("plugin registers");
    host.activate_all().expect("plugin activates");

    // When: shutdown is requested more than once.
    host.shutdown().expect("first shutdown succeeds");
    host.shutdown().expect("second shutdown is idempotent");

    // Then: the finalizer ran once for the single activation.
    assert_eq!(*cleanup_count.lock().expect("cleanup count lock"), 1);
}

#[test]
fn enable_disable_controls_hook_execution() {
    // Given: an active plugin contributing a hook.
    let mut host = PluginHost::new();
    let plugin_id = PluginId::new("plugin.toggle");
    host.register_internal(plugin(plugin_id.as_str()).with_hook(
        HookName::PreToolUse,
        0,
        append_step("enabled"),
    ))
    .expect("plugin registers");
    host.activate_all().expect("plugin activates");

    // When: the plugin is disabled, then enabled again.
    host.disable_plugin(&plugin_id).expect("plugin disables");
    let disabled = host
        .trigger_hook(HookName::PreToolUse, HookValue::json(json!({"steps": []})))
        .expect("disabled hook trigger succeeds");
    host.enable_plugin(&plugin_id).expect("plugin enables");
    let enabled = host
        .trigger_hook(HookName::PreToolUse, HookValue::json(json!({"steps": []})))
        .expect("enabled hook trigger succeeds");

    // Then: disabled plugins are skipped and re-enabled plugins contribute again.
    assert_eq!(disabled.payload(), &json!({"steps": []}));
    assert_eq!(enabled.payload(), &json!({"steps": ["enabled"]}));
}

#[test]
fn status_snapshot_reports_sources_hooks_errors_and_statuses() {
    // Given: one active plugin and one disabled plugin with stable source metadata.
    let mut host = PluginHost::new();
    host.register_internal(plugin("plugin.active").with_activation_order(20).with_hook(
        HookName::SessionStart,
        5,
        |invocation| Ok(invocation.value().clone()),
    ))
    .expect("active plugin registers");
    host.register_internal(plugin("plugin.disabled").with_activation_order(10))
        .expect("disabled plugin registers");
    host.disable_plugin(&PluginId::new("plugin.disabled"))
        .expect("plugin disables before activation");
    host.activate_all().expect("active plugin activates");

    // When: status is snapped for UI/CLI reporting.
    let snapshot = host.status_snapshot();

    // Then: entries are deterministic and carry provenance, hooks, and status.
    assert_eq!(snapshot.plugins.len(), 2);
    assert_eq!(snapshot.plugins[0].plugin_id.as_str(), "plugin.disabled");
    assert_eq!(snapshot.plugins[0].status, PluginStatusKind::Disabled);
    assert_eq!(snapshot.plugins[1].plugin_id.as_str(), "plugin.active");
    assert_eq!(snapshot.plugins[1].status, PluginStatusKind::Active);
    assert_eq!(snapshot.plugins[1].hooks[0].name, HookName::SessionStart);
    assert_eq!(
        snapshot.plugins[1].source,
        PluginSource::built_in("plugin.active")
    );
    assert!(snapshot.plugins[1].errors.is_empty());
}

#[test]
fn active_tool_descriptors_are_consumed_from_registered_plugins() {
    // Given: one active built-in tool plugin and one disabled tool plugin.
    let mut host = PluginHost::new();
    let active_id = PluginId::new("builtin.tools");
    let disabled_id = PluginId::new("builtin.disabled-tools");
    host.register_internal(
        plugin(active_id.as_str()).with_tool_descriptor(
            ToolDescriptor::new(
                active_id.clone(),
                "Bash",
                "Run a shell command",
                json!({"type":"object","required":["command"]}),
            )
            .with_executor(ToolExecutorKind::BuiltIn, "Bash")
            .with_approval_policy(ToolApprovalPolicy::Mutating)
            .with_visibility(DescriptorVisibility::ModelVisible),
        ),
    )
    .expect("active tool plugin registers");
    host.register_internal(
        plugin(disabled_id.as_str()).with_tool_descriptor(ToolDescriptor::new(
            disabled_id.clone(),
            "Hidden",
            "Hidden tool",
            json!({"type":"object"}),
        )),
    )
    .expect("disabled tool plugin registers");
    host.disable_plugin(&disabled_id).expect("plugin disables");

    // When: the host activates and lists tool descriptors for the catalog.
    host.activate_all().expect("plugins activate");
    let tools = host.tool_descriptors();

    // Then: only active plugin descriptors are exposed through the host path.
    assert_eq!(tools.len(), 1);
    assert_eq!(tools[0].plugin_id, active_id);
    assert_eq!(tools[0].name, "Bash");
    assert_eq!(tools[0].approval_policy, ToolApprovalPolicy::Mutating);
}

#[test]
fn active_provider_descriptors_are_consumed_from_registered_plugins() {
    // Given: one active provider plugin and one disabled provider plugin.
    let mut host = PluginHost::new();
    let active_id = PluginId::new("builtin.providers");
    let disabled_id = PluginId::new("builtin.disabled-providers");
    host.register_internal(
        plugin(active_id.as_str()).with_provider_descriptor(
            ProviderDescriptor::new(active_id.clone(), "anthropic")
                .with_executor(ProviderExecutorKind::BuiltIn, "anthropic")
                .with_model_info("claude-opus-4-7", "Claude Opus 4.7", Some(200_000), None)
                .with_visibility(DescriptorVisibility::HostVisible),
        ),
    )
    .expect("active provider plugin registers");
    host.register_internal(plugin(disabled_id.as_str()).with_provider_descriptor(
        ProviderDescriptor::new(disabled_id.clone(), "hidden").with_model("hidden-model"),
    ))
    .expect("disabled provider plugin registers");
    host.disable_plugin(&disabled_id).expect("plugin disables");

    // When: the host activates and lists provider descriptors for selection/catalog paths.
    host.activate_all().expect("plugins activate");
    let providers = host.provider_descriptors();

    // Then: only active plugin descriptors are exposed through the host path.
    assert_eq!(providers.len(), 1);
    assert_eq!(providers[0].plugin_id, active_id);
    assert_eq!(providers[0].provider, "anthropic");
    assert_eq!(providers[0].executor.kind, ProviderExecutorKind::BuiltIn);
    assert_eq!(providers[0].models[0].id, "claude-opus-4-7");
}

fn plugin(id: &str) -> PluginRegistration {
    PluginRegistration::new(PluginManifest::new(
        PluginId::new(id),
        PluginVersion::new("0.1.0"),
        PluginSource::built_in(id),
    ))
}

fn append_step(
    step: &'static str,
) -> impl Fn(HookInvocation<'_>) -> Result<HookValue, PluginHostError> + Send + Sync + 'static {
    move |invocation| {
        let mut steps = invocation
            .value()
            .payload()
            .get("steps")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        steps.push(json!(step));
        Ok(HookValue::json(json!({"steps": steps})))
    }
}
