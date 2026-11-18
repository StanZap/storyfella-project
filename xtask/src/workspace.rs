//! Where things live: the workspace root, the resolved cargo target directory,
//! and the built app binary.
//!
//! Nothing here is relative to the *working* directory in the way the shell
//! usually assumes — commands run against absolute paths computed from the
//! discovered workspace root, so `cargo xtask …` behaves the same from any
//! subdirectory.

use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use serde::Deserialize;

/// The desktop package name.
pub const APP_PACKAGE: &str = "storyfella-studio";
/// The desktop binary (package and binary share a name).
pub const APP_BIN: &str = "storyfella-studio";

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

    /// The gitignored home for validation logs (`AGENTS.md` → Validation).
    pub fn e2e_logs(&self) -> PathBuf {
        self.root.join(".svs-e2e/logs")
    }

    /// The cargo target directory, honouring `.cargo/config.toml`'s
    /// `build.target-dir` redirect (supports parent paths containing `:` —
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

    /// The binary `cargo build -p storyfella-studio` produces.
    pub fn app_binary(&self, profile: Profile) -> Result<PathBuf> {
        let binary = self
            .target_dir()?
            .join(profile.dir())
            .join(APP_BIN)
            .with_extension(std::env::consts::EXE_EXTENSION);
        if !binary.is_file() {
            bail!("expected a built app binary at {}", binary.display());
        }
        Ok(binary)
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

#[derive(Deserialize)]
struct CargoConfig {
    build: Option<CargoBuild>,
}

#[derive(Deserialize)]
struct CargoBuild {
    #[serde(rename = "target-dir")]
    target_dir: Option<String>,
}
