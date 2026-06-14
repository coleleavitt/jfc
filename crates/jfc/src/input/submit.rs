use super::slash_commands::handle_slash_command;
use super::*;
pub async fn handle_submit_text(
    app: &mut App,
    text: String,
    tx: &mpsc::Sender<crate::runtime::EngineEvent>,
) -> anyhow::Result<()> {
    handle_submit(app, text, tx).await
}

pub(super) async fn handle_submit(
    app: &mut App,
    text: String,
    tx: &mpsc::Sender<crate::runtime::EngineEvent>,
) -> anyhow::Result<()> {
    // `# <fact>` quick-add: a prompt whose first non-space character is `#`
    // is a memory note, not a turn. Append it to project memory (so the
    // recall engine can surface it later) and stop — no stream, no model.
    // Mirrors Claude Code's `#` shortcut. `#` alone (empty body) falls
    // through and is sent normally.
    {
        let trimmed = text.trim_start();
        let fact = trimmed.trim_start_matches('#').trim();
        if trimmed.starts_with('#') && !fact.is_empty() {
            let root = app
                .engine
                .git_root
                .clone()
                .flatten()
                .unwrap_or_else(|| std::path::PathBuf::from(&app.engine.cwd));
            let toast_msg = match jfc_memory::create_memory(
                jfc_memory::MemoryLevel::Project,
                jfc_memory::MemoryType::Context,
                jfc_memory::MemoryScope::Private,
                fact,
                &root,
            ) {
                Ok(path) => format!(
                    "remembered → {}",
                    path.file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or("memory")
                ),
                Err(e) => format!("memory save failed: {e}"),
            };
            jfc_engine::toast::push_with_cap(
                &mut app.engine.toasts,
                jfc_engine::toast::Toast::new(jfc_engine::toast::ToastKind::Info, toast_msg),
            );
            // Clear the prompt; this was a note, not a turn.
            app.textarea.select_all();
            app.textarea.cut();
            return Ok(());
        }
    }

    // Expand any `[Pasted #N · …]` chips back to their full text so the model
    // receives what the user actually pasted, then drop the stash for the next
    // turn. A chip the user deleted simply won't match — harmless.
    let text = if app.pasted_texts.is_empty() {
        text
    } else {
        let mut expanded = text;
        for (chip, content) in &app.pasted_texts {
            expanded = expanded.replace(chip.as_str(), content);
        }
        app.pasted_texts.clear();
        expanded
    };

    if app.engine.compacting_started_at.is_some() {
        tracing::info!(
            target: "jfc::ui::queue",
            "handle_submit: compaction active — queueing prompt instead of starting stream"
        );
        super::key_dispatch::queue_prompt_for_later(app, text);
        return Ok(());
    }

    tracing::info!(
        target: "jfc::input",
        text_len = text.len(),
        text_preview = %text.chars().take(80).collect::<String>(),
        model = %app.engine.model,
        message_count = app.engine.messages.len(),
        editing_idx = ?app.editing_message_idx,
        "handle_submit"
    );

    // Pasted-image extraction ([Image #N] refs) is frontend state — match
    // and consume the staged attachments for this turn before handing the
    // prompt to the engine.
    let submit_attachments: Vec<crate::attachments::Attachment> = if !app.pasted_images.is_empty() {
        let mut referenced_ids: Vec<u32> = Vec::new();
        let re_pattern = regex::Regex::new(r"\[Image #(\d+)\]").unwrap();
        for cap in re_pattern.captures_iter(&text) {
            if let Ok(id) = cap[1].parse::<u32>() {
                referenced_ids.push(id);
            }
        }

        let mut matched: Vec<crate::attachments::Attachment> = Vec::new();
        let mut remaining: Vec<crate::attachments::PastedContent> = Vec::new();
        for pc in std::mem::take(&mut app.pasted_images) {
            if referenced_ids.contains(&pc.id) {
                matched.push(pc.attachment);
            } else {
                remaining.push(pc);
            }
        }
        if !remaining.is_empty() {
            tracing::info!(
                target: "jfc::input::paste",
                dropped = remaining.len(),
                "dropping unreferenced pasted images (markers deleted by user)"
            );
        }
        tracing::info!(
            target: "jfc::input::paste",
            matched = matched.len(),
            "matched [Image #N] attachments for submit"
        );
        matched
    } else {
        Vec::new()
    };

    // Edit-resubmit cursor is frontend state; the engine performs the
    // history rewrite given the index.
    let edit_at = app.editing_message_idx.take();

    if text.starts_with('/') {
        // `/check` re-runs the cargo-check producer. Handled here (not in
        // `handle_slash_command`) because it needs the tx channel to emit
        // `DiagnosticsUpdated` from a spawned task.
        if text.trim() == "/check" {
            let tx_diag = tx.clone();
            let cwd = std::env::current_dir().unwrap_or_else(|_| ".".into());
            tokio::spawn(async move {
                jfc_engine::diagnostics_producer::run_once(cwd, tx_diag).await;
            });
        }
        handle_slash_command(app, &text, Some(tx)).await;
        return Ok(());
    }

    // Everything from hooks through stream spawn is the engine's submit op.
    let _outcome =
        crate::runtime::ops::submit_prompt(&mut app.engine, tx, text, submit_attachments, edit_at)
            .await?;
    Ok(())
}
