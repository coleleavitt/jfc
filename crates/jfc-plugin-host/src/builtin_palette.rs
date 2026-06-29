use jfc_plugin_sdk::{
    DescriptorVisibility, ExtensionSlot, PluginId, RuntimeActionDescriptor, RuntimeActionKind,
    UiSlotDescriptor,
};

#[derive(Clone, Copy)]
enum BuiltinPaletteAction {
    Host(&'static str),
    Slash(&'static str),
    Runtime(RuntimeActionKind),
}

const COMMAND_PALETTE_SLOTS: &[(&str, &str, i32, BuiltinPaletteAction)] = &[
    (
        "command_palette.clear_messages",
        "Clear Messages (/clear)",
        120,
        BuiltinPaletteAction::Host("clear_messages"),
    ),
    (
        "command_palette.compact_conversation",
        "Compact Conversation (/compact)",
        119,
        BuiltinPaletteAction::Slash("/compact"),
    ),
    (
        "command_palette.continue_session",
        "Continue Most Recent Session (/continue)",
        118,
        BuiltinPaletteAction::Slash("/continue"),
    ),
    (
        "command_palette.toggle_sessions_sidebar",
        "Toggle Sessions Sidebar (Ctrl+B)",
        117,
        BuiltinPaletteAction::Host("toggle_sessions_sidebar"),
    ),
    (
        "command_palette.toggle_info_sidebar",
        "Toggle Info Sidebar (Ctrl+S)",
        116,
        BuiltinPaletteAction::Host("toggle_info_sidebar"),
    ),
    (
        "command_palette.open_model_picker",
        "Open Model Picker (Ctrl+M)",
        115,
        BuiltinPaletteAction::Host("open_model_picker"),
    ),
    (
        "command_palette.open_theme_picker",
        "Open Theme Picker (/theme)",
        114,
        BuiltinPaletteAction::Host("open_theme_picker"),
    ),
    (
        "command_palette.theme_catppuccin",
        "Use Catppuccin Theme (/theme catppuccin)",
        113,
        BuiltinPaletteAction::Host("theme_catppuccin"),
    ),
    (
        "command_palette.theme_tokyo_night",
        "Use Tokyo Night Theme (/theme tokyo-night)",
        112,
        BuiltinPaletteAction::Host("theme_tokyo_night"),
    ),
    (
        "command_palette.theme_gruvbox",
        "Use Gruvbox Theme (/theme gruvbox)",
        111,
        BuiltinPaletteAction::Host("theme_gruvbox"),
    ),
    (
        "command_palette.toggle_thinking",
        "Toggle Thinking (Ctrl+O)",
        110,
        BuiltinPaletteAction::Host("toggle_thinking"),
    ),
    (
        "command_palette.raise_reasoning_effort",
        "Raise Reasoning Effort (Alt+.)",
        109,
        BuiltinPaletteAction::Host("raise_reasoning_effort"),
    ),
    (
        "command_palette.lower_reasoning_effort",
        "Lower Reasoning Effort (Alt+,)",
        108,
        BuiltinPaletteAction::Host("lower_reasoning_effort"),
    ),
    (
        "command_palette.show_tasks",
        "Show Tasks (/tasks)",
        107,
        BuiltinPaletteAction::Slash("/tasks"),
    ),
    (
        "command_palette.show_help",
        "Show Help (/help)",
        106,
        BuiltinPaletteAction::Slash("/help"),
    ),
    (
        "command_palette.plugin_diagnostics",
        "Run Plugin Diagnostics",
        91,
        BuiltinPaletteAction::Runtime(RuntimeActionKind::PluginDiagnostics),
    ),
    (
        "command_palette.sessions",
        "Run /sessions",
        90,
        BuiltinPaletteAction::Slash("/sessions"),
    ),
    (
        "command_palette.config",
        "Run /config",
        89,
        BuiltinPaletteAction::Slash("/config"),
    ),
    (
        "command_palette.doctor",
        "Run /doctor",
        88,
        BuiltinPaletteAction::Slash("/doctor"),
    ),
    (
        "command_palette.diff",
        "Run /diff",
        87,
        BuiltinPaletteAction::Slash("/diff"),
    ),
    (
        "command_palette.memory",
        "Run /memory",
        86,
        BuiltinPaletteAction::Slash("/memory"),
    ),
    (
        "command_palette.skills",
        "Run /skills",
        85,
        BuiltinPaletteAction::Slash("/skills"),
    ),
    (
        "command_palette.commit",
        "Run /commit",
        84,
        BuiltinPaletteAction::Slash("/commit"),
    ),
    (
        "command_palette.review",
        "Run /review",
        83,
        BuiltinPaletteAction::Slash("/review"),
    ),
    (
        "command_palette.status",
        "Run /status",
        82,
        BuiltinPaletteAction::Slash("/status"),
    ),
    (
        "command_palette.agents",
        "Run /agents",
        81,
        BuiltinPaletteAction::Slash("/agents"),
    ),
    (
        "command_palette.claude_md",
        "Run /claude-md",
        80,
        BuiltinPaletteAction::Slash("/claude-md"),
    ),
    (
        "command_palette.market",
        "Run /market",
        79,
        BuiltinPaletteAction::Slash("/market"),
    ),
    (
        "command_palette.timeline",
        "Run /timeline",
        78,
        BuiltinPaletteAction::Slash("/timeline"),
    ),
    (
        "command_palette.export",
        "Run /export",
        77,
        BuiltinPaletteAction::Slash("/export"),
    ),
];

pub(crate) fn command_palette_slot_descriptors(
    plugin_id: PluginId,
) -> impl Iterator<Item = UiSlotDescriptor> {
    COMMAND_PALETTE_SLOTS
        .iter()
        .map(move |(id, label, priority, action)| {
            let descriptor = UiSlotDescriptor::new(
                plugin_id.clone(),
                ExtensionSlot::CommandPalette,
                *id,
                *label,
            )
            .with_priority(*priority)
            .with_visibility(DescriptorVisibility::HostVisible);
            match action {
                BuiltinPaletteAction::Host(action) => descriptor.with_host_action(*action),
                BuiltinPaletteAction::Slash(command) => descriptor.with_slash_command(*command),
                BuiltinPaletteAction::Runtime(_) => descriptor,
            }
        })
}

pub(crate) fn command_palette_runtime_action_descriptors(
    plugin_id: PluginId,
) -> impl Iterator<Item = RuntimeActionDescriptor> {
    COMMAND_PALETTE_SLOTS
        .iter()
        .map(move |(id, label, priority, action)| {
            let kind = match action {
                BuiltinPaletteAction::Host(_) => RuntimeActionKind::HostAction,
                BuiltinPaletteAction::Slash(_) => RuntimeActionKind::SlashCommand,
                BuiltinPaletteAction::Runtime(kind) => *kind,
            };
            let descriptor = RuntimeActionDescriptor::new(
                plugin_id.clone(),
                *id,
                *label,
                "Built-in command-palette runtime action",
                kind,
            )
            .with_priority(*priority)
            .with_visibility(DescriptorVisibility::HostVisible);
            match action {
                BuiltinPaletteAction::Host(action) => descriptor.with_host_action(*action),
                BuiltinPaletteAction::Slash(command) => descriptor.with_slash_command(*command),
                BuiltinPaletteAction::Runtime(_) => descriptor,
            }
        })
}
