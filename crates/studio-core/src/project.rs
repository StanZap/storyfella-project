//! Portable, application-owned project folders. In-memory image paths remain
//! absolute; only the shared folder's database stores relative resource paths.
use std::{
    fs,
    io::Write,
    path::{Component, Path, PathBuf},
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

impl ProjectFolder {
    /// Create a new folder; never merge with or replace an existing directory.
    pub fn create(
        root: &Path,
        name: &str,
        registry: &ArtifactRegistry,
        source: &str,
    ) -> Result<Self> {
        Self::write_new(root, name, None, registry, source)
    }

    /// Copy an existing saved project into a portable folder, retaining identities.
    /// The original database/source/images are never modified or removed.
    pub fn import(root: &Path, stored: &StoredProject) -> Result<Self> {
        ensure!(
            !stored.story.path().is_some_and(|path| path
                .parent()
                .is_some_and(|parent| parent.join(MANIFEST).exists())),
            "This project already uses a shared folder; move or copy the complete folder instead"
        );
        let mut registry = stored.registry.clone();
        for artifact in &mut registry.artifacts {
            for revision in &mut artifact.revisions {
                if let Some(value) = &mut revision.asset_path {
                    if Path::new(value).is_relative() {
                        let parent = stored
                            .story
                            .path()
                            .and_then(Path::parent)
                            .context("Original project source has no folder")?;
                        *value = parent
                            .join(&*value)
                            .to_str()
                            .context("Artwork path is not UTF-8")?
                            .to_owned();
                    }
                }
            }
        }
        Self::write_new(
            root,
            &stored.name,
            Some(stored.id),
            &registry,
            stored.story.source(),
        )
    }

    fn write_new(
        root: &Path,
        name: &str,
        id: Option<Uuid>,
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
        if let Some(id) = id {
            db.set_project_id(id)?;
        }
        db.rename_project(name)?;
        let id = db.load()?.id;
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
        let manifest = toml::to_string_pretty(&Manifest {
            version: 1,
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
        let manifest: Manifest = toml::from_str(&fs::read_to_string(root.join(MANIFEST))?)?;
        ensure!(
            manifest.version == 1,
            "Unsupported Storyfella project version {}",
            manifest.version
        );
        let folder = Self {
            root,
            id: manifest.project_id,
        };
        ensure!(folder.database().is_file(), "Project database is missing");
        folder.resolve(DATABASE)?;
        folder.resolve("story.sf")?;
        folder.resolve("artwork")?;
        folder.resolve("atelier")?;
        ensure!(
            folder.root.join("story.sf").is_file(),
            "Story source is missing"
        );
        // Do not create a missing database or export an empty replacement source.
        ensure!(
            ProjectDb::open(folder.database())?.load()?.id == folder.id,
            "Manifest and database project identities differ"
        );
        Ok(folder)
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
            let root = parent.canonicalize()?;
            let manifest: Manifest = toml::from_str(&fs::read_to_string(root.join(MANIFEST))?)?;
            ensure!(
                manifest.version == 1,
                "Unsupported Storyfella project version {}",
                manifest.version
            );
            return Ok(Some(ProjectFolder {
                root,
                id: manifest.project_id,
            }));
        }
    }
    Ok(None)
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
