//! `dsh` binary.

use dsh_cli::run_cli;

#[tokio::main]
async fn main() {
    let code = run_cli(std::env::args().collect()).await;
    std::process::exit(code);
}
