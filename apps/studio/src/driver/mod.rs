//! The automation driver — a procedural user for the app.
//!
//! A plan is a sequence of steps written in the app's own language (the
//! closed vocabulary below), loaded from YAML files in `config/plans/`.
//! Plans never mention selectors, coordinates, or DOM — each step is an
//! intent the app executes through the same code paths the UI uses
//! (`crate::commands`), and outcomes are asserted against app state.
//!
//! Two run modes:
//! - `Guide` — slow and narrated: the window shows a guide card with the
//!   step message, waits, then watches the app perform the action.
//! - `Check` — fast: the same steps run back to back, expectations gate
//!   progress, and a pass/fail report is produced. This is the regression
//!   suite — a plan that passes means the flows it covers still work.
//!
//! Plans support authoring and saved playback. Visual production commands
//! are rejected on load, before any plan step can change the project.

use std::{path::PathBuf, str::FromStr};

use serde::Deserialize;

use storyfella_studio_core::config::AppConfig;
use storyfella_studio_core::registry::ArtifactKind;

mod run;
pub use run::run_plan;

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

/// Authoring and playback guides. Visual production commands are rejected.
#[derive(Clone, Debug, PartialEq)]
pub enum StepVerb {
    /// Switch screens (the rail actions).
    GoProjects,
    GoStory,
    GoAssets,
    GoSettings,
    /// Start a new story (same path as the Projects screen form).
    CreateProject(String),
    /// Open a recent project by its story name.
    OpenProject(String),
    /// Create the populated sample project (`app::sample`), so a tour can
    /// show full screens without waiting on a backend. Guide mode types its
    /// entire source in the Writer; Check mode applies it immediately.
    SampleProject,
    /// Select an artifact by name — the asset detail and inspector.
    Open(String),
    /// Return Assets to the grid.
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
    /// Show or hide the command palette.
    Palette(bool),
    Command(String),
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
    /// for the selected vision provider.
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
    pub artwork_revision: Option<uuid::Uuid>,
    pub staged: Option<std::collections::BTreeMap<String, StagedExpectation>>,
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StagedExpectation {
    pub asset: String,
    pub revision: Option<uuid::Uuid>,
}

impl StepVerb {
    fn label(&self) -> String {
        match self {
            Self::GoProjects => "open Projects".to_owned(),
            Self::GoStory => "open Story".to_owned(),
            Self::GoAssets => "open Assets".to_owned(),
            Self::GoSettings => "open Settings".to_owned(),
            Self::CreateProject(name) => format!("create story \"{name}\""),
            Self::OpenProject(name) => format!("open story \"{name}\""),
            Self::SampleProject => "create the sample project".to_owned(),
            Self::Open(name) => format!("open artifact \"{name}\""),
            Self::Back => "back to the grid".to_owned(),
            Self::Say => String::new(),
            Self::Caret(line) => format!("caret to line {line}"),
            Self::TypeStory(_) => "type in the Writer".to_owned(),
            Self::Palette(true) => "open the palette".to_owned(),
            Self::Palette(false) => "close the palette".to_owned(),
            Self::Command(command) => command.clone(),
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
        let kind = ArtifactKind::from_str;
        Ok(match action {
            "go_projects" => Self::GoProjects,
            "go_story" => Self::GoStory,
            "go_assets" => Self::GoAssets,
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
            "type_story" => Self::TypeStory(text(&value)?),
            "palette" => Self::Palette(value.as_bool().unwrap_or(true)),
            "go_settings" => Self::GoSettings,
            "create_project" => Self::CreateProject(text(&value)?),
            "open_project" => Self::OpenProject(text(&value)?),
            "command" => Self::Command(text(&value)?),
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
        let plan: Self = serde_yaml::from_str(yaml)
            .map_err(|error| format!("plan is not valid YAML: {error}"))?;
        for step in &plan.steps {
            if let StepVerb::Command(command) = &step.verb {
                let operation = storyfella_studio_core::registry::slash::parse_slash(command)
                    .map_err(|error| error.to_string())?;
                crate::commands::validate_studio_operations(&[operation])
                    .map_err(|error| error.details)?;
            }
        }
        Ok(plan)
    }
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
    pub window_size: Option<(u32, u32)>,
    pub keep_window: bool,
    pub manual_save: bool,
}

impl PlanLaunch {
    pub fn from_args(args: &[String]) -> Self {
        let mut path = None;
        let mut mode = GuideMode::Guide;
        let mut window_size = None;
        let mut keep_window = false;
        let mut manual_save = false;
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
                "--window-size" => {
                    i += 1;
                    window_size = args
                        .get(i)
                        .and_then(|value| value.split_once('x'))
                        .and_then(|(width, height)| {
                            Some((width.parse().ok()?, height.parse().ok()?))
                        })
                        .filter(|(width, height)| {
                            (320..=4096).contains(width) && (320..=4096).contains(height)
                        });
                }
                "--keep-window" => keep_window = true,
                "--manual-save" => manual_save = true,
                _ => {}
            }
            i += 1;
        }
        Self {
            window_size: path.as_ref().and(window_size),
            keep_window: path.is_some() && keep_window,
            manual_save: path.is_some() && manual_save,
            path,
            mode,
        }
    }

    /// Points a `--plan` run at scratch config files and a scratch native-server
    /// state directory under `.svs-e2e/hermetic/`, so the driver never reads or
    /// writes the developer's real machine-local settings — neither this app's,
    /// the shared inference configuration, nor the shared inference service state.
    ///
    /// The files need not exist: a missing app overlay contributes nothing, and a
    /// missing `clipforge.toml` represents an empty registry, so the run is
    /// self-contained. An explicit `$SVS_USER_CONFIG`/`$SVS_AI_CONFIG`/
    /// `$SVS_STATE_DIR` already in the environment wins, so a run can still be
    /// aimed at a specific config.
    pub fn make_hermetic(&self) {
        if self.path.is_none() {
            return;
        }
        let dir = storyfella_studio_core::paths::root().join(".svs-e2e/hermetic");
        set_env_unless_set(
            storyfella_studio_core::paths::APP_CONFIG_ENV,
            dir.join("studio.toml"),
        );
        set_env_unless_set(
            storyfella_studio_core::paths::AI_CONFIG_ENV,
            dir.join("clipforge.toml"),
        );
        set_env_unless_set(
            storyfella_studio_core::paths::STATE_DIR_ENV,
            dir.join("inference-state"),
        );
    }

    /// The `--plan` to run at startup, with how its run ends: a Check run
    /// prints its verdict and exits unless `--keep-window` is given; a Guide
    /// run shows its report in the window.
    pub fn plan(&self) -> Option<(Result<Plan, String>, GuideMode, PlanCompletion)> {
        let path = self.path.as_ref()?;
        let plan = std::fs::read_to_string(path)
            .map_err(|error| format!("cannot read {}: {error}", path.display()))
            .and_then(|yaml| Plan::from_yaml(&yaml));
        let completion = match (self.mode, self.keep_window) {
            (GuideMode::Guide, _) => PlanCompletion::ShowReport,
            (GuideMode::Check, false) => PlanCompletion::PrintAndExit,
            (GuideMode::Check, true) => PlanCompletion::PrintAndKeepWindow,
        };
        Some((plan, self.mode, completion))
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

#[derive(Clone, Copy)]
pub enum PlanCompletion {
    ShowReport,
    PrintAndExit,
    PrintAndKeepWindow,
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

    #[test]
    fn plans_reject_visual_commands_before_running_any_steps() {
        let error = Plan::from_yaml(
            "name: forbidden\nsteps:\n  - create_project: untouched\n  - command: '/modify c:mara blue coat'\n",
        ).unwrap_err();
        assert!(error.contains("Atelier"));
        for removed in ["compose", "simulate_render"] {
            assert!(
                Plan::from_yaml(&format!("name: removed\nsteps:\n  - {removed}: mara\n")).is_err()
            );
        }
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
        let mut source = crate::sample::STORY.to_owned();
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
  - go_assets: true
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
