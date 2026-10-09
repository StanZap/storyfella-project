use anyhow::{ensure, Context, Result};
use base64::{engine::general_purpose::STANDARD, Engine};
use image::{DynamicImage, RgbaImage};
use serde::{Deserialize, Serialize};
use std::{fs, io::Write, path::Path};

pub const MAX_EDGE: u32 = 8192;
pub const MAX_PIXELS: u64 = 32_000_000;
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq)]
pub struct Point {
    pub x: f32,
    pub y: f32,
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum Tool {
    Pencil,
    Eraser,
    Hand,
}
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub enum Brush {
    Graphite,
    #[default]
    Ink,
    Charcoal,
    Marker,
}
impl Brush {
    pub const ALL: [Self; 4] = [Self::Graphite, Self::Ink, Self::Charcoal, Self::Marker];
    pub fn name(self) -> &'static str {
        match self {
            Self::Graphite => "Graphite pencil",
            Self::Ink => "Ink",
            Self::Charcoal => "Charcoal",
            Self::Marker => "Marker",
        }
    }
    pub fn size(self) -> f32 {
        match self {
            Self::Graphite => 5.,
            Self::Ink => 8.,
            Self::Charcoal => 28.,
            Self::Marker => 36.,
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Stroke {
    #[serde(default)]
    pub brush: Brush,
    /// Empty on older projects and mouse strokes; otherwise one normalized value per point.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pressures: std::sync::Arc<Vec<f32>>,
    pub tool: Tool,
    pub size: f32,
    pub opacity: f32,
    pub points: std::sync::Arc<Vec<Point>>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Layer {
    pub name: String,
    pub visible: bool,
    pub opacity: f32,
    pub strokes: Vec<Stroke>,
    #[serde(default)]
    pub image: Option<ImageLayer>,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ImageLayer {
    pub reference: std::sync::Arc<Reference>,
    /// Original pixel coordinates to canvas coordinates: a,b,c,d,tx,ty.
    pub transform: [f32; 6],
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Reference {
    #[serde(skip)]
    pub fingerprint: String,
    pub name: String,
    pub image: String,
    pub favorite: bool,
    pub tags: String,
    pub provenance: String,
    #[serde(default)]
    pub parent: Option<String>,
    #[serde(default)]
    pub prompt: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Document {
    pub version: u32,
    pub title: String,
    pub width: u32,
    pub height: u32,
    pub layers: Vec<Layer>,
    pub active_layer: usize,
    pub references: Vec<std::sync::Arc<Reference>>,
    pub reference: Option<usize>,
    pub note: String,
}
impl Default for Document {
    fn default() -> Self {
        Self {
            version: 1,
            title: "Untitled study".into(),
            width: 2400,
            height: 1800,
            layers: vec![Layer {
                name: "Sketch".into(),
                visible: true,
                opacity: 1.,
                strokes: vec![],
                image: None,
            }],
            active_layer: 0,
            references: vec![],
            reference: None,
            note: String::new(),
        }
    }
}
impl Document {
    pub fn embedded_image_bytes(&self) -> usize {
        self.references.iter().map(|r| r.image.len()).sum::<usize>()
            + self
                .layers
                .iter()
                .filter_map(|l| l.image.as_ref())
                .map(|i| i.reference.image.len())
                .sum::<usize>()
    }
    pub fn reposition_image_layer(&mut self, dx: f32, dy: f32, scale: f32) -> Result<()> {
        ensure!(
            dx.is_finite() && dy.is_finite() && scale.is_finite() && scale > 0.,
            "Invalid layer movement"
        );
        let mut layer = self.layers[self.active_layer].clone();
        let image = layer
            .image
            .as_mut()
            .context("Select an image layer first")?;
        let cx = self.width as f32 / 2.;
        let cy = self.height as f32 / 2.;
        let t = &mut image.transform;
        for value in &mut t[..4] {
            *value *= scale;
        }
        t[4] = (t[4] - cx) * scale + cx + dx;
        t[5] = (t[5] - cy) * scale + cy + dy;
        ensure!(
            t.iter().all(|v| v.is_finite() && v.abs() < 1_000_000.)
                && (t[0] * t[3] - t[1] * t[2]).abs() > 0.000001,
            "Image scale or position limit reached"
        );
        for stroke in &mut layer.strokes {
            stroke.size = (stroke.size * scale).clamp(0.1, 512.);
            for point in std::sync::Arc::make_mut(&mut stroke.points) {
                point.x = (point.x - cx) * scale + cx + dx;
                point.y = (point.y - cy) * scale + cy + dy;
                ensure!(
                    point.x.is_finite()
                        && point.y.is_finite()
                        && point.x.abs() < 1_000_000.
                        && point.y.abs() < 1_000_000.,
                    "Layer position limit reached"
                );
            }
        }
        self.layers[self.active_layer] = layer;
        Ok(())
    }
    pub fn validate(&self) -> Result<()> {
        ensure!(self.version == 1, "Unsupported project version");
        validate_size(self.width, self.height)?;
        ensure!(
            !self.layers.is_empty()
                && self.layers.len() <= 24
                && self.active_layer < self.layers.len(),
            "Invalid layers"
        );
        ensure!(
            self.references.len() <= 128
                && self.reference.is_none_or(|i| i < self.references.len()),
            "Invalid references"
        );
        ensure!(
            self.embedded_image_bytes() <= 64 * 1024 * 1024,
            "Embedded references exceed 64 MB"
        );
        let mut points = 0;
        for layer in &self.layers {
            if let Some(image) = &layer.image {
                ensure!(
                    image
                        .transform
                        .iter()
                        .all(|v| v.is_finite() && v.abs() < 1_000_000.),
                    "Invalid image layer transform"
                );
                ensure!(
                    (image.transform[0] * image.transform[3]
                        - image.transform[1] * image.transform[2])
                        .abs()
                        > 0.000001,
                    "Image layer transform has no area"
                );
                decode_reference(&image.reference)?;
            }
            ensure!(
                layer.opacity.is_finite() && (0. ..=1.).contains(&layer.opacity),
                "Invalid layer opacity"
            );
            for stroke in &layer.strokes {
                ensure!(
                    stroke.size.is_finite()
                        && (0.1..=512.).contains(&stroke.size)
                        && stroke.opacity.is_finite()
                        && (0. ..=1.).contains(&stroke.opacity)
                        && stroke.tool != Tool::Hand,
                    "Invalid stroke"
                );
                ensure!(
                    stroke.pressures.is_empty() || stroke.pressures.len() == stroke.points.len(),
                    "Invalid pressure samples"
                );
                ensure!(
                    stroke
                        .pressures
                        .iter()
                        .all(|p| p.is_finite() && (0. ..=1.).contains(p)),
                    "Invalid pen pressure"
                );
                points += stroke.points.len();
                ensure!(points <= 2_000_000, "Project has too many stroke points");
                ensure!(
                    stroke.points.iter().all(|p| p.x.is_finite()
                        && p.y.is_finite()
                        && p.x.abs() < 1_000_000.
                        && p.y.abs() < 1_000_000.),
                    "Invalid stroke coordinates"
                );
            }
        }
        for reference in &self.references {
            decode_reference(reference)?;
        }
        Ok(())
    }
    pub fn resize(&mut self, width: u32, height: u32, scale_art: bool) -> Result<()> {
        validate_size(width, height)?;
        if scale_art {
            let sx = width as f32 / self.width as f32;
            let sy = height as f32 / self.height as f32;
            for layer in &mut self.layers {
                if let Some(image) = &mut layer.image {
                    let t = &mut image.transform;
                    t[0] *= sx;
                    t[2] *= sx;
                    t[4] *= sx;
                    t[1] *= sy;
                    t[3] *= sy;
                    t[5] *= sy;
                }
                for stroke in &mut layer.strokes {
                    stroke.size = (stroke.size * (sx * sy).sqrt()).clamp(0.1, 512.);
                    for p in std::sync::Arc::make_mut(&mut stroke.points) {
                        p.x *= sx;
                        p.y *= sy;
                    }
                }
            }
        }
        self.width = width;
        self.height = height;
        Ok(())
    }
    pub fn rotate(&mut self) {
        let h = self.height as f32;
        for layer in &mut self.layers {
            if let Some(image) = &mut layer.image {
                let [a, b, c, d, x, y] = image.transform;
                image.transform = [-b, a, -d, c, h - y, x];
            }
            for stroke in &mut layer.strokes {
                for p in std::sync::Arc::make_mut(&mut stroke.points) {
                    *p = Point { x: h - p.y, y: p.x };
                }
            }
        }
        std::mem::swap(&mut self.width, &mut self.height);
    }
    pub fn flip(&mut self) {
        for layer in &mut self.layers {
            if let Some(image) = &mut layer.image {
                let [a, b, c, d, x, y] = image.transform;
                image.transform = [-a, b, -c, d, self.width as f32 - x, y];
            }
            for stroke in &mut layer.strokes {
                for p in std::sync::Arc::make_mut(&mut stroke.points) {
                    p.x = self.width as f32 - p.x;
                }
            }
        }
    }
}
pub fn validate_size(w: u32, h: u32) -> Result<()> {
    ensure!(
        w >= 64 && h >= 64 && w <= MAX_EDGE && h <= MAX_EDGE && w as u64 * h as u64 <= MAX_PIXELS,
        "Canvas must be 64–8192 px per edge, at most 32 megapixels"
    );
    Ok(())
}
pub struct History {
    undo: Vec<Document>,
    redo: Vec<Document>,
}
impl History {
    pub fn new() -> Self {
        Self {
            undo: vec![],
            redo: vec![],
        }
    }
    pub fn checkpoint(&mut self, doc: &Document) {
        self.undo.push(doc.clone());
        if self.undo.len() > 64 {
            self.undo.remove(0);
        }
        self.redo.clear();
    }
    pub fn undo(&mut self, doc: &mut Document) -> bool {
        if let Some(previous) = self.undo.pop() {
            self.redo.push(std::mem::replace(doc, previous));
            true
        } else {
            false
        }
    }
    pub fn redo(&mut self, doc: &mut Document) -> bool {
        if let Some(next) = self.redo.pop() {
            self.undo.push(std::mem::replace(doc, next));
            true
        } else {
            false
        }
    }
    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }
    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }
}
pub fn load(path: &Path) -> Result<Document> {
    ensure!(
        fs::metadata(path)?.len() <= 128 * 1024 * 1024,
        "Project exceeds 128 MB"
    );
    let mut doc: Document =
        serde_json::from_slice(&fs::read(path)?).context("Not an Atelier project")?;
    doc.validate()?;
    for reference in &mut doc.references {
        let reference = std::sync::Arc::make_mut(reference);
        reference.fingerprint = crate::library::fingerprint(&reference.image);
    }
    for layer in &mut doc.layers {
        if let Some(image) = &mut layer.image {
            let reference = std::sync::Arc::make_mut(&mut image.reference);
            reference.fingerprint = crate::library::fingerprint(&reference.image);
        }
    }
    Ok(doc)
}
pub fn save(path: &Path, doc: &Document) -> Result<()> {
    doc.validate()?;
    let bytes = serde_json::to_vec(doc)?;
    ensure!(
        bytes.len() <= 128 * 1024 * 1024,
        "Project exceeds 128 MB; remove some references"
    );
    atomic_write(path, &bytes)
}
pub fn atomic_write(path: &Path, bytes: &[u8]) -> Result<()> {
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        fs::create_dir_all(parent)?;
    }
    let temporary = path.with_file_name(format!(
        ".{}.{}.tmp",
        path.file_name().unwrap_or_default().to_string_lossy(),
        std::process::id()
    ));
    let result = (|| {
        let mut file = fs::File::create(&temporary)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        fs::rename(&temporary, path)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}
pub fn reference_from_bytes(name: String, bytes: &[u8], provenance: String) -> Result<Reference> {
    ensure!(bytes.len() <= 20 * 1024 * 1024, "Reference exceeds 20 MB");
    let mut reader = image::ImageReader::new(std::io::Cursor::new(bytes)).with_guessed_format()?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(8192);
    limits.max_image_height = Some(8192);
    limits.max_alloc = Some(128 * 1024 * 1024);
    reader.limits(limits);
    let decoded = reader.decode().context("Unsupported or damaged image")?;
    ensure!(
        decoded.width() as u64 * decoded.height() as u64 <= MAX_PIXELS,
        "Image exceeds 32 megapixels"
    );
    let image = STANDARD.encode(bytes);
    Ok(Reference {
        fingerprint: crate::library::fingerprint(&image),
        name,
        image,
        favorite: false,
        tags: String::new(),
        provenance,
        parent: None,
        prompt: String::new(),
    })
}
pub fn thumbnail(image: DynamicImage, edge: u32) -> DynamicImage {
    if image.width().max(image.height()) > edge {
        image.thumbnail(edge, edge)
    } else {
        image
    }
}
pub fn decode_reference(reference: &Reference) -> Result<DynamicImage> {
    ensure!(
        reference.image.len() <= 28 * 1024 * 1024,
        "Reference data exceeds limit"
    );
    let bytes = STANDARD.decode(&reference.image)?;
    let mut reader = image::ImageReader::new(std::io::Cursor::new(bytes)).with_guessed_format()?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(MAX_EDGE);
    limits.max_image_height = Some(MAX_EDGE);
    limits.max_alloc = Some(128 * 1024 * 1024);
    reader.limits(limits);
    let image = reader.decode()?;
    ensure!(
        image.width() as u64 * image.height() as u64 <= MAX_PIXELS,
        "Image exceeds 32 megapixels"
    );
    Ok(image)
}
pub fn raster(doc: &Document, max_edge: u32, transparent: bool) -> Result<RgbaImage> {
    let scale = (max_edge as f32 / doc.width.max(doc.height) as f32).min(1.);
    let w = (doc.width as f32 * scale).round().max(1.) as u32;
    let h = (doc.height as f32 * scale).round().max(1.) as u32;
    let mut output = tiny_skia::Pixmap::new(w, h).context("Could not allocate canvas")?;
    if !transparent {
        output.fill(tiny_skia::Color::from_rgba8(255, 253, 248, 255));
    }
    for layer in doc.layers.iter().filter(|l| l.visible && l.opacity > 0.) {
        let mut pixels = image_layer_pixels(layer, w, h, scale)?;
        for stroke in &layer.strokes {
            if stroke.points.is_empty() {
                continue;
            }
            if stroke.brush != Brush::Ink || !stroke.pressures.is_empty() {
                paint_brush(&mut pixels, stroke, scale);
                continue;
            }
            let mut paint = tiny_skia::Paint::default();
            paint.set_color_rgba8(41, 37, 44, (stroke.opacity * 255.) as u8);
            if stroke.tool == Tool::Eraser {
                paint.blend_mode = tiny_skia::BlendMode::DestinationOut;
            }
            let radius = (stroke.size * scale / 2.).max(0.25);
            let first = stroke.points[0];
            if stroke.points.len() == 1 {
                if let Some(path) =
                    tiny_skia::PathBuilder::from_circle(first.x * scale, first.y * scale, radius)
                {
                    pixels.fill_path(
                        &path,
                        &paint,
                        tiny_skia::FillRule::Winding,
                        tiny_skia::Transform::identity(),
                        None,
                    );
                }
            } else {
                let mut path = tiny_skia::PathBuilder::new();
                path.move_to(first.x * scale, first.y * scale);
                for segment in crate::stroke_curve::segments(stroke, 1) {
                    path.line_to(segment[2] * scale, segment[3] * scale);
                }
                if let Some(path) = path.finish() {
                    pixels.stroke_path(
                        &path,
                        &paint,
                        &tiny_skia::Stroke {
                            width: radius * 2.,
                            line_cap: tiny_skia::LineCap::Round,
                            line_join: tiny_skia::LineJoin::Round,
                            ..Default::default()
                        },
                        tiny_skia::Transform::identity(),
                        None,
                    );
                }
            }
        }
        output.draw_pixmap(
            0,
            0,
            pixels.as_ref(),
            &tiny_skia::PixmapPaint {
                opacity: layer.opacity,
                ..Default::default()
            },
            tiny_skia::Transform::identity(),
            None,
        );
    }
    let mut bytes = output.take();
    // tiny-skia uses premultiplied RGBA; image encoders need straight RGBA.
    for p in bytes.chunks_exact_mut(4) {
        if p[3] != 0 {
            for i in 0..3 {
                p[i] = ((p[i] as u32 * 255 + p[3] as u32 / 2) / p[3] as u32).min(255) as u8;
            }
        }
    }
    RgbaImage::from_raw(w, h, bytes).context("Invalid raster buffer")
}
/// Shared image placement for the CPU exporter and Metal layer initialization.
pub fn image_layer_pixels(
    layer: &Layer,
    width: u32,
    height: u32,
    scale: f32,
) -> Result<tiny_skia::Pixmap> {
    let mut pixels = tiny_skia::Pixmap::new(width, height).context("Could not allocate layer")?;
    if let Some(image) = &layer.image {
        let decoded = decode_reference(&image.reference)?.to_rgba8();
        let (w, h) = decoded.dimensions();
        let mut bytes = decoded.into_raw();
        for p in bytes.chunks_exact_mut(4) {
            for c in 0..3 {
                p[c] = ((p[c] as u32 * p[3] as u32 + 127) / 255) as u8;
            }
        }
        let source = tiny_skia::Pixmap::from_vec(
            bytes,
            tiny_skia::IntSize::from_wh(w, h).context("Invalid image dimensions")?,
        )
        .context("Invalid image pixels")?;
        let [a, b, c, d, x, y] = image.transform.map(|value| value * scale);
        pixels.draw_pixmap(
            0,
            0,
            source.as_ref(),
            &tiny_skia::PixmapPaint {
                quality: tiny_skia::FilterQuality::Bilinear,
                ..Default::default()
            },
            tiny_skia::Transform::from_row(a, b, c, d, x, y),
            None,
        );
    }
    Ok(pixels)
}
#[cfg(any(test, not(target_os = "macos")))]
pub fn export(path: &Path, doc: &Document, transparent: bool, quality: u8) -> Result<()> {
    let jpeg = path
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("jpg") || e.eq_ignore_ascii_case("jpeg"));
    export_image(path, raster(doc, MAX_EDGE, transparent && !jpeg)?, quality)
}
pub fn export_image(path: &Path, image: RgbaImage, quality: u8) -> Result<()> {
    let extension = path
        .extension()
        .unwrap_or_default()
        .to_string_lossy()
        .to_lowercase();
    let jpeg = matches!(extension.as_str(), "jpg" | "jpeg");
    ensure!(jpeg || extension == "png", "Use a .png or .jpg filename");
    let image = DynamicImage::ImageRgba8(image);
    let mut bytes = std::io::Cursor::new(Vec::new());
    if jpeg {
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut bytes, quality.clamp(1, 100))
            .encode_image(&DynamicImage::ImageRgb8(image.to_rgb8()))?;
    } else {
        image.write_to(&mut bytes, image::ImageFormat::Png)?;
    }
    atomic_write(path, &bytes.into_inner())
}

// Union circular footprints before compositing so opacity does not depend on event frequency.
// Grain is anchored to document coordinates and is deterministic across saves/renders.
fn paint_brush(pixels: &mut tiny_skia::Pixmap, stroke: &Stroke, scale: f32) {
    let radius = stroke.size * scale / 2.;
    let margin = radius + 2.;
    let segments = crate::stroke_curve::segments(stroke, 0);
    let curve_points: Vec<_> = segments
        .iter()
        .flat_map(|s| [Point { x: s[0], y: s[1] }, Point { x: s[2], y: s[3] }])
        .collect();
    let min_x = (curve_points
        .iter()
        .map(|p| p.x * scale)
        .fold(f32::INFINITY, f32::min)
        - margin)
        .floor()
        .max(0.) as u32;
    let min_y = (curve_points
        .iter()
        .map(|p| p.y * scale)
        .fold(f32::INFINITY, f32::min)
        - margin)
        .floor()
        .max(0.) as u32;
    let max_x = (curve_points
        .iter()
        .map(|p| p.x * scale)
        .fold(f32::NEG_INFINITY, f32::max)
        + margin)
        .ceil()
        .max(0.)
        .min(pixels.width() as f32) as u32;
    let max_y = (curve_points
        .iter()
        .map(|p| p.y * scale)
        .fold(f32::NEG_INFINITY, f32::max)
        + margin)
        .ceil()
        .max(0.)
        .min(pixels.height() as f32) as u32;
    if min_x >= max_x || min_y >= max_y {
        return;
    }
    let width = (max_x - min_x) as usize;
    let mut coverage = vec![0f32; width * (max_y - min_y) as usize];
    let mut stamp = |x: f32, y: f32, p: f32| {
        let r = (radius * (0.12 + 0.88 * p)).max(0.25);
        let strength = if stroke.tool == Tool::Eraser {
            1.
        } else {
            0.15 + 0.85 * p
        };
        let left = (x - r - 1.).floor().max(min_x as f32) as u32;
        let top = (y - r - 1.).floor().max(min_y as f32) as u32;
        let right = (x + r + 1.).ceil().min(max_x as f32).max(0.) as u32;
        let bottom = (y + r + 1.).ceil().min(max_y as f32).max(0.) as u32;
        for py in top..bottom {
            for px in left..right {
                let distance = (px as f32 + 0.5 - x).hypot(py as f32 + 0.5 - y);
                let edge = if stroke.brush == Brush::Charcoal && stroke.tool != Tool::Eraser {
                    (1. - distance / r).clamp(0., 1.).sqrt()
                } else {
                    (r + 0.5 - distance).clamp(0., 1.)
                };
                let index = (py - min_y) as usize * width + (px - min_x) as usize;
                coverage[index] = coverage[index].max(edge * strength);
            }
        }
    };
    for segment in segments {
        let [ax, ay, bx, by, pa, pb] = segment;
        let distance = (bx - ax).hypot(by - ay) * scale;
        let steps = (distance / (radius * 0.12).clamp(0.5, 4.)).ceil().max(1.) as usize;
        for step in 0..=steps {
            let t = step as f32 / steps as f32;
            stamp(
                (ax + (bx - ax) * t) * scale,
                (ay + (by - ay) * t) * scale,
                pa + (pb - pa) * t,
            );
        }
    }
    let stride = pixels.width() as usize;
    let data = pixels.data_mut();
    for py in min_y..max_y {
        for px in min_x..max_x {
            let mut alpha =
                coverage[(py - min_y) as usize * width + (px - min_x) as usize] * stroke.opacity;
            if alpha == 0. {
                continue;
            }
            if stroke.tool != Tool::Eraser {
                let x = (px as f32 / scale).floor() as u32;
                let y = (py as f32 / scale).floor() as u32;
                let mut hash = x.wrapping_mul(374761393) ^ y.wrapping_mul(668265263);
                hash = (hash ^ (hash >> 13)).wrapping_mul(1274126177);
                let grain = (hash & 255) as f32 / 255.;
                alpha *= match stroke.brush {
                    Brush::Graphite => {
                        if grain < 0.18 {
                            0.12
                        } else {
                            0.3 + grain * 0.65
                        }
                    }
                    Brush::Charcoal => 0.35 + grain * 0.65,
                    Brush::Marker => 0.35,
                    Brush::Ink => 1.,
                };
            }
            let i = (py as usize * stride + px as usize) * 4;
            let colors = [41., 37., 44., 255.];
            for c in 0..4 {
                data[i + c] = if stroke.tool == Tool::Eraser {
                    (data[i + c] as f32 * (1. - alpha)).round() as u8
                } else {
                    (colors[c] * alpha + data[i + c] as f32 * (1. - alpha)).round() as u8
                };
            }
        }
    }
}
