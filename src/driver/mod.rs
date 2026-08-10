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
    GoCanvas,
    GoSettings,
    /// Start a new story (same path as the Projects screen form).
    CreateProject(String),
    /// Open a recent project by its story name.
    OpenProject(String),
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
            Self::GoCanvas => "open Canvas".to_owned(),
            Self::GoSettings => "open Settings".to_owned(),
            Self::CreateProject(name) => format!("create story \"{name}\""),
            Self::OpenProject(name) => format!("open story \"{name}\""),
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
            "go_canvas" => Self::GoCanvas,
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
        let (action, value) =
            action.ok_or_else(|| D::Error::custom("a step needs an action, e.g. create_project: \"name\""))?;
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
    Failed { reason: String },
    /// A soft step that did not hold: reported, but not a plan failure.
    Warned { reason: String },
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
        self.steps.iter().filter(|(_, status)| status.is_failure()).count()
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
                println!("warning: could not save the project before exit: {}", error.details);
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
        StepVerb::GoCanvas => {
            ctx.screen.set(AppScreen::Canvas);
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
        StepVerb::OpenProject(name) => {
            match find_project_by_name(&ctx.config.project_dir, name) {
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
            }
        }
        StepVerb::Command(command) => {
            let backend = (ctx.backend)();
            let defaults = ctx.defaults.read().clone();
            match commands::run_slash_command(ctx.app_state, backend, defaults, ui.clone(), command) {
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
            status(ok, format!("expected an artifact named \"{name}\" in the registry"))
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
            status(ok, format!("expected exactly {count} {kind}s in the registry"))
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
async fn wait_for_op(
    mut ui: ExecutionUi,
    mut session: Signal<Option<GuideSession>>,
) -> StepStatus {
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
        let slice = (POLL_INTERVAL_MS as u64).div_ceil(active.speed.max(1) as u64).max(20);
        tokio::time::sleep(Duration::from_millis(slice)).await;
        remaining = remaining.saturating_sub(slice);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        assert_eq!(plan.steps[0].message.as_deref(), Some("Watch the composer."));
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
