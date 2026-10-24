//! Project artwork, generation destinations and upgrades share one document model.
use crate::document::{self, CanvasArtboard, Document, ImageLayer, Layer, Reference};
use anyhow::{ensure, Context, Result};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum SelectionMode {
    #[default]
    Blend,
    Separate,
}
pub type PixelRect = (u32, u32, u32, u32);
pub type BlendCrop = ((u32, u32), PixelRect);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Destination {
    NewArtboard,
    Candidate {
        artboard: u32,
        rect: (u32, u32, u32, u32),
    },
}

#[derive(Clone, Copy)]
pub enum Review {
    Accept,
    Reference,
    Discard,
}

impl Document {
    pub fn generation_source(&self, crop: Option<PixelRect>) -> Result<Reference> {
        let mut raster = document::raster(&self.artwork(), document::MAX_EDGE, true)?;
        if let Some((x, y, w, h)) = crop {
            ensure!(
                w > 0
                    && h > 0
                    && x.saturating_add(w) <= self.width
                    && y.saturating_add(h) <= self.height,
                "Selection is outside the artboard"
            );
            raster = image::imageops::crop_imm(&raster, x, y, w, h).to_image();
        }
        let mut bytes = std::io::Cursor::new(Vec::new());
        raster.write_to(&mut bytes, image::ImageFormat::Png)?;
        let mut source = document::reference_from_bytes(
            format!("Artboard {} · {}", self.canvas_id + 1, self.title),
            &bytes.into_inner(),
            "Accepted Atelier artboard snapshot".into(),
        )?;
        if let Some(previous) = self
            .layers
            .iter()
            .rev()
            .filter(|l| l.visible && l.opacity > 0. && !l.candidate)
            .find_map(|l| l.image.as_ref().map(|i| &i.reference))
        {
            let previous_id = crate::library::id(previous);
            source.parent = if crate::library::id(&source) == previous_id {
                previous.parent.clone()
            } else {
                Some(previous_id)
            };
        }
        Ok(source)
    }

    pub fn artwork_metadata(&self) -> (String, String) {
        let sources: Vec<_> = self
            .layers
            .iter()
            .filter(|l| l.visible && l.opacity > 0. && !l.candidate)
            .filter_map(|l| l.image.as_ref().map(|i| (l.name.as_str(), &i.reference)))
            .collect();
        let prompt = if self.note.trim().is_empty() && sources.len() == 1 {
            sources[0].1.prompt.clone()
        } else {
            self.note.clone()
        };
        let mut provenance =
            "Approved Atelier artboard · accepted visible layers and painted artwork".to_owned();
        for (name, reference) in sources {
            provenance.push_str(&format!(
                "\nLayer: {name}\nImage: {}\n{}",
                crate::library::id(reference),
                reference.provenance
            ));
            if let Some(parent) = &reference.parent {
                provenance.push_str(&format!("\nParent: {parent}"));
            }
        }
        (prompt, provenance)
    }

    pub fn layer_bounds(&self) -> Option<(f32, f32, f32, f32)> {
        use base64::Engine;
        let layer = &self.layers[self.active_layer];
        let mut points = Vec::new();
        if let Some(image) = &layer.image {
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(&image.reference.image)
                .ok()?;
            let (w, h) = image::ImageReader::new(std::io::Cursor::new(bytes))
                .with_guessed_format()
                .ok()?
                .into_dimensions()
                .ok()?;
            let [a, b, c, d, x, y] = image.transform;
            for (px, py) in [
                (0., 0.),
                (w as f32, 0.),
                (w as f32, h as f32),
                (0., h as f32),
            ] {
                points.push((a * px + c * py + x, b * px + d * py + y));
            }
        }
        for stroke in &layer.strokes {
            for p in stroke.points.iter() {
                points.push((p.x - stroke.size / 2., p.y - stroke.size / 2.));
                points.push((p.x + stroke.size / 2., p.y + stroke.size / 2.));
            }
        }
        if points.is_empty() {
            return None;
        }
        Some(points.into_iter().fold(
            (f32::MAX, f32::MAX, f32::MIN, f32::MIN),
            |(x, y, r, b), (px, py)| (x.min(px), y.min(py), r.max(px), b.max(py)),
        ))
    }

    /// Hide candidates in a snapshot, keeping on-screen review independent of export.
    pub fn artwork(&self) -> Self {
        let mut result = self.clone();
        for layer in &mut result.layers {
            if layer.candidate {
                layer.visible = false;
            }
        }
        result
    }

    /// Version 1 displayed generated references as artboards without giving them
    /// layers. Promote those images once; retain their original payload and lineage.
    pub fn migrate(&mut self) -> Result<()> {
        if self.version == 1 {
            let selected = self
                .reference
                .and_then(|i| self.references.get(i))
                .map(|r| crate::library::id(r));
            let generated: Vec<_> = self
                .references
                .iter()
                .filter(|r| !r.prompt.is_empty())
                .filter(|r| {
                    !self
                        .layers
                        .iter()
                        .chain(self.canvases.iter().flat_map(|c| &c.layers))
                        .any(|l| {
                            l.image
                                .as_ref()
                                .is_some_and(|i| i.reference.image == r.image)
                        })
                })
                .cloned()
                .collect();
            // Projects above the artboard limit keep every original as a reference.
            for reference in generated
                .into_iter()
                .take(24usize.saturating_sub(self.canvases.len()))
            {
                let promoted_id = crate::library::id(&reference);
                let decoded = document::decode_reference(&reference)?;
                let id = self.next_artboard_id()?;
                self.canvases.push(CanvasArtboard {
                    id,
                    width: decoded.width().max(64),
                    height: decoded.height().max(64),
                    layers: vec![image_layer(
                        reference,
                        decoded.width(),
                        decoded.height(),
                        (0, 0, decoded.width(), decoded.height()),
                        false,
                    )?],
                    active_layer: 0,
                    references: vec![],
                    references_visible: true,
                });
                self.references
                    .retain(|r| crate::library::id(r) != promoted_id);
            }
            self.reference = selected.and_then(|id| {
                self.references
                    .iter()
                    .position(|r| crate::library::id(r) == id)
            });
            self.version = 2;
        }
        Ok(())
    }

    fn next_artboard_id(&self) -> Result<u32> {
        self.canvases
            .iter()
            .map(|c| c.id)
            .chain([self.canvas_id])
            .max()
            .unwrap_or_default()
            .checked_add(1)
            .context("Artboard ID limit reached")
    }

    /// Attach to the captured artboard, even if the user has since opened another.
    /// Validate on a clone so a failed attachment never changes the document.
    pub fn attach_result(
        &mut self,
        reference: Arc<Reference>,
        destination: Destination,
    ) -> Result<()> {
        let decoded = document::decode_reference(&reference)?;
        let (width, height) = (decoded.width(), decoded.height());
        let mut next = self.clone();
        match destination {
            Destination::NewArtboard => {
                let previous_id = next.canvas_id;
                let references = next.artboard_references.clone();
                next.create_canvas()?;
                next.artboard_references = references;
                next.width = width.max(64);
                next.height = height.max(64);
                next.layers = vec![image_layer(
                    reference,
                    width,
                    height,
                    (0, 0, width, height),
                    false,
                )?];
                next.active_layer = 0;
                // Background completion must not switch away from artwork the
                // user is currently editing. Replace only an empty placeholder.
                if let Some(index) = next.canvases.iter().position(|c| c.id == previous_id) {
                    let generated = next.current_canvas();
                    let previous = std::mem::replace(&mut next.canvases[index], generated);
                    next.set_current_canvas(previous);
                }
            }
            Destination::Candidate { artboard, rect } => {
                let (w, h, layers, active) = if artboard == next.canvas_id {
                    (
                        next.width,
                        next.height,
                        &mut next.layers,
                        &mut next.active_layer,
                    )
                } else {
                    let canvas = next
                        .canvases
                        .iter_mut()
                        .find(|c| c.id == artboard)
                        .context(
                        "The source artboard no longer exists; recover the result from the queue",
                    )?;
                    (
                        canvas.width,
                        canvas.height,
                        &mut canvas.layers,
                        &mut canvas.active_layer,
                    )
                };
                ensure!(
                    rect.0.saturating_add(rect.2) <= w && rect.1.saturating_add(rect.3) <= h,
                    "The artboard was resized; recover the result from the queue"
                );
                ensure!(
                    layers.len() < 24,
                    "Maximum 24 layers; recover the result from the queue"
                );
                layers.push(image_layer(reference, width, height, rect, true)?);
                *active = layers.len() - 1;
            }
        }
        next.validate()?;
        *self = next;
        Ok(())
    }
}

fn image_layer(
    reference: Arc<Reference>,
    width: u32,
    height: u32,
    rect: (u32, u32, u32, u32),
    candidate: bool,
) -> Result<Layer> {
    ensure!(
        width > 0 && height > 0 && rect.2 > 0 && rect.3 > 0,
        "Image has no area"
    );
    Ok(Layer {
        name: reference.name.clone(),
        visible: true,
        opacity: 1.,
        candidate,
        strokes: vec![],
        image: Some(ImageLayer {
            reference,
            transform: [
                rect.2 as f32 / width as f32,
                0.,
                0.,
                rect.3 as f32 / height as f32,
                rect.0 as f32,
                rect.1 as f32,
            ],
        }),
    })
}

/// Blend routes receive the complete accepted artwork as context. Only the selected
/// rectangle is attached; provider changes outside it cannot alter the original.
pub fn crop_blend(
    reference: &Reference,
    source_size: (u32, u32),
    rect: (u32, u32, u32, u32),
) -> Result<Reference> {
    let output = document::decode_reference(reference)?.to_rgba8();
    let output = image::imageops::resize(
        &output,
        source_size.0,
        source_size.1,
        image::imageops::FilterType::Lanczos3,
    );
    let crop = image::imageops::crop_imm(&output, rect.0, rect.1, rect.2, rect.3).to_image();
    let mut bytes = std::io::Cursor::new(Vec::new());
    crop.write_to(&mut bytes, image::ImageFormat::Png)?;
    let mut result = document::reference_from_bytes(
        reference.name.clone(),
        &bytes.into_inner(),
        reference.provenance.clone(),
    )?;
    result.parent = reference.parent.clone();
    result.prompt = reference.prompt.clone();
    Ok(result)
}
