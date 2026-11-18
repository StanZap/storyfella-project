//! The command line: one `clap` subcommand per task.
//!
//! Flag parsing stays here; the work each subcommand does lives in
//! [`crate::tasks`]. Shared argument groups (the build profile) are `flatten`ed
//! into the commands that need them.

use std::path::PathBuf;

use clap::{Args, Parser, Subcommand, ValueEnum};

use crate::workspace::Profile;

#[derive(Parser)]
#[command(
    name = "xtask",
    version,
    about = "Development tasks for the Storyfella workspace",
    long_about = "Wraps the project's build, run, check and test workflows \
                  so the exact tool invocations live in one place. Run it as \
                  `cargo xtask <command>` (the alias is in .cargo/config.toml).",
    after_help = "Examples:\n  \
        cargo xtask run                          build (debug) and launch Studio\n  \
        cargo xtask run -r                       build and launch a release binary\n  \
        cargo xtask run -l                       …logging the app to .svs-e2e/logs/\n  \
        cargo xtask plan config/plans/smoke.yaml\n  \
        cargo xtask plan config/plans/ui-tour.yaml --mode guide\n  \
        cargo xtask test"
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand)]
pub enum Command {
    /// Build Studio (no launch).
    Build(BuildArgs),
    /// Build Studio, then launch it.
    Run(RunArgs),
    /// Build, then play a YAML plan through the app (`--plan <file> --mode …`).
    Plan(PlanArgs),
    /// Type-check the whole workspace (`cargo check --workspace --all-targets`).
    Check,
    /// Lint the whole workspace (`cargo clippy`); `--deny` turns warnings into errors.
    Clippy(ClippyArgs),
    /// Format the workspace (`cargo fmt --all`).
    Fmt(FmtArgs),
    /// Run the workspace test suite (`cargo test --workspace`).
    Test,
    /// Report prerequisite tools and provisioned directories.
    Doctor,
    /// Remove build output (the cargo target directory).
    Clean,
}

/// The debug/release profile, shared by every command that builds.
///
/// `-r`/`--release` selects release and `-d`/`--debug` selects debug; the two
/// conflict, so a typo cannot silently pick one. When neither is given, the
/// command's own default applies.
#[derive(Args, Clone, Copy, Debug, Default)]
pub struct ProfileArgs {
    /// Build in release mode.
    #[arg(short = 'r', long, conflicts_with = "debug")]
    release: bool,
    /// Build in debug mode.
    #[arg(short = 'd', long, conflicts_with = "release")]
    debug: bool,
}

impl ProfileArgs {
    /// Resolve to a concrete profile, using `default_release` only when neither
    /// flag was given.
    pub fn resolve(self, default_release: bool) -> Profile {
        if self.release || (!self.debug && default_release) {
            Profile::Release
        } else {
            Profile::Debug
        }
    }
}

#[derive(Args)]
pub struct BuildArgs {
    #[command(flatten)]
    pub profile: ProfileArgs,
    /// Extra arguments forwarded verbatim to `cargo build`.
    #[arg(last = true, value_name = "CARGO_ARGS")]
    pub passthrough: Vec<String>,
}

#[derive(Args)]
pub struct RunArgs {
    #[command(flatten)]
    pub profile: ProfileArgs,
    /// Tee output to `.svs-e2e/logs/` (named for the launched app).
    #[arg(short = 'l', long)]
    pub log: bool,
    /// Arguments forwarded to the app binary, after `--`.
    #[arg(last = true, value_name = "APP_ARGS")]
    pub app_args: Vec<String>,
}

#[derive(Args)]
pub struct PlanArgs {
    /// Plan file, relative to the workspace root (`config/plans/*.yaml`).
    #[arg(value_name = "PLAN")]
    pub plan: PathBuf,
    /// `check` runs fast and sets the exit code; `guide` narrates in the UI.
    #[arg(long, value_enum, default_value_t = PlanMode::Check)]
    pub mode: PlanMode,
    #[command(flatten)]
    pub profile: ProfileArgs,
    /// Tee the app's stdout/stderr to `.svs-e2e/logs/app-<unix>.log`.
    #[arg(short = 'l', long)]
    pub log: bool,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
pub enum PlanMode {
    /// Narrated walkthrough shown in the UI.
    Guide,
    /// Fast, backend-free regression run; non-zero exit on failure.
    Check,
}

impl PlanMode {
    /// The value the app's `PlanLaunch` parser expects (`--mode <value>`).
    pub fn as_arg(self) -> &'static str {
        match self {
            PlanMode::Guide => "guide",
            PlanMode::Check => "check",
        }
    }
}

#[derive(Args)]
pub struct ClippyArgs {
    /// Deny warnings (`cargo clippy … -- -D warnings`).
    #[arg(long)]
    pub deny: bool,
}

#[derive(Args)]
pub struct FmtArgs {
    /// Check formatting without rewriting files (`cargo fmt -- --check`).
    #[arg(long)]
    pub check: bool,
}
