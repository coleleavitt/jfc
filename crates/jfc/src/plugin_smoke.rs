use anyhow::Context;
use jfc_plugin_host::{
    PluginDiscoveryOptions, PluginDiscoverySearchRoot,
    reload_cached_discovered_resource_plugin_state,
};
use jfc_plugin_sdk::{
    BridgeProviderContent, BridgeProviderMessage, BridgeProviderRole, BridgeProviderStreamOptions,
    BridgeRequest, ProviderDescriptor, ProviderExecutorKind, ToolDescriptor, ToolExecutorKind,
};

mod bridge;

pub(crate) async fn smoke_plugin(name: &str) -> anyhow::Result<String> {
    let plugin_name = crate::plugin_paths::sanitize_plugin_name(name)?;
    let root = crate::plugin_paths::plugins_root()?;
    let state = reload_cached_discovered_resource_plugin_state(
        PluginDiscoveryOptions::new()
            .with_search_root(PluginDiscoverySearchRoot::global_plugins_dir(root)),
        None,
    )?;
    let tools = state
        .host
        .tool_descriptors()
        .into_iter()
        .filter(|tool| {
            tool.plugin_id.as_str() == plugin_name
                && tool.executor.kind == ToolExecutorKind::ProcessBridge
        })
        .collect::<Vec<_>>();
    let providers = state
        .host
        .provider_descriptors()
        .into_iter()
        .filter(|provider| {
            provider.plugin_id.as_str() == plugin_name
                && provider.executor.kind == ProviderExecutorKind::ProcessBridge
        })
        .collect::<Vec<_>>();
    if tools.is_empty() && providers.is_empty() {
        anyhow::bail!("plugin `{plugin_name}` has no process-bridge tools or providers");
    }

    let mut out = String::new();
    out.push_str(&format!("plugin smoke: {plugin_name}\n"));
    out.push_str(&format!(
        "descriptors: tools={} providers={}\n",
        tools.len(),
        providers.len()
    ));
    for tool in &tools {
        smoke_tool(tool, &mut out).await?;
    }
    for provider in &providers {
        smoke_provider(provider, &mut out).await?;
    }
    Ok(out)
}

async fn smoke_tool(tool: &ToolDescriptor, out: &mut String) -> anyhow::Result<()> {
    let command = bridge::parse_process_bridge_handler(&tool.executor.handler)?;
    let describe =
        bridge::run_bridge_request(&command, "describe", BridgeRequest::Describe).await?;
    bridge::ensure_describe_response(&describe, &tool.name)?;
    out.push_str(&format!("describe {}: ok\n", tool.name));

    let frames = bridge::run_bridge_request(
        &command,
        "tool-smoke",
        BridgeRequest::ToolCall {
            tool: tool.name.clone(),
            tool_id: None,
            input: serde_json::json!({ "message": "smoke test" }),
        },
    )
    .await?;
    let result = bridge::tool_result_text(&frames).with_context(|| {
        format!(
            "process-bridge tool `{}` did not return a tool_result",
            tool.name
        )
    })?;
    out.push_str(&format!("tool {}: {result}\n", tool.name));
    Ok(())
}

async fn smoke_provider(provider: &ProviderDescriptor, out: &mut String) -> anyhow::Result<()> {
    let command = bridge::parse_process_bridge_handler(&provider.executor.handler)?;
    let describe =
        bridge::run_bridge_request(&command, "describe", BridgeRequest::Describe).await?;
    bridge::ensure_describe_response(&describe, &provider.provider)?;
    out.push_str(&format!("describe {}: ok\n", provider.provider));

    let model = provider
        .models
        .first()
        .map(|model| model.id.clone())
        .unwrap_or_else(|| "smoke-model".to_owned());
    let frames = bridge::run_bridge_request(
        &command,
        "provider-smoke",
        BridgeRequest::ProviderStream {
            provider: provider.provider.clone(),
            messages: vec![BridgeProviderMessage {
                role: BridgeProviderRole::User,
                content: vec![BridgeProviderContent::Text {
                    text: "smoke test".to_owned(),
                }],
            }],
            options: BridgeProviderStreamOptions::new(model.clone()).max_tokens(128),
        },
    )
    .await?;
    let text = bridge::provider_text(&frames).with_context(|| {
        format!(
            "process-bridge provider `{}` did not return text",
            provider.provider
        )
    })?;
    out.push_str(&format!(
        "provider {}/{}: {text}\n",
        provider.provider, model
    ));
    Ok(())
}
