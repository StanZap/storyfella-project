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
    Projects,
    Draw,
    Library,
    Image,
    Artboards,
}
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Panel {
    References,
    None,
    Canvas,
    Settings,
    Help,
    Brushes,
    Queue,
}

fn aspect_index(width: u32, height: u32) -> usize {
    if width == height {
        0
    } else if width * 2 == height * 3 {
        1
    } else if height * 2 == width * 3 {
        2
    } else {
        3
    }
}

fn size_index(short_edge: u32) -> usize {
    [768, 1024, 1280]
        .into_iter()
        .position(|size| size == short_edge)
        .unwrap_or(3)
}

fn steps_index(steps: Option<u32>) -> usize {
    match steps {
        None => 0,
        Some(value) => [16, 25, 40]
            .into_iter()
            .position(|preset| preset == value)
            .map(|index| index + 1)
            .unwrap_or(4),
    }
}

fn aspect_choices(width: u32, height: u32) -> Vec<String> {
    let mut choices = vec![
        "Square · 1:1".into(),
        "Landscape · 3:2".into(),
        "Portrait · 2:3".into(),
    ];
    if aspect_index(width, height) == 3 {
        choices.push(format!("Custom · {width}×{height}"));
    }
    choices
}

fn size_choices(short_edge: u32) -> Vec<String> {
    let mut choices = vec![
        "Size · 768 px".into(),
        "Size · 1024 px".into(),
        "Size · 1280 px".into(),
    ];
    if size_index(short_edge) == 3 {
        choices.push(format!("Size · {short_edge} px"));
    }
    choices
}

fn steps_choices(steps: Option<u32>) -> Vec<String> {
    let mut choices = vec![
        "Steps · Auto".into(),
        "Steps · 16".into(),
        "Steps · 25".into(),
        "Steps · 40".into(),
    ];
    if steps_index(steps) == 4 {
        choices.push(format!("Steps · {}", steps.unwrap_or_default()));
    }
    choices
}

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

pub struct ProjectCard {
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
    pub project_tabs: Vec<crate::project_tabs::ProjectTab>,
    pub next_project_epoch: u64,
    pub library: Vec<(Arc<document::Reference>, Arc<RenderImage>)>,
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
    pub show_edit_composer: bool,
    pub show_artboards_composer: bool,
    pub show_prompt_details: bool,
    pub projects: Vec<ProjectCard>,
    pub projects_loading: bool,
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
    pub scaling_layer: bool,
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
    pub show_drawing_composer: bool,
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
    pub artboard_aspect_select: Entity<SelectState<Vec<String>>>,
    pub artboard_size_select: Entity<SelectState<Vec<String>>>,
    pub artboard_steps_select: Entity<SelectState<Vec<String>>>,
    /// The searchable picker that adds a LoRA adapter to the local settings.
    pub lora_select: Entity<SelectState<SearchableVec<String>>>,
    /// Per-adapter weight inputs, keyed by the registered adapter name.
    pub lora_weights: std::collections::BTreeMap<String, Entity<InputState>>,
    /// The last probed state of the machine's shared local image server.
    pub server_status: Option<ServerStatus>,
    pub provider_readiness: String,
    pub settings_save: Option<Settings>,
    pub settings_save_error: Option<String>,
    pub generation: Option<Arc<AtomicBool>>,
    pub generation_error: Option<String>,
    /// The queued and recently completed image requests, oldest first.
    pub queue: Queue,
    /// The canvas region (document coordinates) selected for "Generate from
    /// drawing"; `None` means the full canvas.
    pub generation_region: Option<(DocPoint, DocPoint)>,
    pub selecting_generation_region: bool,
    /// The drag anchor while a region is being selected on the canvas.
    pub(crate) region_drag: Option<DocPoint>,
    /// The prompt used by the floating drawing composer.
    pub selection_mode: crate::workflow::SelectionMode,
    pub job_id: u64,
    pub job_epoch: u64,
    pub timer_seconds: u32,
    pub timer_remaining: Duration,
    pub timer_running: bool,
    timer_tick: Instant,
    pub exercise: usize,
    pub title: Entity<InputState>,
    pub layer_name: Entity<InputState>,
    pub layer_dx: Entity<InputState>,
    pub layer_dy: Entity<InputState>,
    pub layer_scale: Entity<InputState>,
    pub layer_opacity: Entity<InputState>,
    pub width: Entity<InputState>,
    pub height: Entity<InputState>,
    pub prompt: Entity<TextareaState>,
    prompt_target: String,
    pub search: Entity<InputState>,
    pub endpoint: Entity<InputState>,
    pub model: Entity<InputState>,
    pub edit_model: Entity<InputState>,
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
        let (doc, mut status, mut error) = match recovered {
            Ok(doc) => (doc, "Recovered your last session".into(), false),
            Err(e) if data_dir.join("recovery.atelier").exists() => (
                Document::default(),
                format!("Recovery could not be opened: {e}"),
                true,
            ),
            Err(_) => (Document::default(), String::new(), false),
        };
        let (project_tabs, doc, initial_epoch, initial_path, initial_dirty) =
            match crate::project_tabs::recover(&data_dir) {
                Ok((tabs, active)) => (tabs, active.doc, active.epoch, active.path, active.dirty),
                Err(recovery_error) => {
                    let snapshots = data_dir.join("open-projects");
                    if snapshots.join("session.json").exists() {
                        status = format!("Open-project recovery failed: {recovery_error}. Snapshots remain in {}",snapshots.display());
                        error = true;
                    }
                    let epoch = std::fs::read_dir(&snapshots)
                        .into_iter()
                        .flatten()
                        .filter_map(|entry| {
                            entry.ok().and_then(|e| {
                                e.path()
                                    .file_stem()
                                    .and_then(|s| s.to_str())
                                    .and_then(|s| s.parse::<u64>().ok())
                            })
                        })
                        .max()
                        .unwrap_or_default()
                        .saturating_add(1);
                    (vec![], doc, epoch, None, false)
                }
            };
        let next_project_epoch = project_tabs
            .iter()
            .map(|tab| tab.epoch)
            .chain([initial_epoch])
            .max()
            .unwrap_or(1);
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
        let artboard_aspect_select = cx.new(|cx| {
            SelectState::new(
                aspect_choices(settings.width, settings.height),
                Some(IndexPath::new(aspect_index(
                    settings.width,
                    settings.height,
                ))),
                window,
                cx,
            )
        });
        let artboard_size_select = cx.new(|cx| {
            SelectState::new(
                size_choices(settings.width.min(settings.height)),
                Some(IndexPath::new(size_index(
                    settings.width.min(settings.height),
                ))),
                window,
                cx,
            )
        });
        let artboard_steps_select = cx.new(|cx| {
            SelectState::new(
                steps_choices(settings.steps),
                Some(IndexPath::new(steps_index(settings.steps))),
                window,
                cx,
            )
        });
        let title = input(doc.title.clone(), window, cx);
        let layer_name = input(doc.layers[doc.active_layer].name.clone(), window, cx);
        let layer_dx = input("0".into(), window, cx);
        let layer_dy = input("0".into(), window, cx);
        let layer_scale = input("100".into(), window, cx);
        let layer_opacity = input(
            format!("{}", doc.layers[doc.active_layer].opacity * 100.),
            window,
            cx,
        );
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
            project_tabs,
            next_project_epoch,
            library: vec![],
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
            show_edit_composer: false,
            show_artboards_composer: true,
            show_prompt_details: false,
            show_drawing_composer: false,
            projects: vec![],
            projects_loading: false,
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
            scaling_layer: false,
            hover: None,
            bounds: Rc::new(Cell::new(Bounds::default())),
            tiles: std::collections::HashMap::new(),
            render_backend: "Starting",
            render_ms: 0.,
            render_segments: 0,
            references,
            page: Page::Projects,
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
            path: initial_path,
            data_dir,
            dirty: initial_dirty,
            switching: false,
            closing: false,
            revision: 1,
            epoch: initial_epoch,
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
            artboard_aspect_select,
            artboard_size_select,
            artboard_steps_select,
            lora_select,
            lora_weights,
            server_status: None,
            provider_readiness: "Checking provider…".into(),
            settings_save: None,
            settings_save_error: None,
            generation: None,
            generation_error: None,
            queue: Queue::default(),
            generation_region: None,
            selecting_generation_region: false,
            region_drag: None,
            selection_mode: crate::workflow::SelectionMode::Blend,
            job_id: 0,
            job_epoch: 0,
            timer_seconds: 300,
            timer_remaining: Duration::from_secs(300),
            timer_running: false,
            timer_tick: Instant::now(),
            exercise: 0,
            title,
            layer_name,
            layer_dx,
            layer_dy,
            layer_scale,
            layer_opacity,
            width,
            height,
            prompt,
            prompt_target,
            search,
            endpoint,
            model,
            edit_model,
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
        app.refresh_projects();
        app.reload_clipforge(window, cx);
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
            &app.artboard_aspect_select,
            window,
            |this, _, event: &SelectEvent<Vec<String>>, window, cx| {
                if let SelectEvent::Confirm(Some(value)) = event {
                    if value.starts_with("Custom") {
                        return;
                    }
                    let aspect = if value.starts_with("Landscape") {
                        1
                    } else if value.starts_with("Portrait") {
                        2
                    } else {
                        0
                    };
                    this.settings_from_inputs(cx);
                    let size = this.settings.width.min(this.settings.height);
                    this.set_artboard_dimensions(aspect, size, window, cx);
                }
            },
        ));
        app.subscriptions.push(cx.subscribe_in(
            &app.artboard_size_select,
            window,
            |this, _, event: &SelectEvent<Vec<String>>, window, cx| {
                if let SelectEvent::Confirm(Some(value)) = event {
                    let Some(size) = value
                        .strip_prefix("Size · ")
                        .and_then(|text| text.strip_suffix(" px"))
                        .and_then(|text| text.parse::<u32>().ok())
                    else {
                        return;
                    };
                    this.settings_from_inputs(cx);
                    this.set_artboard_dimensions(
                        if this.settings.width > this.settings.height {
                            1
                        } else if this.settings.height > this.settings.width {
                            2
                        } else {
                            0
                        },
                        size,
                        window,
                        cx,
                    );
                }
            },
        ));
        app.subscriptions.push(cx.subscribe_in(
            &app.artboard_steps_select,
            window,
            |this, _, event: &SelectEvent<Vec<String>>, window, cx| {
                if let SelectEvent::Confirm(Some(value)) = event {
                    let steps = value
                        .strip_prefix("Steps · ")
                        .and_then(|text| text.parse::<u32>().ok());
                    this.steps.update(cx, |state, cx| {
                        state.set_value(
                            steps.map(|count| count.to_string()).unwrap_or_default(),
                            window,
                            cx,
                        )
                    });
                    this.save_settings(cx);
                    cx.notify();
                }
            },
        ));
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
            this.persist_open_projects();
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
            self.persist_open_projects();
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
    pub(crate) fn capture_prompt(&mut self, cx: &mut Context<Self>) {
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
            if let Some(id) = self
                .prompt_target
                .strip_prefix("canvas:")
                .and_then(|s| s.split(':').next())
                .and_then(|id| id.parse().ok())
            {
                self.doc
                    .artboard_prompt_targets
                    .insert(id, self.prompt_target.clone());
            }
            self.revision += 1;
            self.dirty = true;
            self.last_edit = Instant::now();
            cx.notify();
        }
    }
    pub fn restore_prompt(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let target = if self.page == Page::Draw {
            match self.region_bounds() {
                Some((x, y, w, h)) => format!(
                    "canvas:{}:{:?}:{x}:{y}:{w}:{h}",
                    self.doc.canvas_id, self.selection_mode
                ),
                None => format!("canvas:{}:full", self.doc.canvas_id),
            }
        } else if self.editing_image {
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
    pub fn reset_recovery_clock(&mut self) {
        self.autosaved_revision = 0;
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
        self.scaling_layer = false;
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
    pub(crate) fn sync_inputs(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.sync_layer_controls(window, cx);
        self.prompt_target = self
            .doc
            .active_prompt_target
            .clone()
            .unwrap_or_else(|| "new".into());
        self.editing_image =
            self.prompt_target != "new" && !self.prompt_target.starts_with("canvas:");
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
            || self
                .doc
                .layers
                .iter()
                .chain(self.doc.canvases.iter().flat_map(|c| &c.layers))
                .any(|l| {
                    l.image
                        .as_ref()
                        .is_some_and(|i| crate::library::id(&i.reference) == target)
                })
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
        let mut artwork_changed = false;
        for layer in self
            .doc
            .layers
            .iter_mut()
            .chain(self.doc.canvases.iter_mut().flat_map(|c| &mut c.layers))
        {
            if let Some(image) = &mut layer.image {
                if crate::library::id(&image.reference) == target {
                    let reference = Arc::make_mut(&mut image.reference);
                    reference.name = name.clone();
                    reference.tags = tags.clone();
                    artwork_changed = true;
                }
            }
        }
        if artwork_changed {
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
            .opened_image
            .as_ref()
            .filter(|(reference, _, _, _)| crate::library::id(reference) == target)
            .map(|(reference, _, _, _)| reference.clone())
            .or_else(|| {
                self.all_references()
                    .into_iter()
                    .find(|reference| crate::library::id(reference) == target)
            })
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

    pub fn remove_reference(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(i) = self.doc.reference {
            self.checkpoint();
            let id = crate::library::id(&self.doc.references[i]);
            self.doc.artboard_references.retain(|r| *r != id);
            for canvas in &mut self.doc.canvases {
                canvas.references.retain(|r| *r != id);
            }
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
                "Untitled project".into()
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
        if self.project_tabs.len() >= 31 {
            self.message("Maximum 32 open projects", true);
            return;
        }
        self.capture_prompt(cx);
        if !self.prepare_project_switch() {
            return;
        }
        self.finish_stroke();
        let filename = format!(
            "project-{}-{}.atelier",
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
                previous_path: self.path.clone(),
                next: None,
            })
            .is_ok();
        self.message(
            if self.switching {
                "Creating project…"
            } else {
                "Storage worker stopped"
            },
            !self.switching,
        );
    }
    pub fn refresh_projects(&mut self) {
        if !self.projects_loading {
            self.projects_loading = true;
            worker::projects(self.data_dir.clone(), self.workers.sender.clone());
        }
    }
    pub fn open_project(&mut self, path: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        let normalized = path.canonicalize().unwrap_or(path.clone());
        let same_path = |candidate: &Option<PathBuf>| {
            candidate
                .as_ref()
                .is_some_and(|p| p.canonicalize().unwrap_or(p.clone()) == normalized)
        };
        if same_path(&self.path) {
            self.activate_project_tab(self.epoch, window, cx);
            return;
        }
        if let Some(epoch) = self
            .project_tabs
            .iter()
            .find(|tab| same_path(&tab.path))
            .map(|tab| tab.epoch)
        {
            self.activate_project_tab(epoch, window, cx);
            return;
        }
        if self.project_tabs.len() >= 31 {
            self.message("Maximum 32 open projects", true);
            return;
        }
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
                previous_path: self.path.clone(),
                next: Some(path),
            })
            .is_ok();
        if !self.switching {
            self.message("Storage worker stopped", true);
        }
    }
    pub(crate) fn prepare_project_switch(&mut self) -> bool {
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
            candidate: false,
            strokes: vec![],
            image: None,
        });
        self.doc.active_layer = self.doc.layers.len() - 1;
        self.changed();
    }

    pub fn sync_layer_controls(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        for (input, value) in [
            (&self.layer_dx, "0".into()),
            (&self.layer_dy, "0".into()),
            (&self.layer_scale, "100".into()),
            (
                &self.layer_opacity,
                format!("{}", self.doc.layers[self.doc.active_layer].opacity * 100.),
            ),
        ] {
            input.update(cx, |s, cx| s.set_value(value, window, cx));
        }
    }

    pub fn apply_layer_controls(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let values: Option<Vec<f32>> = [
            &self.layer_dx,
            &self.layer_dy,
            &self.layer_scale,
            &self.layer_opacity,
        ]
        .iter()
        .map(|input| input.read(cx).value().parse().ok())
        .collect();
        let Some(values) = values.filter(|v| {
            v.iter().all(|x| x.is_finite()) && v[2] > 0. && (0. ..=100.).contains(&v[3])
        }) else {
            self.message(
                "Enter finite movement, positive scale and opacity from 0 to 100",
                true,
            );
            return;
        };
        let previous = self.doc.clone();
        match self
            .doc
            .reposition_image_layer(values[0], values[1], values[2] / 100.)
        {
            Ok(()) => {
                self.history.checkpoint(&previous);
                self.doc.layers[self.doc.active_layer].opacity = values[3] / 100.;
                self.changed();
                self.sync_layer_controls(window, cx);
            }
            Err(error) => self.message(error.to_string(), true),
        }
        cx.notify();
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
    fn clamped_doc_point(&self, p: gpui_kit::Point<Pixels>) -> DocPoint {
        let bounds = self.paper_bounds();
        let scale = self.view.scale(
            (
                self.bounds.get().size.width.into(),
                self.bounds.get().size.height.into(),
            ),
            (self.doc.width, self.doc.height),
        );
        let (width, height) = self.view.dimensions(self.doc.width, self.doc.height);
        let view_point = DocPoint {
            x: (f32::from(p.x - bounds.origin.x) / scale).clamp(0., width),
            y: (f32::from(p.y - bounds.origin.y) / scale).clamp(0., height),
        };
        let document_point =
            self.view
                .to_document(view_point, self.doc.width as f32, self.doc.height as f32);
        DocPoint {
            x: document_point.x.clamp(0., self.doc.width as f32),
            y: document_point.y.clamp(0., self.doc.height as f32),
        }
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
        if self.selecting_generation_region && event.button == MouseButton::Left {
            if let Some(point) = self.doc_point(event.position) {
                self.region_drag = Some(point);
                self.generation_region = Some((point, point));
                cx.notify();
            }
            return;
        }
        if self.tool == Tool::Hand && self.moving_image && event.button == MouseButton::Left {
            if let Some(point) = self.doc_point(event.position) {
                self.history.checkpoint(&self.doc);
                self.scaling_layer = self.doc.layer_bounds().is_some_and(|(_, _, x, y)| {
                    (point.x - x).hypot(point.y - y)
                        * self.view.scale(
                            (
                                self.bounds.get().size.width.into(),
                                self.bounds.get().size.height.into(),
                            ),
                            (self.doc.width, self.doc.height),
                        )
                        < 16.
                });
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
            if let Some(anchor) = self.region_drag {
                self.generation_region = Some((anchor, self.clamped_doc_point(event.position)));
                cx.notify();
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
                    let (dx, dy, scale) = if self.scaling_layer {
                        let (x, y, _, _) = self.doc.layer_bounds().unwrap_or((0., 0., 1., 1.));
                        let factor = ((point.x - x).hypot(point.y - y)
                            / (last.x - x).hypot(last.y - y).max(1.))
                        .clamp(0.5, 2.);
                        (
                            (x - self.doc.width as f32 / 2.) * (1. - factor),
                            (y - self.doc.height as f32 / 2.) * (1. - factor),
                            factor,
                        )
                    } else {
                        (point.x - last.x, point.y - last.y, 1.)
                    };
                    match self.doc.reposition_image_layer(dx, dy, scale) {
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

    fn set_artboard_dimensions(
        &mut self,
        aspect: usize,
        short_edge: u32,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let short_edge = short_edge.min(if aspect == 0 { 2048 } else { 1364 });
        let (width, height) = match aspect {
            1 => (short_edge * 3 / 2, short_edge),
            2 => (short_edge, short_edge * 3 / 2),
            _ => (short_edge, short_edge),
        };
        self.generation_width.update(cx, |state, cx| {
            state.set_value(width.to_string(), window, cx)
        });
        self.generation_height.update(cx, |state, cx| {
            state.set_value(height.to_string(), window, cx)
        });
        self.settings.width = width;
        self.settings.height = height;
        self.save_settings(cx);
        self.sync_artboard_selects(window, cx);
        cx.notify();
    }

    pub fn sync_artboard_selects(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.settings_from_inputs(cx);
        self.artboard_aspect_select.update(cx, |state, cx| {
            state.set_items(
                aspect_choices(self.settings.width, self.settings.height),
                window,
                cx,
            );
            state.set_selected_index(
                Some(IndexPath::new(aspect_index(
                    self.settings.width,
                    self.settings.height,
                ))),
                window,
                cx,
            );
        });
        self.artboard_size_select.update(cx, |state, cx| {
            let short_edge = self.settings.width.min(self.settings.height);
            state.set_items(size_choices(short_edge), window, cx);
            state.set_selected_index(Some(IndexPath::new(size_index(short_edge))), window, cx);
        });
        self.artboard_steps_select.update(cx, |state, cx| {
            state.set_items(steps_choices(self.settings.steps), window, cx);
            state.set_selected_index(
                Some(IndexPath::new(steps_index(self.settings.steps))),
                window,
                cx,
            );
        });
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
        self.provider_readiness = match providers::validate_inputs(&self.settings, false, 0, false)
        {
            Ok(()) => format!(
                "{} configured · {} · {}",
                self.settings.provider.name(),
                if self.settings.provider.supports_edit() {
                    "create and edit"
                } else {
                    "create only"
                },
                if self.settings.provider.native_architecture().is_some() {
                    "one source · starts on demand"
                } else {
                    "account readiness checked on submission"
                }
            ),
            Err(error) => format!("Setup needed: {error}"),
        };
        if self.settings.provider.native_architecture().is_none() {
            self.server_status = None;
            return;
        }
        let _ = self.workers.probe.send(self.settings.clone());
    }

    pub fn open_clipforge(&mut self) {
        if self.workers.storage.send(Storage::OpenClipforge).is_err() {
            self.message("Storage worker stopped", true);
        }
    }

    pub fn reload_clipforge(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Ok(registry) = clipforge_inference::config::user_or_built_in() {
            if let Some(endpoint) = providers::shared_endpoint(self.settings.provider, &registry) {
                self.settings.endpoint = endpoint.clone();
                self.endpoint
                    .update(cx, |s, cx| s.set_value(endpoint, window, cx));
            }
        }
        self.refresh_lora_select(window, cx);
        self.refresh_server_status();
        self.generation_error = None;
        cx.notify();
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
        self.settings_save_error = None;
        if self
            .workers
            .storage
            .send(Storage::ApplySettings(
                self.settings.clone(),
                self.data_dir.join("settings.json"),
            ))
            .is_err()
        {
            self.settings_save_error =
                Some("Storage worker stopped. Try restarting Atelier.".into());
            self.message("Storage worker stopped", true);
        } else {
            self.settings_save = Some(self.settings.clone());
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
        self.sync_artboard_selects(window, cx);
        self.generation_error = None;
        self.capture_prompt(cx);
        cx.notify();
    }

    pub fn open_canvas(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.capture_prompt(cx);
        self.page = Page::Draw;
        self.show_drawing_composer = false;
        self.generation_region = None;
        self.selecting_generation_region = false;
        self.restore_drawing_context();
        self.restore_prompt(window, cx);
        window.focus(&self.focus, cx);
        cx.notify();
    }

    pub fn restore_drawing_context(&mut self) {
        let Some(target) = self.doc.artboard_prompt_targets.get(&self.doc.canvas_id) else {
            return;
        };
        let parts: Vec<_> = target.split(':').collect();
        if parts.len() == 7 {
            let coords: Option<Vec<u32>> = parts[3..].iter().map(|s| s.parse().ok()).collect();
            if let Some(rect) = coords.filter(|r| {
                r[0].saturating_add(r[2]) <= self.doc.width
                    && r[1].saturating_add(r[3]) <= self.doc.height
            }) {
                self.selection_mode = if parts[2] == "Separate" {
                    crate::workflow::SelectionMode::Separate
                } else {
                    crate::workflow::SelectionMode::Blend
                };
                self.generation_region = Some((
                    DocPoint {
                        x: rect[0] as f32,
                        y: rect[1] as f32,
                    },
                    DocPoint {
                        x: (rect[0] + rect[2]) as f32,
                        y: (rect[1] + rect[3]) as f32,
                    },
                ));
                self.selecting_generation_region = true;
            }
        }
    }

    pub fn create_canvas_artboard(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.capture_prompt(cx);
        self.finish_stroke();
        let previous = self.doc.clone();
        if let Err(error) = self.doc.create_canvas() {
            self.message(format!("Canvas: {error}"), true);
            return;
        }
        self.history.checkpoint(&previous);
        self.doc.active_prompt_target = Some("new".into());
        self.changed();
        self.view = Viewport::default();
        self.panel = Panel::None;
        self.sync_inputs(window, cx);
        self.page = Page::Artboards;
        cx.notify();
    }

    pub fn open_canvas_artboard(
        &mut self,
        canvas_id: u32,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.capture_prompt(cx);
        if canvas_id != self.doc.canvas_id {
            let Some(index) = self
                .doc
                .canvases
                .iter()
                .position(|canvas| canvas.id == canvas_id)
            else {
                return;
            };
            self.checkpoint();
            let current = self.doc.current_canvas();
            let selected = std::mem::replace(&mut self.doc.canvases[index], current);
            self.doc.set_current_canvas(selected);
            self.doc.reference = self.doc.artboard_references.first().and_then(|id| {
                self.doc
                    .references
                    .iter()
                    .position(|r| crate::library::id(r) == *id)
            });
            self.changed();
            self.view = Viewport::default();
            self.panel = Panel::None;
            self.sync_inputs(window, cx);
        }
        self.open_canvas(window, cx);
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
            self.input_references().first().cloned()
        };
        let editing = source.is_some();
        let references: Vec<_> = {
            self.input_references()
                .iter()
                .filter(|r| source.as_ref().is_none_or(|source| r.image != source.image))
                .cloned()
                .collect()
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
        if let Err(error) =
            providers::validate_inputs(&self.settings, editing, references.len(), mask.is_some())
        {
            self.generation_error = Some(error.to_string());
            return;
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

        self.queue.enqueue(
            QueuedRequest {
                settings: self.settings.clone(),
                prompt,
                source,
                references,
                mask,
                editing,
                destination: crate::workflow::Destination::NewArtboard,
                blend: None,
            },
            self.epoch,
        );
        self.show_edit_composer = false;
        self.page = Page::Artboards;
        self.editing_image = false;
        self.restore_prompt(window, cx);
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
        let (epoch, request) = {
            let Some(item) = self.queue.get_mut(id) else {
                return;
            };
            item.state = JobState::Running {
                progress: String::new(),
            };
            (item.epoch, item.request.clone())
        };
        let cancel = Arc::new(AtomicBool::new(false));
        self.job_id = id;
        self.job_epoch = epoch;
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
        self.queue.enqueue(request, epoch);
        self.message("Request re-queued", false);
        self.pump_queue();
        cx.notify();
    }

    /// Opens the generated reference associated with a completed queue item.
    pub fn open_queue_result(&mut self, id: u64, cx: &mut Context<Self>) {
        let Some(reference) = self.queue.get(id).and_then(|item| item.result.clone()) else {
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
            .chain(
                self.doc
                    .layers
                    .iter()
                    .chain(self.doc.canvases.iter().flat_map(|c| &c.layers))
                    .filter_map(|l| l.image.as_ref().map(|i| i.reference.clone())),
            )
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
    pub fn use_full_canvas(&mut self, cx: &mut Context<Self>) {
        self.region_drag = None;
        self.generation_region = None;
        self.selecting_generation_region = false;
        cx.notify();
    }

    /// Opens the floating drawing prompt beside the canvas.
    pub fn open_from_drawing(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.show_drawing_composer = !self.show_drawing_composer;
        self.region_drag = None;
        if self.show_drawing_composer {
            self.prompt.update(cx, |state, cx| state.focus(window, cx));
        } else {
            window.focus(&self.focus, cx);
        }
        cx.notify();
    }

    pub fn toggle_artboards_composer(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.show_artboards_composer = !self.show_artboards_composer;
        if self.show_artboards_composer {
            self.prompt.update(cx, |state, cx| state.focus(window, cx));
        } else {
            window.focus(&self.focus, cx);
        }
        cx.notify();
    }

    pub fn open_prompt(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match self.page {
            Page::Draw => self.open_from_drawing(window, cx),
            Page::Artboards => self.toggle_artboards_composer(window, cx),
            Page::Image if self.settings.provider.supports_edit() => {
                self.show_edit_composer = !self.show_edit_composer;
                if self.show_edit_composer {
                    self.edit_opened_image(window, cx);
                } else {
                    window.focus(&self.focus, cx);
                }
            }
            _ => {}
        }
        cx.notify();
    }

    /// Toggles area selection. A drag on the canvas defines the region.
    pub fn toggle_box_select(&mut self, cx: &mut Context<Self>) {
        if self.selecting_generation_region {
            self.use_full_canvas(cx);
            return;
        }
        self.selecting_generation_region = true;
        self.generation_region = None;
        self.show_drawing_composer = true;
        cx.notify();
    }

    /// Called on canvas mouse-up, ending a region drag. A drag too small to be a
    /// real selection is discarded and the area mode follows the result.
    pub fn finish_drawing_region(&mut self, cx: &mut Context<Self>) {
        self.region_drag = None;
        if self.generation_region.is_some() && self.region_bounds().is_none() {
            self.generation_region = None;
        }
        cx.notify();
    }

    /// Capture accepted artwork and a destination before queueing. A later UI
    /// selection, resize or artboard switch cannot redirect this request.
    pub fn generate_from_drawing(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.restore_prompt(window, cx);
        self.capture_prompt(cx);
        self.settings_from_inputs(cx);
        self.finish_stroke();
        let prompt = self.prompt.read(cx).value().trim().to_owned();
        let result = (|| -> anyhow::Result<QueuedRequest> {
            anyhow::ensure!(!prompt.is_empty(), "Describe the image or change you want");
            let region = self.region_bounds();
            anyhow::ensure!(
                !self.selecting_generation_region || region.is_some(),
                "Drag on the artboard to select an area"
            );
            let separate =
                region.is_some() && self.selection_mode == crate::workflow::SelectionMode::Separate;
            let source =
                Arc::new(
                    self.doc
                        .generation_source(if separate { region } else { None })?,
                );
            let references = self.input_references();
            let mut settings = self.settings.clone();
            if !separate {
                settings.width = self.doc.width;
                settings.height = self.doc.height;
            }
            crate::providers::validate_inputs(&settings, true, references.len(), false)?;
            let rect = region.unwrap_or((0, 0, self.doc.width, self.doc.height));
            // Keep the accepted source embedded for durable parent/version resolution.
            let extra = if self.doc.references.iter().any(|r| r.image == source.image) {
                0
            } else {
                source.image.len()
            };
            anyhow::ensure!(
                self.doc.references.len() < 128
                    && self.doc.embedded_image_bytes() + extra <= 64 * 1024 * 1024,
                "Project reference limit reached"
            );
            if extra > 0 {
                self.doc.references.push(source.clone());
                let preview = worker::render_image(
                    document::thumbnail(document::decode_reference(&source)?, 800).to_rgba8(),
                );
                self.references.push(preview.clone());
                self.reference_cache.push((source.clone(), preview));
            }
            Ok(QueuedRequest {
                settings,
                prompt: prompt.clone(),
                source: Some(source),
                references,
                mask: None,
                editing: true,
                destination: if separate {
                    crate::workflow::Destination::NewArtboard
                } else {
                    crate::workflow::Destination::Candidate {
                        artboard: self.doc.canvas_id,
                        rect,
                    }
                },
                blend: if !separate {
                    region.map(|r| ((self.doc.width, self.doc.height), r))
                } else {
                    None
                },
            })
        })();
        match result {
            Ok(request) => {
                self.changed();
                self.queue.enqueue(request, self.epoch);
                self.generation_error = None;
                self.pump_queue();
                self.message(
                    "Queued · original artwork preserved; edits arrive as candidates",
                    false,
                );
            }
            Err(error) => {
                self.generation_error = Some(error.to_string());
                self.message(error.to_string(), true);
            }
        }
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
        if automatic && path == self.data_dir.join("recovery.atelier") {
            self.persist_open_projects();
        }
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
        let import_into_project = self.panel == Panel::References;
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
                        let _ = entity.update_in(cx, |this, window, cx| {
                            if epoch != this.epoch && import_into_project {
                                return;
                            }
                            match result {
                                Ok(Ok(Some(paths))) if !paths.is_empty() => match picker {
                                    Picker::Open => {
                                        this.open_project(paths[0].clone(), window, cx);
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
        if self.page != Page::Projects
            && self.page != Page::Artboards
            && self.opening_image.is_none()
            && self.region_drag.is_none()
        {
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
                Event::WorkspaceSaved(result) => if let Err(error) = result {
                    self.closing = false;
                    self.message(format!("Open-project recovery save failed: {error}"),true);
                },
                Event::ClipforgeOpened(result) => match result {
                    Ok(()) => self.message("Clipforge opened · manage providers there, then return and Refresh readiness",false),
                    Err(error) => self.message(format!("Clipforge: {error}"),true),
                },
                Event::Settings(result) => match result {
                    Ok(()) => self.message("Settings saved", false),
                    Err(e) => self.message(format!("Settings: {e}"), true),
                },
                Event::SettingsApplied(result) => {
                    let pending = self.settings_save.take();
                    match result {
                        Ok(()) => {
                            if let Some(saved) = pending {
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
                Event::Projects(result) => {
                    self.projects_loading = false;
                    match result {
                        Ok(projects) => {
                            self.projects = projects
                                .into_iter()
                                .map(|project| ProjectCard {
                                    title: project.title,
                                    path: project.path,
                                    image: worker::render_image(project.image),
                                    note: project.note,
                                })
                                .collect()
                        }
                        Err(e) => self.message(format!("Project list: {e}"), true),
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
                                    self.refresh_projects();
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
                            let keep_previous = self.path.is_some() || !self.doc.references.is_empty()
                                || self.doc.layers.iter().any(|l|l.image.is_some() || !l.strokes.is_empty())
                                || self.doc.prompt_drafts.values().any(|draft| !draft.trim().is_empty());
                            let previous = self.park_project();
                            if keep_previous { self.project_tabs.push(previous); }
                            self.doc = doc;
                            self.next_project_epoch += 1;
                            self.epoch = self.next_project_epoch;
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
                            self.page = Page::Artboards;
                            self.show_artboards_composer = true;
                            self.show_edit_composer = false;
                            self.sync_inputs(window, cx);
                            self.editing_image = false;
                            self.restore_prompt(window, cx);
                            self.sync_artboard_selects(window, cx);
                            self.message("Project ready", false);
                            self.persist_open_projects();
                            if self.path.is_none() {
                                let filename = format!(
                                    "project-{}-{}.atelier",
                                    std::time::SystemTime::now()
                                        .duration_since(std::time::UNIX_EPOCH)
                                        .unwrap_or_default()
                                        .as_millis(),
                                    self.epoch
                                );
                                let path = self.data_dir.join(filename);
                                self.path = Some(path.clone());
                                self.queue_save(path, false);
                            }
                        }
                        Err(e) => self.message(format!("Could not open project: {e}"), true),
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
                            if self.panel == Panel::References {
                                if let Some(reference) = self.doc.references.last().cloned() {
                                    self.choose_reference(reference, window, cx);
                                }
                            }
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
                                    let request = self.queue.get(id).map(|item| item.request.clone());
                                    let reference = Arc::new(reference);
                                    // Keep every result recoverable in the queue, including attachment failures.
                                    if let Some(item) = self.queue.get_mut(id) {
                                        item.result = Some(reference.clone());
                                    }
                                    if self.job_epoch == self.epoch {
                                        if let Some(request) = request {
                                            let attached = request.blend.map(|(size, rect)|
                                                crate::workflow::crop_blend(&reference, size, rect).map(Arc::new))
                                                .unwrap_or_else(|| Ok(reference.clone()));
                                            match attached.and_then(|r| {
                                                self.checkpoint();
                                                self.doc.attach_result(r, request.destination)
                                            }) {
                                                Ok(()) => {
                                                    self.changed();
                                                    self.sync_inputs(window, cx);
                                                    if matches!(request.destination, crate::workflow::Destination::Candidate { .. }) {
                                                        self.message("Candidate ready · accept, hide or discard in Layers", false);
                                                        self.dock = true;
                                                    } else {
                                                        self.message("New artboard ready", false);
                                                    }
                                                }
                                                Err(error) => self.message(format!("Image saved; could not attach: {error}"), true),
                                            }
                                        }
                                    } else if let Some(tab) = self.project_tabs.iter_mut().find(|tab|tab.epoch == self.job_epoch) {
                                        if let Some(request) = request {
                                            let attached = request.blend.map(|(size,rect)| crate::workflow::crop_blend(&reference,size,rect).map(Arc::new))
                                                .unwrap_or_else(||Ok(reference.clone()));
                                            let previous = tab.doc.clone();
                                            match attached.and_then(|r|tab.doc.attach_result(r,request.destination)) {
                                                Ok(()) => { tab.history.checkpoint(&previous); tab.revision += 1; tab.dirty = true; }
                                                Err(error) => self.message(format!("Image saved in queue; attachment failed: {error}"),true),
                                            }
                                            self.persist_open_projects();
                                        }
                                    }
                                    let _ = image;
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
                match key {
                    "z" => self.undo_mask(modifiers.shift),
                    _ => return,
                }
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
                    self.moving_image = false;
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
                    self.show_drawing_composer = false;
                    self.generation_region = None;
                    self.selecting_generation_region = false;
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
