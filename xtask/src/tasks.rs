//! What each subcommand actually does.
//!
//! Thin wrappers over the documented workflows: build (debug or release),
//! launch, check, lint, format, test, doctor, clean. The
//! only logic worth reading here is path resolution and exit-code propagation —
//! everything else is a command assembled and handed to [`crate::proc`].

use std::process::{Command as Process, ExitCode, ExitStatus};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{bail, Result};

use crate::cli::{BuildArgs, Cli, ClippyArgs, Command, FmtArgs, PlanArgs, RunArgs};
use crate::proc;
use crate::workspace::{Profile, Workspace, APP_PACKAGE};

/// Dispatch a parsed command line.
pub fn dispatch(cli: Cli) -> Result<ExitCode> {
    let ws = Workspace::discover()?;
    match cli.command {
        Command::Build(args) => build(&ws, args),
        Command::Run(args) => run(&ws, args),
        Command::Plan(args) => plan(&ws, args),
        Command::Check => check(&ws),
        Command::Clippy(args) => clippy(&ws, args),
        Command::Fmt(args) => fmt(&ws, args),
        Command::Test => test(&ws),
        Command::Doctor => doctor(&ws),
        Command::Clean => clean(&ws),
    }
}

fn build(ws: &Workspace, args: BuildArgs) -> Result<ExitCode> {
    let profile = args.profile.resolve(false);
    build_app(ws, profile, &args.passthrough)?;
    let binary = ws.app_binary(profile)?;
    proc::done(format!("built {} ({})", binary.display(), profile.label()));
    Ok(ExitCode::SUCCESS)
}

fn run(ws: &Workspace, args: RunArgs) -> Result<ExitCode> {
    launch_app(ws, args.profile.resolve(false), args.app_args, args.log)
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

fn doctor(ws: &Workspace) -> Result<ExitCode> {
    // Tools that must be present for the documented workflows to run at all.
    proc::step("toolchain");
    let tools = [("cargo", "--version", true), ("rustc", "--version", true)];
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
    Ok(ExitCode::SUCCESS)
}

/// Build the desktop app with cargo.
fn build_app(ws: &Workspace, profile: Profile, extra: &[String]) -> Result<()> {
    let mut cmd = Process::new("cargo");
    cmd.current_dir(ws.root())
        .args(["build", "-p", APP_PACKAGE]);
    if profile.is_release() {
        cmd.arg("--release");
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
    build_app(ws, profile, &[])?;
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

fn exit_code(status: ExitStatus) -> ExitCode {
    ExitCode::from(status.code().unwrap_or(1).clamp(0, 255) as u8)
}

fn unix_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or(0)
}
