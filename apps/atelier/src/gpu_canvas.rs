//! Sparse, persistent layer tiles. Only appended segments and affected tiles are dispatched.
use crate::{
    canvas_renderer::{present_surface, TileBatch, TileKey, TILE},
    document::{Brush, Document, Tool},
    gpu_surface::Surface,
    viewport::Viewport,
};
use anyhow::{bail, ensure, Context, Result};
use metal::*;
use std::collections::{BTreeMap, BTreeSet, HashMap};

const CACHE_BYTES: usize = 768 * 1024 * 1024;
struct LayerTile {
    base: Texture,
    painted: Texture,
    coverage: Texture,
    stroke: Option<usize>,
}
pub struct Renderer {
    device: Device,
    queue: CommandQueue,
    clear: ComputePipelineState,
    paper: ComputePipelineState,
    brush: ComputePipelineState,
    composite: ComputePipelineState,
    trace: ComputePipelineState,
    orient: ComputePipelineState,
    surface_cache: core_video::metal_texture_cache::CVMetalTextureCache,
    readback_bytes: std::cell::Cell<u64>,
    layers: Vec<HashMap<TileKey, LayerTile>>,
    previous: Option<Document>,
    presented: HashMap<TileKey, std::sync::Arc<Surface>>,
    orientation: (u8, bool),
    tracing: bool,
    bytes: usize,
    reference: Option<Texture>,
    reference_id: Option<std::sync::Arc<crate::document::Reference>>,
    pub last_segments: usize,
}
impl Renderer {
    pub fn new() -> Result<Self> {
        let device = Device::system_default().context("Metal device unavailable")?;
        let library = device
            .new_library_with_source(include_str!("brushes.metal"), &CompileOptions::new())
            .map_err(anyhow::Error::msg)?;
        let pipeline = |name| -> Result<ComputePipelineState> {
            let f = library
                .get_function(name, None)
                .map_err(anyhow::Error::msg)?;
            device
                .new_compute_pipeline_state_with_function(&f)
                .map_err(anyhow::Error::msg)
        };
        Ok(Self {
            queue: device.new_command_queue(),
            clear: pipeline("clear_tile")?,
            paper: pipeline("paper_tile")?,
            brush: pipeline("brush_tile")?,
            composite: pipeline("composite_tile")?,
            trace: pipeline("trace_tile")?,
            orient: pipeline("orient_tile")?,
            surface_cache: core_video::metal_texture_cache::CVMetalTextureCache::new(
                None,
                device.clone(),
                None,
            )
            .map_err(|code| anyhow::anyhow!("Metal surface cache: {code}"))?,
            readback_bytes: std::cell::Cell::new(0),
            device,
            layers: vec![],
            previous: None,
            presented: HashMap::new(),
            orientation: (0, false),
            tracing: false,
            bytes: 0,
            reference: None,
            reference_id: None,
            last_segments: 0,
        })
    }
    fn texture(&self, width: u32, height: u32, format: MTLPixelFormat) -> Texture {
        let desc = TextureDescriptor::new();
        desc.set_texture_type(MTLTextureType::D2);
        desc.set_width(width as u64);
        desc.set_height(height as u64);
        desc.set_pixel_format(format);
        desc.set_storage_mode(if self.device.has_unified_memory() {
            MTLStorageMode::Shared
        } else {
            MTLStorageMode::Managed
        });
        desc.set_usage(MTLTextureUsage::ShaderRead | MTLTextureUsage::ShaderWrite);
        self.device.new_texture(&desc)
    }
    fn dispatch(
        &self,
        command: &CommandBufferRef,
        pipeline: &ComputePipelineStateRef,
        textures: &[&TextureRef],
        params: &[f32],
        segments: Option<&[[f32; 6]]>,
    ) {
        let encoder = command.new_compute_command_encoder();
        encoder.set_compute_pipeline_state(pipeline);
        for (i, texture) in textures.iter().enumerate() {
            encoder.set_texture(i as u64, Some(texture));
        }
        let index = if let Some(segments) = segments {
            let buffer = self.device.new_buffer_with_data(
                segments.as_ptr().cast(),
                std::mem::size_of_val(segments) as u64,
                MTLResourceOptions::StorageModeShared,
            );
            encoder.set_buffer(0, Some(&buffer), 0);
            1
        } else {
            0
        };
        if !params.is_empty() {
            encoder.set_bytes(
                index,
                std::mem::size_of_val(params) as u64,
                params.as_ptr().cast(),
            );
        }
        let target = textures.last().unwrap();
        encoder.dispatch_thread_groups(
            MTLSize::new(
                (if segments.is_some() {
                    params[10] as u64
                } else {
                    target.width()
                })
                .div_ceil(16),
                (if segments.is_some() {
                    params[11] as u64
                } else {
                    target.height()
                })
                .div_ceil(16),
                1,
            ),
            MTLSize::new(16, 16, 1),
        );
        encoder.end_encoding();
    }
    fn clear_texture(&self, command: &CommandBufferRef, texture: &TextureRef) {
        self.dispatch(command, &self.clear, &[texture], &[], None);
    }
    fn new_tile(&self, command: &CommandBufferRef, key: TileKey, doc: &Document) -> LayerTile {
        let w = TILE.min(doc.width - key.0 * TILE);
        let h = TILE.min(doc.height - key.1 * TILE);
        let tile = LayerTile {
            base: self.texture(w, h, MTLPixelFormat::RGBA8Unorm),
            painted: self.texture(w, h, MTLPixelFormat::RGBA8Unorm),
            coverage: self.texture(w, h, MTLPixelFormat::R32Float),
            stroke: None,
        };
        self.clear_texture(command, &tile.painted);
        tile
    }
    fn append(
        &mut self,
        command: &CommandBufferRef,
        doc: &Document,
        layer: usize,
        index: usize,
        from: usize,
        dirty: &mut BTreeSet<TileKey>,
    ) -> Result<()> {
        let stroke = &doc.layers[layer].strokes[index];
        let mut bins: BTreeMap<TileKey, Vec<[f32; 6]>> = BTreeMap::new();
        for segment in segments(stroke, from) {
            for key in segment_tiles(segment, stroke.size / 2. + 1., doc.width, doc.height) {
                bins.entry(key).or_default().push(segment);
            }
        }
        for (key, segments) in bins {
            self.last_segments += segments.len();
            if !self.layers[layer].contains_key(&key) {
                let bytes = TILE.min(doc.width - key.0 * TILE) as usize
                    * TILE.min(doc.height - key.1 * TILE) as usize
                    * 12;
                ensure!(
                    self.bytes + bytes <= CACHE_BYTES,
                    "Painting exceeds the 768 MiB GPU layer cache"
                );
                let tile = self.new_tile(command, key, doc);
                self.layers[layer].insert(key, tile);
                self.bytes += bytes;
            }
            let tile = &mut self.layers[layer].get_mut(&key).unwrap();
            let fresh = tile.stroke != Some(index);
            if fresh {
                let blit = command.new_blit_command_encoder();
                blit.copy_from_texture(
                    &tile.painted,
                    0,
                    0,
                    MTLOrigin { x: 0, y: 0, z: 0 },
                    MTLSize::new(tile.painted.width(), tile.painted.height(), 1),
                    &tile.base,
                    0,
                    0,
                    MTLOrigin { x: 0, y: 0, z: 0 },
                );
                blit.end_encoding();
                tile.stroke = Some(index);
            }
            // The pre-stroke base stays immutable across all batches and input events.
            let base = tile.base.clone();
            let coverage = tile.coverage.clone();
            let painted = tile.painted.clone();
            if fresh {
                self.clear_texture(command, &coverage);
            }
            for batch in segments.chunks(64) {
                let margin = stroke.size / 2. + 1.;
                let origin = [(key.0 * TILE) as f32, (key.1 * TILE) as f32];
                let left = (batch
                    .iter()
                    .map(|s| s[0].min(s[2]))
                    .fold(f32::INFINITY, f32::min)
                    - margin
                    - origin[0])
                    .floor()
                    .max(0.);
                let top = (batch
                    .iter()
                    .map(|s| s[1].min(s[3]))
                    .fold(f32::INFINITY, f32::min)
                    - margin
                    - origin[1])
                    .floor()
                    .max(0.);
                let right = (batch
                    .iter()
                    .map(|s| s[0].max(s[2]))
                    .fold(f32::NEG_INFINITY, f32::max)
                    + margin
                    - origin[0])
                    .ceil()
                    .min(painted.width() as f32);
                let bottom = (batch
                    .iter()
                    .map(|s| s[1].max(s[3]))
                    .fold(f32::NEG_INFINITY, f32::max)
                    + margin
                    - origin[1])
                    .ceil()
                    .min(painted.height() as f32);
                if right <= left || bottom <= top {
                    continue;
                }
                let params = [
                    batch.len() as f32,
                    origin[0],
                    origin[1],
                    stroke.size / 2.,
                    stroke.opacity,
                    match stroke.brush {
                        Brush::Graphite => 0.,
                        Brush::Ink => 1.,
                        Brush::Charcoal => 2.,
                        Brush::Marker => 3.,
                    },
                    if stroke.tool == Tool::Eraser { 1. } else { 0. },
                    0.,
                    left,
                    top,
                    right - left,
                    bottom - top,
                ];
                self.dispatch(
                    command,
                    &self.brush,
                    &[&base, &coverage, &painted],
                    &params,
                    Some(batch),
                );
            }
            dirty.insert(key);
        }
        Ok(())
    }
    fn composite_tile(
        &self,
        command: &CommandBufferRef,
        key: TileKey,
        doc: &Document,
        transparent: bool,
    ) -> Result<std::sync::Arc<Surface>> {
        let surface = std::sync::Arc::new(Surface::new(
            &self.surface_cache,
            TILE.min(doc.width - key.0 * TILE),
            TILE.min(doc.height - key.1 * TILE),
        )?);
        let output = &surface.texture;
        self.dispatch(
            command,
            &self.paper,
            &[output],
            &[if transparent { 1. } else { 0. }],
            None,
        );
        for (i, layer) in doc
            .layers
            .iter()
            .enumerate()
            .filter(|(_, l)| l.visible && l.opacity > 0.)
        {
            if let Some(tile) = self.layers[i].get(&key) {
                self.dispatch(
                    command,
                    &self.composite,
                    &[&tile.painted, output],
                    &[layer.opacity, 0., 0.],
                    None,
                );
            }
        }
        if !transparent && self.tracing {
            if let Some(reference) = &self.reference {
                let scale = (doc.width as f32 / reference.width() as f32)
                    .min(doc.height as f32 / reference.height() as f32)
                    .min(1.);
                self.dispatch(
                    command,
                    &self.trace,
                    &[reference, output],
                    &[
                        (key.0 * TILE) as f32,
                        (key.1 * TILE) as f32,
                        (doc.width as f32 - reference.width() as f32 * scale) / 2.,
                        (doc.height as f32 - reference.height() as f32 * scale) / 2.,
                        scale,
                    ],
                    None,
                );
            }
        }
        Ok(surface)
    }
    fn read(&self, texture: &TextureRef) -> image::RgbaImage {
        if texture.storage_mode() == MTLStorageMode::Managed {
            let command = self.queue.new_command_buffer();
            let blit = command.new_blit_command_encoder();
            blit.synchronize_resource(texture);
            blit.end_encoding();
            Self::complete(command).expect("GPU export/readback synchronization");
        }
        let w = texture.width() as u32;
        let h = texture.height() as u32;
        let mut bytes = vec![0u8; w as usize * h as usize * 4];
        self.readback_bytes
            .set(self.readback_bytes.get() + bytes.len() as u64);
        texture.get_bytes(
            bytes.as_mut_ptr().cast(),
            w as u64 * 4,
            MTLRegion::new_2d(0, 0, w as u64, h as u64),
            0,
        );
        for pixel in bytes.chunks_exact_mut(4) {
            if texture.pixel_format() == MTLPixelFormat::BGRA8Unorm {
                pixel.swap(0, 2);
            }
            if pixel[3] > 0 && pixel[3] < 255 {
                for c in 0..3 {
                    pixel[c] = ((pixel[c] as u32 * 255 + pixel[3] as u32 / 2) / pixel[3] as u32)
                        .min(255) as u8;
                }
            }
        }
        image::RgbaImage::from_raw(w, h, bytes).unwrap()
    }
    fn complete(command: &CommandBufferRef) -> Result<()> {
        command.commit();
        command.wait_until_completed();
        if command.status() == MTLCommandBufferStatus::Error {
            bail!("Metal command buffer failed");
        }
        Ok(())
    }
    pub fn render(&mut self, doc: &Document, view: Viewport, tracing: bool) -> Result<TileBatch> {
        let result = self.render_inner(doc, view, tracing);
        if result.is_err() {
            self.previous = None;
            self.layers.clear();
            self.presented.clear();
            self.bytes = 0;
        }
        result
    }
    pub fn export(&mut self, doc: &Document, transparent: bool) -> Result<image::RgbaImage> {
        self.render(doc, Viewport::default(), false)?;
        let mut output = image::RgbaImage::new(doc.width, doc.height);
        if !transparent {
            for (key, tile) in &self.presented {
                image::imageops::replace(
                    &mut output,
                    &self.read(&tile.texture),
                    (key.0 * TILE) as i64,
                    (key.1 * TILE) as i64,
                );
            }
        } else {
            let command = self.queue.new_command_buffer();
            let textures: Vec<_> = self
                .presented
                .keys()
                .map(|key| Ok((*key, self.composite_tile(command, *key, doc, true)?)))
                .collect::<Result<Vec<_>>>()?;
            Self::complete(command)?;
            for (key, texture) in textures {
                image::imageops::replace(
                    &mut output,
                    &self.read(&texture.texture),
                    (key.0 * TILE) as i64,
                    (key.1 * TILE) as i64,
                );
            }
        }
        Ok(output)
    }
    #[cfg(test)]
    fn snapshot(&self) -> HashMap<TileKey, image::RgbaImage> {
        self.presented
            .iter()
            .map(|(key, surface)| (*key, self.read(&surface.texture)))
            .collect()
    }
    fn render_inner(&mut self, doc: &Document, view: Viewport, tracing: bool) -> Result<TileBatch> {
        let start = std::time::Instant::now();
        self.last_segments = 0;
        let rebuild = self
            .previous
            .as_ref()
            .is_none_or(|old| !can_append(old, doc));
        let metadata_changed = self.previous.as_ref().is_none_or(|old| {
            old.layers
                .iter()
                .zip(&doc.layers)
                .any(|(a, b)| a.visible != b.visible || a.opacity != b.opacity)
        });
        let reference = doc.reference.and_then(|i| doc.references.get(i));
        let reference_changed = match (&self.reference_id, reference) {
            (Some(a), Some(b)) => !std::sync::Arc::ptr_eq(a, b),
            (None, None) => false,
            _ => true,
        };
        if reference_changed {
            self.reference = if let Some(reference) = reference {
                let image = crate::document::decode_reference(reference)?.to_rgba8();
                let texture =
                    self.texture(image.width(), image.height(), MTLPixelFormat::RGBA8Unorm);
                texture.replace_region(
                    MTLRegion::new_2d(0, 0, image.width() as u64, image.height() as u64),
                    0,
                    image.as_ptr().cast(),
                    image.width() as u64 * 4,
                );
                Some(texture)
            } else {
                None
            };
            self.reference_id = reference.cloned();
        }
        let all = rebuild
            || metadata_changed
            || tracing != self.tracing
            || (tracing && reference_changed);
        self.tracing = tracing;
        if rebuild {
            self.layers = (0..doc.layers.len()).map(|_| HashMap::new()).collect();
            self.bytes = 0;
            self.presented.clear();
            for (layer, item) in doc
                .layers
                .iter()
                .enumerate()
                .filter(|(_, l)| l.image.is_some())
            {
                let pixels = crate::document::image_layer_pixels(item, doc.width, doc.height, 1.)?;
                let image = image::RgbaImage::from_raw(doc.width, doc.height, pixels.take())
                    .context("Invalid image layer pixels")?;
                for y in (0..doc.height).step_by(TILE as usize) {
                    for x in (0..doc.width).step_by(TILE as usize) {
                        let width = TILE.min(doc.width - x);
                        let height = TILE.min(doc.height - y);
                        let bytes = width as usize * height as usize * 12;
                        ensure!(self.bytes + bytes <= CACHE_BYTES, "Image layers exceed the 768 MiB GPU cache. Reduce canvas size or remove a layer.");
                        let part =
                            image::imageops::crop_imm(&image, x, y, width, height).to_image();
                        let painted = self.texture(width, height, MTLPixelFormat::RGBA8Unorm);
                        painted.replace_region(
                            MTLRegion::new_2d(0, 0, width as u64, height as u64),
                            0,
                            part.as_ptr().cast(),
                            width as u64 * 4,
                        );
                        let base = self.texture(width, height, MTLPixelFormat::RGBA8Unorm);
                        let coverage = self.texture(width, height, MTLPixelFormat::R32Float);
                        self.layers[layer].insert(
                            TileKey(x / TILE, y / TILE),
                            LayerTile {
                                base,
                                painted,
                                coverage,
                                stroke: None,
                            },
                        );
                        self.bytes += bytes;
                    }
                }
            }
        }
        let queue = self.queue.clone();
        let command = queue.new_command_buffer();
        let mut dirty = BTreeSet::new();
        for layer in 0..doc.layers.len() {
            let old = if rebuild {
                None
            } else {
                self.previous.as_ref().map(|d| &d.layers[layer])
            };
            let count = old.map_or(0, |l| l.strokes.len());
            let from = old
                .and_then(|l| l.strokes.last())
                .map_or(0, |s| s.points.len());
            let append_last = count > 0 && doc.layers[layer].strokes[count - 1].points.len() > from;
            if append_last {
                self.append(command, doc, layer, count - 1, from, &mut dirty)?;
            }
            for index in count..doc.layers[layer].strokes.len() {
                self.append(command, doc, layer, index, 0, &mut dirty)?;
            }
        }
        if all {
            for y in (0..doc.height).step_by(TILE as usize) {
                for x in (0..doc.width).step_by(TILE as usize) {
                    dirty.insert(TileKey(x / TILE, y / TILE));
                }
            }
        }
        let changed: Vec<_> = dirty.iter().copied().collect();
        let textures: Vec<_> = dirty
            .into_iter()
            .map(|key| Ok((key, self.composite_tile(command, key, doc, false)?)))
            .collect::<Result<Vec<_>>>()?;
        Self::complete(command)?;
        for (key, texture) in textures {
            self.presented.insert(key, texture);
        }
        let orientation = (view.turns % 4, view.mirror);
        let reset = rebuild || self.orientation != orientation;
        // Updated tile keys are determined before textures are consumed.
        let keys: Vec<_> = if reset || all {
            self.presented.keys().copied().collect()
        } else {
            changed
        };
        let orient_command = self.queue.new_command_buffer();
        let mut tiles = Vec::with_capacity(keys.len());
        for key in keys {
            let source = &self.presented[&key];
            let surface = if orientation == (0, false) {
                source.clone()
            } else {
                let (w, h) = if view.turns.is_multiple_of(2) {
                    (source.width(), source.height())
                } else {
                    (source.height(), source.width())
                };
                let surface = std::sync::Arc::new(Surface::new(&self.surface_cache, w, h)?);
                self.dispatch(
                    orient_command,
                    &self.orient,
                    &[&source.texture, &surface.texture],
                    &[view.turns as f32, if view.mirror { 1. } else { 0. }],
                    None,
                );
                surface
            };
            tiles.push(present_surface(
                key,
                surface,
                view,
                doc.width,
                doc.height,
                source.width(),
                source.height(),
            ));
        }
        if orientation != (0, false) && !tiles.is_empty() {
            Self::complete(orient_command)?;
        }
        self.surface_cache.flush(0);
        self.previous = Some(doc.clone());
        self.orientation = orientation;
        Ok(TileBatch {
            reset,
            tiles,
            backend: "Metal",
            elapsed_ms: start.elapsed().as_secs_f64() * 1000.,
            segments: self.last_segments,
        })
    }
}
fn can_append(old: &Document, new: &Document) -> bool {
    old.width == new.width
        && old.height == new.height
        && old.layers.len() == new.layers.len()
        && old.layers.iter().zip(&new.layers).all(|(a, b)| {
            a.image == b.image
                && a.strokes.len() <= b.strokes.len()
                && a.strokes.iter().enumerate().all(|(i, s)| {
                    let n = &b.strokes[i];
                    if std::sync::Arc::ptr_eq(&s.points, &n.points)
                        && std::sync::Arc::ptr_eq(&s.pressures, &n.pressures)
                    {
                        s.tool == n.tool
                            && s.brush == n.brush
                            && s.size == n.size
                            && s.opacity == n.opacity
                    } else if i + 1 < a.strokes.len() {
                        s == n
                    } else {
                        s.tool == n.tool
                            && s.brush == n.brush
                            && s.size == n.size
                            && s.opacity == n.opacity
                            && s.points.len() <= n.points.len()
                            && n.points[..s.points.len()] == s.points[..]
                            && ((s.pressures.is_empty() && n.pressures.is_empty())
                                || (!s.pressures.is_empty()
                                    && s.pressures.len() <= n.pressures.len()
                                    && n.pressures[..s.pressures.len()] == s.pressures[..]))
                    }
                })
        })
}
#[cfg(test)]
use crate::document::Stroke;
use crate::stroke_curve::segments;
fn segment_tiles(s: [f32; 6], radius: f32, w: u32, h: u32) -> Vec<TileKey> {
    let x0 = (s[0].min(s[2]) - radius).floor().max(0.) as u32;
    let y0 = (s[1].min(s[3]) - radius).floor().max(0.) as u32;
    let x1 = (s[0].max(s[2]) + radius).ceil().max(0.).min(w as f32) as u32;
    let y1 = (s[1].max(s[3]) + radius).ceil().max(0.).min(h as f32) as u32;
    if x0 >= x1 || y0 >= y1 {
        return vec![];
    }
    let mut keys = vec![];
    for y in y0 / TILE..=(y1 - 1) / TILE {
        for x in x0 / TILE..=(x1 - 1) / TILE {
            let bounds = [
                (x * TILE) as f32 - radius,
                (y * TILE) as f32 - radius,
                ((x + 1) * TILE).min(w) as f32 + radius,
                ((y + 1) * TILE).min(h) as f32 + radius,
            ];
            if segment_intersects(s, bounds) {
                keys.push(TileKey(x, y));
            }
        }
    }
    keys
}

fn segment_intersects(s: [f32; 6], bounds: [f32; 4]) -> bool {
    let mut enter = 0f32;
    let mut exit = 1f32;
    for axis in 0..2 {
        let delta = s[axis + 2] - s[axis];
        if delta.abs() < 0.00001 {
            if s[axis] < bounds[axis] || s[axis] > bounds[axis + 2] {
                return false;
            }
        } else {
            let a = (bounds[axis] - s[axis]) / delta;
            let b = (bounds[axis + 2] - s[axis]) / delta;
            enter = enter.max(a.min(b));
            exit = exit.min(a.max(b));
            if enter > exit {
                return false;
            }
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    fn document() -> Document {
        Document {
            width: 1024,
            height: 768,
            ..Document::default()
        }
    }
    fn line(x: f32, y: f32, brush: Brush) -> Stroke {
        Stroke {
            tool: Tool::Pencil,
            brush,
            size: 18.,
            opacity: 0.8,
            points: vec![
                crate::document::Point { x, y },
                crate::document::Point { x: x + 60., y },
            ]
            .into(),
            pressures: vec![0.2, 1.].into(),
        }
    }
    #[test]
    #[ignore = "Requires a physical Metal device"]
    fn metal_incremental_tiles_match_rebuild_and_skip_old_segments() {
        objc2::rc::autoreleasepool(|_| {
            let mut renderer = Renderer::new().unwrap();
            let mut doc = document();
            doc.layers[0].strokes.push(line(20., 30., Brush::Graphite));
            let first = renderer.render(&doc, Viewport::default(), false).unwrap();
            assert_eq!(
                renderer.readback_bytes.get(),
                0,
                "Live rendering must not read back pixels"
            );
            assert!(first.reset);
            assert_eq!(first.tiles.len(), 4);
            let s = &mut doc.layers[0].strokes[0];
            std::sync::Arc::make_mut(&mut s.points)
                .push(crate::document::Point { x: 130., y: 45. });
            std::sync::Arc::make_mut(&mut s.pressures).push(0.5);
            let incremental = renderer.render(&doc, Viewport::default(), false).unwrap();
            assert!(!incremental.reset);
            assert_eq!(incremental.tiles.len(), 1);
            assert!(
                incremental.segments > 1,
                "A bent input interval should be curved"
            );
            assert_eq!(
                renderer.readback_bytes.get(),
                0,
                "Incremental rendering must remain GPU-resident"
            );
            let mut fresh = Renderer::new().unwrap();
            fresh.render(&doc, Viewport::default(), false).unwrap();
            assert_eq!(renderer.snapshot(), fresh.snapshot());
            let noop = renderer.render(&doc, Viewport::default(), false).unwrap();
            assert!(noop.tiles.is_empty());
            assert_eq!(noop.segments, 0);
            println!(
                "Metal: first {:.2} ms, incremental {:.2} ms, {} segment / {} tile",
                first.elapsed_ms,
                incremental.elapsed_ms,
                incremental.segments,
                incremental.tiles.len()
            );
        });
    }
    #[test]
    #[ignore = "Requires a physical Metal device"]
    fn metal_layers_eraser_undo_and_transforms_match_fresh_renderer() {
        objc2::rc::autoreleasepool(|_| {
            let mut renderer = Renderer::new().unwrap();
            let mut doc = document();
            for brush in Brush::ALL {
                doc.layers[0].strokes.push(line(490., 200., brush));
            }
            doc.layers.push(crate::document::Layer {
                name: "Top".into(),
                visible: true,
                opacity: 0.5,
                strokes: vec![line(490., 200., Brush::Ink)],
                image: None,
            });
            renderer.render(&doc, Viewport::default(), false).unwrap();
            let mut eraser = line(490., 200., Brush::Ink);
            eraser.tool = Tool::Eraser;
            eraser.opacity = 1.;
            doc.layers[1].strokes.push(eraser);
            renderer.render(&doc, Viewport::default(), false).unwrap();
            for change in 0..5 {
                match change {
                    0 => doc.layers[1].visible = false,
                    1 => {
                        doc.layers[1].visible = true;
                        doc.layers[0].opacity = 0.4;
                    }
                    2 => {
                        doc.layers[1].strokes.pop();
                    }
                    3 => doc.rotate(),
                    _ => doc.flip(),
                }
                renderer.render(&doc, Viewport::default(), false).unwrap();
                let mut fresh = Renderer::new().unwrap();
                fresh.render(&doc, Viewport::default(), false).unwrap();
                assert_eq!(renderer.snapshot(), fresh.snapshot());
            }
            let view = Viewport {
                turns: 1,
                mirror: true,
                ..Viewport::default()
            };
            let transformed = renderer.render(&doc, view, false).unwrap();
            assert!(transformed.reset);
            assert_eq!(transformed.segments, 0);
        });
    }
    #[test]
    #[ignore = "Requires a physical Metal device"]
    fn metal_pressure_and_pencil_grain_are_real_pixel_changes() {
        objc2::rc::autoreleasepool(|_| {
            let mut renderer = Renderer::new().unwrap();
            let mut doc = Document {
                width: 128,
                height: 96,
                ..Document::default()
            };
            let mut stroke = line(10., 48., Brush::Ink);
            std::sync::Arc::make_mut(&mut stroke.points)[1].x = 110.;
            stroke.size = 20.;
            stroke.opacity = 1.;
            stroke.pressures = vec![0.1, 1.].into();
            doc.layers[0].strokes.push(stroke);
            renderer.render(&doc, Viewport::default(), false).unwrap();
            let image = renderer.read(&renderer.presented[&TileKey(0, 0)].texture);
            assert!(image.get_pixel(25, 48)[0] > image.get_pixel(95, 48)[0] + 70);
            assert!(image.get_pixel(25, 54)[0] > 240);
            assert!(image.get_pixel(95, 54)[0] < 100);
            doc.layers[0].strokes[0].brush = Brush::Graphite;
            std::sync::Arc::make_mut(&mut doc.layers[0].strokes[0].pressures).clear();
            renderer.render(&doc, Viewport::default(), false).unwrap();
            let image = renderer.read(&renderer.presented[&TileKey(0, 0)].texture);
            let values: std::collections::HashSet<_> =
                (20..100).map(|x| image.get_pixel(x, 48)[0]).collect();
            assert!(values.len() > 20);
        });
    }
    #[test]
    #[ignore = "Requires a physical Metal device"]
    fn metal_image_layers_preserve_color_alpha_erase_and_transforms() {
        objc2::rc::autoreleasepool(|_| {
            let original = image::RgbaImage::from_pixel(128, 96, image::Rgba([20, 80, 180, 128]));
            let mut bytes = std::io::Cursor::new(Vec::new());
            original
                .write_to(&mut bytes, image::ImageFormat::Png)
                .unwrap();
            let reference = crate::document::reference_from_bytes(
                "Imported".into(),
                &bytes.into_inner(),
                "Metal image layer test".into(),
            )
            .unwrap();
            let mut doc = Document {
                width: 128,
                height: 96,
                ..Document::default()
            };
            doc.layers[0].image = Some(crate::document::ImageLayer {
                reference: std::sync::Arc::new(reference),
                transform: [1., 0., 0., 1., 0., 0.],
            });
            let mut renderer = Renderer::new().unwrap();
            let actual = renderer.export(&doc, true).unwrap();
            for (a, b) in actual
                .get_pixel(64, 48)
                .0
                .into_iter()
                .zip([20u8, 80, 180, 128])
            {
                assert!(a.abs_diff(b) <= 1);
            }
            let mut eraser = line(10., 48., Brush::Ink);
            std::sync::Arc::make_mut(&mut eraser.points)[1].x = 110.;
            eraser.tool = Tool::Eraser;
            eraser.size = 20.;
            eraser.opacity = 1.;
            doc.layers[0].strokes.push(eraser);
            let erased = renderer.export(&doc, true).unwrap();
            assert_eq!(erased.get_pixel(64, 48)[3], 0);
            assert_eq!(erased.get_pixel(64, 20)[3], 128);
            doc.layers[0].strokes.clear();
            for step in 0..3 {
                match step {
                    0 => doc.rotate(),
                    1 => doc.flip(),
                    _ => {
                        doc.resize(192, 192, true).unwrap();
                    }
                }
                let reused = renderer.export(&doc, true).unwrap();
                let fresh = Renderer::new().unwrap().export(&doc, true).unwrap();
                assert_eq!(reused, fresh);
                let cpu = crate::document::raster(&doc, 192, true).unwrap();
                assert_eq!(reused.dimensions(), cpu.dimensions());
                for (gpu, cpu) in reused.pixels().zip(cpu.pixels()) {
                    for (a, b) in gpu.0.into_iter().zip(cpu.0) {
                        assert!(a.abs_diff(b) <= 2);
                    }
                }
            }
        });
    }
    #[test]
    #[ignore = "Requires a physical Metal device"]
    fn metal_exports_preserve_alpha_and_layer_eraser() {
        objc2::rc::autoreleasepool(|_| {
            let mut renderer = Renderer::new().unwrap();
            let mut doc = Document {
                width: 128,
                height: 96,
                ..Document::default()
            };
            let mut stroke = line(10., 48., Brush::Ink);
            stroke.opacity = 0.5;
            std::sync::Arc::make_mut(&mut stroke.pressures).clear();
            doc.layers[0].strokes.push(stroke);
            let transparent = renderer.export(&doc, true).unwrap();
            assert_eq!(transparent.get_pixel(0, 0)[3], 0);
            let p = transparent.get_pixel(30, 48);
            assert!((126..=129).contains(&p[3]));
            assert!((39..=43).contains(&p[0]));
            let mut eraser = line(10., 48., Brush::Ink);
            eraser.tool = Tool::Eraser;
            eraser.opacity = 1.;
            std::sync::Arc::make_mut(&mut eraser.pressures).clear();
            doc.layers[0].strokes.push(eraser);
            assert_eq!(renderer.export(&doc, true).unwrap().get_pixel(30, 48)[3], 0);
            assert_eq!(
                renderer.export(&doc, false).unwrap().get_pixel(30, 48).0,
                [255, 253, 248, 255]
            );
        });
    }
    #[test]
    #[ignore = "Requires a physical Metal device"]
    fn metal_tracing_overlay_is_cached_and_excluded_from_export() {
        objc2::rc::autoreleasepool(|_| {
            let mut renderer = Renderer::new().unwrap();
            let mut doc = Document {
                width: 128,
                height: 96,
                ..Document::default()
            };
            let image = image::RgbaImage::from_pixel(64, 64, image::Rgba([0, 0, 255, 255]));
            let mut bytes = std::io::Cursor::new(Vec::new());
            image::DynamicImage::ImageRgba8(image)
                .write_to(&mut bytes, image::ImageFormat::Png)
                .unwrap();
            doc.references.push(std::sync::Arc::new(
                crate::document::reference_from_bytes(
                    "Test".into(),
                    bytes.get_ref(),
                    "Test".into(),
                )
                .unwrap(),
            ));
            doc.reference = Some(0);
            renderer.render(&doc, Viewport::default(), false).unwrap();
            let paper = renderer
                .read(&renderer.presented[&TileKey(0, 0)].texture)
                .get_pixel(50, 50)
                .0;
            let overlay = renderer.render(&doc, Viewport::default(), true).unwrap();
            assert_eq!(overlay.segments, 0);
            assert!(
                renderer
                    .read(&renderer.presented[&TileKey(0, 0)].texture)
                    .get_pixel(50, 50)[0]
                    < paper[0] - 30
            );
            assert!(renderer
                .render(&doc, Viewport::default(), true)
                .unwrap()
                .tiles
                .is_empty());
            assert_eq!(
                renderer.export(&doc, false).unwrap().get_pixel(50, 50).0,
                paper
            );
        });
    }
    #[test]
    #[ignore = "Requires a physical Metal device"]
    fn metal_gpui_samples_shared_bgra_surfaces_with_clipping() {
        use gpui_kit::{
            point, size, Bounds, ContentMask, DevicePixels, PaintSurface, PlatformHeadlessRenderer,
            ScaledPixels, Scene,
        };
        objc2::rc::autoreleasepool(|_| {
            let mut renderer = Renderer::new().unwrap();
            let mut doc = Document {
                width: 128,
                height: 96,
                ..Document::default()
            };
            doc.layers[0].strokes.push(line(10., 48., Brush::Graphite));
            let batch = renderer.render(&doc, Viewport::default(), false).unwrap();
            assert_eq!(renderer.readback_bytes.get(), 0);
            let crate::canvas_renderer::TileContent::Surface(surface) = &batch.tiles[0].image
            else {
                panic!("Expected a GPU surface")
            };
            let bounds = Bounds::new(
                point(ScaledPixels(0.), ScaledPixels(0.)),
                size(ScaledPixels(128.), ScaledPixels(96.)),
            );
            let mut scene = Scene::default();
            scene.insert_primitive(PaintSurface {
                order: 0,
                bounds,
                content_mask: ContentMask {
                    bounds: Bounds::new(
                        point(ScaledPixels(0.), ScaledPixels(0.)),
                        size(ScaledPixels(64.), ScaledPixels(96.)),
                    ),
                },
                image_buffer: surface.buffer(),
            });
            scene.finish();
            let mut gpui = gpui_apple::metal_renderer::MetalHeadlessRenderer::new();
            let displayed = gpui
                .render_scene_to_image(&scene, size(DevicePixels(128), DevicePixels(96)))
                .unwrap();
            let source = renderer.read(&surface.texture);
            for (x, y) in [(0, 0), (30, 48), (50, 45)] {
                assert_eq!(
                    displayed.get_pixel(x, y),
                    source.get_pixel(x, y),
                    "BGRA color/alpha mismatch"
                );
            }
            assert_eq!(
                displayed.get_pixel(100, 48).0,
                [0, 0, 0, 255],
                "Surface must respect the canvas clipping mask"
            );
            assert_eq!(
                renderer.readback_bytes.get(),
                128 * 96 * 4,
                "Only explicit test readback may access pixels"
            );
        });
    }
    #[test]
    #[ignore = "Requires a physical Metal device"]
    fn metal_published_surfaces_are_immutable_and_orientation_stays_on_gpu() {
        objc2::rc::autoreleasepool(|_| {
            let mut renderer = Renderer::new().unwrap();
            let mut doc = Document {
                width: 128,
                height: 96,
                ..Document::default()
            };
            doc.layers[0].strokes.push(line(10., 48., Brush::Graphite));
            let batch = renderer.render(&doc, Viewport::default(), false).unwrap();
            let crate::canvas_renderer::TileContent::Surface(original) = &batch.tiles[0].image
            else {
                panic!("Expected GPU tile")
            };
            let before = renderer.read(&original.texture);
            let mut dark = line(10., 48., Brush::Charcoal);
            dark.size = 40.;
            doc.layers[0].strokes.push(dark);
            renderer.render(&doc, Viewport::default(), false).unwrap();
            assert_eq!(
                before,
                renderer.read(&original.texture),
                "Publishing a later frame must not overwrite a displayed surface"
            );
            let source = renderer.read(&renderer.presented[&TileKey(0, 0)].texture);
            for turns in 0..4 {
                for mirror in [false, true] {
                    let readbacks = renderer.readback_bytes.get();
                    let batch = renderer
                        .render(
                            &doc,
                            Viewport {
                                turns,
                                mirror,
                                ..Viewport::default()
                            },
                            false,
                        )
                        .unwrap();
                    assert_eq!(
                        renderer.readback_bytes.get(),
                        readbacks,
                        "Rotation and mirror must not read pixels"
                    );
                    if let Some(tile) = batch.tiles.first() {
                        let crate::canvas_renderer::TileContent::Surface(surface) = &tile.image
                        else {
                            panic!("Expected GPU tile")
                        };
                        let mut expected = match turns {
                            1 => image::imageops::rotate90(&source),
                            2 => image::imageops::rotate180(&source),
                            3 => image::imageops::rotate270(&source),
                            _ => source.clone(),
                        };
                        if mirror {
                            image::imageops::flip_horizontal_in_place(&mut expected);
                        }
                        assert_eq!(renderer.read(&surface.texture), expected);
                    }
                }
            }
        });
    }
    #[test]
    #[ignore = "Requires a physical Metal device; dense 16 MP benchmark"]
    fn metal_dense_painting_benchmark() {
        objc2::rc::autoreleasepool(|_| {
            let mut renderer = Renderer::new().unwrap();
            let mut doc = Document {
                width: 4096,
                height: 4096,
                ..Document::default()
            };
            for layer in 0..2 {
                if layer > 0 {
                    doc.layers.push(crate::document::Layer {
                        name: "Shading".into(),
                        visible: true,
                        opacity: 0.7,
                        strokes: vec![],
                        image: None,
                    });
                }
                for i in 0..400 {
                    let mut stroke = line(
                        ((i * 137) % 3900) as f32,
                        ((i * 179) % 3900) as f32,
                        Brush::ALL[i % 4],
                    );
                    stroke.size = 40.;
                    let start = stroke.points[0];
                    stroke.points = (0..32)
                        .map(|j| crate::document::Point {
                            x: start.x + j as f32 * 3.,
                            y: start.y + (j as f32 * 0.2).sin() * 14.,
                        })
                        .collect::<Vec<_>>()
                        .into();
                    stroke.pressures = (0..32)
                        .map(|j| 0.3 + j as f32 / 50.)
                        .collect::<Vec<_>>()
                        .into();
                    doc.layers[layer].strokes.push(stroke);
                }
            }
            let cold = renderer.render(&doc, Viewport::default(), false).unwrap();
            let last = doc.layers[1].strokes.last_mut().unwrap();
            let p = *last.points.last().unwrap();
            std::sync::Arc::make_mut(&mut last.points).push(crate::document::Point {
                x: p.x + 8.,
                y: p.y + 1.,
            });
            std::sync::Arc::make_mut(&mut last.pressures).push(0.8);
            let new_segments = segments(last, last.points.len() - 1).len();
            let warm = renderer.render(&doc, Viewport::default(), false).unwrap();
            assert_eq!(
                renderer.readback_bytes.get(),
                0,
                "Dense paintings must not read back tiles"
            );
            assert!(
                warm.segments <= new_segments * 2,
                "Only the new input interval may be dispatched"
            );
            assert!(warm.tiles.len() <= 2);
            assert!(!warm.reset);
            println!("{}: 4096×4096, 2 layers, 800 strokes, 25,600 points: rebuild {:.2} ms; append {:.2} ms, {} segments in {} tiles; GPU layer cache {:.1} MiB",renderer.device.name(),cold.elapsed_ms,warm.elapsed_ms,warm.segments,warm.tiles.len(),renderer.bytes as f64/1024./1024.);
        });
    }
    #[test]
    fn segment_binning_clips_and_covers_tile_boundary() {
        assert_eq!(
            segment_tiles([500., 200., 530., 200., 1., 1.], 10., 1024, 768),
            vec![TileKey(0, 0), TileKey(1, 0)]
        );
        assert!(segment_tiles([-100., -100., -50., -50., 1., 1.], 5., 1024, 768).is_empty());
        assert!(
            !segment_tiles([0., 0., 1024., 768., 1., 1.], 5., 1024, 768).contains(&TileKey(0, 1))
        );
    }
}
