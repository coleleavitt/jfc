fn mission_subject(prompt: &str) -> String {
    let first_line = prompt
        .lines()
        .find(|line| !line.trim().is_empty())
        .unwrap_or("mission")
        .trim();
    let preview = if first_line.len() > 72 {
        let boundary = first_line
            .char_indices()
            .map(|(idx, _)| idx)
            .take_while(|idx| *idx <= 72)
            .last()
            .unwrap_or(72);
        format!("{}...", &first_line[..boundary])
    } else {
        first_line.to_owned()
    };
    format!("Mission: {preview}")
}

fn mission_description(prompt: &str, route: &jfc_core::MissionRoute) -> String {
    format!(
        "Original user mission:\n{prompt}\n\nRouting: {}. {}\n\n\
         Decompose into child tasks when useful. Route solver-worthy child work through \
         bounty/market execution, then preserve distilled evidence, validation results, \
         and promotion candidates for memory, skills, prompts, or tool definitions.",
        route.kind.label(),
        route.reason
    )
}

fn mission_active_form(route: &jfc_core::MissionRoute) -> &'static str {
    match route.kind {
        jfc_core::MissionRouteKind::Direct => "Answer directly",
        jfc_core::MissionRouteKind::Solo => "Execute mission directly",
        jfc_core::MissionRouteKind::Assisted => "Delegate mission to helper",
        jfc_core::MissionRouteKind::Bounty => "Route mission through bounty market",
    }
}

fn mission_execution_metadata(
    route: &jfc_core::MissionRoute,
) -> Option<jfc_session::TaskExecutionMetadata> {
    match route.kind {
        jfc_core::MissionRouteKind::Direct => None,
        jfc_core::MissionRouteKind::Solo => Some(jfc_session::TaskExecutionMetadata::solo(
            route.reason.clone(),
        )),
        jfc_core::MissionRouteKind::Assisted => Some(jfc_session::TaskExecutionMetadata::assisted(
            route.reason.clone(),
        )),
        jfc_core::MissionRouteKind::Bounty => Some(jfc_session::TaskExecutionMetadata::bounty(
            route.reason.clone(),
            None,
        )),
    }
}

pub fn seed_mission_task(
    store: &jfc_session::TaskStore,
    route: &jfc_core::MissionRoute,
    prompt: &str,
) -> Option<String> {
    if !route.create_task_graph {
        return None;
    }
    let execution = mission_execution_metadata(route)?;

    let subject = mission_subject(prompt);
    let description = mission_description(prompt, route);
    let task = match store.create(
        subject,
        description,
        Some(mission_active_form(route).to_owned()),
        Vec::<String>::new(),
    ) {
        Ok(task) => task,
        Err(jfc_session::TaskError::DuplicateSubject { existing_id, .. }) => {
            return Some(existing_id.to_string());
        }
        Err(error) => {
            tracing::warn!(
                target: "jfc::mission_router",
                %error,
                "failed to seed mission task"
            );
            return None;
        }
    };

    let patch = jfc_session::TaskPatch {
        metadata: Some(execution.to_task_metadata(None)),
        acceptance_criteria: Some(
            "Drive the mission through solver/validator evidence, apply or summarize the \
             winning result, and record only distilled learning artifacts."
                .to_owned(),
        ),
        risk: route.risk,
        kind: Some(jfc_session::TaskKind::Task),
        tags: Some(route.tags.clone()),
        priority: Some(1),
        ..Default::default()
    };

    match store.update(task.id.as_str(), patch) {
        Ok(updated) => Some(updated.id.to_string()),
        Err(error) => {
            tracing::warn!(
                target: "jfc::mission_router",
                task_id = %task.id,
                %error,
                "failed to annotate seeded mission task"
            );
            Some(task.id.to_string())
        }
    }
}
