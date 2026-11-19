//! The editable `.sf` source file. SQLite owns project metadata and artwork,
//! while this document owns exact authored text and its last disk snapshot.
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, OpenOptions},
    io::{self, Read, Write},
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
    crate::project::shared_source_path(project).unwrap_or_else(|| project.with_extension("sf"))
}

/// Resolve a writable include by its exact path so a pending write cannot be
/// redirected by changing a symlink between publication and recovery.
pub fn included_edit_path(entry: &Path, relative: &str) -> Result<PathBuf, StoryFileError> {
    let fail = |source| StoryFileError::Read {
        path: PathBuf::from(relative),
        source,
    };
    if !storyfella_language::valid_source_path(relative)
        || (cfg!(windows) && relative.contains(':'))
    {
        return Err(fail(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Use a project-relative .sf path",
        )));
    }
    let root = entry.parent().ok_or(StoryFileError::Unbound)?;
    let path = root.join(relative);
    if path == entry || path.canonicalize().map_err(fail)? != path {
        return Err(fail(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "Use the include's canonical project-relative path",
        )));
    }
    Ok(path)
}

/// Load the reachable source snapshot without allowing links to escape the
/// entry file's directory. Drafts are keyed by project-relative source path and
/// remain owned by the host; they are never written by this loader.
pub fn load_source_workspace(
    entry: &Path,
    drafts: &std::collections::BTreeMap<String, String>,
) -> Result<storyfella_compiler::SourceWorkspace, storyfella_compiler::WorkspaceError> {
    load_workspace(entry, drafts, None)
}

/// Apply a total byte budget before parsing each reachable source file.
pub fn load_bounded_source_workspace(
    entry: &Path,
    drafts: &BTreeMap<String, String>,
    max_bytes: usize,
) -> Result<storyfella_compiler::SourceWorkspace, storyfella_compiler::WorkspaceError> {
    load_workspace(entry, drafts, Some(max_bytes))
}

fn load_workspace(
    entry: &Path,
    drafts: &BTreeMap<String, String>,
    mut remaining: Option<usize>,
) -> Result<storyfella_compiler::SourceWorkspace, storyfella_compiler::WorkspaceError> {
    use storyfella_compiler::WorkspaceError;
    let fail = |message| {
        WorkspaceError::entry(
            entry
                .file_name()
                .unwrap_or(entry.as_os_str())
                .to_string_lossy()
                .into_owned(),
            message,
        )
    };
    let entry = entry
        .canonicalize()
        .map_err(|error| fail(format!("Cannot open {}: {error}", entry.display())))?;
    let root = entry
        .parent()
        .ok_or_else(|| fail("Story source has no parent folder".into()))?;
    let name = entry
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| fail("Story filename is not UTF-8".into()))?
        .to_owned();
    let mut loaded = BTreeSet::new();
    storyfella_compiler::SourceWorkspace::load_with(name, |relative| {
        if cfg!(windows) && relative.contains(':') {
            return Err(format!("Source path is not valid on Windows: {relative}"));
        }
        let candidate = root.join(relative);
        let draft = drafts.get(relative);
        let canonical = match candidate.canonicalize() {
            Ok(path) => path,
            Err(error) if error.kind() == io::ErrorKind::NotFound && draft.is_some() => {
                if !matches!(fs::symlink_metadata(&candidate), Err(error) if error.kind() == io::ErrorKind::NotFound)
                {
                    return Err(format!("Cannot open {relative}: {error}"));
                }
                let parent = candidate
                    .parent()
                    .expect("source is relative to its workspace");
                parent
                    .canonicalize()
                    .map_err(|error| format!("Cannot open {relative}: {error}"))?
                    .join(
                        candidate
                            .file_name()
                            .expect("valid source path has a filename"),
                    )
            }
            Err(error) => return Err(format!("Cannot open {relative}: {error}")),
        };
        if !canonical.starts_with(root) {
            return Err(format!("Source escapes the project folder: {relative}"));
        }
        if !loaded.insert(canonical.clone()) {
            return Err(format!(
                "Source included more than once through different paths: {relative}"
            ));
        }
        let text = match (draft, remaining) {
            (Some(draft), Some(bytes)) if draft.len() > bytes => {
                return Err("Source workspace exceeds its byte budget".into());
            }
            (Some(draft), _) => draft.clone(),
            (None, Some(bytes)) => {
                let mut source = String::new();
                fs::File::open(&canonical)
                    .and_then(|file| {
                        file.take((bytes as u64).saturating_add(1))
                            .read_to_string(&mut source)
                    })
                    .map_err(|error| format!("Cannot read {relative}: {error}"))?;
                if source.len() > bytes {
                    return Err("Source workspace exceeds its byte budget".into());
                }
                source
            }
            (None, None) => fs::read_to_string(canonical)
                .map_err(|error| format!("Cannot read {relative}: {error}"))?,
        };
        if let Some(remaining) = &mut remaining {
            *remaining -= text.len();
        }
        Ok(text)
    })
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct StoryDocument {
    path: Option<PathBuf>,
    source: String,
    saved_source: String,
    /// The draft has this file. Only an additional source can lack it: one
    /// created and not yet saved, then undone, or one the draft removes.
    exists: bool,
    /// The file was on disk at the last read or write.
    on_disk: bool,
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
            exists: true,
            on_disk: true,
        })
    }

    /// A file the draft does not have yet, to be written at `path` when the
    /// draft that gives it text is saved.
    fn absent(path: PathBuf) -> Self {
        Self {
            path: Some(path),
            ..Self::default()
        }
    }

    /// Whether the draft has this file.
    pub fn exists(&self) -> bool {
        self.exists
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

    /// Gives the draft this text, and so this file.
    pub fn set_source(&mut self, source: String) {
        self.source = source;
        self.exists = self.path.is_some();
    }

    /// Takes this file out of the draft; saving removes it from disk.
    fn remove(&mut self) {
        self.source.clear();
        self.exists = false;
    }

    pub fn is_dirty(&self) -> bool {
        self.exists != self.on_disk || self.source != self.saved_source
    }

    /// Validate the exact source snapshot before committing a catalog/source
    /// pair. The catalog transaction serializes cooperating Studio writers.
    pub(crate) fn check_disk(&self) -> Result<(), StoryFileError> {
        let path = self.path.as_deref().ok_or(StoryFileError::Unbound)?;
        let unchanged = if self.on_disk {
            read(path)? == self.saved_source
        } else {
            !path.exists()
        };
        if !unchanged {
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
    /// A file the draft creates is written new and one it removes is
    /// deleted, each only while the disk still matches the last snapshot.
    pub fn save(&mut self) -> Result<(), StoryFileError> {
        let path = self.path.as_deref().ok_or(StoryFileError::Unbound)?;
        if !self.is_dirty() {
            return if self.on_disk { self.reload() } else { Ok(()) };
        }
        self.check_disk()?;
        match (self.exists, self.on_disk) {
            (true, true) => replace(path, &self.source, &self.saved_source)?,
            (true, false) => write_new(path, &self.source)?,
            (false, _) => fs::remove_file(path).map_err(|source| StoryFileError::Write {
                path: path.to_owned(),
                source,
            })?,
        }
        self.saved_source.clone_from(&self.source);
        self.on_disk = self.exists;
        Ok(())
    }

    /// Explicit reload discards local edits only after a successful read.
    pub fn reload(&mut self) -> Result<(), StoryFileError> {
        let path = self.path.as_deref().ok_or(StoryFileError::Unbound)?;
        let source = read(path)?;
        self.saved_source.clone_from(&source);
        self.source = source;
        self.exists = true;
        self.on_disk = true;
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
            if !document.exists() {
                return Err(StoryFileError::Read {
                    path: relative.into(),
                    source: io::Error::new(
                        io::ErrorKind::NotFound,
                        "This file is not in the draft",
                    ),
                });
            }
            if !document.is_dirty() {
                document.reload()?;
            }
            return Ok(document);
        }
        let candidate = self
            .candidate(relative)
            .map_err(|source| StoryFileError::Read {
                path: relative.into(),
                source,
            })?;
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

    /// `relative` resolved under the root, once it names a `.sf` file
    /// there by a plain relative path.
    fn candidate(&self, relative: &str) -> io::Result<PathBuf> {
        use std::path::Component;
        let path = Path::new(relative);
        if relative.is_empty()
            || !path.components().all(|c| matches!(c, Component::Normal(_)))
            || path.extension().is_none_or(|ext| ext != "sf")
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Enter a project-relative .sf path",
            ));
        }
        Ok(self.root.join(path))
    }

    /// The file at `relative`, while the draft has it.
    pub fn get(&self, relative: &str) -> Option<&StoryDocument> {
        self.documents
            .get(relative)
            .filter(|document| document.exists())
    }

    /// Whether this holds `relative` at all, as a file the draft has or one
    /// it created or removed.
    pub fn tracks(&self, relative: &str) -> bool {
        self.documents.contains_key(relative)
    }

    /// Adds `relative`, which is on no disk yet, as a file the draft does not
    /// have; giving it text creates it, and saving writes it.
    pub fn add_absent(&mut self, relative: &str) -> Result<(), StoryFileError> {
        let fail = |message: &str| StoryFileError::Write {
            path: relative.into(),
            source: io::Error::new(io::ErrorKind::InvalidInput, message.to_owned()),
        };
        if self.documents.contains_key(relative) {
            return Err(fail("This file is already part of the story"));
        }
        if !storyfella_language::valid_source_path(relative) {
            return Err(fail("Source path contains unsupported characters"));
        }
        let candidate = self
            .candidate(relative)
            .map_err(|source| StoryFileError::Write {
                path: relative.into(),
                source,
            })?;
        let parent = candidate
            .parent()
            .ok_or_else(|| fail("Choose a folder inside this project"))?
            .canonicalize()
            .map_err(|source| StoryFileError::Write {
                path: candidate.clone(),
                source,
            })?;
        if !parent.starts_with(&self.root) {
            return Err(fail("Choose a folder inside this project"));
        }
        let path = parent.join(candidate.file_name().ok_or_else(|| fail("Name the file"))?);
        if fs::symlink_metadata(&path).is_ok() {
            return Err(fail("A file with this name already exists"));
        }
        self.documents
            .insert(relative.to_owned(), StoryDocument::absent(path));
        Ok(())
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
        if !storyfella_language::valid_source_path(&relative) {
            return Err(fail("Source path contains unsupported characters"));
        }
        write_new(&destination, "")?;
        self.open(&relative)?;
        Ok(relative)
    }

    /// Gives `relative` this text in the draft, creating it if the draft
    /// did not have it.
    pub fn set_source(&mut self, relative: &str, source: String) -> Result<(), StoryFileError> {
        self.documents
            .get_mut(relative)
            .ok_or(StoryFileError::Unbound)?
            .set_source(source);
        Ok(())
    }

    /// Takes `relative` out of the draft. Saving deletes it from disk.
    pub fn remove(&mut self, relative: &str) -> Result<(), StoryFileError> {
        self.documents
            .get_mut(relative)
            .ok_or(StoryFileError::Unbound)?
            .remove();
        Ok(())
    }

    /// Every file this holds, with its text, or `None` when the draft does
    /// not have it.
    pub fn drafts(&self) -> BTreeMap<String, Option<String>> {
        self.documents
            .iter()
            .map(|(path, document)| {
                (
                    path.clone(),
                    document.exists().then(|| document.source().to_owned()),
                )
            })
            .collect()
    }

    /// Only unsaved documents override disk when building a fresh snapshot.
    pub fn overrides(&self) -> BTreeMap<String, String> {
        self.documents
            .iter()
            .filter(|(_, document)| document.exists() && document.is_dirty())
            .map(|(path, document)| (path.clone(), document.source().to_owned()))
            .collect()
    }

    /// Restore editor text, and which files the draft has, without replacing
    /// the disk baselines used for conflict detection. Files opened after the
    /// snapshot stay unchanged.
    pub fn restore_drafts(&mut self, drafts: &BTreeMap<String, Option<String>>) {
        for (path, source) in drafts {
            if let Some(document) = self.documents.get_mut(path) {
                match source {
                    Some(source) => document.set_source(source.clone()),
                    None => document.remove(),
                }
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
