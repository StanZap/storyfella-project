use crate::{
    document::{self, Document, Reference},
    providers::{self, Request},
    viewport::Viewport,
};
use anyhow::{Context, Result};
use image::RgbaImage;
use std::{
    path::PathBuf,
    sync::{
        mpsc::{self, Receiver, Sender},
        Arc,
    },
};

pub struct Study {
    pub title: String,
    pub path: PathBuf,
    pub image: RgbaImage,
    pub note: String,
}
pub enum Event {
    Studies(Result<Vec<Study>>),
    Raster(u64, Result<crate::canvas_renderer::TileBatch>),
    Saved {
        revision: u64,
        epoch: u64,
        path: PathBuf,
        automatic: bool,
        result: Result<()>,
    },
    Exported(Result<PathBuf>),
    Switched(u64, Result<(Document, Vec<RgbaImage>, Option<PathBuf>)>),
    Imported(u64, Result<Vec<(Reference, RgbaImage)>>),
    Generated(u64, Result<(Reference, RgbaImage)>),
    GenerationProgress(u64, String),
    Workflow(Result<serde_json::Value>),
    Settings(Result<()>),
    SettingsApplied(Result<()>),
}
pub enum Storage {
    Save {
        doc: Document,
        path: PathBuf,
        revision: u64,
        epoch: u64,
        automatic: bool,
    },
    Export(Document, PathBuf, bool, u8),
    Switch {
        epoch: u64,
        previous: Document,
        archive: PathBuf,
        next: Option<PathBuf>,
    },
    Import(u64, Vec<PathBuf>),
    Workflow(PathBuf),
    Settings(providers::Settings, PathBuf),
    ApplySettings(providers::Settings, PathBuf, Option<String>),
    Barrier(Sender<()>),
}
pub struct Workers {
    pub storage: Sender<Storage>,
    pub render: Sender<(u64, Document, Viewport, bool)>,
    pub events: Receiver<Event>,
    pub sender: Sender<Event>,
}
pub fn start() -> Workers {
    let (sender, events) = mpsc::channel();
    let (storage, queue) = mpsc::channel();
    let tx = sender.clone();
    std::thread::spawn(move || {
        while let Ok(command) = queue.recv() {
            let event = match command {
                Storage::Save {
                    doc,
                    path,
                    revision,
                    epoch,
                    automatic,
                } => Event::Saved {
                    result: document::save(&path, &doc),
                    path,
                    revision,
                    epoch,
                    automatic,
                },
                Storage::Export(doc, path, transparent, quality) => {
                    #[cfg(target_os = "macos")]
                    let result = objc2::rc::autoreleasepool(|_| {
                        (|| {
                            let jpeg = path.extension().is_some_and(|e| {
                                e.eq_ignore_ascii_case("jpg") || e.eq_ignore_ascii_case("jpeg")
                            });
                            let image = crate::gpu_canvas::Renderer::new()?
                                .export(&doc, transparent && !jpeg)?;
                            document::export_image(&path, image, quality)
                        })()
                    });
                    #[cfg(not(target_os = "macos"))]
                    let result = document::export(&path, &doc, transparent, quality);
                    Event::Exported(result.map(|_| path))
                }
                Storage::Switch {
                    epoch,
                    previous,
                    archive,
                    next,
                } => Event::Switched(
                    epoch,
                    (|| {
                        document::save(&archive, &previous)?;
                        let doc = if let Some(path) = &next {
                            document::load(path)?
                        } else {
                            Document::default()
                        };
                        let images = doc
                            .references
                            .iter()
                            .map(|r| {
                                document::decode_reference(r)
                                    .map(|i| document::thumbnail(i, 800).to_rgba8())
                            })
                            .collect::<Result<Vec<_>>>()?;
                        Ok((doc, images, next))
                    })(),
                ),
                Storage::Import(epoch, paths) => Event::Imported(
                    epoch,
                    (|| {
                        let mut refs = vec![];
                        for path in paths {
                            anyhow::ensure!(
                                std::fs::metadata(&path)?.len() <= 20 * 1024 * 1024,
                                "Reference exceeds 20 MB"
                            );
                            let name = path
                                .file_stem()
                                .unwrap_or_default()
                                .to_string_lossy()
                                .into_owned();
                            let reference = document::reference_from_bytes(
                                name,
                                &std::fs::read(&path)?,
                                format!(
                                    "Imported · {}",
                                    path.file_name().unwrap_or_default().to_string_lossy()
                                ),
                            )?;
                            let image =
                                document::thumbnail(document::decode_reference(&reference)?, 800)
                                    .to_rgba8();
                            refs.push((reference, image));
                            anyhow::ensure!(
                                refs.iter()
                                    .map(|(reference, _)| reference.image.len())
                                    .sum::<usize>()
                                    <= 64 * 1024 * 1024,
                                "Selected references exceed 64 MB"
                            );
                        }
                        Ok(refs)
                    })(),
                ),
                Storage::Workflow(path) => Event::Workflow((|| {
                    anyhow::ensure!(
                        std::fs::metadata(&path)?.len() <= 2 * 1024 * 1024,
                        "Workflow exceeds 2 MB"
                    );
                    let value: serde_json::Value = serde_json::from_slice(&std::fs::read(path)?)?;
                    anyhow::ensure!(
                        value.is_object() && value.get("nodes").is_none(),
                        "Export workflow in API format, not the editor format"
                    );
                    Ok(value)
                })()),
                Storage::Settings(settings, path) => Event::Settings(
                    serde_json::to_vec(&settings)
                        .map_err(anyhow::Error::from)
                        .and_then(|bytes| document::atomic_write(&path, &bytes)),
                ),
                Storage::ApplySettings(settings, path, key) => Event::SettingsApplied(
                    apply_settings(&settings, &path, key.as_deref(), providers::store_key),
                ),
                Storage::Barrier(ack) => {
                    let _ = ack.send(());
                    continue;
                }
            };
            if tx.send(event).is_err() {
                break;
            }
        }
    });
    let (render, queue) = mpsc::channel::<(u64, Document, Viewport, bool)>();
    let tx = sender.clone();
    std::thread::spawn(move || {
        #[cfg(target_os = "macos")]
        let mut renderer = objc2::rc::autoreleasepool(|_| crate::canvas_renderer::Renderer::new());
        #[cfg(not(target_os = "macos"))]
        let mut renderer = crate::canvas_renderer::Renderer::new();
        while let Ok((revision, doc, view, tracing)) = queue.recv() {
            #[cfg(target_os = "macos")]
            let result = objc2::rc::autoreleasepool(|_| match renderer.as_mut() {
                Ok(renderer) => renderer.render(&doc, view, tracing),
                Err(error) => Err(anyhow::anyhow!("GPU canvas initialization: {error}")),
            });
            #[cfg(not(target_os = "macos"))]
            let result = match renderer.as_mut() {
                Ok(renderer) => renderer.render(&doc, view, tracing),
                Err(error) => Err(anyhow::anyhow!("Canvas initialization: {error}")),
            };
            if tx.send(Event::Raster(revision, result)).is_err() {
                break;
            }
        }
    });
    Workers {
        storage,
        render,
        events,
        sender,
    }
}
pub(crate) fn apply_settings(
    settings: &providers::Settings,
    path: &std::path::Path,
    key: Option<&str>,
    store_key: impl FnOnce(providers::Provider, &str) -> Result<()>,
) -> Result<()> {
    let bytes = serde_json::to_vec(settings)?;
    if let Some(key) = key.filter(|key| !key.trim().is_empty()) {
        store_key(settings.provider, key.trim()).context("API key was not saved")?;
    }
    document::atomic_write(path, &bytes).context("Settings file was not saved")
}
pub fn generate(mut request: Request, id: u64, sender: Sender<Event>, directory: PathBuf) {
    std::thread::spawn(move || {
        let progress_sender = sender.clone();
        request.progress = Some(Box::new(move |message| {
            let _ = progress_sender.send(Event::GenerationProgress(id, message));
        }));
        let result = providers::generate(request).and_then(|reference| {
            let decoded = document::decode_reference(&reference)?;
            let image = document::thumbnail(decoded.clone(), 800).to_rgba8();
            let stem = format!(
                "reference-{}-{id}",
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)?
                    .as_millis()
            );
            let mut png = std::io::Cursor::new(Vec::new());
            decoded.write_to(&mut png, image::ImageFormat::Png)?;
            document::atomic_write(&directory.join(format!("{stem}.png")), &png.into_inner())?;
            document::atomic_write(
                &directory.join(format!("{stem}.json")),
                &serde_json::to_vec(&reference)?,
            )?;
            Ok((reference, image))
        });
        let _ = sender.send(Event::Generated(id, result));
    });
}
pub fn render_image(mut image: RgbaImage) -> Arc<gpui_kit::RenderImage> {
    // GPUI's sprite atlas expects BGRA.
    for pixel in image.as_flat_samples_mut().samples.chunks_exact_mut(4) {
        pixel.swap(0, 2);
    }
    Arc::new(gpui_kit::RenderImage::new(vec![image::Frame::new(image)]))
}

pub fn studies(directory: PathBuf, sender: Sender<Event>) {
    std::thread::spawn(move || {
        let result = (|| -> Result<Vec<Study>> {
            if !directory.exists() {
                return Ok(vec![]);
            }
            let mut paths = std::fs::read_dir(directory)?
                .filter_map(|entry| entry.ok())
                .map(|entry| entry.path())
                .filter(|path| path.extension().is_some_and(|ext| ext == "atelier"))
                .collect::<Vec<_>>();
            paths.sort_by_key(|path| std::fs::metadata(path).and_then(|m| m.modified()).ok());
            paths.reverse();
            let mut studies = vec![];
            for path in paths.into_iter().take(48) {
                if let Ok(doc) = document::load(&path) {
                    let image = document::raster(&doc, 360, false)?;
                    studies.push(Study {
                        title: doc.title,
                        path,
                        image,
                        note: doc.note,
                    });
                }
            }
            Ok(studies)
        })();
        let _ = sender.send(Event::Studies(result));
    });
}
