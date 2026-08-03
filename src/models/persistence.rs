use std::{fs, path::Path};

use thiserror::Error;

use super::Project;

pub struct ProjectStore;

#[derive(Debug, Error)]
pub enum PersistenceError {
    #[error("failed to read project {path}: {source}")]
    Read {
        path: String,
        source: std::io::Error,
    },
    #[error("failed to write project {path}: {source}")]
    Write {
        path: String,
        source: std::io::Error,
    },
    #[error("failed to parse project {path}: {source}")]
    Parse {
        path: String,
        source: toml::de::Error,
    },
    #[error("failed to serialize project: {0}")]
    Serialize(#[source] toml::ser::Error),
}

impl ProjectStore {
    pub fn load(path: impl AsRef<Path>) -> Result<Project, PersistenceError> {
        let path = path.as_ref();
        let contents = fs::read_to_string(path).map_err(|source| PersistenceError::Read {
            path: path.display().to_string(),
            source,
        })?;
        toml::from_str(&contents).map_err(|source| PersistenceError::Parse {
            path: path.display().to_string(),
            source,
        })
    }

    pub fn save(project: &Project, path: impl AsRef<Path>) -> Result<(), PersistenceError> {
        let path = path.as_ref();
        let contents = toml::to_string_pretty(project).map_err(PersistenceError::Serialize)?;
        fs::write(path, contents).map_err(|source| PersistenceError::Write {
            path: path.display().to_string(),
            source,
        })
    }
}
