//! Where the app finds its files.
//!
//! Every path the app resolves for itself — the config file, the plan
//! directory, and the `paths.*` directories the config points at — is anchored
//! to one **project root** rather than the process working directory. That is
//! what lets the GUI, the `svs` CLI, the plan driver and (later) the packaged
//! bundle agree on where things are, whichever directory they are launched
//! from.
//!
//! The root is, in order of preference:
//!
//! 1. `$SVS_HOME`, when set and non-empty;
//! 2. the nearest ancestor of the working directory that holds
//!    `config/app.toml` — the checkout, for every dev invocation;
//! 3. the nearest ancestor of the *executable* that holds `config/app.toml`,
//!    which is how a packaged bundle resolves the resources shipped beside it;
//! 4. the working directory, which leaves a config-less run on the platform
//!    application directories, exactly as it behaved before anchoring.
//!
//! Layered over that root config is one **user config** file in the platform
//! application *config* directory (see [`user_config_path`]). It is only read —
//! never searched for — so it cannot move the root; it exists so a machine can
//! keep an API key or a different endpoint out of the repository.

use std::path::{Path, PathBuf};

use directories::ProjectDirs;

/// The `directories` identity every platform path the app resolves shares: the
/// config, cache, and data (assets/projects) directories.
/// Retain the original identity across the Storyfella workspace rename so
/// existing user settings, project databases, and assets remain discoverable.
pub const APP_QUALIFIER: &str = "dev";
pub const APP_ORGANIZATION: &str = "Smart Visual Sequencer";
pub const APP_APPLICATION: &str = "Smart Visual Sequencer";

/// The config file, relative to the project root.
pub const CONFIG_FILE: &str = "config/app.toml";

/// The user config's file name, inside [`user_config_path`]'s directory.
pub const USER_CONFIG_FILE: &str = "app.toml";

/// The user providers file name, inside [`user_providers_path`]'s directory.
pub const USER_PROVIDERS_FILE: &str = "providers.toml";

/// Where plans live, relative to the project root.
pub const PLANS_DIR: &str = "config/plans";

/// The providers registry file, relative to the project root.
pub const PROVIDERS_FILE: &str = "providers.toml";

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

/// The resolved config file.
pub fn config_path() -> PathBuf {
    root().join(CONFIG_FILE)
}

/// The resolved plans directory.
pub fn plans_dir() -> PathBuf {
    root().join(PLANS_DIR)
}

/// The resolved providers registry file.
pub fn providers_path() -> PathBuf {
    root().join(PROVIDERS_FILE)
}

/// The platform application directories, or `None` where the platform has no
/// home directory. One identity for every `ProjectDirs` the app builds.
pub fn project_dirs() -> Option<ProjectDirs> {
    ProjectDirs::from(APP_QUALIFIER, APP_ORGANIZATION, APP_APPLICATION)
}

/// The user-level config file layered over [`config_path`] — the platform
/// application *config* directory (on Linux,
/// `~/.config/smartvisualsequencer/app.toml`).
///
/// `$SVS_USER_CONFIG` overrides the location when set and non-empty, which
/// lets tests and scratch runs point it at a temporary file. `None` when it is
/// unset and the platform has no config directory, meaning there is no user
/// layer.
pub fn user_config_path() -> Option<PathBuf> {
    if let Some(value) = std::env::var_os("SVS_USER_CONFIG") {
        let path = PathBuf::from(value);
        if !path.as_os_str().is_empty() {
            return Some(path);
        }
    }
    project_dirs().map(|dirs| dirs.config_dir().join(USER_CONFIG_FILE))
}

/// The user-level providers file layered over [`providers_path`] — the platform
/// application *config* directory (on Linux,
/// `~/.config/smartvisualsequencer/providers.toml`).
///
/// `$SVS_USER_PROVIDERS` overrides the location when set and non-empty, which
/// lets tests and scratch runs point it at a temporary file. `None` when it is
/// unset and the platform has no config directory, meaning there is no user
/// layer.
pub fn user_providers_path() -> Option<PathBuf> {
    if let Some(value) = std::env::var_os("SVS_USER_PROVIDERS") {
        let path = PathBuf::from(value);
        if !path.as_os_str().is_empty() {
            return Some(path);
        }
    }
    project_dirs().map(|dirs| dirs.config_dir().join(USER_PROVIDERS_FILE))
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

/// The nearest ancestor of `start` (inclusive) holding [`CONFIG_FILE`].
fn find_root(start: &Path) -> Option<PathBuf> {
    let start = std::path::absolute(start).ok()?;
    start
        .ancestors()
        .find(|dir| dir.join(CONFIG_FILE).is_file())
        .map(Path::to_path_buf)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A temp tree with `config/app.toml` at `root/` and a nested directory
    /// below it, so the upward search has something to find and something to
    /// search from.
    fn nested_tree() -> (PathBuf, PathBuf) {
        let root = std::env::temp_dir().join(format!("svs-paths-{}", uuid::Uuid::new_v4()));
        let nested = root.join("apps").join("cli");
        std::fs::create_dir_all(nested.join("..")).expect("nested dir should create");
        std::fs::create_dir_all(root.join("config")).expect("config dir should create");
        std::fs::write(root.join(CONFIG_FILE), "").expect("config fixture should write");
        (root, nested)
    }

    #[test]
    fn find_root_walks_up_to_the_directory_holding_the_config() {
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
    fn the_module_constants_stay_relative_to_the_root() {
        // The constants are what `root()` searches for; a path-separator typo
        // here would make every search fail silently.
        assert_eq!(config_path(), root().join("config/app.toml"));
        assert_eq!(plans_dir(), root().join("config/plans"));
        assert_eq!(providers_path(), root().join("providers.toml"));
        assert!(Path::new(CONFIG_FILE).is_relative());
        assert!(Path::new(PLANS_DIR).is_relative());
        assert!(Path::new(PROVIDERS_FILE).is_relative());
        assert!(Path::new(USER_CONFIG_FILE).is_relative());
        assert!(Path::new(USER_PROVIDERS_FILE).is_relative());
    }

    #[test]
    fn the_user_config_lives_in_the_platform_config_dir() {
        if std::env::var_os("SVS_USER_CONFIG").is_some() {
            return; // an explicit override is the user's choice; nothing to assert
        }
        let dirs = project_dirs().expect("the platform exposes application directories");
        let path = user_config_path().expect("the platform exposes a config directory");
        assert_eq!(path, dirs.config_dir().join(USER_CONFIG_FILE));
        #[cfg(target_os = "linux")]
        assert!(path.ends_with("smartvisualsequencer/app.toml"), "{path:?}");
    }

    #[test]
    fn the_user_providers_lives_in_the_platform_config_dir() {
        if std::env::var_os("SVS_USER_PROVIDERS").is_some() {
            return; // an explicit override is the user's choice; nothing to assert
        }
        let dirs = project_dirs().expect("the platform exposes application directories");
        let path = user_providers_path().expect("the platform exposes a config directory");
        assert_eq!(path, dirs.config_dir().join(USER_PROVIDERS_FILE));
        #[cfg(target_os = "linux")]
        assert!(
            path.ends_with("smartvisualsequencer/providers.toml"),
            "{path:?}"
        );
    }
}
