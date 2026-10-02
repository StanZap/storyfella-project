//! Preparing the image model: which weight files the served profile loads,
//! where each comes from, and the in-app download that fills the gaps.
//!
//! The `model_setup` binary does the same thing on the command line; this is
//! the layer the app's Settings screen drives, so a first run can prepare the
//! model with progress instead of sending the user to a terminal.

use std::path::PathBuf;

use clipforge_inference::{DownloadProgress, ModelStore, ModelStoreError};

use crate::config::AppConfig;

/// One weight file the image profile loads — where it lives, how big it is,
/// and whether a local override replaces the download.
#[derive(Clone, Debug, PartialEq)]
pub struct ModelArtifactStatus {
    pub filename: String,
    /// The path the server loads the artifact from (the override path when
    /// one applies).
    pub path: PathBuf,
    /// The artifact's recorded size, in bytes.
    pub size_bytes: u64,
    /// A local file used in place of a download, when the provider overrides
    /// this artifact under `weights`.
    pub local_override: Option<PathBuf>,
}

impl ModelArtifactStatus {
    /// Whether the artifact still needs downloading: no local override and no
    /// file on disk yet.
    pub fn needs_download(&self) -> bool {
        self.local_override.is_none() && !self.path.is_file()
    }

    /// Whether the artifact is in place — an existing override, or a
    /// provisioned file.
    pub fn is_present(&self) -> bool {
        match &self.local_override {
            Some(path) => path.is_file(),
            None => self.path.is_file(),
        }
    }
}

/// The image profile's artifacts, in the order the server loads them.
pub fn image_artifacts(config: &AppConfig) -> Vec<ModelArtifactStatus> {
    config
        .image
        .artifact_plan()
        .iter()
        .map(|entry| ModelArtifactStatus {
            filename: entry.artifact.filename.to_owned(),
            path: entry.path.clone(),
            size_bytes: entry.artifact.size_bytes,
            local_override: entry.local_override.clone(),
        })
        .collect()
}

/// Downloads and verifies every artifact that still needs downloading, in
/// order, reporting progress as it goes. Artifacts the provider overrides
/// with a local file are never fetched.
pub async fn provision_image_model<F>(
    config: &AppConfig,
    progress: F,
) -> Result<(), ModelStoreError>
where
    F: FnMut(DownloadProgress),
{
    let artifacts: Vec<_> = config
        .image
        .artifact_plan()
        .iter()
        .filter(|entry| entry.needs_download())
        .map(|entry| entry.artifact)
        .collect();
    let store = ModelStore::new(config.image.model_root());
    store.ensure_all(&artifacts, progress).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn presence_and_download_reflect_the_override_and_the_disk() {
        let directory =
            std::env::temp_dir().join(format!("svs-provision-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&directory).expect("temp dir");
        let present = directory.join("present.safetensors");
        std::fs::write(&present, b"x").expect("write");

        let missing = ModelArtifactStatus {
            filename: "missing.safetensors".to_owned(),
            path: directory.join("missing.safetensors"),
            size_bytes: 10,
            local_override: None,
        };
        assert!(missing.needs_download());
        assert!(!missing.is_present());

        let provisioned = ModelArtifactStatus {
            path: present.clone(),
            ..missing.clone()
        };
        assert!(!provisioned.needs_download());
        assert!(provisioned.is_present());

        // An override never downloads, and is present only if its file exists.
        let override_missing = ModelArtifactStatus {
            local_override: Some(directory.join("nope.safetensors")),
            ..missing.clone()
        };
        assert!(!override_missing.needs_download());
        assert!(!override_missing.is_present());

        let override_present = ModelArtifactStatus {
            local_override: Some(present.clone()),
            ..missing.clone()
        };
        assert!(override_present.is_present());
        std::fs::remove_dir_all(&directory).ok();
    }
}
