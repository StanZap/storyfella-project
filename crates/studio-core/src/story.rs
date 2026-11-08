//! The editable `.sf` source file. SQLite owns project metadata and artwork,
//! while this document owns exact authored text and its last disk snapshot.
use std::{
    collections::BTreeMap,
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

/// List source files for the project explorer without following symlinks.
pub fn list_source_files(entry: &Path) -> io::Result<Vec<String>> {
    let entry = entry.canonicalize()?;
    let root = entry
        .parent()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "Story has no folder"))?;
    let mut pending = vec![root.to_owned()];
    let mut files = Vec::new();
    let mut directories = 0usize;
    while let Some(directory) = pending.pop() {
        directories += 1;
        if directories > 1024 {
            return Err(io::Error::other("Too many project folders"));
        }
        for item in fs::read_dir(directory)? {
            let item = item?;
            let kind = item.file_type()?;
            if kind.is_dir() && !item.file_name().to_string_lossy().starts_with('.') {
                pending.push(item.path());
            } else if kind.is_file() && item.path().extension().is_some_and(|ext| ext == "sf") {
                let path = item.path();
                let relative = path.strip_prefix(root).map_err(io::Error::other)?;
                let name = relative.to_str().ok_or_else(|| {
                    io::Error::new(io::ErrorKind::InvalidData, "Source path is not UTF-8")
                })?;
                files.push(name.replace(std::path::MAIN_SEPARATOR, "/"));
                if files.len() > 4096 {
                    return Err(io::Error::other("Too many story files"));
                }
            }
        }
    }
    files.sort();
    Ok(files)
}

/// Load the reachable source snapshot without allowing links to escape the
/// entry file's directory. Drafts are keyed by project-relative source path and
/// remain owned by the host; they are never written by this loader.
pub fn load_source_workspace(
    entry: &Path,
    drafts: &std::collections::BTreeMap<String, String>,
) -> Result<storyfella_compiler::SourceWorkspace, String> {
    let entry = entry
        .canonicalize()
        .map_err(|error| format!("Cannot open {}: {error}", entry.display()))?;
    let root = entry.parent().ok_or("Story source has no parent folder")?;
    let name = entry
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or("Story filename is not UTF-8")?
        .to_owned();
    storyfella_compiler::SourceWorkspace::load_with(name.clone(), |relative| {
        if let Some(draft) = drafts.get(relative) {
            return Ok(draft.clone());
        }
        let candidate = root.join(relative);
        let canonical = candidate
            .canonicalize()
            .map_err(|error| format!("Cannot open {relative}: {error}"))?;
        if !canonical.starts_with(root) {
            return Err(format!("Source escapes the project folder: {relative}"));
        }
        fs::read_to_string(canonical).map_err(|error| format!("Cannot read {relative}: {error}"))
    })
    .map_err(|error| format!("{}:{}: {}", error.path, error.span.start, error.message))
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

    /// Last successfully read or written text, for guarded session recovery.
    pub fn saved_source(&self) -> &str {
        &self.saved_source
    }

    pub fn set_source(&mut self, source: String) {
        self.source = source;
    }

    pub fn is_dirty(&self) -> bool {
        self.source != self.saved_source
    }

    /// Validate the exact source snapshot before committing a catalog/source
    /// pair. The catalog transaction serializes cooperating Studio writers.
    pub(crate) fn check_disk(&self) -> Result<(), StoryFileError> {
        let path = self.path.as_deref().ok_or(StoryFileError::Unbound)?;
        if read(path)? != self.saved_source {
            return Err(StoryFileError::Conflict {
                path: path.to_owned(),
            });
        }
        Ok(())
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

/// Open non-entry source files and retain each file's own saved snapshot.
/// The entry file continues to participate in the database's authoring save.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AdditionalSources {
    root: PathBuf,
    entry: PathBuf,
    documents: BTreeMap<String, StoryDocument>,
}

impl AdditionalSources {
    pub fn new(entry: &Path) -> Result<Self, StoryFileError> {
        let entry = entry
            .canonicalize()
            .map_err(|source| StoryFileError::Read {
                path: entry.to_owned(),
                source,
            })?;
        let root = entry.parent().ok_or(StoryFileError::Unbound)?.to_owned();
        Ok(Self {
            root,
            entry,
            documents: BTreeMap::new(),
        })
    }

    /// Open a project-relative `.sf` source. Symlinks may not escape the root.
    pub fn open(&mut self, relative: &str) -> Result<&StoryDocument, StoryFileError> {
        if self.documents.contains_key(relative) {
            let document = self
                .documents
                .get_mut(relative)
                .ok_or(StoryFileError::Unbound)?;
            if !document.is_dirty() {
                document.reload()?;
            }
            return Ok(document);
        }
        use std::path::Component;
        let path = Path::new(relative);
        if relative.is_empty()
            || !path.components().all(|c| matches!(c, Component::Normal(_)))
            || path.extension().is_none_or(|ext| ext != "sf")
        {
            return Err(StoryFileError::Read {
                path: path.to_owned(),
                source: io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "Enter a project-relative .sf path",
                ),
            });
        }
        let candidate = self.root.join(path);
        let canonical = candidate
            .canonicalize()
            .map_err(|source| StoryFileError::Read {
                path: candidate.clone(),
                source,
            })?;
        if !canonical.starts_with(&self.root) || canonical == self.entry {
            return Err(StoryFileError::Read {
                path: candidate,
                source: io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "Source must be inside this project and distinct from the entry file",
                ),
            });
        }
        if self
            .documents
            .values()
            .any(|document| document.path() == Some(canonical.as_path()))
        {
            return Err(StoryFileError::Read {
                path: candidate,
                source: io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "This file is already open under another path",
                ),
            });
        }
        self.documents
            .insert(relative.to_owned(), StoryDocument::open(&canonical)?);
        Ok(&self.documents[relative])
    }

    pub fn get(&self, relative: &str) -> Option<&StoryDocument> {
        self.documents.get(relative)
    }

    /// Create an empty source without replacing any existing file.
    pub fn create(&mut self, path: &Path) -> Result<String, StoryFileError> {
        let fail = |message: &str| StoryFileError::Write {
            path: path.to_owned(),
            source: io::Error::new(io::ErrorKind::InvalidInput, message),
        };
        let parent = path
            .parent()
            .ok_or_else(|| fail("Choose a folder inside this project"))?
            .canonicalize()
            .map_err(|source| StoryFileError::Write {
                path: path.to_owned(),
                source,
            })?;
        if !parent.starts_with(&self.root) {
            return Err(fail("Choose a folder inside this project"));
        }
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| fail("Source filename must be UTF-8"))?;
        let destination = parent.join(name);
        if destination
            .extension()
            .is_none_or(|extension| extension != "sf")
        {
            return Err(fail("Use the .sf extension"));
        }
        let relative = destination
            .strip_prefix(&self.root)
            .map_err(|_| fail("Choose a folder inside this project"))?
            .to_str()
            .ok_or_else(|| fail("Source path must be UTF-8"))?
            .replace(std::path::MAIN_SEPARATOR, "/");
        if relative.contains(['"', '\n', '\r', '\\']) {
            return Err(fail("Source path contains unsupported characters"));
        }
        write_new(&destination, "")?;
        self.open(&relative)?;
        Ok(relative)
    }

    pub fn set_source(&mut self, relative: &str, source: String) -> Result<(), StoryFileError> {
        self.documents
            .get_mut(relative)
            .ok_or(StoryFileError::Unbound)?
            .set_source(source);
        Ok(())
    }

    pub fn drafts(&self) -> BTreeMap<String, String> {
        self.documents
            .iter()
            .map(|(path, document)| (path.clone(), document.source().to_owned()))
            .collect()
    }

    /// Only unsaved documents override disk when building a fresh snapshot.
    pub fn overrides(&self) -> BTreeMap<String, String> {
        self.documents
            .iter()
            .filter(|(_, document)| document.is_dirty())
            .map(|(path, document)| (path.clone(), document.source().to_owned()))
            .collect()
    }

    /// Restore editor text without replacing the disk baselines used for
    /// conflict detection. Files opened after the snapshot stay unchanged.
    pub fn restore_drafts(&mut self, drafts: &BTreeMap<String, String>) {
        for (path, source) in drafts {
            if let Some(document) = self.documents.get_mut(path) {
                document.set_source(source.clone());
            }
        }
    }

    pub fn is_dirty(&self) -> bool {
        self.documents.values().any(StoryDocument::is_dirty)
    }

    /// Check every dirty file before writing any. If a later write fails,
    /// successfully saved documents retain their new baselines for a retry.
    pub fn save(&mut self) -> Result<(), StoryFileError> {
        for document in self
            .documents
            .values()
            .filter(|document| document.is_dirty())
        {
            document.check_disk()?;
        }
        for document in self
            .documents
            .values_mut()
            .filter(|document| document.is_dirty())
        {
            document.save()?;
        }
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
