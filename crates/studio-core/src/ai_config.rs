//! Shared provider registry and inference service state paths.
//! All participating apps use these platform locations or the same explicit
//! overrides. The service stores worker logs, estimates and policy separately
//! from provider configuration.

use std::path::{Path, PathBuf};
use std::process::Command;

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

/// Shared inference policy, worker logs and resource observations.
pub fn default_state_dir() -> Option<PathBuf> {
    ProjectDirs::from(QUALIFIER, ORGANIZATION, APPLICATION)
        .map(|dirs| dirs.data_dir().to_path_buf())
}

/// Open Clipforge directly to provider management.
pub fn open_provider_manager() -> std::io::Result<()> {
    let executable = if cfg!(windows) {
        "clipforge-desktop.exe"
    } else {
        "clipforge-desktop"
    };
    match Command::new(executable).arg("--manage-providers").spawn() {
        Ok(mut child) => {
            std::thread::spawn(move || {
                let _ = child.wait();
            });
            return Ok(());
        }
        Err(error) if error.kind() != std::io::ErrorKind::NotFound => return Err(error),
        Err(_) => {}
    }
    #[cfg(target_os = "macos")]
    if Command::new("/usr/bin/open")
        .args([
            "-n",
            "-b",
            "com.clipforge.desktop",
            "--args",
            "--manage-providers",
        ])
        .status()?
        .success()
    {
        return Ok(());
    }
    Err(std::io::Error::new(
        std::io::ErrorKind::NotFound,
        "Could not find Clipforge. Install its desktop app or make clipforge-desktop available on PATH.",
    ))
}

/// The configured shared registry. An absent file contains no providers.
pub fn shared_registry() -> Result<Providers, ProviderError> {
    match crate::paths::clipforge_config_path() {
        Some(path) => {
            migrate_legacy_config_at(&path);
            shared_registry_at(&path)
        }
        None => Providers::from_toml("", Path::new(".")),
    }
}

/// Read a shared registry at a known path without inventing provider entries.
pub fn shared_registry_at(path: &Path) -> Result<Providers, ProviderError> {
    match Providers::load(path) {
        Err(ProviderError::Read { source, .. })
            if source.kind() == std::io::ErrorKind::NotFound =>
        {
            Providers::from_toml("", path.parent().unwrap_or(Path::new(".")))
        }
        result => result,
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
    fn a_missing_shared_registry_is_empty_and_a_malformed_one_is_reported() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.svs-e2e/tests");
        std::fs::create_dir_all(&root).unwrap();
        let directory = tempfile::tempdir_in(root).unwrap();
        let path = directory.path().join("clipforge.toml");
        assert_eq!(shared_registry_at(&path).unwrap().list().count(), 0);
        assert!(!path.exists());
        std::fs::write(&path, "[providers\n").unwrap();
        assert!(shared_registry_at(&path).is_err());
    }

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
