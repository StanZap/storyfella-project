//! The automation driver — a procedural user for the app.
//!
//! A plan is a sequence of steps written in the app's own language (the
//! closed vocabulary below), loaded from YAML files in `config/plans/`.
//! Plans never mention selectors, coordinates, or DOM — each step is an
//! intent the app executes through the same code paths the UI uses
//! (`crate::commands`), and outcomes are asserted against app state.
//!
//! Two run modes:
//! - `Guide` — slow and narrated: the UI shows an assistant card (see
//!   `crate::ui::guide`) with the step message, waits, then watches the app
//!   perform the action. This is the tutoring experience.
//! - `Check` — fast: the same steps run back to back, expectations gate
//!   progress, and a pass/fail report is produced. This is the regression
//!   suite — a plan that passes means the flows it covers still work.
//!
//! Checkpoints cannot be answered by a procedural run: the driver
//! auto-rejects them, so any op that stops to ask fails fast with the
//! checkpoint message. `modify` used to be such an op — it segmented and
//! asked for mask approval. It no longer does: an instruction edit has no
//! mask, so `modify` runs unattended in a plan (see `config/plans/edit.yaml`).
//! The opt-in mask path (`--mask-prompt` + `--inpaint-prompt`) still
//! checkpoints and still cannot be driven from a plan.

use std::{
    path::{Path, PathBuf},
    str::FromStr,
    time::{Duration, Instant},
};

use dioxus::prelude::*;
use serde::Deserialize;

use crate::{
    app::AppConfig,
    commands::{self, ExecutionUi, OpActivity},
    persistence::ProjectDb,
    registry::{ArtifactKind, GenerationDefaults},
    state::AppState,
    ui::AppScreen,
};

/// Where the app looks for plan files (`config/plans/`, next to
/// `config/app.toml` — same CWD-relative convention).
pub const PLANS_DIR: &str = "config/plans";

/// One step in a plan: an action in the app's vocabulary, plus the
/// tutorial narration (`message`) and pacing.
#[derive(Clone, Debug, PartialEq)]
pub struct Step {
    pub verb: StepVerb,
    pub message: Option<String>,
    /// Guide-mode pacing for this step (overrides `plan.speed_ms`).
    pub wait_ms: Option<u64>,
    /// A failing expectation warns instead of failing the plan.
    pub soft: bool,
}

/// The closed step vocabulary. `Command`/`Compose` are the escape hatches
/// into the full slash language the app owns (`/create character …`,
/// `/variant …`, `/attach-lora …` — everything the composer accepts).
/// `Compose` goes one step closer to the surface: it *types* the text
/// into the canvas composer (visible, character by character) and submits
/// it exactly like a user pressing Enter.
#[derive(Clone, Debug, PartialEq)]
pub enum StepVerb {
    /// Switch screens (the rail actions).
    GoProjects,
    GoStory,
    GoCanvas,
    GoSettings,
    /// Start a new story (same path as the Projects screen form).
    CreateProject(String),
    /// Open a recent project by its story name.
    OpenProject(String),
    /// Create the populated sample project (`app::sample`), so a tour can
    /// show full screens without waiting on a backend.
    SampleProject,
    /// Select an artifact by name — the canvas detail and inspector.
    Open(String),
    /// Return the canvas to the grid.
    Back,
    /// Narration with no action. A tour needs to be able to *say* something
    /// without pretending to do something — repeating a navigation step to
    /// carry a message is how a demo starts looking like it is stuck.
    Say,
    /// Move the writer's caret to a line, so the scope panel visibly
    /// re-derives.
    Caret(usize),
    /// Move the grid's keyboard cursor onto an artifact, so the focus ring
    /// moves the way it does under `hjkl`.
    Cursor(String),
    /// Show or hide the command palette.
    Palette(bool),
    /// Pick a canvas tool, so the rail visibly changes the way a key does.
    SelectTool(crate::ui::tools::Tool),
    /// Paint brush dabs onto the open image's mask.
    ///
    /// Dabs are given in fractions of the image, not pixels and not screen
    /// coordinates: a plan describes *where on the picture*, which survives
    /// any window size, and stays out of the view tree the way every other
    /// step does.
    Paint(Vec<PaintDab>),
    /// Throw the painted mask away.
    ClearMask,
    /// Assert whether a painted mask exists — the thing `--mask painted`
    /// resolves to, so a plan can check the brush reached the operation
    /// layer without generating anything.
    ExpectPaintedMask(bool),
    /// Put an artifact into the rendering state, wait, then complete it
    /// with its existing picture.
    ///
    /// **Demo-only.** No model runs and no new image is produced; this
    /// exists so the in-flight UX — the dimmed tile, the badge, the busy
    /// composer — can be seen without provisioning a backend. Never use it
    /// in a plan that is asserting real generation.
    SimulateRender(String),
    /// Run one slash command through the composer's exact pipeline.
    Command(String),
    /// Type into the canvas composer and submit it, like a user.
    Compose(String),
    /// Assertions against app state.
    ExpectProject(String),
    ExpectArtifact {
        name: String,
        kind: Option<ArtifactKind>,
    },
    ExpectArtifactCount {
        kind: ArtifactKind,
        count: usize,
    },
    ExpectImage(String),
    ExpectNoError,
}

impl StepVerb {
    fn label(&self) -> String {
        match self {
            Self::GoProjects => "open Projects".to_owned(),
            Self::GoStory => "open Story".to_owned(),
            Self::GoCanvas => "open Canvas".to_owned(),
            Self::GoSettings => "open Settings".to_owned(),
            Self::CreateProject(name) => format!("create story \"{name}\""),
            Self::OpenProject(name) => format!("open story \"{name}\""),
            Self::SampleProject => "create the sample project".to_owned(),
            Self::Open(name) => format!("open artifact \"{name}\""),
            Self::Back => "back to the grid".to_owned(),
            Self::Say => String::new(),
            Self::Caret(line) => format!("caret to line {line}"),
            Self::SelectTool(tool) => format!("pick the {} tool", tool.label().to_lowercase()),
            Self::Paint(dabs) => format!("paint {} dabs of mask", dabs.len()),
            Self::ClearMask => "clear the painted mask".to_owned(),
            Self::ExpectPaintedMask(present) => {
                if *present {
                    "a painted mask exists".to_owned()
                } else {
                    "no painted mask".to_owned()
                }
            }
            Self::Cursor(name) => format!("cursor to \"{name}\""),
            Self::Palette(true) => "open the palette".to_owned(),
            Self::Palette(false) => "close the palette".to_owned(),
            Self::SimulateRender(name) => format!("show \"{name}\" rendering"),
            Self::Command(command) => command.clone(),
            Self::Compose(text) => format!("type \"{text}\" and send"),
            Self::ExpectProject(name) => format!("story is \"{name}\""),
            Self::ExpectArtifact { name, .. } => format!("artifact \"{name}\" exists"),
            Self::ExpectArtifactCount { kind, count } => format!("{count} {kind}s exist"),
            Self::ExpectImage(name) => format!("artifact \"{name}\" has an image"),
            Self::ExpectNoError => "no errors are showing".to_owned(),
        }
    }

    /// YAML: `- <action>: <args>` — the action key selects the verb.
    fn parse(action: &str, value: serde_yaml::Value) -> Result<Self, String> {
        let text = |value: &serde_yaml::Value| -> Result<String, String> {
            value
                .as_str()
                .map(str::to_owned)
                .ok_or_else(|| format!("action {action} expects a string argument"))
        };
        let map = expect_mapping;
        let kind = |text: &str| -> Result<ArtifactKind, String> {
            ArtifactKind::from_str(text)
                .map_err(|_| format!("unknown artifact kind \"{text}\" (character, environment, object, scene, story, beat)"))
        };
        Ok(match action {
            "go_projects" => Self::GoProjects,
            "go_story" => Self::GoStory,
            "go_canvas" => Self::GoCanvas,
            "sample_project" => Self::SampleProject,
            "open" => Self::Open(text(&value)?),
            "back" => Self::Back,
            "say" => Self::Say,
            "caret" => Self::Caret(
                value
                    .as_u64()
                    .ok_or_else(|| "action caret expects a line number".to_owned())?
                    as usize,
            ),
            "cursor" => Self::Cursor(text(&value)?),
            "palette" => Self::Palette(value.as_bool().unwrap_or(true)),
            "select_tool" => {
                let name = text(&value)?;
                Self::SelectTool(
                    crate::ui::tools::Tool::parse(&name)
                        .ok_or_else(|| format!("{name:?} is not a canvas tool the rail offers"))?,
                )
            }
            "clear_mask" => Self::ClearMask,
            "expect_painted_mask" => Self::ExpectPaintedMask(value.as_bool().unwrap_or(true)),
            "paint" => {
                let serde_yaml::Value::Sequence(dabs) = &value else {
                    return Err("paint takes a list of {x, y, r} dabs".to_owned());
                };
                let mut parsed = Vec::new();
                for dab in dabs {
                    let field = |name: &str| -> Result<f64, String> {
                        dab.get(serde_yaml::Value::String(name.into()))
                            .and_then(serde_yaml::Value::as_f64)
                            .ok_or_else(|| format!("a paint dab needs a numeric {name}"))
                    };
                    parsed.push(PaintDab {
                        x: field("x")?,
                        y: field("y")?,
                        r: field("r")?,
                    });
                }
                if parsed.is_empty() {
                    return Err("paint needs at least one dab".to_owned());
                }
                Self::Paint(parsed)
            }
            "simulate_render" => Self::SimulateRender(text(&value)?),
            "go_settings" => Self::GoSettings,
            "create_project" => Self::CreateProject(text(&value)?),
            "open_project" => Self::OpenProject(text(&value)?),
            "command" => Self::Command(text(&value)?),
            "compose" => Self::Compose(text(&value)?),
            "expect_project" => Self::ExpectProject(text(&value)?),
            "expect_artifact" => match &value {
                serde_yaml::Value::Mapping(mapping) => {
                    let name = text(
                        mapping
                            .get(serde_yaml::Value::String("name".into()))
                            .ok_or("expect_artifact mapping needs a name")?,
                    )?;
                    let kind = mapping
                        .get(serde_yaml::Value::String("kind".into()))
                        .map(text)
                        .transpose()?
                        .map(|k| kind(&k))
                        .transpose()?;
                    Self::ExpectArtifact { name, kind }
                }
                _ => Self::ExpectArtifact {
                    name: text(&value)?,
                    kind: None,
                },
            },
            "expect_artifact_count" => {
                let mapping = map(action, &value)?;
                let kind_text = text(
                    mapping
                        .get(serde_yaml::Value::String("kind".into()))
                        .ok_or("expect_artifact_count needs a kind")?,
                )?;
                let count = mapping
                    .get(serde_yaml::Value::String("count".into()))
                    .and_then(|value| value.as_u64())
                    .ok_or("expect_artifact_count needs an integer count")?;
                Self::ExpectArtifactCount {
                    kind: kind(&kind_text)?,
                    count: count as usize,
                }
            }
            "expect_image" => Self::ExpectImage(text(&value)?),
            "expect_no_error" => Self::ExpectNoError,
            other => return Err(format!("unknown step action \"{other}\"")),
        })
    }
}

/// The mapping-argument form of a step action (`{ kind: …, count: … }`).
fn expect_mapping<'a>(
    action: &str,
    value: &'a serde_yaml::Value,
) -> Result<&'a serde_yaml::Mapping, String> {
    value
        .as_mapping()
        .ok_or_else(|| format!("action {action} expects a mapping argument"))
}

impl<'de> Deserialize<'de> for Step {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        use serde::de::Error;
        let mapping = serde_yaml::Mapping::deserialize(deserializer)?;
        let mut message = None;
        let mut wait_ms = None;
        let mut soft = false;
        let mut action: Option<(String, serde_yaml::Value)> = None;
        for (key, value) in mapping {
            let key = key
                .as_str()
                .ok_or_else(|| D::Error::custom("step keys must be strings"))?;
            match key {
                "message" => {
                    message = Some(
                        value
                            .as_str()
                            .ok_or_else(|| D::Error::custom("message must be a string"))?
                            .to_owned(),
                    );
                }
                "wait_ms" => {
                    wait_ms = Some(
                        value
                            .as_u64()
                            .ok_or_else(|| D::Error::custom("wait_ms must be an integer"))?,
                    );
                }
                "soft" => {
                    soft = value
                        .as_bool()
                        .ok_or_else(|| D::Error::custom("soft must be a boolean"))?;
                }
                other => {
                    if action.is_some() {
                        return Err(D::Error::custom(
                            "a step has exactly one action (plus optional message/wait_ms/soft)",
                        ));
                    }
                    action = Some((other.to_owned(), value));
                }
            }
        }
        let (action, value) = action.ok_or_else(|| {
            D::Error::custom("a step needs an action, e.g. create_project: \"name\"")
        })?;
        let verb = StepVerb::parse(&action, value).map_err(D::Error::custom)?;
        Ok(Step {
            verb,
            message,
            wait_ms,
            soft,
        })
    }
}

/// A plan document: the narration metadata plus an ordered step list.
#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct Plan {
    pub name: String,
    #[serde(default)]
    pub description: String,
    /// Default guide-mode pacing in milliseconds.
    #[serde(default)]
    pub speed_ms: Option<u64>,
    pub steps: Vec<Step>,
}

impl Plan {
    pub fn from_yaml(yaml: &str) -> Result<Self, String> {
        serde_yaml::from_str(yaml).map_err(|error| format!("plan is not valid YAML: {error}"))
    }
}

/// One plan file on disk, as the Plans screen lists it.
#[derive(Clone, Debug, PartialEq)]
pub struct PlanFile {
    pub path: PathBuf,
    pub plan: Plan,
    /// Parse failures keep the file visible with the reason.
    pub error: Option<String>,
}

/// Loads every `*.yaml` in the plans directory; unreadable or unparseable
/// files yield entries with an error (so a broken plan is visible, not
/// silently dropped).
pub fn discover(dir: &Path) -> Vec<PlanFile> {
    let mut files: Vec<PlanFile> = match std::fs::read_dir(dir) {
        Ok(entries) => entries
            .filter_map(|entry| {
                let entry = entry.ok()?;
                let path = entry.path();
                if path.extension().and_then(|ext| ext.to_str()) != Some("yaml") {
                    return None;
                }
                let raw = match std::fs::read_to_string(&path) {
                    Ok(raw) => raw,
                    Err(error) => {
                        let name = path
                            .file_stem()
                            .and_then(|s| s.to_str())
                            .unwrap_or("plan")
                            .to_owned();
                        return Some(PlanFile {
                            path,
                            plan: Plan {
                                name,
                                description: String::new(),
                                speed_ms: None,
                                steps: Vec::new(),
                            },
                            error: Some(format!("cannot read: {error}")),
                        });
                    }
                };
                let plan = match Plan::from_yaml(&raw) {
                    Ok(plan) => plan,
                    Err(error) => {
                        let name = path
                            .file_stem()
                            .and_then(|s| s.to_str())
                            .unwrap_or("plan")
                            .to_owned();
                        return Some(PlanFile {
                            path,
                            plan: Plan {
                                name,
                                description: String::new(),
                                speed_ms: None,
                                steps: Vec::new(),
                            },
                            error: Some(error),
                        });
                    }
                };
                Some(PlanFile {
                    path,
                    plan,
                    error: None,
                })
            })
            .collect(),
        Err(_) => Vec::new(),
    };
    files.sort_by(|a, b| a.plan.name.cmp(&b.plan.name));
    files
}

/// The two run modes: narrated tutoring vs. fast regression.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum GuideMode {
    #[default]
    Guide,
    Check,
}

impl GuideMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Guide => "guide",
            Self::Check => "check",
        }
    }

    pub fn from_str(value: &str) -> Self {
        match value {
            "check" => Self::Check,
            _ => Self::Guide,
        }
    }
}

/// CLI launch options for the desktop binary (`--plan <file> --mode …`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PlanLaunch {
    pub path: Option<PathBuf>,
    pub mode: GuideMode,
}

impl PlanLaunch {
    pub fn from_args(args: &[String]) -> Self {
        let mut path = None;
        let mut mode = GuideMode::Guide;
        let mut i = 0;
        while i < args.len() {
            match args[i].as_str() {
                "--plan" => {
                    i += 1;
                    if let Some(value) = args.get(i) {
                        path = Some(PathBuf::from(value));
                    }
                }
                "--mode" => {
                    i += 1;
                    if let Some(value) = args.get(i) {
                        mode = GuideMode::from_str(value);
                    }
                }
                _ => {}
            }
            i += 1;
        }
        Self { path, mode }
    }
}

/// Per-step outcome of a run.
#[derive(Clone, Debug, PartialEq)]
pub enum StepStatus {
    Passed,
    Failed {
        reason: String,
    },
    /// A soft step that did not hold: reported, but not a plan failure.
    Warned {
        reason: String,
    },
}

impl StepStatus {
    fn is_failure(&self) -> bool {
        matches!(self, Self::Failed { .. })
    }
}

/// The result of running a plan end to end.
#[derive(Clone, Debug, PartialEq)]
pub struct RunReport {
    pub plan_name: String,
    pub steps: Vec<(String, StepStatus)>,
    pub duration_ms: u64,
}

impl RunReport {
    pub fn passed(&self) -> usize {
        self.steps
            .iter()
            .filter(|(_, status)| !status.is_failure())
            .count()
    }

    pub fn failed(&self) -> usize {
        self.steps
            .iter()
            .filter(|(_, status)| status.is_failure())
            .count()
    }

    /// The check-mode report: one line per step, then the verdict.
    pub fn print(&self) {
        println!("plan: {} ({}ms)", self.plan_name, self.duration_ms);
        for (label, status) in &self.steps {
            match status {
                StepStatus::Passed => println!("  ok   {label}"),
                StepStatus::Failed { reason } => println!("  FAIL {label}: {reason}"),
                StepStatus::Warned { reason } => println!("  warn {label}: {reason}"),
            }
        }
        println!(
            "verdict: {} passed, {} failed",
            self.passed(),
            self.failed()
        );
    }
}

/// Resolves a revision's stored image reference to a file on disk.
///
/// Revisions store a bare asset filename and the UI serves it through the
/// asset handler; a plan step that has to *read* the image needs the real
/// path. An absolute path is taken as-is, so fixtures outside the asset
/// directory still work.
fn resolve_asset(config: &AppConfig, stored: &str) -> PathBuf {
    let raw = Path::new(stored);
    if raw.is_absolute() && raw.exists() {
        return raw.to_path_buf();
    }
    let filename = raw.file_name().unwrap_or(raw.as_os_str());
    let generated = config.asset_dir.join("generated").join(filename);
    if generated.exists() {
        generated
    } else {
        raw.to_path_buf()
    }
}

/// One brush dab in a plan, in fractions of the image: `x` and `y` of its
/// width and height, `r` of its width.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PaintDab {
    pub x: f64,
    pub y: f64,
    pub r: f64,
}

/// What the runner needs to act: the app's signals, plus the driver's own
/// execution surface (checkpoints auto-reject, failures land on its
/// error signal — observed, never the global toast).
#[derive(Clone)]
pub struct DriveContext {
    pub config: AppConfig,
    pub app_state: Signal<AppState>,
    pub backend: Signal<crate::registry::backend::CreativeBackend>,
    pub defaults: Signal<GenerationDefaults>,
    pub screen: Signal<AppScreen>,
    /// The canvas composer text — a `compose` step types into it visibly.
    pub composer: Signal<String>,
    /// The two cursors and the palette, so a demo can show state rather
    /// than only describe it.
    pub writer_caret: crate::ui::WriterCaret,
    pub grid_cursor: crate::ui::GridCursor,
    pub palette_open: Signal<bool>,
    /// The canvas tool rail and the mask painted on the open image, so a
    /// plan can demonstrate the brush through the same state a hand drives.
    pub active_tool: crate::ui::canvas::ActiveTool,
    pub painted_strokes: crate::ui::canvas::PaintedStrokes,
}

/// The live state of a run, as the assistant card renders it.
#[derive(Clone, Debug, PartialEq)]
pub struct GuideSession {
    pub plan: Plan,
    pub mode: GuideMode,
    /// Index of the step currently executing.
    pub current: usize,
    /// The running operation summary, while an op is in flight.
    pub busy: Option<String>,
    pub paused: bool,
    /// Narration pacing multiplier (1x, 2x, 4x).
    pub speed: u32,
    pub state: GuideState,
}

#[derive(Clone, Debug, PartialEq)]
pub enum GuideState {
    Running,
    Done(RunReport),
}

const POLL_INTERVAL_MS: u64 = 80;
const STEP_TIMEOUT_MS: u64 = 30_000;
const OP_TIMEOUT_MS: u64 = 300_000;
const IMAGE_TIMEOUT_MS: u64 = 300_000;

/// Starts a plan run: installs the session (the assistant card appears),
/// then executes the steps on a spawned task. `exit_on_done` prints the
/// report and exits the process (the CLI check mode).
pub fn run_plan(
    ctx: DriveContext,
    ui: ExecutionUi,
    plan: Plan,
    mode: GuideMode,
    mut session: Signal<Option<GuideSession>>,
    exit_on_done: bool,
) {
    session.set(Some(GuideSession {
        plan: plan.clone(),
        mode,
        current: 0,
        busy: None,
        paused: false,
        speed: 1,
        state: GuideState::Running,
    }));
    spawn(async move {
        let report = run_steps(ctx.clone(), ui, &plan, mode, session).await;
        let exit_code = if report.failed() > 0 { 1 } else { 0 };
        if exit_on_done {
            // Check mode exits the process right after the report; the
            // autosave effect may not get a render tick in between, so the
            // final state is persisted here, before the report.
            if let Err(error) = commands::save_project(ctx.app_state) {
                println!(
                    "warning: could not save the project before exit: {}",
                    error.details
                );
            }
            report.print();
            std::process::exit(exit_code);
        }
        session.with_mut(|slot| {
            if let Some(session) = slot {
                session.state = GuideState::Done(report);
            }
        });
    });
}

async fn run_steps(
    ctx: DriveContext,
    ui: ExecutionUi,
    plan: &Plan,
    mode: GuideMode,
    mut session: Signal<Option<GuideSession>>,
) -> RunReport {
    let started = Instant::now();
    let mut results: Vec<(String, StepStatus)> = Vec::new();
    for (index, step) in plan.steps.iter().enumerate() {
        if session.read().is_none() {
            break; // cancelled
        }
        session.with_mut(|slot| {
            if let Some(session) = slot {
                session.current = index;
                session.state = GuideState::Running;
            }
        });
        if mode == GuideMode::Guide {
            let wait = step.wait_ms.or(plan.speed_ms).unwrap_or(2_000);
            wait_with_pause(&session, wait).await;
        }
        if session.read().is_none() {
            break;
        }
        let status = execute_step(ctx.clone(), ui.clone(), step, session).await;
        let mut status = status;
        if step.soft {
            if let StepStatus::Failed { reason } = status {
                status = StepStatus::Warned { reason };
            }
        }
        results.push((step.verb.label(), status.clone()));
        if status.is_failure() {
            break; // fail-fast: the flow is broken, stop walking it
        }
    }
    session.with_mut(|slot| {
        if let Some(session) = slot {
            session.busy = None;
        }
    });
    RunReport {
        plan_name: plan.name.clone(),
        steps: results,
        duration_ms: started.elapsed().as_millis() as u64,
    }
}

async fn execute_step(
    mut ctx: DriveContext,
    mut ui: ExecutionUi,
    step: &Step,
    session: Signal<Option<GuideSession>>,
) -> StepStatus {
    match &step.verb {
        StepVerb::GoProjects => {
            ctx.screen.set(AppScreen::Projects);
            StepStatus::Passed
        }
        StepVerb::GoStory => {
            ctx.screen.set(AppScreen::Writer);
            StepStatus::Passed
        }
        StepVerb::GoCanvas => {
            ctx.screen.set(AppScreen::Canvas);
            StepStatus::Passed
        }
        StepVerb::SampleProject => {
            match commands::create_project(
                &ctx.config.project_dir,
                "Keeper of the Low Light",
                ctx.app_state,
            ) {
                Ok(_) => {
                    let (registry, story) = crate::app::sample::project(&ctx.config.asset_dir);
                    let mut state = ctx.app_state.write();
                    state.registry = registry;
                    state.story_source = story;
                    state.has_unsaved_changes = true;
                    drop(state);
                    ui.error_report.set(None);
                    ctx.screen.set(AppScreen::Canvas);
                    StepStatus::Passed
                }
                Err(report) => StepStatus::Failed {
                    reason: report.details.clone(),
                },
            }
        }
        StepVerb::Open(name) => {
            let id = ctx
                .app_state
                .read()
                .registry
                .artifacts
                .iter()
                .find(|artifact| artifact.name.eq_ignore_ascii_case(name))
                .map(|artifact| artifact.id);
            match id {
                Some(id) => {
                    ctx.app_state.write().select_artifact(id);
                    ctx.screen.set(AppScreen::Canvas);
                    StepStatus::Passed
                }
                None => StepStatus::Failed {
                    reason: format!("no artifact named {name:?}"),
                },
            }
        }
        StepVerb::Back => {
            ctx.app_state.write().clear_selection();
            StepStatus::Passed
        }
        StepVerb::Say => StepStatus::Passed,
        StepVerb::Caret(line) => {
            ctx.writer_caret.0.clone().set(*line);
            ctx.screen.set(AppScreen::Writer);
            StepStatus::Passed
        }
        StepVerb::Cursor(name) => {
            let query = String::new();
            let ids = crate::ui::gallery::ordered_ids(&ctx.app_state.read().registry, &query);
            let target = ctx
                .app_state
                .read()
                .registry
                .artifacts
                .iter()
                .find(|artifact| artifact.name.eq_ignore_ascii_case(name))
                .map(|artifact| artifact.id);
            match target.and_then(|id| ids.iter().position(|candidate| *candidate == id)) {
                Some(index) => {
                    ctx.grid_cursor.0.clone().set(index);
                    ctx.app_state.write().clear_selection();
                    ctx.screen.set(AppScreen::Canvas);
                    StepStatus::Passed
                }
                None => StepStatus::Failed {
                    reason: format!("no tile for {name:?}"),
                },
            }
        }
        StepVerb::Palette(open) => {
            ctx.palette_open.clone().set(*open);
            StepStatus::Passed
        }
        StepVerb::SelectTool(tool) => {
            ctx.active_tool.0.clone().set(*tool);
            StepStatus::Passed
        }
        StepVerb::ClearMask => {
            ctx.painted_strokes.0.clone().write().clear();
            crate::ui::canvas::write_painted_mask(ctx.app_state, &[], None);
            StepStatus::Passed
        }
        StepVerb::ExpectPaintedMask(present) => {
            let mask = ctx.app_state.read().painted_mask.clone();
            match (present, mask) {
                (true, Some(path)) if std::path::Path::new(&path).exists() => StepStatus::Passed,
                (true, Some(path)) => StepStatus::Failed {
                    reason: format!("the painted mask {path} was not written"),
                },
                (true, None) => StepStatus::Failed {
                    reason: "nothing is painted".to_owned(),
                },
                (false, None) => StepStatus::Passed,
                (false, Some(path)) => StepStatus::Failed {
                    reason: format!("expected no painted mask, found {path}"),
                },
            }
        }
        StepVerb::Paint(dabs) => {
            // Fractions are of the open image, so the plan needs it open and
            // needs its real dimensions — the artifact's default size is a
            // guess, and painting against a guess lands the mask elsewhere.
            let image = ctx
                .app_state
                .read()
                .selected_artifact()
                .and_then(|artifact| {
                    ctx.app_state
                        .read()
                        .display_image(artifact)
                        .map(str::to_owned)
                });
            let Some(image) = image else {
                return StepStatus::Failed {
                    reason: "paint needs an artifact with an image open".to_owned(),
                };
            };
            // A revision stores a bare asset filename, which the UI resolves
            // through the asset handler; the driver has to resolve it on
            // disk instead.
            let on_disk = resolve_asset(&ctx.config, &image);
            let Ok((width, height)) = crate::registry::image_ops::dimensions(&on_disk) else {
                return StepStatus::Failed {
                    reason: format!("could not read the size of {}", on_disk.display()),
                };
            };
            let mut strokes = ctx.painted_strokes.0;
            for dab in dabs {
                let radius = dab.r * width as f64;
                strokes.write().push(crate::registry::pipeline::Stroke {
                    center_x: dab.x * width as f64,
                    center_y: dab.y * height as f64,
                    radius_x: radius,
                    radius_y: radius,
                });
            }
            // Write the mask now rather than waiting for the canvas to
            // re-render: the very next step may be the command that uses it.
            let painted = strokes.read().clone();
            crate::ui::canvas::write_painted_mask(ctx.app_state, &painted, Some((width, height)));
            StepStatus::Passed
        }
        StepVerb::SimulateRender(name) => {
            let id = ctx
                .app_state
                .read()
                .registry
                .artifacts
                .iter()
                .find(|artifact| artifact.name.eq_ignore_ascii_case(name))
                .map(|artifact| artifact.id);
            let Some(id) = id else {
                return StepStatus::Failed {
                    reason: format!("no artifact named {name:?}"),
                };
            };
            // Reuse the artifact's current picture: nothing is generated,
            // only the states around it are shown.
            let existing = ctx
                .app_state
                .read()
                .registry
                .latest_image(id)
                .and_then(|revision| revision.asset_path.clone());
            let prompt = ctx
                .app_state
                .read()
                .registry
                .artifact(id)
                .map(|artifact| artifact.description.clone())
                .unwrap_or_default();

            let revision = {
                let mut state = ctx.app_state.write();
                state.registry.start_revision(
                    id,
                    prompt,
                    Some(4471),
                    Some("krea-2-turbo-q4".into()),
                    crate::registry::RevisionOrigin::Canon,
                )
            };
            let Ok(revision) = revision else {
                return StepStatus::Failed {
                    reason: "could not start the demonstration revision".to_owned(),
                };
            };
            if let Some(artifact) = ctx.app_state.write().registry.artifact_mut(id) {
                if let Some(entry) = artifact
                    .revisions
                    .iter_mut()
                    .find(|candidate| candidate.id == revision)
                {
                    entry.status = crate::registry::RevisionStatus::Generating;
                }
            }
            // Long enough to read the badge, short enough not to stall.
            tokio::time::sleep(std::time::Duration::from_millis(2200)).await;
            match existing {
                Some(path) => {
                    let _ = ctx.app_state.write().registry.finish_revision(
                        id,
                        revision,
                        path,
                        Vec::new(),
                    );
                }
                None => {
                    let _ = ctx.app_state.write().registry.cancel_revision(id, revision);
                }
            }
            StepStatus::Passed
        }
        StepVerb::GoSettings => {
            ctx.screen.set(AppScreen::Settings);
            StepStatus::Passed
        }
        StepVerb::CreateProject(name) => {
            match commands::create_project(&ctx.config.project_dir, name, ctx.app_state) {
                Ok(_) => {
                    ctx.screen.set(AppScreen::Canvas);
                    ui.error_report.set(None);
                    StepStatus::Passed
                }
                Err(report) => StepStatus::Failed {
                    reason: report.details.clone(),
                },
            }
        }
        StepVerb::OpenProject(name) => match find_project_by_name(&ctx.config.project_dir, name) {
            Some(path) => match commands::open_project(&path, ctx.app_state) {
                Ok(()) => {
                    ctx.screen.set(AppScreen::Canvas);
                    StepStatus::Passed
                }
                Err(report) => StepStatus::Failed {
                    reason: report.details.clone(),
                },
            },
            None => StepStatus::Failed {
                reason: format!("no project named \"{name}\" in the projects directory"),
            },
        },
        StepVerb::Command(command) => {
            let backend = (ctx.backend)();
            let defaults = ctx.defaults.read().clone();
            match commands::run_slash_command(ctx.app_state, backend, defaults, ui.clone(), command)
            {
                Err(report) => StepStatus::Failed {
                    reason: report.details.clone(),
                },
                Ok(()) => wait_for_op(ui, session).await,
            }
        }
        StepVerb::Compose(text) => {
            // Type the text into the composer character by character (the
            // user sees it appear), pause briefly, then submit — clearing
            // the composer first, exactly like the canvas submit handler.
            let mut composer = ctx.composer;
            for character in text.chars() {
                if session.read().is_none() {
                    return StepStatus::Failed {
                        reason: "the run was cancelled while typing".to_owned(),
                    };
                }
                composer.with_mut(|current| current.push(character));
                tokio::time::sleep(Duration::from_millis(16)).await;
            }
            tokio::time::sleep(Duration::from_millis(120)).await;
            composer.with_mut(|current| current.clear());
            let backend = (ctx.backend)();
            let defaults = ctx.defaults.read().clone();
            match commands::run_slash_command(ctx.app_state, backend, defaults, ui.clone(), text) {
                Err(report) => StepStatus::Failed {
                    reason: report.details.clone(),
                },
                Ok(()) => wait_for_op(ui, session).await,
            }
        }
        StepVerb::ExpectProject(name) => {
            let expected = name.clone();
            let app_state = ctx.app_state;
            let ok = poll_until(
                move || {
                    let state = app_state.read();
                    let matches = state.project_name.eq_ignore_ascii_case(&expected)
                        && state.project_path.is_some();
                    matches
                },
                STEP_TIMEOUT_MS,
            )
            .await;
            status(ok, format!("expected the open story to be \"{name}\""))
        }
        StepVerb::ExpectArtifact { name, kind } => {
            let expected = name.clone();
            let kind = *kind;
            let app_state = ctx.app_state;
            let ok = poll_until(
                move || {
                    let state = app_state.read();
                    let found = state.registry.iter().any(|artifact| {
                        artifact.name.eq_ignore_ascii_case(&expected)
                            && kind.is_none_or(|kind| artifact.kind == kind)
                    });
                    found
                },
                STEP_TIMEOUT_MS,
            )
            .await;
            status(
                ok,
                format!("expected an artifact named \"{name}\" in the registry"),
            )
        }
        StepVerb::ExpectArtifactCount { kind, count } => {
            let kind = *kind;
            let count = *count;
            let app_state = ctx.app_state;
            let ok = poll_until(
                move || {
                    let state = app_state.read();
                    let matches = state
                        .registry
                        .iter()
                        .filter(|artifact| artifact.kind == kind)
                        .count()
                        == count;
                    matches
                },
                STEP_TIMEOUT_MS,
            )
            .await;
            status(
                ok,
                format!("expected exactly {count} {kind}s in the registry"),
            )
        }
        StepVerb::ExpectImage(name) => {
            let expected = name.clone();
            let app_state = ctx.app_state;
            let ok = poll_until(
                move || {
                    let state = app_state.read();
                    let has_image = state
                        .registry
                        .iter()
                        .find(|artifact| artifact.name.eq_ignore_ascii_case(&expected))
                        .and_then(|artifact| state.registry.latest_image(artifact.id))
                        .and_then(|revision| revision.asset_path.as_ref())
                        .map(|path| Path::new(path).exists())
                        .unwrap_or(false);
                    has_image
                },
                IMAGE_TIMEOUT_MS,
            )
            .await;
            status(ok, format!("expected \"{name}\" to have a finished image"))
        }
        StepVerb::ExpectNoError => {
            let ok = ui.error_report.read().is_none();
            status(ok, "expected no error report".to_owned())
        }
    }
}

fn status(ok: bool, reason: String) -> StepStatus {
    if ok {
        StepStatus::Passed
    } else {
        StepStatus::Failed { reason }
    }
}

/// Waits for a submitted operation to settle back to Idle, mirroring its
/// summary on the session (the assistant card shows what the app is doing).
/// The busy sync must not live inside a poll closure — signals need `&mut`,
/// closures passed to polling are `Fn`.
async fn wait_for_op(mut ui: ExecutionUi, mut session: Signal<Option<GuideSession>>) -> StepStatus {
    let deadline = Instant::now() + Duration::from_millis(OP_TIMEOUT_MS);
    let done = loop {
        if session.read().is_none() {
            break false; // cancelled
        }
        if !ui.activity.read().is_busy() {
            break true;
        }
        if let OpActivity::Running { summary } = ui.activity.read().clone() {
            session.with_mut(|slot| {
                if let Some(session) = slot {
                    if session.busy.as_deref() != Some(summary.as_str()) {
                        session.busy = Some(summary.clone());
                    }
                }
            });
        }
        if Instant::now() >= deadline {
            break false;
        }
        tokio::time::sleep(Duration::from_millis(POLL_INTERVAL_MS)).await;
    };
    session.with_mut(|slot| {
        if let Some(session) = slot {
            session.busy = None;
        }
    });
    ui.reject_pending();
    if !done {
        StepStatus::Failed {
            reason: "the operation did not finish in time".to_owned(),
        }
    } else if let Some(report) = ui.error_report.read().clone() {
        StepStatus::Failed {
            reason: format!("{} — {}", report.code.code(), report.details),
        }
    } else {
        StepStatus::Passed
    }
}

/// Finds the project database whose stored story name matches, scanning
/// the projects directory (the Recent-list equivalent).
fn find_project_by_name(project_dir: &Path, name: &str) -> Option<PathBuf> {
    let entries = std::fs::read_dir(project_dir).ok()?;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("db") {
            continue;
        }
        if let Ok(stored) = ProjectDb::open(&path).and_then(|db| db.load()) {
            if stored.name.eq_ignore_ascii_case(name) {
                return Some(path);
            }
        }
    }
    None
}

/// Polls a condition (reading app state) until it holds or the timeout
/// expires. The only sleeping primitive the runner needs.
async fn poll_until<F: Fn() -> bool>(check: F, timeout_ms: u64) -> bool {
    let deadline = Instant::now() + Duration::from_millis(timeout_ms);
    while Instant::now() < deadline {
        if check() {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(POLL_INTERVAL_MS)).await;
    }
    check()
}

/// Guide-mode narration pacing: waits the step's duration, honoring pause
/// and speed from the assistant card; a cancelled session returns early.
async fn wait_with_pause(session: &Signal<Option<GuideSession>>, wait_ms: u64) {
    let mut remaining = wait_ms;
    while remaining > 0 {
        let active = session.read().clone();
        let Some(active) = active else {
            return;
        };
        if active.paused {
            tokio::time::sleep(Duration::from_millis(80)).await;
            continue;
        }
        let slice = (POLL_INTERVAL_MS as u64)
            .div_ceil(active.speed.max(1) as u64)
            .max(20);
        tokio::time::sleep(Duration::from_millis(slice)).await;
        remaining = remaining.saturating_sub(slice);
    }
}

#[cfg(test)]
mod tests {
    /// Every shipped plan must parse — a typo in a step key is otherwise
    /// only discovered when someone tries to run it.
    #[test]
    fn shipped_plans_parse() {
        let dir = std::path::Path::new(PLANS_DIR);
        let entries = std::fs::read_dir(dir).expect("plans directory exists");
        let mut seen = 0;
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("yaml") {
                continue;
            }
            let raw = std::fs::read_to_string(&path).expect("plan is readable");
            Plan::from_yaml(&raw).unwrap_or_else(|error| panic!("{path:?}: {error}"));
            seen += 1;
        }
        assert!(seen > 0, "no plans found to check");
    }

    use super::*;

    /// The brush verbs are the only ones that carry geometry, and geometry
    /// is where a plan could start describing the screen instead of the
    /// picture. These are fractions of the image, so they hold at any
    /// window size — which is the property worth pinning.
    #[test]
    fn brush_steps_parse_as_image_fractions() {
        let plan = Plan::from_yaml(
            r#"
name: "Brush"
description: "d"
steps:
  - select_tool: "brush"
  - paint:
      - { x: 0.5, y: 0.25, r: 0.1 }
      - { x: 0.4, y: 0.3, r: 0.08 }
  - clear_mask: true
"#,
        )
        .unwrap();
        assert_eq!(
            plan.steps[0].verb,
            StepVerb::SelectTool(crate::ui::tools::Tool::Brush)
        );
        assert_eq!(
            plan.steps[1].verb,
            StepVerb::Paint(vec![
                PaintDab {
                    x: 0.5,
                    y: 0.25,
                    r: 0.1
                },
                PaintDab {
                    x: 0.4,
                    y: 0.3,
                    r: 0.08
                },
            ])
        );
        assert_eq!(plan.steps[2].verb, StepVerb::ClearMask);
    }

    #[test]
    fn a_tool_a_plan_cannot_use_is_rejected_at_parse() {
        // Crop is in the rail but does nothing yet. A plan naming it should
        // fail when it loads, not silently do nothing halfway through a demo.
        let error = Plan::from_yaml(
            "name: \"x\"\ndescription: \"d\"\nsteps:\n  - select_tool: \"nonsense\"\n",
        )
        .unwrap_err();
        assert!(error.contains("nonsense"), "{error}");
    }

    #[test]
    fn paint_needs_dabs_with_all_three_fields() {
        let error = Plan::from_yaml(
            "name: \"x\"\ndescription: \"d\"\nsteps:\n  - paint:\n      - { x: 0.5, y: 0.2 }\n",
        )
        .unwrap_err();
        assert!(error.contains("r"), "{error}");
    }

    #[test]
    fn minimal_steps_parse() {
        let plan = Plan::from_yaml(
            r#"
name: smoke
steps:
  - create_project: "My Story"
    message: "Let's begin."
  - command: "/create story once upon a time"
  - expect_artifact: "once"
  - expect_no_error: true
"#,
        )
        .expect("plan parses");
        assert_eq!(plan.name, "smoke");
        assert_eq!(plan.steps.len(), 4);
        assert_eq!(
            plan.steps[0],
            Step {
                verb: StepVerb::CreateProject("My Story".into()),
                message: Some("Let's begin.".into()),
                wait_ms: None,
                soft: false,
            }
        );
        assert_eq!(
            plan.steps[2].verb,
            StepVerb::ExpectArtifact {
                name: "once".into(),
                kind: None,
            }
        );
    }

    #[test]
    fn expect_artifact_kind_map_parses() {
        let plan = Plan::from_yaml(
            r#"
name: kinds
steps:
  - expect_artifact: { name: "ada", kind: character }
  - expect_artifact_count: { kind: scene, count: 2 }
"#,
        )
        .expect("plan parses");
        assert_eq!(
            plan.steps[0].verb,
            StepVerb::ExpectArtifact {
                name: "ada".into(),
                kind: Some(ArtifactKind::Character),
            }
        );
        assert_eq!(
            plan.steps[1].verb,
            StepVerb::ExpectArtifactCount {
                kind: ArtifactKind::Scene,
                count: 2,
            }
        );
    }

    #[test]
    fn compose_steps_parse_like_user_typed_text() {
        let plan = Plan::from_yaml(
            r#"
name: compose
steps:
  - compose: "/create character ada a keeper"
    message: "Watch the composer."
"#,
        )
        .expect("plan parses");
        assert_eq!(
            plan.steps[0].verb,
            StepVerb::Compose("/create character ada a keeper".into())
        );
        assert_eq!(
            plan.steps[0].message.as_deref(),
            Some("Watch the composer.")
        );
    }

    #[test]
    fn unknown_actions_are_rejected() {
        let error = Plan::from_yaml(
            r#"
name: bad
steps:
  - wiggle_the_mouse: true
"#,
        )
        .expect_err("plan rejects");
        assert!(error.contains("unknown step action"), "{error}");
    }

    #[test]
    fn two_actions_in_one_step_are_rejected() {
        let error = Plan::from_yaml(
            r#"
name: bad
steps:
  - go_canvas: true
    go_projects: true
"#,
        )
        .expect_err("plan rejects");
        assert!(error.contains("exactly one action"), "{error}");
    }

    #[test]
    fn report_counts_passed_failed_and_warned() {
        let report = RunReport {
            plan_name: "p".into(),
            steps: vec![
                ("a".into(), StepStatus::Passed),
                ("b".into(), StepStatus::Failed { reason: "x".into() }),
                ("c".into(), StepStatus::Warned { reason: "y".into() }),
            ],
            duration_ms: 10,
        };
        assert_eq!(report.passed(), 2);
        assert_eq!(report.failed(), 1);
    }
}
