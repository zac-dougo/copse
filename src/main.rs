mod app;
mod discovery;
mod forest;
mod herdr;
mod map;
mod tracker;

use clap::Parser;

#[derive(Parser, Debug)]
#[command(
    name = "copse",
    version,
    about = "Terminal board for tracking work across worktrees"
)]
struct Cli {}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let _cli = Cli::parse();
    let cwd = std::env::current_dir()?;
    if let Err(e) = app::run(cwd).await {
        eprintln!("copse: {e}");
        std::process::exit(1);
    }
    Ok(())
}
