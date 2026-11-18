//! Putting a bundled app on the current user's `PATH`.
//!
//! `bundle --install` copies the platform's distributable artifact into a
//! per-user location that needs no elevation: on Linux the AppImage becomes
//! `<bin>/<command>`; on macOS the `.app` is installed into `~/Applications`
//! and its executable linked from `<bin>`. `<bin>` is `$XDG_BIN_HOME` when set,
//! otherwise `~/.local/bin`, which most desktop sessions already have on
//! `PATH`. Nothing is moved out of `dist/`: the bundle artifact stays where the
//! task wrote it, and the installed command is a separate copy the next
//! `--install` replaces.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};

use crate::proc;

/// One bundled app: the name its package artifacts carry, and the command
/// installed on `PATH`.
pub struct App {
    /// The command name placed in the user's bin directory.
    command: &'static str,
    /// The product name the packager derives artifact file names from.
    product: &'static str,
    /// The executable inside the macOS bundle (`Contents/MacOS/<binary>`).
    macos_binary: &'static str,
}

pub const STUDIO: App = App {
    command: "storyfella-studio",
    product: "Storyfella Studio",
    macos_binary: "storyfella-studio",
};

/// Install the selected artifact from the current bundle operation, returning the
/// path that is now on `PATH`.
pub fn bundled(source: &Path, app: &App) -> Result<PathBuf> {
    let bin = bin_dir()?;
    // Both Unix layouts compile everywhere so the macOS branch is checked on
    // every host, not only where it runs.
    #[cfg(unix)]
    let installed = if cfg!(target_os = "macos") {
        install_macos_bundle(source, app, &bin, &applications_dir()?)?
    } else {
        install_appimage(source, app, &bin)?
    };
    #[cfg(not(unix))]
    let installed: PathBuf = {
        let _ = (source, app, &bin);
        bail!(
            "`--install` supports macOS and Linux; the Windows installer package \
             adds the app to PATH itself"
        )
    };
    proc::done(format!("installed {}", installed.display()));
    if !on_path(&bin) {
        proc::step(format!(
            "{} is not on PATH yet — add it to your shell profile to run `{}` by name",
            bin.display(),
            app.command
        ));
    }
    Ok(installed)
}

/// The directory installed commands live in.
fn bin_dir() -> Result<PathBuf> {
    if let Some(dir) = std::env::var_os("XDG_BIN_HOME").filter(|dir| !dir.is_empty()) {
        return Ok(PathBuf::from(dir));
    }
    let home = std::env::var_os("HOME").filter(|home| !home.is_empty()).context(
        "neither XDG_BIN_HOME nor HOME is set, so there is no per-user bin directory to install into",
    )?;
    Ok(PathBuf::from(home).join(".local/bin"))
}

/// Whether `dir` is one of the entries a shell would search.
fn on_path(dir: &Path) -> bool {
    std::env::var_os("PATH")
        .is_some_and(|paths| std::env::split_paths(&paths).any(|entry| entry == dir))
}

/// Letters and digits only, lowercased, so `Storyfella Studio_0.1.0_amd64` and
/// `storyfella-studio_0.1.0_amd64` both name the same product.
fn search_key(text: &str) -> String {
    text.chars()
        .filter(char::is_ascii_alphanumeric)
        .map(|character| character.to_ascii_lowercase())
        .collect()
}

/// Find exactly one matching artifact from this bundle operation.
pub fn artifact(out_dir: &Path, app: &App, extension: &str) -> Result<PathBuf> {
    let wanted = search_key(app.product);
    let mut found = Vec::new();
    for entry in fs::read_dir(out_dir).with_context(|| format!("reading {}", out_dir.display()))? {
        let path = entry?.path();
        if path.extension().is_some_and(|ext| ext == extension)
            && path
                .file_stem()
                .and_then(|stem| stem.to_str())
                .is_some_and(|stem| search_key(stem).contains(&wanted))
        {
            found.push(path);
        }
    }
    match found.as_slice() {
        [path] => Ok(path.clone()),
        [] => bail!(
            "no {} .{} artifact was produced in {}",
            app.product,
            extension,
            out_dir.display()
        ),
        _ => bail!(
            "multiple {} .{} artifacts were produced in {}",
            app.product,
            extension,
            out_dir.display()
        ),
    }
}

fn applications_dir() -> Result<PathBuf> {
    let home = std::env::var_os("HOME")
        .filter(|home| !home.is_empty())
        .context("HOME is not set, so there is no per-user Applications directory")?;
    Ok(PathBuf::from(home).join("Applications"))
}

/// Copy the app's AppImage into `bin` under its command name.
#[cfg(unix)]
fn install_appimage(source: &Path, app: &App, bin: &Path) -> Result<PathBuf> {
    use std::os::unix::fs::PermissionsExt;

    fs::create_dir_all(bin).with_context(|| format!("creating {}", bin.display()))?;
    let dest = bin.join(app.command);
    // Stage beside the destination so the rename replaces an installed copy
    // atomically; a running app keeps the image it mapped.
    let staging = bin.join(format!(".{}.new", app.command));
    fs::copy(source, &staging)
        .with_context(|| format!("copying {} into {}", source.display(), bin.display()))?;
    fs::set_permissions(&staging, fs::Permissions::from_mode(0o755))
        .with_context(|| format!("making {} executable", staging.display()))?;
    fs::rename(&staging, &dest).with_context(|| format!("installing {}", dest.display()))?;
    Ok(dest)
}

/// Install the staged bundle only after validating its executable and link.
#[cfg(unix)]
fn install_macos_bundle(
    source: &Path,
    app: &App,
    bin: &Path,
    applications: &Path,
) -> Result<PathBuf> {
    use std::os::unix::fs::symlink;
    use std::process::Command as Process;

    let name = source.file_name().context("the bundle has no file name")?;
    let dest = applications.join(name);
    fs::create_dir_all(applications)?;
    fs::create_dir_all(bin)?;
    let stage = tempfile::Builder::new()
        .prefix(".storyfella-install-")
        .tempdir_in(applications)?;
    let staged = stage.path().join(name);
    let mut copy = Process::new("cp");
    copy.arg("-R").arg(source).arg(&staged);
    proc::require(&mut copy)?;
    let relative_executable = Path::new("Contents/MacOS").join(app.macos_binary);
    if !staged.join(&relative_executable).is_file() {
        bail!(
            "{} has no {}",
            source.display(),
            relative_executable.display()
        );
    }
    let link_stage = tempfile::Builder::new()
        .prefix(".storyfella-link-")
        .tempdir_in(bin)?;
    let staged_link = link_stage.path().join(app.command);
    symlink(dest.join(&relative_executable), &staged_link)?;
    let previous = stage.path().join("previous.app");
    let had_previous = dest.exists();
    if had_previous {
        fs::rename(&dest, &previous).with_context(|| format!("replacing {}", dest.display()))?;
    }
    if let Err(error) = fs::rename(&staged, &dest) {
        if had_previous {
            if let Err(restore) = fs::rename(&previous, &dest) {
                let saved = stage.keep().join("previous.app");
                bail!("installing {} failed: {error}; restoring the previous app failed: {restore}; it remains at {}", dest.display(), saved.display());
            }
        }
        return Err(error).with_context(|| format!("installing {}", dest.display()));
    }
    let link = bin.join(app.command);
    if let Err(error) = fs::rename(&staged_link, &link) {
        let restore = fs::remove_dir_all(&dest).and_then(|()| {
            if had_previous {
                fs::rename(&previous, &dest)
            } else {
                Ok(())
            }
        });
        if let Err(restore) = restore {
            let saved = stage.keep().join("previous.app");
            bail!("linking {} failed: {error}; restoring the previous app failed: {restore}; any previous app remains at {}", link.display(), saved.display());
        }
        return Err(error).with_context(|| format!("linking {}", link.display()));
    }
    Ok(link)
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    fn staged(dir: &Path, name: &str, contents: &str) -> PathBuf {
        let path = dir.join(name);
        fs::write(&path, contents).unwrap();
        path
    }

    #[test]
    fn installs_the_appimage_that_carries_the_product_name() {
        let root = tempfile::tempdir().unwrap();
        let out = root.path().join("dist");
        let bin = root.path().join("bin");
        fs::create_dir_all(&out).unwrap();
        staged(&out, "Some-Other_App_0.1.0_amd64.AppImage", "other");
        staged(&out, "Storyfella Studio_0.1.0_amd64.AppImage", "studio");

        let installed =
            install_appimage(&artifact(&out, &STUDIO, "AppImage").unwrap(), &STUDIO, &bin).unwrap();

        assert_eq!(installed, bin.join("storyfella-studio"));
        assert_eq!(fs::read_to_string(&installed).unwrap(), "studio");
        assert_eq!(
            fs::read_to_string(out.join("Some-Other_App_0.1.0_amd64.AppImage")).unwrap(),
            "other",
            "the artifact stays in the bundle directory"
        );
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(&installed).unwrap().permissions().mode();
            assert_eq!(mode & 0o111, 0o111, "the installed command is executable");
        }
    }

    #[test]
    fn a_reinstall_replaces_the_previous_copy() {
        let root = tempfile::tempdir().unwrap();
        let out = root.path().join("dist");
        let bin = root.path().join("bin");
        fs::create_dir_all(&out).unwrap();
        staged(&out, "Storyfella Studio_0.1.0_amd64.AppImage", "first");
        install_appimage(&artifact(&out, &STUDIO, "AppImage").unwrap(), &STUDIO, &bin).unwrap();
        fs::remove_file(out.join("Storyfella Studio_0.1.0_amd64.AppImage")).unwrap();
        staged(&out, "Storyfella Studio_0.2.0_amd64.AppImage", "second");

        let installed =
            install_appimage(&artifact(&out, &STUDIO, "AppImage").unwrap(), &STUDIO, &bin).unwrap();

        assert_eq!(installed, bin.join("storyfella-studio"));
        assert_eq!(fs::read_to_string(&installed).unwrap(), "second");
        assert!(
            !bin.join(".storyfella-studio.new").exists(),
            "no staging file is left behind"
        );
    }

    #[test]
    fn a_bundle_without_the_apps_artifact_names_the_flag_that_builds_it() {
        let root = tempfile::tempdir().unwrap();
        let out = root.path().join("dist");
        let bin = root.path().join("bin");
        fs::create_dir_all(&out).unwrap();
        staged(&out, "Storyfella-Studio_0.1.0_amd64.deb", "package");

        let error = artifact(&out, &STUDIO, "AppImage").unwrap_err().to_string();

        assert!(error.contains("no Storyfella Studio .AppImage"), "{error}");
        assert!(!bin.exists(), "nothing is installed without an artifact");
    }

    #[test]
    fn product_names_match_regardless_of_spacing_and_case() {
        assert_eq!(
            search_key("Storyfella Studio_0.1.0_amd64"),
            "storyfellastudio010amd64"
        );
        assert!(
            search_key("storyfella-studio_0.1.0_amd64").contains(&search_key("Storyfella Studio"))
        );
        assert!(!search_key("smart-visual-sequencer_0.1.0_x86_64")
            .contains(&search_key("Storyfella Studio")));
    }

    #[test]
    fn bundle_selection_ignores_old_artifacts_outside_the_current_staging_directory() {
        let root = tempfile::tempdir().unwrap();
        let current = root.path().join("current");
        fs::create_dir_all(&current).unwrap();
        staged(root.path(), "Storyfella Studio_old.AppImage", "old");
        staged(&current, "Storyfella Studio_new.deb", "new");
        assert!(artifact(&current, &STUDIO, "AppImage").is_err());
    }

    #[test]
    fn malformed_macos_bundle_leaves_the_previous_app_and_link_intact() {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("source/Storyfella Studio.app");
        let applications = root.path().join("Applications");
        let bin = root.path().join("bin");
        let previous = applications.join("Storyfella Studio.app/Contents/MacOS");
        fs::create_dir_all(&source).unwrap();
        fs::create_dir_all(&previous).unwrap();
        fs::create_dir_all(&bin).unwrap();
        fs::write(previous.join("storyfella-studio"), "old").unwrap();
        std::os::unix::fs::symlink(previous.join("storyfella-studio"), bin.join(STUDIO.command))
            .unwrap();

        assert!(install_macos_bundle(&source, &STUDIO, &bin, &applications).is_err());
        assert_eq!(fs::read_to_string(bin.join(STUDIO.command)).unwrap(), "old");
    }

    #[test]
    fn link_failure_restores_the_previous_macos_app() {
        let root = tempfile::tempdir().unwrap();
        let source = root
            .path()
            .join("source/Storyfella Studio.app/Contents/MacOS");
        let applications = root.path().join("Applications");
        let bin = root.path().join("bin");
        let previous = applications.join("Storyfella Studio.app/Contents/MacOS");
        fs::create_dir_all(&source).unwrap();
        fs::create_dir_all(&previous).unwrap();
        fs::create_dir_all(bin.join(STUDIO.command)).unwrap();
        fs::write(source.join("storyfella-studio"), "new").unwrap();
        fs::write(previous.join("storyfella-studio"), "old").unwrap();

        assert!(install_macos_bundle(
            &root.path().join("source/Storyfella Studio.app"),
            &STUDIO,
            &bin,
            &applications
        )
        .is_err());
        assert_eq!(
            fs::read_to_string(previous.join("storyfella-studio")).unwrap(),
            "old"
        );
    }

    #[test]
    fn valid_macos_bundle_replaces_the_app_and_command() {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("source/Storyfella Studio.app");
        let applications = root.path().join("Applications");
        let bin = root.path().join("bin");
        let staged_executable = source.join("Contents/MacOS/storyfella-studio");
        let old_executable =
            applications.join("Storyfella Studio.app/Contents/MacOS/storyfella-studio");
        fs::create_dir_all(staged_executable.parent().unwrap()).unwrap();
        fs::create_dir_all(old_executable.parent().unwrap()).unwrap();
        fs::create_dir_all(&bin).unwrap();
        fs::write(staged_executable, "new").unwrap();
        fs::write(old_executable, "old").unwrap();
        fs::write(bin.join(STUDIO.command), "old command").unwrap();

        let link = install_macos_bundle(&source, &STUDIO, &bin, &applications).unwrap();
        assert_eq!(fs::read_to_string(link).unwrap(), "new");
    }
}
