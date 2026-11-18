//! Portable, application-owned project folders. In-memory image paths remain
//! absolute; only the shared folder's database stores relative resource paths.
use std::{
    fs::{self, File, OpenOptions},
    io::{self, Write},
    path::{Component, Path, PathBuf},
    sync::Arc,
};

use anyhow::{bail, ensure, Context, Result};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{
    persistence::{ProjectDb, StoredProject},
    registry::ArtifactRegistry,
};

pub const MANIFEST: &str = "storyfella.toml";
const DATABASE: &str = "story.db";
const ASSETS_SOURCE: &str = "assets/story.sf";
const CURRENT_VERSION: u32 = 2;
const AUTHORING_LOCK: &str = ".storyfella-studio.lock";

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    version: u32,
    project_id: Uuid,
}

#[derive(Clone, Debug)]
pub struct ProjectFolder {
    root: PathBuf,
    id: Uuid,
}

/// Retained by Studio while the project owns editable Writer buffers.
#[derive(Clone, Debug)]
pub struct AuthoringLease {
    root: PathBuf,
    project_id: Uuid,
    epoch: Uuid,
    lock: Arc<File>,
}

impl PartialEq for AuthoringLease {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.lock, &other.lock)
    }
}

impl AuthoringLease {
    pub fn epoch(&self) -> Uuid {
        self.epoch
    }

    pub fn belongs_to(&self, folder: &ProjectFolder) -> bool {
        self.root == folder.root && self.project_id == folder.id
    }

    /// Fail if the project was moved, replaced, or the lock file was removed
    /// after the lease was acquired. A copied project can retain its UUID.
    pub fn verify(&self) -> Result<()> {
        let current = ProjectFolder::locate(&self.root)?;
        ensure!(
            self.belongs_to(&current)
                && same_file::Handle::from_file(self.lock.try_clone()?)?
                    == same_file::Handle::from_path(self.root.join(AUTHORING_LOCK))?,
            "The shared project moved or was replaced. Reopen the project to continue."
        );
        Ok(())
    }
}

impl ProjectFolder {
    /// Create a new folder; never merge with or replace an existing directory.
    pub fn create(
        root: &Path,
        name: &str,
        registry: &ArtifactRegistry,
        source: &str,
    ) -> Result<Self> {
        ensure!(!name.trim().is_empty(), "Enter a project name");
        fs::create_dir(root)
            .with_context(|| format!("Create new project folder {}", root.display()))?;
        // The folder itself is reserved atomically. Publish the manifest last:
        // an interrupted/failed copy is never advertised as a complete project.
        let root = root.canonicalize()?;
        let db = ProjectDb::open(root.join(DATABASE))?;
        db.rename_project(name)?;
        let id = db.load()?.id;
        fs::create_dir(root.join("assets"))?;
        fs::create_dir(root.join("artwork"))?;
        fs::create_dir(root.join("atelier"))?;
        let folder = Self { root, id };
        let mut registry = registry.clone();
        crate::catalog::reconcile_declarations(&mut registry, source, false);
        let catalog = crate::authoring::AssetCatalog::from_registry(&registry);
        let analysis = storyfella_language::analyze(source, &catalog.assets);
        crate::authoring::reconcile(&mut registry, &analysis);
        let portable = folder.store_images(&registry)?;
        db.save_registry(&portable)?;
        let mut story = db.load()?.story;
        story.set_source(source.to_owned());
        story.save()?;
        fs::rename(
            folder.root.join("story.sf"),
            folder.root.join(ASSETS_SOURCE),
        )?;
        let manifest = toml::to_string_pretty(&Manifest {
            version: CURRENT_VERSION,
            project_id: id,
        })?;
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(folder.manifest())?;
        file.write_all(manifest.as_bytes())?;
        file.sync_all()?;
        Ok(folder)
    }

    pub fn open(path: &Path) -> Result<Self> {
        let folder = Self::locate(path)?;
        folder.load()?;
        Ok(folder)
    }

    /// Resolve Studio's shared-project entry points before loading SQLite,
    /// which can migrate storage and recover a pending authoring write.
    pub fn locate_for_authoring(path: &Path) -> Result<Option<Self>> {
        let path = path.canonicalize()?;
        if path.is_dir() || path.file_name().is_some_and(|name| name == MANIFEST) {
            return Self::locate(&path).map(Some);
        }
        let path = if path.extension().is_some_and(|ext| ext == "sf") {
            if let Some(database) = database_for_source(&path)? {
                database
            } else {
                let database = path.with_extension("db");
                if !database.is_file() {
                    return Ok(None);
                }
                database.canonicalize()?
            }
        } else {
            path
        };
        let folder = for_database(&path)?;
        if let Some(folder) = &folder {
            folder.check_layout()?;
        }
        Ok(folder)
    }

    fn locate(path: &Path) -> Result<Self> {
        let root = if path.is_dir() {
            path
        } else {
            ensure!(
                path.file_name().is_some_and(|name| name == MANIFEST),
                "Open a Storyfella folder or its {MANIFEST} manifest"
            );
            path.parent().context("Manifest has no project directory")?
        }
        .canonicalize()?;
        let folder = Self::read_manifest(root)?;
        folder.check_layout()?;
        Ok(folder)
    }

    fn check_layout(&self) -> Result<()> {
        ensure!(self.database().is_file(), "Project database is missing");
        self.resolve(DATABASE)?;
        self.resolve(ASSETS_SOURCE)?;
        self.resolve("artwork")?;
        self.resolve("atelier")?;
        ensure!(
            self.root.join(ASSETS_SOURCE).is_file(),
            "Story source is missing"
        );
        Ok(())
    }

    pub fn load(&self) -> Result<StoredProject> {
        let stored = ProjectDb::open_existing(self.database())?.load()?;
        ensure!(
            stored.id == self.id,
            "Manifest and database project identities differ"
        );
        Ok(stored)
    }

    pub fn claim_authoring(&self) -> Result<AuthoringLease> {
        let path = self.root.join(AUTHORING_LOCK);
        let mut options = OpenOptions::new();
        options.read(true).write(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let file = match options.clone().create_new(true).open(&path) {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                ensure!(
                    fs::symlink_metadata(&path)?.is_file(),
                    "Invalid Studio lock file"
                );
                options.open(&path)?
            }
            Err(error) => return Err(error.into()),
        };
        match file.try_lock().map_err(io::Error::from) {
            Ok(()) => Ok(AuthoringLease {
                root: self.root.clone(),
                project_id: self.id,
                epoch: Uuid::new_v4(),
                lock: Arc::new(file),
            }),
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                bail!("This project is open for editing in another Studio window or MCP session. Close it there, then try again.");
            }
            Err(error) => Err(error).context("Could not claim the Studio project lock"),
        }
    }

    fn read_manifest(root: PathBuf) -> Result<Self> {
        let manifest: Manifest = toml::from_str(&fs::read_to_string(root.join(MANIFEST))?)?;
        ensure!(
            manifest.version == CURRENT_VERSION,
            "Unsupported Storyfella project version {}",
            manifest.version
        );
        Ok(Self {
            root,
            id: manifest.project_id,
        })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }
    pub fn id(&self) -> Uuid {
        self.id
    }
    pub fn manifest(&self) -> PathBuf {
        self.root.join(MANIFEST)
    }
    pub fn database(&self) -> PathBuf {
        self.root.join(DATABASE)
    }
    pub fn atelier_document(&self) -> Result<PathBuf> {
        self.resolve("atelier/main.atelier")
    }

    /// Import immutable revision images before committing database references.
    /// Existing destinations must contain identical bytes, never newer artwork.
    fn store_images(&self, registry: &ArtifactRegistry) -> Result<ArtifactRegistry> {
        let mut portable = registry.clone();
        for artifact in &mut portable.artifacts {
            for revision in &mut artifact.revisions {
                let Some(value) = &revision.asset_path else {
                    continue;
                };
                let input = Path::new(value);
                let input = if input.is_absolute() {
                    input.to_owned()
                } else {
                    self.resolve(value)?
                };
                let extension = input
                    .extension()
                    .and_then(|s| s.to_str())
                    .filter(|s| s.chars().all(|c| c.is_ascii_alphanumeric()))
                    .unwrap_or("png");
                let relative = format!("artwork/{}.{}", revision.id, extension);
                let output = self.resolve(&relative)?;
                if input == output && output.is_file() {
                    revision.asset_path = Some(relative);
                    continue;
                }
                let bytes = fs::read(&input)
                    .with_context(|| format!("Read artwork {}", input.display()))?;
                if output.exists() {
                    ensure!(
                        fs::read(&output)? == bytes,
                        "Artwork revision {} has conflicting contents",
                        revision.id
                    );
                } else {
                    let mut file = fs::OpenOptions::new()
                        .write(true)
                        .create_new(true)
                        .open(&output)?;
                    file.write_all(&bytes)?;
                    file.sync_all()?;
                }
                revision.asset_path = Some(relative);
            }
        }
        Ok(portable)
    }

    pub(crate) fn resolve(&self, value: &str) -> Result<PathBuf> {
        let path = Path::new(value);
        ensure!(
            !value.is_empty() && path.components().all(|c| matches!(c, Component::Normal(_))),
            "Invalid project resource path: {value}"
        );
        let joined = self.root.join(path);
        if joined.exists() {
            ensure!(
                joined.canonicalize()?.starts_with(&self.root),
                "Project resource escapes its folder: {value}"
            );
        } else if let Some(parent) = joined.parent() {
            ensure!(
                parent.canonicalize()?.starts_with(&self.root),
                "Project resource parent escapes its folder: {value}"
            );
        }
        Ok(joined)
    }
}

/// Recognize only the fixed shared database location. Standalone databases keep
/// their established behavior and are adopted through an explicit copy.
pub(crate) fn for_database(path: &Path) -> Result<Option<ProjectFolder>> {
    if path.file_name().is_some_and(|name| name == DATABASE) {
        if let Some(parent) = path.parent().filter(|p| p.join(MANIFEST).exists()) {
            return ProjectFolder::read_manifest(parent.canonicalize()?).map(Some);
        }
    }
    Ok(None)
}

/// Resolve a shared entry source back to its database without confusing an
/// included chapter with the entry point.
pub fn database_for_source(source: &Path) -> Result<Option<PathBuf>> {
    let source = source.canonicalize()?;
    let Some(parent) = source
        .parent()
        .filter(|_| source.file_name().is_some_and(|name| name == "story.sf"))
    else {
        return Ok(None);
    };
    let root = parent
        .parent()
        .filter(|root| parent.ends_with("assets") && root.join(MANIFEST).exists());
    let Some(root) = root else { return Ok(None) };
    let folder = ProjectFolder::read_manifest(root.to_owned())?;
    ensure!(
        folder.resolve(ASSETS_SOURCE)? == source,
        "Open the project's entry story or manifest instead"
    );
    let database = folder.database();
    ensure!(database.is_file(), "Project database is missing");
    Ok(Some(database))
}

pub(crate) fn shared_source_path(database: &Path) -> Option<PathBuf> {
    let folder = for_database(database).ok()??;
    Some(folder.root.join(ASSETS_SOURCE))
}

pub(crate) fn prepare_registry(
    path: &Path,
    registry: &ArtifactRegistry,
) -> Result<ArtifactRegistry> {
    match for_database(path)? {
        Some(folder) => folder.store_images(registry),
        None => Ok(registry.clone()),
    }
}

pub(crate) fn resolve_registry(
    path: &Path,
    id: Uuid,
    registry: &mut ArtifactRegistry,
) -> Result<()> {
    if let Some(folder) = for_database(path)? {
        ensure!(
            folder.id == id,
            "Manifest and database project identities differ"
        );
        for artifact in &mut registry.artifacts {
            for revision in &mut artifact.revisions {
                if let Some(value) = &mut revision.asset_path {
                    if Path::new(value).is_absolute() {
                        bail!("Shared project contains a nonportable artwork path: {value}");
                    }
                    *value = folder
                        .resolve(value)?
                        .to_str()
                        .context("Artwork path is not UTF-8")?
                        .to_owned();
                }
            }
        }
    }
    Ok(())
}
