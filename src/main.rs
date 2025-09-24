mod app;
mod config;
mod git;
mod layout;
mod monitor;
mod terminal;
mod ui;
mod workspace;

use anyhow::Result;
use clap::Parser;
use std::path::PathBuf;
use tracing_subscriber::EnvFilter;

#[derive(Parser, Debug)]
#[command(name = "rgb")]
#[command(about = "Rust Good Vibes - Terminal multiplexer and workspace manager", long_about = None)]
struct Args {
    /// Project directory to open
    #[arg(value_name = "DIR")]
    directory: Option<PathBuf>,

    /// Config file path
    #[arg(short, long, value_name = "FILE")]
    config: Option<PathBuf>,

    /// Enable debug logging
    #[arg(short, long)]
    debug: bool,

    /// Command to execute in new terminal
    #[arg(short = 'e', long)]
    execute: Option<String>,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();

    // Initialize logging
    let filter = if args.debug {
        EnvFilter::new("debug")
    } else {
        EnvFilter::from_default_env()
    };

    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .init();

    // Load configuration
    let config = config::load_config(args.config)?;

    // Determine project directory
    let project_dir = args.directory
        .or_else(|| std::env::current_dir().ok())
        .unwrap_or_else(|| PathBuf::from("."));

    // Create and run the application
    let mut app = app::RgbApp::new(config, project_dir)?;

    // If execute command is provided, create initial terminal with it
    if let Some(cmd) = args.execute {
        app.create_terminal_with_command(&cmd).await?;
    }

    app.run().await?;

    Ok(())
}
