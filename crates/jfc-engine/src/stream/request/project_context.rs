use std::sync::Arc;

use crate::runtime::StreamRequestOverrides;
use jfc_provider::{ModelId, Provider, ProviderMessage};

use super::memory::{
    append_cross_project_knowledge, append_memory_recall_context,
    append_session_start_knowledge_brief, fast_recall_model, sdk_memory_store_prompt_section,
};
use super::messages::last_user_text;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) struct ProjectContextStats {
    pub(super) memory_context_chars: usize,
    pub(super) fresh_recall_chars: usize,
    pub(super) project_instructions_chars: usize,
    pub(super) provider_history_archive_recall_ids: Vec<String>,
}

/// First agent turn ⇔ at most one user message in the transcript so far. Used to
/// emit the session-start knowledge brief exactly once, at the opening.
fn is_first_turn(messages: &[ProviderMessage]) -> bool {
    messages
        .iter()
        .filter(|m| matches!(m.role, jfc_provider::ProviderRole::User))
        .count()
        <= 1
}

pub(super) async fn append_project_context(
    system_prompt: &mut String,
    overrides: &mut StreamRequestOverrides,
    provider: &Arc<dyn Provider>,
    messages: &[ProviderMessage],
    model: &ModelId,
) -> ProjectContextStats {
    let mut stats = ProjectContextStats::default();
    if let Ok(cwd_path) = std::env::current_dir() {
        let hierarchy =
            crate::prompt_context_cache::context_hierarchy(&cwd_path, &overrides.extra_dirs);
        if let Some(layered) = hierarchy.rendered {
            system_prompt.push_str("\n\n");
            stats.project_instructions_chars += layered.len();
            system_prompt.push_str(&layered);
        }
        // Extract disallowed-tools from frontmatter and merge with CLI ones.
        let fm_disallowed = hierarchy.disallowed_tools;
        if !fm_disallowed.is_empty() {
            overrides.disallowed_tools.extend(fm_disallowed);
        }

        let memories = crate::prompt_context_cache::memories(&cwd_path);

        let config = crate::config::load_arc();
        let recall_enabled = crate::memory_recall::is_enabled(config.memory_recall_enabled);
        let plan_recall_enabled = crate::plan_recall::is_enabled(config.plan_recall_enabled);
        // Run recall on a fast (haiku) model, not the main model — the other
        // half of the cold-recall speedup (alongside running the two recalls
        // concurrently below).
        let recall_model = fast_recall_model(&provider, model);

        // Memory recall and plan recall are INDEPENDENT two-phase LLM
        // round-trips. They used to run as back-to-back `.await`s — up to ~4
        // sequential LLM calls (≈4–12s on a cold cache) blocking the turn
        // before `provider.stream()` ever fires, which is the dominant reason
        // a cold turn lags a thin client. Run them CONCURRENTLY so cold-recall
        // latency is the slower of the two, not their sum. Both are cache
        // hits / no-ops in the steady state. (`tokio::join!` runs them on this
        // one task — no extra threads, shared `&` borrows are fine.)
        let memory_fut = async {
            if recall_enabled
                && !memories.is_empty()
                && let Some(query) = last_user_text(messages)
            {
                let trimmed = query.trim();
                if !trimmed.is_empty() && !trimmed.starts_with('/') {
                    // Cache check BEFORE run_recall so we only fire the
                    // `MemoryRecalled` toast on a fresh (cache-miss) recall,
                    // not on every agentic-loop continuation.
                    let was_cached = crate::memory_recall::cached_recall(trimmed).is_some();
                    let block = crate::memory_recall::run_recall(
                        trimmed,
                        &memories,
                        provider.clone(),
                        recall_model.clone(),
                    )
                    .await;
                    let fresh = !was_cached && block.is_some();
                    return (block, fresh);
                }
            }
            (None, false)
        };
        let plan_fut = async {
            if plan_recall_enabled
                && let Ok(plan_store) = crate::plan::PlanStore::open_project(Some(&cwd_path))
            {
                let plans = plan_store.list(None);
                if !plans.is_empty()
                    && let Some(query) = last_user_text(messages)
                {
                    let trimmed = query.trim();
                    if !trimmed.is_empty() && !trimmed.starts_with('/') {
                        // `run_plan_recall` handles its own caching.
                        return crate::plan_recall::run_plan_recall(
                            trimmed,
                            &plans,
                            provider.clone(),
                            recall_model.clone(),
                        )
                        .await;
                    }
                }
            }
            None
        };
        // Bound the wait: haiku recall almost always finishes well under this,
        // but a network hiccup must never stall the turn. On timeout, proceed
        // with no recall (the full-memory-dump fallback below) — the turn
        // starts; recall just doesn't enrich this one.
        const RECALL_DEADLINE_MS: u64 = 1500;
        let ((recall_block, recall_was_fresh), plan_block) = match tokio::time::timeout(
            std::time::Duration::from_millis(RECALL_DEADLINE_MS),
            async { tokio::join!(memory_fut, plan_fut) },
        )
        .await
        {
            Ok(r) => r,
            Err(_) => {
                tracing::debug!(
                    target: "jfc::stream",
                    deadline_ms = RECALL_DEADLINE_MS,
                    "recall exceeded deadline; proceeding without it this turn"
                );
                ((None, false), None)
            }
        };

        let memory_stats = append_memory_recall_context(
            system_prompt,
            recall_block.as_ref(),
            &memories,
            recall_enabled,
            recall_was_fresh,
        );
        stats.memory_context_chars += memory_stats.prompt_chars;
        stats.fresh_recall_chars += memory_stats.fresh_recall_chars;

        // Cross-project knowledge recall (jfc-knowledge). Screened as reference
        // data. Appended after the per-project memory block.
        // On the FIRST turn, also emit a session-start "knowledge brief" so the
        // agent opens with its accumulated cross-project memory (the diagram's
        // MEMORY BANK read at session start — "never starts blind again").
        if is_first_turn(messages) {
            crate::warm_knowledge_before_prompt(
                cwd_path.clone(),
                std::time::Duration::from_millis(750),
            )
            .await;
            stats.memory_context_chars += append_session_start_knowledge_brief(
                system_prompt,
                &cwd_path,
                overrides.session_id.as_deref(),
            )
            .await;
        }
        if let Some(query) = last_user_text(messages) {
            stats.memory_context_chars += append_cross_project_knowledge(
                system_prompt,
                &cwd_path,
                &query,
                overrides.session_id.as_deref(),
            )
            .await;
        }
        if let Ok(Some(memory_store_section)) = tokio::time::timeout(
            std::time::Duration::from_millis(1500),
            sdk_memory_store_prompt_section(),
        )
        .await
        {
            stats.memory_context_chars += memory_store_section.len();
            system_prompt.push_str(&memory_store_section);
        }
        if let Some(block) = plan_block {
            tracing::debug!(
                target: "jfc::stream",
                plan_recall_block_len = block.len(),
                "appending plan recall block"
            );
            stats.memory_context_chars += block.len();
            system_prompt.push_str(&block);
        }
        if let Some(query) = last_user_text(messages) {
            let trimmed = query.trim();
            if !trimmed.is_empty()
                && !trimmed.starts_with('/')
                && let Some(recall) =
                    crate::context_accounting::provider_history_archive_recall_block(
                        trimmed,
                        3,
                        &overrides.provider_history_archive_seen,
                    )
            {
                tracing::debug!(
                    target: "jfc::stream",
                    provider_history_archive_recall_chars = recall.block.len(),
                    provider_history_archive_recall_count = recall.archive_ids.len(),
                    "appending provider-history archive recall block"
                );
                stats.memory_context_chars += recall.block.len();
                stats
                    .provider_history_archive_recall_ids
                    .extend(recall.archive_ids);
                system_prompt.push_str(&recall.block);
            }
        }

        // t221 — AutoSearchHints: scan the user's prompt for code-path /
        // symbol mentions and inject a recall hint block built from project
        // + user memory. Parallels the memory_recall / plan_recall hooks
        // but is local (no LLM call) and always cheap to run.
        if let Some(last_user_query) = last_user_text(messages) {
            let trimmed = last_user_query.trim();
            if !trimmed.is_empty()
                && !trimmed.starts_with('/')
                && let Some(hint_block) =
                    jfc_learn::auto_hints::run_pre_turn_hint(trimmed, &cwd_path)
            {
                tracing::debug!(
                    target: "jfc::stream",
                    hint_block_len = hint_block.len(),
                    "injecting auto-hint recall block"
                );
                system_prompt.push_str("\n\n");
                stats.memory_context_chars += hint_block.len();
                system_prompt.push_str(&hint_block);
            }
        }

        let git_ctx = crate::git_context::get_git_context();
        if git_ctx.current_branch.is_some() || !git_ctx.recent_commits.is_empty() {
            system_prompt.push_str("\n\n");
            system_prompt.push_str(&git_ctx.to_prompt_string());
        }

        if let Some(env_block) = crate::env_context::get().to_prompt_string() {
            system_prompt.push_str(&env_block);
        }
    }
    stats
}
