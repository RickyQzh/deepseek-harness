//! `dsh` clap launcher.

mod parse;
mod run;

pub use parse::{CliError, HeadlessLaunch, ParsedCli, parse_cli};
pub use run::run_cli;
