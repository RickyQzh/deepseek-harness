//! `dsh` clap launcher.

mod parse;
mod run;

#[cfg(test)]
mod phase5_exit;

pub use parse::{CliError, HeadlessLaunch, ParsedCli, parse_cli};
pub use run::run_cli;
