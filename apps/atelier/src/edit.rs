use crate::document::{self, Document, Stroke};
use anyhow::{ensure, Result};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MaskDraft {
    pub width: u32,
    pub height: u32,
    pub strokes: Vec<Stroke>,
}
impl MaskDraft {
    pub fn composite(
        &self,
        original: &document::Reference,
        generated: &document::Reference,
    ) -> Result<document::Reference> {
        let source = document::decode_reference(original)?.to_rgba8();
        let mut output = document::decode_reference(generated)?.to_rgba8();
        ensure!(source.dimensions() == output.dimensions() && source.dimensions() == (self.width,self.height), "Masked output dimensions changed. Choose an editing model that preserves the source dimensions.");
        let mask = image::load_from_memory(&self.png()?)?.to_rgba8();
        for ((target, source), mask) in output.pixels_mut().zip(source.pixels()).zip(mask.pixels())
        {
            let coverage = mask[0] as u32;
            if coverage == 0 {
                *target = *source;
                continue;
            }
            if coverage == 255 {
                continue;
            }
            let alpha = target[3] as u32 * coverage + source[3] as u32 * (255 - coverage);
            for c in 0..3 {
                target[c] = (target[c] as u32 * target[3] as u32 * coverage
                    + source[c] as u32 * source[3] as u32 * (255 - coverage)
                    + alpha / 2)
                    .checked_div(alpha)
                    .unwrap_or(0)
                    .min(255) as u8;
            }
            target[3] = ((alpha + 127) / 255) as u8;
        }
        let mut bytes = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageRgba8(output).write_to(&mut bytes, image::ImageFormat::Png)?;
        document::reference_from_bytes(
            generated.name.clone(),
            &bytes.into_inner(),
            generated.provenance.clone(),
        )
    }
    pub fn preview(&self, edge: u32) -> Result<image::RgbaImage> {
        self.validate()?;
        let mut doc = Document {
            width: self.width,
            height: self.height,
            ..Document::default()
        };
        doc.layers[0].strokes = self.strokes.clone();
        let mut pixels = document::raster(&doc, edge, true)?;
        for pixel in pixels.pixels_mut() {
            pixel.0 = [121, 14, 38, pixel[3] / 2];
        }
        Ok(pixels)
    }
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.width > 0
                && self.height > 0
                && self.width <= document::MAX_EDGE
                && self.height <= document::MAX_EDGE
                && self.width as u64 * self.height as u64 <= document::MAX_PIXELS,
            "Invalid mask dimensions"
        );
        let mut doc = Document {
            width: self.width.max(64),
            height: self.height.max(64),
            ..Document::default()
        };
        doc.layers[0].strokes = self.strokes.clone();
        doc.validate()
    }
    pub fn png(&self) -> Result<Vec<u8>> {
        self.validate()?;
        ensure!(
            !self.strokes.is_empty(),
            "Paint the area you want to edit first"
        );
        let mut doc = Document {
            width: self.width,
            height: self.height,
            ..Document::default()
        };
        doc.layers[0].strokes = self.strokes.clone();
        let mut pixels = document::raster(&doc, document::MAX_EDGE, true)?;
        let mut has_editable_area = false;
        for pixel in pixels.pixels_mut() {
            let alpha = pixel[3];
            has_editable_area |= alpha > 0;
            pixel.0 = [alpha, alpha, alpha, 255];
        }
        ensure!(
            has_editable_area,
            "The mask is empty. Paint the area you want to edit."
        );
        let mut bytes = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageRgba8(pixels).write_to(&mut bytes, image::ImageFormat::Png)?;
        Ok(bytes.into_inner())
    }
}
