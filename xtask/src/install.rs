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

pub const ATELIER: App = App {
    command: "atelier",
    product: "Atelier",
    macos_binary: "Atelier",
};

/// Install the artifact the bundle just wrote into `out_dir`, returning the
/// path that is now on `PATH`.
pub fn bundled(out_dir: &Path, app: &App) -> Result<PathBuf> {
    let bin = bin_dir()?;
    // Both Unix layouts compile everywhere so the macOS branch is checked on
    // every host, not only where it runs.
    #[cfg(unix)]
    let installed = if cfg!(target_os = "macos") {
        install_macos_bundle(out_dir, app, &bin)?
    } else {
        install_appimage(out_dir, app, &bin)?
    };
    #[cfg(not(unix))]
    let installed: PathBuf = {
        let _ = (out_dir, app, &bin);
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

/// The newest AppImage in `out_dir` whose name carries this app's product name.
#[cfg(unix)]
fn newest_appimage(out_dir: &Path, app: &App) -> Result<PathBuf> {
    let wanted = search_key(app.product);
    let mut found = Vec::new();
    let entries =
        fs::read_dir(out_dir).with_context(|| format!("reading {}", out_dir.display()))?;
    for entry in entries {
        let path = entry?.path();
        let Some(stem) = path.file_stem().and_then(|stem| stem.to_str()) else {
            continue;
        };
        if path.extension().is_some_and(|ext| ext == "AppImage")
            && search_key(stem).contains(&wanted)
        {
            found.push(path);
        }
    }
    if found.is_empty() {
        bail!(
            "no {} AppImage in {} — build one with `--package-types appimage`",
            app.product,
            out_dir.display()
        );
    }
    found.sort_by_key(|path| fs::metadata(path).and_then(|meta| meta.modified()).ok());
    Ok(found.pop().expect("at least one AppImage was found"))
}

/// Copy the app's AppImage into `bin` under its command name.
#[cfg(unix)]
fn install_appimage(out_dir: &Path, app: &App, bin: &Path) -> Result<PathBuf> {
    use std::os::unix::fs::PermissionsExt;

    let source = newest_appimage(out_dir, app)?;
    fs::create_dir_all(bin).with_context(|| format!("creating {}", bin.display()))?;
    let dest = bin.join(app.command);
    // Stage beside the destination so the rename replaces an installed copy
    // atomically; a running app keeps the image it mapped.
    let staging = bin.join(format!(".{}.new", app.command));
    fs::copy(&source, &staging)
        .with_context(|| format!("copying {} into {}", source.display(), bin.display()))?;
    fs::set_permissions(&staging, fs::Permissions::from_mode(0o755))
        .with_context(|| format!("making {} executable", staging.display()))?;
    fs::rename(&staging, &dest).with_context(|| format!("installing {}", dest.display()))?;
    Ok(dest)
}

/// Install the `.app` into `~/Applications` and link its executable into `bin`.
#[cfg(unix)]
fn install_macos_bundle(out_dir: &Path, app: &App, bin: &Path) -> Result<PathBuf> {
    use std::os::unix::fs::symlink;
    use std::process::Command as Process;

    let source = newest_bundle(out_dir, app)?;
    let home = std::env::var_os("HOME")
        .filter(|home| !home.is_empty())
        .context("HOME is not set, so there is no per-user Applications directory")?;
    let applications = PathBuf::from(home).join("Applications");
    fs::create_dir_all(&applications)
        .with_context(|| format!("creating {}", applications.display()))?;
    let name = source
        .file_name()
        .context("the bundle has no file name")?
        .to_owned();
    let dest = applications.join(&name);

    // Copy beside the destination first; `cp -R` keeps the packager's layout.
    let staging = applications.join(format!(".{}.new", name.to_string_lossy()));
    let _ = fs::remove_dir_all(&staging);
    let mut copy = Process::new("cp");
    copy.args(["-R"]).arg(&source).arg(&staging);
    proc::require(&mut copy)?;
    let previous = applications.join(format!(".{}.old", name.to_string_lossy()));
    let _ = fs::remove_dir_all(&previous);
    if dest.exists() {
        fs::rename(&dest, &previous).with_context(|| format!("replacing {}", dest.display()))?;
    }
    fs::rename(&staging, &dest).with_context(|| format!("installing {}", dest.display()))?;
    let _ = fs::remove_dir_all(&previous);

    let executable = dest.join("Contents/MacOS").join(app.macos_binary);
    if !executable.is_file() {
        bail!(
            "{} has no Contents/MacOS/{}",
            dest.display(),
            app.macos_binary
        );
    }
    fs::create_dir_all(bin).with_context(|| format!("creating {}", bin.display()))?;
    let link = bin.join(app.command);
    match fs::remove_file(&link) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            return Err(error).with_context(|| format!("replacing {}", link.display()));
        }
    }
    symlink(&executable, &link).with_context(|| format!("linking {}", link.display()))?;
    Ok(link)
}

/// The newest `.app` bundle in `out_dir` whose name carries this app's product
/// name.
#[cfg(unix)]
fn newest_bundle(out_dir: &Path, app: &App) -> Result<PathBuf> {
    let wanted = search_key(app.product);
    let mut found = Vec::new();
    let entries =
        fs::read_dir(out_dir).with_context(|| format!("reading {}", out_dir.display()))?;
    for entry in entries {
        let path = entry?.path();
        if path.is_dir()
            && path.extension().is_some_and(|ext| ext == "app")
            && path
                .file_stem()
                .and_then(|stem| stem.to_str())
                .is_some_and(|stem| search_key(stem).contains(&wanted))
        {
            found.push(path);
        }
    }
    if found.is_empty() {
        bail!(
            "no {} application bundle in {} — build one with `--package-types macos`",
            app.product,
            out_dir.display()
        );
    }
    found.sort_by_key(|path| fs::metadata(path).and_then(|meta| meta.modified()).ok());
    Ok(found.pop().expect("at least one bundle was found"))
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

        let installed = install_appimage(&out, &STUDIO, &bin).unwrap();

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
        staged(&out, "Atelier_0.1.0_amd64.AppImage", "first");
        install_appimage(&out, &ATELIER, &bin).unwrap();
        fs::remove_file(out.join("Atelier_0.1.0_amd64.AppImage")).unwrap();
        staged(&out, "Atelier_0.2.0_amd64.AppImage", "second");

        let installed = install_appimage(&out, &ATELIER, &bin).unwrap();

        assert_eq!(installed, bin.join("atelier"));
        assert_eq!(fs::read_to_string(&installed).unwrap(), "second");
        assert!(
            !bin.join(".atelier.new").exists(),
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

        let error = install_appimage(&out, &STUDIO, &bin)
            .unwrap_err()
            .to_string();

        assert!(error.contains("--package-types appimage"), "{error}");
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
}
