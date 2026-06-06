use crate::app::App;
use crate::types::ChatMessage;

/// Dispatch the `/worktree ...` subcommands. Argument string is the slice after
/// `/worktree `: empty / `"list"` lists, `"create <name>"` creates,
/// `"remove <name>"` removes, `"switch <name>"` prints the manual cd hint.
///
/// `App.cwd` is fixed at startup, so `switch` cannot teleport the running
/// session into a different checkout. It tells the user how to do it manually.
pub(super) async fn handle_worktree_command(app: &mut App, args: &str) {
    let mut it = args.split_whitespace();
    let sub = it.next().unwrap_or("");
    let arg = it.next().unwrap_or("");
    let repo_root = std::path::PathBuf::from(&app.cwd);

    fn echo(app: &mut App, raw: String, body: String) {
        app.messages.push(ChatMessage::user(raw));
        app.messages.push(ChatMessage::assistant(body));
    }

    async fn list_body(cwd: &str) -> String {
        match crate::worktrees::list_worktrees_async(&std::path::PathBuf::from(cwd)).await {
            Ok(rows) if rows.is_empty() => "No worktrees registered.".to_owned(),
            Ok(rows) => {
                let mut s = format!("**{} worktree(s):**\n\n", rows.len());
                for w in &rows {
                    let branch = if w.branch.is_empty() {
                        "(none)"
                    } else {
                        w.branch.as_str()
                    };
                    s.push_str(&format!("- `{}` — branch `{}`\n", w.path, branch));
                }
                s
            }
            Err(e) => format!("**Error:** {e}"),
        }
    }

    match sub {
        "" | "list" => {
            let body = list_body(&app.cwd).await;
            echo(app, "/worktree list".to_owned(), body);
        }
        "create" => {
            if arg.is_empty() {
                echo(
                    app,
                    "/worktree create".to_owned(),
                    "Usage: `/worktree create <name>` (alphanumeric, dash, underscore)".to_owned(),
                );
                return;
            }
            if let Err(e) = crate::worktrees::validate_name(arg) {
                echo(
                    app,
                    format!("/worktree create {arg}"),
                    format!("**Error:** {e}"),
                );
                return;
            }
            let body = match crate::worktrees::create_worktree_async(&repo_root, arg).await {
                Ok(w) => format!(
                    "Created worktree `{}` on branch `{}`.\n\n\
                     Switch into it with:\n```\ncd {}\n```\nthen re-run `jfc`.",
                    w.path, w.branch, w.path
                ),
                Err(e) => format!("**Error:** {e}"),
            };
            echo(app, format!("/worktree create {arg}"), body);
        }
        "remove" => {
            if arg.is_empty() {
                echo(
                    app,
                    "/worktree remove".to_owned(),
                    "Usage: `/worktree remove <name>` (the `jfc/<name>` branch is preserved)"
                        .to_owned(),
                );
                return;
            }
            if let Err(e) = crate::worktrees::validate_name(arg) {
                echo(
                    app,
                    format!("/worktree remove {arg}"),
                    format!("**Error:** {e}"),
                );
                return;
            }
            let body = match crate::worktrees::remove_worktree_async(&repo_root, arg).await {
                Ok(()) => format!(
                    "Removed worktree `.jfc-worktrees/{arg}`. The branch `jfc/{arg}` is preserved \
                     — recover with `git switch jfc/{arg}` from any checkout."
                ),
                Err(e) => format!("**Error:** {e}"),
            };
            echo(app, format!("/worktree remove {arg}"), body);
        }
        "switch" => {
            if arg.is_empty() {
                echo(
                    app,
                    "/worktree switch".to_owned(),
                    "Usage: `/worktree switch <name>`".to_owned(),
                );
                return;
            }
            if let Err(e) = crate::worktrees::validate_name(arg) {
                echo(
                    app,
                    format!("/worktree switch {arg}"),
                    format!("**Error:** {e}"),
                );
                return;
            }
            let target = std::path::PathBuf::from(&app.cwd)
                .join(".jfc-worktrees")
                .join(arg);
            // jfc's cwd is captured at startup, so we can't transparently
            // teleport mid-session — print the manual recipe.
            let body = format!(
                "To switch into `{name}`, run:\n```\ncd {path}\n```\nthen re-launch `jfc`. \
                 (jfc captures its cwd at startup; live cwd-switch is not yet wired.)",
                name = arg,
                path = target.display()
            );
            echo(app, format!("/worktree switch {arg}"), body);
        }
        other => {
            echo(
                app,
                format!("/worktree {args}"),
                format!(
                    "Unknown subcommand `{other}`. Try `/worktree list|create <name>|remove <name>|switch <name>`."
                ),
            );
        }
    }
}
