//! Renders a preview's stage to a PNG without a window, for the live MCP
//! bridge.
//!
//! GPUI has no on-screen snapshot on macOS, so a capture draws the stage in a
//! headless GPUI app on the Metal renderer and reads back its pixels. That app
//! lives on one worker thread for the life of the process: it costs about half
//! a second to start and a few milliseconds per capture after that.
use std::{
    io::Cursor,
    sync::{mpsc, Arc, Mutex, OnceLock},
    thread,
};

use anyhow::{anyhow, Context as _, Result};
use gpui::{px, size, AppContext as _, HeadlessAppContext};
use storyfella_playback_panel::Still;
use storyfella_studio_core::playback::{stage::Viewer, PlaybackSession};

/// The longest side of a returned image, in pixels.
const MAX_SIDE: u32 = 800;

struct Job {
    session: PlaybackSession,
    viewer: Viewer,
    width: u32,
    height: u32,
    reply: mpsc::Sender<Result<Vec<u8>>>,
}

/// The stage of `session` as `viewer` sees it in a `width` × `height`
/// point viewport, as PNG bytes no larger than [`MAX_SIDE`] on a side.
/// Blocks until drawn; call it off the UI thread.
pub fn stage_png(
    session: PlaybackSession,
    viewer: Viewer,
    width: u32,
    height: u32,
) -> Result<Vec<u8>> {
    let (reply, answer) = mpsc::channel();
    worker()
        .lock()
        .map_err(|_| anyhow!("The capture worker stopped"))?
        .send(Job {
            session,
            viewer,
            width,
            height,
            reply,
        })
        .map_err(|_| anyhow!("The capture worker stopped"))?;
    answer
        .recv()
        .map_err(|_| anyhow!("The capture worker stopped"))?
}

fn worker() -> &'static Mutex<mpsc::Sender<Job>> {
    static WORKER: OnceLock<Mutex<mpsc::Sender<Job>>> = OnceLock::new();
    WORKER.get_or_init(|| {
        let (send, jobs) = mpsc::channel::<Job>();
        thread::Builder::new()
            .name("stage-capture".into())
            .spawn(move || {
                let mut app = match headless() {
                    Ok(app) => app,
                    Err(error) => {
                        let message = format!("{error:#}");
                        for job in jobs {
                            let _ = job.reply.send(Err(anyhow!(message.clone())));
                        }
                        return;
                    }
                };
                for job in jobs {
                    let png = draw(&mut app, job.session, job.viewer, job.width, job.height);
                    let _ = job.reply.send(png);
                }
            })
            .expect("the capture thread starts");
        Mutex::new(send)
    })
}

fn headless() -> Result<HeadlessAppContext> {
    let mut app = HeadlessAppContext::with_platform(
        Arc::new(gpui_apple::AppleTextSystem::new()),
        Arc::new(gpui_kit::assets::Assets),
        || {
            Ok(Some(Box::new(
                gpui_apple::metal_renderer::MetalHeadlessRenderer::new()?,
            )))
        },
    );
    app.update(gpui_kit::init);
    Ok(app)
}

fn draw(
    app: &mut HeadlessAppContext,
    session: PlaybackSession,
    viewer: Viewer,
    width: u32,
    height: u32,
) -> Result<Vec<u8>> {
    let window = app
        .open_window(size(px(width as f32), px(height as f32)), move |_, cx| {
            cx.new(|_| Still::new(session, viewer))
        })
        .context("Studio could not open a window to draw the stage in")?;
    app.run_until_parked();
    let pixels = app
        .capture_screenshot(window.into())
        .context("Studio could not draw the stage")?;
    let _ = window.update(app, |_, window, _| window.remove_window());
    let image = image::DynamicImage::ImageRgba8(pixels);
    let image = if image.width().max(image.height()) > MAX_SIDE {
        image.thumbnail(MAX_SIDE, MAX_SIDE)
    } else {
        image
    };
    let mut png = Cursor::new(Vec::new());
    image.write_to(&mut png, image::ImageFormat::Png)?;
    Ok(png.into_inner())
}

#[cfg(test)]
mod tests {
    use storyfella_runtime::AssetPolicy;
    use storyfella_studio_core::{config::PreviewSettings, registry::ArtifactRegistry};

    use super::*;

    /// Pins the headless API and what it draws, so a gpui-pre bump that
    /// breaks either fails here rather than in an agent's capture.
    #[test]
    fn a_capture_draws_the_stage_at_the_viewport_size() {
        let session = PlaybackSession::compile_with_policy(
            "scene harbor\n  narrate: The lamps come on one by one.\n  end\n",
            &ArtifactRegistry::new(),
            AssetPolicy::Preview,
        )
        .unwrap();
        let viewer = Viewer {
            previewing_draft: true,
            settings: PreviewSettings::default(),
        };
        let started = std::time::Instant::now();
        let png = stage_png(session.clone(), viewer, 480, 360).unwrap();
        let cold = started.elapsed();
        if let Ok(path) = std::env::var("STAGE_CAPTURE_OUT") {
            std::fs::write(path, &png).unwrap();
        }
        let image = image::load_from_memory_with_format(&png, image::ImageFormat::Png)
            .unwrap()
            .to_rgba8();
        // Drawn at 2x, 960 × 720, then fitted inside 800 on a side.
        assert_eq!(image.dimensions(), (800, 600));
        let ink = |xs: std::ops::Range<u32>, ys: std::ops::Range<u32>| {
            ys.flat_map(|y| xs.clone().map(move |x| (x, y)))
                .any(|(x, y)| image.get_pixel(x, y).0[0] > 0xa0)
        };
        // A scene without backdrop art stands on a dark ground, named in
        // its corner, with the passage in the reading card below.
        let corner = image.get_pixel(4, 4).0;
        assert!(
            corner[..3].iter().all(|channel| *channel < 0x20),
            "ground is {corner:?}"
        );
        assert!(ink(60..160, 60..90), "the scene is named in the corner");
        assert!(ink(90..600, 470..510), "the passage is in the card");
        assert!(!ink(90..600, 200..280), "nothing else is lit");

        let started = std::time::Instant::now();
        stage_png(session, viewer, 480, 360).unwrap();
        let warm = started.elapsed();
        eprintln!("capture: cold {cold:?}, warm {warm:?}");
    }
}
