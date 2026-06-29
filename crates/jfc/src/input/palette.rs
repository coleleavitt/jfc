use crate::app::App;
use jfc_plugin_sdk::{ExtensionSlot, UiSlotDescriptor};

const FALLBACK_PALETTE_ITEMS: &[&str] = &[
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

pub fn palette_items(app: &App) -> Vec<String> {
    let all = command_palette_labels(&app.plugins.ui_slots);
    let all = if all.is_empty() {
        FALLBACK_PALETTE_ITEMS
            .iter()
            .map(|label| (*label).to_owned())
            .collect()
    } else {
        all
    };

    if app.palette.input.is_empty() {
        all
    } else {
        let needle = app.palette.input.to_lowercase();
        all.into_iter()
            .filter(|item| item.to_lowercase().contains(&needle))
            .collect()
    }
}

fn command_palette_labels(slots: &[UiSlotDescriptor]) -> Vec<String> {
    let descriptors = command_palette_slots(slots);
    descriptors
        .into_iter()
        .map(|slot| slot.label.clone())
        .collect()
}

pub(super) fn command_palette_slots(slots: &[UiSlotDescriptor]) -> Vec<&UiSlotDescriptor> {
    let mut descriptors = slots
        .iter()
        .filter(|slot| slot.slot == ExtensionSlot::CommandPalette)
        .collect::<Vec<_>>();
    descriptors.sort_by(|left, right| {
        right
            .priority
            .cmp(&left.priority)
            .then_with(|| left.label.cmp(&right.label))
    });
    descriptors
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

    let all = app.model_picker.query_cache.get_or_insert_with(key, || {
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
