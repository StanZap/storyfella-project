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
    about = "Development tasks for Storyfella Studio",
    long_about = "Wraps the project's build, run, bundle, check and test workflows \
                  so the exact tool invocations live in one place. Run it as \
                  `cargo xtask <command>` (the alias is in .cargo/config.toml).",
    after_help = "Examples:\n  \
        cargo xtask run                          build (debug) and launch the app\n  \
        cargo xtask run -r                       build and launch a release binary\n  \
        cargo xtask run -l                       …logging the app to .svs-e2e/logs/\n  \
        cargo xtask plan config/plans/smoke.yaml\n  \
        cargo xtask plan config/plans/tour.yaml --mode guide\n  \
        cargo xtask bundle --package-types appimage deb\n  \
        cargo xtask test"
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand)]
pub enum Command {
    /// Build the desktop app with `dx` (no launch).
    Build(BuildArgs),
    /// Build the desktop app, then launch it.
    Run(RunArgs),
    /// Build, then play a YAML plan through the app (`--plan <file> --mode …`).
    Plan(PlanArgs),
    /// Serve the desktop app with hot reload (`dx serve`).
    Serve(ServeArgs),
    /// Bundle distributable artifacts with `dx bundle`.
    Bundle(BundleArgs),
    /// Type-check the whole workspace (`cargo check --workspace --all-targets`).
    Check,
    /// Lint the whole workspace (`cargo clippy`); `--deny` turns warnings into errors.
    Clippy(ClippyArgs),
    /// Format the workspace (`cargo fmt --all`).
    Fmt(FmtArgs),
    /// Run the workspace test suite (`cargo test --workspace`).
    Test,
    /// Rebuild the Tailwind stylesheet (`npm run css:build`).
    Css(CssArgs),
    /// Report prerequisite tools and provisioned directories.
    Doctor,
    /// Remove build output (cargo target directory and `dist/`).
    Clean,
}

/// The debug/release profile, shared by every command that builds.
///
/// `-r`/`--release` selects release and `-d`/`--debug` selects debug; the two
/// conflict, so a typo cannot silently pick one. When neither is given, the
/// command's own default applies: debug for `build`/`run`/`serve`, release for
/// `bundle`.
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
    /// Extra arguments forwarded verbatim to `dx build`.
    #[arg(last = true, value_name = "DX_ARGS")]
    pub passthrough: Vec<String>,
}

#[derive(Args)]
pub struct RunArgs {
    #[command(flatten)]
    pub profile: ProfileArgs,
    /// Tee the app's stdout/stderr to `.svs-e2e/logs/app-<unix>.log`.
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
pub struct ServeArgs {
    #[command(flatten)]
    pub profile: ProfileArgs,
    /// Extra arguments forwarded verbatim to `dx serve`.
    #[arg(last = true, value_name = "DX_ARGS")]
    pub passthrough: Vec<String>,
}

#[derive(Args)]
pub struct BundleArgs {
    #[command(flatten)]
    pub profile: ProfileArgs,
    /// Bundle formats to produce. Repeatable, or several values after one flag.
    #[arg(
        long,
        value_enum,
        num_args = 1..,
        default_values_t = [PackageType::Appimage],
        value_name = "TYPE"
    )]
    pub package_types: Vec<PackageType>,
    /// Where to place the artifacts (default `dist/` at the workspace root).
    #[arg(long, value_name = "DIR")]
    pub out_dir: Option<PathBuf>,
}

/// The `dx bundle --package-types` values. Only the ones a desktop app can use
/// are named; `dx config schema` documents the rest.
#[derive(Clone, Copy, Debug, ValueEnum)]
pub enum PackageType {
    /// Linux AppImage (the default, and the only self-contained one).
    Appimage,
    /// Linux Debian package.
    Deb,
    /// Linux RPM package.
    Rpm,
    /// macOS `.app` bundle.
    Macos,
    /// macOS `.dmg`.
    Dmg,
    /// Windows `.msi`.
    Msi,
    /// Windows NSIS installer.
    Nsis,
}

impl PackageType {
    pub fn as_arg(self) -> &'static str {
        match self {
            PackageType::Appimage => "appimage",
            PackageType::Deb => "deb",
            PackageType::Rpm => "rpm",
            PackageType::Macos => "macos",
            PackageType::Dmg => "dmg",
            PackageType::Msi => "msi",
            PackageType::Nsis => "nsis",
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

#[derive(Args)]
pub struct CssArgs {
    /// Install the frontend toolchain (`npm install`) before building.
    #[arg(long)]
    pub install: bool,
}
