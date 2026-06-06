//! `jfc changes` — CLI surface over the agent change-set lifecycle.
//!
//! The headless twin of the `/changes` slash command: list/show/test/apply/
//! revert change-sets from the shell. All operations delegate to
//! `crate::changeset`, which holds the git + store logic, so the two surfaces
//! never drift.

use std::path::PathBuf;

use clap::Subcommand;

#[derive(Subcommand, Debug)]
pub(super) enum ChangesSubcommand {
    /// List every recorded change-set (newest first).
    List,
    /// Show full detail for one change-set.
    Show { id: String },
    /// Run a test command in a change's worktree and record the result.
    Test {
        id: String,
        /// The command to run (everything after `--`).
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        command: Vec<String>,
    },
    /// Merge an Approved change-set's branch into the base.
    Apply { id: String },
    /// Revert a previously applied change-set.
    Revert { id: String },
}

fn cwd() -> PathBuf {
    std::env::current_dir().unwrap_or_default()
}

pub(super) async fn run_changes_subcommand(sub: ChangesSubcommand) -> anyhow::Result<()> {
    let root = cwd();
    let out = match sub {
        ChangesSubcommand::List => crate::changeset::list_changes(&root),
        ChangesSubcommand::Show { id } => crate::changeset::show_change(&root, &id),
        ChangesSubcommand::Test { id, command } => {
            let cmd = command.join(" ");
            if cmd.trim().is_empty() {
                "usage: jfc changes test <id> -- <command>".to_string()
            } else {
                crate::changeset::test_change(&root, &id, &cmd).await
            }
        }
        ChangesSubcommand::Apply { id } => crate::changeset::apply_change(&root, &id).await,
        ChangesSubcommand::Revert { id } => crate::changeset::revert_change(&root, &id).await,
    };
    println!("{out}");
    Ok(())
}
