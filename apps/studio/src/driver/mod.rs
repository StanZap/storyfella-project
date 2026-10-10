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
//! Checkpoints cannot be answered by a procedural run: an operation that
//! stops to ask fails fast, because a plan cannot answer a question only a
//! user can. `modify` no longer does: an instruction edit is unattended, so
//! it runs in a plan (see `config/plans/edit.yaml`).

use std::{
    path::{Path, PathBuf},
    str::FromStr,
    time::{Duration, Instant},
};

use dioxus::prelude::*;
use serde::Deserialize;

use storyfella_studio_core::config::AppConfig;
use storyfella_studio_core::errors::CodedError;
use storyfella_studio_core::persistence::ProjectDb;
use storyfella_studio_core::registry::{ArtifactKind, GenerationDefaults};
use storyfella_studio_core::vision;

use crate::{
    commands::{self, ExecutionUi, OpActivity},
    state::AppState,
    ui::AppScreen,
};

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
    /// show full screens without waiting on a backend. Guide mode types its
    /// entire source in the Writer; Check mode applies it immediately.
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
    /// Append authored text in the Writer, character by character in Guide mode.
    TypeStory(String),
    /// Move the grid's keyboard cursor onto an artifact, so the focus ring
    /// moves the way it does under `hjkl`.
    Cursor(String),
    /// Show or hide the command palette.
    Palette(bool),
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
    ExpectStorySequence(Vec<String>),
    OpenProjectFile(String),
    Preview,
    Playback(storyfella_runtime::Command),
    PlaybackBookmark,
    PlaybackRestore,
    ExpectPlayback(PlaybackExpectation),
    /// Ask the configured `vision` provider whether an artifact's current
    /// image answers a question about it — a real check that the generation
    /// produced what was asked for, where [`Self::ExpectImage`] only checks
    /// that *a* picture exists.
    ///
    /// Non-hermetic and slow (a vision round trip, and a managed server may
    /// have to start), so it belongs in plans that already need the vision
    /// endpoint. `expect`/`expect_not` are case-insensitive substring gates,
    /// as in `svs eval`.
    ExpectVision {
        image: String,
        question: String,
        expect: Option<String>,
        expect_not: Option<String>,
    },
    ExpectNoError,
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlaybackExpectation {
    pub scene: String,
    pub phase: storyfella_runtime::Phase,
    pub text: Option<String>,
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
            Self::TypeStory(_) => "type in the Writer".to_owned(),
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
            Self::ExpectStorySequence(_) => "story scene/beat sequence matches source".into(),
            Self::OpenProjectFile(path) => format!("open project file {path}"),
            Self::Preview => "start the current draft in Preview".into(),
            Self::Playback(command) => format!("playback {command:?}"),
            Self::PlaybackBookmark => "bookmark playback state".into(),
            Self::PlaybackRestore => "restore playback bookmark".into(),
            Self::ExpectPlayback(expected) => {
                format!("playback is {:?} in {}", expected.phase, expected.scene)
            }
            Self::ExpectVision { image, .. } => format!("vision check on \"{image}\""),
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
            "type_story" => Self::TypeStory(text(&value)?),
            "palette" => Self::Palette(value.as_bool().unwrap_or(true)),
            "simulate_render" => Self::SimulateRender(text(&value)?),
            "go_settings" => Self::GoSettings,
            "create_project" => Self::CreateProject(text(&value)?),
            "open_project" => Self::OpenProject(text(&value)?),
            "command" => Self::Command(text(&value)?),
            "compose" => Self::Compose(text(&value)?),
            "expect_project" => Self::ExpectProject(text(&value)?),
            "open_project_file" => Self::OpenProjectFile(text(&value)?),
            "preview" => Self::Preview,
            "playback" => Self::Playback(serde_yaml::from_value(value).map_err(|e| e.to_string())?),
            "playback_bookmark" => Self::PlaybackBookmark,
            "playback_restore" => Self::PlaybackRestore,
            "expect_playback" => {
                Self::ExpectPlayback(serde_yaml::from_value(value).map_err(|e| e.to_string())?)
            }
            "expect_story_sequence" => Self::ExpectStorySequence(
                serde_yaml::from_value(value)
                    .map_err(|e| format!("expect_story_sequence expects declaration keys: {e}"))?,
            ),
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
            "expect_vision" => {
                let mapping = map(action, &value)?;
                let image = text(
                    mapping
                        .get(serde_yaml::Value::String("image".into()))
                        .ok_or("expect_vision needs an image (an artifact name)")?,
                )?;
                let question = text(
                    mapping
                        .get(serde_yaml::Value::String("question".into()))
                        .ok_or("expect_vision needs a question")?,
                )?;
                let optional = |key: &str| -> Result<Option<String>, String> {
                    mapping
                        .get(serde_yaml::Value::String(key.into()))
                        .map(text)
                        .transpose()
                };
                Self::ExpectVision {
                    image,
                    question,
                    expect: optional("expect")?,
                    expect_not: optional("expect_not")?,
                }
            }
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
#[serde(deny_unknown_fields)]
pub struct Plan {
    pub name: String,
    #[serde(default)]
    pub description: String,
    /// Default guide-mode pacing in milliseconds.
    #[serde(default)]
    pub speed_ms: Option<u64>,
    /// Guide typing rate, independent of narration pacing (1–200 characters/s).
    #[serde(
        default = "default_typing_cps",
        deserialize_with = "deserialize_typing_cps"
    )]
    pub typing_cps: u16,
    pub steps: Vec<Step>,
}

fn default_typing_cps() -> u16 {
    60
}

fn deserialize_typing_cps<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<u16, D::Error> {
    let rate = u16::deserialize(deserializer)?;
    if (1..=200).contains(&rate) {
        Ok(rate)
    } else {
        Err(serde::de::Error::custom(
            "typing_cps must be between 1 and 200",
        ))
    }
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
                                typing_cps: default_typing_cps(),
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
                                typing_cps: default_typing_cps(),
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

    pub fn from_value(value: &str) -> Self {
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
                        mode = GuideMode::from_value(value);
                    }
                }
                _ => {}
            }
            i += 1;
        }
        Self { path, mode }
    }

    /// Points a `--plan` run at a scratch user-level config file under
    /// `.svs-e2e/hermetic/`, so the driver never reads or writes the
    /// developer's real machine-local settings.
    ///
    /// The file need not exist: a missing overlay contributes nothing, so the
    /// run sees exactly the checked-in `config/storyfella.toml`.
    /// An explicit `$SVS_USER_CONFIG` already in the
    /// environment wins, so a run can still be aimed at a specific config.
    pub fn make_hermetic(&self) {
        if self.path.is_none() {
            return;
        }
        let dir = storyfella_studio_core::paths::root().join(".svs-e2e/hermetic");
        set_env_unless_set("SVS_USER_CONFIG", dir.join("storyfella.toml"));
    }

    /// Plan-created projects, generated fixtures and cache are scratch artifacts,
    /// separate from ordinary user project storage and repository source assets.
    pub fn configure_paths(&self, config: &mut AppConfig) {
        if self.path.is_some() {
            let scratch = storyfella_studio_core::paths::root().join(".svs-e2e");
            config.project_dir = scratch.join("projects");
            config.asset_dir = scratch.join("artwork");
            config.cache_dir = scratch.join("cache");
        }
    }
}

/// Sets `key` to `value` unless the environment already defines it (an empty
/// value counts as unset, matching `paths::*_path`).
fn set_env_unless_set(key: &str, value: PathBuf) {
    match std::env::var_os(key) {
        Some(existing) if !existing.is_empty() => {}
        _ => std::env::set_var(key, value),
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

/// What the runner needs to act: the app's signals, plus the driver's own
/// execution surface (failures land on its error signal — observed, never
/// the global toast).
#[derive(Clone)]
pub struct DriveContext {
    pub config: AppConfig,
    pub app_state: Signal<AppState>,
    pub backend: Signal<storyfella_studio_core::registry::backend::CreativeBackend>,
    pub defaults: Signal<GenerationDefaults>,
    pub screen: Signal<AppScreen>,
    pub playback: Signal<storyfella_playback_ui::PlaybackModel>,
    /// The canvas composer text — a `compose` step types into it visibly.
    pub composer: Signal<String>,
    /// The two cursors and the palette, so a demo can show state rather
    /// than only describe it.
    pub writer_caret: crate::ui::WriterCaret,
    pub grid_cursor: crate::ui::GridCursor,
    pub palette_open: Signal<bool>,
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
    pub typing_cps: u16,
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
        typing_cps: plan.typing_cps,
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
            // `std::process::exit` runs no destructors, so stop the server the
            // app owns here or the check run would orphan it.
            crate::shutdown::run();
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
    // The vision session (if a step needs one) lives for the whole run, so a
    // managed server is started once rather than per assessment.
    let mut vision: Option<vision::VisionSession> = None;
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
        let status = execute_step(ctx.clone(), ui.clone(), step, session, &mut vision).await;
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
    vision: &mut Option<vision::VisionSession>,
) -> StepStatus {
    match &step.verb {
        StepVerb::OpenProjectFile(path) => {
            let path = storyfella_studio_core::paths::root().join(path);
            match commands::open_project(&path, ctx.app_state) {
                Ok(()) => StepStatus::Passed,
                Err(error) => StepStatus::Failed {
                    reason: error.details,
                },
            }
        }
        StepVerb::Preview => {
            crate::ui::preview::start_preview(ctx.playback, ctx.app_state);
            ctx.screen.set(AppScreen::Preview);
            let error = ctx.playback.read().error.clone();
            status(error.is_none(), error.unwrap_or_default())
        }
        StepVerb::Playback(command) => {
            ctx.playback.write().dispatch(command.clone());
            let error = ctx.playback.read().error.clone();
            status(error.is_none(), error.unwrap_or_default())
        }
        StepVerb::PlaybackBookmark => {
            ctx.playback.write().bookmark();
            StepStatus::Passed
        }
        StepVerb::PlaybackRestore => {
            ctx.playback.write().restore_bookmark();
            let error = ctx.playback.read().error.clone();
            status(error.is_none(), error.unwrap_or_default())
        }
        StepVerb::ExpectPlayback(expected) => {
            let model = ctx.playback.read();
            let Some(session) = &model.session else {
                return StepStatus::Failed {
                    reason: "no playback session".into(),
                };
            };
            let runtime = &session.runtime;
            let text = match &runtime.current().kind {
                storyfella_compiler::InstructionKind::Speech { text, .. }
                | storyfella_compiler::InstructionKind::Narration { text } => {
                    Some(text.value.as_str())
                }
                _ => None,
            };
            let visuals = session.active_visuals();
            let valid = runtime.state().phase == expected.phase
                && runtime.plan().scenes[runtime.state().scene].id.0 == expected.scene
                && expected.text.as_deref().is_none_or(|t| Some(t) == text)
                && visuals.len() == runtime.state().assets.len()
                && visuals.iter().all(|v| {
                    v.image_url.starts_with("data:image/png;base64,")
                        && !v.binding.revision.is_empty()
                });
            status(
                valid,
                format!(
                    "expected {expected:?}; got {:?}, text {text:?}",
                    runtime.state()
                ),
            )
        }
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
                    {
                        let mut state = ctx.app_state.write();
                        state.registry = registry;
                        state.set_story_source(String::new());
                        state.has_unsaved_changes = true;
                    }
                    ui.error_report.set(None);
                    type_story(&mut ctx, &session, &story).await
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
            ctx.writer_caret.navigate(storyfella_language::Position {
                line: *line as u32,
                character: 0,
            });
            ctx.screen.set(AppScreen::Writer);
            StepStatus::Passed
        }
        StepVerb::TypeStory(text) => type_story(&mut ctx, &session, text).await,
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
                    Some(storyfella_studio_core::registry::pipeline::DEFAULT_MODEL.to_owned()),
                    storyfella_studio_core::registry::RevisionOrigin::Canon,
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
                    entry.status = storyfella_studio_core::registry::RevisionStatus::Generating;
                }
            }
            // Long enough to read the badge, short enough not to stall.
            tokio::time::sleep(std::time::Duration::from_millis(2200)).await;
            match existing {
                Some(path) => {
                    let _ = ctx
                        .app_state
                        .write()
                        .registry
                        .finish_revision(id, revision, path);
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
            ctx.screen.set(AppScreen::Canvas);
            composer.set(String::new());
            if !type_text(&session, text, |typed| composer.set(typed.to_owned())).await {
                return StepStatus::Failed {
                    reason: "the run was cancelled while typing".to_owned(),
                };
            }
            let guide_mode = session
                .read()
                .as_ref()
                .is_some_and(|s| s.mode == GuideMode::Guide);
            if !wait_with_pause(&session, if guide_mode { 120 } else { 0 }).await {
                return StepStatus::Failed {
                    reason: "the run was cancelled before submission".to_owned(),
                };
            }
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
                    state.project_name.eq_ignore_ascii_case(&expected)
                        && state.project_path.is_some()
                },
                STEP_TIMEOUT_MS,
            )
            .await;
            status(ok, format!("expected the open story to be \"{name}\""))
        }
        StepVerb::ExpectStorySequence(expected) => {
            let registry = ctx.app_state.read().registry.clone();
            let actual: Vec<String> = registry
                .story_scenes
                .iter()
                .flat_map(|scene| {
                    std::iter::once(scene.name.clone()).chain(
                        scene
                            .beats
                            .iter()
                            .map(|beat| format!("{}.{}", scene.name, beat.name)),
                    )
                })
                .collect();
            let valid = storyfella_studio_core::authoring::validate_associations(&registry).is_ok();
            status(
                valid && &actual == expected,
                format!("expected {expected:?}, got {actual:?}"),
            )
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
                        .and_then(|artifact| state.registry.active_revision(artifact.id))
                        .and_then(|revision| revision.asset_path.as_ref())
                        .map(|path| Path::new(path).exists())
                        .unwrap_or(false);
                    has_image
                },
                IMAGE_TIMEOUT_MS,
            )
            .await;
            status(
                ok,
                format!("expected \"{name}\" to have a finished active image"),
            )
        }
        StepVerb::ExpectVision {
            image,
            question,
            expect,
            expect_not,
        } => {
            let Some(deployment) = ctx.config.vision.clone() else {
                return StepStatus::Failed {
                    reason: "no `vision` provider is configured in storyfella.toml".to_owned(),
                };
            };
            if vision.is_none() {
                match vision::VisionSession::start(deployment).await {
                    Ok(started) => *vision = Some(started),
                    Err(error) => {
                        let report = error.report().log();
                        return StepStatus::Failed {
                            reason: format!("{} — {}", report.code.code(), report.details),
                        };
                    }
                }
            }
            let session = vision.as_ref().expect("the session was just started");

            // The image currently shown for the artifact (its active revision).
            let path = {
                let state = ctx.app_state.read();
                let artifact = state
                    .registry
                    .iter()
                    .find(|artifact| artifact.name.eq_ignore_ascii_case(image));
                artifact.and_then(|artifact| state.display_image(artifact).map(str::to_owned))
            };
            let Some(path) = path else {
                return StepStatus::Failed {
                    reason: format!("artifact \"{image}\" has no image to assess"),
                };
            };
            let attachment = match vision::image_for_chat(Path::new(&path)) {
                Ok(attachment) => attachment,
                Err(error) => {
                    let report = error.report().log();
                    return StepStatus::Failed {
                        reason: format!("{} — {}", report.code.code(), report.details),
                    };
                }
            };
            let answer = match session
                .assess(vec![attachment], question, vision::DEFAULT_MAX_TOKENS)
                .await
            {
                Ok(answer) => answer,
                Err(error) => {
                    let report = error.report().log();
                    return StepStatus::Failed {
                        reason: format!("{} — {}", report.code.code(), report.details),
                    };
                }
            };
            match vision::check(&answer, expect.as_deref(), expect_not.as_deref()) {
                Ok(()) => StepStatus::Passed,
                Err(reason) => StepStatus::Failed {
                    reason: format!("{reason} (model said: {answer})"),
                },
            }
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
async fn wait_for_op(ui: ExecutionUi, mut session: Signal<Option<GuideSession>>) -> StepStatus {
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
async fn wait_with_pause(session: &Signal<Option<GuideSession>>, wait_ms: u64) -> bool {
    let mut remaining = wait_ms;
    loop {
        let control = session.read().as_ref().map(|s| (s.paused, s.speed));
        let Some((paused, speed)) = control else {
            return false;
        };
        if paused {
            tokio::time::sleep(Duration::from_millis(80)).await;
            continue;
        }
        if remaining == 0 {
            return true;
        }
        let speed = u64::from(speed.max(1));
        let slice = remaining.div_ceil(speed).min(POLL_INTERVAL_MS);
        tokio::time::sleep(Duration::from_millis(slice)).await;
        remaining = remaining.saturating_sub(slice * speed);
    }
}

async fn type_story(
    ctx: &mut DriveContext,
    session: &Signal<Option<GuideSession>>,
    text: &str,
) -> StepStatus {
    ctx.screen.set(AppScreen::Writer);
    let original = ctx.app_state.read().story.source().to_owned();
    let completed = type_text(session, text, |typed| {
        let source = format!("{original}{typed}");
        let position = storyfella_language::Source::new(&source)
            .position(source.len())
            .unwrap();
        ctx.app_state.write().set_story_source(source);
        ctx.writer_caret.navigate(position);
    })
    .await;
    status(completed, "the run was cancelled while typing".to_owned())
}

/// Emit UTF-8-safe prefixes through the same state setters as manual edits.
/// Check mode applies the text once; Guide mode reads live controls per character.
async fn type_text(
    session: &Signal<Option<GuideSession>>,
    text: &str,
    mut emit: impl FnMut(&str),
) -> bool {
    if !wait_with_pause(session, 0).await {
        return false;
    }
    if session
        .read()
        .as_ref()
        .is_some_and(|s| s.mode == GuideMode::Check)
    {
        emit(text);
        return true;
    }
    for (offset, character) in text.char_indices() {
        loop {
            if !wait_with_pause(session, 0).await {
                return false;
            }
            let Some(rate) = session.read().as_ref().map(|s| s.typing_cps) else {
                return false;
            };
            tokio::time::sleep(Duration::from_secs_f64(1.0 / f64::from(rate.max(1)))).await;
            let active = session.read();
            let Some(active) = active.as_ref() else {
                return false;
            };
            if !active.paused {
                break;
            }
        }
        emit(&text[..offset + character.len_utf8()]);
    }
    true
}

#[cfg(test)]
mod tests {
    /// Every shipped plan must parse — a typo in a step key is otherwise
    /// only discovered when someone tries to run it.
    #[test]
    fn shipped_plans_parse() {
        let dir = storyfella_studio_core::paths::plans_dir();
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

    #[test]
    fn typing_settings_are_validated() {
        let default = Plan::from_yaml("name: typing\nsteps: []\n").unwrap();
        assert_eq!(default.typing_cps, 60);
        for rate in [1, 200] {
            let plan =
                Plan::from_yaml(&format!("name: typing\ntyping_cps: {rate}\nsteps: []\n")).unwrap();
            assert_eq!(plan.typing_cps, rate);
        }
        for rate in ["0", "201", "-1", "fast"] {
            assert!(
                Plan::from_yaml(&format!("name: typing\ntyping_cps: {rate}\nsteps: []\n")).is_err()
            );
        }
        let error = Plan::from_yaml("name: typing\ntyping_cp: 20\nsteps: []\n").unwrap_err();
        assert!(error.contains("typing_cp"), "{error}");
        assert!(error.contains("unknown field"), "{error}");
    }

    #[test]
    fn tour_appends_valid_source() {
        let tour = Plan::from_yaml(include_str!("../../../../config/plans/ui-tour.yaml")).unwrap();
        let mut source = crate::app::sample::STORY.to_owned();
        for step in tour.steps {
            if let StepVerb::TypeStory(text) = step.verb {
                source.push_str(&text);
            }
        }
        let analysis = storyfella_language::analyze(&source, &[]);
        assert!(source.contains("  beat signal\n"));
        assert!(
            !analysis
                .diagnostics
                .iter()
                .any(|d| d.severity == storyfella_language::Severity::Error),
            "{:?}",
            analysis.diagnostics
        );
    }

    fn typing_session(mode: GuideMode) -> GuideSession {
        GuideSession {
            plan: Plan::from_yaml("name: typing\nsteps: []\n").unwrap(),
            mode,
            current: 0,
            busy: None,
            paused: false,
            speed: 1,
            typing_cps: 200,
            state: GuideState::Running,
        }
    }

    #[component]
    fn TypingProbe() -> Element {
        let mut session = use_signal(|| Some(typing_session(GuideMode::Guide)));
        use_hook(move || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_time()
                .build()
                .unwrap();
            runtime.block_on(async {
                let mut prefixes = Vec::new();
                assert!(
                    type_text(&session, "é🌊\r\n", |text| prefixes
                        .push(text.to_owned()))
                    .await
                );
                assert_eq!(prefixes, ["é", "é🌊", "é🌊\r", "é🌊\r\n"]);

                // Check mode has no character delay: its first poll can only
                // remain pending without emitting text if pause is honored.
                session.set(Some(typing_session(GuideMode::Check)));
                session.with_mut(|s| s.as_mut().unwrap().paused = true);
                let count = std::rc::Rc::new(std::cell::Cell::new(0));
                let observed = count.clone();
                let mut controller = session;
                let mut typing = Box::pin(type_text(&session, "ab", move |_| {
                    observed.set(observed.get() + 1);
                }));
                std::future::poll_fn(|context| {
                    use std::future::Future;
                    assert!(typing.as_mut().poll(context).is_pending());
                    assert_eq!(count.get(), 0);
                    controller.with_mut(|s| s.as_mut().unwrap().paused = false);
                    std::task::Poll::Ready(())
                })
                .await;
                assert!(typing.await);
                assert_eq!(count.get(), 1);

                // Stop preserves the emitted prefix and never emits the rest.
                session.set(Some(typing_session(GuideMode::Guide)));
                let mut partial = String::new();
                let mut cancelled_session = session;
                assert!(
                    !type_text(&session, "é🌊", |text| {
                        partial = text.to_owned();
                        cancelled_session.set(None);
                    })
                    .await
                );
                assert_eq!(partial, "é");

                // Check mode emits the exact complete string once, without animation.
                session.set(Some(typing_session(GuideMode::Check)));
                let mut emissions = Vec::new();
                assert!(
                    type_text(&session, "é🌊\r\n", |text| emissions
                        .push(text.to_owned()))
                    .await
                );
                assert_eq!(emissions, ["é🌊\r\n"]);
            });
        });
        rsx! {}
    }

    #[test]
    fn typing_obeys_pause_stop_and_mode_without_corrupting_unicode() {
        let mut dom = VirtualDom::new(TypingProbe);
        dom.rebuild_in_place();
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
    fn expect_vision_parses_its_mapping() {
        let plan = Plan::from_yaml(
            r#"
name: vision
steps:
  - expect_vision:
      image: "mara"
      question: "Is her hair auburn?"
      expect: "yes"
      expect_not: "no"
"#,
        )
        .expect("plan parses");
        assert_eq!(
            plan.steps[0].verb,
            StepVerb::ExpectVision {
                image: "mara".into(),
                question: "Is her hair auburn?".into(),
                expect: Some("yes".into()),
                expect_not: Some("no".into()),
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
