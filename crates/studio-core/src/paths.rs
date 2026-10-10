//! Where the app finds its files.
//!
//! There are two configuration files, split by concern and both platform-local:
//!
//! * the **app config** ([`config_path`]) — `studio.toml` in the platform
//!   application config directory — owns paths, generation defaults, LoRA
//!   bindings and runtime prefs;
//! * the **shared AI config** ([`clipforge_config_path`]) — the machine's
//!   `clipforge.toml` — owns the provider registry, and is the same file Atelier
//!   and Clipforge read.
//!
//! Explicit relative app paths — the plan directory and the `paths.*`
//! directories the config points at — are anchored to one **project root**
//! rather than the process working directory, so the GUI, the `svs` CLI, the
//! plan driver and the packaged bundle agree on where they are.
//!
//! The root is, in order of preference:
//!
//! 1. `$SVS_HOME`, when set and non-empty;
//! 2. the nearest ancestor of the working directory that holds `config/plans` —
//!    the checkout, for every dev invocation;
//! 3. the nearest ancestor of the *executable* that holds `config/plans`, which
//!    is how a packaged bundle resolves the resources shipped beside it;
//! 4. the working directory.
//!
//! Model storage defaults to OS-local Storyfella application data in all builds;
//! it is independent of the checkout unless explicitly overridden.

use std::path::{Path, PathBuf};

use directories::{BaseDirs, ProjectDirs};

/// The `directories` identity every platform path the app resolves shares: the
/// config, cache, and data (assets/projects) directories.
/// Retain the original identity across the Storyfella workspace rename so
/// existing user settings, project databases, and assets remain discoverable.
pub const APP_QUALIFIER: &str = "dev";
pub const APP_ORGANIZATION: &str = "Smart Visual Sequencer";
pub const APP_APPLICATION: &str = "Smart Visual Sequencer";

/// The app config file's name, inside the platform application config directory.
pub const CONFIG_FILE: &str = "studio.toml";

/// The environment variable overriding the app config file's location, so tests
/// and scratch runs can point it a temporary file.
pub const APP_CONFIG_ENV: &str = "SVS_USER_CONFIG";

/// The environment variable overriding the shared AI config file's location, so
/// tests and scratch runs can point it away from the machine's real
/// `clipforge.toml`.
pub const AI_CONFIG_ENV: &str = "SVS_AI_CONFIG";

/// The environment variable overriding the shared native-server state directory
/// (process markers and detached-server logs), so tests and hermetic plan runs
/// never touch the machine's real one.
pub const STATE_DIR_ENV: &str = "SVS_STATE_DIR";

/// Where plans live, relative to the project root — and the directory that
/// identifies the root: its nearest ancestor holding it.
pub const PLANS_DIR: &str = "config/plans";

/// The project root — see the module comment for the search order.
pub fn root() -> PathBuf {
    if let Some(value) = std::env::var_os("SVS_HOME") {
        let home = PathBuf::from(value);
        if !home.as_os_str().is_empty() {
            return std::path::absolute(&home).unwrap_or(home);
        }
    }
    if let Ok(cwd) = std::env::current_dir() {
        if let Some(found) = find_root(&cwd) {
            return found;
        }
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(found) = exe.parent().and_then(find_root) {
            return found;
        }
    }
    std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
}

/// The app config file — `studio.toml` in the platform application *config*
/// directory (on Linux `~/.config/smartvisualsequencer/studio.toml`).
///
/// `$SVS_USER_CONFIG` overrides the location when set and non-empty, which lets
/// tests and scratch runs point it at a temporary file. `None` when it is unset
/// and the platform has no config directory.
pub fn config_path() -> Option<PathBuf> {
    if let Some(value) = std::env::var_os(APP_CONFIG_ENV) {
        let path = PathBuf::from(value);
        if !path.as_os_str().is_empty() {
            return Some(path);
        }
    }
    project_dirs().map(|dirs| dirs.config_dir().join(CONFIG_FILE))
}

/// The resolved plans directory.
pub fn plans_dir() -> PathBuf {
    root().join(PLANS_DIR)
}

/// The platform application directories, or `None` where the platform has no
/// home directory. One identity for every `ProjectDirs` the app builds.
pub fn project_dirs() -> Option<ProjectDirs> {
    ProjectDirs::from(APP_QUALIFIER, APP_ORGANIZATION, APP_APPLICATION)
}

/// The machine's shared AI configuration file — `clipforge.toml` at the
/// platform location every front end resolves
/// (`clipforge_inference::config::default_config_path()`; on Linux
/// `~/.config/clipforge/clipforge.toml`). It is created with Storyfella's
/// defaults on first load when absent (see `config::load`).
///
/// `$SVS_AI_CONFIG` overrides the location when set and non-empty, so tests and
/// scratch runs can point it at a temporary file. `None` when it is unset and
/// the platform has no config directory.
pub fn clipforge_config_path() -> Option<PathBuf> {
    if let Some(value) = std::env::var_os(AI_CONFIG_ENV) {
        let path = PathBuf::from(value);
        if !path.as_os_str().is_empty() {
            return Some(path);
        }
    }
    clipforge_inference::config::default_config_path()
}

/// The shared native-server state directory — process markers and detached-server
/// logs at the platform location every front end resolves
/// (`clipforge_inference::config::default_state_dir()`; on Linux
/// `~/.local/share/clipforge`). One directory for the machine, so a server one
/// front end started is found and reused by the others rather than a second one
/// being launched.
///
/// `$SVS_STATE_DIR` overrides the location when set and non-empty, so tests and
/// hermetic plan runs stay off the developer's real state. `None` when it is
/// unset and the platform has no data directory.
pub fn state_dir() -> Option<PathBuf> {
    if let Some(value) = std::env::var_os(STATE_DIR_ENV) {
        let path = PathBuf::from(value);
        if !path.as_os_str().is_empty() {
            return Some(path);
        }
    }
    clipforge_inference::config::default_state_dir()
}

/// Large model files belong in local application data, never roaming config,
/// the checkout, or the installed bundle. Resolves XDG/Windows/macOS through
/// the native directory helper; does not create folders while reading config.
pub fn model_dir() -> Option<PathBuf> {
    let dirs = BaseDirs::new()?;
    let name = if cfg!(any(target_os = "macos", target_os = "windows")) {
        "Storyfella"
    } else {
        "storyfella"
    };
    Some(dirs.data_local_dir().join(name).join("models"))
}

/// `path` made absolute against [`root`] — unless it already is absolute, so a
/// path the user wrote as `/opt/models` is never rewritten.
pub fn resolve(path: impl AsRef<Path>) -> PathBuf {
    let path = path.as_ref();
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        root().join(path)
    }
}

/// The nearest ancestor of `start` (inclusive) holding [`PLANS_DIR`].
fn find_root(start: &Path) -> Option<PathBuf> {
    let start = std::path::absolute(start).ok()?;
    start
        .ancestors()
        .find(|dir| dir.join(PLANS_DIR).is_dir())
        .map(Path::to_path_buf)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A temp tree with `config/plans/` at `root/` and a nested directory below
    /// it, so the upward search has something to find and something to search
    /// from.
    fn nested_tree() -> (PathBuf, PathBuf) {
        let root = std::env::temp_dir().join(format!("svs-paths-{}", uuid::Uuid::new_v4()));
        let nested = root.join("apps").join("cli");
        std::fs::create_dir_all(nested.join("..")).expect("nested dir should create");
        std::fs::create_dir_all(root.join(PLANS_DIR)).expect("plans dir should create");
        (root, nested)
    }

    #[test]
    fn find_root_walks_up_to_the_directory_holding_the_plans() {
        let (root, nested) = nested_tree();
        let expected = std::path::absolute(&root).expect("temp dir should absolute-ize");
        assert_eq!(find_root(&nested).as_deref(), Some(expected.as_path()));
        assert_eq!(find_root(&expected).as_deref(), Some(expected.as_path()));
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn resolve_joins_relative_paths_to_the_root_and_leaves_absolute_ones() {
        let (root, _) = nested_tree();
        let absolute = Path::new("/opt/models");
        assert_eq!(resolve(absolute), absolute.to_path_buf());
        // A relative path resolves under *some* root; assert the shape rather
        // than a specific root, which depends on where the tests run.
        let resolved = resolve("models");
        assert!(resolved.is_absolute(), "{resolved:?} should be absolute");
        assert!(resolved.ends_with("models"));
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn the_plans_directory_stays_relative_to_the_root() {
        // The constant is what `root()` searches for; a path-separator typo here
        // would make every search fail silently.
        assert_eq!(plans_dir(), root().join(PLANS_DIR));
        assert!(Path::new(PLANS_DIR).is_relative());
        // The config file's name is a bare file name, not a root-relative path.
        assert_eq!(Path::new(CONFIG_FILE).components().count(), 1);
    }

    #[test]
    fn the_app_config_lives_in_the_platform_config_dir() {
        if std::env::var_os(APP_CONFIG_ENV).is_some() {
            return; // an explicit override is the user's choice; nothing to assert
        }
        let dirs = project_dirs().expect("the platform exposes application directories");
        let path = config_path().expect("the platform exposes a config directory");
        assert_eq!(path, dirs.config_dir().join(CONFIG_FILE));
        #[cfg(target_os = "linux")]
        assert!(
            path.ends_with("smartvisualsequencer/studio.toml"),
            "{path:?}"
        );
    }

    #[test]
    fn models_live_in_local_application_data() {
        let dirs = BaseDirs::new().expect("platform directories");
        let path = model_dir().expect("model directory");
        assert!(path.starts_with(dirs.data_local_dir()));
        assert!(path.ends_with("models"));
        assert!(!path.starts_with(root()));
    }

    #[test]
    fn the_shared_ai_config_defaults_to_the_clipforge_config_directory() {
        if std::env::var_os(AI_CONFIG_ENV).is_some() {
            return; // an explicit override is the user's choice; nothing to assert
        }
        let path = clipforge_config_path().expect("the platform exposes a config directory");
        assert_eq!(
            path,
            clipforge_inference::config::default_config_path().unwrap()
        );
        #[cfg(target_os = "linux")]
        assert!(path.ends_with("clipforge/clipforge.toml"), "{path:?}");
    }

    #[test]
    fn the_shared_state_dir_defaults_to_the_clipforge_data_directory() {
        if std::env::var_os(STATE_DIR_ENV).is_some() {
            return; // an explicit override is the user's choice; nothing to assert
        }
        let path = state_dir().expect("the platform exposes a data directory");
        assert_eq!(
            path,
            clipforge_inference::config::default_state_dir().unwrap()
        );
        assert!(path.is_absolute(), "{path:?}");
    }
}
