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

/// An editor must save against the snapshot it actually displayed. The source
/// document and unrelated artwork remain untouched on a stale write or error.
pub fn save_edit(
    db: &ProjectDb,
    snapshot: &StoredProject,
    edit: CatalogEdit,
) -> Result<(Uuid, StoredProject)> {
    let current = db.load()?;
    ensure!(
        current.id == snapshot.id,
        "This catalog snapshot belongs to another project"
    );
    if let CatalogEdit::Describe { id, name, .. } = &edit {
        if current
            .registry
            .artifact(*id)
            .is_some_and(|item| item.name != name.trim())
        {
            let renamed =
                rename_references(&current.registry, current.story.source(), *id, name.trim())?;
            ensure!(renamed == current.story.source(), "This item is used in the story. Rename it in Studio so its source references keep the same identity");
        }
    }
    let mut registry = snapshot.registry.clone();
    let id = apply(&mut registry, edit)?;
    db.save_registry_checked(&registry, snapshot.registry_version)?;
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
