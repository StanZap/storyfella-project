//! Session artwork decoded once for GPUI.
//!
//! A session's visuals are fixed for its lifetime and carried as PNG data
//! URLs, so they are decoded when the session is installed rather than on
//! every frame.
use std::{io::Cursor, sync::Arc};

use base64::{engine::general_purpose::STANDARD, Engine};
use gpui_kit::{Image, ImageFormat};
use storyfella_studio_core::playback::{PlaybackSession, Visual};

const PNG_DATA_URL: &str = "data:image/png;base64,";

#[derive(Clone)]
pub struct StageImage {
    pub image: Arc<Image>,
    /// Width over height. GPUI derives an image's width only from a fixed
    /// pixel height, and actors are sized as a share of the stage.
    pub aspect: f32,
}

/// Decoded images, in the same order as the session's visuals.
#[derive(Clone, Default)]
pub struct StageImages(Vec<Option<StageImage>>);

impl StageImages {
    pub fn decode(session: &PlaybackSession) -> Self {
        Self(session.visuals.iter().map(decode).collect())
    }

    /// The decoded image for `visual`, which must belong to the session these
    /// images were decoded from.
    pub fn get(&self, session: &PlaybackSession, visual: &Visual) -> Option<&StageImage> {
        let index = session
            .visuals
            .iter()
            .position(|candidate| std::ptr::eq(candidate, visual))?;
        self.0.get(index)?.as_ref()
    }
}

fn decode(visual: &Visual) -> Option<StageImage> {
    let bytes = STANDARD
        .decode(visual.image_url.strip_prefix(PNG_DATA_URL)?)
        .ok()?;
    let (width, height) =
        image::ImageReader::with_format(Cursor::new(&bytes), image::ImageFormat::Png)
            .into_dimensions()
            .ok()?;
    (width > 0 && height > 0).then(|| StageImage {
        aspect: width as f32 / height as f32,
        image: Arc::new(Image::from_bytes(ImageFormat::Png, bytes)),
    })
}
