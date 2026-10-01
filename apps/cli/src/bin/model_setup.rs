use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::Parser;
use svs_core::{
    config::AppConfig,
    runtime::{generation_profile, ModelStore},
};

#[derive(Debug, Parser)]
#[command(about = "Download and verify the Qwen-Image-2.1 model artifacts")]
struct Arguments {
    #[arg(long)]
    model_dir: Option<PathBuf>,
}

#[tokio::main]
async fn main() -> Result<()> {
    let arguments = Arguments::parse();
    let config = AppConfig::load().context("failed to load application configuration")?;
    let store = ModelStore::new(arguments.model_dir.unwrap_or(config.model_dir));
    let mut previous = (String::new(), u64::MAX);
    store
        .ensure_profile(generation_profile(), |progress| {
            let percent = progress
                .downloaded_bytes
                .saturating_mul(100)
                .checked_div(progress.total_bytes)
                .unwrap_or(0)
                .min(100);
            if previous.0 != progress.filename || previous.1 != percent {
                println!("{}: {percent}%", progress.filename);
                previous = (progress.filename, percent);
            }
        })
        .await
        .context("failed to provision the Qwen-Image-2.1 profile")?;
    println!("model profile is downloaded and checksum-verified");
    Ok(())
}
