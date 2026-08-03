use std::{
    fs,
    path::{Path, PathBuf},
};

use thiserror::Error;

#[derive(Clone, Debug)]
pub struct ModelStore {
    root: PathBuf,
}

#[derive(Debug, Error)]
pub enum ModelStoreError {
    #[error("failed to create model directory {path}: {source}")]
    CreateDirectory {
        path: String,
        source: std::io::Error,
    },
}

impl ModelStore {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    pub fn prepare(&self) -> Result<(), ModelStoreError> {
        fs::create_dir_all(&self.root).map_err(|source| ModelStoreError::CreateDirectory {
            path: self.root.display().to_string(),
            source,
        })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }
}

// TODO: Add resumable HTTPS downloads, checksums, progress events, and cancellation.
