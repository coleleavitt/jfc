//! `jfc doctor` — diagnose filesystem paths for XDG compatibility.
//!
//! Non-destructive: reports where legacy, non-XDG paths are in use and
//! recommends modern locations. The CLI does not move or delete any data.

use clap::Subcommand;
use jfc_config::paths::XdgPathDiagnosticService;

#[derive(Subcommand, Debug)]
pub(super) enum DoctorSubcommand {
    /// Scan common jfc paths and report legacy locations with suggestions.
    Paths,
}

pub(super) async fn run_doctor_subcommand(sub: DoctorSubcommand) -> anyhow::Result<()> {
    match sub {
        DoctorSubcommand::Paths => paths_diagnostic(),
    }
}

fn paths_diagnostic() -> anyhow::Result<()> {
    print!("{}", XdgPathDiagnosticService::default().report());
    Ok(())
}
