use anyhow::{Context, Result};
use clap::Parser;
use clipforge_inference::ModelStore;
use storyfella_studio_core::config::AppConfig;

#[derive(Debug, Parser)]
#[command(about = "Download and verify the Qwen-Image-2.1 model artifacts")]
struct Arguments {}

#[tokio::main]
async fn main() -> Result<()> {
    Arguments::parse();
    let deployment = AppConfig::load()
        .context("failed to load the configuration")?
        .image;

    // The provider names the model tree; artifacts it overrides with a local
    // file are never downloaded.
    let store = ModelStore::new(deployment.model_root());
    let mut artifacts = Vec::new();
    for entry in deployment.artifact_plan() {
        match entry.local_override {
            Some(path) => println!(
                "{}: using local override {}",
                entry.artifact.filename,
                path.display()
            ),
            None => artifacts.push(entry.artifact),
        }
    }

    let mut previous = (String::new(), u64::MAX);
    store
        .ensure_all(&artifacts, |progress| {
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
