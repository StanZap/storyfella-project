//! Installing the bundled app for the current user.
//!
//! `bundle --install` copies `Storyfella Studio.app` into `~/Applications` and
//! links its executable as `<bin>/storyfella-studio`, where `<bin>` is
//! `$XDG_BIN_HOME` when set, otherwise `~/.local/bin`. Nothing needs elevation,
//! and the bundle stays in `dist/` for the next build to replace.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command as Process;

use anyhow::{bail, Context, Result};

use crate::proc;

/// The bundle `apps/studio/package-macos.sh` writes into `dist/studio`.
pub const BUNDLE: &str = "Storyfella Studio.app";
/// The executable inside it, `Contents/MacOS/<EXECUTABLE>`.
const EXECUTABLE: &str = "storyfella-studio";
/// The command placed in the user's bin directory.
const COMMAND: &str = "storyfella-studio";

/// Installs the bundle in `out_dir`, returning the command now on `PATH`.
pub fn bundled(out_dir: &Path) -> Result<PathBuf> {
    let home = std::env::var_os("HOME")
        .filter(|home| !home.is_empty())
        .context("HOME is not set, so there is no per-user Applications directory")?;
    let home = PathBuf::from(home);
    let bin = match std::env::var_os("XDG_BIN_HOME").filter(|dir| !dir.is_empty()) {
        Some(dir) => PathBuf::from(dir),
        None => home.join(".local/bin"),
    };
    let link = install(&out_dir.join(BUNDLE), &home.join("Applications"), &bin)?;
    proc::done(format!("installed {}", link.display()));
    let on_path = std::env::var_os("PATH")
        .is_some_and(|paths| std::env::split_paths(&paths).any(|entry| entry == bin));
    if !on_path {
        proc::step(format!(
            "{} is not on PATH yet — add it to your shell profile to run `{COMMAND}` by name",
            bin.display()
        ));
    }
    Ok(link)
}

/// Copies `source` into `applications`, replacing an installed copy, and
/// links its executable from `bin`.
fn install(source: &Path, applications: &Path, bin: &Path) -> Result<PathBuf> {
    if !source.join("Contents/MacOS").join(EXECUTABLE).is_file() {
        bail!(
            "no bundle at {} — build one with `cargo xtask bundle`",
            source.display()
        );
    }
    fs::create_dir_all(applications)
        .with_context(|| format!("creating {}", applications.display()))?;
    let dest = applications.join(BUNDLE);
    // Copy beside the destination first, so a failed copy leaves the
    // installed app alone; `cp -R` keeps the bundle's signature intact.
    let staging = applications.join(format!(".{BUNDLE}.new"));
    let previous = applications.join(format!(".{BUNDLE}.old"));
    let _ = fs::remove_dir_all(&staging);
    let _ = fs::remove_dir_all(&previous);
    let mut copy = Process::new("cp");
    copy.arg("-R").arg(source).arg(&staging);
    proc::require(&mut copy)?;
    if dest.exists() {
        fs::rename(&dest, &previous).with_context(|| format!("replacing {}", dest.display()))?;
    }
    fs::rename(&staging, &dest).with_context(|| format!("installing {}", dest.display()))?;
    let _ = fs::remove_dir_all(&previous);

    fs::create_dir_all(bin).with_context(|| format!("creating {}", bin.display()))?;
    let link = bin.join(COMMAND);
    match fs::remove_file(&link) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error).with_context(|| format!("replacing {}", link.display())),
    }
    std::os::unix::fs::symlink(dest.join("Contents/MacOS").join(EXECUTABLE), &link)
        .with_context(|| format!("linking {}", link.display()))?;
    Ok(link)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bundle(dir: &Path, contents: &str) -> PathBuf {
        let bundle = dir.join(BUNDLE);
        fs::create_dir_all(bundle.join("Contents/MacOS")).unwrap();
        fs::write(bundle.join("Contents/MacOS").join(EXECUTABLE), contents).unwrap();
        bundle
    }

    #[test]
    fn installs_the_bundle_and_links_its_executable() {
        let root = tempfile::tempdir().unwrap();
        let source = bundle(&root.path().join("dist"), "studio");
        let applications = root.path().join("Applications");
        let bin = root.path().join("bin");

        let link = install(&source, &applications, &bin).unwrap();

        assert_eq!(link, bin.join(COMMAND));
        assert_eq!(fs::read_to_string(&link).unwrap(), "studio");
        assert_eq!(
            fs::read_link(&link).unwrap(),
            applications
                .join(BUNDLE)
                .join("Contents/MacOS")
                .join(EXECUTABLE)
        );
        assert!(source.exists(), "the bundle stays in dist");
    }

    #[test]
    fn a_reinstall_replaces_the_previous_copy() {
        let root = tempfile::tempdir().unwrap();
        let applications = root.path().join("Applications");
        let bin = root.path().join("bin");
        install(
            &bundle(&root.path().join("first"), "first"),
            &applications,
            &bin,
        )
        .unwrap();

        let link = install(
            &bundle(&root.path().join("second"), "second"),
            &applications,
            &bin,
        )
        .unwrap();

        assert_eq!(fs::read_to_string(&link).unwrap(), "second");
        let left: Vec<_> = fs::read_dir(&applications)
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        assert_eq!(left, [BUNDLE], "no staging or previous copy is left behind");
    }

    #[test]
    fn nothing_is_installed_without_a_bundle() {
        let root = tempfile::tempdir().unwrap();
        let applications = root.path().join("Applications");

        let error = install(
            &root.path().join(BUNDLE),
            &applications,
            &root.path().join("bin"),
        )
        .unwrap_err()
        .to_string();

        assert!(error.contains("cargo xtask bundle"), "{error}");
        assert!(!applications.exists());
    }
}
