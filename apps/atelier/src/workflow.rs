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
    NewLayer {
        artboard: u32,
        rect: (u32, u32, u32, u32),
    },
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
    /// Import separate artwork layers into the artboard captured when the import began.
    pub fn attach_imported_images(
        &mut self,
        artboard: u32,
        rect: (u32, u32, u32, u32),
        images: Vec<Reference>,
    ) -> Result<Vec<usize>> {
        ensure!(!images.is_empty(), "No images to import");
        let extra = images.iter().map(|image| image.image.len()).sum::<usize>();
        ensure!(
            self.embedded_image_bytes().saturating_add(extra) <= 64 * 1024 * 1024,
            "Project images exceed 64 MB"
        );
        let mut next = self.clone();
        let (width, height, layers) = if artboard == next.canvas_id {
            (next.width, next.height, &mut next.layers)
        } else {
            let canvas = next
                .canvases
                .iter_mut()
                .find(|canvas| canvas.id == artboard)
                .context("The destination artboard no longer exists")?;
            (canvas.width, canvas.height, &mut canvas.layers)
        };
        ensure!(
            rect.2 > 0
                && rect.3 > 0
                && rect.0.saturating_add(rect.2) <= width
                && rect.1.saturating_add(rect.3) <= height,
            "The destination artboard was resized; import the images again"
        );
        ensure!(
            layers.len() + images.len() <= 24,
            "Import would exceed the 24-layer limit"
        );
        let first = layers.len();
        let count = images.len();
        let gap = if count > 1 {
            16f32.min(rect.2 as f32 / (count as f32 * 4.))
        } else {
            0.
        };
        let cell_width = (rect.2 as f32 - gap * (count.saturating_sub(1) as f32)) / count as f32;
        for (index, image) in images.into_iter().enumerate() {
            let decoded = document::decode_reference(&image)?;
            let (source_width, source_height) = (decoded.width(), decoded.height());
            let scale =
                (cell_width / source_width as f32).min(rect.3 as f32 / source_height as f32);
            let mut layer = image_layer(Arc::new(image), source_width, source_height, rect, false)?;
            layer.image.as_mut().unwrap().transform = [
                scale,
                0.,
                0.,
                scale,
                rect.0 as f32
                    + index as f32 * (cell_width + gap)
                    + (cell_width - source_width as f32 * scale) / 2.,
                rect.1 as f32 + (rect.3 as f32 - source_height as f32 * scale) / 2.,
            ];
            layers.push(layer);
        }
        next.validate()?;
        *self = next;
        Ok((first..first + count).collect())
    }

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
            .filter(|l| self.layer_is_visible(l) && !l.candidate)
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
            .filter(|l| self.layer_is_visible(l) && !l.candidate)
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
        self.layer_bounds_for(self.active_layer)
    }

    /// Find visible, unlocked artwork under the pointer, optionally cycling below a layer.
    pub fn layer_at_point(
        &self,
        point: document::Point,
        tolerance: f32,
        after: Option<usize>,
        skip: &[usize],
    ) -> Option<usize> {
        use base64::Engine;
        use image::GenericImageView;
        let mut order: Vec<_> = (0..self.layers.len()).rev().collect();
        if let Some(position) = after.and_then(|index| order.iter().position(|item| *item == index))
        {
            let count = order.len();
            if count > 0 {
                order.rotate_left((position + 1) % count);
            }
        }
        order.into_iter().find_map(|index| {
            if skip.contains(&index) {
                return None;
            }
            let layer = &self.layers[index];
            if !self.layer_is_visible(layer) || self.layer_is_locked(layer) {
                return None;
            }
            let (left, top, right, bottom) = self.layer_bounds_for(index)?;
            if point.x < left - tolerance
                || point.x > right + tolerance
                || point.y < top - tolerance
                || point.y > bottom + tolerance
            {
                return None;
            }
            let mut hit = layer
                .image
                .as_ref()
                .and_then(|image| {
                    let [a, b, c, d, x, y] = image.transform;
                    let determinant = a * d - b * c;
                    if determinant.abs() < f32::EPSILON {
                        return None;
                    }
                    let (dx, dy) = (point.x - x, point.y - y);
                    let source_x = (d * dx - c * dy) / determinant;
                    let source_y = (-b * dx + a * dy) / determinant;
                    if source_x < 0. || source_y < 0. {
                        return None;
                    }
                    if let Some([x, y, width, height]) = image.crop {
                        if source_x < x as f32
                            || source_y < y as f32
                            || source_x >= x.saturating_add(width) as f32
                            || source_y >= y.saturating_add(height) as f32
                        {
                            return None;
                        }
                    }
                    let bytes = base64::engine::general_purpose::STANDARD
                        .decode(&image.reference.image)
                        .ok()?;
                    let decoded = image::load_from_memory(&bytes).ok()?;
                    if source_x >= decoded.width() as f32 || source_y >= decoded.height() as f32 {
                        return None;
                    }
                    Some(decoded.get_pixel(source_x as u32, source_y as u32).0[3] > 0)
                })
                .unwrap_or(false);
            for stroke in &layer.strokes {
                if stroke.opacity <= 0. {
                    continue;
                }
                let radius = if stroke.tool == document::Tool::Eraser {
                    stroke.size / 2.
                } else {
                    (stroke.size / 2.).max(tolerance)
                };
                let distance = |a: &document::Point, b: &document::Point| {
                    let dx = b.x - a.x;
                    let dy = b.y - a.y;
                    let length2 = dx * dx + dy * dy;
                    let t = if length2 > 0. {
                        (((point.x - a.x) * dx + (point.y - a.y) * dy) / length2).clamp(0., 1.)
                    } else {
                        0.
                    };
                    (point.x - a.x - t * dx).hypot(point.y - a.y - t * dy)
                };
                if stroke
                    .points
                    .first()
                    .is_some_and(|first| distance(first, first) <= radius)
                    || stroke
                        .points
                        .windows(2)
                        .any(|pair| distance(&pair[0], &pair[1]) <= radius)
                {
                    hit = stroke.tool != document::Tool::Eraser;
                }
            }
            hit.then_some(index)
        })
    }

    /// Cache eligible bounds once when a marquee gesture starts.
    pub fn selectable_layer_bounds(&self) -> Vec<(usize, (f32, f32, f32, f32))> {
        self.layers
            .iter()
            .enumerate()
            .filter_map(|(index, layer)| {
                if !self.layer_is_visible(layer) || self.layer_is_locked(layer) {
                    return None;
                }
                self.layer_bounds_for(index).map(|bounds| (index, bounds))
            })
            .collect()
    }

    pub fn layer_bounds_for(&self, index: usize) -> Option<(f32, f32, f32, f32)> {
        use base64::Engine;
        let layer = self.layers.get(index)?;
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
            let [left, top, width, height] = image.crop.unwrap_or([0, 0, w, h]);
            let (left, top, right, bottom) = (
                left as f32,
                top as f32,
                left.saturating_add(width) as f32,
                top.saturating_add(height) as f32,
            );
            for (px, py) in [(left, top), (right, top), (right, bottom), (left, bottom)] {
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

    pub fn selected_layer_bounds(&self, indices: &[usize]) -> Option<(f32, f32, f32, f32)> {
        indices
            .iter()
            .filter_map(|index| self.layer_bounds_for(*index))
            .fold(None, |bounds, (left, top, right, bottom)| {
                Some(match bounds {
                    Some((x0, y0, x1, y1)) => {
                        (x0.min(left), y0.min(top), x1.max(right), y1.max(bottom))
                    }
                    None => (left, top, right, bottom),
                })
            })
    }

    pub fn translate_layers(&mut self, indices: &[usize], dx: f32, dy: f32) -> Result<()> {
        ensure!(dx.is_finite() && dy.is_finite(), "Invalid layer movement");
        let indices = indices
            .iter()
            .copied()
            .collect::<std::collections::BTreeSet<_>>();
        for index in &indices {
            let layer = self.layers.get(*index).context("Layer no longer exists")?;
            if let Some(image) = &layer.image {
                ensure!(
                    (image.transform[4] + dx).is_finite()
                        && (image.transform[5] + dy).is_finite()
                        && (image.transform[4] + dx).abs() < 1_000_000.
                        && (image.transform[5] + dy).abs() < 1_000_000.,
                    "Layer position limit reached"
                );
            }
            ensure!(
                layer
                    .strokes
                    .iter()
                    .flat_map(|stroke| stroke.points.iter())
                    .all(|point| {
                        let x = point.x + dx;
                        let y = point.y + dy;
                        x.is_finite()
                            && y.is_finite()
                            && x.abs() < 1_000_000.
                            && y.abs() < 1_000_000.
                    }),
                "Layer position limit reached"
            );
        }
        for index in indices {
            let layer = self
                .layers
                .get_mut(index)
                .context("Layer no longer exists")?;
            if let Some(image) = &mut layer.image {
                image.transform[4] += dx;
                image.transform[5] += dy;
            }
            for stroke in &mut layer.strokes {
                for point in std::sync::Arc::make_mut(&mut stroke.points) {
                    point.x += dx;
                    point.y += dy;
                }
            }
        }
        Ok(())
    }

    pub fn transform_layers_to_bounds(
        &mut self,
        indices: &[usize],
        target: (f32, f32, f32, f32),
    ) -> Result<()> {
        let (left, top, right, bottom) = self
            .selected_layer_bounds(indices)
            .context("Selected layers have no transformable artwork")?;
        let (x, y, width, height) = target;
        ensure!(
            [x, y, width, height].iter().all(|v| v.is_finite()) && width > 0. && height > 0.,
            "Position and size must be finite and positive"
        );
        let (source_width, source_height) = (right - left, bottom - top);
        ensure!(
            source_width > 0. && source_height > 0.,
            "Selection has no area"
        );
        let (scale_x, scale_y) = (width / source_width, height / source_height);
        ensure!(
            scale_x.is_finite() && scale_y.is_finite() && scale_x > 0. && scale_y > 0.,
            "Invalid selection size"
        );
        let indices = indices
            .iter()
            .copied()
            .collect::<std::collections::BTreeSet<_>>();
        for index in &indices {
            let layer = self.layers.get(*index).context("Layer no longer exists")?;
            if let Some(image) = &layer.image {
                let [a, b, c, d, tx, ty] = image.transform;
                let determinant = (a * scale_x) * (d * scale_y) - (b * scale_y) * (c * scale_x);
                ensure!(
                    [
                        a * scale_x,
                        c * scale_x,
                        b * scale_y,
                        d * scale_y,
                        (tx - left) * scale_x + x,
                        (ty - top) * scale_y + y
                    ]
                    .iter()
                    .all(|v| v.is_finite() && v.abs() < 1_000_000.),
                    "Selection transform exceeds the supported range"
                );
                ensure!(
                    determinant.is_finite() && determinant.abs() > 0.000001,
                    "Selection size is too small for image layers"
                );
            }
            ensure!(
                layer
                    .strokes
                    .iter()
                    .flat_map(|stroke| stroke.points.iter())
                    .all(|point| {
                        let px = (point.x - left) * scale_x + x;
                        let py = (point.y - top) * scale_y + y;
                        px.is_finite()
                            && py.is_finite()
                            && px.abs() < 1_000_000.
                            && py.abs() < 1_000_000.
                    }),
                "Selection transform exceeds the supported range"
            );
        }
        let stroke_scale = (scale_x * scale_y).sqrt();
        for index in indices {
            let layer = &mut self.layers[index];
            if let Some(image) = &mut layer.image {
                let [a, b, c, d, tx, ty] = image.transform;
                image.transform = [
                    a * scale_x,
                    b * scale_y,
                    c * scale_x,
                    d * scale_y,
                    (tx - left) * scale_x + x,
                    (ty - top) * scale_y + y,
                ];
            }
            for stroke in &mut layer.strokes {
                stroke.size = (stroke.size * stroke_scale).clamp(0.1, 512.);
                for point in std::sync::Arc::make_mut(&mut stroke.points) {
                    point.x = (point.x - left) * scale_x + x;
                    point.y = (point.y - top) * scale_y + y;
                }
            }
        }
        Ok(())
    }

    /// Rotate images and paint together around a fixed artboard-space pivot.
    pub fn rotate_layers(
        &mut self,
        indices: &[usize],
        pivot: document::Point,
        angle: f32,
    ) -> Result<()> {
        ensure!(
            pivot.x.is_finite() && pivot.y.is_finite() && angle.is_finite(),
            "Invalid rotation"
        );
        let (sin, cos) = angle.sin_cos();
        let rotate = |x: f32, y: f32| {
            let (dx, dy) = (x - pivot.x, y - pivot.y);
            (pivot.x + cos * dx - sin * dy, pivot.y + sin * dx + cos * dy)
        };
        let valid = |values: &[f32]| values.iter().all(|v| v.is_finite() && v.abs() < 1_000_000.);
        let indices = indices
            .iter()
            .copied()
            .collect::<std::collections::BTreeSet<_>>();
        ensure!(!indices.is_empty(), "Select artwork to rotate");
        let mut replacements = Vec::with_capacity(indices.len());
        for index in indices {
            let mut layer = self
                .layers
                .get(index)
                .context("Layer no longer exists")?
                .clone();
            if let Some(image) = &mut layer.image {
                let [a, b, c, d, x, y] = image.transform;
                let (x, y) = rotate(x, y);
                let transform = [
                    cos * a - sin * b,
                    sin * a + cos * b,
                    cos * c - sin * d,
                    sin * c + cos * d,
                    x,
                    y,
                ];
                ensure!(valid(&transform), "Rotation exceeds the supported range");
                image.transform = transform;
            }
            for stroke in &mut layer.strokes {
                for point in std::sync::Arc::make_mut(&mut stroke.points) {
                    let (x, y) = rotate(point.x, point.y);
                    ensure!(valid(&[x, y]), "Rotation exceeds the supported range");
                    point.x = x;
                    point.y = y;
                }
            }
            replacements.push((index, layer));
        }
        for (index, layer) in replacements {
            self.layers[index] = layer;
        }
        Ok(())
    }

    pub fn flip_layers(
        &mut self,
        indices: &[usize],
        horizontal: bool,
        vertical: bool,
    ) -> Result<()> {
        ensure!(horizontal || vertical, "Choose a flip direction");
        let (left, top, right, bottom) = self
            .selected_layer_bounds(indices)
            .context("Selected layers have no transformable artwork")?;
        let indices = indices
            .iter()
            .copied()
            .collect::<std::collections::BTreeSet<_>>();
        for index in &indices {
            let layer = self.layers.get(*index).context("Layer no longer exists")?;
            if let Some(image) = &layer.image {
                let [a, b, c, d, x, y] = image.transform;
                let (flipped_x, flipped_y) = (
                    if horizontal { left + right - x } else { x },
                    if vertical { top + bottom - y } else { y },
                );
                ensure!(
                    [a, b, c, d, flipped_x, flipped_y]
                        .iter()
                        .all(|v| v.is_finite() && v.abs() < 1_000_000.),
                    "Layer position limit reached"
                );
            }
            ensure!(
                layer
                    .strokes
                    .iter()
                    .flat_map(|stroke| stroke.points.iter())
                    .all(|point| {
                        let x = if horizontal {
                            left + right - point.x
                        } else {
                            point.x
                        };
                        let y = if vertical {
                            top + bottom - point.y
                        } else {
                            point.y
                        };
                        x.is_finite()
                            && y.is_finite()
                            && x.abs() < 1_000_000.
                            && y.abs() < 1_000_000.
                    }),
                "Layer position limit reached"
            );
        }
        for index in indices {
            let layer = self
                .layers
                .get_mut(index)
                .context("Layer no longer exists")?;
            if let Some(image) = &mut layer.image {
                let [mut a, mut b, mut c, mut d, mut x, mut y] = image.transform;
                if horizontal {
                    a = -a;
                    c = -c;
                    x = left + right - x;
                }
                if vertical {
                    b = -b;
                    d = -d;
                    y = top + bottom - y;
                }
                image.transform = [a, b, c, d, x, y];
            }
            for stroke in &mut layer.strokes {
                for point in std::sync::Arc::make_mut(&mut stroke.points) {
                    if horizontal {
                        point.x = left + right - point.x;
                    }
                    if vertical {
                        point.y = top + bottom - point.y;
                    }
                }
            }
        }
        Ok(())
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
                let id = self.allocate_canvas_id()?;
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
                    paint_layer: None,
                    layer_groups: Default::default(),
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
        // Some early development builds wrote these optional fields as version 2.
        if self.version == 2 && self.uses_extended_layer_features() {
            self.version = 3;
        }
        Ok(())
    }

    /// Apply a review decision to the active candidate layer.
    pub(crate) fn review_candidate_layer(&mut self, review: Review) {
        debug_assert!(self.layers[self.active_layer].candidate);
        match review {
            Review::Accept => self.layers[self.active_layer].candidate = false,
            Review::Discard | Review::Reference => self.remove_reviewed_candidate(),
        }
        self.prune_layer_groups();
    }

    fn remove_reviewed_candidate(&mut self) {
        if self.layers.len() == 1 {
            self.layers[0] = Layer {
                name: "Paint".into(),
                visible: true,
                locked: false,
                opacity: 1.,
                group: None,
                candidate: false,
                strokes: vec![],
                image: None,
            };
            self.paint_layer = Some(0);
        } else {
            let removed = self.active_layer;
            self.layers.remove(removed);
            self.paint_layer = self.paint_layer.and_then(|index| {
                if index == removed {
                    None
                } else if index > removed {
                    Some(index - 1)
                } else {
                    Some(index)
                }
            });
            self.active_layer = self.active_layer.min(self.layers.len() - 1);
        }
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
                next.layer_groups.clear();
                next.active_layer = 0;
                next.paint_layer = None;
                // Background completion must not switch away from artwork the
                // user is currently editing. Replace only an empty placeholder.
                if let Some(index) = next.canvases.iter().position(|c| c.id == previous_id) {
                    let generated = next.current_canvas();
                    let previous = std::mem::replace(&mut next.canvases[index], generated);
                    next.set_current_canvas(previous);
                }
            }
            Destination::Candidate { artboard, rect }
            | Destination::NewLayer { artboard, rect } => {
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
                let mut layer = image_layer(reference, width, height, rect, true)?;
                if matches!(destination, Destination::NewLayer { .. }) {
                    let scale = (rect.2 as f32 / width as f32).min(rect.3 as f32 / height as f32);
                    layer.image.as_mut().unwrap().transform = [
                        scale,
                        0.,
                        0.,
                        scale,
                        rect.0 as f32 + (rect.2 as f32 - width as f32 * scale) / 2.,
                        rect.1 as f32 + (rect.3 as f32 - height as f32 * scale) / 2.,
                    ];
                }
                layers.push(layer);
                *active = layers.len() - 1;
            }
        }
        next.validate()?;
        *self = next;
        Ok(())
    }
}

pub(crate) fn image_layer(
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
        locked: false,
        opacity: 1.,
        group: None,
        candidate,
        strokes: vec![],
        image: Some(ImageLayer {
            reference,
            crop: None,
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
