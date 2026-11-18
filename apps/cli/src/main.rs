//! Storyfella's story tooling and project-scoped MCP server.

use std::io;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};

mod mcp;
mod story;

#[derive(Parser)]
#[command(name = "sf-cli", version, about = "Storyfella story and project tools")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Run a project-scoped MCP server over standard input and output.
    Mcp(mcp::McpArgs),
    /// Check, compile, or trace a Storyfella story.
    Story {
        #[command(subcommand)]
        story: story::StoryCommand,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let logging = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .compact();
    if matches!(cli.command, Command::Mcp(_)) {
        logging.with_writer(io::stderr).init();
    } else {
        logging.init();
    }
    let runtime = tokio::runtime::Runtime::new().context("failed to start the async runtime")?;
    runtime.block_on(async {
        match cli.command {
            Command::Mcp(args) => mcp::run(args).await,
            Command::Story { story } => story::run(story),
        }
    })
}
