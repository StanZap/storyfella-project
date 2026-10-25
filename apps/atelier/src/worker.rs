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

pub struct ProjectPreview {
    pub title: String,
    pub path: PathBuf,
    pub image: RgbaImage,
    pub note: String,
}
pub enum Event {
    ProjectClosed {
        epoch: u64,
        revision: u64,
        result: Result<PathBuf>,
    },
    WorkspaceSaved(Result<()>),
    ClipforgeOpened(Result<()>),
    Library(Result<Vec<(Reference, RgbaImage)>>),
    OpenedImage(String, Result<(RgbaImage, crate::edit::MaskDraft)>),
    ComparedImage(String, Result<RgbaImage>),
    MaskSaved(String, u64, Result<()>),
    LibraryUpdated(Result<()>),
    MaskPreview(u64, Result<RgbaImage>),
    Projects(Result<Vec<ProjectPreview>>),
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
    Imported(u64, bool, Result<Vec<(Reference, RgbaImage)>>),
    Generated(u64, Result<(Reference, RgbaImage)>),
    GenerationProgress(u64, String),
    Workflow(Result<serde_json::Value>),
    Settings(Result<()>),
    SettingsApplied(Result<()>),
    ServerStatus(crate::native_images::ServerStatus),
}
pub enum Storage {
    CloseProject {
        doc: Document,
        path: PathBuf,
        epoch: u64,
        revision: u64,
    },
    WorkspaceRecovery(PathBuf, Vec<(crate::project_tabs::RecoveryTab, Document)>),
    Artwork(Arc<Reference>, PathBuf),
    CanvasArtwork(Document, PathBuf, bool),
    /// Loads the shared reference library (read-only).
    Library(PathBuf),
    /// Adds originals to the shared reference library, then reloads it.
    LibraryAdd(PathBuf, Vec<Arc<Reference>>),
    LibraryUpdate(PathBuf, Arc<Reference>, crate::library::Update),
    OpenImage(Arc<Reference>, PathBuf),
    CompareImage(Arc<Reference>),
    MaskDraft(PathBuf, String, u64, crate::edit::MaskDraft),
    Original(Arc<Reference>, PathBuf),
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
        previous_path: Option<PathBuf>,
        next: Option<PathBuf>,
    },
    Import(u64, Vec<PathBuf>, PathBuf, bool),
    Workflow(PathBuf),
    Settings(providers::Settings, PathBuf),
    ApplySettings(providers::Settings, PathBuf),
    OpenClipforge,
    Barrier(Sender<()>),
}
pub struct Workers {
    pub native: Arc<crate::native_images::NativeImages>,
    pub mask: Sender<(u64, crate::edit::MaskDraft)>,
    pub storage: Sender<Storage>,
    pub probe: Sender<providers::Settings>,
    pub render: Sender<(u64, Document, Viewport, bool)>,
    pub events: Receiver<Event>,
    pub sender: Sender<Event>,
}
pub fn start() -> Workers {
    let (sender, events) = mpsc::channel();
    let (mask, mask_queue) = mpsc::channel::<(u64, crate::edit::MaskDraft)>();
    let mask_sender = sender.clone();
    std::thread::spawn(move || {
        while let Ok(mut pending) = mask_queue.recv() {
            while let Ok(next) = mask_queue.try_recv() {
                pending = next;
            }
            let (revision, draft) = pending;
            if mask_sender
                .send(Event::MaskPreview(revision, draft.preview(1024)))
                .is_err()
            {
                break;
            }
        }
    });
    let (storage, queue) = mpsc::channel();
    let tx = sender.clone();
    std::thread::spawn(move || {
        while let Ok(command) = queue.recv() {
            let event = match command {
                Storage::CloseProject {
                    doc,
                    path,
                    epoch,
                    revision,
                } => Event::ProjectClosed {
                    epoch,
                    revision,
                    result: document::save(&path, &doc).map(|_| path),
                },
                Storage::WorkspaceRecovery(directory, projects) => {
                    Event::WorkspaceSaved(crate::project_tabs::save_session(&directory, projects))
                }
                Storage::Artwork(reference, path) => Event::Exported((|| {
                    storyfella_studio_core::artwork::ApprovedArtwork {
                        version: 1,
                        name: reference.name.clone(),
                        image: reference.image.clone(),
                        prompt: reference.prompt.clone(),
                        provenance: reference.provenance.clone(),
                    }
                    .save(&path)?;
                    Ok(path)
                })()),
                Storage::CanvasArtwork(doc, path, transparent) => Event::Exported((|| {
                    let (prompt, provenance) = doc.artwork_metadata();
                    #[cfg(target_os = "macos")]
                    let image = objc2::rc::autoreleasepool(|_| {
                        crate::gpu_canvas::Renderer::new()?.export(&doc, transparent)
                    })?;
                    #[cfg(not(target_os = "macos"))]
                    let image = document::raster(&doc, doc.width.max(doc.height), transparent)?;
                    let mut bytes = std::io::Cursor::new(Vec::new());
                    image.write_to(&mut bytes, image::ImageFormat::Png)?;
                    let reference =
                        document::reference_from_bytes(doc.title, &bytes.into_inner(), provenance)?;
                    storyfella_studio_core::artwork::ApprovedArtwork {
                        version: 1,
                        name: reference.name,
                        image: reference.image,
                        prompt,
                        provenance: reference.provenance,
                    }
                    .save(&path)?;
                    Ok(path)
                })(
                )),
                Storage::Library(directory) => Event::Library((|| {
                    crate::library::load(&directory)?
                        .into_iter()
                        .map(|reference| {
                            let image =
                                document::thumbnail(document::decode_reference(&reference)?, 360)
                                    .to_rgba8();
                            Ok((reference, image))
                        })
                        .collect()
                })()),
                Storage::LibraryAdd(directory, references) => Event::Library((|| {
                    for reference in references {
                        crate::library::store(&directory, &reference)?;
                    }
                    crate::library::load(&directory)?
                        .into_iter()
                        .map(|reference| {
                            let image =
                                document::thumbnail(document::decode_reference(&reference)?, 360)
                                    .to_rgba8();
                            Ok((reference, image))
                        })
                        .collect()
                })()),
                Storage::LibraryUpdate(directory, reference, update) => {
                    Event::LibraryUpdated(crate::library::update(&directory, &reference, update))
                }
                Storage::OpenImage(reference, directory) => Event::OpenedImage(
                    crate::library::id(&reference),
                    (|| {
                        let image = document::decode_reference(&reference)?.to_rgba8();
                        let path =
                            directory.join(format!("{}.mask", crate::library::id(&reference)));
                        let draft = if path.exists() {
                            anyhow::ensure!(
                                std::fs::metadata(&path)?.len() <= 16 * 1024 * 1024,
                                "Mask draft exceeds 16 MB"
                            );
                            let draft: crate::edit::MaskDraft =
                                serde_json::from_slice(&std::fs::read(&path)?)?;
                            draft.validate()?;
                            anyhow::ensure!(
                                (draft.width, draft.height) == image.dimensions(),
                                "Saved mask dimensions do not match the image"
                            );
                            draft
                        } else {
                            crate::edit::MaskDraft {
                                width: image.width(),
                                height: image.height(),
                                strokes: vec![],
                            }
                        };
                        Ok((image, draft))
                    })(),
                ),
                Storage::CompareImage(reference) => Event::ComparedImage(
                    crate::library::id(&reference),
                    document::decode_reference(&reference).map(|image| image.to_rgba8()),
                ),
                Storage::MaskDraft(directory, id, revision, draft) => Event::MaskSaved(
                    id.clone(),
                    revision,
                    (|| {
                        draft.validate()?;
                        anyhow::ensure!(
                            id.len() == 64 && id.bytes().all(|b| b.is_ascii_hexdigit()),
                            "Invalid mask identifier"
                        );
                        document::atomic_write(
                            &directory.join(format!("{id}.mask")),
                            &serde_json::to_vec(&draft)?,
                        )
                    })(),
                ),
                Storage::Original(reference, path) => Event::Exported((|| {
                    document::export_image(
                        &path,
                        document::decode_reference(&reference)?.to_rgba8(),
                        95,
                    )?;
                    Ok(path)
                })()),
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
                    previous_path,
                    next,
                } => Event::Switched(
                    epoch,
                    (|| {
                        document::save(&archive, &previous)?;
                        if let Some(path) = previous_path {
                            document::save(&path, &previous)?;
                        }
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
                Storage::Import(epoch, paths, directory, into_project) => Event::Imported(
                    epoch,
                    into_project,
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
                                    <= if into_project {
                                        64 * 1024 * 1024
                                    } else {
                                        512 * 1024 * 1024
                                    },
                                "Selected images exceed the import size limit"
                            );
                        }
                        for (reference, _) in &refs {
                            // The picker imports to the shared collection before
                            // embedding the chosen image in its project.
                            crate::library::store(&directory, reference)?;
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
                Storage::ApplySettings(settings, path) => {
                    Event::SettingsApplied(apply_settings(&settings, &path))
                }
                Storage::OpenClipforge => Event::ClipforgeOpened(open_clipforge()),
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
    let (probe, probe_queue) = mpsc::channel::<providers::Settings>();
    let probe_sender = sender.clone();
    std::thread::spawn(move || {
        while let Ok(mut settings) = probe_queue.recv() {
            // Coalesce to the latest request; only the newest state matters.
            while let Ok(next) = probe_queue.try_recv() {
                settings = next;
            }
            if probe_sender
                .send(Event::ServerStatus(crate::native_images::server_status(
                    &settings,
                )))
                .is_err()
            {
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
        native: Arc::new(Default::default()),
        mask,
        storage,
        probe,
        render,
        events,
        sender,
    }
}
pub(crate) fn apply_settings(settings: &providers::Settings, path: &std::path::Path) -> Result<()> {
    let bytes = serde_json::to_vec(settings)?;
    document::atomic_write(path, &bytes).context("Settings file was not saved")
}
pub fn generate(mut request: Request, id: u64, sender: Sender<Event>, directory: PathBuf) {
    std::thread::spawn(move || {
        let mask = request.mask.clone();
        let source = request.source.clone();
        let parent = request
            .source
            .as_ref()
            .map(|source| crate::library::id(source));
        let prompt = request.prompt.clone();
        let progress_sender = sender.clone();
        request.progress = Some(Box::new(move |message| {
            let _ = progress_sender.send(Event::GenerationProgress(id, message));
        }));
        let result = providers::generate(request).and_then(|mut reference| {
            if let (Some(mask), Some(source)) = (&mask, &source) {
                reference = mask.composite(source, &reference)?;
            }
            reference.parent = parent;
            reference.prompt = prompt;
            if let Some(source) = &source {
                reference.name = format!("Edit · {}", source.name);
            }
            let directory = directory.join("generated");
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
    for pixel in image.as_flat_samples_mut().samples.as_chunks_mut::<4>().0 {
        pixel.swap(0, 2);
    }
    Arc::new(gpui_kit::RenderImage::new(vec![image::Frame::new(image)]))
}

fn open_clipforge() -> Result<()> {
    // Clipforge has a desktop identity but no setup deep-link contract.
    #[cfg(target_os = "macos")]
    {
        let status = std::process::Command::new("open")
            .args(["-b", "com.clipforge.desktop"])
            .status()?;
        anyhow::ensure!(
            status.success(),
            "Could not open Clipforge. Install its desktop app, then retry."
        );
    }
    #[cfg(not(target_os = "macos"))]
    {
        #[cfg(target_os = "windows")]
        let executable = "clipforge-desktop.exe";
        #[cfg(not(target_os = "windows"))]
        let executable = "clipforge-desktop";
        std::process::Command::new(executable).spawn()?;
    }
    Ok(())
}

pub fn projects(directory: PathBuf, sender: Sender<Event>) {
    std::thread::spawn(move || {
        let result = (|| -> Result<Vec<ProjectPreview>> {
            if !directory.exists() {
                return Ok(vec![]);
            }
            let mut paths = std::fs::read_dir(directory)?
                .filter_map(|entry| entry.ok())
                .map(|entry| entry.path())
                .filter(|path| {
                    path.extension().is_some_and(|ext| ext == "atelier")
                        && path
                            .file_name()
                            .is_none_or(|name| name != "recovery.atelier")
                })
                .collect::<Vec<_>>();
            paths.sort_by_key(|path| std::fs::metadata(path).and_then(|m| m.modified()).ok());
            paths.reverse();
            let mut projects = vec![];
            for path in paths.into_iter().take(48) {
                if let Ok(doc) = document::load(&path) {
                    let current_has_artwork = doc
                        .layers
                        .iter()
                        .any(|layer| !layer.strokes.is_empty() || layer.image.is_some());
                    let image = if current_has_artwork {
                        document::raster(&doc, 360, false)?
                    } else if let Some(canvas) = doc.canvases.iter().rev().find(|canvas| {
                        canvas
                            .layers
                            .iter()
                            .any(|layer| !layer.strokes.is_empty() || layer.image.is_some())
                    }) {
                        document::raster_layers(
                            canvas.width,
                            canvas.height,
                            &canvas.layers,
                            360,
                            false,
                        )?
                    } else if let Some(reference) = doc.references.last() {
                        document::thumbnail(document::decode_reference(reference)?, 360).to_rgba8()
                    } else {
                        document::raster(&doc, 360, false)?
                    };
                    projects.push(ProjectPreview {
                        title: doc.title,
                        path,
                        image,
                        note: doc.note,
                    });
                }
            }
            Ok(projects)
        })();
        let _ = sender.send(Event::Projects(result));
    });
}
