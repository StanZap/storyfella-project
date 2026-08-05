//! Pure image primitives for the pipeline layer.
//!
//! Every function here is deterministic: given the same input files it
//! produces the same output file. This is what makes the composite fallback
//! testable — "only the masked region changes" is an assertable property
//! (pixels outside the mask are bit-identical to the original).

use std::path::{Path, PathBuf};

use image::{GrayImage, RgbaImage};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ImageOpsError {
    #[error("could not open {path}: {source}")]
    Open {
        path: PathBuf,
        source: image::ImageError,
    },
    #[error("could not write {path}: {source}")]
    Write {
        path: PathBuf,
        source: image::ImageError,
    },
    #[error("dimension mismatch: {what} is {actual:?}, expected {expected:?}")]
    DimensionMismatch {
        what: &'static str,
        actual: (u32, u32),
        expected: (u32, u32),
    },
    #[error("union requires at least one mask")]
    EmptyUnion,
}

/// Blends `replacement` into `original` everywhere the hard mask is active.
///
/// Guarantee: every pixel *outside* the hard mask is bit-identical to the
/// original, even with feathering. Feathering only softens the transition on
/// pixels *inside* the mask (a blurred copy of the mask is clamped to zero
/// outside it), so the outside boundary is exactly preserved.
pub fn composite(
    original: &Path,
    mask: &Path,
    replacement: &Path,
    feather_radius: u32,
    output: &Path,
) -> Result<(), ImageOpsError> {
    let original_image = open_rgba(original)?;
    let replacement_image = open_rgba(replacement)?;
    let mask_image = open_luma(mask)?;

    let (width, height) = original_image.dimensions();
    let expected = (width, height);
    if replacement_image.dimensions() != expected {
        return Err(ImageOpsError::DimensionMismatch {
            what: "replacement image",
            actual: replacement_image.dimensions(),
            expected,
        });
    }
    if mask_image.dimensions() != expected {
        return Err(ImageOpsError::DimensionMismatch {
            what: "mask",
            actual: mask_image.dimensions(),
            expected,
        });
    }

    let hard = mask_image.as_raw();
    let blended_alpha: Vec<u8> = if feather_radius == 0 {
        hard.clone()
    } else {
        let blurred = box_blur(hard, width, height, feather_radius);
        hard.iter()
            .zip(blurred)
            .map(|(&inside, value)| if inside > 0 { value } else { 0 })
            .collect()
    };

    let mut output_image = original_image.clone();
    let original_raw = original_image.as_raw();
    let replacement_raw = replacement_image.as_raw();
    let output_raw = output_image.as_mut();

    for y in 0..height {
        for x in 0..width {
            let index = (y * width + x) as usize;
            let alpha = blended_alpha[index] as u32;
            if alpha == 0 {
                continue; // outside the mask: keep the original pixel untouched
            }
            for channel in 0..4 {
                let offset = index * 4 + channel;
                let original_value = original_raw[offset] as u32;
                let replacement_value = replacement_raw[offset] as u32;
                output_raw[offset] =
                    ((original_value * (255 - alpha) + replacement_value * alpha + 127) / 255)
                        as u8;
            }
        }
    }

    output_image
        .save(output)
        .map_err(|source| ImageOpsError::Write {
            path: output.to_path_buf(),
            source,
        })
}

/// Produces the inverse mask (255 - value per pixel).
pub fn invert(mask: &Path, output: &Path) -> Result<(), ImageOpsError> {
    let image = open_luma(mask)?;
    let mut inverted = image.clone();
    for value in inverted.as_mut() {
        *value = 255 - *value;
    }
    save_luma(&inverted, output)
}

/// Blurs a binary mask into a soft-edged mask.
pub fn feather(mask: &Path, radius: u32, output: &Path) -> Result<(), ImageOpsError> {
    let image = open_luma(mask)?;
    let (width, height) = image.dimensions();
    let feathered = if radius == 0 {
        image
    } else {
        GrayImage::from_raw(
            width,
            height,
            box_blur(image.as_raw(), width, height, radius),
        )
        .expect("blur preserves dimensions")
    };
    save_luma(&feathered, output)
}

/// Unions several masks pixel-wise (per-pixel maximum).
pub fn union(masks: &[PathBuf], output: &Path) -> Result<(), ImageOpsError> {
    let Some(first) = masks.first() else {
        return Err(ImageOpsError::EmptyUnion);
    };
    let mut image = open_luma(first)?;
    let (width, height) = image.dimensions();
    for mask in &masks[1..] {
        let next = open_luma(mask)?;
        if next.dimensions() != (width, height) {
            return Err(ImageOpsError::DimensionMismatch {
                what: "union member",
                actual: next.dimensions(),
                expected: (width, height),
            });
        }
        for (target, value) in image.as_mut().iter_mut().zip(next.as_raw().iter()) {
            *target = (*target).max(*value);
        }
    }
    save_luma(&image, output)
}

fn open_rgba(path: &Path) -> Result<RgbaImage, ImageOpsError> {
    Ok(image::open(path)
        .map_err(|source| ImageOpsError::Open {
            path: path.to_path_buf(),
            source,
        })?
        .to_rgba8())
}

fn open_luma(path: &Path) -> Result<GrayImage, ImageOpsError> {
    Ok(image::open(path)
        .map_err(|source| ImageOpsError::Open {
            path: path.to_path_buf(),
            source,
        })?
        .to_luma8())
}

fn save_luma(image: &GrayImage, output: &Path) -> Result<(), ImageOpsError> {
    image.save(output).map_err(|source| ImageOpsError::Write {
        path: output.to_path_buf(),
        source,
    })
}

/// Separable box blur with a (2*radius + 1) window, operating on raw luma
/// values and clamping at the image edges. Returns a freshly allocated buffer
/// of the same length.
///
/// Implemented as a running sum over the row/column conceptually extended by
/// `radius` clamped pixels on each side, so edge windows average the clamped
/// edge value correctly.
fn box_blur(source: &[u8], width: u32, height: u32, radius: u32) -> Vec<u8> {
    let width = width as usize;
    let height = height as usize;
    let radius = radius as usize;
    let window = radius * 2 + 1;
    let mut horizontal = vec![0u32; source.len()];

    // Horizontal pass.
    for y in 0..height {
        let row = y * width;
        let mut sum: u32 = 0;
        for index in 0..(width + window - 1) {
            let position = index.saturating_sub(radius).min(width - 1);
            sum += source[row + position] as u32;
            if index >= window {
                let removed = (index - window).saturating_sub(radius).min(width - 1);
                sum -= source[row + removed] as u32;
            }
            if index + 1 >= window && index + 1 - window < width {
                horizontal[row + index + 1 - window] = sum;
            }
        }
    }

    // Vertical pass.
    let mut vertical = vec![0u32; source.len()];
    for x in 0..width {
        let mut sum: u32 = 0;
        for index in 0..(height + window - 1) {
            let position = index.saturating_sub(radius).min(height - 1);
            sum += horizontal[position * width + x];
            if index >= window {
                let removed = (index - window).saturating_sub(radius).min(height - 1);
                sum -= horizontal[removed * width + x];
            }
            if index + 1 >= window && index + 1 - window < height {
                vertical[(index + 1 - window) * width + x] = sum;
            }
        }
    }

    vertical
        .iter()
        .map(|&sum| (sum / (window * window) as u32) as u8)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{ImageBuffer, Luma, Rgba};
    use std::path::PathBuf;
    use uuid::Uuid;

    fn test_dir() -> PathBuf {
        std::env::temp_dir().join(format!("svs-image-ops-{}", Uuid::new_v4()))
    }

    fn write_rgba(path: &Path, width: u32, height: u32, color: [u8; 4]) {
        ImageBuffer::from_pixel(width, height, Rgba(color))
            .save(path)
            .expect("test image should save");
    }

    fn write_luma(path: &Path, width: u32, height: u32, value: u8) {
        ImageBuffer::from_pixel(width, height, Luma([value]))
            .save(path)
            .expect("test mask should save");
    }

    /// A mask that is white (active) on the left half and black on the right.
    fn write_half_mask(path: &Path, width: u32, height: u32) {
        let mut image = GrayImage::from_pixel(width, height, Luma([0]));
        for y in 0..height {
            for x in 0..width / 2 {
                image.put_pixel(x, y, Luma([255]));
            }
        }
        image.save(path).expect("test mask should save");
    }

    #[test]
    fn composite_preserves_pixels_outside_the_mask_without_feathering() {
        let dir = test_dir();
        std::fs::create_dir_all(&dir).unwrap();
        let original = dir.join("original.png");
        let mask = dir.join("mask.png");
        let replacement = dir.join("replacement.png");
        let output = dir.join("output.png");
        write_rgba(&original, 64, 48, [200, 40, 40, 255]);
        write_half_mask(&mask, 64, 48);
        write_rgba(&replacement, 64, 48, [40, 40, 200, 255]);

        composite(&original, &mask, &replacement, 0, &output).unwrap();

        let result = image::open(&output).unwrap().to_rgba8();
        let reference = image::open(&original).unwrap().to_rgba8();
        for y in 0..48 {
            for x in 0..64 {
                if x >= 32 {
                    assert_eq!(
                        result.get_pixel(x, y),
                        reference.get_pixel(x, y),
                        "pixel ({x}, {y}) outside the mask must be bit-identical"
                    );
                } else {
                    assert_eq!(result.get_pixel(x, y), &Rgba([40, 40, 200, 255]));
                }
            }
        }
    }

    #[test]
    fn composite_preserves_pixels_outside_the_mask_with_feathering() {
        let dir = test_dir();
        std::fs::create_dir_all(&dir).unwrap();
        let original = dir.join("original.png");
        let mask = dir.join("mask.png");
        let replacement = dir.join("replacement.png");
        let output = dir.join("output.png");
        write_rgba(&original, 64, 48, [200, 40, 40, 255]);
        write_half_mask(&mask, 64, 48);
        write_rgba(&replacement, 64, 48, [40, 40, 200, 255]);

        composite(&original, &mask, &replacement, 8, &output).unwrap();

        let result = image::open(&output).unwrap().to_rgba8();
        let reference = image::open(&original).unwrap().to_rgba8();
        for y in 0..48 {
            for x in 32..64 {
                assert_eq!(
                    result.get_pixel(x, y),
                    reference.get_pixel(x, y),
                    "pixel ({x}, {y}) outside the mask must be bit-identical with feathering"
                );
            }
        }
        // The boundary pixel just inside the mask must have been blended
        // (feathering ramps the blend inside the mask).
        assert_ne!(result.get_pixel(31, 24), &Rgba([40, 40, 200, 255]));
    }

    #[test]
    fn composite_rejects_dimension_mismatches() {
        let dir = test_dir();
        std::fs::create_dir_all(&dir).unwrap();
        let original = dir.join("original.png");
        let mask = dir.join("mask.png");
        let replacement = dir.join("replacement.png");
        write_rgba(&original, 64, 48, [0, 0, 0, 255]);
        write_luma(&mask, 64, 48, 255);
        write_rgba(&replacement, 32, 48, [0, 0, 0, 255]);

        let error = composite(&original, &mask, &replacement, 0, &dir.join("out.png")).unwrap_err();
        assert!(
            matches!(error, ImageOpsError::DimensionMismatch { .. }),
            "expected a dimension mismatch, got {error}"
        );
    }

    #[test]
    fn invert_flips_mask_values() {
        let dir = test_dir();
        std::fs::create_dir_all(&dir).unwrap();
        let mask = dir.join("mask.png");
        let output = dir.join("inverted.png");
        write_half_mask(&mask, 64, 48);

        invert(&mask, &output).unwrap();

        let result = image::open(&output).unwrap().to_luma8();
        assert_eq!(result.get_pixel(0, 0).0[0], 0, "left half should flip to 0");
        assert_eq!(
            result.get_pixel(40, 24).0[0],
            255,
            "right half flips to 255"
        );
    }

    #[test]
    fn union_takes_the_per_pixel_maximum() {
        let dir = test_dir();
        std::fs::create_dir_all(&dir).unwrap();
        let top = dir.join("top.png");
        let bottom = dir.join("bottom.png");
        let output = dir.join("union.png");
        let mut top_image = GrayImage::from_pixel(32, 32, Luma([0]));
        let mut bottom_image = GrayImage::from_pixel(32, 32, Luma([0]));
        for x in 0..32 {
            top_image.put_pixel(x, 8, Luma([255]));
            bottom_image.put_pixel(x, 24, Luma([255]));
        }
        top_image.save(&top).unwrap();
        bottom_image.save(&bottom).unwrap();

        union(&[top, bottom], &output).unwrap();

        let result = image::open(&output).unwrap().to_luma8();
        assert_eq!(result.get_pixel(0, 8).0[0], 255);
        assert_eq!(result.get_pixel(0, 24).0[0], 255);
        assert_eq!(result.get_pixel(0, 16).0[0], 0);
    }

    #[test]
    fn feather_softens_a_hard_mask_edge() {
        let dir = test_dir();
        std::fs::create_dir_all(&dir).unwrap();
        let mask = dir.join("mask.png");
        let output = dir.join("feathered.png");
        write_half_mask(&mask, 64, 48);

        feather(&mask, 4, &output).unwrap();

        let result = image::open(&output).unwrap().to_luma8();
        // Deep inside the mask stays fully white, outside stays black...
        assert_eq!(result.get_pixel(8, 24).0[0], 255);
        assert_eq!(result.get_pixel(56, 24).0[0], 0);
        // ...and the boundary is now a partial value rather than a hard edge.
        let boundary = result.get_pixel(31, 24).0[0];
        assert!(
            boundary > 0 && boundary < 255,
            "boundary should ramp, got {boundary}"
        );
    }
}
