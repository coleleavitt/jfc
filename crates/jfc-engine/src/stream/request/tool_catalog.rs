use crate::runtime::StreamRequestOverrides;
use crate::tools;
use jfc_provider::{ProviderContent, ProviderMessage, ToolDef};

use super::intent::{conversation_is_mid_tool_loop, user_text_requests_action};
use super::messages::last_user_text;
use super::tools::preserve_non_action_tool;

pub(super) struct AdvertisedToolCatalog {
    pub(super) tools: Vec<ToolDef>,
    pub(super) advertised_tool_count: usize,
    pub(super) action_expected: bool,
}

pub(super) async fn prepare_advertised_tools(
    system_prompt: &mut String,
    messages: &[ProviderMessage],
    overrides: &StreamRequestOverrides,
    hcom_available: bool,
    local_advisor_absent: bool,
    effective_brief_mode: bool,
    pewter_owl_tool: bool,
) -> AdvertisedToolCatalog {
    let mut full_tool_catalog = tools::all_tool_defs_with_mcp().await;
    append_historical_hidden_builtin_tool_defs(&mut full_tool_catalog, messages);
    tools::sync_tool_definitions_to_db(&full_tool_catalog);
    let rsi_tool_patches =
        super::rsi_runtime::apply_active_tool_definition_patches(&mut full_tool_catalog).await;
    if rsi_tool_patches > 0 {
        tracing::debug!(
            target: "jfc::stream::rsi",
            patched_tools = rsi_tool_patches,
            "applied active RSI tool definition patches"
        );
    }
    let full_tool_count = full_tool_catalog.len();
    let mut advertised_tools = if overrides.allowed_tools.is_empty() {
        let tool_intent = last_user_text(messages);
        let selected =
            tools::progressive_tool_defs(full_tool_catalog, messages, tool_intent.as_deref());
        tracing::debug!(
            target: "jfc::stream::tools",
            selected = selected.len(),
            full = full_tool_count,
            "selected progressive tool catalog"
        );
        selected
    } else {
        full_tool_catalog
    };
    tools::apply_send_user_message_policy(
        &mut advertised_tools,
        effective_brief_mode,
        pewter_owl_tool,
    );
    if !hcom_available {
        advertised_tools.retain(|tool| !tools::is_hcom_tool_name(&tool.name));
    }

    #[cfg(feature = "permission-automation")]
    {
        let cwd_for_perms =
            std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
        let cfg = crate::config::feature_config::FeatureConfig::load(&cwd_for_perms);
        let rules = crate::permissions::RuleSet::from_config(&cfg);
        let before = advertised_tools.len();
        let mut suppressed: Vec<String> = Vec::new();
        advertised_tools.retain(|t| {
            let decision = crate::permissions::check_tool_permission(&rules, &t.name, None);
            if matches!(decision.action, crate::permissions::PermissionAction::Deny) {
                suppressed.push(t.name.clone());
                false
            } else {
                true
            }
        });
        if !suppressed.is_empty() {
            tracing::info!(
                target: "jfc::stream::permissions",
                suppressed_count = suppressed.len(),
                tools = ?suppressed,
                "pre-flight: suppressed denied tools from catalog"
            );
            system_prompt.push_str(&format!(
                "\n\n## Tools suppressed by policy\n\nThe following tools \
                     are denied by `.jfc/permissions.toml` and are NOT available \
                     this session: {}.\n",
                suppressed.join(", "),
            ));
        }
        let _ = before;
    }

    // Hide JFC's local Advisor tool unless a local advisor model is configured.
    // The upstream server advisor, when active, is injected through
    // StreamOptions instead of the normal local tool catalog.
    if local_advisor_absent {
        advertised_tools.retain(|t| t.name != "Advisor");
    }

    if !overrides.allowed_tools.is_empty() {
        let before = advertised_tools.len();
        let allowed_lower: Vec<String> = overrides
            .allowed_tools
            .iter()
            .map(|t| t.to_lowercase())
            .collect();
        let mut suppressed: Vec<String> = Vec::new();
        advertised_tools.retain(|t| {
            if allowed_lower.contains(&t.name.to_lowercase()) {
                true
            } else {
                suppressed.push(t.name.clone());
                false
            }
        });
        if !suppressed.is_empty() {
            tracing::info!(
                target: "jfc::stream::tools",
                removed = suppressed.len(),
                total_before = before,
                tools = ?suppressed,
                "removed tools outside allowlist"
            );
            system_prompt.push_str(&format!(
                "\n\n## Tools suppressed by managed/user allowlist\n\nOnly these tools \
                     are available this session: {}.\n",
                overrides.allowed_tools.join(", "),
            ));
        }
    }

    if !overrides.disallowed_tools.is_empty() {
        let before = advertised_tools.len();
        let disallowed_lower: Vec<String> = overrides
            .disallowed_tools
            .iter()
            .map(|t| t.to_lowercase())
            .collect();
        let mut suppressed: Vec<String> = Vec::new();
        advertised_tools.retain(|t| {
            if disallowed_lower.contains(&t.name.to_lowercase()) {
                suppressed.push(t.name.clone());
                false
            } else {
                true
            }
        });
        if !suppressed.is_empty() {
            tracing::info!(
                target: "jfc::stream::tools",
                removed = suppressed.len(),
                total_before = before,
                tools = ?suppressed,
                "removed disallowed tools from catalog"
            );
        }
    }

    // A post-tool continuation re-sends the conversation with the trailing
    // user turn carrying only tool_result blocks. The model is mid-loop and
    // MUST keep its tools — treat that as action-expected regardless of what
    // the last *text* prompt looked like, otherwise the catalog is stripped
    // and the model emits raw <tool_calls> XML until it hits max tokens.
    let mid_tool_loop = conversation_is_mid_tool_loop(messages);
    let action_expected = mid_tool_loop
        || last_user_text(messages)
            .as_deref()
            .map(user_text_requests_action)
            .unwrap_or(false);
    if !action_expected && !advertised_tools.is_empty() {
        let before = advertised_tools.len();
        advertised_tools.retain(|tool| preserve_non_action_tool(&tool.name));
        tracing::debug!(
            target: "jfc::stream::tools",
            before,
            after = advertised_tools.len(),
            "reduced tool catalog for non-action prompt"
        );
    }
    let advertised_tool_names = advertised_tools
        .iter()
        .map(|tool| tool.name.clone())
        .collect::<Vec<_>>();
    if advertised_tools
        .iter()
        .any(|tool| tools::is_hcom_tool_name(&tool.name))
        && let Some(section) = tools::hcom_system_prompt_section()
    {
        system_prompt.push_str("\n\n");
        system_prompt.push_str(section);
    }
    if let Some(rules) = crate::review::tool_scoped_prompt_rules(&advertised_tool_names) {
        system_prompt.push_str("\n\n");
        system_prompt.push_str(&rules);
    }
    let advertised_tool_count = advertised_tools.len();
    AdvertisedToolCatalog {
        tools: advertised_tools,
        advertised_tool_count,
        action_expected,
    }
}

fn append_historical_hidden_builtin_tool_defs(
    catalog: &mut Vec<ToolDef>,
    messages: &[ProviderMessage],
) {
    let historical_names: std::collections::HashSet<String> = messages
        .iter()
        .flat_map(|message| message.content.iter())
        .filter_map(|content| match content {
            ProviderContent::ToolUse { name, .. } => Some(name.trim().to_ascii_lowercase()),
            _ => None,
        })
        .collect();
    if historical_names.is_empty() {
        return;
    }

    let mut existing_names: std::collections::HashSet<String> = catalog
        .iter()
        .map(|tool| tool.name.trim().to_ascii_lowercase())
        .collect();
    for tool in tools::all_tool_defs() {
        let normalized = tool.name.trim().to_ascii_lowercase();
        if tools::is_model_hidden_builtin_tool_name(&tool.name)
            && historical_names.contains(&normalized)
            && existing_names.insert(normalized)
        {
            catalog.push(tool);
        }
    }
}
