//! Where things live: the workspace root, the resolved cargo target directory,
//! and the `dx`-laid-out app binary.
//!
//! Nothing here is relative to the *working* directory in the way the shell
//! usually assumes — commands run against absolute paths computed from the
//! discovered workspace root, so `cargo xtask …` behaves the same from any
//! subdirectory.

use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use serde::Deserialize;

/// The desktop package name; `dx` also names its build directory after it.
pub const APP_PACKAGE: &str = "smart-visual-sequencer";
/// The desktop binary (package and binary share a name).
pub const APP_BIN: &str = "smart-visual-sequencer";

/// Which cargo profile to build.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Profile {
    Debug,
    Release,
}

impl Profile {
    /// The directory name cargo uses for this profile.
    pub fn dir(self) -> &'static str {
        match self {
            Profile::Debug => "debug",
            Profile::Release => "release",
        }
    }

    pub fn is_release(self) -> bool {
        matches!(self, Profile::Release)
    }

    /// Human-facing name for log lines.
    pub fn label(self) -> &'static str {
        match self {
            Profile::Debug => "debug",
            Profile::Release => "release",
        }
    }
}

/// The repository, anchored at its root.
pub struct Workspace {
    root: PathBuf,
}

impl Workspace {
    /// Find the workspace root: the nearest ancestor of the working directory
    /// holding a `Cargo.toml` with a `[workspace]` table.
    ///
    /// Falls back to this crate's own manifest directory (baked in at compile
    /// time) so the command still works when invoked from somewhere unrelated,
    /// such as a desktop launcher.
    pub fn discover() -> Result<Self> {
        let cwd = std::env::current_dir().context("reading the working directory")?;
        if let Some(root) = find_root(&cwd) {
            return Ok(Self { root });
        }
        let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
        if let Some(root) = manifest.parent().and_then(find_root) {
            return Ok(Self { root });
        }
        bail!(
            "could not locate the workspace root — no ancestor of {} holds a \
             Cargo.toml with a [workspace] table",
            cwd.display()
        )
    }

    /// The repository root.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Resolve a path against the workspace root when it is relative.
    pub fn resolve(&self, path: &Path) -> PathBuf {
        if path.is_absolute() {
            path.to_path_buf()
        } else {
            self.root.join(path)
        }
    }

    pub fn desktop(&self) -> PathBuf {
        self.root.join("apps/desktop")
    }

    /// Where `cargo xtask bundle` writes artifacts by default.
    pub fn dist(&self) -> PathBuf {
        self.root.join("dist")
    }

    /// The gitignored home for validation logs (`AGENTS.md` → Validation).
    pub fn e2e_logs(&self) -> PathBuf {
        self.root.join(".svs-e2e/logs")
    }

    /// The cargo target directory, honouring `.cargo/config.toml`'s
    /// `build.target-dir` redirect (the `:` in this path makes it mandatory —
    /// never assume `<root>/target`).
    pub fn target_dir(&self) -> Result<PathBuf> {
        if let Some(dir) = std::env::var_os("CARGO_TARGET_DIR") {
            return Ok(self.resolve(Path::new(&dir)));
        }
        let config = self.root.join(".cargo/config.toml");
        if config.is_file() {
            let text = std::fs::read_to_string(&config)
                .with_context(|| format!("reading {}", config.display()))?;
            let parsed: CargoConfig =
                toml::from_str(&text).with_context(|| format!("parsing {}", config.display()))?;
            if let Some(dir) = parsed.build.and_then(|build| build.target_dir) {
                return Ok(self.resolve(Path::new(&dir)));
            }
        }
        Ok(self.root.join("target"))
    }

    /// The binary `dx` drops after a desktop build.
    ///
    /// The documented layout is
    /// `<target>/dx/<package>/<profile>/<platform>/app/<bin>`; a recursive
    /// search under the package directory covers any future `dx` that moves the
    /// platform segment.
    pub fn app_binary(&self, profile: Profile) -> Result<PathBuf> {
        let dir = self
            .target_dir()?
            .join("dx")
            .join(APP_PACKAGE)
            .join(profile.dir());
        let direct = dir.join(host_platform_dir()).join("app").join(APP_BIN);
        if direct.is_file() {
            return Ok(direct);
        }
        if let Some(found) = find_app_binary(&dir) {
            return Ok(found);
        }
        bail!(
            "could not find a built app binary under {}\n  \
             expected {} — did `dx build` succeed?",
            dir.display(),
            direct.display()
        )
    }
}

/// The `dx` platform directory for the host.
fn host_platform_dir() -> &'static str {
    match std::env::consts::OS {
        "macos" => "macos",
        "windows" => "windows",
        _ => "linux",
    }
}

/// The nearest ancestor of `start` holding a workspace manifest.
fn find_root(start: &Path) -> Option<PathBuf> {
    let mut dir = Some(start);
    while let Some(current) = dir {
        let manifest = current.join("Cargo.toml");
        if manifest.is_file()
            && std::fs::read_to_string(&manifest)
                .map(|text| text.contains("[workspace]"))
                .unwrap_or(false)
        {
            return Some(current.to_path_buf());
        }
        dir = current.parent();
    }
    None
}

/// The app binary inside an `app/` directory, found by walking `dir`.
fn find_app_binary(dir: &Path) -> Option<PathBuf> {
    let entries = std::fs::read_dir(dir).ok()?;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if let Some(found) = find_app_binary(&path) {
                return Some(found);
            }
        } else if entry.file_name().to_string_lossy() == APP_BIN
            && path
                .parent()
                .and_then(Path::file_name)
                .map(|name| name == "app")
                .unwrap_or(false)
        {
            return Some(path);
        }
    }
    None
}

#[derive(Deserialize)]
struct CargoConfig {
    build: Option<CargoBuild>,
}

#[derive(Deserialize)]
struct CargoBuild {
    #[serde(rename = "target-dir")]
    target_dir: Option<String>,
}
