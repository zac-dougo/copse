mod discovery;
mod tracker;

use clap::Parser;

#[derive(Parser, Debug)]
#[command(
    name = "copse",
    version,
    about = "Terminal board for tracking work across worktrees"
)]
struct Cli {}

fn main() {
    let _cli = Cli::parse();
    // Scaffold: no board logic yet. Printing version proves the binary runs.
    println!("copse {}", env!("CARGO_PKG_VERSION"));
}
