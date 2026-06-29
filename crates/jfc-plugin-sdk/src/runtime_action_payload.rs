use crate::{RuntimeActionDescriptor, RuntimeActionKind, RuntimeActionOpenPanelTarget};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeActionPayloadError {
    MissingHostAction,
    MissingSlashCommand,
    InvalidSlashCommand,
    MissingOpenPanel,
    UnsupportedOpenPanelTarget,
    InvalidOpenPanelExecuteFlag,
    InvalidPluginSmokeTarget,
}

impl RuntimeActionPayloadError {
    pub const fn as_manifest_reason(self) -> &'static str {
        match self {
            Self::MissingHostAction => "host_action requires a non-empty string payload.action",
            Self::MissingSlashCommand => {
                "slash_command requires a non-empty string payload.command"
            }
            Self::InvalidSlashCommand => "slash_command payload.command must start with /",
            Self::MissingOpenPanel => "open_panel requires a non-empty string payload.panel",
            Self::UnsupportedOpenPanelTarget => {
                "open_panel payload.panel is not a supported panel target"
            }
            Self::InvalidOpenPanelExecuteFlag => "open_panel execute flags must be booleans",
            Self::InvalidPluginSmokeTarget => {
                "plugin_smoke payload plugin/name must be non-empty strings"
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RuntimeActionOpenPanelPayload<'a> {
    pub target: RuntimeActionOpenPanelTarget,
    pub panel_id: Option<&'a str>,
    pub panel_plugin_id: Option<&'a str>,
    pub widget_id: Option<&'a str>,
    pub widget_plugin_id: Option<&'a str>,
    pub plugin_id: Option<&'a str>,
    pub execute_panel_action: bool,
    pub execute_widget_action: bool,
}

impl RuntimeActionDescriptor {
    pub fn validate_payload(&self) -> Result<(), RuntimeActionPayloadError> {
        match self.kind {
            RuntimeActionKind::HostAction => self.host_action_payload().map(|_| ()),
            RuntimeActionKind::SlashCommand => self.slash_command_payload().map(|_| ()),
            RuntimeActionKind::OpenPanel => self.open_panel_payload().map(|_| ()),
            RuntimeActionKind::PluginSmoke => self.validate_plugin_smoke_payload(),
            RuntimeActionKind::RefreshMetrics
            | RuntimeActionKind::SendTeammateMessage
            | RuntimeActionKind::RefreshPromptContext
            | RuntimeActionKind::PluginDiagnostics => Ok(()),
        }
    }

    pub fn host_action_payload(&self) -> Result<&str, RuntimeActionPayloadError> {
        self.payload_text("action")
            .ok_or(RuntimeActionPayloadError::MissingHostAction)
    }

    pub fn slash_command_payload(&self) -> Result<&str, RuntimeActionPayloadError> {
        let command = self
            .payload_text("command")
            .ok_or(RuntimeActionPayloadError::MissingSlashCommand)?;
        if command.starts_with('/') {
            Ok(command)
        } else {
            Err(RuntimeActionPayloadError::InvalidSlashCommand)
        }
    }

    pub fn open_panel_payload(
        &self,
    ) -> Result<RuntimeActionOpenPanelPayload<'_>, RuntimeActionPayloadError> {
        let panel = self
            .payload_text("panel")
            .ok_or(RuntimeActionPayloadError::MissingOpenPanel)?;
        let target = RuntimeActionOpenPanelTarget::parse(panel)
            .ok_or(RuntimeActionPayloadError::UnsupportedOpenPanelTarget)?;
        let execute_panel_action = self.payload_bool("execute_panel_action")?.unwrap_or(false);
        let execute_widget_action = self.payload_bool("execute_widget_action")?.unwrap_or(false);
        Ok(RuntimeActionOpenPanelPayload {
            target,
            panel_id: self.payload_text("panel_id"),
            panel_plugin_id: self.payload_text("panel_plugin_id"),
            widget_id: self.payload_text("widget_id"),
            widget_plugin_id: self.payload_text("widget_plugin_id"),
            plugin_id: self.payload_text("plugin_id"),
            execute_panel_action,
            execute_widget_action,
        })
    }

    pub fn plugin_smoke_target(&self) -> Result<&str, RuntimeActionPayloadError> {
        self.validate_plugin_smoke_payload()?;
        Ok(self
            .payload_text("plugin")
            .or_else(|| self.payload_text("name"))
            .unwrap_or_else(|| self.plugin_id.as_str()))
    }

    pub fn payload_text(&self, key: &str) -> Option<&str> {
        self.payload
            .as_ref()?
            .get(key)?
            .as_str()
            .filter(|value| !value.trim().is_empty())
    }

    fn payload_bool(&self, key: &str) -> Result<Option<bool>, RuntimeActionPayloadError> {
        let Some(value) = self.payload.as_ref().and_then(|payload| payload.get(key)) else {
            return Ok(None);
        };
        value
            .as_bool()
            .map(Some)
            .ok_or(RuntimeActionPayloadError::InvalidOpenPanelExecuteFlag)
    }

    fn validate_plugin_smoke_payload(&self) -> Result<(), RuntimeActionPayloadError> {
        let Some(payload) = self.payload.as_ref() else {
            return Ok(());
        };
        for key in ["plugin", "name"] {
            let Some(value) = payload.get(key) else {
                continue;
            };
            if value.as_str().is_none_or(|text| text.trim().is_empty()) {
                return Err(RuntimeActionPayloadError::InvalidPluginSmokeTarget);
            }
        }
        Ok(())
    }
}
