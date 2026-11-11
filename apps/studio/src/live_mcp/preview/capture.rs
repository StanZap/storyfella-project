use std::{cell::RefCell, io::Cursor, rc::Rc, time::Duration};

use anyhow::{ensure, Context, Result};
use base64::{engine::general_purpose::STANDARD, Engine};
use block2::RcBlock;
use dioxus::{
    desktop::{self, wry::WebViewExtMacOS},
    prelude::*,
};
use objc2::MainThreadMarker;
use objc2_app_kit::{NSBitmapImageFileType, NSBitmapImageRep, NSImage};
use objc2_foundation::{NSDictionary, NSError, NSPoint, NSRect, NSSize};
use objc2_web_kit::WKSnapshotConfiguration;
use storyfella_playback_ui::{PlaybackModel, PlaybackPanel};
use storyfella_studio_core::playback::PlaybackSession;
use tokio::{sync::oneshot, time::timeout};

const CAPTURE_TIMEOUT: Duration = Duration::from_secs(8);

type Ready = Rc<RefCell<Option<oneshot::Sender<[f64; 4]>>>>;

#[derive(Clone, Props)]
struct FrameProps {
    snapshot: Rc<PlaybackSession>,
    ready: Ready,
    width: u32,
    height: u32,
}

impl PartialEq for FrameProps {
    fn eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.snapshot, &other.snapshot)
            && Rc::ptr_eq(&self.ready, &other.ready)
            && self.width == other.width
            && self.height == other.height
    }
}

#[allow(non_snake_case)]
fn Frame(props: FrameProps) -> Element {
    let staged_images = props
        .snapshot
        .runtime
        .staged()
        .iter()
        .filter(|actor| props.snapshot.staged_visual(actor).is_some())
        .count();
    let model = use_signal(|| {
        let mut model = PlaybackModel::default();
        model.install(Ok((*props.snapshot).clone()));
        model
    });
    use_future(move || {
        let ready = props.ready.borrow_mut().take();
        async move {
            if let Some(ready) = ready {
                let evaluation = document::eval(
                    r#"
                    const expected = await dioxus.recv();
                    const stage = document.querySelector('.sf-stage');
                    if (!stage) return null;
                    if (stage.querySelectorAll('img.sf-actor').length !== expected) return null;
                    await Promise.all([...stage.querySelectorAll('img')].map(img => img.decode()));
                    // Decoding alone can precede WebKit's compositing of positioned images.
                    await Promise.race([
                        new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve))),
                        new Promise(resolve => setTimeout(resolve, 250)),
                    ]);
                    const { x, y, width, height } = stage.getBoundingClientRect();
                    return [x, y, width, height];
                "#,
                );
                if evaluation.send(staged_images).is_err() {
                    return;
                }
                let result = evaluation.join::<[f64; 4]>().await;
                if let Ok(rect) = result {
                    let _ = ready.send(rect);
                }
            }
        }
    });
    let (width, height) = (props.width, props.height);
    rsx! {
        style { "html,body,#main {{ margin:0; width:100%; height:100%; overflow:hidden; }}" }
        div { style: "width:{width}px; height:{height}px;",
            PlaybackPanel { model, on_apply: EventHandler::new(|_| {}) }
        }
    }
}

struct CaptureWindow(desktop::DesktopContext);

impl Drop for CaptureWindow {
    fn drop(&mut self) {
        self.0.close();
    }
}

pub(super) async fn stage_png(
    parent: &desktop::DesktopContext,
    session: &PlaybackSession,
    width: u32,
    height: u32,
) -> Result<String> {
    let (ready, painted) = oneshot::channel();
    let dom = VirtualDom::new_with_props(
        Frame,
        FrameProps {
            snapshot: Rc::new(session.clone()),
            ready: Rc::new(RefCell::new(Some(ready))),
            width,
            height,
        },
    );
    let config = desktop::Config::new().with_window(
        desktop::WindowBuilder::new()
            .with_title("Storyfella Preview Capture")
            .with_inner_size(desktop::LogicalSize::new(
                f64::from(width),
                f64::from(height),
            ))
            .with_visible(false)
            .with_focused(false),
    );
    let window = CaptureWindow(
        parent
            .new_window(dom, config)
            .try_resolve()
            .await
            .context("Studio could not open the hidden capture window")?,
    );
    window.0.set_inner_size(desktop::LogicalSize::new(
        f64::from(width),
        f64::from(height),
    ));
    let [x, y, w, h] = timeout(CAPTURE_TIMEOUT, painted)
        .await
        .context("The Preview frame did not finish loading")?
        .context("The Preview stage could not be rendered")?;
    ensure!(
        x >= 0. && y >= 0. && (w - f64::from(width)).abs() < 1. && h > 0.,
        "The Preview stage did not lay out at the requested size"
    );
    let rect = NSRect::new(NSPoint::new(x, y), NSSize::new(w, h));
    let bytes = timeout(CAPTURE_TIMEOUT, snapshot(&window.0, rect))
        .await
        .context("The native Preview capture timed out")??;
    drop(window);
    tokio::task::spawn_blocking(move || encode_bounded(bytes)).await?
}

async fn snapshot(window: &desktop::DesktopContext, rect: NSRect) -> Result<Vec<u8>> {
    let mtm = MainThreadMarker::new().context("Preview capture requires the main thread")?;
    let view = window.webview.webview();
    let (send, receive) = oneshot::channel();
    let send = RefCell::new(Some(send));
    let completion = RcBlock::new(move |image: *mut NSImage, error: *mut NSError| {
        if let Some(send) = send.borrow_mut().take() {
            // WebKit supplies a borrowed NSImage for the duration of this callback.
            let bytes = unsafe { image.as_ref() }
                .filter(|_| error.is_null())
                .and_then(png_bytes);
            let _ = send.send(bytes);
        }
    });
    // Both the WebView and snapshot configuration are owned by this main-thread window.
    unsafe {
        let config = WKSnapshotConfiguration::new(mtm);
        config.setRect(rect);
        view.takeSnapshotWithConfiguration_completionHandler(Some(&config), &completion);
    }
    receive
        .await?
        .context("WebKit could not capture the Preview stage")
}

fn png_bytes(image: &NSImage) -> Option<Vec<u8>> {
    let data = image.TIFFRepresentation()?;
    let bitmap = NSBitmapImageRep::imageRepWithData(&data)?;
    // PNG uses default encoder properties, so the dictionary is empty.
    let png = unsafe {
        bitmap.representationUsingType_properties(NSBitmapImageFileType::PNG, &NSDictionary::new())
    }?;
    Some(png.to_vec())
}

fn encode_bounded(bytes: Vec<u8>) -> Result<String> {
    let image = image::load_from_memory_with_format(&bytes, image::ImageFormat::Png)?;
    if image.width().max(image.height()) <= 800 {
        return Ok(STANDARD.encode(bytes));
    }
    let mut png = Cursor::new(Vec::new());
    image
        .thumbnail(800, 800)
        .write_to(&mut png, image::ImageFormat::Png)?;
    Ok(STANDARD.encode(png.into_inner()))
}
