//! Executing a plan: each step acts on the app through a [`Mailbox`], from a
//! thread of its own, so pacing, polling and vision calls never block the UI.
use std::{
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use storyfella_studio_core::config::AppConfig;
use storyfella_studio_core::errors::CodedError;
use storyfella_studio_core::persistence::ProjectDb;
use storyfella_studio_core::playback::PlaybackModel;
use storyfella_studio_core::vision;

use super::{
    GuideMode, GuideSession, GuideState, Plan, PlanCompletion, PlaybackExpectation, RunReport,
    StagedExpectation, Step, StepStatus, StepVerb,
};
use crate::{
    commands::{self, OpActivity},
    host::{with_playback, Driven, Mailbox, Place},
    state::AppState,
};

const POLL_INTERVAL_MS: u64 = 80;
const STEP_TIMEOUT_MS: u64 = 30_000;
const OP_TIMEOUT_MS: u64 = 300_000;
const IMAGE_TIMEOUT_MS: u64 = 300_000;

/// Installs the guide session, then executes the steps on a driver thread.
/// CLI launches can print the verdict and either exit or leave the window open.
pub fn run_plan(
    app: &mut dyn Driven,
    host: Mailbox,
    plan: Plan,
    mode: GuideMode,
    completion: PlanCompletion,
) {
    *app.guide() = Some(GuideSession {
        plan: plan.clone(),
        mode,
        current: 0,
        busy: None,
        paused: false,
        speed: 1,
        typing_cps: plan.typing_cps,
        state: GuideState::Running,
    });
    let config = app.config().clone();
    std::thread::Builder::new()
        .name("plan driver".into())
        .spawn(move || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("plan driver runtime");
            runtime.block_on(async move {
                let report = run_steps(&host, &config, &plan, mode).await;
                complete(&host, report, completion).await;
            });
        })
        .expect("start the plan driver");
}

async fn complete(host: &Mailbox, report: RunReport, completion: PlanCompletion) {
    match completion {
        PlanCompletion::PrintAndExit => {
            // The process exits right after the report, before autosave could
            // run, so the final state is persisted here.
            let saved = host
                .call(|app| commands::save_project(app.state()))
                .await
                .unwrap_or(Ok(()));
            if let Err(error) = saved {
                println!(
                    "warning: could not save the project before exit: {}",
                    error.details
                );
            }
            report.print();
            std::process::exit(if report.failed() > 0 { 1 } else { 0 });
        }
        PlanCompletion::PrintAndKeepWindow => {
            report.print();
            host.call(|app| *app.guide() = None).await;
        }
        PlanCompletion::ShowReport => {
            host.call(move |app| {
                if let Some(session) = app.guide() {
                    session.state = GuideState::Done(report);
                }
            })
            .await;
        }
    }
}

/// Whether the run is still wanted: Stop clears the session.
async fn active(host: &Mailbox) -> bool {
    host.call(|app| app.guide().is_some())
        .await
        .unwrap_or(false)
}

async fn run_steps(host: &Mailbox, config: &AppConfig, plan: &Plan, mode: GuideMode) -> RunReport {
    let started = Instant::now();
    let mut results: Vec<(String, StepStatus)> = Vec::new();
    // The vision session (if a step needs one) lives for the whole run, so a
    // managed server is started once rather than per assessment.
    let mut vision: Option<vision::VisionSession> = None;
    for (index, step) in plan.steps.iter().enumerate() {
        let started = host
            .call(move |app| {
                let Some(session) = app.guide() else {
                    return false;
                };
                session.current = index;
                session.state = GuideState::Running;
                true
            })
            .await
            .unwrap_or(false);
        if !started {
            break;
        }
        if mode == GuideMode::Guide {
            let wait = step.wait_ms.or(plan.speed_ms).unwrap_or(2_000);
            wait_with_pause(host, wait).await;
        }
        if !active(host).await {
            break;
        }
        let mut status = execute_step(host, config, step, &mut vision).await;
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
    host.call(|app| {
        if let Some(session) = app.guide() {
            session.busy = None;
        }
    })
    .await;
    RunReport {
        plan_name: plan.name.clone(),
        steps: results,
        duration_ms: started.elapsed().as_millis() as u64,
    }
}

/// Runs `act` on the app; a closed app fails the step.
async fn on_app(
    host: &Mailbox,
    act: impl FnOnce(&mut dyn Driven) -> StepStatus + Send + 'static,
) -> StepStatus {
    host.call(act).await.unwrap_or_else(|| StepStatus::Failed {
        reason: "Studio closed during the run".into(),
    })
}

fn playback_status(model: &PlaybackModel) -> StepStatus {
    let error = model.error.clone();
    status(error.is_none(), error.unwrap_or_default())
}

async fn execute_step(
    host: &Mailbox,
    config: &AppConfig,
    step: &Step,
    vision: &mut Option<vision::VisionSession>,
) -> StepStatus {
    match step.verb.clone() {
        StepVerb::OpenProjectFile(path) => {
            let path = storyfella_studio_core::paths::root().join(path);
            on_app(host, move |app| {
                match commands::open_project(&path, app.state()) {
                    Ok(()) => {
                        app.show(Place::Write);
                        StepStatus::Passed
                    }
                    Err(error) => StepStatus::Failed {
                        reason: error.details,
                    },
                }
            })
            .await
        }
        StepVerb::Preview => {
            on_app(host, |app| {
                let session = crate::preview::compile(app.state());
                app.install_playback(session);
                app.show(Place::Play);
                with_playback(app, |model| playback_status(model))
            })
            .await
        }
        StepVerb::Playback(command) => {
            on_app(host, move |app| {
                with_playback(app, |model| {
                    model.dispatch(command);
                    playback_status(model)
                })
            })
            .await
        }
        StepVerb::PlaybackBookmark => {
            on_app(host, |app| {
                with_playback(app, PlaybackModel::bookmark);
                StepStatus::Passed
            })
            .await
        }
        StepVerb::PlaybackRestore => {
            on_app(host, |app| {
                with_playback(app, |model| {
                    model.restore_bookmark();
                    playback_status(model)
                })
            })
            .await
        }
        StepVerb::ExpectPlayback(expected) => {
            on_app(host, move |app| {
                with_playback(app, |model| check_playback(model, &expected))
            })
            .await
        }
        StepVerb::GoProjects => show(host, Place::Home).await,
        StepVerb::GoStory => show(host, Place::Write).await,
        StepVerb::GoAssets => show(host, Place::Board).await,
        StepVerb::GoSettings => show(host, Place::Settings).await,
        StepVerb::SampleProject => {
            let project_dir = config.project_dir.clone();
            let asset_dir = config.asset_dir.clone();
            let story = on_app_value(host, move |app| {
                commands::create_project(&project_dir, "Keeper of the Low Light", app.state())
                    .map(|_| {
                        let (registry, story) = crate::sample::project(&asset_dir);
                        let state = app.state();
                        state.registry = registry;
                        state.set_story_source(String::new());
                        state.has_unsaved_changes = true;
                        app.set_error(None);
                        story
                    })
                    .map_err(|report| report.details)
            })
            .await;
            match story {
                Ok(story) => type_story(host, &story).await,
                Err(reason) => StepStatus::Failed { reason },
            }
        }
        StepVerb::Open(name) => {
            on_app(host, move |app| match find_asset(app.state(), &name) {
                Some(id) => {
                    app.state().select_artifact(id);
                    app.show(Place::Board);
                    StepStatus::Passed
                }
                None => StepStatus::Failed {
                    reason: format!("no artifact named {name:?}"),
                },
            })
            .await
        }
        StepVerb::Back => {
            on_app(host, |app| {
                app.state().clear_selection();
                StepStatus::Passed
            })
            .await
        }
        StepVerb::Say => StepStatus::Passed,
        StepVerb::Caret(line) => {
            on_app(host, move |app| {
                app.set_caret(storyfella_language::Position {
                    line: line as u32,
                    character: 0,
                });
                app.show(Place::Write);
                StepStatus::Passed
            })
            .await
        }
        StepVerb::TypeStory(text) => type_story(host, &text).await,
        StepVerb::Palette(open) => {
            on_app(host, move |app| {
                app.set_palette(open);
                StepStatus::Passed
            })
            .await
        }
        StepVerb::CreateProject(name) => {
            let project_dir = config.project_dir.clone();
            on_app(host, move |app| {
                match commands::create_project(&project_dir, &name, app.state()) {
                    Ok(_) => {
                        app.show(Place::Write);
                        app.set_error(None);
                        StepStatus::Passed
                    }
                    Err(report) => StepStatus::Failed {
                        reason: report.details,
                    },
                }
            })
            .await
        }
        StepVerb::OpenProject(name) => match find_project_by_name(&config.project_dir, &name) {
            Some(path) => {
                on_app(host, move |app| {
                    match commands::open_project(&path, app.state()) {
                        Ok(()) => {
                            app.show(Place::Write);
                            StepStatus::Passed
                        }
                        Err(report) => StepStatus::Failed {
                            reason: report.details,
                        },
                    }
                })
                .await
            }
            None => StepStatus::Failed {
                reason: format!("no project named \"{name}\" in the projects directory"),
            },
        },
        StepVerb::Command(command) => {
            let started = on_app(host, move |app| {
                let defaults = app.config().generation_defaults.clone();
                match commands::begin_slash_command(app.state(), defaults, &command) {
                    Ok(pending) => {
                        app.set_error(None);
                        app.start(pending);
                        StepStatus::Passed
                    }
                    Err(report) => StepStatus::Failed {
                        reason: report.details,
                    },
                }
            })
            .await;
            if started.is_failure() {
                started
            } else {
                wait_for_op(host).await
            }
        }
        StepVerb::ExpectProject(name) => {
            let expected = name.clone();
            let ok = poll_until(
                host,
                move |state| {
                    state.project_name.eq_ignore_ascii_case(&expected)
                        && state.project_path.is_some()
                },
                STEP_TIMEOUT_MS,
            )
            .await;
            status(ok, format!("expected the open story to be \"{name}\""))
        }
        StepVerb::ExpectStorySequence(expected) => {
            on_app(host, move |app| {
                let registry = &app.state().registry;
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
                let valid =
                    storyfella_studio_core::authoring::validate_associations(registry).is_ok();
                status(
                    valid && actual == expected,
                    format!("expected {expected:?}, got {actual:?}"),
                )
            })
            .await
        }
        StepVerb::ExpectArtifact { name, kind } => {
            let expected = name.clone();
            let ok = poll_until(
                host,
                move |state| {
                    state.registry.iter().any(|artifact| {
                        artifact.name.eq_ignore_ascii_case(&expected)
                            && kind.is_none_or(|kind| artifact.kind == kind)
                    })
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
            let ok = poll_until(
                host,
                move |state| {
                    state
                        .registry
                        .iter()
                        .filter(|artifact| artifact.kind == kind)
                        .count()
                        == count
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
            let ok = poll_until(
                host,
                move |state| {
                    state
                        .registry
                        .iter()
                        .find(|artifact| artifact.name.eq_ignore_ascii_case(&expected))
                        .and_then(|artifact| state.registry.active_revision(artifact.id))
                        .and_then(|revision| revision.asset_path.as_ref())
                        .is_some_and(|path| Path::new(path).exists())
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
            let Some(deployment) = config.vision.clone() else {
                return StepStatus::Failed {
                    reason: "no `vision` provider is configured in clipforge.toml".to_owned(),
                };
            };
            let session = vision
                .get_or_insert_with(|| vision::VisionSession::new(deployment, &config.state_dir));
            // The image currently shown for the artifact (its active revision).
            let wanted = image.clone();
            let path = host
                .call(move |app| {
                    let state = app.state();
                    state
                        .registry
                        .iter()
                        .find(|artifact| artifact.name.eq_ignore_ascii_case(&wanted))
                        .and_then(|artifact| AppState::display_image(artifact).map(str::to_owned))
                })
                .await
                .flatten();
            let Some(path) = path else {
                return StepStatus::Failed {
                    reason: format!("artifact \"{image}\" has no image to assess"),
                };
            };
            let attachment = match vision::image_for_chat(Path::new(&path)) {
                Ok(attachment) => attachment,
                Err(error) => return coded_failure(error.report().log()),
            };
            let answer = match session
                .assess(vec![attachment], &question, vision::DEFAULT_MAX_TOKENS)
                .await
            {
                Ok(answer) => answer,
                Err(error) => return coded_failure(error.report().log()),
            };
            match vision::check(&answer, expect.as_deref(), expect_not.as_deref()) {
                Ok(()) => StepStatus::Passed,
                Err(reason) => StepStatus::Failed {
                    reason: format!("{reason} (model said: {answer})"),
                },
            }
        }
        StepVerb::ExpectNoError => {
            on_app(host, |app| {
                status(app.error().is_none(), "expected no error report".to_owned())
            })
            .await
        }
    }
}

async fn on_app_value<R: Send + 'static>(
    host: &Mailbox,
    act: impl FnOnce(&mut dyn Driven) -> Result<R, String> + Send + 'static,
) -> Result<R, String> {
    host.call(act)
        .await
        .unwrap_or_else(|| Err("Studio closed during the run".into()))
}

async fn show(host: &Mailbox, place: Place) -> StepStatus {
    on_app(host, move |app| {
        app.show(place);
        StepStatus::Passed
    })
    .await
}

fn find_asset(state: &AppState, name: &str) -> Option<uuid::Uuid> {
    state
        .registry
        .artifacts
        .iter()
        .find(|artifact| artifact.name.eq_ignore_ascii_case(name))
        .map(|artifact| artifact.id)
}

fn coded_failure(report: storyfella_studio_core::errors::ReportedError) -> StepStatus {
    StepStatus::Failed {
        reason: format!("{} — {}", report.code.code(), report.details),
    }
}

fn check_playback(model: &PlaybackModel, expected: &PlaybackExpectation) -> StepStatus {
    let Some(session) = &model.session else {
        return StepStatus::Failed {
            reason: "no playback session".into(),
        };
    };
    let runtime = &session.runtime;
    let text = match &runtime.current().kind {
        storyfella_compiler::InstructionKind::Speech { text, .. }
        | storyfella_compiler::InstructionKind::Narration { text } => Some(text.value.as_str()),
        _ => None,
    };
    let visuals = session.active_visuals();
    let staged: std::collections::BTreeMap<_, _> = runtime
        .staged()
        .iter()
        .map(|actor| {
            let revision = session
                .staged_visual(actor)
                .and_then(|visual| visual.binding.revision.parse().ok());
            (
                actor.instance.clone(),
                StagedExpectation {
                    asset: actor.asset.name.value.clone(),
                    revision,
                },
            )
        })
        .collect();
    let valid = runtime.state().phase == expected.phase
        && runtime.plan().scenes[runtime.state().scene].id.0 == expected.scene
        && expected.text.as_deref().is_none_or(|t| Some(t) == text)
        && expected
            .staged
            .as_ref()
            .is_none_or(|expected| *expected == staged)
        && expected.artwork_revision.is_none_or(|revision| {
            visuals
                .iter()
                .any(|visual| visual.binding.revision == revision.to_string())
        })
        && visuals.len() == runtime.state().assets.len()
        && visuals
            .iter()
            .all(|v| !v.png.is_empty() && !v.binding.revision.is_empty());
    status(
        valid,
        format!(
            "expected {expected:?}; got {:?}, text {text:?}, staged {staged:?}",
            runtime.state()
        ),
    )
}

fn status(ok: bool, reason: String) -> StepStatus {
    if ok {
        StepStatus::Passed
    } else {
        StepStatus::Failed { reason }
    }
}

/// Waits for a started operation to settle back to Idle, mirroring its
/// summary on the session so the guide card shows what the app is doing.
async fn wait_for_op(host: &Mailbox) -> StepStatus {
    let deadline = Instant::now() + Duration::from_millis(OP_TIMEOUT_MS);
    let done = loop {
        let progress = host
            .call(|app| {
                let activity = app.activity();
                let session = app.guide().as_mut()?;
                session.busy = match activity {
                    OpActivity::Running { summary } => Some(summary),
                    OpActivity::Idle => None,
                };
                Some(session.busy.is_none())
            })
            .await
            .flatten();
        match progress {
            None => break false, // cancelled
            Some(true) => break true,
            Some(false) if Instant::now() >= deadline => break false,
            Some(false) => tokio::time::sleep(Duration::from_millis(POLL_INTERVAL_MS)).await,
        }
    };
    if !done {
        host.call(|app| {
            if let Some(session) = app.guide() {
                session.busy = None;
            }
        })
        .await;
        return StepStatus::Failed {
            reason: "the operation did not finish in time".to_owned(),
        };
    }
    match host.call(|app| app.error()).await.flatten() {
        Some(report) => coded_failure(report),
        None => StepStatus::Passed,
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

/// Polls a condition on the app's state until it holds or the timeout
/// expires.
async fn poll_until(
    host: &Mailbox,
    check: impl Fn(&AppState) -> bool + Clone + Send + 'static,
    timeout_ms: u64,
) -> bool {
    let deadline = Instant::now() + Duration::from_millis(timeout_ms);
    loop {
        let check = check.clone();
        let holds = host.call(move |app| check(app.state())).await;
        match holds {
            Some(true) => return true,
            None => return false,
            Some(false) if Instant::now() >= deadline => return false,
            Some(false) => tokio::time::sleep(Duration::from_millis(POLL_INTERVAL_MS)).await,
        }
    }
}

/// The guide card's live controls: `None` once the run is stopped.
async fn controls(host: &Mailbox) -> Option<(bool, u32, u16, GuideMode)> {
    host.call(|app| {
        app.guide()
            .as_ref()
            .map(|s| (s.paused, s.speed, s.typing_cps, s.mode))
    })
    .await
    .flatten()
}

/// Guide-mode narration pacing: waits the step's duration, honoring pause
/// and speed from the guide card; a stopped run returns early.
async fn wait_with_pause(host: &Mailbox, wait_ms: u64) -> bool {
    let mut remaining = wait_ms;
    loop {
        let Some((paused, speed, _, _)) = controls(host).await else {
            return false;
        };
        if paused {
            tokio::time::sleep(Duration::from_millis(POLL_INTERVAL_MS)).await;
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

async fn type_story(host: &Mailbox, text: &str) -> StepStatus {
    let original = host
        .call(|app| {
            app.show(Place::Write);
            app.state().story.source().to_owned()
        })
        .await;
    let Some(original) = original else {
        return status(false, "Studio closed during the run".into());
    };
    let completed = type_text(host, text, async |typed| {
        let source = format!("{original}{typed}");
        host.call(move |app| {
            let position = storyfella_language::Source::new(&source)
                .position(source.len())
                .expect("the end of a source is a position");
            app.state().set_story_source(source);
            app.set_caret(position);
        })
        .await;
    })
    .await;
    status(completed, "the run was cancelled while typing".to_owned())
}

/// Emits UTF-8-safe prefixes through the same state setters as manual edits.
/// Check mode applies the text once; Guide mode reads live controls per character.
async fn type_text(host: &Mailbox, text: &str, mut emit: impl AsyncFnMut(&str)) -> bool {
    if !wait_with_pause(host, 0).await {
        return false;
    }
    let Some((_, _, _, mode)) = controls(host).await else {
        return false;
    };
    if mode == GuideMode::Check {
        emit(text).await;
        return true;
    }
    for (offset, character) in text.char_indices() {
        loop {
            if !wait_with_pause(host, 0).await {
                return false;
            }
            let Some((_, _, rate, _)) = controls(host).await else {
                return false;
            };
            tokio::time::sleep(Duration::from_secs_f64(1.0 / f64::from(rate.max(1)))).await;
            match controls(host).await {
                None => return false,
                Some((false, ..)) => break,
                Some((true, ..)) => {}
            }
        }
        emit(&text[..offset + character.len_utf8()]).await;
    }
    true
}

#[cfg(test)]
mod tests;
