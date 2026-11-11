//! The original artwork migration. Only version 1 files use this schema.
use std::{collections::BTreeMap, sync::Arc};

use anyhow::{Context, Result};
use serde::Deserialize;

use super::{decode_reference, CanvasArtboard, Document, ImageLayer, Layer, Reference, Stroke};
use crate::{library, workflow::image_layer};

#[derive(Deserialize)]
struct VersionOne {
    title: String,
    #[serde(flatten)]
    canvas: Canvas,
    #[serde(default)]
    canvas_id: u32,
    #[serde(default)]
    canvas_created: bool,
    #[serde(default)]
    canvases: Vec<Artboard>,
    references: Vec<Arc<Reference>>,
    reference: Option<usize>,
    note: String,
    #[serde(default)]
    prompt_drafts: BTreeMap<String, String>,
    active_prompt_target: Option<String>,
}

#[derive(Deserialize)]
struct Artboard {
    id: u32,
    #[serde(flatten)]
    canvas: Canvas,
}

#[derive(Deserialize)]
struct Canvas {
    width: u32,
    height: u32,
    layers: Vec<OriginalLayer>,
    active_layer: usize,
}

#[derive(Deserialize)]
struct OriginalLayer {
    name: String,
    visible: bool,
    opacity: f32,
    strokes: Vec<Stroke>,
    image: Option<OriginalImage>,
}

#[derive(Deserialize)]
struct OriginalImage {
    reference: Arc<Reference>,
    transform: [f32; 6],
}

impl Canvas {
    fn into_artboard(self, id: u32) -> CanvasArtboard {
        CanvasArtboard {
            id,
            width: self.width,
            height: self.height,
            layers: self
                .layers
                .into_iter()
                .map(|layer| Layer {
                    name: layer.name,
                    visible: layer.visible,
                    locked: false,
                    opacity: layer.opacity,
                    group: None,
                    candidate: false,
                    strokes: layer.strokes,
                    image: layer.image.map(|image| ImageLayer {
                        reference: image.reference,
                        transform: image.transform,
                        crop: None,
                    }),
                })
                .collect(),
            active_layer: self.active_layer,
            paint_layer: None,
            layer_groups: BTreeMap::new(),
            references: Vec::new(),
            references_visible: true,
            drawing_target: Default::default(),
        }
    }
}

pub(super) fn upgrade(bytes: &[u8]) -> Result<Document> {
    let original: VersionOne =
        serde_json::from_slice(bytes).context("Invalid Atelier version 1 drawing")?;
    let mut doc = Document {
        title: original.title,
        canvases: original
            .canvases
            .into_iter()
            .map(|board| board.canvas.into_artboard(board.id))
            .collect(),
        references: original.references,
        reference: original.reference,
        note: original.note,
        prompt_drafts: original.prompt_drafts,
        active_prompt_target: original.active_prompt_target,
        ..Document::default()
    };
    doc.set_current_canvas(original.canvas.into_artboard(original.canvas_id));
    doc.canvas_created = original.canvas_created;
    promote_generated_references(&mut doc)?;
    Ok(doc)
}

fn promote_generated_references(doc: &mut Document) -> Result<()> {
    let selected = doc
        .reference
        .and_then(|i| doc.references.get(i))
        .map(|reference| library::id(reference));
    let generated: Vec<_> = doc
        .references
        .iter()
        .filter(|reference| !reference.prompt.is_empty())
        .filter(|reference| {
            !doc.layers
                .iter()
                .chain(doc.canvases.iter().flat_map(|board| &board.layers))
                .any(|layer| {
                    layer
                        .image
                        .as_ref()
                        .is_some_and(|image| image.reference.image == reference.image)
                })
        })
        .take(24usize.saturating_sub(doc.canvases.len()))
        .cloned()
        .collect();
    // Keep any originals that would exceed the artboard limit as references.
    for reference in generated {
        let promoted_id = library::id(&reference);
        let decoded = decode_reference(&reference)?;
        let (width, height) = (decoded.width(), decoded.height());
        let id = doc.allocate_canvas_id()?;
        doc.canvases.push(CanvasArtboard {
            id,
            width: width.max(64),
            height: height.max(64),
            layers: vec![image_layer(
                reference,
                width,
                height,
                (0, 0, width, height),
                false,
            )?],
            active_layer: 0,
            paint_layer: None,
            layer_groups: BTreeMap::new(),
            references: Vec::new(),
            references_visible: true,
            drawing_target: Default::default(),
        });
        doc.references
            .retain(|reference| library::id(reference) != promoted_id);
    }
    doc.reference = selected.and_then(|id| {
        doc.references
            .iter()
            .position(|reference| library::id(reference) == id)
    });
    Ok(())
}
