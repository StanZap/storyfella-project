use super::{ui_color, DockView, DrawingApp, PaintColor, Panel};
use crate::{
    document::{self, Color, Document, Layer, Point as DocPoint, Stroke, Tool},
    shapes::{self, ShapeKind},
};
use gpui_kit::{component::color_picker::ColorPickerState, Context, Entity, Window};

impl DrawingApp {
    pub(crate) fn color_picker(&self, target: PaintColor) -> &Entity<ColorPickerState> {
        &self.color_pickers[target.index()]
    }

    pub(crate) fn clip_paint_to_shape(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(id) = self.selected_shape else {
            return;
        };
        let previous = self.doc.clone();
        match self.doc.add_clipped_layer(id) {
            Ok(layer) => {
                self.history.checkpoint(&previous);
                self.tool = Tool::Pencil;
                self.panel = Panel::None;
                self.dock = true;
                self.dock_view = DockView::Layers;
                self.select_layer(layer, window, cx);
                self.changed();
            }
            Err(error) => self.message(format!("Could not clip paint: {error:#}"), false),
        }
        cx.notify();
    }

    pub(crate) fn clear_paint_clip(&mut self, layer: usize, cx: &mut Context<Self>) {
        let previous = self.doc.clone();
        match self.doc.clear_layer_clip(layer) {
            Ok(()) => {
                self.history.checkpoint(&previous);
                self.changed();
            }
            Err(error) => self.message(format!("Could not clear the clip: {error:#}"), false),
        }
        cx.notify();
    }

    pub(crate) fn delete_selected_shape(&mut self, cx: &mut Context<Self>) {
        let Some(id) = self.selected_shape else {
            return;
        };
        let previous = self.doc.clone();
        match self.doc.remove_shape(id) {
            Ok(()) => {
                self.history.checkpoint(&previous);
                self.selected_shape = None;
                self.changed();
            }
            Err(error) => self.message(format!("Could not delete the shape: {error:#}"), false),
        }
        cx.notify();
    }

    pub(crate) fn art_color(&self, target: PaintColor) -> Color {
        match target {
            PaintColor::Brush => self.paint_color,
            PaintColor::Fill => self.shape_fill,
            PaintColor::Outline => self.shape_outline,
        }
    }

    pub(crate) fn choose_art_color(
        &mut self,
        target: PaintColor,
        color: Color,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.set_art_color(target, color, cx);
        let applied = self.art_color(target);
        self.color_picker(target).update(cx, |picker, cx| {
            picker.set_value(ui_color(applied), window, cx)
        });
    }

    fn set_art_color(&mut self, target: PaintColor, color: Color, cx: &mut Context<Self>) {
        if self.art_color(target) == color {
            return;
        }
        if target != PaintColor::Brush {
            if let Some(mut stroke) = self.selected_shape().cloned() {
                let shape = stroke.shape.as_mut().expect("Selected shape");
                if target == PaintColor::Fill {
                    shape.fill = color;
                } else {
                    stroke.color = color;
                }
                if !self.commit_shape_edit(stroke) {
                    cx.notify();
                    return;
                }
            }
        }
        match target {
            PaintColor::Brush => self.paint_color = color,
            PaintColor::Fill => self.shape_fill = color,
            PaintColor::Outline => self.shape_outline = color,
        }
        self.recent_colors.retain(|&item| item != color);
        self.recent_colors.insert(0, color);
        self.recent_colors.truncate(8);
        cx.notify();
    }

    pub(crate) fn toggle_palette_color(&mut self, color: Color) {
        if let Some(index) = self.doc.palette.iter().position(|&item| item == color) {
            self.checkpoint();
            self.doc.palette.remove(index);
        } else if self.doc.palette.len() < 12 {
            self.checkpoint();
            self.doc.palette.push(color);
        } else {
            self.message("The palette holds up to 12 colors", false);
            return;
        }
        self.changed();
    }

    pub(super) fn sample_art_color(
        &mut self,
        target: PaintColor,
        point: DocPoint,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match document::raster(&self.doc.artwork(), document::MAX_EDGE, false) {
            Ok(image) => {
                let x = (point.x as u32).min(image.width() - 1);
                let y = (point.y as u32).min(image.height() - 1);
                self.choose_art_color(target, image.get_pixel(x, y).0, window, cx);
            }
            Err(error) => self.message(format!("Could not sample the artboard: {error:#}"), true),
        }
    }

    pub(crate) fn selected_shape(&self) -> Option<&Stroke> {
        let (layer, index) = self.doc.shape_location(self.selected_shape?)?;
        Some(&self.doc.layers[layer].strokes[index])
    }

    pub(crate) fn choose_shape_kind(&mut self, kind: ShapeKind) {
        self.finish_stroke();
        self.eyedropper = None;
        self.selected_shape = None;
        self.tool = Tool::Shape;
        self.moving_image = false;
        self.selecting_generation_region = false;
        self.shape_kind = kind;
    }

    pub(super) fn select_shape_at(
        &mut self,
        point: DocPoint,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.selected_shape = self
            .doc
            .layers
            .iter()
            .rev()
            .filter(|layer| {
                !layer.candidate
                    && !self.doc.layer_is_locked(layer)
                    && self.doc.layer_is_visible(layer)
                    && layer.clip_shape.is_none_or(|id| {
                        shapes::clip_source(&self.doc.layers, id)
                            .is_some_and(|clip| shapes::contains(clip, point))
                    })
            })
            .flat_map(|layer| layer.strokes.iter().rev())
            .find(|stroke| shapes::hit(stroke, point, 5.))
            .and_then(|stroke| stroke.shape.as_ref().map(|shape| shape.id));
        if let Some(stroke) = self.selected_shape().cloned() {
            let shape = stroke.shape.expect("Selected shape");
            self.shape_fill = shape.fill;
            self.shape_outline = stroke.color;
            self.shape_width = stroke.size;
            for target in [PaintColor::Fill, PaintColor::Outline] {
                let color = ui_color(self.art_color(target));
                self.color_picker(target)
                    .update(cx, |picker, cx| picker.set_value(color, window, cx));
            }
            self.panel = Panel::Shapes;
        }
    }

    pub(crate) fn adjust_shape_width(&mut self, factor: f32) {
        let width = (self.shape_width * factor).clamp(0.5, 256.);
        if let Some(mut stroke) = self.selected_shape().cloned() {
            stroke.size = width;
            if !self.commit_shape_edit(stroke) {
                return;
            }
        }
        self.shape_width = width;
    }

    pub(crate) fn transform_shape(&mut self, scale: f32, angle: f32) {
        if let Some(mut stroke) = self.selected_shape().cloned() {
            shapes::transform(&mut stroke, scale, angle, 0., 0.);
            let size = stroke.size;
            if self.commit_shape_edit(stroke) {
                self.shape_width = size;
            }
        }
    }

    fn commit_shape_edit(&mut self, stroke: Stroke) -> bool {
        let id = stroke.shape.as_ref().expect("Shape to edit").id;
        let previous = self.doc.clone();
        if let Err(error) = self.doc.replace_shape(id, stroke) {
            self.message(format!("Could not edit the shape: {error:#}"), false);
            return false;
        }
        self.history.checkpoint(&previous);
        self.changed();
        true
    }

    pub(super) fn constrain_shape(&self, start: DocPoint, end: DocPoint) -> DocPoint {
        let (x, y) = (end.x - start.x, end.y - start.y);
        if self.shape_kind == ShapeKind::Line {
            let step = std::f32::consts::FRAC_PI_4;
            let angle = (y.atan2(x) / step).round() * step;
            return DocPoint {
                x: start.x + x.hypot(y) * angle.cos(),
                y: start.y + x.hypot(y) * angle.sin(),
            };
        }
        let edge = x.abs().max(y.abs());
        DocPoint {
            x: start.x + edge.copysign(x),
            y: start.y + edge.copysign(y),
        }
    }

    pub(crate) fn shape_preview(&self) -> Option<Stroke> {
        let (start, end, moving) = self.shape_drag?;
        if moving {
            let mut stroke = self.selected_shape()?.clone();
            shapes::transform(&mut stroke, 1., 0., end.x - start.x, end.y - start.y);
            Some(stroke)
        } else {
            shapes::stroke(
                self.shape_kind,
                start,
                end,
                self.shape_fill,
                self.shape_outline,
                self.shape_width,
            )
        }
    }

    pub(crate) fn finish_shape(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let preview = self.shape_preview();
        let Some((start, end, moving)) = self.shape_drag.take() else {
            return;
        };
        let moved = (end.x - start.x).hypot(end.y - start.y) >= 1.;
        if !moved {
            if !moving {
                self.select_shape_at(start, window, cx);
            }
            return;
        }
        let Some(stroke) = preview else { return };
        if moving {
            self.commit_shape_edit(stroke);
            return;
        }
        let id = stroke.shape.as_ref().expect("New shape").id;
        let previous = self.doc.clone();
        let mut next = previous.clone();
        let layer = if let Some(layer) = next.paint_layer {
            layer
        } else if next.layers[next.active_layer].image.is_none()
            && !next.layers[next.active_layer].candidate
        {
            next.active_layer
        } else if next.layers.len() < 24 {
            next.layers.push(Layer {
                name: "Shapes".into(),
                visible: true,
                locked: false,
                opacity: 1.,
                group: None,
                candidate: false,
                clip_shape: None,
                strokes: Vec::new(),
                image: None,
            });
            next.layers.len() - 1
        } else {
            self.message("Maximum 24 layers. Choose a paint target in Layers.", false);
            return;
        };
        if let Err(error) = next.add_mark(layer, stroke) {
            self.message(format!("Could not add the shape: {error:#}"), false);
            return;
        }
        next.paint_layer = Some(layer);
        self.history.checkpoint(&previous);
        self.doc = next;
        self.selected_shape = Some(id);
        self.changed();
        self.sync_layer_controls(window, cx);
    }

    #[cfg(unix)]
    pub(crate) fn drawing_gesture_active(&self) -> bool {
        self.active_stroke
            || self.transform_original.is_some()
            || self.artboard_resize_drag.is_some()
            || self.region_drag.is_some()
            || self.image_crop.is_some()
            || self.shape_drag.is_some()
    }

    #[cfg(unix)]
    pub(crate) fn apply_mcp_artwork(
        &mut self,
        doc: Document,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.checkpoint();
        let added_layer = doc.layers.len() > self.doc.layers.len();
        self.doc = doc;
        self.selected_shape = None;
        self.changed();
        if added_layer {
            self.select_layer(self.doc.active_layer, window, cx);
        } else {
            self.sync_layer_controls(window, cx);
        }
        cx.notify();
    }
}
