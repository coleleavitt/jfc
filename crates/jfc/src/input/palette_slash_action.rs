use crate::app::App;
use crate::runtime::EngineEvent;
use tokio::sync::mpsc;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct PaletteSlashCommand<'a> {
    raw: &'a str,
    name: &'a str,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PaletteSlashCommandParseError {
    MissingSlash,
    UnknownCommand,
}

impl<'a> PaletteSlashCommand<'a> {
    fn parse(raw: &'a str) -> Result<Self, PaletteSlashCommandParseError> {
        let raw = raw.trim();
        let Some(name) = raw.split_whitespace().next() else {
            return Err(PaletteSlashCommandParseError::MissingSlash);
        };
        if !name.starts_with('/') {
            return Err(PaletteSlashCommandParseError::MissingSlash);
        }
        if !super::slash_commands_table()
            .iter()
            .any(|(command, _)| *command == name)
        {
            return Err(PaletteSlashCommandParseError::UnknownCommand);
        }
        Ok(Self { raw, name })
    }

    fn as_str(self) -> &'a str {
        self.raw
    }

    fn is_compact(self) -> bool {
        self.name == "/compact"
    }
}

pub(super) async fn execute_palette_slash_command_name(
    app: &mut App,
    command: &str,
    tx: &mpsc::Sender<EngineEvent>,
) {
    match PaletteSlashCommand::parse(command) {
        Ok(command) => execute_palette_slash_command(app, command, tx).await,
        Err(error) => {
            tracing::warn!(
                target: "jfc::palette",
                command,
                error = ?error,
                "rejected invalid palette slash command"
            );
        }
    }
}

pub(super) async fn execute_palette_slash_command(
    app: &mut App,
    command: PaletteSlashCommand<'_>,
    tx: &mpsc::Sender<EngineEvent>,
) {
    if command.is_compact() {
        tracing::info!(
            target: "jfc::compact",
            model = %app.engine.model,
            message_count = app.engine.messages.len(),
            "palette: Compact Conversation triggered"
        );
    };
    super::run_slash_command_with_tx(app, command.as_str(), tx).await;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn palette_slash_command_parse_accepts_known_command_normal() {
        let command = PaletteSlashCommand::parse("/help").expect("known slash command");

        assert_eq!(command.as_str(), "/help");
    }

    #[test]
    fn palette_slash_command_parse_accepts_known_command_with_args_normal() {
        let command =
            PaletteSlashCommand::parse("/theme catppuccin").expect("known slash command with args");

        assert_eq!(command.as_str(), "/theme catppuccin");
    }

    #[test]
    fn palette_slash_command_parse_rejects_missing_slash_robust() {
        assert_eq!(
            PaletteSlashCommand::parse("help"),
            Err(PaletteSlashCommandParseError::MissingSlash)
        );
    }

    #[test]
    fn palette_slash_command_parse_rejects_unknown_command_robust() {
        assert_eq!(
            PaletteSlashCommand::parse("/definitely-not-real"),
            Err(PaletteSlashCommandParseError::UnknownCommand)
        );
    }
}
