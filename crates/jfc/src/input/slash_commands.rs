//! Slash-command dispatch — frontend side.
//!
//! Since stage 8 of the jfc-engine extraction, command SEMANTICS live in
//! `jfc_engine::commands` (one registry, shared by every frontend); this
//! module keeps only the VIEW commands — anything whose behavior is frontend
//! state (modal editing, help overlay, theme picker, clipboard, the
//! remote-control sidecar) — and falls through to the engine for the rest.
//!
//! The `slash_commands!` macro still exists for the one job a macro is
//! blessed for: generating a static table AND a dispatch match from one
//! declarative list, so the two can never drift.

use super::view_commands::*;
use super::*;
use crate::runtime::EngineEvent;

/// Generate the `VIEW_SLASH_COMMANDS` metadata table and the `dispatch_view`
/// match from a single declarative list.
macro_rules! slash_commands {
    (
        $( $canon:literal $([ $($alias:literal),* $(,)? ])? $help:literal => $handler:ident ),* $(,)?
    ) => {
        /// Frontend-owned slash commands (table half). The full autocomplete
        /// surface is [`slash_commands_table`], which merges the engine's
        /// registry.
        pub(crate) const VIEW_SLASH_COMMANDS: &[(&str, &str)] = &[
            $(
                ($canon, $help),
                $( $( ($alias, $help), )* )?
            )*
        ];

        /// Route a view command to its handler; `false` when the name is not
        /// a view command (the caller then asks the engine).
        async fn dispatch_view(
            app: &mut App,
            parts: &[&str],
            text: &str,
            tx: Option<&mpsc::Sender<EngineEvent>>,
        ) -> bool {
            match parts[0] {
                $(
                    $canon $( $(| $alias)* )? => {
                        $handler(app, parts, text, tx).await;
                        true
                    }
                )*
                _ => false,
            }
        }
    };
}

slash_commands! {
        "/remote-control" ["/rc"] "toggle remote-control server (WS on port 4242)" => cmd_remote_control,
        "/copy" [] "copy transcript text to the clipboard (last/all/N)" => cmd_copy,
        "/verbose" [] "toggle expanded-by-default tool blocks" => cmd_verbose,
        "/vim" [] "toggle vim modal editing in the prompt" => cmd_vim,
        "/help" [] "show jfc help" => cmd_help,
        "/theme" [] "open picker or switch theme" => cmd_theme,
        "/voice" [] "voice mode: /voice [hold|tap|vad|off|doctor]" => cmd_voice,
}

/// The complete autocomplete/help surface: view commands plus the engine's
/// registry, deduped by name (first definition wins). Built once and leaked —
/// `command_spec::register_slash_commands` wants a `'static` slice.
pub(crate) fn slash_commands_table() -> &'static [(&'static str, &'static str)] {
    static TABLE: std::sync::OnceLock<&'static [(&'static str, &'static str)]> =
        std::sync::OnceLock::new();
    TABLE.get_or_init(|| {
        let mut seen = std::collections::HashSet::new();
        let mut merged: Vec<(&'static str, &'static str)> = Vec::new();
        for &(name, help) in VIEW_SLASH_COMMANDS
            .iter()
            .chain(jfc_engine::commands::ENGINE_SLASH_COMMANDS.iter())
        {
            if seen.insert(name) {
                merged.push((name, help));
            }
        }
        Box::leak(merged.into_boxed_slice())
    })
}

pub async fn run_slash_command(app: &mut App, text: &str) {
    handle_slash_command(app, text, None).await
}

pub async fn run_slash_command_with_tx(app: &mut App, text: &str, tx: &mpsc::Sender<EngineEvent>) {
    handle_slash_command(app, text, Some(tx)).await
}

pub(super) async fn handle_slash_command(
    app: &mut App,
    text: &str,
    tx: Option<&mpsc::Sender<EngineEvent>>,
) {
    let parts: Vec<&str> = text.splitn(2, ' ').collect();
    if !dispatch_view(app, &parts, text, tx).await {
        // Engine semantics (or skill fallthrough) — shared with headless and
        // remote frontends.
        let _ = jfc_engine::commands::run_command(&mut app.engine, text, tx).await;
    }
    app.scroll_to_bottom();
}

/// Minimal application/x-www-form-urlencoded encoder for query strings.
/// Pulling in `urlencoding` or `url` for the two callers (`/bug` form
/// link generation) is overkill — the encoder only needs to handle ASCII
/// + UTF-8 bytes that browsers reliably decode.
pub fn url_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char);
            }
            b' ' => out.push('+'),
            _ => out.push_str(&format!("%{:02X}", b)),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    // Robust — the merged table contains both surfaces and dedups aliases
    // by name: a view command, an engine command, and no duplicate names.
    #[test]
    fn merged_table_covers_both_surfaces_robust() {
        let table = slash_commands_table();
        assert!(table.iter().any(|(n, _)| *n == "/theme"), "view command");
        assert!(
            table.iter().any(|(n, _)| *n == "/compact"),
            "engine command"
        );
        let mut seen = std::collections::HashSet::new();
        for (n, _) in table {
            assert!(
                seen.insert(*n),
                "duplicate command name in merged table: {n}"
            );
        }
    }
}
