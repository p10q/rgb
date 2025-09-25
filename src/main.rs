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

    // Check if we should log to file (for debugging without interfering with TUI)
    if std::env::var("RGB_LOG_FILE").is_ok() {
        let log_file = std::fs::File::create("rgb_debug.log").expect("Failed to create log file");
        tracing_subscriber::fmt()
            .with_writer(log_file)
            .with_ansi(false)
            .with_env_filter(filter)
            .init();

        // Log that we're using file logging
        tracing::info!("RGB starting with file logging to rgb_debug.log");
    } else {
        tracing_subscriber::fmt()
            .with_env_filter(filter)
            .init();
    }

    // Load configuration
    let config = config::load_config(args.config)?;

    // Determine project directory
    let project_dir = args.directory
        .or_else(|| std::env::current_dir().ok())
        .unwrap_or_else(|| PathBuf::from("."));

    // Create and run the application
    match app::RgbApp::new(config, project_dir) {
        Ok(mut app) => {
            // If execute command is provided, create initial terminal with it
            if let Some(cmd) = args.execute {
                app.create_terminal_with_command(&cmd).await?;
            }

            app.run().await?;
        }
        Err(e) => {
            eprintln!("Failed to initialize RGB: {}", e);
            eprintln!("\nCommon issues:");
            eprintln!("- Make sure you're running in a real terminal (not in an IDE terminal)");
            eprintln!("- Try running with: TERM=xterm-256color ./target/debug/rgb");
            eprintln!("- On macOS, you may need to run in Terminal.app or iTerm2");
            return Err(e);
        }
    }

    Ok(())
}
