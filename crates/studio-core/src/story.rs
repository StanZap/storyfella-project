//! The editable `.sf` source file. SQLite owns project metadata and artwork,
//! while this document owns exact authored text and its last disk snapshot.
use std::{
    fs::{self, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
};

use thiserror::Error;
use uuid::Uuid;

use crate::errors::{CodedError, ErrorCode};

#[derive(Debug, Error)]
pub enum StoryFileError {
    #[error("could not read story file {path}: {source}")]
    Read { path: PathBuf, source: io::Error },
    #[error("could not write story file {path}: {source}")]
    Write { path: PathBuf, source: io::Error },
    #[error("story file {path} changed on disk; the editor's text has been preserved")]
    Conflict { path: PathBuf },
    #[error("create a project before saving this story draft")]
    Unbound,
}

impl CodedError for StoryFileError {
    fn code(&self) -> ErrorCode {
        match self {
            Self::Read { .. } => ErrorCode::StoryRead,
            Self::Write { .. } => ErrorCode::StoryWrite,
            Self::Conflict { .. } => ErrorCode::StoryConflict,
            Self::Unbound => ErrorCode::StoryUnbound,
        }
    }
}

/// The `.sf` file beside a project database; the pair can move together.
pub fn source_path(project: &Path) -> PathBuf {
    project.with_extension("sf")
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct StoryDocument {
    path: Option<PathBuf>,
    source: String,
    saved_source: String,
}

impl StoryDocument {
    pub fn open(path: &Path) -> Result<Self, StoryFileError> {
        let canonical = path.canonicalize().map_err(|source| StoryFileError::Read {
            path: path.to_owned(),
            source,
        })?;
        let source = read(&canonical)?;
        Ok(Self {
            path: Some(canonical),
            saved_source: source.clone(),
            source,
        })
    }

    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    pub fn source(&self) -> &str {
        &self.source
    }

    pub fn set_source(&mut self, source: String) {
        self.source = source;
    }

    pub fn is_dirty(&self) -> bool {
        self.source != self.saved_source
    }

    /// Preserve a draft before resolving a disk conflict, without changing
    /// this document's path or saved snapshot. Never overwrite another file.
    pub fn save_copy(&self, path: &Path) -> Result<(), StoryFileError> {
        write_new(path, &self.source)
    }

    /// A clean editor incorporates external edits; a dirty one must still
    /// match its last disk snapshot before replacing the file.
    pub fn save(&mut self) -> Result<(), StoryFileError> {
        let path = self.path.as_deref().ok_or(StoryFileError::Unbound)?;
        if !self.is_dirty() {
            return self.reload();
        }
        if read(path)? != self.saved_source {
            return Err(StoryFileError::Conflict {
                path: path.to_owned(),
            });
        }
        replace(path, &self.source, &self.saved_source)?;
        self.saved_source.clone_from(&self.source);
        Ok(())
    }

    /// Explicit reload discards local edits only after a successful read.
    pub fn reload(&mut self) -> Result<(), StoryFileError> {
        let path = self.path.as_deref().ok_or(StoryFileError::Unbound)?;
        let source = read(path)?;
        self.saved_source.clone_from(&source);
        self.source = source;
        Ok(())
    }
}

fn read(path: &Path) -> Result<String, StoryFileError> {
    fs::read_to_string(path).map_err(|source| StoryFileError::Read {
        path: path.to_owned(),
        source,
    })
}

/// Export the v8 Writer draft before removing its SQLite column. An existing
/// different file blocks migration rather than choosing one copy or losing text.
pub(crate) fn export_draft(path: &Path, source: &str) -> Result<(), StoryFileError> {
    match fs::read_to_string(path) {
        Ok(existing) if source.is_empty() || existing == source => Ok(()),
        Ok(_) => Err(StoryFileError::Conflict {
            path: path.to_owned(),
        }),
        Err(error) if error.kind() == io::ErrorKind::NotFound => write_new(path, source),
        Err(source) => Err(StoryFileError::Read {
            path: path.to_owned(),
            source,
        }),
    }
}

fn write_new(path: &Path, source: &str) -> Result<(), StoryFileError> {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|source| StoryFileError::Write {
            path: path.to_owned(),
            source,
        })?;
    let result = file
        .write_all(source.as_bytes())
        .and_then(|_| file.sync_all());
    drop(file);
    if result.is_err() {
        // Only this function created the file. A failed export keeps the
        // SQLite draft and must not leave a partial file blocking the retry.
        let _ = fs::remove_file(path);
    }
    result.map_err(|source| StoryFileError::Write {
        path: path.to_owned(),
        source,
    })
}

/// Stage in the same directory so rename replaces a complete file atomically.
/// The temporary file is never used as an alternate document format.
fn replace(path: &Path, source: &str, expected: &str) -> Result<(), StoryFileError> {
    let temporary = path.with_extension(format!("sf-{}.tmp", Uuid::new_v4()));
    let mut created = false;
    let staged = (|| -> io::Result<()> {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)?;
        created = true;
        file.write_all(source.as_bytes())?;
        file.sync_all()?;
        file.set_permissions(fs::metadata(path)?.permissions())?;
        drop(file);
        Ok(())
    })();
    // Check again after writing the staging file, immediately before publish.
    let result = match staged {
        Err(source) => Err(StoryFileError::Write {
            path: path.to_owned(),
            source,
        }),
        Ok(()) => match read(path) {
            Ok(current) if current == expected => {
                fs::rename(&temporary, path).map_err(|source| StoryFileError::Write {
                    path: path.to_owned(),
                    source,
                })
            }
            Ok(_) => Err(StoryFileError::Conflict {
                path: path.to_owned(),
            }),
            Err(error) => Err(error),
        },
    };
    if result.is_err() && created {
        let _ = fs::remove_file(&temporary);
    }
    result
}
