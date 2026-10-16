use crate::{
    document::{self, Brush, Document, History, Layer, Point as DocPoint, Stroke, Tool},
    native_images::ServerStatus,
    providers::{self, Settings},
    queue::{JobState, Queue, QueuedRequest},
    viewport::Viewport,
    worker::{self, Event, Storage, Workers},
};
use gpui_kit::{
    component::{
        input::{InputState, TextareaState},
        select::{SearchableVec, SelectEvent, SelectState},
        IndexPath,
    },
    *,
};
use std::{
    cell::Cell,
    path::PathBuf,
    rc::Rc,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc,
    },
    time::{Duration, Instant},
};

pub enum Picker {
    Artwork,
    Original,
    Save,
    Open,
    Import,
    Export,
    Workflow,
}
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Page {
    Draw,
    Library,
    Image,
}
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Panel {
    None,
    Canvas,
    Settings,
    Help,
    Brushes,
    Queue,
    FromDrawing,
}

/// The default prompt for "Generate from drawing" when the whole canvas is used.
pub(crate) const FROM_DRAWING_PROMPT: &str = "Generate an image using this drawing as a reference.";
/// The default prompt when a canvas region is selected. Box select crops the
/// reference to that region, so the rest of the image is composed to match it.
pub(crate) const FROM_DRAWING_REGION_PROMPT: &str = "Generate an image using the selected area of this drawing as a reference. Modify only the selected area and preserve the rest.";

/// Clamps two document-space corners to a pixel rectangle (x, y, width, height),
/// or `None` when a side is smaller than 16 px.
pub(crate) fn region_rect(
    a: DocPoint,
    b: DocPoint,
    width: u32,
    height: u32,
) -> Option<(u32, u32, u32, u32)> {
    let (width, height) = (width as f32, height as f32);
    let x0 = a.x.min(b.x).floor().clamp(0., width) as u32;
    let y0 = a.y.min(b.y).floor().clamp(0., height) as u32;
    let x1 = a.x.max(b.x).ceil().clamp(0., width) as u32;
    let y1 = a.y.max(b.y).ceil().clamp(0., height) as u32;
    let (w, h) = (x1.saturating_sub(x0), y1.saturating_sub(y0));
    (w >= 16 && h >= 16).then_some((x0, y0, w, h))
}

pub struct StudyCard {
    pub title: String,
    pub path: PathBuf,
    pub image: Arc<RenderImage>,
    pub note: String,
}
#[derive(Clone)]
pub enum CanvasImage {
    #[cfg(any(test, not(target_os = "macos")))]
    Image(Arc<RenderImage>),
    #[cfg(target_os = "macos")]
    Surface(Arc<crate::gpu_surface::Surface>),
}
fn drop_canvas_image(window: &mut Window, image: CanvasImage) {
    match image {
        #[cfg(any(test, not(target_os = "macos")))]
        CanvasImage::Image(image) => {
            let _ = window.drop_image(image);
        }
        #[cfg(target_os = "macos")]
        CanvasImage::Surface(_) => {}
    }
    let _ = window;
}
pub struct CanvasTile {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
    pub image: CanvasImage,
}
pub struct DrawingApp {
    pub library: Vec<(Arc<document::Reference>, Arc<RenderImage>)>,
    pub shared_library: bool,
    pub library_loading: bool,
    pub library_error: Option<String>,
    pub opened_image: Option<(Arc<document::Reference>, Arc<RenderImage>, u32, u32)>,
    pub opening_image: Option<Arc<document::Reference>>,
    pub image_zoom: f32,
    pub image_auto_fit: bool,
    pub image_pan: DocPoint,
    pub image_drag: Option<gpui_kit::Point<Pixels>>,
    pub image_bounds: Rc<Cell<Bounds<Pixels>>>,
    pub comparing: bool,
    pub comparison: Option<(String, Arc<RenderImage>, u32, u32)>,
    pub comparison_requested: Option<String>,
    pub mask: Option<crate::edit::MaskDraft>,
    pub mask_tool: Tool,
    pub mask_size: f32,
    pub mask_active: bool,
    pub mask_preview: Option<Arc<RenderImage>>,
    pub mask_revision: u64,
    pub mask_dirty: bool,
    pub mask_save_error: Option<String>,
    pub mask_redo: Vec<Stroke>,
    pub use_mask: bool,
    pub input_references: Vec<Arc<document::Reference>>,
    pub generation_view_id: Option<String>,
    pub studies: Vec<StudyCard>,
    pub show_studies: bool,
    pub studies_loading: bool,
    pub doc: Document,
    pub history: History,
    pub view: Viewport,
    pub tool: Tool,
    pub brush: Brush,
    pub brush_previews: Vec<Arc<RenderImage>>,
    pub pressure_enabled: bool,
    pub pressure_curve: f32,
    pub pen_pressure: Option<f32>,
    tablet: crate::tablet::TabletInput,
    pub pencil_size: f32,
    pub eraser_size: f32,
    pub opacity: f32,
    pub stabilize: bool,
    pub active_stroke: bool,
    pub last_pan: Option<gpui_kit::Point<Pixels>>,
    pub moving_image: bool,
    pub last_image_point: Option<DocPoint>,
    pub hover: Option<gpui_kit::Point<Pixels>>,
    pub bounds: Rc<Cell<Bounds<Pixels>>>,
    pub tiles: std::collections::HashMap<crate::canvas_renderer::TileKey, CanvasTile>,
    pub render_backend: &'static str,
    pub render_ms: f64,
    pub render_segments: usize,
    pub references: Vec<Arc<RenderImage>>,
    pub reference_cache: Vec<(Arc<document::Reference>, Arc<RenderImage>)>,
    pub ref_zoom: f32,
    pub ref_pan: DocPoint,
    pub ref_drag: Option<gpui_kit::Point<Pixels>>,
    pub page: Page,
    /// Where the image inspector should return when its Back control is used.
    pub image_return_page: Page,
    pub panel: Panel,
    pub dock: bool,
    pub grid: bool,
    pub guides: bool,
    pub grayscale: bool,
    pub favorite_filter: bool,
    pub tracing: bool,
    pub scale_art: bool,
    pub transparent: bool,
    pub jpeg: bool,
    pub jpeg_quality: u8,
    pub path: Option<PathBuf>,
    pub data_dir: PathBuf,
    pub dirty: bool,
    pub switching: bool,
    pub closing: bool,
    pub revision: u64,
    pub epoch: u64,
    pub visual_revision: u64,
    rendered_revision: u64,
    render_pending: bool,
    autosaved_revision: u64,
    save_pending: bool,
    manual_pending: bool,
    last_edit: Instant,
    autosave_retry_after: Instant,
    pub status: String,
    pub error: bool,
    pub settings: Settings,
    pub generation_provider_select: Entity<SelectState<Vec<String>>>,
    pub settings_provider_select: Entity<SelectState<Vec<String>>>,
    /// The searchable picker that adds a LoRA adapter to the local settings.
    pub lora_select: Entity<SelectState<SearchableVec<String>>>,
    /// Per-adapter weight inputs, keyed by the registered adapter name.
    pub lora_weights: std::collections::BTreeMap<String, Entity<InputState>>,
    /// The last probed state of the machine's shared local image server.
    pub server_status: Option<ServerStatus>,
    pub settings_save: Option<(Settings, String)>,
    pub settings_save_error: Option<String>,
    pub generation: Option<Arc<AtomicBool>>,
    pub generation_error: Option<String>,
    /// The queued and recently completed image requests, oldest first.
    pub queue: Queue,
    /// The canvas region (document coordinates) selected for "Generate from
    /// drawing"; `None` means the full canvas.
    pub generation_region: Option<(DocPoint, DocPoint)>,
    /// The drag anchor while a region is being selected on the canvas.
    pub(crate) region_drag: Option<DocPoint>,
    /// The prompt used by the "Generate from drawing" panel.
    pub from_drawing_prompt: Entity<TextareaState>,
    pub job_id: u64,
    pub job_epoch: u64,
    pub timer_seconds: u32,
    pub timer_remaining: Duration,
    pub timer_running: bool,
    timer_tick: Instant,
    pub exercise: usize,
    pub title: Entity<InputState>,
    pub layer_name: Entity<InputState>,
    pub width: Entity<InputState>,
    pub height: Entity<InputState>,
    pub prompt: Entity<TextareaState>,
    prompt_target: String,
    pub(crate) restore_draft_source: bool,
    pub search: Entity<InputState>,
    pub endpoint: Entity<InputState>,
    pub model: Entity<InputState>,
    pub edit_model: Entity<InputState>,
    pub key: Entity<InputState>,
    pub positive_node: Entity<InputState>,
    pub latent_node: Entity<InputState>,
    pub seed_node: Entity<InputState>,
    pub steps: Entity<InputState>,
    pub note: Entity<TextareaState>,
    pub ref_name: Entity<InputState>,
    pub ref_tags: Entity<InputState>,
    pub generation_width: Entity<InputState>,
    pub generation_height: Entity<InputState>,
    pub editing_image: bool,
    pub image_details_open: bool,
    pub focus: FocusHandle,
    pub(crate) workers: Workers,
    subscriptions: Vec<Subscription>,
}
impl DrawingApp {
    pub fn new(data_dir: PathBuf, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let recovered = document::load(&data_dir.join("recovery.atelier"));
        let (doc, status, error) = match recovered {
            Ok(doc) => (doc, "Recovered your last session".into(), false),
            Err(e) if data_dir.join("recovery.atelier").exists() => (
                Document::default(),
                format!("Recovery could not be opened: {e}"),
                true,
            ),
            Err(_) => (Document::default(), String::new(), false),
        };
        let settings: Settings = std::fs::read(data_dir.join("settings.json"))
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default();
        let input = |value: String, window: &mut Window, cx: &mut Context<Self>| {
            cx.new(|cx| InputState::new(window, cx).default_value(value))
        };
        let choices = providers::Provider::ALL
            .into_iter()
            .map(|p| p.name().to_owned())
            .collect::<Vec<_>>();
        let selected = providers::Provider::ALL
            .iter()
            .position(|p| *p == settings.provider)
            .map(IndexPath::new);
        let generation_provider_select =
            cx.new(|cx| SelectState::new(choices.clone(), selected, window, cx));
        let settings_provider_select = cx.new(|cx| SelectState::new(choices, selected, window, cx));
        let title = input(doc.title.clone(), window, cx);
        let layer_name = input(doc.layers[doc.active_layer].name.clone(), window, cx);
        let width = input(doc.width.to_string(), window, cx);
        let height = input(doc.height.to_string(), window, cx);
        let prompt_target = doc
            .active_prompt_target
            .clone()
            .unwrap_or_else(|| "new".into());
        let restoring_edit = prompt_target != "new";
        let prompt = cx.new(|cx| {
            TextareaState::new(window, cx)
                .placeholder("Describe your image or changes…")
                .default_value(
                    doc.prompt_drafts
                        .get(&prompt_target)
                        .cloned()
                        .unwrap_or_default(),
                )
        });
        let search = cx.new(|cx| InputState::new(window, cx).placeholder("Search references"));
        let endpoint = input(settings.endpoint.clone(), window, cx);
        let model = input(settings.model.clone(), window, cx);
        let edit_model = input(settings.edit_model.clone(), window, cx);
        let key_value = clipforge_inference::config::user_or_built_in()
            .ok()
            .and_then(|registry| providers::shared_stored_api_key(settings.provider, &registry))
            .unwrap_or_default();
        let key = cx.new(|cx| {
            InputState::new(window, cx)
                .default_value(key_value)
                .masked(true)
        });
        let positive_node = input(settings.positive_node.clone(), window, cx);
        let latent_node = input(settings.latent_node.clone(), window, cx);
        let seed_node = input(settings.seed_node.clone(), window, cx);
        let steps = cx.new(|cx| {
            InputState::new(window, cx)
                .default_value(
                    settings
                        .steps
                        .map(|steps| steps.to_string())
                        .unwrap_or_default(),
                )
                .step(1.0)
                .min(1.0)
        });
        let lora_weights: std::collections::BTreeMap<String, Entity<InputState>> = settings
            .loras
            .iter()
            .map(|lora| {
                let input = cx.new(|cx| {
                    InputState::new(window, cx)
                        .default_value(format!("{:.2}", lora.multiplier))
                        .step(0.05)
                        .min(0.0)
                        .max(2.0)
                });
                (lora.name.clone(), input)
            })
            .collect();
        let addable_loras: Vec<String> = crate::native_images::available_loras(&settings)
            .into_iter()
            .filter(|name| !settings.loras.iter().any(|lora| lora.name == *name))
            .collect();
        let lora_select = cx.new(|cx| {
            SelectState::new(SearchableVec::new(addable_loras), None, window, cx).searchable(true)
        });
        let generation_width = cx.new(|cx| {
            InputState::new(window, cx)
                .default_value(settings.width.to_string())
                .step(64.0)
                .min(256.0)
                .max(2048.0)
        });
        let generation_height = cx.new(|cx| {
            InputState::new(window, cx)
                .default_value(settings.height.to_string())
                .step(64.0)
                .min(256.0)
                .max(2048.0)
        });
        let from_drawing_prompt = cx.new(|cx| {
            TextareaState::new(window, cx)
                .placeholder("Describe the image to generate from your drawing…")
                .default_value(FROM_DRAWING_PROMPT.to_string())
        });
        let note = cx.new(|cx| {
            TextareaState::new(window, cx)
                .placeholder("What did you notice?")
                .default_value(doc.note.clone())
        });
        let ref_name = input(String::new(), window, cx);
        let ref_tags = input(String::new(), window, cx);
        let references: Vec<_> = doc
            .references
            .iter()
            .filter_map(|r| document::decode_reference(r).ok())
            .map(|i| worker::render_image(i.thumbnail(800, 800).to_rgba8()))
            .collect();
        let reference_cache = doc
            .references
            .iter()
            .cloned()
            .zip(references.iter().cloned())
            .collect();
        let mut app = Self {
            library: vec![],
            shared_library: true,
            library_loading: false,
            library_error: None,
            opened_image: None,
            opening_image: None,
            image_zoom: 1.,
            image_auto_fit: true,
            image_pan: DocPoint { x: 0., y: 0. },
            image_drag: None,
            image_bounds: Rc::new(Cell::new(Bounds::default())),
            comparing: false,
            comparison: None,
            comparison_requested: None,
            mask: None,
            mask_tool: Tool::Hand,
            mask_size: 64.,
            mask_active: false,
            mask_preview: None,
            mask_revision: 0,
            mask_dirty: false,
            mask_save_error: None,
            mask_redo: vec![],
            use_mask: false,
            input_references: vec![],
            generation_view_id: None,
            studies: vec![],
            show_studies: false,
            studies_loading: false,
            reference_cache,
            ref_zoom: 1.,
            ref_pan: DocPoint { x: 0., y: 0. },
            ref_drag: None,
            doc,
            history: History::new(),
            view: Viewport::default(),
            tool: Tool::Pencil,
            brush: Brush::Graphite,
            brush_previews: Brush::ALL
                .into_iter()
                .map(|brush| {
                    let mut preview = Document {
                        width: 256,
                        height: 64,
                        ..Document::default()
                    };
                    preview.layers[0].strokes.push(Stroke {
                        brush,
                        tool: Tool::Pencil,
                        size: brush.size().max(7.),
                        opacity: 0.9,
                        points: (0..64)
                            .map(|i| DocPoint {
                                x: 10. + i as f32 * 3.7,
                                y: 32. + (i as f32 * 0.1).sin() * 10.,
                            })
                            .collect::<Vec<_>>()
                            .into(),
                        pressures: (0..64)
                            .map(|i| 0.2 + i as f32 / 80.)
                            .collect::<Vec<_>>()
                            .into(),
                    });
                    worker::render_image(
                        document::raster(&preview, 256, false).expect("Valid brush preview"),
                    )
                })
                .collect::<Vec<_>>(),
            pressure_enabled: true,
            pressure_curve: 1.,
            pen_pressure: None,
            tablet: crate::tablet::TabletInput::new(),
            pencil_size: 5.,
            eraser_size: 40.,
            opacity: 0.8,
            stabilize: false,
            active_stroke: false,
            last_pan: None,
            moving_image: false,
            last_image_point: None,
            hover: None,
            bounds: Rc::new(Cell::new(Bounds::default())),
            tiles: std::collections::HashMap::new(),
            render_backend: "Starting",
            render_ms: 0.,
            render_segments: 0,
            references,
            page: Page::Draw,
            image_return_page: Page::Library,
            panel: Panel::None,
            dock: true,
            grid: false,
            guides: false,
            grayscale: false,
            favorite_filter: false,
            tracing: false,
            scale_art: false,
            transparent: false,
            jpeg: false,
            jpeg_quality: 90,
            path: None,
            data_dir,
            dirty: false,
            switching: false,
            closing: false,
            revision: 1,
            epoch: 1,
            visual_revision: 1,
            rendered_revision: 0,
            render_pending: false,
            autosaved_revision: 1,
            save_pending: false,
            manual_pending: false,
            last_edit: Instant::now(),
            autosave_retry_after: Instant::now(),
            status,
            error,
            settings,
            generation_provider_select,
            settings_provider_select,
            lora_select,
            lora_weights,
            server_status: None,
            settings_save: None,
            settings_save_error: None,
            generation: None,
            generation_error: None,
            queue: Queue::default(),
            generation_region: None,
            region_drag: None,
            from_drawing_prompt,
            job_id: 0,
            job_epoch: 0,
            timer_seconds: 300,
            timer_remaining: Duration::from_secs(300),
            timer_running: false,
            timer_tick: Instant::now(),
            exercise: 0,
            title,
            layer_name,
            width,
            height,
            prompt,
            prompt_target,
            restore_draft_source: restoring_edit,
            search,
            endpoint,
            model,
            edit_model,
            key,
            positive_node,
            latent_node,
            seed_node,
            steps,
            note,
            ref_name,
            ref_tags,
            generation_width,
            generation_height,
            editing_image: restoring_edit,
            image_details_open: false,
            focus: cx.focus_handle(),
            workers: worker::start(),
            subscriptions: vec![],
        };
        app.sync_ref_inputs(window, cx);
        app.refresh_library();
        app.refresh_server_status();
        app.subscriptions
            .push(cx.observe(&app.prompt, |this, _, cx| {
                this.capture_prompt(cx);
            }));
        for select in [
            &app.generation_provider_select,
            &app.settings_provider_select,
        ] {
            app.subscriptions.push(cx.subscribe_in(
                select,
                window,
                |this, _, event: &SelectEvent<Vec<String>>, window, cx| {
                    if let SelectEvent::Confirm(Some(value)) = event {
                        if let Some(provider) = providers::Provider::ALL
                            .into_iter()
                            .find(|p| p.name() == value)
                        {
                            if this.settings.provider != provider {
                                this.select_provider(provider, window, cx);
                            }
                        }
                    }
                },
            ));
        }
        app.subscriptions.push(cx.subscribe_in(
            &app.lora_select,
            window,
            |this, _, event: &SelectEvent<SearchableVec<String>>, window, cx| {
                if let SelectEvent::Confirm(Some(name)) = event {
                    this.add_lora(name.clone(), window, cx);
                }
            },
        ));
        let weak = cx.weak_entity();
        window.on_window_should_close(cx, move |_, cx| {
            weak.update(cx, |this, cx| {
                this.capture_prompt(cx);
                if !this.closing {
                    this.closing = true;
                    this.finish_stroke();
                    this.finish_mask();
                    if this.mask_dirty {
                        this.save_mask();
                    }
                    this.queue_save(this.data_dir.join("recovery.atelier"), true);
                    cx.notify();
                }
                false
            })
            .unwrap_or(true)
        });
        app.subscriptions.push(cx.on_app_quit(|this, cx| {
            this.capture_prompt(cx);
            this.finish_stroke();
            this.finish_mask();
            let path = this.data_dir.join("recovery.atelier");
            let (ack, rx) = mpsc::channel();
            let _ = this.workers.storage.send(Storage::Save {
                doc: this.doc.clone(),
                path,
                revision: this.revision,
                epoch: this.epoch,
                automatic: true,
            });
            let _ = this.workers.storage.send(Storage::Barrier(ack));
            cx.background_executor().spawn(async move {
                let _ = rx.recv_timeout(Duration::from_secs(3));
            })
        }));
        cx.spawn_in(window, async move |this, cx| loop {
            cx.background_executor()
                .timer(Duration::from_millis(16))
                .await;
            if this
                .update_in(cx, |this, window, cx| this.tick(window, cx))
                .is_err()
            {
                break;
            }
        })
        .detach();
        app
    }
    pub fn request_close(&mut self, cx: &mut Context<Self>) {
        self.capture_prompt(cx);
        if !self.closing {
            self.closing = true;
            self.finish_stroke();
            self.finish_mask();
            if self.mask_dirty {
                self.save_mask();
            }
            self.queue_save(self.data_dir.join("recovery.atelier"), true);
            cx.notify();
        }
    }
    pub fn message(&mut self, message: impl Into<String>, error: bool) {
        self.status = message.into();
        self.error = error;
    }
    fn capture_prompt(&mut self, cx: &mut Context<Self>) {
        let value = self.prompt.read(cx).value().to_string();
        if self.doc.active_prompt_target.as_deref() != Some(&self.prompt_target)
            || self
                .doc
                .prompt_drafts
                .get(&self.prompt_target)
                .map(String::as_str)
                .unwrap_or("")
                != value
        {
            self.doc
                .prompt_drafts
                .insert(self.prompt_target.clone(), value);
            self.doc.active_prompt_target = Some(self.prompt_target.clone());
            self.revision += 1;
            self.dirty = true;
            self.last_edit = Instant::now();
            cx.notify();
        }
    }
    pub fn restore_prompt(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let target = if self.editing_image {
            self.edit_source()
                .map(|r| crate::library::id(&r))
                .unwrap_or_else(|| "new".into())
        } else {
            "new".into()
        };
        if target == self.prompt_target {
            return;
        }
        self.capture_prompt(cx);
        self.prompt_target = target;
        let value = self
            .doc
            .prompt_drafts
            .get(&self.prompt_target)
            .cloned()
            .unwrap_or_default();
        self.prompt
            .update(cx, |s, cx| s.set_value(value, window, cx));
        self.generation_error = None;
    }
    pub fn changed(&mut self) {
        self.revision += 1;
        self.visual_revision += 1;
        self.dirty = true;
        self.last_edit = Instant::now();
    }
    pub fn view_changed(&mut self) {
        self.finish_stroke();
        self.visual_revision += 1;
    }
    pub fn checkpoint(&mut self) {
        self.finish_stroke();
        self.history.checkpoint(&self.doc);
    }
    pub fn finish_stroke(&mut self) {
        self.active_stroke = false;
        self.last_pan = None;
        self.last_image_point = None;
    }
    pub fn size(&self) -> f32 {
        if self.tool == Tool::Eraser {
            self.eraser_size
        } else {
            self.pencil_size
        }
    }
    pub fn brush_size(&mut self, multiplier: f32) {
        if self.tool == Tool::Eraser {
            self.eraser_size = (self.eraser_size * multiplier).clamp(1., 256.);
        } else {
            self.pencil_size = (self.pencil_size * multiplier).clamp(1., 128.);
        }
    }
    pub fn undo(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.finish_stroke();
        if self.history.undo(&mut self.doc) {
            self.changed();
            self.sync_inputs(window, cx);
        }
    }
    pub fn redo(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.finish_stroke();
        if self.history.redo(&mut self.doc) {
            self.changed();
            self.sync_inputs(window, cx);
        }
    }
    fn sync_inputs(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.prompt_target = self
            .doc
            .active_prompt_target
            .clone()
            .unwrap_or_else(|| "new".into());
        self.editing_image = self.prompt_target != "new";
        let draft = self
            .doc
            .prompt_drafts
            .get(&self.prompt_target)
            .cloned()
            .unwrap_or_default();
        self.prompt
            .update(cx, |s, cx| s.set_value(draft, window, cx));
        for (input, value) in [
            (&self.title, self.doc.title.clone()),
            (
                &self.layer_name,
                self.doc.layers[self.doc.active_layer].name.clone(),
            ),
            (&self.width, self.doc.width.to_string()),
            (&self.height, self.doc.height.to_string()),
        ] {
            input.update(cx, |s, cx| s.set_value(value, window, cx));
        }
        self.note
            .update(cx, |s, cx| s.set_value(self.doc.note.clone(), window, cx));
        self.sync_ref_inputs(window, cx);
    }
    pub fn sync_ref_inputs(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.references = self
            .doc
            .references
            .iter()
            .filter_map(|reference| {
                self.reference_cache
                    .iter()
                    .find(|(cached, _)| cached.image == reference.image)
                    .map(|(_, image)| image.clone())
            })
            .collect();
        let reference = self.doc.reference.and_then(|i| self.doc.references.get(i));
        let name = reference.map(|r| r.name.clone()).unwrap_or_default();
        let tags = reference.map(|r| r.tags.clone()).unwrap_or_default();
        self.ref_name
            .update(cx, |s, cx| s.set_value(name, window, cx));
        self.ref_tags
            .update(cx, |s, cx| s.set_value(tags, window, cx));
    }
    pub fn select_reference(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        self.capture_prompt(cx);
        self.ref_zoom = 1.;
        self.ref_pan = DocPoint { x: 0., y: 0. };
        self.doc.reference = Some(index);
        self.changed();
        self.sync_ref_inputs(window, cx);
        self.restore_prompt(window, cx);
    }
    /// Renames the inspected image (on the Image page) or the selected project
    /// reference, updating every copy the app holds: the project's own record,
    /// the inspected copy, the cached preview label and the shared record when
    /// the image is in the reference library.
    pub fn edit_reference(&mut self, cx: &mut Context<Self>) {
        let name = self.ref_name.read(cx).value().trim().to_owned();
        let tags = self.ref_tags.read(cx).value().to_string();
        if name.is_empty() || name.len() > 512 || tags.len() > 32_768 {
            self.message(
                "Enter a name (up to 512 bytes) and tags (up to 32 KB)",
                true,
            );
            return;
        }
        let target = if self.page == Page::Image {
            self.opened_image
                .as_ref()
                .map(|(reference, _, _, _)| crate::library::id(reference))
        } else {
            self.doc
                .reference
                .and_then(|i| self.doc.references.get(i))
                .map(|reference| crate::library::id(reference))
        };
        let Some(target) = target else {
            return;
        };
        if self
            .doc
            .references
            .iter()
            .any(|reference| crate::library::id(reference) == target)
        {
            self.checkpoint();
        }
        if let Some(i) = self
            .doc
            .references
            .iter()
            .position(|reference| crate::library::id(reference) == target)
        {
            let reference = Arc::make_mut(&mut self.doc.references[i]);
            reference.name = name.clone();
            reference.tags = tags.clone();
            if let Some((cached, _)) = self
                .reference_cache
                .iter_mut()
                .find(|(reference, _)| crate::library::id(reference) == target)
            {
                let cached = Arc::make_mut(cached);
                cached.name = name.clone();
                cached.tags = tags.clone();
            }
            self.changed();
        }
        if let Some((reference, _, _, _)) = &mut self.opened_image {
            if crate::library::id(reference) == target {
                let reference = Arc::make_mut(reference);
                reference.name = name.clone();
                reference.tags = tags.clone();
            }
        }
        if let Some(reference) = self
            .all_references()
            .into_iter()
            .find(|reference| crate::library::id(reference) == target)
        {
            let _ = self.workers.storage.send(Storage::LibraryUpdate(
                self.data_dir.join("library"),
                reference,
                crate::library::Update::Details,
            ));
        }
        self.message("Image details saved", false);
    }
    pub fn save_note(&mut self, cx: &mut Context<Self>) {
        self.checkpoint();
        self.doc.note = self.note.read(cx).value().to_string();
        self.changed();
        self.message("Reflection saved", false);
    }
    pub fn toggle_favorite(&mut self, index: usize) {
        self.checkpoint();
        let reference = Arc::make_mut(&mut self.doc.references[index]);
        reference.favorite = !reference.favorite;
        let _ = self.workers.storage.send(Storage::LibraryUpdate(
            self.data_dir.join("library"),
            self.doc.references[index].clone(),
            crate::library::Update::Favorite,
        ));
        self.changed();
    }
    pub fn toggle_library_favorite(&mut self, index: usize) {
        if let Some((reference, _)) = self.library.get_mut(index) {
            let updated = Arc::make_mut(reference);
            updated.favorite = !updated.favorite;
            let _ = self.workers.storage.send(Storage::LibraryUpdate(
                self.data_dir.join("library"),
                reference.clone(),
                crate::library::Update::Favorite,
            ));
        }
    }
    pub fn remove_reference(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(i) = self.doc.reference {
            self.checkpoint();
            self.doc.references.remove(i);
            self.references.remove(i);
            self.doc.reference = if self.doc.references.is_empty() {
                None
            } else {
                Some(i.min(self.doc.references.len() - 1))
            };
            self.changed();
            self.sync_ref_inputs(window, cx);
        }
    }
    pub fn apply_canvas(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let result = (|| -> anyhow::Result<()> {
            let width = self.width.read(cx).value().parse::<u32>()?;
            let height = self.height.read(cx).value().parse::<u32>()?;
            document::validate_size(width, height)?;
            self.checkpoint();
            self.doc.resize(width, height, self.scale_art)?;
            let title = self.title.read(cx).value().trim().to_owned();
            let layer_name = self.layer_name.read(cx).value().trim().to_owned();
            if !layer_name.is_empty() {
                self.doc.layers[self.doc.active_layer].name = layer_name;
            }
            self.doc.title = if title.is_empty() {
                "Untitled study".into()
            } else {
                title
            };
            Ok(())
        })();
        match result {
            Ok(()) => {
                self.changed();
                self.view = Viewport::default();
                self.panel = Panel::None;
                self.sync_inputs(window, cx);
            }
            Err(e) => self.message(format!("Canvas: {e}"), true),
        }
    }
    pub fn new_document(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        self.capture_prompt(cx);
        if !self.prepare_project_switch() {
            return;
        }
        self.finish_stroke();
        let filename = format!(
            "study-{}-{}.atelier",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis(),
            self.epoch
        );
        self.switching = self
            .workers
            .storage
            .send(Storage::Switch {
                epoch: self.epoch,
                previous: self.doc.clone(),
                archive: self.data_dir.join("sessions").join(filename),
                next: None,
            })
            .is_ok();
        self.message(
            if self.switching {
                "Archiving previous study…"
            } else {
                "Storage worker stopped"
            },
            !self.switching,
        );
    }
    pub fn refresh_studies(&mut self) {
        if !self.studies_loading {
            self.studies_loading = true;
            worker::studies(self.data_dir.join("sessions"), self.workers.sender.clone());
        }
    }
    pub fn open_study(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        self.capture_prompt(cx);
        if !self.prepare_project_switch() {
            return;
        }
        self.finish_stroke();
        let archive = self.data_dir.join("sessions").join(format!(
            "before-open-{}.atelier",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis()
        ));
        self.switching = self
            .workers
            .storage
            .send(Storage::Switch {
                epoch: self.epoch,
                previous: self.doc.clone(),
                archive,
                next: Some(path),
            })
            .is_ok();
        if !self.switching {
            self.message("Storage worker stopped", true);
        }
    }
    fn prepare_project_switch(&mut self) -> bool {
        if self.switching {
            return false;
        }
        self.finish_mask();
        if self.mask_save_error.is_some() {
            self.page = Page::Image;
            self.message("Save the current mask before switching projects", true);
            return false;
        }
        if self.mask_dirty {
            self.save_mask();
        }
        self.mask_save_error.is_none()
    }
    pub fn add_layer(&mut self) {
        if self.doc.layers.len() >= 24 {
            self.message("Maximum 24 layers", true);
            return;
        }
        self.checkpoint();
        self.doc.layers.push(Layer {
            name: format!("Layer {}", self.doc.layers.len() + 1),
            visible: true,
            opacity: 1.,
            strokes: vec![],
            image: None,
        });
        self.doc.active_layer = self.doc.layers.len() - 1;
        self.changed();
    }
    pub fn delete_layer(&mut self) {
        if self.doc.layers.len() > 1 {
            self.checkpoint();
            self.doc.layers.remove(self.doc.active_layer);
            self.doc.active_layer = self.doc.active_layer.min(self.doc.layers.len() - 1);
            self.changed();
        }
    }
    pub fn move_layer(&mut self, up: bool) {
        let i = self.doc.active_layer;
        let next = if up { i + 1 } else { i.saturating_sub(1) };
        if next < self.doc.layers.len() && next != i {
            self.checkpoint();
            self.doc.layers.swap(i, next);
            self.doc.active_layer = next;
            self.changed();
        }
    }
    pub fn paper_bounds(&self) -> Bounds<Pixels> {
        let available = self.bounds.get();
        let scale = self.view.scale(
            (available.size.width.into(), available.size.height.into()),
            (self.doc.width, self.doc.height),
        );
        let (w, h) = self.view.dimensions(self.doc.width, self.doc.height);
        Bounds::new(
            point(
                available.origin.x
                    + (available.size.width - px(w * scale)) / 2.
                    + px(self.view.pan.x),
                available.origin.y
                    + (available.size.height - px(h * scale)) / 2.
                    + px(self.view.pan.y),
            ),
            size(px(w * scale), px(h * scale)),
        )
    }
    pub fn doc_point(&self, p: gpui_kit::Point<Pixels>) -> Option<DocPoint> {
        let bounds = self.paper_bounds();
        if !bounds.contains(&p) {
            return None;
        }
        let scale = self.view.scale(
            (
                self.bounds.get().size.width.into(),
                self.bounds.get().size.height.into(),
            ),
            (self.doc.width, self.doc.height),
        );
        Some(self.view.to_document(
            DocPoint {
                x: f32::from(p.x - bounds.origin.x) / scale,
                y: f32::from(p.y - bounds.origin.y) / scale,
            },
            self.doc.width as f32,
            self.doc.height as f32,
        ))
    }
    pub fn pointer_down(
        &mut self,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.closing || self.switching {
            return;
        }
        window.focus(&self.focus, cx);
        self.finish_stroke();
        self.tablet.drain();
        if self.panel == Panel::FromDrawing && event.button == MouseButton::Left {
            if let Some(point) = self.doc_point(event.position) {
                self.region_drag = Some(point);
                self.generation_region = Some((point, point));
                cx.notify();
            }
            return;
        }
        if self.tool == Tool::Hand
            && self.moving_image
            && event.button == MouseButton::Left
            && self.doc.layers[self.doc.active_layer].image.is_some()
        {
            if let Some(point) = self.doc_point(event.position) {
                self.history.checkpoint(&self.doc);
                self.last_image_point = Some(point);
            }
            return;
        }
        if self.tool == Tool::Hand || event.button == MouseButton::Middle {
            self.last_pan = Some(event.position);
            return;
        }
        if let Some(p) = self.doc_point(event.position) {
            if !self.doc.layers[self.doc.active_layer].visible {
                self.message("Show this layer before drawing", true);
                return;
            }
            self.history.checkpoint(&self.doc);
            let brush_size = self.size();
            self.doc.layers[self.doc.active_layer].strokes.push(Stroke {
                brush: self.brush,
                pressures: (if self.pressure_enabled {
                    crate::tablet::current_pressure()
                        .map(|p| vec![p.powf(self.pressure_curve)])
                        .unwrap_or_default()
                } else {
                    vec![]
                })
                .into(),
                tool: self.tool,
                size: brush_size,
                opacity: if self.tool == Tool::Eraser {
                    1.
                } else {
                    self.opacity
                },
                points: vec![p].into(),
            });
            self.active_stroke = true;
            self.changed();
            cx.notify();
        }
    }
    pub fn pointer_move(&mut self, event: &MouseMoveEvent, cx: &mut Context<Self>) {
        if self.region_drag.is_some() {
            if let Some(point) = self.doc_point(event.position) {
                if let Some(anchor) = self.region_drag {
                    self.generation_region = Some((anchor, point));
                    cx.notify();
                }
            }
            return;
        }
        // Consume standalone tablet points before this later mouse event to preserve order.
        self.tablet_samples(cx);
        self.pointer_sample(event, crate::tablet::current_pressure(), cx);
    }
    fn pointer_sample(
        &mut self,
        event: &MouseMoveEvent,
        pressure: Option<f32>,
        cx: &mut Context<Self>,
    ) {
        self.hover = Some(event.position);
        self.pen_pressure = pressure;
        if let Some(last) = self.last_image_point {
            if event.dragging() {
                if let Some(point) = self.doc_point(event.position) {
                    match self
                        .doc
                        .reposition_image_layer(point.x - last.x, point.y - last.y, 1.)
                    {
                        Ok(()) => {
                            self.last_image_point = Some(point);
                            self.changed();
                        }
                        Err(error) => self.message(error.to_string(), true),
                    }
                }
            } else {
                self.last_image_point = None;
            }
            cx.notify();
            return;
        }
        if let Some(last) = self.last_pan {
            if event.pressed_button.is_some() {
                self.view.pan.x += f32::from(event.position.x - last.x);
                self.view.pan.y += f32::from(event.position.y - last.y);
                self.last_pan = Some(event.position);
            } else {
                self.last_pan = None;
            }
            cx.notify();
            return;
        }
        if self.active_stroke {
            if !event.dragging() {
                self.finish_stroke();
                return;
            }
            if let Some(mut p) = self.doc_point(event.position) {
                let stroke = self.doc.layers[self.doc.active_layer]
                    .strokes
                    .last_mut()
                    .unwrap();
                if let Some(previous) = stroke.points.last() {
                    if self.stabilize {
                        p.x = previous.x + (p.x - previous.x) * 0.55;
                        p.y = previous.y + (p.y - previous.y) * 0.55;
                    }
                    if (p.x - previous.x).hypot(p.y - previous.y) < 0.5
                        && stroke.pressures.last().is_none_or(|last| {
                            self.pen_pressure.is_none_or(|value| {
                                (*last - value.powf(self.pressure_curve)).abs() < 0.01
                            })
                        })
                    {
                        return;
                    }
                }
                if stroke.points.len() < 100_000 {
                    if self.pressure_enabled && stroke.pressures.is_empty() {
                        if let Some(pressure) = self.pen_pressure {
                            stroke.pressures =
                                vec![pressure.powf(self.pressure_curve); stroke.points.len()]
                                    .into();
                        }
                    }
                    if !stroke.pressures.is_empty() {
                        Arc::make_mut(&mut stroke.pressures)
                            .push(self.pen_pressure.unwrap_or(1.).powf(self.pressure_curve));
                    }
                    Arc::make_mut(&mut stroke.points).push(p);
                    self.changed();
                }
            } else {
                self.finish_stroke();
            } // Never connect a stroke across excursions outside paper.
        }
        cx.notify();
    }
    pub fn settings_from_inputs(&mut self, cx: &mut Context<Self>) {
        self.settings.endpoint = self.endpoint.read(cx).value().trim().to_owned();
        self.settings.model = self.model.read(cx).value().trim().to_owned();
        self.settings.edit_model = self.edit_model.read(cx).value().trim().to_owned();
        self.settings.positive_node = self.positive_node.read(cx).value().trim().to_owned();
        self.settings.latent_node = self.latent_node.read(cx).value().trim().to_owned();
        self.settings.seed_node = self.seed_node.read(cx).value().trim().to_owned();
        self.settings.steps = self
            .steps
            .read(cx)
            .value()
            .trim()
            .parse::<u32>()
            .ok()
            .filter(|steps| *steps > 0);
        if let Ok(width) = self.generation_width.read(cx).value().trim().parse::<u32>() {
            self.settings.width = width.clamp(256, 2048);
        }
        if let Ok(height) = self
            .generation_height
            .read(cx)
            .value()
            .trim()
            .parse::<u32>()
        {
            self.settings.height = height.clamp(256, 2048);
        }
        for lora in &mut self.settings.loras {
            if let Some(input) = self.lora_weights.get(&lora.name) {
                if let Ok(multiplier) = input.read(cx).value().trim().parse::<f32>() {
                    lora.multiplier = multiplier.clamp(0.0, 2.0);
                }
            }
        }
    }

    /// The adapters the local provider offers that are not applied yet.
    pub fn available_lora_names(&self) -> Vec<String> {
        crate::native_images::available_loras(&self.settings)
            .into_iter()
            .filter(|name| !self.settings.loras.iter().any(|lora| lora.name == *name))
            .collect()
    }

    /// Rebuilds the adapter picker's items and clears its selection.
    pub fn refresh_lora_select(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let names = self.available_lora_names();
        self.lora_select.update(cx, |state, cx| {
            state.set_items(SearchableVec::new(names), window, cx);
            state.set_selected_index(None, window, cx);
        });
    }

    /// Applies an adapter at full strength (the picker's confirm action).
    pub fn add_lora(&mut self, name: String, window: &mut Window, cx: &mut Context<Self>) {
        if self.settings.loras.iter().any(|lora| lora.name == name) {
            return;
        }
        let input = cx.new(|cx| {
            InputState::new(window, cx)
                .default_value("1.00")
                .step(0.05)
                .min(0.0)
                .max(2.0)
        });
        self.lora_weights.insert(name.clone(), input);
        self.settings.loras.push(providers::LoraSetting {
            name,
            multiplier: 1.0,
        });
        self.refresh_lora_select(window, cx);
        self.save_settings(cx);
    }

    /// Removes an applied adapter.
    pub fn remove_lora(&mut self, name: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.settings.loras.retain(|lora| lora.name != name);
        self.lora_weights.remove(name);
        self.refresh_lora_select(window, cx);
        self.save_settings(cx);
    }

    /// Re-probes the machine's shared local image server in the background.
    pub fn refresh_server_status(&mut self) {
        if self.settings.provider.native_architecture().is_none() {
            self.server_status = None;
            return;
        }
        let _ = self.workers.probe.send(self.settings.clone());
    }
    pub fn select_provider(
        &mut self,
        provider: providers::Provider,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.settings_from_inputs(cx);
        self.settings.select_provider(provider);
        self.generation_error = None;
        for select in [
            &self.generation_provider_select,
            &self.settings_provider_select,
        ] {
            select.update(cx, |state, cx| {
                state.set_selected_value(&provider.name().to_owned(), window, cx);
                cx.notify();
            });
        }
        self.model.update(cx, |s, cx| {
            s.set_value(self.settings.model.clone(), window, cx)
        });
        self.edit_model.update(cx, |s, cx| {
            s.set_value(self.settings.edit_model.clone(), window, cx)
        });
        self.endpoint.update(cx, |s, cx| {
            s.set_value(self.settings.endpoint.clone(), window, cx)
        });
        let key = clipforge_inference::config::user_or_built_in()
            .ok()
            .and_then(|registry| providers::shared_stored_api_key(provider, &registry))
            .unwrap_or_default();
        self.key.update(cx, |s, cx| s.set_value(key, window, cx));
        if !provider.supports_edit() {
            self.editing_image = false;
        }
        self.refresh_lora_select(window, cx);
        self.refresh_server_status();
        self.save_settings(cx);
        cx.notify();
    }
    pub fn select_image_preset(
        &mut self,
        model: &str,
        edit_model: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.model
            .update(cx, |s, cx| s.set_value(model, window, cx));
        self.edit_model
            .update(cx, |s, cx| s.set_value(edit_model, window, cx));
        if edit_model.is_empty() {
            self.editing_image = false;
        }
        self.save_settings(cx);
        cx.notify();
    }
    pub fn save_settings(&mut self, cx: &mut Context<Self>) {
        self.settings_from_inputs(cx);
        self.settings.remember_provider();
        if self
            .workers
            .storage
            .send(Storage::Settings(
                self.settings.clone(),
                self.data_dir.join("settings.json"),
            ))
            .is_err()
        {
            self.message("Storage worker stopped", true);
        }
    }
    pub fn apply_settings(&mut self, cx: &mut Context<Self>) {
        if self.settings_save.is_some() {
            return;
        }
        self.settings_from_inputs(cx);
        self.settings.remember_provider();
        let key = self.key.read(cx).value().trim().to_owned();
        self.settings_save_error = None;
        if self
            .workers
            .storage
            .send(Storage::ApplySettings(
                self.settings.clone(),
                self.data_dir.join("settings.json"),
                key.clone(),
            ))
            .is_err()
        {
            self.settings_save_error =
                Some("Storage worker stopped. Try restarting Atelier.".into());
            self.message("Storage worker stopped", true);
        } else {
            self.settings_save = Some((self.settings.clone(), key));
        }
    }

    /// Starts a new image from a game-art brief while leaving the prompt fully editable.
    pub fn apply_creation_recipe(
        &mut self,
        prompt: &str,
        width: u32,
        height: u32,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.editing_image = false;
        self.restore_prompt(window, cx);
        self.prompt.update(cx, |state, cx| {
            state.set_value(prompt.to_owned(), window, cx)
        });
        self.generation_width.update(cx, |state, cx| {
            state.set_value(width.to_string(), window, cx)
        });
        self.generation_height.update(cx, |state, cx| {
            state.set_value(height.to_string(), window, cx)
        });
        self.settings.width = width;
        self.settings.height = height;
        self.generation_error = None;
        self.capture_prompt(cx);
        cx.notify();
    }

    /// Validates the composer's current request, captures it as a queue entry,
    /// and starts it when the queue is idle. Validation stays here so a bad
    /// request never reaches the queue.
    pub fn enqueue_generation(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.restore_prompt(window, cx);
        self.settings_from_inputs(cx);
        let source = if self.editing_image {
            if !self.settings.provider.supports_edit() {
                self.message("Choose a hosted provider to edit an image", true);
                return;
            }
            match self.edit_source() {
                Some(source) => Some(source),
                None => {
                    self.message("Select an image from your collection to edit", true);
                    return;
                }
            }
        } else {
            None
        };
        let references: Vec<_> = if self.editing_image {
            self.input_references
                .iter()
                .filter(|r| source.as_ref().is_none_or(|source| r.image != source.image))
                .cloned()
                .collect()
        } else {
            vec![]
        };
        let mask = if self.editing_image && self.use_mask {
            match (&source, &self.opened_image, &self.mask) {
                (Some(source), Some((opened, _, _, _)), Some(mask))
                    if source.image == opened.image =>
                {
                    Some(mask.clone())
                }
                _ => {
                    self.generation_error = Some(
                        "Open the selected image to paint its editing mask, or turn off Use mask."
                            .into(),
                    );
                    return;
                }
            }
        } else {
            None
        };
        if let Some(provider) = self.settings.provider.hosted() {
            let model = if self.editing_image {
                &self.settings.edit_model
            } else {
                &self.settings.model
            };
            match clipforge_inference::images::HostedDeployment::new(provider, model) {
                Ok(deployment) => {
                    if references.len() + usize::from(source.is_some())
                        > deployment.max_reference_images()
                    {
                        self.generation_error = Some("This model accepts only one input image. Choose Qwen Edit 2511 for multiple references, or remove the extra references.".into());
                        return;
                    }
                    if mask.is_some() && !deployment.supports_mask() {
                        self.generation_error = Some("This model does not accept masks. Choose FLUX Fill in Settings, or turn off Use mask.".into());
                        return;
                    }
                    if deployment.supports_mask()
                        && mask.as_ref().is_none_or(|mask| mask.strokes.is_empty())
                    {
                        self.generation_error = Some("FLUX Fill needs an editing mask. Open the image and paint the area to change.".into());
                        return;
                    }
                }
                Err(error) => {
                    self.generation_error = Some(error.to_string());
                    return;
                }
            }
        }
        let prompt = self.prompt.read(cx).value().to_string();
        if prompt.trim().is_empty() {
            self.message(
                if self.editing_image {
                    "Describe what you want to change"
                } else {
                    "Describe the image you want"
                },
                true,
            );
            return;
        }
        self.generation_error = None;
        let view_id = self
            .opened_image
            .as_ref()
            .filter(|_| self.page == Page::Image)
            .map(|(r, _, _, _)| crate::library::id(r));
        self.queue.enqueue(
            QueuedRequest {
                settings: self.settings.clone(),
                prompt,
                source,
                references,
                mask,
                editing: self.editing_image,
                placement: None,
            },
            self.epoch,
            view_id,
        );
        self.message(format!("Queued · {} waiting", self.queue.pending()), false);
        self.pump_queue();
        cx.notify();
    }
    /// Starts the next queued request when nothing is running and the queue is
    /// not paused. A completion calls this again, so exactly one runs at a time.
    fn pump_queue(&mut self) {
        if self.generation.is_some() || self.queue.is_paused() {
            return;
        }
        let Some(id) = self.queue.next_queued() else {
            return;
        };
        let (epoch, view_id, request) = {
            let Some(item) = self.queue.get_mut(id) else {
                return;
            };
            item.state = JobState::Running {
                progress: String::new(),
            };
            (item.epoch, item.view_id.clone(), item.request.clone())
        };
        let cancel = Arc::new(AtomicBool::new(false));
        self.job_id = id;
        self.job_epoch = epoch;
        self.generation_view_id = view_id;
        let size = request.size();
        worker::generate(
            providers::Request {
                native: self.workers.native.clone(),
                settings: request.settings,
                prompt: request.prompt,
                size,
                source: request.source,
                references: request.references,
                mask: request.mask,
                progress: None,
                cancel: cancel.clone(),
            },
            id,
            self.workers.sender.clone(),
            self.data_dir.clone(),
        );
        self.generation = Some(cancel);
        self.message(
            format!("Generating · {} queued", self.queue.pending()),
            false,
        );
    }
    /// Pauses or resumes dispatch. A running image always finishes.
    pub fn queue_pause_toggle(&mut self, cx: &mut Context<Self>) {
        let paused = self.queue.toggle_paused();
        self.message(
            if paused {
                "Queue paused · the running image finishes"
            } else {
                "Queue resumed"
            },
            false,
        );
        if !paused {
            self.pump_queue();
        }
        cx.notify();
    }
    /// Drops finished history, keeping queued and running work.
    pub fn queue_clear_finished(&mut self, cx: &mut Context<Self>) {
        self.queue.clear_finished();
        self.message("Cleared finished requests", false);
        cx.notify();
    }
    /// Removes a queued (not yet started) request.
    pub fn queue_remove(&mut self, id: u64, cx: &mut Context<Self>) {
        self.queue.remove(id);
        cx.notify();
    }

    /// Re-queues a failed request with the same captured settings and source.
    pub fn queue_retry(&mut self, id: u64, cx: &mut Context<Self>) {
        let Some(item) = self.queue.get(id) else {
            return;
        };
        let JobState::Failed(_) = item.state else {
            return;
        };
        let request = item.request.clone();
        let epoch = item.epoch;
        let view_id = item.view_id.clone();
        self.queue.enqueue(request, epoch, view_id);
        self.message("Request re-queued", false);
        self.pump_queue();
        cx.notify();
    }

    /// Opens the generated reference associated with a completed queue item.
    pub fn open_queue_result(&mut self, id: u64, cx: &mut Context<Self>) {
        let Some(result_id) = self.queue.get(id).and_then(|item| item.result_id.clone()) else {
            return;
        };
        let Some(reference) = self
            .all_references()
            .into_iter()
            .find(|reference| crate::library::id(reference) == result_id)
        else {
            self.message("The generated image is no longer available", true);
            return;
        };
        self.open_image(reference, cx);
    }
    /// Every image the app can resolve: the project's own images first, then the
    /// shared reference library, deduplicated by content so a promoted image is
    /// not listed twice. A project image is the authoritative record when the
    /// same image also exists in the library.
    pub fn all_references(&self) -> Vec<Arc<document::Reference>> {
        let mut seen = std::collections::HashSet::new();
        self.doc
            .references
            .iter()
            .cloned()
            .chain(self.library.iter().map(|(reference, _)| reference.clone()))
            .filter(|reference| seen.insert(crate::library::id(reference)))
            .collect()
    }
    /// Whether the inspected image is in the shared reference library.
    pub fn image_is_shared(&self) -> bool {
        self.opened_image
            .as_ref()
            .is_some_and(|(reference, _, _, _)| {
                let id = crate::library::id(reference);
                self.library
                    .iter()
                    .any(|(candidate, _)| crate::library::id(candidate) == id)
            })
    }
    /// Adds the inspected image to the shared reference library so another
    /// project can reuse it.
    pub fn promote_opened_image(&mut self, cx: &mut Context<Self>) {
        let Some((reference, _, _, _)) = self.opened_image.clone() else {
            return;
        };
        if self.image_is_shared() {
            self.message("This image is already in the reference library", false);
            cx.notify();
            return;
        }
        if self
            .workers
            .storage
            .send(Storage::LibraryAdd(
                self.data_dir.join("library"),
                vec![reference.clone()],
            ))
            .is_err()
        {
            self.message("Storage worker stopped. Try restarting Atelier.", true);
        } else {
            self.message(
                format!("Added “{}” to the reference library", reference.name),
                false,
            );
        }
        cx.notify();
    }
    /// The selected canvas region as a clamped pixel rectangle (x, y, width,
    /// height), or `None` when the whole canvas is used or the selection is too
    /// small to be meaningful.
    pub fn region_bounds(&self) -> Option<(u32, u32, u32, u32)> {
        let (a, b) = self.generation_region?;
        region_rect(a, b, self.doc.width, self.doc.height)
    }

    /// Uses the whole canvas as the reference area and clears any selection.
    pub fn use_full_canvas(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.region_drag = None;
        self.generation_region = None;
        self.set_from_drawing_default(false, window, cx);
        cx.notify();
    }

    /// Opens the "Generate from drawing" panel and seeds its prompt for the
    /// current area mode.
    pub fn open_from_drawing(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.panel = Panel::FromDrawing;
        self.region_drag = None;
        self.set_from_drawing_default(self.generation_region.is_some(), window, cx);
        cx.notify();
    }

    /// Toggles box select. Turning it on seeds the region with the whole canvas,
    /// which a canvas drag then refines; turning it off returns to the full
    /// canvas.
    pub fn toggle_box_select(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.generation_region.is_some() {
            self.use_full_canvas(window, cx);
            return;
        }
        self.generation_region = Some((
            DocPoint { x: 0., y: 0. },
            DocPoint {
                x: self.doc.width as f32,
                y: self.doc.height as f32,
            },
        ));
        self.set_from_drawing_default(true, window, cx);
        cx.notify();
    }

    /// Replaces the panel prompt with the mode's default unless the user edited it.
    fn set_from_drawing_default(
        &mut self,
        region: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let default = if region {
            FROM_DRAWING_REGION_PROMPT
        } else {
            FROM_DRAWING_PROMPT
        };
        let current = self.from_drawing_prompt.read(cx).value().trim().to_owned();
        if current.is_empty()
            || current == FROM_DRAWING_PROMPT
            || current == FROM_DRAWING_REGION_PROMPT
        {
            self.from_drawing_prompt.update(cx, |state, cx| {
                state.set_value(default.to_string(), window, cx)
            });
        }
    }

    /// Called on canvas mouse-up, ending a region drag. A drag too small to be a
    /// real selection is discarded and the area mode follows the result.
    pub fn finish_drawing_region(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.region_drag = None;
        if self.generation_region.is_some() && self.region_bounds().is_none() {
            self.generation_region = None;
        }
        if self.panel == Panel::FromDrawing {
            self.set_from_drawing_default(self.region_bounds().is_some(), window, cx);
        }
    }

    /// Rasterizes the drawing (cropped to the selected region when one is set),
    /// opens it as the editing source and queues a generation with the panel's
    /// prompt.
    pub fn generate_from_drawing(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.settings.provider.supports_edit() {
            self.message(
                "Choose an editing provider — Qwen (local) or a hosted edit model — to generate from a drawing",
                true,
            );
            return;
        }
        self.finish_stroke();
        let prompt = self.from_drawing_prompt.read(cx).value().trim().to_owned();
        if prompt.is_empty() {
            self.message("Describe the image to generate from your drawing", true);
            return;
        }
        let raster = match document::raster(&self.doc, document::MAX_EDGE, true) {
            Ok(raster) => raster,
            Err(error) => {
                self.message(format!("Could not read the drawing: {error}"), true);
                return;
            }
        };
        let region = self.region_bounds();
        let image = match region {
            Some((x, y, w, h)) => image::imageops::crop_imm(&raster, x, y, w, h).to_image(),
            None => raster,
        };
        let label = match region {
            Some((_, _, w, h)) => format!("{} region {w}×{h}", self.doc.title),
            None => format!("{} drawing", self.doc.title),
        };
        let bytes = match (|| -> anyhow::Result<Vec<u8>> {
            let mut cursor = std::io::Cursor::new(Vec::new());
            image.write_to(&mut cursor, image::ImageFormat::Png)?;
            Ok(cursor.into_inner())
        })() {
            Ok(bytes) => bytes,
            Err(error) => {
                self.message(format!("Could not use the drawing: {error}"), true);
                return;
            }
        };
        let reference = match document::reference_from_bytes(
            label,
            &bytes,
            "Atelier drawing · used as a generation reference".into(),
        ) {
            Ok(reference) => Arc::new(reference),
            Err(error) => {
                self.message(format!("Could not use the drawing: {error}"), true);
                return;
            }
        };
        let preview = match document::decode_reference(&reference) {
            Ok(decoded) => worker::render_image(document::thumbnail(decoded, 800).to_rgba8()),
            Err(error) => {
                self.message(format!("Could not use the drawing: {error}"), true);
                return;
            }
        };
        self.checkpoint();
        self.reference_cache
            .push((reference.clone(), preview.clone()));
        self.doc.references.push(reference.clone());
        self.references.push(preview);
        self.doc.reference = Some(self.doc.references.len() - 1);
        self.changed();
        self.panel = Panel::None;
        self.use_mask = false;
        self.editing_image = false;
        // Seed the composer prompt for this new source so the request carries it.
        self.settings_from_inputs(cx);
        let target = crate::library::id(&reference);
        self.prompt_target = target.clone();
        self.doc.active_prompt_target = Some(target.clone());
        self.doc
            .prompt_drafts
            .insert(target.clone(), prompt.clone());
        self.prompt
            .update(cx, |state, cx| state.set_value(prompt.clone(), window, cx));
        self.queue.enqueue(
            QueuedRequest {
                settings: self.settings.clone(),
                prompt,
                source: Some(reference),
                references: vec![],
                mask: None,
                editing: true,
                placement: region,
            },
            self.epoch,
            Some(target),
        );
        self.pump_queue();
        self.message(
            if region.is_some() {
                "Queued from the selected region"
            } else {
                "Queued from the drawing"
            },
            false,
        );
        cx.notify();
    }

    /// Stops the running request. Its result is discarded when it arrives.
    pub fn cancel_generation(&mut self) {
        if let Some(cancel) = &self.generation {
            cancel.store(true, Ordering::Relaxed);
            if let Some(item) = self.queue.get_mut(self.job_id) {
                item.state = JobState::Running {
                    progress: "Cancelling…".into(),
                };
            }
            self.message("Cancellation requested · waiting for provider", false);
        }
    }
    pub fn save(&mut self, save_as: bool, window: &mut Window, cx: &mut Context<Self>) {
        self.capture_prompt(cx);
        self.finish_stroke();
        if self.manual_pending {
            return;
        }
        if !save_as {
            if let Some(path) = self.path.clone() {
                self.queue_save(path, false);
                return;
            }
        }
        self.pick(Picker::Save, window, cx);
    }
    fn queue_save(&mut self, path: PathBuf, automatic: bool) {
        if automatic {
            self.save_pending = true;
        } else {
            self.manual_pending = true;
        }
        if self
            .workers
            .storage
            .send(Storage::Save {
                doc: self.doc.clone(),
                path,
                revision: self.revision,
                epoch: self.epoch,
                automatic,
            })
            .is_err()
        {
            self.message("Storage worker stopped", true);
            self.save_pending = false;
            self.manual_pending = false;
        }
    }
    pub fn pick(&mut self, picker: Picker, window: &mut Window, cx: &mut Context<Self>) {
        self.finish_stroke();
        let epoch = self.epoch;
        let import_into_project = self.page != Page::Library || !self.shared_library;
        let export_image = self.opened_image.as_ref().map(|(r, _, _, _)| r.clone());
        let export_canvas =
            (self.page != Page::Image).then(|| (self.doc.clone(), self.transparent));
        let entity = cx.weak_entity();
        match picker {
            Picker::Save | Picker::Export | Picker::Original | Picker::Artwork => {
                let suggested = if matches!(picker, Picker::Save) {
                    "Untitled.atelier"
                } else if matches!(picker, Picker::Artwork) {
                    "Approved.sfart"
                } else if matches!(picker, Picker::Original) {
                    "Original.png"
                } else if self.jpeg {
                    "Study.jpg"
                } else {
                    "Study.png"
                };
                let directory = self
                    .path
                    .as_ref()
                    .and_then(|p| p.parent())
                    .unwrap_or(&self.data_dir);
                let receiver = cx.prompt_for_new_path(directory, Some(suggested));
                window
                    .spawn(cx, async move |cx| {
                        let result = receiver.await;
                        let _ = entity.update_in(cx, |this, _, cx| {
                            if epoch != this.epoch {
                                return;
                            }
                            match result {
                                Ok(Ok(Some(mut path))) => {
                                    if matches!(picker, Picker::Save) {
                                        if path.extension().is_none() {
                                            path.set_extension("atelier");
                                        }
                                        this.queue_save(path, false);
                                    } else if matches!(picker, Picker::Artwork) {
                                        if path.extension().is_none() {
                                            path.set_extension("sfart");
                                        }
                                        if let Some((doc, transparent)) = &export_canvas {
                                            let _ =
                                                this.workers.storage.send(Storage::CanvasArtwork(
                                                    doc.clone(),
                                                    path,
                                                    *transparent,
                                                ));
                                            this.message(
                                                "Exporting approved canvas artwork…",
                                                false,
                                            );
                                        } else if let Some(reference) = &export_image {
                                            let _ = this
                                                .workers
                                                .storage
                                                .send(Storage::Artwork(reference.clone(), path));
                                            this.message("Exporting approved artwork…", false);
                                        }
                                    } else if matches!(picker, Picker::Original) {
                                        if path.extension().is_none() {
                                            path.set_extension("png");
                                        }
                                        if let Some(reference) = &export_image {
                                            let _ = this
                                                .workers
                                                .storage
                                                .send(Storage::Original(reference.clone(), path));
                                            this.message("Exporting original…", false);
                                        }
                                    } else {
                                        if path.extension().is_none() {
                                            path.set_extension(if this.jpeg {
                                                "jpg"
                                            } else {
                                                "png"
                                            });
                                        }
                                        let _ = this.workers.storage.send(Storage::Export(
                                            this.doc.clone(),
                                            path,
                                            this.transparent,
                                            this.jpeg_quality,
                                        ));
                                        this.message("Exporting…", false);
                                    }
                                }
                                Ok(Ok(None)) => {}
                                _ => this.message("File dialog could not be opened", true),
                            }
                            cx.notify();
                        });
                    })
                    .detach();
            }
            _ => {
                let receiver = cx.prompt_for_paths(PathPromptOptions {
                    files: true,
                    directories: false,
                    multiple: matches!(picker, Picker::Import),
                    prompt: Some(
                        match picker {
                            Picker::Open => "Open Atelier project",
                            Picker::Import => "Import reference images",
                            _ => "Load ComfyUI API workflow",
                        }
                        .into(),
                    ),
                });
                window
                    .spawn(cx, async move |cx| {
                        let result = receiver.await;
                        let _ = entity.update_in(cx, |this, _, cx| {
                            if epoch != this.epoch && import_into_project {
                                return;
                            }
                            match result {
                                Ok(Ok(Some(paths))) if !paths.is_empty() => match picker {
                                    Picker::Open => {
                                        this.open_study(paths[0].clone(), cx);
                                    }
                                    Picker::Import => {
                                        if import_into_project
                                            && this.doc.references.len() + paths.len() > 128
                                        {
                                            this.message(
                                                "Maximum 128 references per project",
                                                true,
                                            );
                                        } else {
                                            let _ = this.workers.storage.send(Storage::Import(
                                                epoch,
                                                paths,
                                                this.data_dir.join("library"),
                                                import_into_project,
                                            ));
                                            this.message("Importing references…", false);
                                        }
                                    }
                                    Picker::Workflow => {
                                        let _ = this
                                            .workers
                                            .storage
                                            .send(Storage::Workflow(paths[0].clone()));
                                    }
                                    _ => {}
                                },
                                Ok(Ok(None)) => {}
                                _ => this.message("File dialog could not be opened", true),
                            }
                            cx.notify();
                        });
                    })
                    .detach();
            }
        }
    }
    fn tablet_samples(&mut self, cx: &mut Context<Self>) {
        for sample in self.tablet.drain() {
            if self.active_stroke && sample.pressure > 0. {
                self.pointer_sample(
                    &MouseMoveEvent {
                        position: gpui_kit::point(px(sample.x), px(sample.y)),
                        pressed_button: Some(MouseButton::Left),
                        modifiers: Modifiers::default(),
                    },
                    Some(sample.pressure),
                    cx,
                );
            }
        }
    }
    fn tick(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.restore_draft_source && self.opening_image.is_none() {
            self.restore_prompt(window, cx);
        }
        if self.page == Page::Image && self.image_auto_fit && self.opened_image.is_some() {
            let bounds = self.image_bounds.get();
            if f32::from(bounds.size.width) > 48. && f32::from(bounds.size.height) > 48. {
                self.image_fit();
            }
        }
        self.tablet_samples(cx);
        let mut refresh = false;
        while let Ok(event) = self.workers.events.try_recv() {
            refresh = true;
            match event {
                Event::Settings(result) => match result {
                    Ok(()) => self.message("Settings saved", false),
                    Err(e) => self.message(format!("Settings: {e}"), true),
                },
                Event::SettingsApplied(result) => {
                    let pending = self.settings_save.take();
                    match result {
                        Ok(()) => {
                            if let Some((saved, _)) = pending {
                                self.settings_from_inputs(cx);
                                self.settings.remember_provider();
                                if self.panel == Panel::Settings && self.settings == saved {
                                    self.panel = Panel::None;
                                    window.focus(&self.focus, cx);
                                }
                                self.message("Settings saved", false);
                            }
                        }
                        Err(error) => {
                            let message = format!("Could not save settings: {error:#}");
                            self.settings_save_error = Some(message.clone());
                            self.message(message, true);
                        }
                    }
                }
                Event::Studies(result) => {
                    self.studies_loading = false;
                    match result {
                        Ok(studies) => {
                            self.studies = studies
                                .into_iter()
                                .map(|study| StudyCard {
                                    title: study.title,
                                    path: study.path,
                                    image: worker::render_image(study.image),
                                    note: study.note,
                                })
                                .collect()
                        }
                        Err(e) => self.message(format!("Study gallery: {e}"), true),
                    }
                }
                Event::Raster(revision, result) => {
                    self.render_pending = false;
                    match result {
                        Ok(batch) => {
                            if batch.reset {
                                for (_, tile) in self.tiles.drain() {
                                    drop_canvas_image(window, tile.image);
                                }
                            }
                            for tile in batch.tiles {
                                let width = tile.image.width();
                                let height = tile.image.height();
                                if let Some(old) = self.tiles.insert(
                                    tile.key,
                                    CanvasTile {
                                        x: tile.x,
                                        y: tile.y,
                                        width,
                                        height,
                                        image: match tile.image {
                                            #[cfg(any(test, not(target_os = "macos")))]
                                            crate::canvas_renderer::TileContent::Image(image) => {
                                                CanvasImage::Image(worker::render_image(image))
                                            }
                                            #[cfg(target_os = "macos")]
                                            crate::canvas_renderer::TileContent::Surface(
                                                surface,
                                            ) => CanvasImage::Surface(surface),
                                        },
                                    },
                                ) {
                                    drop_canvas_image(window, old.image);
                                }
                            }
                            self.render_backend = batch.backend;
                            self.render_ms = batch.elapsed_ms;
                            self.render_segments = batch.segments;
                            self.rendered_revision = revision;
                        }
                        Err(e) => {
                            self.message(format!("Canvas rendering: {e}"), true);
                            self.rendered_revision = revision;
                        }
                    }
                }
                Event::Saved {
                    revision,
                    epoch,
                    path,
                    automatic,
                    result,
                } => {
                    if automatic && path == self.data_dir.join("recovery.atelier") {
                        self.save_pending = false;
                    }
                    if !automatic {
                        self.manual_pending = false;
                    }
                    match result {
                        Ok(()) => {
                            if epoch == self.epoch {
                                if automatic && path == self.data_dir.join("recovery.atelier") {
                                    self.autosaved_revision = revision;
                                    if self.closing
                                        && revision == self.revision
                                        && !self.mask_dirty
                                        && self.mask_save_error.is_none()
                                    {
                                        window.remove_window();
                                        return;
                                    }
                                } else if !automatic {
                                    self.path = Some(path);
                                    if revision == self.revision {
                                        self.dirty = false;
                                    }
                                    self.message("Project saved", false);
                                }
                            }
                        }
                        Err(e) => {
                            self.closing = false;
                            self.autosave_retry_after = Instant::now() + Duration::from_secs(5);
                            self.message(format!("Save failed: {e}"), true)
                        }
                    }
                }
                Event::Exported(result) => match result {
                    Ok(path) => self.message(
                        format!(
                            "Exported {}",
                            path.file_name().unwrap_or_default().to_string_lossy()
                        ),
                        false,
                    ),
                    Err(e) => self.message(format!("Export failed: {e}"), true),
                },
                Event::Switched(epoch, result) => {
                    self.switching = false;
                    if epoch != self.epoch {
                        continue;
                    }
                    // Mask writes precede the switch on the storage queue. Keep
                    // the current study and draft if that write failed.
                    if self.mask_save_error.is_some() {
                        self.page = Page::Image;
                        continue;
                    }
                    match result {
                        Ok((doc, images, path)) => {
                            self.finish_stroke();
                            self.doc = doc;
                            self.epoch += 1;
                            self.revision += 1;
                            self.visual_revision += 1;
                            self.dirty = path.is_none();
                            self.path = path;
                            self.history = History::new();
                            self.view = Viewport::default();
                            self.references =
                                images.into_iter().map(worker::render_image).collect();
                            self.reference_cache = self
                                .doc
                                .references
                                .iter()
                                .cloned()
                                .zip(self.references.iter().cloned())
                                .collect();
                            self.page = Page::Draw;
                            self.sync_inputs(window, cx);
                            self.restore_saved_source(cx);
                            self.message("Study ready · previous study archived", false);
                        }
                        Err(e) => self.message(format!("Could not switch studies: {e}"), true),
                    }
                }
                Event::Library(result) => {
                    self.library_loading = false;
                    match result {
                        Ok(items) => {
                            self.library = items
                                .into_iter()
                                .map(|(reference, image)| {
                                    (Arc::new(reference), worker::render_image(image))
                                })
                                .collect();
                            if self.restore_draft_source {
                                self.restore_saved_source(cx);
                            }
                        }
                        Err(error) => {
                            self.library_error = Some(format!("Could not load library: {error:#}"))
                        }
                    }
                }
                Event::LibraryUpdated(result) => match result {
                    Ok(()) => self.refresh_library(),
                    Err(error) => {
                        self.library_error =
                            Some(format!("Library details were not saved: {error:#}"));
                        self.message(format!("Library details were not saved: {error:#}"), true);
                    }
                },
                Event::OpenedImage(id, result) => {
                    if self
                        .opening_image
                        .as_ref()
                        .is_some_and(|r| crate::library::id(r) == id)
                    {
                        let reference = self.opening_image.take().expect("Matched pending image");
                        match result {
                            Ok((image, draft)) => {
                                let width = image.width();
                                let height = image.height();
                                if let Some((_, old, _, _)) = self.opened_image.take() {
                                    let _ = window.drop_image(old);
                                }
                                self.opened_image =
                                    Some((reference, worker::render_image(image), width, height));
                                self.restore_prompt(window, cx);
                                if let Some((reference, _, _, _)) = &self.opened_image {
                                    self.ref_name.update(cx, |s, cx| {
                                        s.set_value(reference.name.clone(), window, cx)
                                    });
                                    self.ref_tags.update(cx, |s, cx| {
                                        s.set_value(reference.tags.clone(), window, cx)
                                    });
                                }
                                self.mask = Some(draft);
                                self.mask_dirty = false;
                                self.mask_save_error = None;
                                if let Some(old) = self.mask_preview.take() {
                                    let _ = window.drop_image(old);
                                }
                                let (width, height) = if width > height {
                                    (1536, 1024)
                                } else if height > width {
                                    (1024, 1536)
                                } else {
                                    (1024, 1024)
                                };
                                self.generation_width
                                    .update(cx, |s, cx| s.set_value(width.to_string(), window, cx));
                                self.generation_height.update(cx, |s, cx| {
                                    s.set_value(height.to_string(), window, cx)
                                });
                                self.settings.width = width;
                                self.settings.height = height;
                                self.mask_redo.clear();
                                self.use_mask = false;
                                self.refresh_mask_preview();
                                self.image_fit();
                            }
                            Err(error) => {
                                self.message(format!("Could not open image: {error:#}"), true)
                            }
                        }
                    }
                }
                Event::ComparedImage(id, result) => {
                    if self.comparison_requested.as_ref() == Some(&id) {
                        self.comparison_requested = None;
                        match result {
                            Ok(image) => {
                                let (width, height) = image.dimensions();
                                if let Some((_, old, _, _)) = self.comparison.take() {
                                    let _ = window.drop_image(old);
                                }
                                self.comparison =
                                    Some((id, worker::render_image(image), width, height));
                                self.comparing = true;
                            }
                            Err(error) => self.message(
                                format!("Could not open original version: {error:#}"),
                                true,
                            ),
                        }
                    }
                }
                Event::MaskSaved(id, revision, result) => match result {
                    Ok(()) => {
                        if revision == self.mask_revision
                            && self
                                .opened_image
                                .as_ref()
                                .is_some_and(|(r, _, _, _)| crate::library::id(r) == id)
                        {
                            self.mask_dirty = false;
                            self.mask_save_error = None;
                        }
                    }
                    Err(error) => {
                        let message = format!("Mask save failed: {error:#}. Check disk space and retry saving the mask.");
                        self.mask_save_error = Some(message.clone());
                        self.closing = false;
                        self.opening_image = None;
                        self.message(message, true);
                    }
                },
                Event::MaskPreview(revision, result) => {
                    if revision == self.mask_revision {
                        match result {
                            Ok(image) => {
                                if let Some(old) = self.mask_preview.take() {
                                    let _ = window.drop_image(old);
                                }
                                self.mask_preview = Some(worker::render_image(image));
                            }
                            Err(error) => {
                                self.message(format!("Mask preview failed: {error:#}"), true)
                            }
                        }
                    }
                }
                Event::Imported(epoch, into_project, result) => {
                    self.refresh_library();
                    if !into_project {
                        match result {
                            Ok(_) => {
                                self.message("Images saved to the shared reference library", false)
                            }
                            Err(error) => self.message(format!("Import failed: {error:#}"), true),
                        }
                        continue;
                    }
                    if epoch != self.epoch {
                        self.message("Import discarded · the project changed", true);
                        continue;
                    }
                    match result {
                        Ok(refs) => {
                            let total = self.doc.embedded_image_bytes()
                                + refs.iter().map(|(r, _)| r.image.len()).sum::<usize>();
                            if self.doc.references.len() + refs.len() > 128
                                || total > 64 * 1024 * 1024
                            {
                                self.message(
                                    "Project reference limit reached (128 images / 64 MB)",
                                    true,
                                );
                                continue;
                            }
                            self.checkpoint();
                            for (reference, image) in refs {
                                let reference = Arc::new(reference);
                                let image = worker::render_image(image);
                                self.reference_cache
                                    .push((reference.clone(), image.clone()));
                                self.doc.references.push(reference);
                                self.references.push(image);
                            }
                            self.doc.reference = Some(self.doc.references.len() - 1);
                            self.changed();
                            self.sync_ref_inputs(window, cx);
                            self.message("References imported", false);
                        }
                        Err(e) => self.message(format!("Import failed: {e}"), true),
                    }
                }
                Event::Generated(id, result) => {
                    self.refresh_library();
                    self.refresh_server_status();
                    if id == self.job_id {
                        let cancelled = self
                            .generation
                            .take()
                            .is_some_and(|c| c.load(Ordering::Relaxed));
                        if cancelled {
                            if let Some(item) = self.queue.get_mut(id) {
                                item.state = JobState::Cancelled;
                            }
                            self.message(
                                "Generation cancelled; provider charges may still apply",
                                false,
                            );
                        } else {
                            match result {
                                Ok((reference, image)) => {
                                    let result_id = crate::library::id(&reference);
                                    let placement =
                                        self.queue.get(id).and_then(|item| item.request.placement);
                                    if self.job_epoch == self.epoch
                                        && self.doc.references.len() < 128
                                        && self.doc.embedded_image_bytes() + reference.image.len()
                                            <= 64 * 1024 * 1024
                                    {
                                        let reference = Arc::new(reference);
                                        if let Some(placement) = placement {
                                            self.checkpoint();
                                            match document::decode_reference(&reference) {
                                                Ok(decoded) => {
                                                    if let Err(error) = self
                                                        .add_generated_image_layer(
                                                            reference.clone(),
                                                            decoded.width(),
                                                            decoded.height(),
                                                            placement,
                                                        )
                                                    {
                                                        self.message(
                                                            format!(
                                                                "Could not place generated image: {error}"
                                                            ),
                                                            true,
                                                        );
                                                    }
                                                }
                                                Err(error) => self.message(
                                                    format!(
                                                        "Could not read generated image dimensions: {error}"
                                                    ),
                                                    true,
                                                ),
                                            }
                                        } else {
                                            self.checkpoint();
                                        }
                                        let image = worker::render_image(image);
                                        self.reference_cache
                                            .push((reference.clone(), image.clone()));
                                        self.doc.references.push(reference.clone());
                                        self.references.push(image);
                                        self.doc.reference = Some(self.doc.references.len() - 1);
                                        self.changed();
                                        self.sync_ref_inputs(window, cx);
                                        if placement.is_some() {
                                            self.generation_region = None;
                                            self.region_drag = None;
                                        }
                                        if self.page == Page::Image
                                            && self.opened_image.as_ref().is_some_and(
                                                |(r, _, _, _)| {
                                                    self.generation_view_id.as_ref()
                                                        == Some(&crate::library::id(r))
                                                },
                                            )
                                        {
                                            self.open_image(reference, cx);
                                        }
                                        self.message("Image ready · original and new version saved to library", false);
                                    } else {
                                        self.message("Image saved to the shared library; this project was not changed",false);
                                    }
                                    if let Some(item) = self.queue.get_mut(id) {
                                        item.state = JobState::Done;
                                        item.result_id = Some(result_id);
                                    }
                                }
                                Err(e) => {
                                    let message = format!("Could not create image: {e:#}");
                                    self.generation_error = Some(message.clone());
                                    if let Some(item) = self.queue.get_mut(id) {
                                        item.state = JobState::Failed(message.clone());
                                    }
                                    self.message(message, true);
                                }
                            }
                        }
                        self.pump_queue();
                    }
                }
                Event::ServerStatus(status) => self.server_status = Some(status),
                Event::GenerationProgress(id, message) => {
                    if id == self.job_id
                        && self
                            .generation
                            .as_ref()
                            .is_some_and(|c| !c.load(Ordering::Relaxed))
                    {
                        if let Some(item) = self.queue.get_mut(id) {
                            item.state = JobState::Running {
                                progress: message.clone(),
                            };
                        }
                        self.message(message, false);
                    }
                }
                Event::Workflow(result) => match result {
                    Ok(workflow) => {
                        self.settings.workflow = workflow;
                        self.save_settings(cx);
                        self.message("API workflow loaded", false);
                    }
                    Err(e) => self.message(format!("Workflow: {e}"), true),
                },
            }
        }
        if !self.render_pending
            && self.rendered_revision != self.visual_revision
            && self
                .workers
                .render
                .send((
                    self.visual_revision,
                    self.doc.clone(),
                    self.view,
                    self.tracing,
                ))
                .is_ok()
        {
            self.render_pending = true;
        }
        if !self.active_stroke
            && !self.save_pending
            && self.autosaved_revision != self.revision
            && self.last_edit.elapsed() > Duration::from_millis(800)
            && Instant::now() >= self.autosave_retry_after
        {
            self.queue_save(self.data_dir.join("recovery.atelier"), true);
        }
        if self.timer_running {
            let elapsed = self.timer_tick.elapsed();
            self.timer_tick = Instant::now();
            let before = self.timer_remaining.as_secs();
            self.timer_remaining = self.timer_remaining.saturating_sub(elapsed);
            if before != self.timer_remaining.as_secs() {
                refresh = true;
            }
            if self.timer_remaining.is_zero() {
                self.timer_running = false;
                self.message("Study complete · take a moment to compare", false);
                refresh = true;
            }
        }
        if refresh {
            cx.notify();
        }
    }
    pub fn timer_toggle(&mut self) {
        if self.timer_remaining.is_zero() {
            self.timer_remaining = Duration::from_secs(self.timer_seconds as u64);
        }
        self.timer_running = !self.timer_running;
        self.timer_tick = Instant::now();
    }
    pub fn key_down(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        // Canvas focus scopes shortcuts; text fields keep normal editing behavior.
        if self.closing || self.switching || !self.focus.is_focused(window) {
            return;
        }
        let key = event.keystroke.key.as_str();
        let modifiers = event.keystroke.modifiers;
        if self.page == Page::Image {
            if modifiers.platform || modifiers.control {
                if key != "z" {
                    return;
                }
                self.undo_mask(modifiers.shift);
            } else {
                match key {
                    "b" => {
                        self.finish_mask();
                        self.mask_tool = Tool::Pencil;
                    }
                    "e" => {
                        self.finish_mask();
                        self.mask_tool = Tool::Eraser;
                    }
                    "h" | "escape" => {
                        self.finish_mask();
                        self.mask_tool = Tool::Hand;
                    }
                    "0" => self.image_fit(),
                    "1" => {
                        self.image_auto_fit = false;
                        self.image_zoom = 1.;
                        self.image_pan = DocPoint { x: 0., y: 0. };
                    }
                    "[" => self.mask_size = (self.mask_size * 0.8).clamp(1., 512.),
                    "]" => self.mask_size = (self.mask_size * 1.25).clamp(1., 512.),
                    _ => return,
                }
            }
            cx.stop_propagation();
            cx.notify();
            return;
        }
        if modifiers.platform || modifiers.control {
            match key {
                "z" => {
                    if modifiers.shift {
                        self.redo(window, cx)
                    } else {
                        self.undo(window, cx)
                    }
                }
                _ => return,
            }
        } else {
            match key {
                "b" => {
                    self.finish_stroke();
                    self.tool = Tool::Pencil
                }
                "e" => {
                    self.finish_stroke();
                    self.tool = Tool::Eraser
                }
                "h" => {
                    self.finish_stroke();
                    self.tool = Tool::Hand
                }
                "[" => self.brush_size(0.8),
                "]" => self.brush_size(1.25),
                "0" => {
                    self.view = Viewport::default();
                    self.view_changed();
                }
                "r" => {
                    self.view.turns = (self.view.turns + 1) % 4;
                    self.view_changed();
                }
                "f" => {
                    self.view.mirror = !self.view.mirror;
                    self.view_changed();
                }
                "escape" => {
                    self.finish_stroke();
                    self.panel = Panel::None;
                }
                _ => return,
            }
        }
        cx.stop_propagation();
        cx.notify();
    }
}
impl Render for DrawingApp {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.render_workspace(window, cx)
    }
}
