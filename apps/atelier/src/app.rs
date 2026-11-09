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
        input::{InputEvent, InputState, TextareaState},
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
    StoryContext,
    Catalog,
    References,
    None,
    Canvas,
    Export,
    Settings,
    Help,
    Brushes,
    Activity,
}
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum DockView {
    Layers,
    References,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum LayerInput {
    X,
    Y,
    Width,
    Height,
    Opacity,
    GroupOpacity,
    Name,
    GroupName,
    Rotation,
}

struct LayerInputDraft {
    field: LayerInput,
    epoch: u64,
    artboard: u32,
    selection: Vec<usize>,
    original: String,
}

struct ArtworkImportDestination {
    epoch: u64,
    artboard: u32,
    rect: (u32, u32, u32, u32),
}

#[derive(Clone)]
pub(crate) struct ImageCropSession {
    pub layer: usize,
    pub artboard: u32,
    pub image: document::ImageLayer,
    pub width: u32,
    pub height: u32,
    pub draft: [u32; 4],
    pub anchor: Option<DocPoint>,
}

impl ImageCropSession {
    fn source_point(&self, point: DocPoint) -> Option<DocPoint> {
        let [a, b, c, d, x, y] = self.image.transform;
        let determinant = a * d - b * c;
        if determinant.abs() < f32::EPSILON {
            return None;
        }
        let (dx, dy) = (point.x - x, point.y - y);
        Some(DocPoint {
            x: ((d * dx - c * dy) / determinant).clamp(0., self.width as f32),
            y: ((-b * dx + a * dy) / determinant).clamp(0., self.height as f32),
        })
    }
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
pub(crate) struct ArtboardResizeDrag {
    start: gpui_kit::Point<Pixels>,
    original: Document,
    handle: (i8, i8),
    scale: f32,
}
pub struct DrawingApp {
    pub(crate) handoff_loading: bool,
    pub(crate) handoff_checked: Instant,
    restart_checked: Instant,
    pub(crate) handoff_open: Option<(
        storyfella_studio_core::atelier::OpenRequest,
        crate::story_context::StoryTarget,
    )>,
    pub catalog: Option<crate::catalog_ui::CatalogState>,
    pub catalog_kind: storyfella_studio_core::registry::ArtifactKind,
    pub catalog_base: Option<uuid::Uuid>,
    pub catalog_error: Option<String>,
    pub catalog_refresh_error: Option<String>,
    pub catalog_busy: bool,
    pub catalog_conflict: Option<storyfella_studio_core::catalog::EditConflict>,
    catalog_refresh_pending: Option<(u64, PathBuf)>,
    catalog_refresh_at: Instant,
    pub catalog_name: Entity<InputState>,
    pub catalog_description: Entity<TextareaState>,
    pub project_tabs: Vec<crate::project_tabs::ProjectTab>,
    pub project_tab_order: Vec<u64>,
    pub closing_project: Option<u64>,
    pub next_project_epoch: u64,
    pub library: Vec<(Arc<document::Reference>, Arc<RenderImage>)>,
    pub library_loading: bool,
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
    pub resize_handle: Option<(i8, i8)>,
    pub rotating_layer: bool,
    pub transform_checkpointed: bool,
    pub transform_original: Option<Document>,
    pub transform_snap_x: Vec<f32>,
    pub transform_snap_y: Vec<f32>,
    pub transform_guides: Vec<(bool, f32)>,
    pub selection_marquee: Option<(DocPoint, DocPoint)>,
    pub marquee_initial_selection: Vec<usize>,
    pub marquee_additive: bool,
    pub marquee_bounds: Vec<(usize, (f32, f32, f32, f32))>,
    pub(crate) image_crop: Option<ImageCropSession>,
    pub collapsed_layer_groups: std::collections::HashSet<(u64, u32, String)>,
    pub editing_layer_group: Option<(u64, u32, String)>,
    pub(crate) artboard_resize_drag: Option<ArtboardResizeDrag>,
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
    pub dock_view: DockView,
    pub selected_layers: Vec<usize>,
    pub layer_selection_anchor: Option<usize>,
    pub show_layer_arrange: bool,
    pub align_each_confirmation: Option<(bool, u64, u32, u64, Vec<usize>)>,
    pub show_advanced_provider: bool,
    pub show_reference_tools: bool,
    pub grid: bool,
    pub guides: bool,
    pub grayscale: bool,
    pub favorite_filter: bool,
    pub tracing: bool,
    pub scale_art: bool,
    pub aspect_locked: bool,
    pub resize_anchor: (u8, u8),
    pub snap_enabled: bool,
    pub transparent: bool,
    pub jpeg: bool,
    pub jpeg_quality: u8,
    pub path: Option<PathBuf>,
    pub(crate) project_owner: Option<crate::ownership::Owner>,
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
    pub(crate) manual_pending: bool,
    last_edit: Instant,
    autosave_retry_after: Instant,
    pub status: String,
    pub activity: crate::activity::Activity,
    pub activity_selection: Option<u64>,
    pub activity_details: bool,
    pub activity_text: Entity<TextareaState>,
    pub activity_copy: Option<&'static str>,
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
    pub settings_save: Option<Settings>,
    pub generation: Option<Arc<AtomicBool>>,
    pub prompt_validation: Option<crate::activity::PromptValidation>,
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
    pub drawing_create_layer: bool,
    pub job_id: u64,
    pub job_epoch: u64,
    pub timer_seconds: u32,
    pub timer_remaining: Duration,
    pub timer_running: bool,
    timer_tick: Instant,
    pub exercise: usize,
    pub title: Entity<InputState>,
    pub layer_name: Entity<InputState>,
    pub group_name: Entity<InputState>,
    pub layer_x: Entity<InputState>,
    pub layer_y: Entity<InputState>,
    pub layer_width: Entity<InputState>,
    pub layer_height: Entity<InputState>,
    pub layer_opacity: Entity<InputState>,
    layer_input_draft: Option<LayerInputDraft>,
    pub layer_gap: Entity<InputState>,
    pub group_opacity: Entity<InputState>,
    pub layer_rotation: Entity<InputState>,
    pub layer_aspect_locked: bool,
    pub width: Entity<InputState>,
    pub height: Entity<InputState>,
    pub prompt: Entity<TextareaState>,
    pub(crate) prompt_target: String,
    pub search: Entity<InputState>,
    pub endpoint: Entity<InputState>,
    pub model: Entity<InputState>,
    pub edit_model: Entity<InputState>,
    pub positive_node: Entity<InputState>,
    pub latent_node: Entity<InputState>,
    pub seed_node: Entity<InputState>,
    pub steps: Entity<InputState>,
    pub request_timeout: Entity<InputState>,
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
        let snapshots = data_dir.join("open-projects");
        let unused_epoch = std::fs::read_dir(&snapshots)
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
        let (
            project_tabs,
            project_tab_order,
            doc,
            initial_epoch,
            initial_path,
            initial_dirty,
            project_owner,
        ) = match crate::project_tabs::recover(&data_dir) {
            Ok(session) => {
                if session.detached {
                    status = "Recovered an Atelier draft as a separate copy because its shared drawing changed owners".into();
                }
                match session.active {
                    Some(active) => (
                        session.tabs,
                        session.order,
                        active.doc,
                        active.epoch,
                        active.path,
                        active.dirty,
                        active.owner,
                    ),
                    None => (
                        vec![],
                        vec![],
                        Document::default(),
                        unused_epoch,
                        None,
                        false,
                        None,
                    ),
                }
            }
            Err(recovery_error) => {
                let snapshots = data_dir.join("open-projects");
                if snapshots.join("session.json").exists() {
                    status = format!(
                        "Open-project recovery failed: {recovery_error}. Snapshots remain in {}",
                        snapshots.display()
                    );
                    error = true;
                }
                let order = if data_dir.join("recovery.atelier").exists() {
                    vec![unused_epoch]
                } else {
                    vec![]
                };
                (vec![], order, doc, unused_epoch, None, false, None)
            }
        };
        let next_project_epoch = project_tabs
            .iter()
            .map(|tab| tab.epoch)
            .chain([initial_epoch])
            .max()
            .unwrap_or(1)
            .max(unused_epoch);
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
        let group_name = input("".into(), window, cx);
        let layer_x = input("0".into(), window, cx);
        let layer_y = input("0".into(), window, cx);
        let layer_width = input("0".into(), window, cx);
        let layer_height = input("0".into(), window, cx);
        let layer_opacity = input(
            format!("{}", doc.layers[doc.active_layer].opacity * 100.),
            window,
            cx,
        );
        let layer_gap = input("24".into(), window, cx);
        let group_opacity = input("100".into(), window, cx);
        let layer_rotation = input("0".into(), window, cx);
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
        let request_timeout = cx.new(|cx| {
            InputState::new(window, cx)
                .default_value(settings.request_timeout_minutes.clamp(1, 1440).to_string())
                .step(1.0)
                .min(1.0)
                .max(1440.0)
        });
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
        let selected_layers = vec![doc.active_layer];
        let layer_selection_anchor = Some(doc.active_layer);
        let mut app = Self {
            handoff_loading: false,
            handoff_checked: Instant::now(),
            restart_checked: Instant::now(),
            handoff_open: None,
            catalog: None,
            catalog_kind: storyfella_studio_core::registry::ArtifactKind::Character,
            catalog_base: None,
            catalog_error: None,
            catalog_refresh_error: None,
            catalog_busy: false,
            catalog_conflict: None,
            catalog_refresh_pending: None,
            catalog_refresh_at: Instant::now(),
            catalog_name: input(String::new(), window, cx),
            catalog_description: cx
                .new(|cx| TextareaState::new(window, cx).placeholder("Story-facing description")),
            project_tabs,
            project_tab_order,
            closing_project: None,
            next_project_epoch,
            library: vec![],
            library_loading: false,
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
            resize_handle: None,
            rotating_layer: false,
            transform_checkpointed: false,
            transform_original: None,
            transform_snap_x: Vec::new(),
            transform_snap_y: Vec::new(),
            transform_guides: Vec::new(),
            selection_marquee: None,
            marquee_initial_selection: Vec::new(),
            marquee_additive: false,
            marquee_bounds: Vec::new(),
            image_crop: None,
            collapsed_layer_groups: Default::default(),
            editing_layer_group: None,
            artboard_resize_drag: None,
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
            dock_view: DockView::Layers,
            selected_layers,
            layer_selection_anchor,
            show_layer_arrange: false,
            align_each_confirmation: None,
            show_advanced_provider: false,
            show_reference_tools: false,
            grid: false,
            guides: false,
            grayscale: false,
            favorite_filter: false,
            tracing: false,
            scale_art: false,
            aspect_locked: false,
            resize_anchor: (1, 1),
            snap_enabled: false,
            transparent: false,
            jpeg: false,
            jpeg_quality: 90,
            path: initial_path,
            project_owner,
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
            activity: crate::activity::Activity::default(),
            activity_selection: None,
            activity_details: false,
            activity_text: cx.new(|cx| TextareaState::new(window, cx)),
            activity_copy: None,
            settings,
            generation_provider_select,
            settings_provider_select,
            artboard_aspect_select,
            artboard_size_select,
            artboard_steps_select,
            lora_select,
            lora_weights,
            server_status: None,
            settings_save: None,
            generation: None,
            prompt_validation: None,
            queue: Queue::default(),
            generation_region: None,
            selecting_generation_region: false,
            region_drag: None,
            selection_mode: crate::workflow::SelectionMode::Blend,
            drawing_create_layer: false,
            job_id: 0,
            job_epoch: 0,
            timer_seconds: 300,
            timer_remaining: Duration::from_secs(300),
            timer_running: false,
            timer_tick: Instant::now(),
            exercise: 0,
            title,
            layer_name,
            group_name,
            layer_x,
            layer_y,
            layer_width,
            layer_height,
            layer_opacity,
            layer_input_draft: None,
            layer_gap,
            group_opacity,
            layer_rotation,
            layer_aspect_locked: true,
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
            request_timeout,
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
        if error {
            app.report_error(
                crate::activity::Scope::App,
                crate::activity::Task::Other,
                app.status.clone(),
            );
            app.status.clear();
        }
        app.sync_layer_controls(window, cx);
        app.sync_ref_inputs(window, cx);
        app.refresh_library();
        app.refresh_projects();
        app.reload_clipforge(window, cx);
        for (input, field) in [
            (app.layer_x.clone(), LayerInput::X),
            (app.layer_y.clone(), LayerInput::Y),
            (app.layer_width.clone(), LayerInput::Width),
            (app.layer_height.clone(), LayerInput::Height),
            (app.layer_opacity.clone(), LayerInput::Opacity),
            (app.group_opacity.clone(), LayerInput::GroupOpacity),
            (app.layer_name.clone(), LayerInput::Name),
            (app.group_name.clone(), LayerInput::GroupName),
            (app.layer_rotation.clone(), LayerInput::Rotation),
        ] {
            app.subscriptions.push(cx.subscribe_in(
                &input,
                window,
                move |this, input, event: &InputEvent, window, cx| {
                    match event {
                        InputEvent::Focus => {
                            if let Some(previous) =
                                this.layer_input_draft.as_ref().map(|draft| draft.field)
                            {
                                if previous != field && previous != LayerInput::Rotation {
                                    this.commit_layer_input(previous, window, cx);
                                }
                            }
                            this.layer_input_draft = Some(LayerInputDraft {
                                field,
                                epoch: this.epoch,
                                artboard: this.doc.canvas_id,
                                selection: this.selected_layers.clone(),
                                original: input.read(cx).value().to_string(),
                            })
                        }
                        InputEvent::PressEnter { .. } if field == LayerInput::Rotation => {
                            this.rotate_selection_by_input(window, cx);
                        }
                        InputEvent::PressEnter { .. } if field != LayerInput::Rotation => {
                            if this.layer_input_draft.is_none() {
                                this.layer_input_draft = Some(LayerInputDraft {
                                    field,
                                    epoch: this.epoch,
                                    artboard: this.doc.canvas_id,
                                    selection: this.selected_layers.clone(),
                                    original: String::new(),
                                });
                            }
                            this.commit_layer_input(field, window, cx);
                        }
                        InputEvent::Blur if input.read(cx).focus_handle(cx).is_focused(window) => {}
                        InputEvent::Blur if field != LayerInput::Rotation => {
                            this.commit_layer_input(field, window, cx);
                        }
                        InputEvent::Blur
                            if this
                                .layer_input_draft
                                .as_ref()
                                .is_some_and(|draft| draft.field == field) =>
                        {
                            this.layer_input_draft = None
                        }
                        _ => {}
                    }
                },
            ));
        }
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
                owner: None,
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
        let message = message.into();
        if error {
            let scope = if self.page == Page::Library
                || (self.page == Page::Image && self.image_return_page == Page::Library)
            {
                crate::activity::Scope::Library
            } else if self.project_tab_order.is_empty() {
                crate::activity::Scope::App
            } else {
                crate::activity::Scope::Project(self.epoch)
            };
            self.report_error(scope, crate::activity::Task::Other, message);
        } else {
            self.status = message;
        }
    }
    pub(crate) fn capture_prompt(&mut self, cx: &mut Context<Self>) {
        if self.project_tab_order.is_empty() {
            return;
        }
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
            if self.drawing_create_layer {
                format!("canvas:{}:layer", self.doc.canvas_id)
            } else {
                match self.region_bounds() {
                    Some((x, y, w, h)) => format!(
                        "canvas:{}:{:?}:{x}:{y}:{w}:{h}",
                        self.doc.canvas_id, self.selection_mode
                    ),
                    None => format!("canvas:{}:full", self.doc.canvas_id),
                }
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
        self.prompt_validation = None;
    }
    pub fn changed(&mut self) {
        self.ensure_project_tab();
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
        if let Some(original) = self.transform_original.take() {
            if self.transform_checkpointed {
                self.history.checkpoint(&original);
            }
        }
        self.active_stroke = false;
        self.last_pan = None;
        self.last_image_point = None;
        self.resize_handle = None;
        self.rotating_layer = false;
        self.transform_checkpointed = false;
        self.transform_snap_x.clear();
        self.transform_snap_y.clear();
        self.transform_guides.clear();
        self.selection_marquee = None;
        self.marquee_initial_selection.clear();
        self.marquee_additive = false;
        self.marquee_bounds.clear();
        self.artboard_resize_drag = None;
    }
    pub fn finish_marquee(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.selection_marquee.is_none() {
            return;
        }
        if let Some(index) = self.selected_layers.last().copied() {
            self.doc.active_layer = index;
            self.layer_selection_anchor = Some(index);
            let name = self.doc.layers[index].name.clone();
            self.layer_name
                .update(cx, |state, cx| state.set_value(name, window, cx));
        }
        self.sync_layer_controls(window, cx);
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
        if self.image_crop.take().is_some() {
            self.visual_revision += 1;
        }
        self.editing_layer_group = None;
        self.selected_layers = vec![self.doc.active_layer];
        self.layer_selection_anchor = Some(self.doc.active_layer);
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
            let height = if self.aspect_locked {
                ((width as f64 * self.doc.height as f64 / self.doc.width as f64).round() as u32)
                    .max(1)
            } else {
                self.height.read(cx).value().parse::<u32>()?
            };
            document::validate_size(width, height)?;
            self.checkpoint();
            self.doc
                .resize(width, height, self.scale_art, self.resize_anchor)?;
            Ok(())
        })();
        match result {
            Ok(()) => {
                self.changed();
                self.view = Viewport::default();
                self.sync_inputs(window, cx);
            }
            Err(e) => self.message(format!("Canvas: {e}"), true),
        }
    }
    pub fn fit_artboard_to_artwork(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let mut bounds: Option<(f32, f32, f32, f32)> = None;
        let mut probe = self.doc.clone();
        for index in 0..probe.layers.len() {
            let layer = &probe.layers[index];
            if !probe.layer_is_visible(layer) || layer.candidate {
                continue;
            }
            probe.active_layer = index;
            if let Some((left, top, right, bottom)) = probe.layer_bounds() {
                bounds = Some(match bounds {
                    Some((x0, y0, x1, y1)) => {
                        (x0.min(left), y0.min(top), x1.max(right), y1.max(bottom))
                    }
                    None => (left, top, right, bottom),
                });
            }
        }
        let Some((left, top, right, bottom)) = bounds else {
            self.message("There is no visible artwork to fit", true);
            return;
        };
        let padding = 24.;
        let width = ((right - left).ceil() + padding * 2.).max(64.) as u32;
        let height = ((bottom - top).ceil() + padding * 2.).max(64.) as u32;
        let result = (|| -> anyhow::Result<()> {
            document::validate_size(width, height)?;
            self.checkpoint();
            self.doc.resize(width, height, false, (0, 0))?;
            self.doc.translate_artwork(padding - left, padding - top);
            Ok(())
        })();
        match result {
            Ok(()) => {
                self.changed();
                self.view = Viewport::default();
                self.sync_inputs(window, cx);
                self.message("Artboard fit to visible artwork", false);
            }
            Err(error) => self.message(format!("Could not fit artboard: {error}"), true),
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
                previous_owner: self.project_owner.clone(),
                data_dir: self.data_dir.clone(),
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
        let path = match crate::ownership::target(&path) {
            Ok((path, _)) => path,
            Err(error) => {
                self.message(format!("Could not open project: {error:#}"), true);
                return;
            }
        };
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
                previous_owner: self.project_owner.clone(),
                data_dir: self.data_dir.clone(),
            })
            .is_ok();
        if !self.switching {
            self.message("Storage worker stopped", true);
        }
    }
    pub(crate) fn prepare_project_switch(&mut self) -> bool {
        if self.switching || self.closing_project.is_some() {
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
            locked: false,
            opacity: 1.,
            group: None,
            candidate: false,
            strokes: vec![],
            image: None,
        });
        self.doc.active_layer = self.doc.layers.len() - 1;
        self.doc.paint_layer = Some(self.doc.active_layer);
        self.selected_layers = vec![self.doc.active_layer];
        self.layer_selection_anchor = Some(self.doc.active_layer);
        self.changed();
    }

    pub fn set_paint_layer(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(layer) = self.doc.layers.get(index) else {
            return;
        };
        if layer.candidate {
            self.message("Accept the candidate before painting directly on it", false);
            return;
        }
        if self.doc.paint_layer != Some(index) {
            self.checkpoint();
            self.doc.paint_layer = Some(index);
            self.changed();
        }
        self.message(
            format!("Paint target: {}", self.doc.layers[index].name),
            false,
        );
        cx.notify();
    }

    fn dispatch_artwork_import(
        &mut self,
        destination: ArtworkImportDestination,
        inputs: Vec<crate::worker::ArtworkImport>,
        cx: &mut Context<Self>,
    ) {
        let ArtworkImportDestination {
            epoch,
            artboard,
            rect,
        } = destination;
        if inputs.is_empty() {
            return;
        }
        if !self.project_tab_order.contains(&epoch) {
            self.message(
                "The destination project was closed; import the images again",
                false,
            );
            cx.notify();
            return;
        }
        if self
            .workers
            .storage
            .send(Storage::ImportArtwork {
                epoch,
                artboard,
                rect,
                inputs,
            })
            .is_err()
        {
            self.message("Import worker stopped. Restart Atelier to retry.", true);
        } else {
            self.message("Importing images as artwork layers…", false);
        }
        cx.notify();
    }

    pub fn import_artwork_paths(&mut self, paths: Vec<PathBuf>, cx: &mut Context<Self>) {
        self.import_artwork_inputs(
            paths
                .into_iter()
                .map(crate::worker::ArtworkImport::File)
                .collect(),
            cx,
        );
    }

    fn import_artwork_inputs(
        &mut self,
        inputs: Vec<crate::worker::ArtworkImport>,
        cx: &mut Context<Self>,
    ) {
        if self.image_crop.is_some() {
            self.message("Apply or cancel the crop before importing", false);
            cx.notify();
            return;
        }
        self.finish_stroke();
        self.ensure_project_tab();
        self.dispatch_artwork_import(
            ArtworkImportDestination {
                epoch: self.epoch,
                artboard: self.doc.canvas_id,
                rect: (0, 0, self.doc.width, self.doc.height),
            },
            inputs,
            cx,
        );
    }

    pub fn paste_artwork(&mut self, cx: &mut Context<Self>) {
        let Some(item) = cx.read_from_clipboard() else {
            self.message("Clipboard does not contain an image", false);
            cx.notify();
            return;
        };
        let mut paths = Vec::new();
        let mut image = None;
        for entry in item.into_entries() {
            match entry {
                ClipboardEntry::ExternalPaths(files) => paths.extend(files.paths().iter().cloned()),
                ClipboardEntry::Image(value) if image.is_none() => image = Some(value.bytes),
                _ => {}
            }
        }
        if !paths.is_empty() {
            self.import_artwork_paths(paths, cx);
        } else if let Some(bytes) = image {
            self.import_artwork_inputs(vec![crate::worker::ArtworkImport::Clipboard(bytes)], cx);
        } else {
            self.message("Clipboard does not contain an image or image file", false);
            cx.notify();
        }
    }

    pub fn import_artwork_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.image_crop.is_some() {
            self.message("Apply or cancel the crop before importing", false);
            cx.notify();
            return;
        }
        self.finish_stroke();
        self.ensure_project_tab();
        let destination = ArtworkImportDestination {
            epoch: self.epoch,
            artboard: self.doc.canvas_id,
            rect: (0, 0, self.doc.width, self.doc.height),
        };
        let receiver = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: true,
            prompt: Some("Import images as artwork layers".into()),
        });
        let entity = cx.weak_entity();
        window
            .spawn(cx, async move |cx| {
                let result = receiver.await;
                let _ = entity.update_in(cx, |this, _, cx| match result {
                    Ok(Ok(Some(paths))) => this.dispatch_artwork_import(
                        destination,
                        paths
                            .into_iter()
                            .map(crate::worker::ArtworkImport::File)
                            .collect(),
                        cx,
                    ),
                    Ok(Ok(None)) => {}
                    _ => {
                        this.message("File dialog could not be opened", true);
                        cx.notify();
                    }
                });
            })
            .detach();
    }

    pub fn select_layer(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        if index >= self.doc.layers.len() {
            return;
        }
        self.finish_stroke();
        self.doc.active_layer = index;
        self.selected_layers = vec![index];
        let group = self.doc.layers[index].group.clone();
        self.sync_inputs(window, cx);
        self.editing_layer_group = group.map(|name| (self.epoch, self.doc.canvas_id, name));
        if let Some(key) = &self.editing_layer_group {
            self.collapsed_layer_groups.remove(key);
        }
        self.sync_layer_controls(window, cx);
        cx.notify();
    }

    pub fn select_layer_range(
        &mut self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if index >= self.doc.layers.len() {
            return;
        }
        self.finish_stroke();
        let anchor = self
            .layer_selection_anchor
            .filter(|anchor| *anchor < self.doc.layers.len())
            .unwrap_or(self.doc.active_layer);
        self.selected_layers = (anchor.min(index)..=anchor.max(index)).collect();
        self.doc.active_layer = index;
        let group = self.doc.layers[index].group.clone().filter(|name| {
            self.selected_layers
                .iter()
                .all(|index| self.doc.layers[*index].group.as_ref() == Some(name))
        });
        self.editing_layer_group = group.map(|name| (self.epoch, self.doc.canvas_id, name));
        let name = self.doc.layers[index].name.clone();
        self.layer_name
            .update(cx, |state, cx| state.set_value(name, window, cx));
        self.sync_layer_controls(window, cx);
        cx.notify();
    }

    pub fn start_image_crop(&mut self, cx: &mut Context<Self>) {
        if self.selected_layers.as_slice() != [self.doc.active_layer] {
            return;
        }
        let layer = &self.doc.layers[self.doc.active_layer];
        if self.doc.layer_is_locked(layer) {
            self.message("Unlock the image layer before cropping", false);
            return;
        }
        let Some(image) = layer.image.clone() else {
            return;
        };
        let decoded = match document::decode_reference(&image.reference) {
            Ok(decoded) => decoded,
            Err(error) => {
                self.message(format!("Could not read image: {error:#}"), true);
                return;
            }
        };
        let (width, height) = (decoded.width(), decoded.height());
        let draft = image.crop.unwrap_or([0, 0, width, height]);
        self.finish_stroke();
        self.image_crop = Some(ImageCropSession {
            layer: self.doc.active_layer,
            artboard: self.doc.canvas_id,
            image,
            width,
            height,
            draft,
            anchor: None,
        });
        self.visual_revision += 1;
        self.tool = Tool::Hand;
        self.moving_image = false;
        self.selecting_generation_region = false;
        self.panel = Panel::None;
        self.show_drawing_composer = false;
        self.message(
            "Drag a crop rectangle on the image, then Apply or Cancel",
            false,
        );
        cx.notify();
    }

    pub fn reset_image_crop_draft(&mut self, cx: &mut Context<Self>) {
        if let Some(crop) = &mut self.image_crop {
            crop.draft = [0, 0, crop.width, crop.height];
            crop.anchor = None;
        }
        cx.notify();
    }

    pub fn apply_image_crop(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(crop) = self.image_crop.take() else {
            return;
        };
        self.visual_revision += 1;
        self.tool = Tool::Hand;
        self.moving_image = true;
        let valid = crop.artboard == self.doc.canvas_id
            && self.selected_layers.as_slice() == [crop.layer]
            && self.doc.layers.get(crop.layer).is_some_and(|layer| {
                !self.doc.layer_is_locked(layer) && layer.image.as_ref() == Some(&crop.image)
            });
        if !valid {
            self.message("The image changed; start cropping again", true);
            cx.notify();
            return;
        }
        let new_crop = (crop.draft != [0, 0, crop.width, crop.height]).then_some(crop.draft);
        if crop.image.crop != new_crop {
            self.checkpoint();
            self.doc.layers[crop.layer].image.as_mut().unwrap().crop = new_crop;
            if new_crop.is_some() {
                self.doc.version = 3;
            }
            self.changed();
        }
        self.sync_layer_controls(window, cx);
        self.message("Crop applied · original image retained", false);
        cx.notify();
    }

    pub fn cancel_image_crop(&mut self, cx: &mut Context<Self>) {
        if self.image_crop.take().is_some() {
            self.visual_revision += 1;
        }
        self.tool = Tool::Hand;
        self.moving_image = true;
        self.message("Crop cancelled", false);
        cx.notify();
    }

    pub fn toggle_layer_selection(
        &mut self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.layer_selection_anchor = Some(index);
        if self.selected_layers.contains(&index) {
            self.selected_layers.retain(|selected| *selected != index);
            if self.doc.active_layer == index && !self.selected_layers.is_empty() {
                self.doc.active_layer = *self.selected_layers.last().unwrap();
            }
        } else if index < self.doc.layers.len() {
            self.selected_layers.push(index);
            self.selected_layers.sort_unstable();
            self.doc.active_layer = index;
        }
        let active_name = self.doc.layers[self.doc.active_layer].name.clone();
        self.layer_name
            .update(cx, |state, cx| state.set_value(active_name, window, cx));
        self.sync_layer_controls(window, cx);
        cx.notify();
    }

    pub fn toggle_layer_lock(&mut self, index: usize, cx: &mut Context<Self>) {
        if index >= self.doc.layers.len() {
            return;
        }
        self.checkpoint();
        self.doc.layers[index].locked = !self.doc.layers[index].locked;
        self.changed();
        cx.notify();
    }

    pub fn rename_active_layer(&mut self, cx: &mut Context<Self>) {
        let name = self.layer_name.read(cx).value().trim().to_owned();
        if name.is_empty() || name.len() > 128 {
            self.message("Layer names must contain 1–128 characters", true);
            return;
        }
        if self.doc.layers[self.doc.active_layer].name != name {
            self.checkpoint();
            self.doc.layers[self.doc.active_layer].name = name;
            self.changed();
        }
        cx.notify();
    }

    pub fn group_selected_layers(&mut self) {
        let mut selected: Vec<_> = self
            .selected_layers
            .iter()
            .copied()
            .filter(|index| *index < self.doc.layers.len())
            .collect();
        if selected.len() < 2 {
            self.message("Select at least two layers to group", true);
            return;
        }
        selected.sort_unstable();
        if selected
            .iter()
            .any(|index| self.doc.layers[*index].group.is_some())
        {
            self.message("Ungroup selected layers before creating a new group", false);
            return;
        }
        if selected.last().unwrap() - selected[0] + 1 != selected.len() {
            self.message(
                "Select adjacent layers to group without changing their stacking order",
                false,
            );
            return;
        }
        let used: std::collections::HashSet<String> = self
            .doc
            .layers
            .iter()
            .filter_map(|layer| layer.group.clone())
            .chain(self.doc.layer_groups.keys().cloned())
            .collect();
        let mut number = 1;
        while used.contains(&format!("Group {number}")) {
            number += 1;
        }
        let group = format!("Group {number}");
        self.checkpoint();
        self.editing_layer_group = None;
        for index in selected {
            self.doc.layers[index].group = Some(group.clone());
        }
        self.changed();
    }

    pub fn ungroup_selected_layers(&mut self) {
        let groups: std::collections::HashSet<_> = self
            .selected_layers
            .iter()
            .filter_map(|index| self.doc.layers.get(*index)?.group.clone())
            .collect();
        if groups.is_empty() {
            return;
        }
        for group in &groups {
            let properties = self.doc.layer_groups.get(group);
            if properties.is_some_and(|properties| {
                !properties.visible || properties.locked || properties.opacity != 1.
            }) {
                self.message(
                    "Show and unlock the group, and set its opacity to 100% before ungrouping",
                    false,
                );
                return;
            }
        }
        self.checkpoint();
        self.editing_layer_group = None;
        for layer in &mut self.doc.layers {
            if layer
                .group
                .as_ref()
                .is_some_and(|name| groups.contains(name))
            {
                layer.group = None;
            }
        }
        self.doc.prune_layer_groups();
        self.changed();
    }

    pub fn select_layer_group(
        &mut self,
        name: &str,
        toggle: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let members: Vec<_> = self
            .doc
            .layers
            .iter()
            .enumerate()
            .filter_map(|(index, layer)| (layer.group.as_deref() == Some(name)).then_some(index))
            .collect();
        if members.is_empty() {
            return;
        }
        self.finish_stroke();
        self.editing_layer_group = None;
        if toggle {
            let remove = members
                .iter()
                .all(|index| self.selected_layers.contains(index));
            if remove {
                self.selected_layers
                    .retain(|index| !members.contains(index));
            } else {
                for &index in &members {
                    if !self.selected_layers.contains(&index) {
                        self.selected_layers.push(index);
                    }
                }
            }
        } else {
            self.selected_layers = members.clone();
        }
        self.selected_layers.sort_unstable();
        if let Some(index) = self.selected_layers.last().copied() {
            self.doc.active_layer = index;
            self.layer_selection_anchor = Some(index);
            let name = self.doc.layers[index].name.clone();
            self.layer_name
                .update(cx, |state, cx| state.set_value(name, window, cx));
        }
        self.sync_layer_controls(window, cx);
        cx.notify();
    }

    /// A fully selected group is arranged as one unit; selected children can be arranged individually.
    pub fn selection_units(&self) -> Vec<Vec<usize>> {
        let mut units = Vec::new();
        let mut assigned = std::collections::HashSet::new();
        for &index in &self.selected_layers {
            if assigned.contains(&index) {
                continue;
            }
            let Some(layer) = self.doc.layers.get(index) else {
                continue;
            };
            let members: Vec<_> = layer
                .group
                .as_ref()
                .map(|name| {
                    self.doc
                        .layers
                        .iter()
                        .enumerate()
                        .filter_map(|(i, layer)| (layer.group.as_ref() == Some(name)).then_some(i))
                        .collect()
                })
                .unwrap_or_default();
            if members.len() > 1
                && layer.group.as_deref() != self.editing_group_name()
                && members.iter().all(|i| self.selected_layers.contains(i))
            {
                assigned.extend(members.iter().copied());
                units.push(members);
            } else {
                assigned.insert(index);
                units.push(vec![index]);
            }
        }
        units
    }

    pub fn editing_group_name(&self) -> Option<&str> {
        self.editing_layer_group
            .as_ref()
            .filter(|(epoch, canvas, _)| *epoch == self.epoch && *canvas == self.doc.canvas_id)
            .map(|(_, _, name)| name.as_str())
    }

    pub fn selected_group_name(&self) -> Option<&str> {
        let units = self.selection_units();
        if units.len() != 1 {
            return None;
        }
        let group = self.doc.layers.get(units[0][0])?.group.as_deref()?;
        if self.editing_group_name() == Some(group) {
            return None;
        }
        let all_selected = self
            .doc
            .layers
            .iter()
            .enumerate()
            .filter(|(_, layer)| layer.group.as_deref() == Some(group))
            .all(|(index, _)| self.selected_layers.contains(&index));
        all_selected.then_some(group)
    }

    /// A complete group and one ungrouped layer can be selected together.
    pub fn selected_group_and_layer(&self) -> Option<(String, usize)> {
        if self.editing_group_name().is_some() {
            return None;
        }
        let units = self.selection_units();
        if units.len() != 2 {
            return None;
        }
        for (group_unit, layer_unit) in [(&units[0], &units[1]), (&units[1], &units[0])] {
            if layer_unit.len() != 1 {
                continue;
            }
            let layer_index = layer_unit[0];
            if self.doc.layers.get(layer_index)?.group.is_some() {
                continue;
            }
            let Some(name) = self
                .doc
                .layers
                .get(group_unit[0])
                .and_then(|layer| layer.group.as_ref())
            else {
                continue;
            };
            let members: Vec<_> = self
                .doc
                .layers
                .iter()
                .enumerate()
                .filter_map(|(index, layer)| (layer.group.as_ref() == Some(name)).then_some(index))
                .collect();
            if members == *group_unit {
                return Some((name.clone(), layer_index));
            }
        }
        None
    }

    pub fn add_selected_layer_to_group(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some((name, index)) = self.selected_group_and_layer() else {
            return;
        };
        if self.doc.layer_is_locked(&self.doc.layers[index]) {
            self.message("Unlock the layer before adding it to a group", false);
            return;
        }
        if self
            .doc
            .layer_groups
            .get(&name)
            .is_some_and(|group| !group.visible || group.locked || group.opacity != 1.)
        {
            self.message(
                "Show and unlock the group, and set its opacity to 100% before adding the layer",
                false,
            );
            return;
        }
        let members: Vec<_> = self
            .doc
            .layers
            .iter()
            .enumerate()
            .filter_map(|(i, layer)| (layer.group.as_deref() == Some(name.as_str())).then_some(i))
            .collect();
        let (Some(&first), Some(&last)) = (members.first(), members.last()) else {
            return;
        };
        if last - first + 1 != members.len()
            || !(index.checked_add(1) == Some(first) || last.checked_add(1) == Some(index))
        {
            self.message(
                "Place the layer next to the group before adding it, so stacking stays unchanged",
                false,
            );
            return;
        }
        self.checkpoint();
        self.doc.layers[index].group = Some(name.clone());
        self.editing_layer_group = None;
        self.changed();
        self.sync_layer_controls(window, cx);
        self.message(format!("Added layer to {name}"), false);
        cx.notify();
    }

    pub fn remove_selected_layer_from_group(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(name) = self.editing_group_name().map(str::to_owned) else {
            return;
        };
        if self.selected_layers.len() != 1 {
            return;
        }
        let index = self.selected_layers[0];
        let Some(layer) = self.doc.layers.get(index) else {
            return;
        };
        if layer.group.as_deref() != Some(name.as_str()) {
            return;
        }
        if self.doc.layer_is_locked(layer) {
            self.message("Unlock the layer and its group first", false);
            return;
        }
        if self
            .doc
            .layer_groups
            .get(&name)
            .is_some_and(|group| !group.visible || group.opacity != 1.)
        {
            self.message(
                "Show the group and set its opacity to 100% before removing the layer",
                false,
            );
            return;
        }
        let members: Vec<_> = self
            .doc
            .layers
            .iter()
            .enumerate()
            .filter_map(|(i, layer)| (layer.group.as_deref() == Some(name.as_str())).then_some(i))
            .collect();
        if members.first() != Some(&index) && members.last() != Some(&index) {
            self.message(
                "Move the layer to an edge of the group before removing it",
                false,
            );
            return;
        }
        self.checkpoint();
        self.doc.layers[index].group = None;
        self.doc.prune_layer_groups();
        self.editing_layer_group = None;
        self.changed();
        self.sync_layer_controls(window, cx);
        self.message("Layer removed from group", false);
        cx.notify();
    }

    pub fn duplicate_selected_layers(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.selected_layers.is_empty() {
            return;
        }
        if self.selection_is_locked() {
            self.message("Unlock selected layers before duplicating them", false);
            return;
        }
        let mut selected = self.selected_layers.clone();
        selected.sort_unstable();
        selected.dedup();
        if self.doc.layers.len() + selected.len() > 24 {
            self.message("Duplicating would exceed the 24-layer limit", true);
            return;
        }
        if selected.iter().any(|index| {
            self.doc
                .layers
                .get(*index)
                .is_none_or(|layer| layer.candidate)
        }) {
            self.message("Accept candidates before duplicating them", false);
            return;
        }
        let groups: std::collections::BTreeSet<_> = selected
            .iter()
            .filter_map(|index| self.doc.layers[*index].group.clone())
            .collect();
        for group in &groups {
            let members: Vec<_> = self
                .doc
                .layers
                .iter()
                .enumerate()
                .filter_map(|(index, layer)| (layer.group.as_ref() == Some(group)).then_some(index))
                .collect();
            let complete = members.iter().all(|index| selected.contains(index));
            if !complete
                && selected
                    .iter()
                    .any(|index| self.doc.layers[*index].group.as_ref() != Some(group))
            {
                self.message(
                    "Select the whole group, or duplicate its children separately",
                    false,
                );
                return;
            }
        }
        self.finish_stroke();
        let previous = self.doc.clone();
        let mut next = previous.clone();
        let mut used: std::collections::HashSet<String> = next
            .layers
            .iter()
            .filter_map(|layer| layer.group.clone())
            .chain(next.layer_groups.keys().cloned())
            .collect();
        let mut copies = std::collections::BTreeMap::new();
        for group in &groups {
            let all = previous
                .layers
                .iter()
                .enumerate()
                .filter(|(_, layer)| layer.group.as_ref() == Some(group))
                .all(|(index, _)| selected.contains(&index));
            if all && self.editing_group_name() != Some(group.as_str()) {
                let mut number = 1;
                while used.contains(&format!("Group {number}")) {
                    number += 1;
                }
                let name = format!("Group {number}");
                used.insert(name.clone());
                if let Some(properties) = previous.layer_groups.get(group) {
                    next.layer_groups.insert(name.clone(), properties.clone());
                }
                copies.insert(group.clone(), name);
            }
        }
        let insert = selected.last().copied().unwrap() + 1;
        let mut duplicated = Vec::with_capacity(selected.len());
        for &index in &selected {
            let mut layer = previous.layers[index].clone();
            if let Some(group) = layer.group.as_ref().and_then(|group| copies.get(group)) {
                layer.group = Some(group.clone());
            }
            if layer.group.is_none() {
                layer.name = format!("{} copy", layer.name);
            }
            duplicated.push(layer);
        }
        let count = duplicated.len();
        next.layers.splice(insert..insert, duplicated);
        next.paint_layer = next.paint_layer.map(|index| {
            if index >= insert {
                index + count
            } else {
                index
            }
        });
        let indices: Vec<_> = (insert..insert + count).collect();
        next.active_layer = *indices.last().unwrap();
        if let Err(error) = next
            .translate_layers(&indices, 16., 16.)
            .and_then(|()| next.validate())
        {
            self.message(format!("Could not duplicate selection: {error:#}"), true);
            return;
        }
        self.history.checkpoint(&previous);
        self.doc = next;
        self.selected_layers = indices;
        self.layer_selection_anchor = Some(self.doc.active_layer);
        if !copies.is_empty() {
            self.editing_layer_group = None;
        }
        let name = self.doc.layers[self.doc.active_layer].name.clone();
        self.layer_name
            .update(cx, |state, cx| state.set_value(name, window, cx));
        self.changed();
        self.sync_layer_controls(window, cx);
        cx.notify();
    }

    pub fn rename_selected_group(&mut self, cx: &mut Context<Self>) {
        let Some(old) = self.selected_group_name().map(str::to_owned) else {
            return;
        };
        let name = self.group_name.read(cx).value().trim().to_owned();
        if name.is_empty() || name.len() > 128 {
            self.message("Group names must contain 1–128 characters", true);
            return;
        }
        if name == old {
            return;
        }
        if self
            .doc
            .layers
            .iter()
            .any(|layer| layer.group.as_deref() == Some(name.as_str()))
        {
            self.message(
                "A group with that name already exists on this artboard",
                true,
            );
            return;
        }
        self.checkpoint();
        if let Some(properties) = self.doc.layer_groups.remove(&old) {
            self.doc.layer_groups.insert(name.clone(), properties);
        }
        for layer in &mut self.doc.layers {
            if layer.group.as_deref() == Some(old.as_str()) {
                layer.group = Some(name.clone());
            }
        }
        let old_key = (self.epoch, self.doc.canvas_id, old);
        if self.collapsed_layer_groups.remove(&old_key) {
            self.collapsed_layer_groups
                .insert((self.epoch, self.doc.canvas_id, name));
        }
        self.changed();
        cx.notify();
    }

    pub fn toggle_group_visibility(&mut self, name: &str, cx: &mut Context<Self>) {
        if !self
            .doc
            .layers
            .iter()
            .any(|layer| layer.group.as_deref() == Some(name))
        {
            return;
        }
        self.checkpoint();
        self.doc.version = 3;
        let group = self.doc.layer_groups.entry(name.to_owned()).or_default();
        group.visible = !group.visible;
        self.changed();
        cx.notify();
    }

    pub fn toggle_group_lock(&mut self, name: &str, cx: &mut Context<Self>) {
        if !self
            .doc
            .layers
            .iter()
            .any(|layer| layer.group.as_deref() == Some(name))
        {
            return;
        }
        self.checkpoint();
        self.doc.version = 3;
        let group = self.doc.layer_groups.entry(name.to_owned()).or_default();
        group.locked = !group.locked;
        self.changed();
        cx.notify();
    }

    pub fn apply_group_opacity(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(name) = self.selected_group_name().map(str::to_owned) else {
            return;
        };
        if !self.doc.group_is_contiguous(&name) {
            self.message(
                "Ungroup and regroup adjacent layers to change group opacity",
                false,
            );
            return;
        }
        if self
            .doc
            .layer_groups
            .get(&name)
            .is_some_and(|group| group.locked)
        {
            self.message("Unlock the group before changing its opacity", false);
            return;
        }
        let value = self.group_opacity.read(cx).value().parse::<f32>().ok();
        let Some(value) = value.filter(|v| v.is_finite() && (0. ..=100.).contains(v)) else {
            self.message("Enter group opacity from 0 to 100", true);
            return;
        };
        let previous = self
            .doc
            .layer_groups
            .get(&name)
            .map_or(1., |group| group.opacity);
        if (previous - value / 100.).abs() > 0.0001 {
            self.checkpoint();
            self.doc.layer_groups.entry(name).or_default().opacity = value / 100.;
            self.doc.version = 3;
            self.changed();
        }
        self.sync_layer_controls(window, cx);
        cx.notify();
    }

    fn commit_layer_input(
        &mut self,
        field: LayerInput,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self
            .layer_input_draft
            .as_ref()
            .is_some_and(|draft| draft.field != field)
        {
            return;
        }
        let Some(draft) = self.layer_input_draft.take() else {
            return;
        };
        if draft.field != field
            || draft.epoch != self.epoch
            || draft.artboard != self.doc.canvas_id
            || draft.selection != self.selected_layers
        {
            return;
        }
        let input = match field {
            LayerInput::X => &self.layer_x,
            LayerInput::Y => &self.layer_y,
            LayerInput::Width => &self.layer_width,
            LayerInput::Height => &self.layer_height,
            LayerInput::Opacity => &self.layer_opacity,
            LayerInput::GroupOpacity => &self.group_opacity,
            LayerInput::Name => &self.layer_name,
            LayerInput::GroupName => &self.group_name,
            LayerInput::Rotation => return,
        }
        .clone();
        let value = input.read(cx).value().trim().to_owned();
        let unchanged_number = !matches!(field, LayerInput::Name | LayerInput::GroupName)
            && value
                .parse::<f32>()
                .ok()
                .zip(draft.original.trim().parse::<f32>().ok())
                .is_some_and(|(left, right)| left.is_finite() && left == right);
        if value != draft.original.trim() && !unchanged_number && !self.selected_layers.is_empty() {
            match field {
                LayerInput::Name => self.rename_active_layer(cx),
                LayerInput::GroupName => self.rename_selected_group(cx),
                LayerInput::GroupOpacity => self.apply_group_opacity(window, cx),
                _ if self.selection_is_locked() => {
                    self.message("Unlock selected layers before editing properties", false)
                }
                _ => {
                    let parsed = value.parse::<f32>().ok().filter(|v| v.is_finite());
                    let valid = parsed.filter(|v| match field {
                        LayerInput::Width | LayerInput::Height => *v > 0.,
                        LayerInput::Opacity => (0. ..=100.).contains(v),
                        _ => true,
                    });
                    if let Some(value) = valid {
                        let previous = self.doc.clone();
                        let mut next = previous.clone();
                        let result = if field == LayerInput::Opacity {
                            for &index in &self.selected_layers {
                                next.layers[index].opacity = value / 100.;
                            }
                            Ok(())
                        } else if let Some((x, y, right, bottom)) =
                            self.doc.selected_layer_bounds(&self.selected_layers)
                        {
                            let (mut width, mut height) = (right - x, bottom - y);
                            let (mut left, mut top) = (x, y);
                            match field {
                                LayerInput::X => left = value,
                                LayerInput::Y => top = value,
                                LayerInput::Width => {
                                    if self.layer_aspect_locked {
                                        height *= value / width;
                                    }
                                    width = value;
                                }
                                LayerInput::Height => {
                                    if self.layer_aspect_locked {
                                        width *= value / height;
                                    }
                                    height = value;
                                }
                                _ => {}
                            }
                            next.transform_layers_to_bounds(
                                &self.selected_layers,
                                (left, top, width, height),
                            )
                        } else {
                            Err(anyhow::anyhow!("Selected artwork has no bounds"))
                        };
                        match result {
                            Ok(()) => {
                                self.history.checkpoint(&previous);
                                self.doc = next;
                                self.changed();
                            }
                            Err(error) => self.message(error.to_string(), true),
                        }
                    } else {
                        self.message(
                            match field {
                                LayerInput::Opacity => "Enter opacity from 0 to 100",
                                LayerInput::Width | LayerInput::Height => "Enter a positive size",
                                _ => "Enter a finite position",
                            },
                            true,
                        );
                    }
                }
            }
        }
        self.sync_layer_controls(window, cx);
        let original = input.read(cx).value().to_string();
        if input.read(cx).focus_handle(cx).is_focused(window) {
            self.layer_input_draft = Some(LayerInputDraft {
                field,
                epoch: self.epoch,
                artboard: self.doc.canvas_id,
                selection: self.selected_layers.clone(),
                original,
            });
        }
        cx.notify();
    }

    pub fn sync_layer_controls(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let focused = self
            .layer_input_draft
            .as_ref()
            .filter(|draft| {
                draft.epoch == self.epoch
                    && draft.artboard == self.doc.canvas_id
                    && draft.selection == self.selected_layers
            })
            .filter(|draft| {
                let input = match draft.field {
                    LayerInput::X => &self.layer_x,
                    LayerInput::Y => &self.layer_y,
                    LayerInput::Width => &self.layer_width,
                    LayerInput::Height => &self.layer_height,
                    LayerInput::Opacity => &self.layer_opacity,
                    LayerInput::GroupOpacity => &self.group_opacity,
                    LayerInput::Name => &self.layer_name,
                    LayerInput::GroupName => &self.group_name,
                    LayerInput::Rotation => &self.layer_rotation,
                };
                input.read(cx).value().as_ref() != draft.original
            })
            .map(|draft| draft.field);
        let group_name = self.selected_group_name().unwrap_or("").to_owned();
        if focused != Some(LayerInput::GroupName) {
            self.group_name
                .update(cx, |state, cx| state.set_value(group_name, window, cx));
        }
        let opacity = self
            .selected_group_name()
            .and_then(|name| self.doc.layer_groups.get(name))
            .map_or(100., |group| group.opacity * 100.);
        if focused != Some(LayerInput::GroupOpacity) {
            self.group_opacity.update(cx, |state, cx| {
                state.set_value(format!("{opacity:.2}"), window, cx)
            });
        }
        let values = self
            .doc
            .selected_layer_bounds(&self.selected_layers)
            .map(|(left, top, right, bottom)| {
                [left, top, right - left, bottom - top].map(|value| format!("{value:.2}"))
            })
            .unwrap_or_else(|| ["0".into(), "0".into(), "0".into(), "0".into()]);
        let common_opacity = self
            .selected_layers
            .first()
            .and_then(|index| self.doc.layers.get(*index))
            .map(|layer| layer.opacity)
            .filter(|opacity| {
                self.selected_layers.iter().all(|index| {
                    self.doc
                        .layers
                        .get(*index)
                        .is_some_and(|layer| (layer.opacity - opacity).abs() < 0.0001)
                })
            })
            .map(|opacity| format!("{:.2}", opacity * 100.))
            .unwrap_or_default();
        for (input, value, field) in [
            (&self.layer_x, values[0].clone(), LayerInput::X),
            (&self.layer_y, values[1].clone(), LayerInput::Y),
            (&self.layer_width, values[2].clone(), LayerInput::Width),
            (&self.layer_height, values[3].clone(), LayerInput::Height),
            (&self.layer_opacity, common_opacity, LayerInput::Opacity),
        ] {
            if focused != Some(field) {
                input.update(cx, |s, cx| s.set_value(value, window, cx));
            }
        }
        if focused.is_none() {
            if let Some(draft) = &mut self.layer_input_draft {
                if draft.epoch == self.epoch
                    && draft.artboard == self.doc.canvas_id
                    && draft.selection == self.selected_layers
                {
                    let input = match draft.field {
                        LayerInput::X => &self.layer_x,
                        LayerInput::Y => &self.layer_y,
                        LayerInput::Width => &self.layer_width,
                        LayerInput::Height => &self.layer_height,
                        LayerInput::Opacity => &self.layer_opacity,
                        LayerInput::GroupOpacity => &self.group_opacity,
                        LayerInput::Name => &self.layer_name,
                        LayerInput::GroupName => &self.group_name,
                        LayerInput::Rotation => &self.layer_rotation,
                    };
                    draft.original = input.read(cx).value().to_string();
                }
            }
        }
    }

    pub(crate) fn selection_is_locked(&self) -> bool {
        self.selected_layers.iter().any(|index| {
            self.doc
                .layers
                .get(*index)
                .is_some_and(|layer| self.doc.layer_is_locked(layer))
        })
    }

    fn snap_axis(start: f32, extent: f32, targets: &[f32], tolerance: f32) -> (f32, Option<f32>) {
        let best = [start, start + extent / 2., start + extent]
            .into_iter()
            .flat_map(|point| {
                targets
                    .iter()
                    .copied()
                    .map(move |target| (target - point, target))
            })
            .filter(|(delta, _)| delta.abs() <= tolerance)
            .min_by(|a, b| a.0.abs().total_cmp(&b.0.abs()));
        best.map_or(((start / 16.).round() * 16., None), |(delta, target)| {
            (start + delta, Some(target))
        })
    }

    fn prepare_transform_snap_targets(&mut self) {
        self.transform_snap_x = vec![0., self.doc.width as f32 / 2., self.doc.width as f32];
        self.transform_snap_y = vec![0., self.doc.height as f32 / 2., self.doc.height as f32];
        for (index, layer) in self.doc.layers.iter().enumerate() {
            if self.selected_layers.contains(&index) || !self.doc.layer_is_visible(layer) {
                continue;
            }
            if let Some((left, top, right, bottom)) = self.doc.layer_bounds_for(index) {
                self.transform_snap_x
                    .extend([left, (left + right) / 2., right]);
                self.transform_snap_y
                    .extend([top, (top + bottom) / 2., bottom]);
            }
        }
    }

    pub fn align_selection(
        &mut self,
        horizontal: bool,
        mode: u8,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let units = self.selection_units();
        if units.len() < 2 || self.selection_is_locked() {
            return;
        }
        let expected = units.len();
        let mut bounds: Vec<_> = units
            .into_iter()
            .filter_map(|unit| {
                self.doc
                    .selected_layer_bounds(&unit)
                    .map(|rect| (unit, rect))
            })
            .collect();
        if bounds.len() != expected {
            self.message("Each selected item needs artwork before alignment", false);
            cx.notify();
            return;
        }
        let (group_start, group_end) = if horizontal {
            (
                bounds
                    .iter()
                    .map(|(_, rect)| rect.0)
                    .fold(f32::MAX, f32::min),
                bounds
                    .iter()
                    .map(|(_, rect)| rect.2)
                    .fold(f32::MIN, f32::max),
            )
        } else {
            (
                bounds
                    .iter()
                    .map(|(_, rect)| rect.1)
                    .fold(f32::MAX, f32::min),
                bounds
                    .iter()
                    .map(|(_, rect)| rect.3)
                    .fold(f32::MIN, f32::max),
            )
        };
        let target = match mode {
            0 => group_start,
            1 => (group_start + group_end) / 2.,
            _ => group_end,
        };
        let previous = self.doc.clone();
        let mut changed = false;
        for (unit, rect) in bounds.drain(..) {
            let (start, end) = if horizontal {
                (rect.0, rect.2)
            } else {
                (rect.1, rect.3)
            };
            let current = match mode {
                0 => start,
                1 => (start + end) / 2.,
                _ => end,
            };
            let delta = target - current;
            if delta.abs() < 0.001 {
                continue;
            }
            let result = if horizontal {
                self.doc.translate_layers(&unit, delta, 0.)
            } else {
                self.doc.translate_layers(&unit, 0., delta)
            };
            if let Err(error) = result {
                self.doc = previous;
                self.message(error.to_string(), true);
                cx.notify();
                return;
            }
            changed = true;
        }
        if changed {
            self.history.checkpoint(&previous);
            self.changed();
            self.sync_layer_controls(window, cx);
        }
        cx.notify();
    }

    pub fn distribute_selection(
        &mut self,
        horizontal: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let units = self.selection_units();
        if units.len() < 3 || self.selection_is_locked() {
            return;
        }
        let expected = units.len();
        let mut items: Vec<_> = units
            .into_iter()
            .filter_map(|unit| {
                self.doc.selected_layer_bounds(&unit).map(|rect| {
                    (
                        unit,
                        if horizontal { rect.0 } else { rect.1 },
                        if horizontal { rect.2 } else { rect.3 },
                    )
                })
            })
            .collect();
        if items.len() != expected {
            self.message(
                "Each selected item needs artwork before distribution",
                false,
            );
            cx.notify();
            return;
        }
        items.sort_by(|a, b| a.1.total_cmp(&b.1));
        let span = items
            .iter()
            .map(|(_, _, end)| *end)
            .fold(f32::NEG_INFINITY, f32::max)
            - items[0].1;
        let total_size: f32 = items.iter().map(|(_, start, end)| end - start).sum();
        let gap = (span - total_size) / (items.len() - 1) as f32;
        let previous = self.doc.clone();
        let mut cursor = items[0].1;
        let mut changed = false;
        for (unit, start, end) in items {
            let target = cursor;
            let delta = target - start;
            let result = if delta.abs() < 0.001 {
                Ok(())
            } else if horizontal {
                self.doc.translate_layers(&unit, delta, 0.)
            } else {
                self.doc.translate_layers(&unit, 0., delta)
            };
            if let Err(error) = result {
                self.doc = previous;
                self.message(error.to_string(), true);
                cx.notify();
                return;
            }
            changed |= delta.abs() >= 0.001;
            cursor += end - start + gap;
        }
        if changed {
            self.history.checkpoint(&previous);
            self.changed();
            self.sync_layer_controls(window, cx);
        }
        cx.notify();
    }

    pub fn set_selection_gap(
        &mut self,
        horizontal: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.selection_units().len() < 2 || self.selection_is_locked() {
            return;
        }
        let gap = self.layer_gap.read(cx).value().parse::<f32>().ok();
        let Some(gap) =
            gap.filter(|value| value.is_finite() && *value >= 0. && *value <= 1_000_000.)
        else {
            self.message("Enter a gap from 0 to 1,000,000 pixels", true);
            return;
        };
        let units = self.selection_units();
        let mut items: Vec<_> = units
            .iter()
            .filter_map(|unit| {
                self.doc.selected_layer_bounds(unit).map(|bounds| {
                    (
                        unit.clone(),
                        if horizontal { bounds.0 } else { bounds.1 },
                        if horizontal { bounds.2 } else { bounds.3 },
                    )
                })
            })
            .collect();
        if items.len() != units.len() {
            self.message(
                "Each selected layer needs visible artwork to set a gap",
                false,
            );
            return;
        }
        items.sort_by(|a, b| a.1.total_cmp(&b.1).then(a.0.cmp(&b.0)));
        let previous = self.doc.clone();
        let mut cursor = items[0].2 + gap;
        let mut changed = false;
        for (unit, start, end) in items.into_iter().skip(1) {
            let delta = cursor - start;
            if delta.abs() > 0.001 {
                let result = if horizontal {
                    self.doc.translate_layers(&unit, delta, 0.)
                } else {
                    self.doc.translate_layers(&unit, 0., delta)
                };
                if let Err(error) = result {
                    self.doc = previous;
                    self.message(error.to_string(), true);
                    return;
                }
                changed = true;
            }
            cursor += end - start + gap;
        }
        if changed {
            self.history.checkpoint(&previous);
            self.changed();
            self.sync_layer_controls(window, cx);
        }
        cx.notify();
    }

    pub fn position_selection_on_artboard(
        &mut self,
        horizontal: bool,
        mode: u8,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.selection_is_locked() {
            self.message("Unlock selected layers before positioning them", false);
            return;
        }
        let Some((left, top, right, bottom)) =
            self.doc.selected_layer_bounds(&self.selected_layers)
        else {
            return;
        };
        let (start, end, extent) = if horizontal {
            (left, right, self.doc.width as f32)
        } else {
            (top, bottom, self.doc.height as f32)
        };
        let delta = match mode {
            0 => -start,
            1 => (extent - start - end) / 2.,
            _ => extent - end,
        };
        if delta.abs() < f32::EPSILON {
            return;
        }
        let previous = self.doc.clone();
        let (dx, dy) = if horizontal { (delta, 0.) } else { (0., delta) };
        match self.doc.translate_layers(&self.selected_layers, dx, dy) {
            Ok(()) => {
                self.history.checkpoint(&previous);
                self.changed();
                self.sync_layer_controls(window, cx);
            }
            Err(error) => self.message(error.to_string(), true),
        }
        cx.notify();
    }

    pub fn align_each_to_artboard(
        &mut self,
        horizontal: bool,
        mode: u8,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.selection_is_locked() {
            self.message("Unlock selected layers before aligning them", false);
            return;
        }
        let units = self.selection_units();
        if units.is_empty() {
            return;
        }
        let extent = if horizontal {
            self.doc.width as f32
        } else {
            self.doc.height as f32
        };
        let target = match mode {
            0 => 0.,
            1 => extent / 2.,
            _ => extent,
        };
        let previous = self.doc.clone();
        let mut next = previous.clone();
        let mut changed = false;
        for unit in units {
            let Some((left, top, right, bottom)) = previous.selected_layer_bounds(&unit) else {
                self.message(
                    "Each selected item needs artwork before it can be aligned",
                    false,
                );
                return;
            };
            let (start, end) = if horizontal {
                (left, right)
            } else {
                (top, bottom)
            };
            let current = match mode {
                0 => start,
                1 => (start + end) / 2.,
                _ => end,
            };
            let delta = target - current;
            if delta.abs() < 0.001 {
                continue;
            }
            let (dx, dy) = if horizontal { (delta, 0.) } else { (0., delta) };
            if let Err(error) = next.translate_layers(&unit, dx, dy) {
                self.message(error.to_string(), true);
                return;
            }
            changed = true;
        }
        if changed {
            self.history.checkpoint(&previous);
            self.doc = next;
            self.changed();
            self.sync_layer_controls(window, cx);
        }
        cx.notify();
    }

    pub fn flip_selection(
        &mut self,
        horizontal: bool,
        vertical: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.selection_is_locked() {
            self.message("Unlock selected layers before flipping them", false);
            return;
        }
        let previous = self.doc.clone();
        match self
            .doc
            .flip_layers(&self.selected_layers, horizontal, vertical)
        {
            Ok(()) => {
                self.history.checkpoint(&previous);
                self.changed();
                self.sync_layer_controls(window, cx);
            }
            Err(error) => self.message(error.to_string(), true),
        }
    }

    pub fn rotate_selection_by_input(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.selection_is_locked() {
            self.message("Unlock selected layers before rotating them", false);
            return;
        }
        let Some((left, top, right, bottom)) =
            self.doc.selected_layer_bounds(&self.selected_layers)
        else {
            self.message("Select artwork to rotate", false);
            return;
        };
        let angle = self.layer_rotation.read(cx).value().parse::<f32>().ok();
        let Some(angle) = angle.filter(|value| value.is_finite() && value.abs() <= 360_000.) else {
            self.message("Enter a finite rotation in degrees", true);
            return;
        };
        if angle.rem_euclid(360.).abs() < 0.0001 {
            return;
        }
        let previous = self.doc.clone();
        let center = DocPoint {
            x: (left + right) / 2.,
            y: (top + bottom) / 2.,
        };
        match self
            .doc
            .rotate_layers(&self.selected_layers, center, angle.to_radians())
        {
            Ok(()) => {
                self.history.checkpoint(&previous);
                self.changed();
                self.layer_rotation
                    .update(cx, |state, cx| state.set_value("0", window, cx));
                self.sync_layer_controls(window, cx);
            }
            Err(error) => self.message(error.to_string(), true),
        }
        cx.notify();
    }

    pub fn set_snap_enabled(&mut self, enabled: bool) {
        self.snap_enabled = enabled;
    }

    pub fn zoom_to_selection(&mut self, cx: &mut Context<Self>) {
        let Some((left, top, right, bottom)) =
            self.doc.selected_layer_bounds(&self.selected_layers)
        else {
            self.message("Select artwork to zoom to it", false);
            return;
        };
        let points = [
            DocPoint { x: left, y: top },
            DocPoint { x: right, y: top },
            DocPoint {
                x: right,
                y: bottom,
            },
            DocPoint { x: left, y: bottom },
        ]
        .map(|point| {
            self.view
                .to_view(point, self.doc.width as f32, self.doc.height as f32)
        });
        let min_x = points
            .iter()
            .map(|point| point.x)
            .fold(f32::INFINITY, f32::min);
        let max_x = points
            .iter()
            .map(|point| point.x)
            .fold(f32::NEG_INFINITY, f32::max);
        let min_y = points
            .iter()
            .map(|point| point.y)
            .fold(f32::INFINITY, f32::min);
        let max_y = points
            .iter()
            .map(|point| point.y)
            .fold(f32::NEG_INFINITY, f32::max);
        let available = self.bounds.get().size;
        let (available_width, available_height): (f32, f32) =
            (available.width.into(), available.height.into());
        let mut base = self.view;
        base.zoom = 1.;
        let base_scale = base.scale(
            (available_width, available_height),
            (self.doc.width, self.doc.height),
        );
        let scale = ((available_width - 96.).max(1.) / (max_x - min_x).max(1.))
            .min((available_height - 96.).max(1.) / (max_y - min_y).max(1.));
        self.view.zoom = (scale / base_scale.max(0.001)).clamp(0.2, 8.);
        let actual_scale = self.view.scale(
            (available_width, available_height),
            (self.doc.width, self.doc.height),
        );
        let (view_width, view_height) = self.view.dimensions(self.doc.width, self.doc.height);
        self.view.pan = DocPoint {
            x: (view_width / 2. - (min_x + max_x) / 2.) * actual_scale,
            y: (view_height / 2. - (min_y + max_y) / 2.) * actual_scale,
        };
        self.view_changed();
        cx.notify();
    }

    pub fn selected_move_range(&self) -> Option<(usize, usize)> {
        if self.selected_layers.is_empty() {
            return None;
        }
        let start = *self.selected_layers.iter().min()?;
        let end = self.selected_layers.iter().max()?.checked_add(1)?;
        if end - start != self.selected_layers.len() || end > self.doc.layers.len() {
            return None;
        }
        let groups: std::collections::HashSet<_> = self.doc.layers[start..end]
            .iter()
            .filter_map(|layer| layer.group.as_ref())
            .collect();
        for group in groups {
            let all = self
                .doc
                .layers
                .iter()
                .enumerate()
                .filter(|(_, layer)| layer.group.as_ref() == Some(group))
                .all(|(index, _)| (start..end).contains(&index));
            if !all
                && self.doc.layers[start..end]
                    .iter()
                    .any(|layer| layer.group.as_ref() != Some(group))
            {
                return None;
            }
        }
        Some((start, end))
    }

    pub fn delete_layer(&mut self) {
        let selected: std::collections::BTreeSet<_> = self
            .selected_layers
            .iter()
            .copied()
            .filter(|index| *index < self.doc.layers.len())
            .collect();
        if selected.is_empty() {
            return;
        }
        if self.selection_is_locked() {
            self.message("Unlock selected layers before deleting them", false);
            return;
        }
        self.checkpoint();
        if selected.len() == self.doc.layers.len() {
            self.doc.layers = vec![Layer {
                name: "Paint".into(),
                visible: true,
                locked: false,
                opacity: 1.,
                group: None,
                candidate: false,
                strokes: vec![],
                image: None,
            }];
            self.doc.paint_layer = Some(0);
            self.doc.active_layer = 0;
            self.doc.prune_layer_groups();
            self.selected_layers = vec![0];
            self.layer_selection_anchor = Some(0);
            self.changed();
            return;
        }
        let start = *selected.first().unwrap();
        let mut index = 0;
        self.doc.layers.retain(|_| {
            let keep = !selected.contains(&index);
            index += 1;
            keep
        });
        self.doc.paint_layer = self.doc.paint_layer.and_then(|index| {
            (!selected.contains(&index)).then(|| index - selected.range(..index).count())
        });
        self.doc.prune_layer_groups();
        self.doc.active_layer = start.min(self.doc.layers.len() - 1);
        self.selected_layers = vec![self.doc.active_layer];
        self.layer_selection_anchor = Some(self.doc.active_layer);
        self.changed();
    }
    pub fn move_layer(&mut self, up: bool) {
        let Some((start, end)) = self.selected_move_range() else {
            return;
        };
        if self.selection_is_locked() {
            self.message("Unlock selected layers before reordering them", false);
            return;
        }
        let neighbor = if up {
            end
        } else if start > 0 {
            start - 1
        } else {
            return;
        };
        if neighbor >= self.doc.layers.len() {
            return;
        }
        let own_group = self.doc.layers[start].group.clone();
        let neighbor_group = self.doc.layers[neighbor].group.clone();
        let partial_group = own_group.as_ref().is_some_and(|group| {
            self.doc.layers.iter().enumerate().any(|(index, layer)| {
                layer.group.as_ref() == Some(group) && !(start..end).contains(&index)
            })
        });
        if partial_group && own_group != neighbor_group {
            self.message(
                "Ungroup the layer before moving it outside its group",
                false,
            );
            return;
        }
        let (mut other_start, mut other_end) = (neighbor, neighbor + 1);
        if let Some(group) = &neighbor_group {
            if own_group.as_ref() != Some(group) {
                while other_start > 0
                    && self.doc.layers[other_start - 1].group.as_ref() == Some(group)
                {
                    other_start -= 1;
                }
                while other_end < self.doc.layers.len()
                    && self.doc.layers[other_end].group.as_ref() == Some(group)
                {
                    other_end += 1;
                }
                if self
                    .doc
                    .layers
                    .iter()
                    .filter(|layer| layer.group.as_ref() == Some(group))
                    .count()
                    != other_end - other_start
                {
                    self.message("Ungroup the non-adjacent layers before reordering", false);
                    return;
                }
            }
        }
        let active_offset = self.doc.active_layer.saturating_sub(start);
        let paint_layer = self.doc.paint_layer;
        self.checkpoint();
        let new_start = if up {
            self.doc.layers[start..other_end].rotate_left(end - start);
            other_end - (end - start)
        } else {
            self.doc.layers[other_start..end].rotate_right(end - start);
            other_start
        };
        self.selected_layers = (new_start..new_start + end - start).collect();
        self.doc.active_layer = new_start + active_offset.min(end - start - 1);
        self.layer_selection_anchor = Some(self.doc.active_layer);
        self.doc.paint_layer = paint_layer.map(|index| {
            if (start..end).contains(&index) {
                new_start + index - start
            } else if up && (end..other_end).contains(&index) {
                index - (end - start)
            } else if !up && (other_start..start).contains(&index) {
                index + (end - start)
            } else {
                index
            }
        });
        self.changed();
    }

    pub fn drop_layer_rows(
        &mut self,
        drag: &crate::ui::DraggedLayerRows,
        target: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if drag.epoch != self.epoch
            || drag.artboard != self.doc.canvas_id
            || drag.revision != self.revision
        {
            self.message("Layers changed while dragging; try again", false);
            return;
        }
        let (start, end) = (drag.start, drag.end);
        if start >= end || end > self.doc.layers.len() || target >= self.doc.layers.len() {
            self.message("Select adjacent layers before dragging", false);
            return;
        }
        if (start..end).contains(&target) {
            return;
        }
        if self.doc.layers[start..end]
            .iter()
            .any(|layer| self.doc.layer_is_locked(layer))
        {
            self.message("Unlock the dragged layers before reordering", false);
            return;
        }
        let groups: std::collections::HashSet<_> = self.doc.layers[start..end]
            .iter()
            .filter_map(|layer| layer.group.clone())
            .collect();
        let single_group = if groups.len() == 1
            && self.doc.layers[start..end]
                .iter()
                .all(|layer| layer.group.is_some())
        {
            groups.iter().next().cloned()
        } else {
            None
        };
        let target_group = self.doc.layers[target].group.clone();
        for group in &groups {
            let all = self
                .doc
                .layers
                .iter()
                .enumerate()
                .filter(|(_, layer)| layer.group.as_ref() == Some(group))
                .all(|(index, _)| (start..end).contains(&index));
            if !all
                && (single_group.as_ref() != Some(group) || target_group.as_ref() != Some(group))
            {
                self.message(
                    "Move grouped children within their group, or remove them from the group first",
                    false,
                );
                return;
            }
        }
        let mut insert = target + 1;
        if let Some(name) = target_group
            .as_ref()
            .filter(|_| target_group != single_group)
        {
            if !self.doc.group_is_contiguous(name) {
                self.message("Ungroup the non-adjacent layers before reordering", false);
                return;
            }
            let group_top = self
                .doc
                .layers
                .iter()
                .rposition(|layer| layer.group.as_ref() == Some(name))
                .unwrap();
            if target != group_top {
                self.message(
                    "Drop on the group row to place the selection above the group",
                    false,
                );
                return;
            }
            insert = group_top + 1;
        }
        if insert >= start && insert <= end {
            return;
        }
        let old_active = self.doc.active_layer;
        let old_paint = self.doc.paint_layer;
        self.checkpoint();
        let count = end - start;
        let new_start = if insert > end {
            self.doc.layers[start..insert].rotate_left(count);
            insert - count
        } else {
            self.doc.layers[insert..end].rotate_right(count);
            insert
        };
        self.doc.paint_layer = old_paint.map(|index| {
            if (start..end).contains(&index) {
                new_start + index - start
            } else if insert > end && (end..insert).contains(&index) {
                index - count
            } else if insert < start && (insert..start).contains(&index) {
                index + count
            } else {
                index
            }
        });
        self.doc.active_layer = if (start..end).contains(&old_active) {
            new_start + old_active - start
        } else {
            new_start + count - 1
        };
        self.selected_layers = (new_start..new_start + count).collect();
        self.layer_selection_anchor = Some(self.doc.active_layer);
        if self.editing_group_name() != single_group.as_deref() {
            self.editing_layer_group = None;
        }
        let name = self.doc.layers[self.doc.active_layer].name.clone();
        self.layer_name
            .update(cx, |state, cx| state.set_value(name, window, cx));
        self.changed();
        self.sync_layer_controls(window, cx);
        cx.notify();
    }

    pub fn move_selection_to_edge(
        &mut self,
        front: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some((start, end)) = self.selected_move_range() else {
            self.message("Select adjacent layers to change their stacking", false);
            return;
        };
        if self.selection_is_locked() {
            self.message("Unlock selected layers before reordering", false);
            return;
        }
        let group = self.doc.layers[start].group.clone();
        let partial_group = group.as_ref().is_some_and(|name| {
            self.doc.layers.iter().enumerate().any(|(index, layer)| {
                layer.group.as_ref() == Some(name) && !(start..end).contains(&index)
            })
        });
        let (lower, upper) = if partial_group {
            let group = group.as_ref().unwrap();
            if !self.doc.group_is_contiguous(group) {
                self.message("Ungroup the non-adjacent layers before reordering", false);
                return;
            }
            (
                self.doc
                    .layers
                    .iter()
                    .position(|layer| layer.group.as_ref() == Some(group))
                    .unwrap(),
                self.doc
                    .layers
                    .iter()
                    .rposition(|layer| layer.group.as_ref() == Some(group))
                    .unwrap()
                    + 1,
            )
        } else {
            (0, self.doc.layers.len())
        };
        if (front && end == upper) || (!front && start == lower) {
            return;
        }
        let old_active = self.doc.active_layer;
        let old_paint = self.doc.paint_layer;
        let count = end - start;
        self.checkpoint();
        let new_start = if front {
            self.doc.layers[start..upper].rotate_left(count);
            upper - count
        } else {
            self.doc.layers[lower..end].rotate_right(count);
            lower
        };
        self.doc.paint_layer = old_paint.map(|index| {
            if (start..end).contains(&index) {
                new_start + index - start
            } else if front && (end..upper).contains(&index) {
                index - count
            } else if !front && (lower..start).contains(&index) {
                index + count
            } else {
                index
            }
        });
        self.doc.active_layer = if (start..end).contains(&old_active) {
            new_start + old_active - start
        } else {
            new_start + count - 1
        };
        self.selected_layers = (new_start..new_start + count).collect();
        self.layer_selection_anchor = Some(self.doc.active_layer);
        self.changed();
        self.sync_layer_controls(window, cx);
        cx.notify();
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
    fn artboard_resize_handle(&self, position: gpui_kit::Point<Pixels>) -> Option<(i8, i8)> {
        if self.view.turns != 0 || self.view.mirror {
            return None;
        }
        let paper = self.paper_bounds();
        let centers = [
            (-1, -1, paper.origin),
            (
                0,
                -1,
                point(paper.origin.x + paper.size.width / 2., paper.origin.y),
            ),
            (
                1,
                -1,
                point(paper.origin.x + paper.size.width, paper.origin.y),
            ),
            (
                -1,
                0,
                point(paper.origin.x, paper.origin.y + paper.size.height / 2.),
            ),
            (
                1,
                0,
                point(
                    paper.origin.x + paper.size.width,
                    paper.origin.y + paper.size.height / 2.,
                ),
            ),
            (
                -1,
                1,
                point(paper.origin.x, paper.origin.y + paper.size.height),
            ),
            (
                0,
                1,
                point(
                    paper.origin.x + paper.size.width / 2.,
                    paper.origin.y + paper.size.height,
                ),
            ),
            (
                1,
                1,
                point(
                    paper.origin.x + paper.size.width,
                    paper.origin.y + paper.size.height,
                ),
            ),
        ];
        centers.into_iter().find_map(|(x, y, center)| {
            (f32::from(position.x - center.x).abs() <= 10.
                && f32::from(position.y - center.y).abs() <= 10.)
                .then_some((x, y))
        })
    }
    pub fn doc_point(&self, p: gpui_kit::Point<Pixels>) -> Option<DocPoint> {
        let bounds = self.paper_bounds();
        if !bounds.contains(&p) {
            return None;
        }
        Some(self.unbounded_doc_point(p))
    }
    fn unbounded_doc_point(&self, p: gpui_kit::Point<Pixels>) -> DocPoint {
        let bounds = self.paper_bounds();
        let scale = self.view.scale(
            (
                self.bounds.get().size.width.into(),
                self.bounds.get().size.height.into(),
            ),
            (self.doc.width, self.doc.height),
        );
        self.view.to_document(
            DocPoint {
                x: f32::from(p.x - bounds.origin.x) / scale,
                y: f32::from(p.y - bounds.origin.y) / scale,
            },
            self.doc.width as f32,
            self.doc.height as f32,
        )
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
    fn begin_layer_marquee(
        &mut self,
        point: DocPoint,
        additive: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.marquee_initial_selection = self.selected_layers.clone();
        self.marquee_additive = additive;
        self.marquee_bounds = self.doc.selectable_layer_bounds();
        if let Some(group) = self.editing_group_name().map(str::to_owned) {
            self.marquee_bounds.retain(|(index, _)| {
                self.doc.layers[*index].group.as_deref() == Some(group.as_str())
            });
        }
        if !additive {
            self.selected_layers.clear();
        }
        self.selection_marquee = Some((point, point));
        self.sync_layer_controls(window, cx);
        cx.notify();
    }
    pub fn pointer_down(
        &mut self,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.closing || self.switching || self.closing_project.is_some() {
            return;
        }
        window.focus(&self.focus, cx);
        self.finish_stroke();
        self.tablet.drain();
        if self.image_crop.is_some() && event.button == MouseButton::Left {
            if let Some(point) = self.doc_point(event.position) {
                let source = self
                    .image_crop
                    .as_ref()
                    .and_then(|crop| crop.source_point(point));
                if let Some(crop) = &mut self.image_crop {
                    crop.anchor = source;
                }
            }
            cx.notify();
            return;
        }
        if self.panel == crate::app::Panel::Canvas && event.button == MouseButton::Left {
            if let Some(handle) = self.artboard_resize_handle(event.position) {
                let viewport = self.bounds.get();
                let scale = self.view.scale(
                    (viewport.size.width.into(), viewport.size.height.into()),
                    (self.doc.width, self.doc.height),
                );
                self.history.checkpoint(&self.doc);
                self.artboard_resize_drag = Some(ArtboardResizeDrag {
                    start: event.position,
                    original: self.doc.clone(),
                    handle,
                    scale,
                });
                return;
            }
            return;
        }
        if self.selecting_generation_region && event.button == MouseButton::Left {
            if let Some(point) = self.doc_point(event.position) {
                self.region_drag = Some(point);
                self.generation_region = Some((point, point));
                cx.notify();
            }
            return;
        }
        if self.tool == Tool::Hand && self.moving_image && event.button == MouseButton::Left {
            {
                let point = self.unbounded_doc_point(event.position);
                let scale = self.view.scale(
                    (
                        self.bounds.get().size.width.into(),
                        self.bounds.get().size.height.into(),
                    ),
                    (self.doc.width, self.doc.height),
                );
                let handle = self
                    .doc
                    .selected_layer_bounds(&self.selected_layers)
                    .and_then(|(left, top, right, bottom)| {
                        [
                            (-1, -1),
                            (0, -1),
                            (1, -1),
                            (-1, 0),
                            (1, 0),
                            (-1, 1),
                            (0, 1),
                            (1, 1),
                        ]
                        .into_iter()
                        .map(|(hx, hy)| {
                            let x = match hx {
                                -1 => left,
                                1 => right,
                                _ => (left + right) / 2.,
                            };
                            let y = match hy {
                                -1 => top,
                                1 => bottom,
                                _ => (top + bottom) / 2.,
                            };
                            ((hx, hy), (point.x - x).hypot(point.y - y) * scale)
                        })
                        .filter(|(_, distance)| *distance < 10.)
                        .min_by(|a, b| a.1.total_cmp(&b.1))
                        .map(|(handle, _)| handle)
                    });
                let rotate_handle = self
                    .doc
                    .selected_layer_bounds(&self.selected_layers)
                    .is_some_and(|(left, top, right, _)| {
                        (point.x - (left + right) / 2.)
                            .hypot(point.y - (top - 28. / scale.max(0.001)))
                            * scale
                            < 12.
                    });
                if handle.is_none() && !rotate_handle && event.modifiers.alt {
                    self.begin_layer_marquee(point, event.modifiers.shift, window, cx);
                    return;
                }
                if handle.is_none() && !rotate_handle {
                    let cycling = event.modifiers.platform || event.modifiers.control;
                    let after = (cycling && self.selected_layers.contains(&self.doc.active_layer))
                        .then_some(self.doc.active_layer);
                    let skip = if cycling && self.selected_group_name().is_some() {
                        self.selected_layers.as_slice()
                    } else {
                        &[]
                    };
                    let hit = self
                        .doc
                        .layer_at_point(point, 6. / scale.max(0.001), after, skip)
                        .or_else(|| {
                            (!skip.is_empty())
                                .then(|| {
                                    self.doc.layer_at_point(
                                        point,
                                        6. / scale.max(0.001),
                                        after,
                                        &[],
                                    )
                                })
                                .flatten()
                        });
                    if let Some(index) = hit {
                        let group = self.doc.layers[index].group.clone();
                        if cycling {
                            if let Some(group) = group
                                .filter(|name| self.editing_group_name() != Some(name.as_str()))
                            {
                                self.select_layer_group(&group, false, window, cx);
                            } else {
                                self.select_layer(index, window, cx);
                            }
                            return;
                        }
                        if let Some(group) = group.filter(|name| {
                            event.click_count < 2
                                && self.editing_group_name() != Some(name.as_str())
                        }) {
                            if event.modifiers.shift {
                                self.select_layer_group(&group, true, window, cx);
                                return;
                            }
                            let all_selected = self
                                .doc
                                .layers
                                .iter()
                                .enumerate()
                                .filter(|(_, layer)| layer.group.as_deref() == Some(group.as_str()))
                                .all(|(member, _)| self.selected_layers.contains(&member));
                            if !all_selected {
                                self.select_layer_group(&group, false, window, cx);
                            }
                        } else if event.modifiers.shift {
                            self.toggle_layer_selection(index, window, cx);
                            return;
                        } else if event.click_count >= 2 || !self.selected_layers.contains(&index) {
                            self.select_layer(index, window, cx);
                        }
                    } else {
                        self.begin_layer_marquee(point, event.modifiers.shift, window, cx);
                        return;
                    }
                }
                if self.selection_is_locked() {
                    self.message("Unlock selected layers before transforming them", false);
                    return;
                }
                self.resize_handle = handle;
                self.rotating_layer = rotate_handle;
                self.last_image_point = Some(point);
                self.transform_original = Some(self.doc.clone());
                if self.snap_enabled && self.resize_handle.is_none() && !self.rotating_layer {
                    self.prepare_transform_snap_targets();
                }
            }
            return;
        }
        if self.tool == Tool::Hand || event.button == MouseButton::Middle {
            self.last_pan = Some(event.position);
            return;
        }
        if let Some(p) = self.doc_point(event.position) {
            let active = self.doc.active_layer;
            let target = self
                .doc
                .paint_layer
                .filter(|index| *index < self.doc.layers.len())
                .or_else(|| {
                    (self.doc.layers[active].image.is_none() && !self.doc.layers[active].candidate)
                        .then_some(active)
                });
            if let Some(index) = target {
                let layer = &self.doc.layers[index];
                if layer.candidate {
                    self.message("Accept the paint target candidate before drawing", false);
                    return;
                }
                if self.doc.layer_is_locked(layer) {
                    self.message("Unlock the paint target before drawing", false);
                    return;
                }
                if !self.doc.layer_is_visible(layer) {
                    self.message("Show the paint target before drawing", false);
                    return;
                }
            } else {
                if self.doc.layers.len() >= 24 {
                    self.message("Maximum 24 layers; choose a paint target in Layers", true);
                    return;
                }
                let group = self.doc.layers[active]
                    .group
                    .as_ref()
                    .and_then(|name| self.doc.layer_groups.get(name));
                if group.is_some_and(|group| group.locked || !group.visible || group.opacity <= 0.)
                {
                    self.message("Show and unlock the group before painting in it", false);
                    return;
                }
            }
            self.history.checkpoint(&self.doc);
            let target = if let Some(index) = target {
                index
            } else {
                let index = active + 1;
                let group = self.doc.layers[active].group.clone();
                let group_selected = group.as_ref().is_some_and(|name| {
                    self.doc
                        .layers
                        .iter()
                        .enumerate()
                        .filter(|(_, layer)| layer.group.as_ref() == Some(name))
                        .all(|(member, _)| self.selected_layers.contains(&member))
                });
                self.doc.layers.insert(
                    index,
                    Layer {
                        name: "Paint".into(),
                        visible: true,
                        locked: false,
                        opacity: 1.,
                        group,
                        candidate: false,
                        strokes: vec![],
                        image: None,
                    },
                );
                for selected in &mut self.selected_layers {
                    if *selected >= index {
                        *selected += 1;
                    }
                }
                if group_selected {
                    self.selected_layers.push(index);
                    self.selected_layers.sort_unstable();
                }
                if let Some(anchor) = &mut self.layer_selection_anchor {
                    if *anchor >= index {
                        *anchor += 1;
                    }
                }
                self.message("Painting on a new layer above the image", false);
                index
            };
            self.doc.paint_layer = Some(target);
            let brush_size = self.size();
            self.doc.layers[target].strokes.push(Stroke {
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
    pub fn select_context_target(
        &mut self,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.tool != Tool::Hand
            || !self.moving_image
            || self.image_crop.is_some()
            || self.selecting_generation_region
        {
            return;
        }
        let point = self.unbounded_doc_point(event.position);
        let scale = self.view.scale(
            (
                self.bounds.get().size.width.into(),
                self.bounds.get().size.height.into(),
            ),
            (self.doc.width, self.doc.height),
        );
        match self
            .doc
            .layer_at_point(point, 6. / scale.max(0.001), None, &[])
        {
            Some(index) if self.selected_layers.contains(&index) => {}
            Some(index) => {
                let group = self.doc.layers[index].group.clone();
                if let Some(group) =
                    group.filter(|name| self.editing_group_name() != Some(name.as_str()))
                {
                    self.select_layer_group(&group, false, window, cx);
                } else {
                    self.select_layer(index, window, cx);
                }
            }
            None => {
                self.selected_layers.clear();
                self.sync_layer_controls(window, cx);
                cx.notify();
            }
        }
    }
    pub fn pointer_move(&mut self, event: &MouseMoveEvent, cx: &mut Context<Self>) {
        if let Some(crop) = &self.image_crop {
            if let Some(anchor) = crop.anchor {
                let end = crop.source_point(self.clamped_doc_point(event.position));
                if let Some(end) = end {
                    let left = anchor
                        .x
                        .min(end.x)
                        .floor()
                        .clamp(0., (crop.width - 1) as f32) as u32;
                    let top = anchor
                        .y
                        .min(end.y)
                        .floor()
                        .clamp(0., (crop.height - 1) as f32) as u32;
                    let right = (anchor.x.max(end.x).ceil() as u32).clamp(left + 1, crop.width);
                    let bottom = (anchor.y.max(end.y).ceil() as u32).clamp(top + 1, crop.height);
                    if let Some(crop) = &mut self.image_crop {
                        crop.draft = [left, top, right - left, bottom - top];
                    }
                    cx.notify();
                }
            }
            return;
        }
        if let Some((anchor, _)) = self.selection_marquee {
            let end = self.clamped_doc_point(event.position);
            self.selection_marquee = Some((anchor, end));
            let mut selected = if self.marquee_additive {
                self.marquee_initial_selection.clone()
            } else {
                Vec::new()
            };
            let (left, right) = (anchor.x.min(end.x), anchor.x.max(end.x));
            let (top, bottom) = (anchor.y.min(end.y), anchor.y.max(end.y));
            for &(index, (x0, y0, x1, y1)) in &self.marquee_bounds {
                if x0 <= right
                    && x1 >= left
                    && y0 <= bottom
                    && y1 >= top
                    && !selected.contains(&index)
                {
                    selected.push(index);
                }
            }
            let groups: std::collections::HashSet<_> = selected
                .iter()
                .filter_map(|index| self.doc.layers.get(*index)?.group.clone())
                .filter(|group| self.editing_group_name() != Some(group.as_str()))
                .collect();
            for (index, layer) in self.doc.layers.iter().enumerate() {
                if layer
                    .group
                    .as_ref()
                    .is_some_and(|group| groups.contains(group))
                    && !selected.contains(&index)
                {
                    selected.push(index);
                }
            }
            selected.sort_unstable();
            self.selected_layers = selected;
            cx.notify();
            return;
        }
        if let Some(drag) = &self.artboard_resize_drag {
            let (dx, dy) = (
                f32::from(event.position.x - drag.start.x) / drag.scale,
                f32::from(event.position.y - drag.start.y) / drag.scale,
            );
            let (x_handle, y_handle) = drag.handle;
            let mut width = if x_handle == 0 {
                drag.original.width
            } else {
                (drag.original.width as f32 + dx * x_handle as f32)
                    .round()
                    .clamp(64., document::MAX_EDGE as f32) as u32
            };
            let mut height = if y_handle == 0 {
                drag.original.height
            } else {
                (drag.original.height as f32 + dy * y_handle as f32)
                    .round()
                    .clamp(64., document::MAX_EDGE as f32) as u32
            };
            if self.aspect_locked {
                let ratio = drag.original.width as f32 / drag.original.height as f32;
                if x_handle != 0 {
                    height = (width as f32 / ratio).round().max(64.) as u32;
                } else {
                    width = (height as f32 * ratio).round().max(64.) as u32;
                }
                width = width.min(document::MAX_EDGE);
                height = height.min(document::MAX_EDGE);
            }
            if width as u64 * height as u64 <= document::MAX_PIXELS {
                let anchor = (
                    if x_handle < 0 {
                        2
                    } else if x_handle > 0 {
                        0
                    } else {
                        1
                    },
                    if y_handle < 0 {
                        2
                    } else if y_handle > 0 {
                        0
                    } else {
                        1
                    },
                );
                self.doc = drag.original.clone();
                if self.doc.resize(width, height, false, anchor).is_ok() {
                    self.changed();
                    cx.notify();
                }
            }
            return;
        }
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
        if let Some(start) = self.last_image_point {
            if event.dragging() {
                {
                    let point = self.unbounded_doc_point(event.position);
                    let Some((left, top, right, bottom)) = self
                        .transform_original
                        .as_ref()
                        .and_then(|original| original.selected_layer_bounds(&self.selected_layers))
                    else {
                        return;
                    };
                    let Some((current_left, current_top, current_right, current_bottom)) =
                        self.doc.selected_layer_bounds(&self.selected_layers)
                    else {
                        return;
                    };
                    let result = if self.rotating_layer {
                        let pivot = DocPoint {
                            x: (left + right) / 2.,
                            y: (top + bottom) / 2.,
                        };
                        let start_angle = (start.y - pivot.y).atan2(start.x - pivot.x);
                        let mut angle = (point.y - pivot.y).atan2(point.x - pivot.x) - start_angle;
                        if event.modifiers.shift {
                            let step = std::f32::consts::PI / 12.;
                            angle = (angle / step).round() * step;
                        }
                        if angle.abs() < 0.0001 && !self.transform_checkpointed {
                            return;
                        }
                        let mut next = self.transform_original.as_ref().unwrap().clone();
                        match next.rotate_layers(&self.selected_layers, pivot, angle) {
                            Ok(()) => {
                                for &index in &self.selected_layers {
                                    if let (Some(current), Some(rotated)) =
                                        (self.doc.layers.get_mut(index), next.layers.get(index))
                                    {
                                        *current = rotated.clone();
                                    }
                                }
                                self.status = format!(
                                    "Rotation: {:.1}° · Shift snaps to 15°",
                                    angle.to_degrees()
                                );
                                Ok(())
                            }
                            Err(error) => Err(error),
                        }
                    } else if let Some((hx, hy)) = self.resize_handle {
                        let original_width = right - left;
                        let original_height = bottom - top;
                        if original_width <= 0. || original_height <= 0. {
                            return;
                        }
                        let multiplier = if event.modifiers.alt { 2. } else { 1. };
                        let mut width = if hx == 0 {
                            original_width
                        } else {
                            (original_width + (point.x - start.x) * hx as f32 * multiplier)
                                .max(0.01)
                        };
                        let mut height = if hy == 0 {
                            original_height
                        } else {
                            (original_height + (point.y - start.y) * hy as f32 * multiplier)
                                .max(0.01)
                        };
                        let keep_aspect = self.layer_aspect_locked != event.modifiers.shift;
                        if keep_aspect {
                            let sx = width / original_width;
                            let sy = height / original_height;
                            let factor = if hx == 0 {
                                sy
                            } else if hy == 0 || (sx - 1.).abs() >= (sy - 1.).abs() {
                                sx
                            } else {
                                sy
                            };
                            width = original_width * factor;
                            height = original_height * factor;
                        }
                        if self.snap_enabled {
                            if hx != 0 {
                                width = (width / 16.).round().max(1.) * 16.;
                                if keep_aspect {
                                    height = width * original_height / original_width;
                                }
                            }
                            if hy != 0 && (!keep_aspect || hx == 0) {
                                height = (height / 16.).round().max(1.) * 16.;
                                if keep_aspect {
                                    width = height * original_width / original_height;
                                }
                            }
                        }
                        let x = if event.modifiers.alt || hx == 0 {
                            (left + right - width) / 2.
                        } else if hx < 0 {
                            right - width
                        } else {
                            left
                        };
                        let y = if event.modifiers.alt || hy == 0 {
                            (top + bottom - height) / 2.
                        } else if hy < 0 {
                            bottom - height
                        } else {
                            top
                        };
                        if (x - current_left).abs() < 0.001
                            && (y - current_top).abs() < 0.001
                            && (width - (current_right - current_left)).abs() < 0.001
                            && (height - (current_bottom - current_top)).abs() < 0.001
                        {
                            return;
                        }
                        let mut next = self.transform_original.as_ref().unwrap().clone();
                        match next.transform_layers_to_bounds(
                            &self.selected_layers,
                            (x, y, width, height),
                        ) {
                            Ok(()) => {
                                for &index in &self.selected_layers {
                                    if let (Some(current), Some(resized)) =
                                        (self.doc.layers.get_mut(index), next.layers.get(index))
                                    {
                                        *current = resized.clone();
                                    }
                                }
                                self.status = format!(
                                    "Size: {:.1} × {:.1} px · Shift changes aspect lock · Option/Alt resizes from center",
                                    width, height
                                );
                                Ok(())
                            }
                            Err(error) => Err(error),
                        }
                    } else {
                        let (proposed_x, proposed_y) =
                            (left + point.x - start.x, top + point.y - start.y);
                        let (target_x, target_y) = if self.snap_enabled {
                            let scale = self.view.scale(
                                (
                                    self.bounds.get().size.width.into(),
                                    self.bounds.get().size.height.into(),
                                ),
                                (self.doc.width, self.doc.height),
                            );
                            let tolerance = 6. / scale.max(0.001);
                            let (x, guide_x) = Self::snap_axis(
                                proposed_x,
                                right - left,
                                &self.transform_snap_x,
                                tolerance,
                            );
                            let (y, guide_y) = Self::snap_axis(
                                proposed_y,
                                bottom - top,
                                &self.transform_snap_y,
                                tolerance,
                            );
                            self.transform_guides.clear();
                            if let Some(value) = guide_x {
                                self.transform_guides.push((true, value));
                            }
                            if let Some(value) = guide_y {
                                self.transform_guides.push((false, value));
                            }
                            (x, y)
                        } else {
                            self.transform_guides.clear();
                            (proposed_x, proposed_y)
                        };
                        let dx = target_x - current_left;
                        let dy = target_y - current_top;
                        if dx.abs() < 0.001 && dy.abs() < 0.001 {
                            cx.notify();
                            return;
                        }
                        self.status = format!("Position: {:.1}, {:.1} px", target_x, target_y);
                        self.doc.translate_layers(&self.selected_layers, dx, dy)
                    };
                    match result {
                        Ok(()) => {
                            self.transform_checkpointed = true;
                            self.changed();
                        }
                        Err(error) => self.message(error.to_string(), true),
                    }
                }
            } else {
                self.finish_stroke();
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
                let Some(stroke) = self
                    .doc
                    .paint_layer
                    .and_then(|index| self.doc.layers.get_mut(index))
                    .and_then(|layer| layer.strokes.last_mut())
                else {
                    self.finish_stroke();
                    return;
                };
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
        if let Ok(minutes) = self.request_timeout.read(cx).value().trim().parse::<u32>() {
            self.settings.request_timeout_minutes = minutes.clamp(1, 1440);
        }
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
        if let Ok(registry) = storyfella_studio_core::ai_config::user_or_built_in() {
            if let Some(endpoint) = providers::shared_endpoint(self.settings.provider, &registry) {
                self.settings.endpoint = endpoint.clone();
                self.endpoint
                    .update(cx, |s, cx| s.set_value(endpoint, window, cx));
            }
        }
        self.refresh_lora_select(window, cx);
        self.refresh_server_status();
        self.prompt_validation = None;
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
        self.prompt_validation = None;
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
        if self
            .workers
            .storage
            .send(Storage::ApplySettings(
                self.settings.clone(),
                self.data_dir.join("settings.json"),
            ))
            .is_err()
        {
            self.report_error(
                crate::activity::Scope::App,
                crate::activity::Task::Settings,
                "Storage worker stopped".into(),
            );
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
        self.prompt_validation = None;
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
        self.drawing_create_layer = false;
        let Some(target) = self.doc.artboard_prompt_targets.get(&self.doc.canvas_id) else {
            return;
        };
        let parts: Vec<_> = target.split(':').collect();
        if parts.len() == 3 && parts[2] == "layer" {
            self.drawing_create_layer = true;
            return;
        }
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
        if self.closing || self.switching || self.closing_project.is_some() {
            return;
        }
        self.ensure_project_tab();
        self.restore_prompt(window, cx);
        self.settings_from_inputs(cx);
        let source = if self.editing_image {
            if !self.settings.provider.supports_edit() {
                self.prompt_issue("Choose a hosted provider to edit an image");
                return;
            }
            match self.edit_source() {
                Some(source) => Some(source),
                None => {
                    self.prompt_issue("Select an image from your collection to edit");
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
                    self.prompt_issue(
                        "Open the selected image to paint its editing mask, or turn off Use mask.",
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
            self.prompt_issue(error.to_string());
            return;
        }
        let prompt = self.prompt.read(cx).value().to_string();
        if prompt.trim().is_empty() {
            self.prompt_issue(if self.editing_image {
                "Describe what you want to change"
            } else {
                "Describe the image you want"
            });
            return;
        }
        self.prompt_validation = None;

        let queued = self.queue.enqueue(
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
        self.queue
            .get_mut(queued)
            .expect("New request exists")
            .context = Some(self.prompt_target.clone());
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
        worker::generate(
            request.into_provider_request(self.workers.native.clone(), cancel.clone()),
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
        let activity = &self.activity;
        self.queue.clear_finished(|id| {
            activity
                .get(id)
                .is_some_and(|report| report.needs_attention())
        });
        self.activity.clear_completed(self.activity_selection);
        self.message("Cleared finished requests", false);
        cx.notify();
    }
    /// Abort only the selected request, leaving dispatch and other jobs intact.
    pub fn queue_abort(&mut self, id: u64, cx: &mut Context<Self>) {
        let running = self
            .queue
            .get(id)
            .is_some_and(|item| matches!(item.state, JobState::Running { .. }));
        if running && (id != self.job_id || self.generation.is_none()) {
            return;
        }
        if !self.queue.abort(id) {
            return;
        }
        if running {
            if let Some(cancel) = &self.generation {
                cancel.store(true, Ordering::Relaxed);
            }
            self.message("Abort requested · waiting for provider", false);
        } else {
            self.message("Queued request aborted", false);
        }
        cx.notify();
    }

    /// Re-queues a failed request with the same captured settings and source.
    pub fn queue_retry(&mut self, id: u64, cx: &mut Context<Self>) {
        let Some(item) = self.queue.get(id) else {
            return;
        };
        let JobState::Failed(report_id) = item.state else {
            return;
        };
        if !self.project_tab_order.contains(&item.epoch)
            || self
                .queue
                .items()
                .iter()
                .any(|item| item.retry_of == Some(report_id) && !item.state.is_finished())
        {
            return;
        }
        let request = item.request.clone();
        let epoch = item.epoch;
        let context = item.context.clone();
        let next = self.queue.enqueue(request, epoch);
        let item = self.queue.get_mut(next).expect("New retry exists");
        item.retry_of = Some(report_id);
        item.context = context;
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
        self.drawing_create_layer = false;
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
        self.drawing_create_layer = false;
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
            if self.drawing_create_layer {
                anyhow::ensure!(
                    self.doc.layers.len() < 24,
                    "Maximum 24 layers; remove a layer before generating"
                );
                let input = self.input_references();
                let source = input.first().cloned();
                let references = input.into_iter().skip(1).collect::<Vec<_>>();
                let editing = source.is_some();
                let settings = self.settings.clone();
                crate::providers::validate_inputs(&settings, editing, references.len(), false)?;
                return Ok(QueuedRequest {
                    settings,
                    prompt: prompt.clone(),
                    source,
                    references,
                    mask: None,
                    editing,
                    destination: crate::workflow::Destination::NewLayer {
                        artboard: self.doc.canvas_id,
                        rect: (0, 0, self.doc.width, self.doc.height),
                    },
                    blend: None,
                });
            }
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
                let destination = request.destination;
                self.changed();
                let id = self.queue.enqueue(request, self.epoch);
                self.queue.get_mut(id).expect("New request exists").context =
                    Some(self.prompt_target.clone());
                self.prompt_validation = None;
                self.pump_queue();
                self.message(
                    match destination {
                        crate::workflow::Destination::NewLayer { .. } => {
                            "Queued · new image will arrive as a candidate layer"
                        }
                        crate::workflow::Destination::Candidate { .. } => {
                            "Queued · original artwork preserved; edit arrives as a candidate"
                        }
                        crate::workflow::Destination::NewArtboard => {
                            "Queued · separate image will open as a new artboard"
                        }
                    },
                    false,
                );
            }
            Err(error) => {
                self.prompt_issue(error.to_string());
            }
        }
        cx.notify();
    }

    pub fn save(&mut self, save_as: bool, window: &mut Window, cx: &mut Context<Self>) {
        if self.closing_project.is_some() {
            return;
        }
        if self.project_tab_order.is_empty() {
            self.message("Open or create a project to save", false);
            return;
        }
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
    pub(crate) fn queue_save(&mut self, path: PathBuf, automatic: bool) {
        let failure_task = if automatic {
            crate::activity::Task::Recovery
        } else {
            crate::activity::Task::Save(path.clone())
        };
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
                owner: self.project_owner.clone(),
            })
            .is_err()
        {
            self.report_error(
                crate::activity::Scope::Project(self.epoch),
                failure_task,
                "Storage worker stopped".into(),
            );
            self.save_pending = false;
            self.manual_pending = false;
        }
    }
    pub fn pick(&mut self, picker: Picker, window: &mut Window, cx: &mut Context<Self>) {
        self.finish_stroke();
        let epoch = self.epoch;
        let export_scope = if self.page == Page::Library
            || (self.page == Page::Image && self.image_return_page == Page::Library)
        {
            crate::activity::Scope::Library
        } else {
            crate::activity::Scope::Project(epoch)
        };
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
                                                    export_scope.clone(),
                                                ));
                                            this.message(
                                                "Exporting approved canvas artwork…",
                                                false,
                                            );
                                        } else if let Some(reference) = &export_image {
                                            let _ = this.workers.storage.send(Storage::Artwork(
                                                reference.clone(),
                                                path,
                                                export_scope.clone(),
                                            ));
                                            this.message("Exporting approved artwork…", false);
                                        }
                                    } else if matches!(picker, Picker::Original) {
                                        if path.extension().is_none() {
                                            path.set_extension("png");
                                        }
                                        if let Some(reference) = &export_image {
                                            let _ = this.workers.storage.send(Storage::Original(
                                                reference.clone(),
                                                path,
                                                export_scope.clone(),
                                            ));
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
                                            export_scope.clone(),
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
                            Picker::Open => "Open Atelier project or Storyfella manifest",
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
        if !self.closing && self.restart_checked.elapsed() >= Duration::from_millis(250) {
            self.restart_checked = Instant::now();
            match storyfella_studio_core::atelier::Mailbox::new(&self.data_dir).take_restart() {
                Ok(true) => self.request_close(cx),
                Ok(false) => {}
                Err(error) => {
                    self.message(format!("Could not read restart request: {error:#}"), true)
                }
            }
        }
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
                Event::StoryContext(request, result) => {
                    self.receive_story_context(request, result, window, cx)
                }
                Event::CatalogRefreshed {
                    epoch,
                    path,
                    result,
                } => {
                    if self.catalog_refresh_pending.as_ref() == Some(&(epoch, path.clone())) {
                        self.catalog_refresh_pending = None;
                    }
                    if epoch == self.epoch && self.shared_catalog_path().as_ref() == Some(&path) {
                        match result {
                            Ok(snapshot) => {
                                self.catalog_refresh_error = None;
                                if let Some(catalog) =
                                    self.catalog.as_mut().filter(|c| c.path == path)
                                {
                                    if snapshot.registry_version
                                        >= catalog.snapshot.registry_version
                                    {
                                        catalog.snapshot = snapshot;
                                    }
                                }
                            }
                            Err(error) => {
                                self.catalog_refresh_error =
                                    Some(format!("Could not refresh assets: {error:#}"))
                            }
                        }
                    }
                }
                Event::CatalogSaved {
                    epoch,
                    path,
                    details_saved,
                    result,
                } => {
                    self.catalog_busy = false;
                    match result {
                        Ok((id, snapshot)) => {
                            if epoch == self.epoch
                                && self
                                    .catalog
                                    .as_ref()
                                    .is_some_and(|catalog| catalog.path == path)
                            {
                                if let Some(catalog) = &mut self.catalog {
                                    catalog.snapshot = snapshot;
                                }
                                self.catalog_error = None;
                                self.catalog_conflict = None;
                                if details_saved {
                                    self.select_catalog_item(id, window, cx);
                                }
                                self.message("Asset catalog saved", false);
                            }
                        }
                        Err(error) => {
                            if epoch == self.epoch {
                                self.catalog_conflict = error
                                    .downcast_ref::<storyfella_studio_core::catalog::EditConflict>()
                                    .cloned();
                                self.catalog_error = Some(format!("{error:#}"));
                            } else {
                                self.report_error(
                                    crate::activity::Scope::Project(epoch),
                                    crate::activity::Task::Save(path),
                                    format!("{error:#}"),
                                );
                            }
                        }
                    }
                }
                Event::WorkspaceSaved(result) => {
                    if let Err(error) = result {
                        self.closing = false;
                        self.report_error(
                            crate::activity::Scope::App,
                            crate::activity::Task::Recovery,
                            format!("Open-project recovery save failed: {error:#}"),
                        );
                    } else {
                        self.activity.resolve(
                            &crate::activity::Scope::App,
                            &crate::activity::Task::Recovery,
                        );
                    }
                }
                Event::ProjectClosed {
                    epoch,
                    revision,
                    publications,
                    result,
                } => {
                    self.apply_save_publications(epoch, &publications);
                    self.finish_project_close(epoch, revision, result, window, cx);
                }
                Event::ClipforgeOpened(result) => match result {
                    Ok(()) => {
                        self.activity.resolve(
                            &crate::activity::Scope::App,
                            &crate::activity::Task::Clipforge,
                        );
                        self.message("Clipforge opened · return to Settings and Reload providers when finished", false);
                    }
                    Err(error) => {
                        self.report_error(
                            crate::activity::Scope::App,
                            crate::activity::Task::Clipforge,
                            format!("Clipforge: {error:#}"),
                        );
                    }
                },
                Event::Settings(result) => match result {
                    Ok(()) => {
                        self.activity.resolve(
                            &crate::activity::Scope::App,
                            &crate::activity::Task::Settings,
                        );
                        self.message("Settings saved", false);
                    }
                    Err(e) => {
                        self.report_error(
                            crate::activity::Scope::App,
                            crate::activity::Task::Settings,
                            format!("Settings: {e:#}"),
                        );
                    }
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
                                self.activity.resolve(
                                    &crate::activity::Scope::App,
                                    &crate::activity::Task::Settings,
                                );
                            }
                        }
                        Err(error) => {
                            let message = format!("Could not save settings: {error:#}");
                            self.report_error(
                                crate::activity::Scope::App,
                                crate::activity::Task::Settings,
                                message,
                            );
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
                    publications,
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
                    self.apply_save_publications(epoch, &publications);
                    match result {
                        Ok(()) => {
                            let scope = crate::activity::Scope::Project(epoch);
                            let task = if automatic {
                                crate::activity::Task::Recovery
                            } else {
                                crate::activity::Task::Save(path.clone())
                            };
                            self.activity.resolve(&scope, &task);
                            if !automatic {
                                if let Some(tab) =
                                    self.project_tabs.iter_mut().find(|tab| tab.epoch == epoch)
                                {
                                    if tab.owner.is_none() || tab.path.as_ref() == Some(&path) {
                                        tab.path = Some(path.clone());
                                        if tab.revision == revision {
                                            tab.dirty = false;
                                        }
                                    }
                                }
                            }
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
                                    if self.project_owner.is_none()
                                        || self.path.as_ref() == Some(&path)
                                    {
                                        self.path = Some(path);
                                        if revision == self.revision {
                                            self.dirty = false;
                                        }
                                    }
                                    self.refresh_projects();
                                    self.message("Project saved", false);
                                }
                            }
                        }
                        Err(e) => {
                            self.closing = false;
                            self.autosave_retry_after = Instant::now() + Duration::from_secs(5);
                            let task = if automatic {
                                crate::activity::Task::Recovery
                            } else {
                                crate::activity::Task::Save(path)
                            };
                            self.report_error(
                                crate::activity::Scope::Project(epoch),
                                task,
                                format!("Save failed: {e:#}"),
                            );
                        }
                    }
                }
                Event::Exported(scope, result) => match result {
                    Ok(path) => {
                        self.activity
                            .resolve(&scope, &crate::activity::Task::Export);
                        self.message(
                            format!(
                                "Exported {}",
                                path.file_name().unwrap_or_default().to_string_lossy()
                            ),
                            false,
                        );
                    }
                    Err(e) => {
                        self.report_error(
                            scope,
                            crate::activity::Task::Export,
                            format!("Export failed: {e:#}"),
                        );
                    }
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
                        Ok(worker::OpenedDocument {
                            doc,
                            images,
                            path,
                            owner,
                        }) => {
                            self.activity.resolve(
                                &crate::activity::Scope::App,
                                &crate::activity::Task::Open,
                            );
                            self.finish_stroke();
                            let keep_previous = !self.project_tab_order.is_empty()
                                && (self.path.is_some()
                                    || !self.doc.references.is_empty()
                                    || self
                                        .doc
                                        .layers
                                        .iter()
                                        .any(|l| l.image.is_some() || !l.strokes.is_empty())
                                    || self
                                        .doc
                                        .prompt_drafts
                                        .values()
                                        .any(|draft| !draft.trim().is_empty()));
                            let previous = self.park_project();
                            if keep_previous {
                                self.project_tabs.push(previous);
                            } else {
                                self.project_tab_order.retain(|epoch| *epoch != self.epoch);
                            }
                            self.doc = doc;
                            self.next_project_epoch += 1;
                            self.epoch = self.next_project_epoch;
                            self.project_tab_order.push(self.epoch);
                            self.revision += 1;
                            self.visual_revision += 1;
                            self.dirty = path.is_none();
                            self.path = path;
                            self.project_owner = owner;
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
                        Err(e) => {
                            self.report_error(
                                crate::activity::Scope::App,
                                crate::activity::Task::Open,
                                format!("Could not open project: {e:#}"),
                            );
                        }
                    }
                }
                Event::Library(result) => {
                    self.library_loading = false;
                    match result {
                        Ok(items) => {
                            self.activity.resolve(
                                &crate::activity::Scope::Library,
                                &crate::activity::Task::Library,
                            );
                            self.library = items
                                .into_iter()
                                .map(|(reference, image)| {
                                    (Arc::new(reference), worker::render_image(image))
                                })
                                .collect();
                        }
                        Err(error) => {
                            let message = format!("Could not load library: {error:#}");
                            self.report_error(
                                crate::activity::Scope::Library,
                                crate::activity::Task::Library,
                                message,
                            );
                        }
                    }
                }
                Event::LibraryUpdated(result) => match result {
                    Ok(()) => self.refresh_library(),
                    Err(error) => {
                        self.report_error(
                            crate::activity::Scope::Library,
                            crate::activity::Task::Other,
                            format!("Library details were not saved: {error:#}"),
                        );
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
                        self.activity.resolve(
                            &crate::activity::Scope::Library,
                            &crate::activity::Task::Mask(id.clone()),
                        );
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
                        let message = format!(
                            "Mask save failed: {error:#}. Check disk space and retry saving the mask."
                        );
                        self.mask_save_error = Some(message.clone());
                        self.closing = false;
                        self.opening_image = None;
                        self.report_error(
                            crate::activity::Scope::Library,
                            crate::activity::Task::Mask(id),
                            message,
                        );
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
                Event::ArtworkImported {
                    epoch,
                    artboard,
                    rect,
                    result,
                } => {
                    match result {
                        Err(error) => {
                            self.report_error(
                                crate::activity::Scope::Project(epoch),
                                crate::activity::Task::Import,
                                format!("Artwork import failed: {error:#}"),
                            );
                        }
                        Ok(_) if !self.project_tab_order.contains(&epoch) => {
                            self.report_error(crate::activity::Scope::Project(epoch), crate::activity::Task::Import,
                                "The destination project was closed; reopen it and import the original files again.".into());
                        }
                        Ok(images) if epoch == self.epoch => {
                            self.finish_stroke();
                            let previous = self.doc.clone();
                            match self.doc.attach_imported_images(artboard, rect, images) {
                                Ok(indices) => {
                                    let count = indices.len();
                                    self.history.checkpoint(&previous);
                                    self.changed();
                                    self.activity.resolve(
                                        &crate::activity::Scope::Project(epoch),
                                        &crate::activity::Task::Import,
                                    );
                                    if self.doc.canvas_id == artboard && self.page == Page::Draw {
                                        self.message(
                                            format!("Imported {count} image layer(s) · select them in Layers"),
                                            false,
                                        );
                                    } else {
                                        self.message(format!("Imported {count} image layer(s) to the source artboard"), false);
                                    }
                                }
                                Err(error) => {
                                    self.report_error(crate::activity::Scope::Project(epoch), crate::activity::Task::Import,
                                    format!("Could not add imported images: {error:#}. Try importing again."));
                                }
                            }
                        }
                        Ok(images) => {
                            if let Some(tab) =
                                self.project_tabs.iter_mut().find(|tab| tab.epoch == epoch)
                            {
                                let previous = tab.doc.clone();
                                match tab.doc.attach_imported_images(artboard, rect, images) {
                                    Ok(_) => {
                                        tab.history.checkpoint(&previous);
                                        tab.revision += 1;
                                        tab.dirty = true;
                                        self.activity.resolve(
                                            &crate::activity::Scope::Project(epoch),
                                            &crate::activity::Task::Import,
                                        );
                                    }
                                    Err(error) => {
                                        self.report_error(crate::activity::Scope::Project(epoch), crate::activity::Task::Import,
                                        format!("Could not add imported images: {error:#}. Try importing again."));
                                    }
                                }
                                self.persist_open_projects();
                            }
                        }
                    }
                    cx.notify();
                }
                Event::Imported(epoch, into_project, result) => {
                    self.refresh_library();
                    if !into_project {
                        match result {
                            Ok(_) => {
                                self.activity.resolve(
                                    &crate::activity::Scope::Library,
                                    &crate::activity::Task::Import,
                                );
                                self.message("Images saved to the shared reference library", false)
                            }
                            Err(error) => {
                                self.report_error(
                                    crate::activity::Scope::Library,
                                    crate::activity::Task::Import,
                                    format!("Import failed: {error:#}"),
                                );
                            }
                        }
                        continue;
                    }
                    if epoch != self.epoch {
                        self.report_error(crate::activity::Scope::Project(epoch), crate::activity::Task::Import, "Imported originals are in Library; the destination project changed before they could be attached.".into());
                        continue;
                    }
                    match result {
                        Ok(refs) => {
                            self.activity.resolve(
                                &crate::activity::Scope::Project(epoch),
                                &crate::activity::Task::Import,
                            );
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
                        Err(e) => {
                            self.report_error(
                                crate::activity::Scope::Project(epoch),
                                crate::activity::Task::Import,
                                format!("Import failed: {e:#}"),
                            );
                        }
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
                            let native = self.queue.get(id).is_some_and(|item| {
                                item.request
                                    .settings
                                    .provider
                                    .native_architecture()
                                    .is_some()
                            });
                            if let Some(item) = self.queue.get_mut(id) {
                                item.state = JobState::Cancelled;
                            }
                            self.message(
                                if native {
                                    "Request aborted · native GPU work may finish on the server"
                                } else {
                                    "Request aborted · provider charges may still apply"
                                },
                                false,
                            );
                        } else {
                            match result {
                                Ok((reference, image)) => {
                                    if let Some(report_id) =
                                        self.queue.get(id).and_then(|item| item.retry_of)
                                    {
                                        if let Some(report) = self.activity.get_mut(report_id) {
                                            report.resolved = true;
                                        }
                                    }
                                    let result_id = crate::library::id(&reference);
                                    let request =
                                        self.queue.get(id).map(|item| item.request.clone());
                                    let reference = Arc::new(reference);
                                    // Keep every result recoverable in the queue, including attachment failures.
                                    if let Some(item) = self.queue.get_mut(id) {
                                        item.result = Some(reference.clone());
                                    }
                                    if self.job_epoch == self.epoch {
                                        if let Some(request) = request {
                                            let attached = request
                                                .blend
                                                .map(|(size, rect)| {
                                                    crate::workflow::crop_blend(
                                                        &reference, size, rect,
                                                    )
                                                    .map(Arc::new)
                                                })
                                                .unwrap_or_else(|| Ok(reference.clone()));
                                            self.finish_stroke();
                                            let previous = self.doc.clone();
                                            let previous_active = self.doc.active_layer;
                                            match attached.and_then(|r| {
                                                self.doc.attach_result(r, request.destination)
                                            }) {
                                                Ok(()) => {
                                                    // A delayed result must not steal the user's current selection.
                                                    let same_artboard =
                                                        self.doc.canvas_id == previous.canvas_id;
                                                    if same_artboard
                                                        && previous_active < self.doc.layers.len()
                                                    {
                                                        self.doc.active_layer = previous_active;
                                                    }
                                                    self.history.checkpoint(&previous);
                                                    self.changed();
                                                    if same_artboard {
                                                        self.sync_layer_controls(window, cx);
                                                    } else {
                                                        self.sync_inputs(window, cx);
                                                    }
                                                    if let crate::workflow::Destination::Candidate { artboard, .. }
                                                        | crate::workflow::Destination::NewLayer { artboard, .. } = request.destination {
                                                        if artboard == self.doc.canvas_id && self.page == Page::Draw {
                                                            self.message("Candidate ready · select it in Layers to review", false);
                                                            self.dock_view = DockView::Layers;
                                                            self.dock = true;
                                                        } else {
                                                            self.message("Candidate ready on its source artboard", false);
                                                        }
                                                    } else {
                                                        self.message("New artboard ready", false);
                                                    }
                                                }
                                                Err(error) => self.message(
                                                    format!(
                                                        "Image saved; could not attach: {error}"
                                                    ),
                                                    true,
                                                ),
                                            }
                                        }
                                    } else if let Some(tab) = self
                                        .project_tabs
                                        .iter_mut()
                                        .find(|tab| tab.epoch == self.job_epoch)
                                    {
                                        if let Some(request) = request {
                                            let attached = request
                                                .blend
                                                .map(|(size, rect)| {
                                                    crate::workflow::crop_blend(
                                                        &reference, size, rect,
                                                    )
                                                    .map(Arc::new)
                                                })
                                                .unwrap_or_else(|| Ok(reference.clone()));
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
                                    self.generation_failed(
                                        id,
                                        format!("Could not create image: {e:#}"),
                                    );
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
        self.poll_story_context(window, cx);
        if self.catalog_refresh_pending.is_none()
            && !self.catalog_busy
            && self.catalog_refresh_at.elapsed() >= Duration::from_secs(2)
        {
            self.catalog_refresh_at = Instant::now();
            if let Some(path) = self
                .shared_catalog_path()
                .filter(|path| self.catalog.as_ref().is_some_and(|c| c.path == *path))
            {
                if self
                    .workers
                    .storage
                    .send(Storage::RefreshCatalog {
                        epoch: self.epoch,
                        path: path.clone(),
                    })
                    .is_ok()
                {
                    self.catalog_refresh_pending = Some((self.epoch, path));
                }
            }
        }
        if !self.render_pending && self.rendered_revision != self.visual_revision {
            let mut preview = self.doc.clone();
            if let Some(crop) = &self.image_crop {
                if crop.artboard == preview.canvas_id {
                    if let Some(layer) = preview.layers.get_mut(crop.layer) {
                        if layer.image.as_ref() == Some(&crop.image) {
                            layer.image.as_mut().unwrap().crop = None;
                        }
                    }
                }
            }
            if self
                .workers
                .render
                .send((self.visual_revision, preview, self.view, self.tracing))
                .is_ok()
            {
                self.render_pending = true;
            }
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
    pub fn cancel_layer_input(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let Some(draft) = self.layer_input_draft.take() else {
            return false;
        };
        let input = match draft.field {
            LayerInput::X => &self.layer_x,
            LayerInput::Y => &self.layer_y,
            LayerInput::Width => &self.layer_width,
            LayerInput::Height => &self.layer_height,
            LayerInput::Opacity => &self.layer_opacity,
            LayerInput::GroupOpacity => &self.group_opacity,
            LayerInput::Name => &self.layer_name,
            LayerInput::GroupName => &self.group_name,
            LayerInput::Rotation => &self.layer_rotation,
        };
        input.update(cx, |state, cx| state.set_value(draft.original, window, cx));
        window.focus(&self.focus, cx);
        cx.notify();
        true
    }
    pub fn key_down(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if event.keystroke.key == "escape" && self.cancel_layer_input(window, cx) {
            cx.stop_propagation();
            return;
        }
        // Canvas focus scopes shortcuts; text fields keep normal editing behavior.
        if self.closing
            || self.switching
            || self.closing_project.is_some()
            || !self.focus.is_focused(window)
        {
            return;
        }
        let key = event.keystroke.key.as_str();
        let modifiers = event.keystroke.modifiers;
        if self.image_crop.is_some() && key != "escape" {
            return;
        }
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
                "v" if self.page == Page::Draw => self.paste_artwork(cx),
                "d" if self.page == Page::Draw && self.tool == Tool::Hand && self.moving_image => {
                    self.duplicate_selected_layers(window, cx);
                }
                "a" if self.page == Page::Draw && self.tool == Tool::Hand && self.moving_image => {
                    self.finish_stroke();
                    let editing = self.editing_group_name().map(str::to_owned);
                    self.selected_layers = self
                        .doc
                        .layers
                        .iter()
                        .enumerate()
                        .filter_map(|(index, layer)| {
                            (self.doc.layer_is_visible(layer)
                                && !self.doc.layer_is_locked(layer)
                                && editing
                                    .as_ref()
                                    .is_none_or(|name| layer.group.as_ref() == Some(name))
                                && self.doc.layer_bounds_for(index).is_some())
                            .then_some(index)
                        })
                        .collect();
                    if editing.is_none() {
                        let groups: std::collections::HashSet<_> = self
                            .selected_layers
                            .iter()
                            .filter_map(|index| self.doc.layers[*index].group.clone())
                            .collect();
                        for (index, layer) in self.doc.layers.iter().enumerate() {
                            if layer
                                .group
                                .as_ref()
                                .is_some_and(|group| groups.contains(group))
                                && !self.selected_layers.contains(&index)
                            {
                                self.selected_layers.push(index);
                            }
                        }
                        self.selected_layers.sort_unstable();
                    }
                    if let Some(index) = self.selected_layers.last().copied() {
                        self.doc.active_layer = index;
                        self.layer_selection_anchor = Some(index);
                        let name = self.doc.layers[index].name.clone();
                        self.layer_name
                            .update(cx, |state, cx| state.set_value(name, window, cx));
                    }
                    self.sync_layer_controls(window, cx);
                }
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
                "backspace" | "delete"
                    if self.page == Page::Draw && self.tool == Tool::Hand && self.moving_image =>
                {
                    let revision = self.revision;
                    self.delete_layer();
                    if self.revision != revision {
                        self.sync_inputs(window, cx);
                    }
                }
                "left" | "right" | "up" | "down"
                    if self.page == Page::Draw && self.tool == Tool::Hand && self.moving_image =>
                {
                    if self.selected_layers.is_empty() {
                        return;
                    }
                    if self.selection_is_locked() {
                        self.message("Unlock selected layers before moving them", false);
                    } else {
                        self.finish_stroke();
                        let amount = if modifiers.shift { 10. } else { 1. };
                        let (dx, dy) = match key {
                            "left" => (-amount, 0.),
                            "right" => (amount, 0.),
                            "up" => (0., -amount),
                            _ => (0., amount),
                        };
                        let previous = self.doc.clone();
                        match self.doc.translate_layers(&self.selected_layers, dx, dy) {
                            Ok(()) => {
                                self.history.checkpoint(&previous);
                                self.changed();
                                self.sync_layer_controls(window, cx);
                            }
                            Err(error) => self.message(error.to_string(), true),
                        }
                    }
                }
                "b" => {
                    self.finish_stroke();
                    self.moving_image = false;
                    self.tool = Tool::Pencil
                }
                "e" => {
                    self.finish_stroke();
                    self.moving_image = false;
                    self.tool = Tool::Eraser
                }
                "v" => {
                    self.finish_stroke();
                    self.tool = Tool::Hand;
                    self.moving_image = true;
                    self.selecting_generation_region = false;
                    self.panel = Panel::None;
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
                    if self.image_crop.is_some() {
                        self.cancel_image_crop(cx);
                        cx.stop_propagation();
                        return;
                    }
                    if self.selection_marquee.is_some() {
                        self.selected_layers = self.marquee_initial_selection.clone();
                        self.sync_layer_controls(window, cx);
                        self.finish_stroke();
                        cx.stop_propagation();
                        cx.notify();
                        return;
                    }
                    if let Some(original) = self.transform_original.take() {
                        if self.transform_checkpointed {
                            for &index in &self.selected_layers {
                                if let (Some(current), Some(previous)) =
                                    (self.doc.layers.get_mut(index), original.layers.get(index))
                                {
                                    *current = previous.clone();
                                }
                            }
                            self.changed();
                        }
                        self.finish_stroke();
                        self.sync_layer_controls(window, cx);
                        cx.stop_propagation();
                        cx.notify();
                        return;
                    }
                    if let Some(group) = self.editing_group_name().map(str::to_owned) {
                        self.select_layer_group(&group, false, window, cx);
                        cx.stop_propagation();
                        cx.notify();
                        return;
                    }
                    self.finish_stroke();
                    if self.tool == Tool::Hand && self.moving_image {
                        self.selected_layers.clear();
                        self.sync_layer_controls(window, cx);
                    }
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
