//! Ordinary saves persist editable work before publishing accepted artwork.
//! Recovery snapshots never publish. A failed second step is retryable from
//! the saved document, including after process exit.
use crate::document::{self, Document};
use anyhow::{Context, Result};
use std::{fmt, io::Cursor, path::Path};
use storyfella_studio_core::{
    catalog::{self, CanvasPublication, SavedCanvas},
    project::ProjectFolder,
};

/// Typed context for partial saves, including the exact revisions that committed
/// if writing the new artwork baselines back into the document failed.
#[derive(Debug)]
pub(crate) enum SaveStage {
    Document,
    Artwork,
    RevisionLinks(Vec<CanvasPublication>),
}

impl fmt::Display for SaveStage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Document => "Could not save the editable document",
            Self::Artwork => {
                "Editable document saved. Review the artwork error before retrying Save"
            }
            Self::RevisionLinks(_) => {
                "Shared artwork saved. Retry Save to finish saving the document’s revision links"
            }
        })
    }
}

pub(crate) fn committed_publications(error: &anyhow::Error) -> &[CanvasPublication] {
    match error.downcast_ref::<SaveStage>() {
        Some(SaveStage::RevisionLinks(publications)) => publications,
        _ => &[],
    }
}

pub(crate) fn apply_publications(doc: &mut Document, publications: &[CanvasPublication]) {
    for publication in publications {
        let baseline = doc
            .artwork_bases
            .get(&publication.canvas_id)
            .copied()
            .flatten();
        if baseline == publication.expected_revision
            && doc.asset_links.iter().any(|link| {
                link.canvas_id == publication.canvas_id && link.asset_id == publication.asset_id
            })
        {
            doc.artwork_bases
                .insert(publication.canvas_id, Some(publication.revision_id));
        }
    }
}

pub(crate) fn save(path: &Path, doc: &Document) -> Result<Vec<CanvasPublication>> {
    save_with_renderer(path, doc, render)
}

/// Closed-project MCP uses the same canvas rasterizer for review and publication.
pub(crate) fn save_raster(
    path: &Path,
    doc: &Document,
    canvas_id: u32,
) -> Result<Vec<CanvasPublication>> {
    save_selected(path, doc, Some(canvas_id), |doc| {
        document::raster(doc, doc.width.max(doc.height), true)
    })
}

fn save_with_renderer(
    path: &Path,
    doc: &Document,
    render: impl FnMut(&Document) -> Result<image::RgbaImage>,
) -> Result<Vec<CanvasPublication>> {
    save_selected(path, doc, None, render)
}

fn save_selected(
    path: &Path,
    doc: &Document,
    canvas_id: Option<u32>,
    render: impl FnMut(&Document) -> Result<image::RgbaImage>,
) -> Result<Vec<CanvasPublication>> {
    document::save(path, doc).context(SaveStage::Document)?;
    let publications = publish(path, doc, canvas_id, render).context(SaveStage::Artwork)?;
    if !publications.is_empty() {
        let mut saved = doc.clone();
        apply_publications(&mut saved, &publications);
        if let Err(error) = document::save(path, &saved) {
            return Err(error.context(SaveStage::RevisionLinks(publications)));
        }
    }
    Ok(publications)
}

fn publish(
    path: &Path,
    doc: &Document,
    canvas_id: Option<u32>,
    mut render: impl FnMut(&Document) -> Result<image::RgbaImage>,
) -> Result<Vec<CanvasPublication>> {
    let Some(parent) = path
        .parent()
        .filter(|parent| parent.file_name().is_some_and(|name| name == "atelier"))
    else {
        return Ok(Vec::new());
    };
    let root = parent
        .parent()
        .context("Shared project folder is missing")?;
    if !root
        .join(storyfella_studio_core::project::MANIFEST)
        .is_file()
    {
        return Ok(Vec::new());
    }
    let folder = ProjectFolder::locate_for_authoring(root)?.context("Shared project is missing")?;
    // Save As copies retain editable work, but do not publish into the source
    // project's catalog just because their links were copied with them.
    if path.canonicalize()? != folder.atelier_document()? {
        return Ok(Vec::new());
    }
    let mut canvases = Vec::new();
    for link in doc
        .asset_links
        .iter()
        .filter(|link| canvas_id.is_none_or(|id| id == link.canvas_id))
    {
        let mut canvas = doc.clone();
        canvas.select_canvas(link.canvas_id)?;
        let accepted = canvas.artwork();
        if !accepted.layers.iter().any(|layer| {
            accepted.layer_is_visible(layer)
                && (layer.image.is_some()
                    || layer.strokes.iter().any(|stroke| !stroke.points.is_empty()))
        }) {
            continue;
        }
        let image = render(&accepted)?;
        let mut png = Cursor::new(Vec::new());
        image.write_to(&mut png, image::ImageFormat::Png)?;
        canvases.push(SavedCanvas {
            asset_id: link.asset_id,
            canvas_id: link.canvas_id,
            expected_revision: doc.artwork_bases.get(&link.canvas_id).copied().flatten(),
            png: png.into_inner(),
        });
    }
    if canvases.is_empty() {
        return Ok(Vec::new());
    }
    catalog::save_canvases(&folder, &canvases)
}

fn render(doc: &Document) -> Result<image::RgbaImage> {
    #[cfg(target_os = "macos")]
    {
        objc2::rc::autoreleasepool(|_| crate::gpu_canvas::Renderer::new()?.export(doc, true))
    }
    #[cfg(not(target_os = "macos"))]
    {
        document::raster(doc, doc.width.max(doc.height), true)
    }
}

#[cfg(test)]
mod production_slice;

#[cfg(test)]
mod tests {
    use super::*;
    use storyfella_studio_core::{
        catalog::{AssetLink, CatalogEdit},
        persistence::ProjectDb,
        playback::PlaybackSession,
        registry::{ArtifactKind, ArtifactRegistry},
    };

    fn layer(color: [u8; 4], candidate: bool) -> crate::document::Layer {
        let image = image::RgbaImage::from_pixel(64, 96, image::Rgba(color));
        let mut bytes = Cursor::new(Vec::new());
        image.write_to(&mut bytes, image::ImageFormat::Png).unwrap();
        let reference = std::sync::Arc::new(
            document::reference_from_bytes(
                "Saved image".into(),
                &bytes.into_inner(),
                String::new(),
            )
            .unwrap(),
        );
        crate::workflow::image_layer(reference, 64, 96, (0, 0, 64, 96), candidate).unwrap()
    }
    fn cpu(doc: &Document) -> Result<image::RgbaImage> {
        document::raster(doc, doc.width.max(doc.height), true)
    }

    #[test]
    fn ordinary_save_publishes_all_linked_artboards_once_preserves_pinned_plans_and_recovers_failures(
    ) {
        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../.svs-e2e/tests")
            .join(format!("canvas-save-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let mut registry = ArtifactRegistry::default();
        let mut create = |name: &str| {
            catalog::apply(
                &mut registry,
                CatalogEdit::Create {
                    kind: ArtifactKind::Character,
                    name: name.into(),
                    description: String::new(),
                    variant_of: None,
                },
            )
            .unwrap()
        };
        let mara = create("mara");
        let tam = create("tam");
        let source = "scene harbor\n  cast: mara\n  beat arrival\n    mara: Hello\n";
        let folder =
            ProjectFolder::create(&root.join("story"), "Harbor", &registry, source).unwrap();
        let path = document::project_document(&folder.manifest()).unwrap();
        let mut doc = document::load(&path).unwrap();
        let first = doc
            .ensure_story_artboard(mara, String::new(), String::new())
            .unwrap();
        doc.width = 64;
        doc.height = 96;
        doc.layers = vec![layer([0, 0, 255, 128], false)];
        let second = doc
            .ensure_story_artboard(tam, String::new(), String::new())
            .unwrap();
        let shape = crate::shapes::stroke(
            crate::shapes::ShapeKind::Rectangle,
            document::Point { x: 0., y: 0. },
            document::Point {
                x: doc.width as f32,
                y: doc.height as f32,
            },
            [0, 255, 0, 255],
            [0; 4],
            1.,
        )
        .unwrap();
        let shape_id = shape.shape.as_ref().unwrap().id;
        doc.add_mark(0, shape).unwrap();
        doc.layers.push(layer([255, 0, 0, 255], true));
        let saved = save_with_renderer(&path, &doc, cpu).unwrap();
        assert_eq!(saved.len(), 2);
        assert_eq!(
            document::load(&path).unwrap().shape_location(shape_id),
            Some((0, 0))
        );
        let db = ProjectDb::open_existing(folder.database()).unwrap();
        let original = db.load().unwrap();
        let first_image = original.registry.latest_image(mara).unwrap();
        let first_bytes = std::fs::read(first_image.asset_path.as_ref().unwrap()).unwrap();
        let image = image::load_from_memory(&first_bytes).unwrap().to_rgba8();
        assert_eq!(image.dimensions(), (64, 96));
        assert_eq!(image.get_pixel(10, 10)[3], 128);
        let other = image::open(
            original
                .registry
                .latest_image(tam)
                .unwrap()
                .asset_path
                .as_ref()
                .unwrap(),
        )
        .unwrap()
        .to_rgba8();
        assert_eq!(
            other.get_pixel(10, 10).0,
            [0, 255, 0, 255],
            "Pending candidates must not enter Studio artwork"
        );
        let plan = PlaybackSession::compile(source, &original.registry)
            .unwrap()
            .runtime
            .plan()
            .clone();
        // Retry the old document snapshot after the registry commit, including
        // the case where final publication metadata was not saved/acknowledged.
        save_with_renderer(&path, &doc, cpu).unwrap();
        assert_eq!(db.load().unwrap(), original);
        apply_publications(&mut doc, &saved);
        let mut history = crate::document::History::new();
        history.checkpoint(&doc);
        doc.layers[0] = layer([255, 255, 0, 255], false);
        doc.canvases
            .iter_mut()
            .find(|canvas| canvas.id == first)
            .unwrap()
            .layers[0] = layer([0, 0, 255, 255], false);
        let newer = save_with_renderer(&path, &doc, cpu).unwrap();
        apply_publications(&mut doc, &newer);
        let current = db.load().unwrap();
        assert!(history.undo(&mut doc));
        assert_eq!(
            doc.artwork_bases[&second],
            current.registry.artifact(tam).unwrap().active_revision_id
        );
        assert!(history.redo(&mut doc));
        assert_eq!(
            doc.artwork_bases[&second],
            current.registry.artifact(tam).unwrap().active_revision_id
        );
        assert_eq!(current.registry.artifact(tam).unwrap().revisions.len(), 2);
        assert_eq!(current.registry.artifact(mara).unwrap().revisions.len(), 2);
        assert_ne!(
            current.registry.artifact(mara).unwrap().active_revision_id,
            Some(first_image.id)
        );
        assert_eq!(
            std::fs::read(first_image.asset_path.as_ref().unwrap()).unwrap(),
            first_bytes
        );
        PlaybackSession::from_plan(plan, &current.registry).unwrap();
        assert_eq!(
            document::load(&path).unwrap().artwork_bases[&first],
            current.registry.artifact(mara).unwrap().active_revision_id
        );
        assert_eq!(
            document::load(&path).unwrap().artwork_bases[&second],
            current.registry.artifact(tam).unwrap().active_revision_id
        );
        // A render failure keeps the editable drawing on disk and changes no
        // registry revision. A corrected normal Save can retry.
        doc.note = "Retain my latest editing".into();
        let error =
            save_with_renderer(&path, &doc, |_| anyhow::bail!("render failed")).unwrap_err();
        assert!(matches!(
            error.downcast_ref::<SaveStage>(),
            Some(SaveStage::Artwork)
        ));
        assert!(committed_publications(&error).is_empty());
        assert_eq!(document::load(&path).unwrap().note, doc.note);
        assert_eq!(db.load().unwrap(), current);
        // An outside image change must not be overwritten by the stale canvas.
        let mut bytes = Cursor::new(Vec::new());
        image::RgbaImage::from_pixel(64, 96, image::Rgba([12u8, 34, 56, 255]))
            .write_to(&mut bytes, image::ImageFormat::Png)
            .unwrap();
        catalog::save_canvases(
            &folder,
            &[SavedCanvas {
                asset_id: tam,
                canvas_id: second,
                expected_revision: current.registry.artifact(tam).unwrap().active_revision_id,
                png: bytes.into_inner(),
            }],
        )
        .unwrap();
        let external = db.load().unwrap();
        doc.layers[0] = layer([255, 0, 255, 255], false);
        doc.canvases
            .iter_mut()
            .find(|canvas| canvas.id == first)
            .unwrap()
            .layers[0] = layer([255, 255, 255, 255], false);
        assert!(save_with_renderer(&path, &doc, cpu).is_err());
        assert_eq!(db.load().unwrap(), external);
        assert_eq!(document::load(&path).unwrap().note, doc.note);
        assert_eq!(
            std::fs::read_to_string(folder.root().join("assets/story.sf")).unwrap(),
            source
        );
        // Explicitly relinking acknowledges the current image for conflict recovery.
        doc.artwork_bases.insert(
            second,
            external.registry.artifact(tam).unwrap().active_revision_id,
        );
        save_with_renderer(&path, &doc, cpu).unwrap();
        assert!(
            ProjectDb::open_existing(folder.database())
                .unwrap()
                .load()
                .unwrap()
                .registry
                .artifact(tam)
                .unwrap()
                .revisions
                .len()
                > external.registry.artifact(tam).unwrap().revisions.len()
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn empty_or_candidate_only_linked_canvases_stay_pending() {
        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../.svs-e2e/tests")
            .join(format!("empty-save-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let folder = ProjectFolder::create(
            &root.join("story"),
            "Story",
            &ArtifactRegistry::default(),
            "",
        )
        .unwrap();
        let copy = folder.root().join("atelier/copy.atelier");
        assert!(
            save_with_renderer(&copy, &Document::default(), |_| anyhow::bail!(
                "Save As must not render"
            ))
            .unwrap()
            .is_empty()
        );
        let path = document::project_document(&folder.manifest()).unwrap();
        let mut doc = document::load(&path).unwrap();
        doc.canvas_created = true;
        doc.asset_links.push(AssetLink {
            asset_id: uuid::Uuid::new_v4(),
            canvas_id: doc.canvas_id,
        });
        for pending in [false, true] {
            if pending {
                doc.layers = vec![layer([255, 0, 0, 255], true)];
            }
            assert!(save_with_renderer(&path, &doc, |_| anyhow::bail!(
                "Nothing accepted to render"
            ))
            .unwrap()
            .is_empty());
        }
        assert!(ProjectDb::open_existing(folder.database())
            .unwrap()
            .load()
            .unwrap()
            .registry
            .artifacts
            .is_empty());
        std::fs::remove_dir_all(root).unwrap();
    }
}
