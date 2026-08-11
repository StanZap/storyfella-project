//! Pure image primitives for the pipeline layer.
//!
//! Every function here is deterministic: given the same input files it
//! produces the same output file. This is what makes the composite fallback
//! testable — "only the masked region changes" is an assertable property
//! (pixels outside the mask are bit-identical to the original).

use std::path::{Path, PathBuf};

use image::{GrayImage, RgbaImage};
use thiserror::Error;

use crate::errors::{CodedError, ErrorCode};

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

impl CodedError for ImageOpsError {
    fn code(&self) -> ErrorCode {
        match self {
            Self::Open { .. } => ErrorCode::ImgOpen,
            Self::Write { .. } => ErrorCode::ImgWrite,
            Self::DimensionMismatch { .. } => ErrorCode::ImgDimensionMismatch,
            Self::EmptyUnion => ErrorCode::ImgEmptyUnion,
        }
    }
}

/// Blends `replacement` into `original` everywhere the hard mask is active.
///
/// With `grow == 0` this guarantees that every pixel *outside* the (eroded)
/// hard mask is bit-identical to the original, even with feathering:
/// feathering only softens the transition on pixels *inside* the mask (a
/// blurred copy of the mask is clamped to zero outside it), so the outside
/// boundary is exactly preserved.
///
/// `grow` chooses which way the boundary mismatch is paid for, and the two
/// directions are not interchangeable:
///
/// - `grow == 0` **erodes**. The seam lands inside the replacement's own
///   content and the guarantee above holds exactly. The cost is that
///   anything the mask *under*-covers survives untouched — where a
///   segmentation mask runs tighter than its subject (hair against a wall,
///   fur, anything with a soft silhouette), the pixels it missed remain as a
///   rim of the old content tracing the old outline.
/// - `grow > 0` **dilates** by that many pixels before feathering, so the
///   blend starts outside the silhouette and covers the rim. This is what
///   INPAINTKREA's own restore node does (`mask_grow` 12, then
///   `mask_feather` 10). It deliberately gives up the bit-identical
///   guarantee: pixels outside the mask do change, within `grow` of the
///   boundary.
///
/// Neither is right for every mask, which is why this is a parameter and
/// not a default. The feather radius scales with the canvas so seams look
/// the same at every size.
pub fn composite(
    original: &Path,
    mask: &Path,
    replacement: &Path,
    feather_radius: u32,
    grow: u32,
    output: &Path,
) -> Result<(), ImageOpsError> {
    let original_image = open_rgba(original)?;
    let replacement_image = open_rgba(replacement)?;
    let mask_image = open_luma(mask)?;

    // The composite is the guaranteed mask-edit mechanism, so it must
    // blend whatever canvases it is given: when the replacement (the
    // inpaint output) has different dimensions than the original (e.g. a
    // per-op size change), both the original and the mask are resized to
    // the replacement's canvas before blending.
    let (width, height) = replacement_image.dimensions();
    let mut original_image = original_image;
    let mut mask_image = mask_image;
    if original_image.dimensions() != (width, height) {
        original_image = image::imageops::resize(
            &original_image,
            width,
            height,
            image::imageops::FilterType::Lanczos3,
        );
    }
    if mask_image.dimensions() != (width, height) {
        // The mask must stay hard-edged: nearest-neighbor resize preserves
        // the selection boundary without introducing feathering fringes.
        mask_image = image::imageops::resize(
            &mask_image,
            width,
            height,
            image::imageops::FilterType::Nearest,
        );
    }

    let hard = mask_image.as_raw();
    // The feather scales with the canvas: the same relative softness at
    // every size keeps seams from becoming proportionally harsher on small
    // canvases.
    let feather = feather_radius.max(width.min(height) / 64);
    let blended_alpha: Vec<u8> = if grow > 0 {
        // Dilate, then feather across the grown boundary. No clamp: letting
        // the blur spread past the original mask is the point — that is what
        // covers a silhouette the mask ran tighter than.
        let grown = dilate_mask(hard, width, height, grow);
        if feather_radius == 0 {
            grown
        } else {
            box_blur(&grown, width, height, feather)
        }
    } else {
        // Erode the selection so the blend seam sits inside the
        // replacement's own content, and clamp the blur to the eroded mask
        // so pixels outside it stay bit-identical.
        let erosion = (width.min(height) / 200).clamp(1, 8);
        let eroded = erode_mask(hard, width, height, erosion);
        if feather_radius == 0 {
            eroded
        } else {
            let blurred = box_blur(&eroded, width, height, feather);
            eroded
                .iter()
                .zip(blurred)
                .map(|(&inside, value)| if inside > 0 { value } else { 0 })
                .collect()
        }
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

/// Replaces the masked region with a flat neutral colour.
///
/// The masked-latent inpaint sends an init image, and the model reads it:
/// measured 2026-08-11, changing the init moved the output by 16 mean levels
/// even at `denoise_strength` 1.0. So what the masked region contains is a
/// *prior* the generation is steered by, and leaving the original content
/// there means steering it back toward what the edit was meant to remove.
///
/// The fill is deliberately flat and deliberately not background-coloured.
/// An earlier revision diffused colour inward from outside the mask, on the
/// assumption that the surroundings approximate what is behind the subject.
/// For a hair mask they do not — most of that boundary is face and shoulder,
/// so the diffusion handed the model a skin-toned prior and the vacated wall
/// came back *further* from the truth (172,171,159 against a real
/// (228,232,231), versus (193,198,192) with no fill at all).
///
/// A flat colour that appears nowhere in the subject says "there is nothing
/// here" without proposing anything. This *is* INPAINTKREA's convention, not
/// an inference from one: `KreaAspectPreservePrepare`
/// (github.com/aitrepreneur/ComfyUI-KreaAspectPreserveOutpaint) fills
/// `mask > 0.5` with pure green in pixel space before generating, and the
/// separate `DefaultGreenMaskColor` node exists only to make a hand-painted
/// mask match that. Note they use *pure* `(0, 255, 0)`; the values tried
/// here so far are not that, and the comparison is still open.
pub fn fill_masked(
    image: &Path,
    mask: &Path,
    colour: [u8; 3],
    output: &Path,
) -> Result<(), ImageOpsError> {
    let source = open_rgba(image)?;
    let (width, height) = source.dimensions();
    let mut mask_image = open_luma(mask)?;
    if mask_image.dimensions() != (width, height) {
        mask_image = image::imageops::resize(
            &mask_image,
            width,
            height,
            image::imageops::FilterType::Nearest,
        );
    }

    let mut filled = source.clone();
    let mask_raw = mask_image.as_raw();
    let raw = filled.as_mut();
    for index in 0..(width * height) as usize {
        if mask_raw[index] >= 128 {
            for channel in 0..3 {
                raw[index * 4 + channel] = colour[channel];
            }
            raw[index * 4 + 3] = 255;
        }
    }

    filled.save(output).map_err(|source| ImageOpsError::Write {
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
/// Grows a hard mask by `radius` pixels, keeping it hard-edged.
///
/// Distinct from the composite's own `grow`, which widens the *blend* only.
/// This widens the region the backend is allowed to re-render. A
/// segmentation mask stops at the silhouette it can resolve, so anything
/// softer than that edge — flyaway hair, fur, motion blur — falls outside
/// it and is never regenerated; no amount of compositing can remove content
/// the model was not permitted to touch. Measured 2026-08-11: blonde
/// strands survived a hair replacement for exactly this reason.
pub fn grow(mask: &Path, radius: u32, output: &Path) -> Result<(), ImageOpsError> {
    let image = open_luma(mask)?;
    let (width, height) = image.dimensions();
    let grown = if radius == 0 {
        image
    } else {
        GrayImage::from_raw(
            width,
            height,
            dilate_mask(image.as_raw(), width, height, radius),
        )
        .expect("dilation preserves dimensions")
    };
    save_luma(&grown, output)
}

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
/// Shrinks a hard mask by `radius` pixels: a pixel stays lit only when
/// every pixel in its `radius`-box is lit (box erosion, out-of-bounds
/// pixels count as unlit). Used before feathering so the blend seam lands
/// inside the replacement's content.
fn erode_mask(source: &[u8], width: u32, height: u32, radius: u32) -> Vec<u8> {
    let width = width as usize;
    let height = height as usize;
    let radius = radius as usize;
    let mut eroded = vec![0u8; width * height];
    for y in 0..height {
        for x in 0..width {
            let mut lit = true;
            'neighbors: for dy in 0..=2 * radius {
                let neighbor_y = y as isize + dy as isize - radius as isize;
                if neighbor_y < 0 || neighbor_y >= height as isize {
                    lit = false;
                    break;
                }
                for dx in 0..=2 * radius {
                    let neighbor_x = x as isize + dx as isize - radius as isize;
                    if neighbor_x < 0 || neighbor_x >= width as isize {
                        lit = false;
                        break 'neighbors;
                    }
                    if source[neighbor_y as usize * width + neighbor_x as usize] == 0 {
                        lit = false;
                        break 'neighbors;
                    }
                }
            }
            if lit {
                eroded[y * width + x] = 255;
            }
        }
    }
    eroded
}

/// Grows a hard mask by `radius` pixels: a pixel lights when any pixel in
/// its `radius`-box is lit. The box structuring element is separable, so
/// this runs as a horizontal max pass then a vertical one rather than the
/// O(r²) neighbourhood scan `erode_mask` uses — `grow` is typically an order
/// of magnitude larger than the erosion.
fn dilate_mask(source: &[u8], width: u32, height: u32, radius: u32) -> Vec<u8> {
    let width = width as usize;
    let height = height as usize;
    let radius = radius as usize;

    let mut horizontal = vec![0u8; width * height];
    for y in 0..height {
        let row = y * width;
        for x in 0..width {
            let start = x.saturating_sub(radius);
            let end = (x + radius).min(width - 1);
            if source[row + start..=row + end]
                .iter()
                .any(|&value| value > 0)
            {
                horizontal[row + x] = 255;
            }
        }
    }

    let mut dilated = vec![0u8; width * height];
    for y in 0..height {
        let start = y.saturating_sub(radius);
        let end = (y + radius).min(height - 1);
        for x in 0..width {
            if (start..=end).any(|row| horizontal[row * width + x] > 0) {
                dilated[y * width + x] = 255;
            }
        }
    }
    dilated
}

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

        composite(&original, &mask, &replacement, 0, 0, &output).unwrap();

        let result = image::open(&output).unwrap().to_rgba8();
        let reference = image::open(&original).unwrap().to_rgba8();
        for y in 0..48 {
            for x in 0..64 {
                // The mask is eroded before blending (erosion 1 on this
                // tiny canvas; the frame border counts as unlit), so the
                // replacement region is x∈[1,30], y∈[1,46]; the boundary
                // band and the frame edges keep the original.
                let in_replacement = (1..31).contains(&x) && (1..47).contains(&y);
                if in_replacement {
                    assert_eq!(result.get_pixel(x, y), &Rgba([40, 40, 200, 255]));
                } else {
                    assert_eq!(
                        result.get_pixel(x, y),
                        reference.get_pixel(x, y),
                        "pixel ({x}, {y}) outside the eroded mask must be bit-identical"
                    );
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

        composite(&original, &mask, &replacement, 8, 0, &output).unwrap();

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

    /// The opposite trade to the two tests above: growing the mask covers
    /// content the mask ran tighter than, at the cost of the
    /// bit-identical-outside guarantee. The rim this reaches is the old
    /// hair a 0.977-scoring mask still missed (2026-08-11).
    #[test]
    fn composite_with_mask_grow_reaches_past_the_mask_boundary() {
        let dir = test_dir();
        std::fs::create_dir_all(&dir).unwrap();
        let original = dir.join("original.png");
        let mask = dir.join("mask.png");
        let replacement = dir.join("replacement.png");
        let grown = dir.join("grown.png");
        let strict = dir.join("strict.png");
        write_rgba(&original, 64, 48, [200, 40, 40, 255]);
        write_half_mask(&mask, 64, 48);
        write_rgba(&replacement, 64, 48, [40, 40, 200, 255]);

        composite(&original, &mask, &replacement, 0, 6, &grown).unwrap();
        composite(&original, &mask, &replacement, 0, 0, &strict).unwrap();

        let grown_image = image::open(&grown).unwrap().to_rgba8();
        let strict_image = image::open(&strict).unwrap().to_rgba8();
        let reference = image::open(&original).unwrap().to_rgba8();

        // Just outside the mask boundary: strict keeps the original, grown
        // replaces it. That difference is the whole point of the parameter.
        assert_eq!(strict_image.get_pixel(34, 24), reference.get_pixel(34, 24));
        assert_eq!(grown_image.get_pixel(34, 24), &Rgba([40, 40, 200, 255]));

        // The growth is bounded: well beyond `grow` the original survives
        // either way, so this never becomes a whole-frame paste.
        assert_eq!(grown_image.get_pixel(50, 24), reference.get_pixel(50, 24));
    }

    /// The masked region must come back as the requested flat colour and
    /// nothing else: no trace of what was under the mask (which is what the
    /// edit is removing) and no sampling of the surroundings (which for a
    /// hair mask is mostly skin).
    #[test]
    fn fill_masked_replaces_the_region_with_the_neutral_colour() {
        let dir = test_dir();
        std::fs::create_dir_all(&dir).unwrap();
        let source = dir.join("source.png");
        let mask = dir.join("mask.png");
        let output = dir.join("filled.png");

        // Left half black (the thing being removed), right half pale.
        let mut image = RgbaImage::new(64, 48);
        for (x, _y, pixel) in image.enumerate_pixels_mut() {
            *pixel = if x < 32 {
                Rgba([10, 10, 10, 255])
            } else {
                Rgba([228, 232, 231, 255])
            };
        }
        image.save(&source).unwrap();
        write_half_mask(&mask, 64, 48);

        fill_masked(&source, &mask, [0, 177, 64], &output).unwrap();
        let filled = image::open(&output).unwrap().to_rgba8();

        // Inside the mask: exactly the fill, not the black it replaced and
        // not the pale tone next door.
        assert_eq!(filled.get_pixel(10, 24), &Rgba([0, 177, 64, 255]));
        assert_eq!(filled.get_pixel(31, 0), &Rgba([0, 177, 64, 255]));
        // Outside the mask nothing moves.
        assert_eq!(filled.get_pixel(50, 24), &Rgba([228, 232, 231, 255]));
    }

    #[test]
    fn composite_resizes_mismatched_canvases_to_the_replacement() {
        let dir = test_dir();
        std::fs::create_dir_all(&dir).unwrap();
        let original = dir.join("original.png");
        let mask = dir.join("mask.png");
        let replacement = dir.join("replacement.png");
        let output = dir.join("output.png");
        write_rgba(&original, 64, 48, [200, 40, 40, 255]);
        write_luma(&mask, 64, 48, 255);
        write_rgba(&replacement, 32, 48, [40, 40, 200, 255]);

        composite(&original, &mask, &replacement, 0, 0, &output).unwrap();

        // The output follows the replacement canvas, with the original's
        // content resized into it (fully masked here, so it is the
        // replacement pixels everywhere — except the 1px erosion band on
        // this tiny canvas, which keeps the resized original).
        let result = image::open(&output).unwrap().to_rgba8();
        assert_eq!(result.dimensions(), (32, 48));
        assert_eq!(result.get_pixel(1, 1), &Rgba([40, 40, 200, 255]));
        assert_eq!(result.get_pixel(30, 46), &Rgba([40, 40, 200, 255]));
        assert_eq!(result.get_pixel(0, 0), &Rgba([200, 40, 40, 255]));
        assert_eq!(result.get_pixel(31, 47), &Rgba([200, 40, 40, 255]));
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
