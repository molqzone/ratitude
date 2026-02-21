mod cli;
mod console;
mod daemon;
mod output_manager;
mod source_scan;
mod sync_controller;

use anyhow::Result;
use tracing::error;
use tracing_subscriber::EnvFilter;

use crate::cli::parse_cli;
use crate::daemon::run_daemon;

#[tokio::main(flavor = "multi_thread")]
async fn main() {
    init_tracing();
    let exit = run().await;
    if let Err(err) = exit {
        error!(error = %err, "rttd exited with error");
        eprintln!("{err:#}");
        std::process::exit(1);
    }
}

async fn run() -> Result<()> {
    let cli = parse_cli()?;
    run_daemon(cli).await
}

fn init_tracing() {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(true)
        .try_init();
}
