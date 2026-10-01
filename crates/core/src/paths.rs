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

use std::path::{Path, PathBuf};

/// The config file, relative to the project root.
pub const CONFIG_FILE: &str = "config/app.toml";

/// Where plans live, relative to the project root.
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

/// The resolved config file.
pub fn config_path() -> PathBuf {
    root().join(CONFIG_FILE)
}

/// The resolved plans directory.
pub fn plans_dir() -> PathBuf {
    root().join(PLANS_DIR)
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
        assert!(Path::new(CONFIG_FILE).is_relative());
        assert!(Path::new(PLANS_DIR).is_relative());
    }
}
