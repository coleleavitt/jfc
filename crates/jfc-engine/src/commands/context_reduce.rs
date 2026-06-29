use crate::commands::prelude::*;
use jfc_context::{ContextDropSpec, ContextReduceOptions, PlannedContextDrops};

#[cfg(test)]
mod tests;

pub(super) async fn cmd_ctx_reduce(
    state: &mut EngineState,
    _parts: &[&str],
    text: &str,
    _tx: Option<&mpsc::Sender<EngineEvent>>,
) {
    let _linkscope_cmd = linkscope::phase("command.ctx_reduce");
    let Some(drop) = command_drop_text(text) else {
        push_ctx_reduce_reply(
            state,
            text,
            "Usage: `/ctx-reduce <tags>` or `/ctx-reduce drop=<tags>` where tags look like `3-5,8`.",
        );
        return;
    };

    let spec = match ContextDropSpec::parse(drop) {
        Ok(spec) => spec,
        Err(error) => {
            push_ctx_reduce_reply(
                state,
                text,
                &format!("ctx_reduce rejected `{drop}`: {error}."),
            );
            return;
        }
    };

    let tags =
        crate::context_reduction::transcript_tags(&state.messages, &state.context_reduction_queue);
    let plan = match PlannedContextDrops::plan(
        &tags,
        &spec,
        ContextReduceOptions::new(crate::context_reduction::PROTECTED_TAIL_MESSAGES),
    ) {
        Ok(plan) => plan,
        Err(error) => {
            push_ctx_reduce_reply(
                state,
                text,
                &format!("ctx_reduce could not queue drops: {error}."),
            );
            return;
        }
    };

    state
        .context_reduction_queue
        .extend(plan.queued().iter().cloned());
    state
        .context_reduction_queue
        .extend(plan.protected_tail_skips().iter().cloned());
    linkscope::record_items(
        "command.ctx_reduce.queued_ranges",
        usize_to_u64_saturating(plan.queued().len()),
    );
    linkscope::record_items(
        "command.ctx_reduce.deferred_ranges",
        usize_to_u64_saturating(plan.protected_tail_skips().len()),
    );
    push_ctx_reduce_reply(state, text, &format_plan_result(&plan));
    crate::runtime::session_save::request_save(state);
}

fn command_drop_text(text: &str) -> Option<&str> {
    let args = text.trim().split_once(char::is_whitespace)?.1.trim();
    if args.is_empty() {
        return None;
    }

    args.strip_prefix("drop=")
        .or_else(|| args.strip_prefix("--drop="))
        .or_else(|| args.strip_prefix("--drop ").map(str::trim_start))
        .or(Some(args))
        .map(str::trim)
        .filter(|drop| !drop.is_empty())
}

fn push_ctx_reduce_reply(state: &mut EngineState, text: &str, reply: &str) {
    state.messages.push(ChatMessage::user(text.to_owned()));
    state
        .messages
        .push(ChatMessage::assistant(reply.to_owned()));
}

fn format_plan_result(plan: &PlannedContextDrops) -> String {
    let queued = queued_tag_count(plan.queued());
    let deferred = queued_tag_count(plan.protected_tail_skips());
    let mut lines = vec![format!(
        "ctx_reduce queued {queued} drop{} across {} range{}.",
        plural(queued),
        plan.queued().len(),
        plural(plan.queued().len())
    )];
    if !plan.protected_tail_skips().is_empty() {
        lines.push(format!(
            "Protected tail deferred {deferred} drop{} across {} range{}: {}.",
            plural(deferred),
            plan.protected_tail_skips().len(),
            plural(plan.protected_tail_skips().len()),
            format_ranges(plan.protected_tail_skips())
        ));
    }
    if !plan.already_pending().is_empty() {
        lines.push(format!(
            "Already queued: {}.",
            format_tags(plan.already_pending())
        ));
    }
    if !plan.already_dropped().is_empty() {
        lines.push(format!(
            "Already dropped: {}.",
            format_tags(plan.already_dropped())
        ));
    }

    lines.join("\n")
}

fn queued_tag_count(drops: &[jfc_context::QueuedContextDrop]) -> usize {
    drops
        .iter()
        .map(|drop| {
            let range = drop.range();
            match usize::try_from(range.end() - range.start() + 1) {
                Ok(width) => width,
                Err(_) => usize::MAX,
            }
        })
        .sum()
}

fn format_ranges(drops: &[jfc_context::QueuedContextDrop]) -> String {
    drops
        .iter()
        .map(|drop| {
            let range = drop.range();
            if range.start() == range.end() {
                format!("§{}§", range.start())
            } else {
                format!("§{}§-§{}§", range.start(), range.end())
            }
        })
        .collect::<Vec<_>>()
        .join(", ")
}

fn format_tags(tags: &[jfc_context::ContextTagId]) -> String {
    tags.iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(", ")
}

fn plural(count: usize) -> &'static str {
    if count == 1 { "" } else { "s" }
}

fn usize_to_u64_saturating(value: usize) -> u64 {
    u64::try_from(value).unwrap_or(u64::MAX)
}
