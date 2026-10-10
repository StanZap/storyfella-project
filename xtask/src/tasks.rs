//! What each subcommand actually does.
//!
//! Thin wrappers over the documented workflows: build (debug or release),
//! launch, bundle, check, lint, format, test, stylesheet, doctor, clean. The
//! only logic worth reading here is path resolution and exit-code propagation —
//! everything else is a command assembled and handed to [`crate::proc`].

use std::path::Path;
use std::process::{Command as Process, ExitCode, ExitStatus};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{bail, Context, Result};

use crate::cli::{
    BuildArgs, BundleArgs, Cli, ClippyArgs, Command, CssArgs, FmtArgs, PlanArgs, RunArgs, ServeArgs,
};
use crate::proc;
use crate::workspace::{Profile, Workspace};

/// Dispatch a parsed command line.
pub fn dispatch(cli: Cli) -> Result<ExitCode> {
    let ws = Workspace::discover()?;
    match cli.command {
        Command::Build(args) => build(&ws, args),
        Command::Run(args) => run(&ws, args),
        Command::Atelier(args) => atelier(&ws, args),
        Command::Plan(args) => plan(&ws, args),
        Command::Serve(args) => serve(&ws, args),
        Command::Bundle(args) => bundle(&ws, args),
        Command::Check => check(&ws),
        Command::Clippy(args) => clippy(&ws, args),
        Command::Fmt(args) => fmt(&ws, args),
        Command::Test => test(&ws),
        Command::Css(args) => css(&ws, args),
        Command::Doctor => doctor(&ws),
        Command::Clean => clean(&ws),
    }
}

fn build(ws: &Workspace, args: BuildArgs) -> Result<ExitCode> {
    let profile = args.profile.resolve(false);
    dx_build(ws, profile, &args.passthrough)?;
    let binary = ws.app_binary(profile)?;
    proc::done(format!("built {} ({})", binary.display(), profile.label()));
    Ok(ExitCode::SUCCESS)
}

fn run(ws: &Workspace, args: RunArgs) -> Result<ExitCode> {
    launch_app(ws, args.profile.resolve(false), args.app_args, args.log)
}

fn atelier(ws: &Workspace, args: RunArgs) -> Result<ExitCode> {
    let mut cmd = Process::new("cargo");
    cmd.current_dir(ws.root())
        .args(["run", "--package", "storyfella-atelier", "--bin", "atelier"]);
    if args.profile.resolve(false).is_release() {
        cmd.arg("--release");
    }
    cmd.arg("--").args(args.app_args);
    launch_with_log(ws, &mut cmd, args.log, "atelier")
}

fn plan(ws: &Workspace, args: PlanArgs) -> Result<ExitCode> {
    let app_args = vec![
        "--plan".to_string(),
        args.plan.to_string_lossy().into_owned(),
        "--mode".to_string(),
        args.mode.as_arg().to_string(),
    ];
    launch_app(ws, args.profile.resolve(false), app_args, args.log)
}

fn serve(ws: &Workspace, args: ServeArgs) -> Result<ExitCode> {
    let profile = args.profile.resolve(false);
    let mut cmd = Process::new("dx");
    cmd.current_dir(ws.studio())
        .args(["serve", "--platform", "desktop"]);
    if profile.is_release() {
        cmd.arg("-r");
    }
    cmd.args(&args.passthrough);
    proc::require(&mut cmd)?;
    Ok(ExitCode::SUCCESS)
}

fn bundle(ws: &Workspace, args: BundleArgs) -> Result<ExitCode> {
    // Shipping a debug bundle is almost never intended, so release is the
    // default here; `-d` still overrides it.
    let profile = args.profile.resolve(true);
    let out_dir = args
        .out_dir
        .as_deref()
        .map(|dir| ws.resolve(dir))
        .unwrap_or_else(|| ws.dist());
    std::fs::create_dir_all(&out_dir).with_context(|| format!("creating {}", out_dir.display()))?;

    let mut cmd = Process::new("dx");
    cmd.current_dir(ws.studio())
        .args(["bundle", "--platform", "desktop"]);
    for package_type in &args.package_types {
        cmd.args(["--package-types", package_type.as_arg()]);
    }
    cmd.arg("--out-dir").arg(&out_dir);
    if profile.is_release() {
        cmd.arg("-r");
    }
    proc::require(&mut cmd)?;

    if let Ok(entries) = std::fs::read_dir(&out_dir) {
        let mut names: Vec<String> = entries
            .flatten()
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        for name in names {
            proc::done(format!("{}/{}", out_dir.display(), name));
        }
    }
    Ok(ExitCode::SUCCESS)
}

fn check(ws: &Workspace) -> Result<ExitCode> {
    cargo(ws, &["check", "--workspace", "--all-targets"])?;
    Ok(ExitCode::SUCCESS)
}

fn clippy(ws: &Workspace, args: ClippyArgs) -> Result<ExitCode> {
    let mut cmd = Process::new("cargo");
    cmd.current_dir(ws.root())
        .args(["clippy", "--workspace", "--all-targets"]);
    if args.deny {
        cmd.args(["--", "-D", "warnings"]);
    }
    proc::require(&mut cmd)?;
    Ok(ExitCode::SUCCESS)
}

fn fmt(ws: &Workspace, args: FmtArgs) -> Result<ExitCode> {
    let mut cmd = Process::new("cargo");
    cmd.current_dir(ws.root()).args(["fmt", "--all"]);
    if args.check {
        cmd.args(["--", "--check"]);
    }
    proc::require(&mut cmd)?;
    Ok(ExitCode::SUCCESS)
}

fn test(ws: &Workspace) -> Result<ExitCode> {
    cargo(ws, &["test", "--workspace"])?;
    Ok(ExitCode::SUCCESS)
}

fn css(ws: &Workspace, args: CssArgs) -> Result<ExitCode> {
    let studio = ws.studio();
    if args.install {
        npm(&studio, &["install"])?;
    } else if !studio.join("node_modules").is_dir() {
        bail!(
            "frontend toolchain missing at {} — run `cargo xtask css --install`",
            studio.join("node_modules").display()
        );
    }
    npm(&studio, &["run", "css:build"])?;
    Ok(ExitCode::SUCCESS)
}

fn doctor(ws: &Workspace) -> Result<ExitCode> {
    // Tools that must be present for the documented workflows to run at all.
    proc::step("toolchain");
    let tools = [
        ("cargo", "--version", true),
        ("rustc", "--version", true),
        ("dx", "--version", true),
        ("node", "--version", false),
        ("npm", "--version", false),
    ];
    let mut missing_required = Vec::new();
    for (program, version_arg, required) in tools {
        match proc::version(program, version_arg) {
            Some(version) => println!("  ok    {program:<6} {version}"),
            None if required => {
                println!("  MISS  {program:<6} not found (required)");
                missing_required.push(program);
            }
            None => println!("  warn  {program:<6} not found"),
        }
    }

    proc::step("provisioned paths");
    let paths = [
        (
            ws.root().join(".cargo/config.toml"),
            true,
            "target-dir redirect",
        ),
        (
            ws.root().join("config/plans"),
            true,
            "plan fixtures + project-root marker",
        ),
        (
            ws.studio().join("node_modules"),
            false,
            "npm install (from apps/studio)",
        ),
        (
            ws.studio().join("assets/tailwind.css"),
            false,
            "cargo xtask css",
        ),
    ];
    for (path, required, hint) in paths {
        let present = path.exists();
        let state = match (present, required) {
            (true, _) => "ok",
            (false, true) => "MISS",
            (false, false) => "warn",
        };
        if !present {
            println!("  {state:4}  {}  ({hint})", path.display());
        } else {
            println!("  {state:4}  {}", path.display());
        }
    }

    if missing_required.is_empty() {
        Ok(ExitCode::SUCCESS)
    } else {
        bail!("missing required tools: {}", missing_required.join(", "))
    }
}

fn clean(ws: &Workspace) -> Result<ExitCode> {
    cargo(ws, &["clean"])?;
    let dist = ws.dist();
    if dist.is_dir() {
        proc::step(format!("removing {}", dist.display()));
        std::fs::remove_dir_all(&dist).with_context(|| format!("removing {}", dist.display()))?;
    }
    Ok(ExitCode::SUCCESS)
}

/// Build the desktop app with `dx` (never plain `cargo`, which skips the asset
/// pipeline and produces an unstyled binary — see `AGENTS.md`).
fn dx_build(ws: &Workspace, profile: Profile, extra: &[String]) -> Result<()> {
    let mut cmd = Process::new("dx");
    cmd.current_dir(ws.studio())
        .args(["build", "--platform", "desktop"]);
    if profile.is_release() {
        cmd.arg("-r");
    }
    cmd.args(extra);
    proc::require(&mut cmd)
}

/// Build, then run the app from the workspace root so its project-root path
/// resolution finds `config/plans`, propagating the app's exit code.
fn launch_app(
    ws: &Workspace,
    profile: Profile,
    app_args: Vec<String>,
    log: bool,
) -> Result<ExitCode> {
    dx_build(ws, profile, &[])?;
    let binary = ws.app_binary(profile)?;
    let mut cmd = Process::new(&binary);
    cmd.args(&app_args).current_dir(ws.root());
    proc::done(format!("launching ({})", profile.label()));
    launch_with_log(ws, &mut cmd, log, "app")
}

fn launch_with_log(ws: &Workspace, cmd: &mut Process, log: bool, prefix: &str) -> Result<ExitCode> {
    let log_path = log.then(|| {
        ws.e2e_logs()
            .join(format!("{prefix}-{}.log", unix_seconds()))
    });
    if let Some(path) = &log_path {
        proc::done(format!("logging to {}", path.display()));
    }
    let status = proc::launch(cmd, log_path.as_deref())?;
    Ok(exit_code(status))
}

fn cargo(ws: &Workspace, args: &[&str]) -> Result<()> {
    let mut cmd = Process::new("cargo");
    cmd.current_dir(ws.root()).args(args);
    proc::require(&mut cmd)
}

fn npm(dir: &Path, args: &[&str]) -> Result<()> {
    let mut cmd = Process::new("npm");
    cmd.current_dir(dir).args(args);
    proc::require(&mut cmd)
}

fn exit_code(status: ExitStatus) -> ExitCode {
    ExitCode::from(status.code().unwrap_or(1).clamp(0, 255) as u8)
}

fn unix_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or(0)
}
