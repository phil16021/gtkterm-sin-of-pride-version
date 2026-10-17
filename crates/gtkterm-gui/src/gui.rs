//! GTK user interface (milestone R4; needs the GTK 4, libadwaita and VTE development packages).

use std::process::ExitCode;

use crate::cli::{Cli, Resolved};

/// Start the GUI.
pub fn run(_cli: Cli, _resolved: Resolved) -> ExitCode {
    eprintln!("The GTK user interface is not implemented yet; use --headless or --mcp-stdio.");
    ExitCode::from(2)
}
