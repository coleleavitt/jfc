use crate::app::App;
use jfc_core::ChatMessage;

use super::theme_picker::{apply_theme, open_theme_picker};

pub(super) async fn execute_palette_action(app: &mut App, label: &str) {
    match label {
        "Clear Messages (/clear)" => {
            app.engine.messages.clear();
            app.engine.streaming_text.clear();
            app.engine.streaming_reasoning.clear();
            app.engine.streaming_response_bytes = 0;
            app.engine.streaming_assistant_idx = None;
            app.switch_session(None);
        }
        "Compact Conversation (/compact)" => {
            tracing::info!(
                target: "jfc::compact",
                model = %app.engine.model,
                message_count = app.engine.messages.len(),
                "palette: Compact Conversation triggered"
            );
            app.engine.force_compact_pending = true;
            app.engine
                .messages
                .push(ChatMessage::user("/compact".into()));
            app.engine.messages.push(ChatMessage::assistant(
                "Compaction queued — runs on the next turn.".into(),
            ));
        }
        "Toggle Sessions Sidebar (Ctrl+B)" => {
            app.show_sidebar = !app.show_sidebar;
            if app.show_sidebar {
                app.session_meta = jfc_session::list_sessions_with_metadata().await;
            }
        }
        "Toggle Info Sidebar (Ctrl+S)" => {
            app.show_info_sidebar = !app.show_info_sidebar;
        }
        "Open Model Picker (Ctrl+M)" => {
            app.show_model_picker = true;
            app.model_picker_filter.clear();
            app.model_picker_selected = 0;
            app.model_picker_models = collect_all_models(app);
        }
        "Open Theme Picker (/theme)" => open_theme_picker(app),
        "Use Catppuccin Theme (/theme catppuccin)" => apply_theme(app, "catppuccin"),
        "Use Tokyo Night Theme (/theme tokyo-night)" => apply_theme(app, "tokyo-night"),
        "Use Gruvbox Theme (/theme gruvbox)" => apply_theme(app, "gruvbox"),
        "Toggle Thinking (Ctrl+O)" => {
            if let Some(idx) = app.engine.messages.len().checked_sub(1) {
                let entry = app.reasoning_expanded.entry(idx).or_insert(false);
                *entry = !*entry;
            }
        }
        "Raise Reasoning Effort (Alt+.)" => {
            super::step_reasoning_effort(app, true);
        }
        "Lower Reasoning Effort (Alt+,)" => {
            super::step_reasoning_effort(app, false);
        }
        "Continue Most Recent Session (/continue)" => {
            super::run_slash_command(app, "/continue").await;
        }
        "Show Tasks (/tasks)" => {
            super::run_slash_command(app, "/tasks").await;
        }
        "Show Help (/help)" => {
            super::run_slash_command(app, "/help").await;
        }
        other if other.starts_with("Run /") => {
            if let Some(command) = other.strip_prefix("Run ") {
                super::run_slash_command(app, command).await;
            }
        }
        _ => {}
    }
}

pub fn palette_items(app: &App) -> Vec<&'static str> {
    let all: &[&str] = &[
        "Clear Messages (/clear)",
        "Compact Conversation (/compact)",
        "Continue Most Recent Session (/continue)",
        "Toggle Sessions Sidebar (Ctrl+B)",
        "Toggle Info Sidebar (Ctrl+S)",
        "Open Model Picker (Ctrl+M)",
        "Open Theme Picker (/theme)",
        "Use Catppuccin Theme (/theme catppuccin)",
        "Use Tokyo Night Theme (/theme tokyo-night)",
        "Use Gruvbox Theme (/theme gruvbox)",
        "Toggle Thinking (Ctrl+O)",
        "Raise Reasoning Effort (Alt+.)",
        "Lower Reasoning Effort (Alt+,)",
        "Show Tasks (/tasks)",
        "Show Help (/help)",
        "Run /sessions",
        "Run /config",
        "Run /doctor",
        "Run /diff",
        "Run /memory",
        "Run /skills",
        "Run /commit",
        "Run /review",
        "Run /status",
        "Run /agents",
        "Run /claude-md",
        "Run /market",
        "Run /timeline",
        "Run /export",
    ];
    if app.palette_input.is_empty() {
        all.to_vec()
    } else {
        let needle = app.palette_input.to_lowercase();
        all.iter()
            .filter(|item| item.to_lowercase().contains(&needle))
            .copied()
            .collect()
    }
}

pub fn collect_all_models(app: &App) -> Vec<jfc_provider::ModelInfo> {
    let fingerprint_input: Vec<_> = app
        .engine
        .providers
        .iter()
        .map(|provider| {
            let models = app
                .engine
                .provider_models
                .get(provider.name())
                .filter(|models| !models.is_empty())
                .cloned()
                .unwrap_or_else(|| provider.available_models());
            (
                provider.name().to_string(),
                models
                    .iter()
                    .map(|model| {
                        (
                            model.provider.to_string(),
                            model.id.to_string(),
                            model.display_name.clone(),
                            model.context_window_tokens,
                        )
                    })
                    .collect::<Vec<_>>(),
            )
        })
        .collect::<Vec<_>>();
    let key = crate::query::QueryKey::ModelPickerModels(crate::query::Fingerprint::new((
        &fingerprint_input,
        app.engine.seat_tier.as_deref(),
    )));

    let all = app.model_picker_query_cache.get_or_insert_with(key, || {
        let merged = fingerprint_input
            .iter()
            .flat_map(|(provider_name, _)| {
                app.engine
                    .provider_models
                    .get(provider_name.as_str())
                    .filter(|models| !models.is_empty())
                    .cloned()
                    .unwrap_or_else(|| {
                        app.engine
                            .providers
                            .iter()
                            .find(|provider| provider.name() == provider_name)
                            .map(|provider| provider.available_models())
                            .unwrap_or_default()
                    })
            })
            .collect();
        jfc_engine::providers::anthropic_models::apply_seat_tier_filter(
            merged,
            app.engine.seat_tier.as_deref(),
        )
    });

    if !app.engine.recent_models.is_empty() {
        let recent = &app.engine.recent_models;
        let mut sorted: Vec<jfc_provider::ModelInfo> = Vec::with_capacity(all.len());
        for recent_model in recent {
            if let Some(model) = all
                .iter()
                .find(|model| model.id.as_str() == recent_model.as_str())
            {
                sorted.push(model.clone());
            }
        }
        for model in &all {
            if !recent.contains(&model.id.to_string()) {
                sorted.push(model.clone());
            }
        }
        sorted
    } else {
        all
    }
}
