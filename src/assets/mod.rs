use std::path::{Path, PathBuf};

#[derive(Clone, Debug)]
pub struct AssetCatalog {
    root: PathBuf,
}

impl AssetCatalog {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }
}

// TODO: Add import, thumbnail, and generated-asset indexing.
