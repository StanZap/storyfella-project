//! What each subcommand actually does.
//!
//! Thin wrappers over the documented workflows: build (debug or release),
//! launch, bundle, check, lint, format, test, stylesheet, doctor, clean. The
//! only logic worth reading here is path resolution and exit-code propagation —
//! everything else is a command assembled and handed to [`crate::proc`].

use std::fs::{self, OpenOptions};
use std::path::{Path, PathBuf};
use std::process::{Command as Process, ExitCode, ExitStatus};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{bail, Context, Result};

use crate::cli::{
    AtelierArgs, AtelierBundleArgs, AtelierCommand, AtelierPackageType, BuildArgs, BundleArgs, Cli,
    ClippyArgs, Command, CssArgs, FmtArgs, PlanArgs, RunArgs, ServeArgs,
};
use crate::install;
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

fn atelier(ws: &Workspace, args: AtelierArgs) -> Result<ExitCode> {
    if let Some(AtelierCommand::Bundle(bundle_args)) = args.command {
        return bundle_atelier(ws, bundle_args);
    }
    let restart = args.restart;
    let args = args.run;
    if args.app_args.first().is_some_and(|arg| arg == "--help") && restart {
        bail!("--restart cannot be combined with Atelier's --help");
    }
    if restart {
        let data_dir = if args.app_args.first().is_some_and(|arg| arg == "--data-dir") {
            ws.resolve(Path::new(
                args.app_args
                    .get(1)
                    .context("--data-dir requires a directory")?,
            ))
        } else {
            storyfella_studio_core::atelier::data_dir()
        };
        let mut build = Process::new("cargo");
        build.current_dir(ws.root()).args([
            "build",
            "--package",
            "storyfella-atelier",
            "--bin",
            "atelier",
        ]);
        if args.profile.resolve(false).is_release() {
            build.arg("--release");
        }
        proc::require(&mut build)?;
        restart_atelier(&data_dir)?;
    }
    let mut cmd = Process::new("cargo");
    cmd.current_dir(ws.root())
        .args(["run", "--package", "storyfella-atelier", "--bin", "atelier"]);
    if args.profile.resolve(false).is_release() {
        cmd.arg("--release");
    }
    cmd.arg("--").args(args.app_args);
    launch_with_log(ws, &mut cmd, args.log, "atelier")
}

fn bundle_atelier(ws: &Workspace, args: AtelierBundleArgs) -> Result<ExitCode> {
    use AtelierPackageType::{Appimage, Macos};

    let linux = match std::env::consts::OS {
        "macos" => {
            if args.package_types.iter().any(|format| *format != Macos) {
                bail!("Atelier can only produce the macos package type on macOS");
            }
            false
        }
        "linux" => {
            if args.package_types.contains(&Macos) {
                bail!("Atelier can only produce appimage and deb package types on Linux");
            }
            true
        }
        _ => bail!("Atelier bundling requires a macOS or Linux host"),
    };

    let out_dir = ws.dist().join("atelier");
    fs::create_dir_all(&out_dir).with_context(|| format!("creating {}", out_dir.display()))?;
    // Packaging writes to shared paths, including the cache of AppImage tools.
    let lock_path = out_dir.join(".bundle.lock");
    let lock = OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(&lock_path)
        .with_context(|| format!("opening {}", lock_path.display()))?;
    lock.lock()
        .with_context(|| format!("locking {}", lock_path.display()))?;
    let profile = args.profile.resolve(linux);
    if linux {
        let formats = if args.package_types.is_empty() {
            vec![Appimage]
        } else {
            args.package_types
        };
        let formats = formats
            .into_iter()
            .map(AtelierPackageType::as_arg)
            .collect::<Vec<_>>()
            .join(",");
        bundle_atelier_linux(ws, &out_dir, profile, &formats)?;
    } else {
        let mut cmd = Process::new("sh");
        cmd.current_dir(ws.root())
            .arg("apps/atelier/package-macos.sh")
            .env("CARGO_TARGET_DIR", ws.target_dir()?);
        if profile.is_release() {
            cmd.arg("--release");
        }
        proc::require(&mut cmd)?;
    }
    if args.install {
        install::bundled(&out_dir, &install::ATELIER)?;
    }
    Ok(ExitCode::SUCCESS)
}

fn bundle_atelier_linux(
    ws: &Workspace,
    out_dir: &Path,
    profile: Profile,
    formats: &str,
) -> Result<()> {
    let packager = cargo_packager(ws)?;

    let target_dir = ws.target_dir()?;
    let mut build = Process::new("cargo");
    build
        .current_dir(ws.root())
        .env("CARGO_TARGET_DIR", &target_dir)
        .args([
            "build",
            "--package",
            "storyfella-atelier",
            "--bin",
            "atelier",
        ]);
    if profile.is_release() {
        build.arg("--release");
    }
    proc::require(&mut build)?;

    // Publish completed packages after the packager succeeds. Keep staging on
    // the same filesystem so replacement of an existing package is atomic.
    let staging = tempfile::Builder::new()
        .prefix(".bundle-")
        .tempdir_in(out_dir)
        .with_context(|| format!("creating a staging directory in {}", out_dir.display()))?;
    // Run the packager binary itself rather than `cargo packager`: its wrapper
    // only strips the extra `packager` argument cargo passes, so the same
    // arguments work either way and a checkout-local install needs no PATH
    // entry.
    let mut package = Process::new(&packager);
    package
        .current_dir(ws.root().join("apps/atelier"))
        .env("CARGO_TARGET_DIR", &target_dir)
        .env("XDG_CACHE_HOME", target_dir.join("atelier-packaging-cache"))
        .env("APPIMAGE_EXTRACT_AND_RUN", "1")
        .args([
            "--packages",
            "storyfella-atelier",
            "--formats",
            formats,
            "--out-dir",
        ])
        .arg(staging.path());
    if profile.is_release() {
        package.arg("--release");
    }
    proc::require(&mut package).with_context(|| {
        format!(
            "packaging Atelier as {formats} (the first AppImage build downloads \
             linuxdeploy from GitHub, and the packager's own log above names the failing step)"
        )
    })?;

    let mut artifacts = Vec::new();
    for entry in fs::read_dir(staging.path())? {
        let entry = entry?;
        if entry.file_type()?.is_file() {
            artifacts.push(entry.file_name());
        }
    }
    if artifacts.is_empty() {
        bail!("cargo packager did not produce any Atelier packages");
    }
    artifacts.sort();
    for artifact in artifacts {
        let dest = out_dir.join(&artifact);
        fs::rename(staging.path().join(artifact), &dest)
            .with_context(|| format!("publishing {}", dest.display()))?;
        proc::done(format!("built {}", dest.display()));
    }
    Ok(())
}

/// The `cargo-packager` release this task drives. Its CLI and
/// `[package.metadata.packager]` schema change between releases, and the
/// workspace's `apps/atelier/Cargo.toml` config is written for this one.
const CARGO_PACKAGER_VERSION: &str = "0.11.8";

/// The pinned packager binary.
///
/// An exact match on `PATH` is used as-is; otherwise the pinned release is
/// installed into `<target>/tools/cargo-packager` the first time it is needed,
/// the same way the other workflows provision their tools. Nothing has to be
/// installed by hand, and nothing is written outside the checkout.
fn cargo_packager(ws: &Workspace) -> Result<PathBuf> {
    match proc::version("cargo-packager", "--version") {
        Some(found) if reports_version(&found, CARGO_PACKAGER_VERSION) => {
            return Ok(PathBuf::from("cargo-packager"));
        }
        Some(found) => proc::step(format!(
            "cargo-packager on PATH reports `{found}`; using the pinned {CARGO_PACKAGER_VERSION} copy instead"
        )),
        None => {}
    }

    let root = ws.target_dir()?.join("tools").join("cargo-packager");
    let binary = root.join("bin").join("cargo-packager");
    if !binary.is_file() {
        proc::step(format!(
            "installing cargo-packager {CARGO_PACKAGER_VERSION} into {}",
            root.display()
        ));
        let mut install = Process::new("cargo");
        install
            .current_dir(ws.root())
            .args([
                "install",
                "cargo-packager",
                "--version",
                CARGO_PACKAGER_VERSION,
                "--locked",
                "--root",
            ])
            .arg(&root);
        proc::require(&mut install).with_context(|| {
            format!(
                "installing cargo-packager {CARGO_PACKAGER_VERSION} (the first run needs \
                 network access to crates.io)"
            )
        })?;
    }

    let shown = binary.display().to_string();
    match proc::version(&shown, "--version") {
        Some(reported) if reports_version(&reported, CARGO_PACKAGER_VERSION) => Ok(binary),
        Some(reported) => bail!(
            "{} reports `{reported}`, not cargo-packager {CARGO_PACKAGER_VERSION}",
            binary.display()
        ),
        None => bail!("{} does not run — remove it and retry", binary.display()),
    }
}

/// Whether a `--version` line names `version` (`cargo-packager 0.11.8`).
fn reports_version(line: &str, version: &str) -> bool {
    line.split_whitespace().last() == Some(version)
}

fn restart_atelier(data_dir: &Path) -> Result<()> {
    let mailbox = storyfella_studio_core::atelier::Mailbox::new(data_dir);
    if mailbox.claim()?.is_some() {
        return Ok(());
    }
    proc::step(format!(
        "requesting Atelier shutdown for {}",
        data_dir.display()
    ));
    let request = mailbox.request_restart()?;
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        if mailbox.claim()?.is_some() {
            let _ = std::fs::remove_file(&request);
            proc::done("previous instance closed after saving recovery");
            return Ok(());
        }
        if Instant::now() >= deadline {
            let _ = std::fs::remove_file(&request);
            bail!(
                "Atelier did not close within 20 seconds. The running version may not support restart requests, or recovery may have failed; close it normally and retry."
            );
        }
        std::thread::sleep(Duration::from_millis(100));
    }
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
    if args.install {
        install::bundled(&out_dir, &install::STUDIO)?;
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
