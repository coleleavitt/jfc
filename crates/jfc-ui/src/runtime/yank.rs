use crate::{
    app::App,
    types::{MessagePart, Role},
};

pub(crate) fn yank_last_assistant(app: &App) {
    let Some(text) = app
        .messages
        .iter()
        .rev()
        .find(|message| message.role == Role::Assistant)
        .map(|message| {
            message
                .parts
                .iter()
                .filter_map(|part| match part {
                    MessagePart::Text(text) => Some(text.clone()),
                    _ => None,
                })
                .collect::<Vec<_>>()
                .join("\n")
        })
        .filter(|text| !text.is_empty())
    else {
        return;
    };

    match arboard::Clipboard::new() {
        Ok(mut clipboard) => {
            if let Err(error) = clipboard.set_text(text.clone()) {
                tracing::warn!(target: "jfc::ui::yank", error = %error, "set_text failed");
            } else {
                tracing::info!(
                    target: "jfc::ui::yank",
                    len = text.len(),
                    "yanked via mouse click"
                );
            }
        }
        Err(error) => {
            tracing::warn!(
                target: "jfc::ui::yank",
                error = %error,
                "clipboard backend unavailable"
            );
        }
    }
}
