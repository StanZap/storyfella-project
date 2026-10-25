//! Image inspection, library reuse, and durable editing drafts.
use crate::{
    app::{DrawingApp, Page},
    document::{self, Brush, Layer, Point as DocPoint, Stroke, Tool},
    worker::{self, Storage},
};
use gpui_kit::*;
use std::sync::Arc;
impl DrawingApp {
    pub fn review_candidate(
        &mut self,
        action: crate::workflow::Review,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let layer = &self.doc.layers[self.doc.active_layer];
        if !layer.candidate {
            return;
        }
        let reference = layer.image.as_ref().map(|i| i.reference.clone());
        if matches!(action, crate::workflow::Review::Reference) && self.doc.references.len() >= 128
        {
            self.message("Maximum 128 references", true);
            return;
        }
        self.checkpoint();
        match action {
            crate::workflow::Review::Accept => {
                self.doc.layers[self.doc.active_layer].candidate = false
            }
            crate::workflow::Review::Reference => {
                if let Some(reference) = reference {
                    if !self
                        .doc
                        .references
                        .iter()
                        .any(|r| r.image == reference.image)
                    {
                        if let Ok(decoded) = document::decode_reference(&reference) {
                            let preview =
                                worker::render_image(document::thumbnail(decoded, 800).to_rgba8());
                            self.references.push(preview.clone());
                            self.reference_cache.push((reference.clone(), preview));
                            self.doc.references.push(reference.clone());
                        }
                    }
                    let _ = self.workers.storage.send(Storage::LibraryAdd(
                        self.data_dir.join("library"),
                        vec![reference],
                    ));
                }
                self.doc.layers.remove(self.doc.active_layer);
                self.doc.active_layer = self.doc.active_layer.min(self.doc.layers.len() - 1);
            }
            crate::workflow::Review::Discard => {
                self.doc.layers.remove(self.doc.active_layer);
                self.doc.active_layer = self.doc.active_layer.min(self.doc.layers.len() - 1);
            }
        }
        self.changed();
        self.sync_layer_controls(window, cx);
        cx.notify();
    }

    pub fn layer_variation(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.capture_prompt(cx);
        let Some(reference) = self.doc.layers[self.doc.active_layer]
            .image
            .as_ref()
            .map(|i| i.reference.clone())
        else {
            return;
        };
        let previous = self.doc.clone();
        match self
            .doc
            .attach_result(reference, crate::workflow::Destination::NewArtboard)
        {
            Ok(()) => {
                self.history.checkpoint(&previous);
                self.changed();
                let newest = self
                    .doc
                    .canvases
                    .iter()
                    .map(|c| c.id)
                    .chain([self.doc.canvas_id])
                    .max()
                    .unwrap_or(self.doc.canvas_id);
                self.open_canvas_artboard(newest, window, cx);
            }
            Err(error) => self.message(error.to_string(), true),
        }
        cx.notify();
    }
    pub fn edit_source(&self) -> Option<Arc<document::Reference>> {
        if self.page == Page::Image {
            if self.opening_image.is_some() {
                return None;
            }
            self.opened_image
                .as_ref()
                .map(|(reference, _, _, _)| reference.clone())
        } else {
            self.doc
                .reference
                .and_then(|i| self.doc.references.get(i))
                .cloned()
        }
    }
    pub fn refresh_library(&mut self) {
        if self.library_loading {
            return;
        }
        self.library_loading = true;
        self.library_error = None;
        if self
            .workers
            .storage
            .send(Storage::Library(self.data_dir.join("library")))
            .is_err()
        {
            self.library_loading = false;
            self.library_error = Some("Library worker stopped. Restart Atelier to retry.".into());
        }
    }
    pub fn input_references(&self) -> Vec<Arc<document::Reference>> {
        self.doc
            .artboard_references
            .iter()
            .filter_map(|id| {
                self.doc
                    .references
                    .iter()
                    .find(|r| crate::library::id(r) == *id)
                    .cloned()
            })
            .collect()
    }

    pub fn choose_reference(
        &mut self,
        reference: Arc<document::Reference>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let id = crate::library::id(&reference);
        if !self.doc.artboard_references.contains(&id) {
            if self.doc.artboard_references.len() >= 2 {
                self.message(
                    "At most two additional references; remove one in the composer first",
                    true,
                );
                return;
            }
            self.checkpoint();
            let index = if let Some(i) = self
                .doc
                .references
                .iter()
                .position(|r| r.image == reference.image)
            {
                i
            } else {
                if self.doc.references.len() >= 128
                    || self.doc.embedded_image_bytes() + reference.image.len() > 64 * 1024 * 1024
                {
                    self.message("Project reference limit reached", true);
                    return;
                }
                let Ok(decoded) = document::decode_reference(&reference) else {
                    return;
                };
                let preview = worker::render_image(document::thumbnail(decoded, 800).to_rgba8());
                self.doc.references.push(reference.clone());
                self.references.push(preview.clone());
                self.reference_cache.push((reference.clone(), preview));
                self.doc.references.len() - 1
            };
            self.doc.artboard_references.push(id);
            self.doc.reference = Some(index);
            self.doc.references_visible = true;
            self.changed();
            self.sync_ref_inputs(window, cx);
        }
        self.panel = crate::app::Panel::None;
        self.dock = true;
        self.message("Reference selected · excluded from artwork exports", false);
        cx.notify();
    }
    pub fn toggle_comparison(&mut self) {
        self.finish_mask();
        if self.comparing {
            self.comparing = false;
            return;
        }
        let Some(parent) = self
            .opened_image
            .as_ref()
            .and_then(|(r, _, _, _)| r.parent.clone())
        else {
            return;
        };
        if self
            .comparison
            .as_ref()
            .is_some_and(|(id, _, _, _)| *id == parent)
        {
            self.comparing = true;
            return;
        }
        let Some(reference) = self
            .all_references()
            .into_iter()
            .find(|r| crate::library::id(r) == parent)
        else {
            self.message("The original version is missing from the library", true);
            return;
        };
        self.comparison_requested = Some(parent);
        if self
            .workers
            .storage
            .send(Storage::CompareImage(reference.clone()))
            .is_err()
        {
            self.comparison_requested = None;
            self.message("Image worker stopped", true);
        }
    }
    fn mask_point(&self, position: gpui_kit::Point<Pixels>) -> Option<DocPoint> {
        let (_, _, width, height) = self.opened_image.as_ref()?;
        let bounds = self.image_bounds.get();
        let x = (f32::from(position.x - bounds.origin.x)
            - (f32::from(bounds.size.width) - *width as f32 * self.image_zoom) / 2.
            - self.image_pan.x)
            / self.image_zoom;
        let y = (f32::from(position.y - bounds.origin.y)
            - (f32::from(bounds.size.height) - *height as f32 * self.image_zoom) / 2.
            - self.image_pan.y)
            / self.image_zoom;
        (x >= 0. && y >= 0. && x < *width as f32 && y < *height as f32).then_some(DocPoint { x, y })
    }
    pub fn mask_down(&mut self, position: gpui_kit::Point<Pixels>) {
        if self.opening_image.is_some() || self.generation.is_some() || self.comparing {
            return;
        }
        if self.mask_tool == Tool::Hand {
            self.image_auto_fit = false;
            self.image_drag = Some(position);
            return;
        }
        let Some(point) = self.mask_point(position) else {
            return;
        };
        if let Some(mask) = &mut self.mask {
            if mask.strokes.iter().map(|s| s.points.len()).sum::<usize>() >= 2_000_000 {
                self.message("Mask point limit reached. Undo or clear the mask.", true);
                return;
            }
            let points: Vec<DocPoint> = if self.mask_tool == Tool::Rect {
                vec![point, point]
            } else {
                vec![point]
            };
            mask.strokes.push(Stroke {
                brush: Brush::Ink,
                pressures: vec![].into(),
                tool: self.mask_tool,
                size: self.mask_size,
                opacity: 1.,
                points: points.into(),
            });
            self.mask_active = true;
            self.use_mask = true;
            self.mask_redo.clear();
            self.mask_dirty = true;
            self.refresh_mask_preview();
        }
    }
    pub fn mask_move(&mut self, position: gpui_kit::Point<Pixels>) {
        if !self.mask_active {
            return;
        }
        if self.mask_tool == Tool::Rect {
            // A rectangle drags its second corner; a stray point outside the
            // image is ignored rather than ending the selection.
            let Some(point) = self.mask_point(position) else {
                return;
            };
            if let Some(stroke) = self.mask.as_mut().and_then(|m| m.strokes.last_mut()) {
                if stroke.tool == Tool::Rect && stroke.points.len() == 2 {
                    Arc::make_mut(&mut stroke.points)[1] = point;
                    self.mask_dirty = true;
                    self.refresh_mask_preview();
                }
            }
            return;
        }
        let Some(point) = self.mask_point(position) else {
            self.finish_mask();
            return;
        };
        if let Some(stroke) = self.mask.as_mut().and_then(|m| m.strokes.last_mut()) {
            if stroke.points.len() >= 100_000 {
                self.finish_mask();
                return;
            }
            Arc::make_mut(&mut stroke.points).push(point);
            self.mask_dirty = true;
            self.refresh_mask_preview();
        }
    }
    pub fn refresh_mask_preview(&mut self) {
        self.mask_revision += 1;
        if let Some(mask) = &self.mask {
            let _ = self.workers.mask.send((self.mask_revision, mask.clone()));
        }
    }
    pub fn finish_mask(&mut self) {
        if !self.mask_active {
            return;
        }
        self.mask_active = false;
        self.save_mask();
    }
    pub fn save_mask(&mut self) {
        if let (Some((reference, _, _, _)), Some(mask)) = (&self.opened_image, &self.mask) {
            if self
                .workers
                .storage
                .send(Storage::MaskDraft(
                    self.data_dir.join("library"),
                    crate::library::id(reference),
                    self.mask_revision,
                    mask.clone(),
                ))
                .is_err()
            {
                let error = "Mask could not be saved. Restart Atelier to retry.".to_owned();
                self.mask_save_error = Some(error.clone());
                self.closing = false;
                self.opening_image = None;
                self.message(error, true);
            }
        }
    }
    pub fn undo_mask(&mut self, redo: bool) {
        self.finish_mask();
        if let Some(mask) = &mut self.mask {
            if redo {
                if let Some(stroke) = self.mask_redo.pop() {
                    mask.strokes.push(stroke);
                }
            } else if let Some(stroke) = mask.strokes.pop() {
                self.mask_redo.push(stroke);
            }
            self.refresh_mask_preview();
            self.mask_dirty = true;
            self.save_mask();
        }
    }

    pub fn open_image(&mut self, reference: Arc<document::Reference>, cx: &mut Context<Self>) {
        self.finish_mask();
        if self.mask_save_error.is_some() {
            self.page = Page::Image;
            self.message("Save the current mask before opening another image", true);
            cx.notify();
            return;
        }
        self.finish_stroke();
        if self.page != Page::Image {
            self.image_return_page = self.page;
        }
        self.page = Page::Image;
        self.show_edit_composer = false;
        self.editing_image = self.settings.provider.supports_edit();
        self.comparing = false;
        self.comparison_requested = None;
        self.mask_tool = Tool::Hand;
        self.image_zoom = 1.;
        self.image_auto_fit = true;
        self.image_pan = DocPoint { x: 0., y: 0. };
        self.opening_image = Some(reference.clone());
        if self
            .workers
            .storage
            .send(Storage::OpenImage(reference, self.data_dir.join("library")))
            .is_err()
        {
            self.opening_image = None;
            self.message("Image worker stopped. Restart Atelier to retry.", true);
        }
        cx.notify();
    }
    pub fn image_fit(&mut self) {
        self.image_auto_fit = true;
        if let Some((_, _, width, height)) = &self.opened_image {
            let bounds = self.image_bounds.get();
            self.image_zoom = ((f32::from(bounds.size.width) - 48.) / *width as f32)
                .min((f32::from(bounds.size.height) - 48.) / *height as f32)
                .clamp(0.02, 8.);
            self.image_pan = DocPoint { x: 0., y: 0. };
        }
    }

    pub fn back_from_image(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.finish_mask();
        if self.mask_save_error.is_some() {
            return;
        }
        self.page = self.image_return_page;
        if self.page == Page::Artboards {
            self.editing_image = false;
            self.restore_prompt(window, cx);
        }
        self.show_edit_composer = false;
        self.panel = crate::app::Panel::None;
        cx.notify();
    }
    pub fn edit_opened_image(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.ensure_project_tab();
        if self.edit_source().is_some() {
            self.editing_image = true;
            self.restore_prompt(window, cx);
            self.prompt.update(cx, |s, cx| s.focus(window, cx));
        }
    }
    pub fn add_opened_image_layer(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some((reference, _, width, height)) = self.opened_image.clone() else {
            return;
        };
        if self.doc.layers.len() >= 24 {
            self.message("Maximum 24 layers", true);
            return;
        }
        let embedded = self.doc.embedded_image_bytes();
        if embedded + reference.image.len() > 64 * 1024 * 1024 {
            self.message(
                "Project images exceed 64 MB. Use a smaller image or remove unused project images.",
                true,
            );
            return;
        }
        let scale =
            (self.doc.width as f32 / width as f32).min(self.doc.height as f32 / height as f32);
        self.checkpoint();
        self.doc.layers.push(Layer {
            name: reference.name.clone(),
            visible: true,
            opacity: 1.,
            candidate: false,
            strokes: vec![],
            image: Some(document::ImageLayer {
                reference,
                transform: [
                    scale,
                    0.,
                    0.,
                    scale,
                    (self.doc.width as f32 - width as f32 * scale) / 2.,
                    (self.doc.height as f32 - height as f32 * scale) / 2.,
                ],
            }),
        });
        self.doc.active_layer = self.doc.layers.len() - 1;
        self.changed();
        self.page = Page::Draw;
        self.dock = true;
        self.tool = Tool::Pencil;
        window.focus(&self.focus, cx);
        self.message("Image layer added · draw or erase on this layer", false);
        cx.notify();
    }
}
