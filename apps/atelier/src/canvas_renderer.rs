//! Full-resolution tile delivery. On macOS brush rasterization and compositing run on Metal.
#[cfg(not(target_os = "macos"))]
use crate::document::Document;
use crate::viewport::Viewport;
#[cfg(not(target_os = "macos"))]
use anyhow::Result;
#[cfg(any(test, not(target_os = "macos")))]
use image::RgbaImage;
pub const TILE: u32 = 512;
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TileKey(pub u32, pub u32);
pub enum TileContent {
    #[cfg(any(test, not(target_os = "macos")))]
    Image(RgbaImage),
    #[cfg(target_os = "macos")]
    Surface(std::sync::Arc<crate::gpu_surface::Surface>),
}
impl TileContent {
    pub fn width(&self) -> u32 {
        match self {
            #[cfg(any(test, not(target_os = "macos")))]
            Self::Image(image) => image.width(),
            #[cfg(target_os = "macos")]
            Self::Surface(surface) => surface.width(),
        }
    }
    pub fn height(&self) -> u32 {
        match self {
            #[cfg(any(test, not(target_os = "macos")))]
            Self::Image(image) => image.height(),
            #[cfg(target_os = "macos")]
            Self::Surface(surface) => surface.height(),
        }
    }
}
pub struct TileImage {
    pub key: TileKey,
    pub x: u32,
    pub y: u32,
    pub image: TileContent,
}
pub struct TileBatch {
    pub reset: bool,
    pub tiles: Vec<TileImage>,
    pub backend: &'static str,
    pub elapsed_ms: f64,
    pub segments: usize,
}
#[cfg(any(test, not(target_os = "macos")))]
pub fn present(
    key: TileKey,
    image: RgbaImage,
    view: Viewport,
    width: u32,
    height: u32,
) -> TileImage {
    let x = key.0 * TILE;
    let y = key.1 * TILE;
    let w = image.width();
    let h = image.height();
    let (mut x, y, image) = match view.turns % 4 {
        1 => (height - y - h, x, image::imageops::rotate90(&image)),
        2 => (
            width - x - w,
            height - y - h,
            image::imageops::rotate180(&image),
        ),
        3 => (y, width - x - w, image::imageops::rotate270(&image)),
        _ => (x, y, image),
    };
    let mut image = image;
    if view.mirror {
        x = view.dimensions(width, height).0 as u32 - x - image.width();
        image::imageops::flip_horizontal_in_place(&mut image);
    }
    TileImage {
        key,
        x,
        y,
        image: TileContent::Image(image),
    }
}
#[cfg(target_os = "macos")]
pub use crate::gpu_canvas::Renderer;
#[cfg(not(target_os = "macos"))]
pub struct Renderer;
#[cfg(not(target_os = "macos"))]
impl Renderer {
    pub fn new() -> Result<Self> {
        Ok(Self)
    }
    pub fn render(&mut self, doc: &Document, view: Viewport, tracing: bool) -> Result<TileBatch> {
        let start = std::time::Instant::now();
        let mut image = crate::document::raster(doc, crate::document::MAX_EDGE, false)?;
        if tracing {
            if let Some(reference) = doc.reference.and_then(|i| doc.references.get(i)) {
                let mut reference = crate::document::decode_reference(reference)?
                    .thumbnail(doc.width, doc.height)
                    .to_rgba8();
                for pixel in reference.pixels_mut() {
                    pixel.0[3] = (pixel.0[3] as f32 * 0.18) as u8;
                }
                let x = (doc.width - reference.width()) / 2;
                let y = (doc.height - reference.height()) / 2;
                image::imageops::overlay(&mut image, &reference, x as i64, y as i64);
            }
        }
        let mut tiles = vec![];
        for y in (0..doc.height).step_by(TILE as usize) {
            for x in (0..doc.width).step_by(TILE as usize) {
                let part = image::imageops::crop_imm(
                    &image,
                    x,
                    y,
                    TILE.min(doc.width - x),
                    TILE.min(doc.height - y),
                )
                .to_image();
                tiles.push(present(
                    TileKey(x / TILE, y / TILE),
                    part,
                    view,
                    doc.width,
                    doc.height,
                ));
            }
        }
        Ok(TileBatch {
            reset: true,
            tiles,
            backend: "CPU",
            elapsed_ms: start.elapsed().as_secs_f64() * 1000.,
            segments: 0,
        })
    }
}

#[cfg(target_os = "macos")]
pub fn present_surface(
    key: TileKey,
    surface: std::sync::Arc<crate::gpu_surface::Surface>,
    view: Viewport,
    w: u32,
    h: u32,
    source_width: u32,
    source_height: u32,
) -> TileImage {
    let x = key.0 * TILE;
    let y = key.1 * TILE;
    let (mut x, y) = match view.turns % 4 {
        1 => (h - y - source_height, x),
        2 => (w - x - source_width, h - y - source_height),
        3 => (y, w - x - source_width),
        _ => (x, y),
    };
    if view.mirror {
        x = view.dimensions(w, h).0 as u32 - x - surface.width();
    }
    TileImage {
        key,
        x,
        y,
        image: TileContent::Surface(surface),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn edge_tiles_have_correct_position_for_every_orientation() {
        let w = 1000;
        let h = 700;
        let key = TileKey(1, 1);
        for turns in 0..4 {
            for mirror in [false, true] {
                let view = Viewport {
                    turns,
                    mirror,
                    ..Viewport::default()
                };
                let image = image::RgbaImage::new(w - TILE, h - TILE);
                let tile = present(key, image, view, w, h);
                let corners =
                    [(512., 512.), (1000., 512.), (512., 700.), (1000., 700.)].map(|(x, y)| {
                        view.to_view(crate::document::Point { x, y }, w as f32, h as f32)
                    });
                let x = corners.iter().map(|p| p.x).fold(f32::INFINITY, f32::min) as u32;
                let y = corners.iter().map(|p| p.y).fold(f32::INFINITY, f32::min) as u32;
                assert_eq!((tile.x, tile.y), (x, y));
                assert_eq!(
                    (tile.image.width(), tile.image.height()),
                    if turns % 2 == 0 {
                        (488, 188)
                    } else {
                        (188, 488)
                    }
                );
            }
        }
    }
}
