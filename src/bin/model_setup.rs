use std::path::PathBuf;

use anyhow::{bail, Context, Result};
use clap::{Parser, ValueEnum};
use smart_visual_sequencer::{
    app::AppConfig,
    runtime::{KreaQuantization, ModelStore},
};

#[derive(Clone, Copy, Debug, ValueEnum)]
enum ProfileArgument {
    Q2,
    Q4,
}

impl From<ProfileArgument> for KreaQuantization {
    fn from(value: ProfileArgument) -> Self {
        match value {
            ProfileArgument::Q2 => Self::Q2,
            ProfileArgument::Q4 => Self::Q4,
        }
    }
}

#[derive(Debug, Parser)]
#[command(about = "Download and verify native Krea 2 model artifacts")]
struct Arguments {
    #[arg(long, value_enum, default_value = "q2")]
    profile: ProfileArgument,
    #[arg(long)]
    model_dir: Option<PathBuf>,
    #[arg(long)]
    accept_krea_license: bool,
}

#[tokio::main]
async fn main() -> Result<()> {
    let arguments = Arguments::parse();
    if !arguments.accept_krea_license {
        bail!(
            "review https://www.krea.ai/krea-2-licensing and rerun with \
             --accept-krea-license"
        );
    }
    let config = AppConfig::load().context("failed to load application configuration")?;
    let store = ModelStore::new(arguments.model_dir.unwrap_or(config.model_dir));
    let mut previous = (String::new(), u64::MAX);
    store
        .ensure_krea_profile(arguments.profile.into(), |progress| {
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
        .context("failed to provision Krea 2 profile")?;
    println!("model profile is downloaded and checksum-verified");
    Ok(())
}
