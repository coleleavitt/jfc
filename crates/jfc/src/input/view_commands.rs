//! View-only slash commands: anything whose behavior is frontend state
//! (modal editing, help overlay, theme picker, clipboard, panels, the
//! remote-control sidecar). Engine-resident command semantics live in
//! `jfc_engine::commands` — see the registry in `slash_commands.rs` for how
//! the two tables merge.

use tokio::sync::mpsc;

use crate::app::App;
use crate::runtime::EngineEvent;
use jfc_core::*;

use super::theme_picker::{apply_theme, open_theme_picker};

/// `/vim` — toggle modal (vim) editing of the prompt. On enable you start in
/// Normal mode; Esc returns to Normal from Insert/Visual.
pub(super) async fn cmd_vim(
    app: &mut App,
    _parts: &[&str],
    _text: &str,
    _tx: Option<&mpsc::Sender<EngineEvent>>,
) {
    let now_on = app.vim.is_none();
    app.vim = if now_on {
        Some(crate::input::vim::VimState::default())
    } else {
        None
    };
    jfc_engine::toast::push_with_cap(
        &mut app.engine.toasts,
        jfc_engine::toast::Toast::new(
            jfc_engine::toast::ToastKind::Info,
            if now_on {
                "vim mode on — Normal mode (i to insert, Esc to return)".to_string()
            } else {
                "vim mode off".to_string()
            },
        ),
    );
}



pub(super) async fn cmd_help(
    app: &mut App,
    _parts: &[&str],
    _text: &str,
    _tx: Option<&mpsc::Sender<EngineEvent>>,
) {
    // Also flip the visual overlay so users get the same
    // keybindings table they'd see from `?`. The text dump
    // below is kept for searchability + transcript export.
    app.show_help = true;
    app.engine.messages.push(ChatMessage::user("/help".into()));

    // Command list is rendered from the unified CommandSpec metadata layer
    // (`command_spec::slash_help_lines`), which reads the SLASH_COMMANDS
    // registry — the same single source that drives dispatch and autocomplete —
    // so /help can never list a command that doesn't exist (or miss one), and
    // it stays in lock-step with `/commands`. Aliases collapse onto their
    // canonical row's help text.
    let mut body = String::from("**Available commands:**\n");
    body.push_str(&jfc_engine::command_spec::slash_help_lines());
    body.push_str(
        "\n\
         **Keys:**\n\
         - Ctrl+B — Toggle sessions sidebar\n\
         - Ctrl+M — Model picker\n\
         - Ctrl+P — Command palette\n\
         - Ctrl+O — Expand reasoning / open diagnostic panel\n\
         - Alt+. / Alt+, — Raise / lower reasoning effort\n\
         - Ctrl+Y — Yank last assistant message to clipboard\n\
         - Ctrl+S — Toggle info sidebar\n\
         - `@` — Autocomplete file paths from cwd\n\
         - Up — Recall most recent queued prompt / cycle history (when input empty)\n\
         - Esc — Dismiss popup / close diagnostic panel\n\
         \n\
         **Env knobs:**\n\
         - `JFC_DISABLE_BELL=1` — silence terminal bell on tool completion\n\
         - `JFC_DISABLE_AUTO_COMPACT=1` — disable auto-compaction\n\
         - `JFC_DISABLE_CARGO_CHECK=1` — skip startup `cargo check`\n\
         - `JFC_AUTOCOMPACT_PCT_OVERRIDE=N` — force compact threshold\n\
         - `JFC_TOOL_TITLE_WIDTH=N` — cap tool title length (default 100)\n\
         - `JFC_ADVISOR_ENABLED=1` — enable the `/advisor` parallel-advice slash command",
    );
    app.engine.messages.push(ChatMessage::assistant(body));
}



pub(super) async fn cmd_verbose(
    app: &mut App,
    parts: &[&str],
    text: &str,
    _tx: Option<&mpsc::Sender<EngineEvent>>,
) {
    // Toggle expanded-by-default tool blocks for the rest of
    // the session. Renderers read `app.verbose_mode` and lift
    // the per-tool preview cap when set.
    app.engine.messages.push(ChatMessage::user(text.to_owned()));
    let arg = parts
        .get(1)
        .copied()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    let target = match arg.as_str() {
        "on" | "true" | "1" => Some(true),
        "off" | "false" | "0" => Some(false),
        "" => Some(!app.verbose_mode),
        _ => None,
    };
    match target {
        Some(v) => {
            app.verbose_mode = v;
            app.engine.messages.push(ChatMessage::assistant(format!(
                "Verbose mode **{}** — tool blocks {} preview cap.",
                if v { "ON" } else { "OFF" },
                if v { "expand past" } else { "respect" },
            )));
        }
        None => {
            app.engine.messages.push(ChatMessage::assistant(
                "Usage: `/verbose [on|off]`. With no arg, toggles.".into(),
            ));
        }
    }
}


/// `/remote-control` (alias `/rc`) — toggle the remote-control WebSocket
/// server on the current session. When enabling, prints the pairing token +
/// connection URL. When already active, prints status; `/rc off` disables it.
pub(super) async fn cmd_remote_control(
    app: &mut App,
    parts: &[&str],
    _text: &str,
    tx: Option<&mpsc::Sender<EngineEvent>>,
) {
    let arg = parts.get(1).copied().unwrap_or("");

    // Disable path.
    if arg.eq_ignore_ascii_case("off") || arg.eq_ignore_ascii_case("stop") {
        match app.remote_host.take() {
            Some(host) => {
                host.shutdown();
                app.engine.messages
                    .push(ChatMessage::assistant("Remote control disabled.".into()));
            }
            None => {
                app.engine.messages.push(ChatMessage::assistant(
                    "Remote control is not active.".into(),
                ));
            }
        }
        return;
    }

    // Status path.
    if arg.eq_ignore_ascii_case("status") {
        let msg = match &app.remote_host {
            Some(host) => format!(
                "Remote control **active** on `ws://{}`\n\
                 Connected clients: {}\n\
                 Token: `{}`",
                host.addr(),
                host.client_count.load(std::sync::atomic::Ordering::Relaxed),
                host.token
            ),
            None => "Remote control is **off**. Run `/remote-control` to enable.".to_string(),
        };
        app.engine.messages.push(ChatMessage::assistant(msg));
        return;
    }

    // Check config-level disable.
    if jfc_engine::config::load_arc()
        .remote_control
        .as_ref()
        .is_some_and(|rc| rc.disabled)
    {
        app.engine.messages.push(ChatMessage::assistant(
            "Remote control is disabled by configuration (`remote_control.disabled = true`)."
                .into(),
        ));
        return;
    }

    // Already active — show status instead of double-starting.
    if app.remote_host.is_some() {
        app.engine.messages.push(ChatMessage::assistant(
            "Remote control is already active. Use `/rc status` or `/rc off`.".into(),
        ));
        return;
    }

    // Enable path. Needs the event-loop tx to inject client input.
    let Some(tx) = tx else {
        app.engine.messages.push(ChatMessage::assistant(
            "Cannot enable remote control without an event channel (internal error).".into(),
        ));
        return;
    };

    let port = jfc_remote::protocol::DEFAULT_PORT;
    match jfc_engine::remote_host::RemoteHost::start(port, tx.clone()).await {
        Ok(host) => {
            let addr = host.addr();
            let token = host.token.clone();
            app.remote_host = Some(host);
            app.engine.messages.push(ChatMessage::assistant(format!(
                "## Remote control enabled\n\n\
                 The session is now mirrored over WebSocket. Connect another \
                 device with:\n\n\
                 ```\n\
                 jfc rc connect ws://{addr} --token {token}\n\
                 ```\n\n\
                 The server is bound to `127.0.0.1` — expose it remotely via:\n\
                 - **Tailscale**: `tailscale serve https+insecure://localhost:{port}`\n\
                 - **SSH tunnel**: `ssh -L {port}:localhost:{port} user@host`\n\
                 - **cloudflared**: `cloudflared tunnel --url http://localhost:{port}`\n\n\
                 Disable anytime with `/rc off`."
            )));
        }
        Err(e) => {
            app.engine.messages.push(ChatMessage::assistant(format!(
                "Failed to start remote control on port {port}: {e}"
            )));
        }
    }
}



pub(super) async fn cmd_copy(
    app: &mut App,
    parts: &[&str],
    text: &str,
    _tx: Option<&mpsc::Sender<EngineEvent>>,
) {
    app.engine.messages.push(ChatMessage::user(text.to_owned()));
    let arg = parts
        .get(1)
        .copied()
        .map(str::trim)
        .filter(|s| !s.is_empty());
    let (payload, scope_label) = match arg {
        None | Some("last") => {
            let body = crate::runtime::last_assistant_text(app).unwrap_or_default();
            (body, "last assistant message".to_owned())
        }
        Some("all") => {
            let body = crate::runtime::full_transcript_text(app);
            (body, "full transcript".to_owned())
        }
        Some(other) => {
            // Numeric tail (`/copy 3` → last 3 messages). On parse
            // failure, fall back to `last` so a typo still copies
            // something useful rather than yielding an error.
            match other.parse::<usize>() {
                Ok(n) if n > 0 => {
                    let body = crate::runtime::tail_transcript_text(app, n);
                    (body, format!("last {n} message(s)"))
                }
                _ => {
                    let body = crate::runtime::last_assistant_text(app).unwrap_or_default();
                    (
                        body,
                        format!("last assistant message (unrecognized arg `{other}`)"),
                    )
                }
            }
        }
    };
    if payload.is_empty() {
        app.engine.messages.push(ChatMessage::assistant(
            "Nothing to copy — the requested scope contains no text.".to_owned(),
        ));
    } else {
        crate::runtime::copy_to_clipboard(&payload, "/copy");
        app.engine.messages.push(ChatMessage::assistant(format!(
                    "Copied {scope_label} ({} chars) to clipboard. OSC 52 escape emitted for SSH/tmux clients.",
                    payload.chars().count()
                )));
    }
}



pub(super) async fn cmd_theme(
    app: &mut App,
    parts: &[&str],
    _text: &str,
    _tx: Option<&mpsc::Sender<EngineEvent>>,
) {
    handle_theme_command(app, parts.get(1).copied().unwrap_or("").trim());
}

/// `/theme [name]` switches the live UI theme or opens the picker.
pub(super) fn handle_theme_command(app: &mut App, args: &str) {
    let name = args.trim();
    if name.is_empty() {
        open_theme_picker(app);
        return;
    }
    match crate::theme::Theme::choice_by_name(name) {
        Some(choice) => apply_theme(app, choice.name),
        None => {
            jfc_engine::toast::push_with_cap(
                &mut app.engine.toasts,
                jfc_engine::toast::Toast::new(
                    jfc_engine::toast::ToastKind::Warning,
                    format!(
                        "unknown theme '{name}' — try one of: {}",
                        crate::theme::Theme::available_names().join(", ")
                    ),
                ),
            );
        }
    }
}
