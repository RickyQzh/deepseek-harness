//! `dsh` clap launcher.

mod parse;
mod run;

#[cfg(test)]
mod phase5_exit;

#[cfg(test)]
mod phase7_exit;

pub use parse::{CliError, HeadlessLaunch, ParsedCli, WebLaunch, parse_cli};
pub use run::run_cli;
