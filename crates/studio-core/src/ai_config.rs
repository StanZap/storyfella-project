//! The shared AI configuration: where any front end finds the providers file and
//! the native-server state directory.
//!
//! `clipforge.toml` is the machine's AI configuration — the provider registry
//! ([`Providers`]) that every front end resolves from, so Storyfella Studio,
//! Atelier and the Clipforge apps reach the *same* server address for each
//! capability instead of each keeping its own file. It lives in the platform
//! application config directory (Linux `~/.config/clipforge/clipforge.toml`),
//! never a working-directory path.
//!
//! [`default_state_dir`] is the matching data directory. Native servers are
//! started *detached* with a marker under it ([`agentkit_ai::native::servers`]), so a server one
//! front end started is found and reused by the others ([`agentkit_ai::native::control`]).

use std::path::{Path, PathBuf};

use directories::ProjectDirs;

use agentkit_ai::deployments::{ProviderError, Providers};

/// The `directories` identity the shared configuration and state live under.
const APPLICATION: &str = "clipforge";
const ORGANIZATION: &str = "clipforge";
const QUALIFIER: &str = "dev";

/// The shared AI configuration file's name inside the platform config directory.
pub const DEFAULT_CONFIG_FILE: &str = "clipforge.toml";

/// The earlier name of [`DEFAULT_CONFIG_FILE`], renamed in place on first load.
const LEGACY_CONFIG_FILE: &str = "providers.toml";

/// The shared AI configuration file — the platform application config directory
/// (Linux `~/.config/clipforge/clipforge.toml`). `None` when the platform has no
/// config directory.
pub fn default_config_path() -> Option<PathBuf> {
    ProjectDirs::from(QUALIFIER, ORGANIZATION, APPLICATION)
        .map(|dirs| dirs.config_dir().join(DEFAULT_CONFIG_FILE))
}

/// The data directory transient server state lives under — process markers and
/// detached-server logs ([`agentkit_ai::native::servers`]). On Linux `~/.local/share/clipforge`.
/// `None` when the platform has none.
pub fn default_state_dir() -> Option<PathBuf> {
    ProjectDirs::from(QUALIFIER, ORGANIZATION, APPLICATION)
        .map(|dirs| dirs.data_dir().to_path_buf())
}

/// The shared registry: the user's `clipforge.toml` when it exists, the built-in
/// provider otherwise (and when the platform has no config directory).
pub fn user_or_built_in() -> Result<Providers, ProviderError> {
    match crate::paths::clipforge_config_path() {
        Some(path) => {
            migrate_legacy_config_at(&path);
            Providers::load_or_built_in(&path)
        }
        None => Ok(Providers::built_in()),
    }
}

/// Brings an existing `providers.toml` next to the shared config to its new name,
/// once, so a rename does not silently drop a machine's configuration.
pub fn migrate_legacy_config() {
    if let Some(path) = default_config_path() {
        migrate_legacy_config_at(&path);
    }
}

/// The path-taking form of [`migrate_legacy_config`], for a front end that
/// resolves the shared file itself (through its own override): renames a legacy
/// `providers.toml` beside `config_path` to `config_path`, unless `config_path`
/// already exists (its presence wins — a rename never overwrites one).
pub fn migrate_legacy_config_at(config_path: &Path) {
    rename_legacy(config_path);
}

/// Renames the legacy sibling of `config_path` to `config_path`, unless the new
/// file already exists (its presence wins — a rename never overwrites one).
fn rename_legacy(config_path: &Path) {
    if config_path.exists() {
        return;
    }
    let Some(directory) = config_path.parent() else {
        return;
    };
    let legacy = directory.join(LEGACY_CONFIG_FILE);
    if legacy.exists() {
        let _ = std::fs::rename(&legacy, config_path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_config_file_lives_in_the_platform_config_directory() {
        let path = default_config_path().expect("a platform config directory");
        assert_eq!(path.file_name().expect("a file name"), DEFAULT_CONFIG_FILE);
        assert!(
            path.to_string_lossy().contains(APPLICATION),
            "{path:?} should live under the `{APPLICATION}` application directory"
        );
        assert!(path.is_absolute(), "{path:?} should be absolute");
    }

    #[test]
    fn the_state_directory_is_platform_absolute() {
        let path = default_state_dir().expect("a platform data directory");
        assert!(path.is_absolute(), "{path:?} should be absolute");
        assert!(path.to_string_lossy().contains(APPLICATION));
    }

    #[test]
    fn a_legacy_config_is_renamed_to_the_current_name() {
        let directory = tempfile::tempdir().expect("temp dir");
        let legacy = directory.path().join(LEGACY_CONFIG_FILE);
        std::fs::write(&legacy, "[providers]\n").expect("legacy file");
        let current = directory.path().join(DEFAULT_CONFIG_FILE);

        migrate_legacy_config_at(&current);

        assert!(!legacy.exists(), "the legacy file should be gone");
        assert!(
            current.exists(),
            "the current file should hold its contents"
        );
    }

    #[test]
    fn an_existing_config_is_not_overwritten_by_a_legacy_one() {
        let directory = tempfile::tempdir().expect("temp dir");
        std::fs::write(directory.path().join(LEGACY_CONFIG_FILE), "old").expect("legacy file");
        let current = directory.path().join(DEFAULT_CONFIG_FILE);
        std::fs::write(&current, "new").expect("current file");

        migrate_legacy_config_at(&current);

        assert_eq!(std::fs::read_to_string(&current).unwrap(), "new");
        assert!(directory.path().join(LEGACY_CONFIG_FILE).exists());
    }
}
