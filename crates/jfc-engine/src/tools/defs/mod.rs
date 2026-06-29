mod agents;
mod daemon;
mod design;
mod economy;
mod filesystem;
mod hcom;
mod interaction;
mod learn;
mod plan;
mod review;
mod tasks;

use jfc_provider::ToolDef;

const DEF_KIND_TOOL: &str = "tool_definition";

pub fn all_tool_defs() -> Vec<ToolDef> {
    let mut defs = Vec::with_capacity(72);
    defs.extend(filesystem::filesystem_tool_defs());
    defs.extend(super::descriptor_router::builtin_tool_defs());
    let descriptor_owned_names = defs
        .iter()
        .map(|tool| tool.name.clone())
        .collect::<std::collections::HashSet<_>>();
    defs.extend(super::descriptor_catalog::external_tool_defs(
        &descriptor_owned_names,
    ));
    defs.extend(tasks::task_tool_defs());
    defs.extend(plan::plan_tool_defs());
    defs.extend(agents::agent_tool_defs());
    defs.extend(economy::economy_tool_defs());
    defs.extend(design::design_tool_defs());
    defs.extend(hcom::hcom_tool_defs());
    defs.extend(interaction::interaction_tool_defs());
    defs.extend(learn::learn_tool_defs());
    defs.extend(review::review_tool_defs());
    defs.extend(daemon::daemon_tool_defs());
    defs
}

pub fn model_tool_defs() -> Vec<ToolDef> {
    all_tool_defs()
        .into_iter()
        .filter(|tool| !is_model_hidden_builtin_tool_name(&tool.name))
        .collect()
}

pub fn is_model_hidden_builtin_tool_name(name: &str) -> bool {
    name.eq_ignore_ascii_case("BashOutput")
}

pub fn sync_tool_definitions_to_db(tools: &[ToolDef]) {
    let Ok(cwd) = std::env::current_dir() else {
        return;
    };
    let Some(store) = open_definition_store(&cwd) else {
        return;
    };
    for tool in tools {
        let body = serde_json::to_string_pretty(&serde_json::json!({
            "description": tool.description,
            "input_schema": tool.input_schema,
        }))
        .unwrap_or_else(|_| tool.description.clone());
        let def = jfc_knowledge::NewDefinition {
            kind: DEF_KIND_TOOL.to_owned(),
            scope: jfc_knowledge::DefinitionScope::Builtin,
            project_key: None,
            namespace: None,
            name: tool.name.clone(),
            title: Some(tool.name.clone()),
            description: Some(tool.description.clone()),
            body: body.clone(),
            metadata_json: serde_json::json!({
                "source": "runtime_tool_catalog",
                "executable_owner": "rust",
            })
            .to_string(),
            source_path: Some(format!("rust:tool:{}", tool.name)),
            source_hash: Some(content_hash(&body)),
            status: jfc_knowledge::DefinitionStatus::Active,
            created_by: "runtime_catalog".to_owned(),
        };
        if let Err(err) =
            jfc_knowledge::block_on_knowledge(async { store.upsert_definition(&def).await })
        {
            tracing::warn!(
                target: "jfc::tools",
                tool = %tool.name,
                error = %err,
                "failed to sync tool definition"
            );
        }
    }
}

fn open_definition_store(project_root: &std::path::Path) -> Option<jfc_knowledge::KnowledgeStore> {
    #[cfg(test)]
    {
        let path = project_root.join(".jfc").join("definition-test.db");
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        jfc_knowledge::block_on_knowledge(async move {
            jfc_knowledge::KnowledgeStore::open(&path).await
        })
        .ok()
    }
    #[cfg(not(test))]
    {
        let _ = project_root;
        jfc_knowledge::block_on_knowledge(async {
            jfc_knowledge::KnowledgeStore::open_default().await
        })
        .ok()
    }
}

fn content_hash(raw: &str) -> String {
    use std::hash::{Hash, Hasher};

    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    raw.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}
