//! Shared catalog edits operate on existing registry identities. They never
//! render images, edit authored source or create visual working documents.
use std::path::PathBuf;

use anyhow::{ensure, Context, Result};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{
    persistence::{ProjectDb, StoredProject},
    registry::{ArtifactKind, ArtifactRegistry, CreateArtifact, RevisionOrigin},
};

pub mod sync;

/// Full-resolution accepted canvas bytes. The displayed revision is the write
/// guard; metadata changes elsewhere need not prevent an artwork-only save.
pub struct SavedCanvas {
    pub asset_id: Uuid,
    pub canvas_id: u32,
    pub expected_revision: Option<Uuid>,
    pub png: Vec<u8>,
}

#[derive(Clone, Debug)]
pub struct CanvasPublication {
    pub asset_id: Uuid,
    pub canvas_id: u32,
    pub expected_revision: Option<Uuid>,
    pub revision_id: Uuid,
}

pub fn save_canvases(
    folder: &crate::project::ProjectFolder,
    canvases: &[SavedCanvas],
) -> Result<Vec<CanvasPublication>> {
    use sha2::{Digest, Sha256};
    use std::fmt::Write as _;
    use std::io::Write;
    let db = ProjectDb::open_existing(folder.database())?;
    let stored = db.load()?;
    ensure!(
        stored.id == folder.id(),
        "The artwork belongs to another project"
    );
    let mut registry = stored.registry.clone();
    let mut publications = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for canvas in canvases {
        ensure!(
            seen.insert(canvas.asset_id),
            "More than one artboard is linked to the same asset. Keep one link before saving"
        );
        image::load_from_memory_with_format(&canvas.png, image::ImageFormat::Png)
            .context("Invalid saved canvas PNG")?;
        let mut signature = format!("Atelier canvas {} sha256:", canvas.canvas_id);
        for byte in Sha256::digest(&canvas.png) {
            write!(&mut signature, "{byte:02x}")?;
        }
        let item = registry
            .artifact(canvas.asset_id)
            .context("A linked asset no longer exists; your canvas document is saved")?;
        let active = item.active_revision_id;
        let unchanged = item.revisions.iter().find(|revision| {
            Some(revision.id) == active && revision.model.as_deref() == Some(&signature)
        });
        let revision_id = if let Some(revision) = unchanged {
            revision.id
        } else {
            ensure!(active == canvas.expected_revision, "Saved artwork changed in another editor. Your canvas document is saved. Review the other image and relink the artboard in Assets before retrying Save");
            let revision = registry.start_revision(
                canvas.asset_id,
                "Saved Atelier artboard".into(),
                None,
                Some(signature),
                RevisionOrigin::Canon,
            )?;
            let path = folder.resolve(&format!("artwork/{revision}.png"))?;
            let mut file = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path)?;
            file.write_all(&canvas.png)?;
            file.sync_all()?;
            registry.finish_revision(
                canvas.asset_id,
                revision,
                path.to_str()
                    .context("Artwork path is not UTF-8")?
                    .to_owned(),
            )?;
            revision
        };
        publications.push(CanvasPublication {
            asset_id: canvas.asset_id,
            canvas_id: canvas.canvas_id,
            expected_revision: canvas.expected_revision,
            revision_id,
        });
    }
    if registry != stored.registry {
        db.save_registry_checked(&registry, stored.registry_version)?;
    }
    Ok(publications)
}

#[derive(Clone, Copy, Debug)]
pub enum EditResolution {
    Check,
    KeepDraft { saved_version: i64 },
}

#[derive(Clone, Debug, thiserror::Error)]
#[error("{conflict}")]
pub struct EditConflict {
    pub saved_version: i64,
    #[source]
    pub conflict: sync::Conflict,
}

pub const KINDS: [ArtifactKind; 4] = [
    ArtifactKind::Character,
    ArtifactKind::Environment,
    ArtifactKind::Object,
    ArtifactKind::Illustration,
];

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AssetLink {
    pub asset_id: Uuid,
    pub canvas_id: u32,
}

pub enum CatalogEdit {
    Create {
        kind: ArtifactKind,
        name: String,
        description: String,
        variant_of: Option<Uuid>,
    },
    Describe {
        id: Uuid,
        name: String,
        description: String,
    },
    ImportImage {
        id: Uuid,
        path: PathBuf,
    },
}

/// Validate before changing the caller's registry. The GUI owns the undo boundary;
/// an image import decodes its input but writes no files or shared state.
pub fn apply(registry: &mut ArtifactRegistry, edit: CatalogEdit) -> Result<Uuid> {
    match edit {
        CatalogEdit::Create {
            kind,
            name,
            description,
            variant_of,
        } => {
            ensure!(
                KINDS.contains(&kind),
                "Choose a character, place, prop or illustration"
            );
            let name = validate_name(registry, None, &name)?;
            Ok(registry.create_artifact(
                kind,
                CreateArtifact {
                    name,
                    description,
                    variant_of,
                    ..Default::default()
                },
            )?)
        }
        CatalogEdit::Describe {
            id,
            name,
            description,
        } => {
            let name = validate_name(registry, Some(id), &name)?;
            let artifact = registry
                .artifact_mut(id)
                .context("Catalog item no longer exists")?;
            ensure!(KINDS.contains(&artifact.kind), "Select a catalog item");
            artifact.name = name;
            artifact.description = description;
            Ok(id)
        }
        CatalogEdit::ImportImage { id, path } => {
            let artifact = registry
                .artifact(id)
                .context("Catalog item no longer exists")?;
            ensure!(KINDS.contains(&artifact.kind), "Select a catalog item");
            let path = path
                .canonicalize()
                .context("Read the selected artwork file")?;
            image::open(&path).context("Choose a readable supported image file")?;
            let path = path
                .to_str()
                .context("Artwork path is not UTF-8")?
                .to_owned();
            let revision = registry.start_revision(
                id,
                "Imported saved artwork".into(),
                None,
                Some("import".into()),
                RevisionOrigin::Canon,
            )?;
            registry.finish_revision(id, revision, path)?;
            Ok(id)
        }
    }
}

fn validate_name(registry: &ArtifactRegistry, id: Option<Uuid>, name: &str) -> Result<String> {
    let name = name.trim();
    ensure!(!name.is_empty(), "Enter a name for this item");
    ensure!(storyfella_language::identifier(name), "Use a reference name starting with a letter or underscore, followed by letters, digits or underscores");
    ensure!(
        !registry
            .artifacts
            .iter()
            .any(|a| Some(a.id) != id && a.name.eq_ignore_ascii_case(name)),
        "Another item already uses that name"
    );
    Ok(name.to_owned())
}

/// Merge the submitted form against its displayed baseline. Independent edits
/// survive; overlapping fields retain the form and fail without writing.
pub fn save_edit(
    db: &ProjectDb,
    snapshot: &StoredProject,
    edit: CatalogEdit,
    resolution: EditResolution,
) -> Result<(Uuid, StoredProject)> {
    let current = db.load()?;
    ensure!(
        current.id == snapshot.id,
        "This catalog snapshot belongs to another project"
    );
    let resolution = match resolution {
        EditResolution::Check => sync::Resolution::Check,
        EditResolution::KeepDraft { saved_version } => {
            ensure!(saved_version == current.registry_version, "The saved details changed again. Review the latest conflict before choosing which draft to keep");
            sync::Resolution::Local
        }
    };
    if let CatalogEdit::Describe { id, name, .. } = &edit {
        if snapshot
            .registry
            .artifact(*id)
            .is_some_and(|item| item.name != name.trim())
        {
            anyhow::bail!("Rename existing items in Studio so saved and unsaved story references keep the same identity");
        }
    }
    let mut registry = snapshot.registry.clone();
    let id = apply(&mut registry, edit)?;
    let registry = sync::merge(&snapshot.registry, &registry, &current.registry, resolution)
        .map_err(|conflict| EditConflict {
            saved_version: current.registry_version,
            conflict,
        })?;
    db.save_registry_checked(&registry, current.registry_version)?;
    Ok((id, db.load()?))
}

/// Rename only references resolved to this exact identity. Prose and unrelated
/// names are untouched. Studio owns applying this source edit and its undo.
pub fn rename_references(
    registry: &ArtifactRegistry,
    source: &str,
    id: Uuid,
    name: &str,
) -> Result<String> {
    validate_name(registry, Some(id), name)?;
    let catalog = crate::authoring::AssetCatalog::from_registry(registry);
    let analysis = storyfella_language::analyze(source, &catalog.assets);
    ensure!(
        !analysis.has_errors(),
        "Resolve source errors before renaming a catalog reference"
    );
    let id = id.to_string();
    let edits: Vec<_> = analysis
        .references
        .iter()
        .filter(|reference| reference.asset_id.as_deref() == Some(id.as_str()))
        .map(|reference| storyfella_language::TextEdit {
            replace: reference.name.span,
            new_text: name.to_owned(),
        })
        .collect();
    storyfella_language::apply_edits(source, &edits).map_err(anyhow::Error::msg)
}

/// Materialize explicit DSL declarations, never inferred dialogue speakers.
/// Invalid/incomplete drafts retain the previous catalog; removed declarations
/// do not delete items or visual work. The language analysis stays side-effect free.
pub fn reconcile_declarations(registry: &mut ArtifactRegistry, source: &str, live: bool) -> bool {
    let catalog = crate::authoring::AssetCatalog::from_registry(registry);
    let analysis = storyfella_language::analyze(source, &catalog.assets);
    if analysis.has_errors() {
        return false;
    }
    fn declarations(
        nodes: &[storyfella_language::Node],
        output: &mut Vec<(ArtifactKind, String, usize)>,
    ) {
        for node in nodes {
            if let storyfella_language::NodeKind::Asset { kind, names } = &node.kind {
                for name in names {
                    output.push((
                        crate::authoring::artifact_kind(*kind),
                        name.value.clone(),
                        node.span.end,
                    ));
                }
            }
            if let storyfella_language::NodeKind::Stage { asset, .. }
            | storyfella_language::NodeKind::Swap { asset, .. } = &node.kind
            {
                output.push((ArtifactKind::Character, asset.value.clone(), node.span.end));
            }
            declarations(&node.children, output);
        }
    }
    let mut declared = Vec::new();
    declarations(&analysis.document.story.scenes, &mut declared);
    let mut candidate = registry.clone();
    let mut changed = false;
    for (kind, name, end) in declared {
        let unfinished_line = analysis
            .document
            .source
            .position(end)
            .and_then(|position| analysis.document.source.line_span(position.line as usize))
            .is_some_and(|line| line.end == source.len());
        if live && unfinished_line {
            continue;
        }
        if let Some(existing) = candidate
            .artifacts
            .iter()
            .find(|a| a.name.eq_ignore_ascii_case(&name))
        {
            if existing.kind != kind {
                return false;
            }
            continue;
        }
        if apply(
            &mut candidate,
            CatalogEdit::Create {
                kind,
                name,
                description: String::new(),
                variant_of: None,
            },
        )
        .is_err()
        {
            return false;
        }
        changed = true;
    }
    if changed {
        *registry = candidate;
    }
    changed
}
