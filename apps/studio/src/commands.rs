//! The intent seam — the app-owned action language.
//!
//! Everything a user can *do* to a project (create/open it, run slash
//! operations) lives here, implemented once against the registry, state,
//! and persistence layers. The canvas composer, the operation log rerun
//! action, and the automation driver (`crate::driver`) all call the same
//! entry points, so an action driven by the driver is indistinguishable
//! from the same action typed by a user.
//!
//! Execution model: operations run on a blocking worker (its own tokio
//! runtime) so a long generation never ties up the UI's async executor.
//! The registry is written back on success **and** on failure (a failed
//! revision is recorded; intermediates are kept); the stack stops at the
//! first failure (fail-fast, ops already applied stay). Checkpoints resolve
//! through [`ApprovalPolicy::auto`]: LLM draft text is accepted, and a
//! degraded LLM step falls back to its deterministic path.

use std::path::{Path, PathBuf};

use dioxus::prelude::*;
use uuid::Uuid;

use storyfella_studio_core::errors::{CodedError, ErrorCode, ReportedError};
use storyfella_studio_core::persistence::ProjectDb;
use storyfella_studio_core::registry::{
    backend::CreativeBackend,
    ops::{self, ExecuteOptions, OpOrigin, Operation},
    pipeline::{ApprovalPolicy, GenerationOverrides, RunOptions},
    slash::{self, is_slash_command},
    ArtifactRegistry, GenerationDefaults,
};

use crate::state::AppState;

/// The running-state of an operation stack (canvas composer, operation log
/// rerun, or the automation driver — all share this one signal shape).
#[derive(Clone, Debug, Default, PartialEq)]
pub enum OpActivity {
    #[default]
    Idle,
    Running {
        summary: String,
    },
}

impl OpActivity {
    pub fn is_busy(&self) -> bool {
        matches!(self, Self::Running { .. })
    }
}

/// The UI-facing surface an operation run reports through: activity and
/// errors. The canvas owns one instance; the driver owns another (its
/// failures land on its own error signal, never the global toast).
#[derive(Clone)]
pub struct ExecutionUi {
    pub activity: Signal<OpActivity>,
    pub error_report: Signal<Option<ReportedError>>,
}

impl ExecutionUi {
    /// A self-contained surface for non-interactive drivers.
    pub fn for_driver() -> Self {
        Self {
            activity: use_signal(OpActivity::default),
            error_report: use_signal(|| None::<ReportedError>),
        }
    }
}

/// Parses command text into an operation; non-slash input is rejected with
/// a hint rather than silently ignored. Static-validation failures keep
/// their precise error codes; parse failures map to the command-level code.
pub fn parse_command(
    input: &str,
    registry: &ArtifactRegistry,
    defaults: &GenerationDefaults,
) -> Result<Operation, ReportedError> {
    if !is_slash_command(input) {
        return Err(ReportedError::plain(
            ErrorCode::AppCommandInvalid,
            "Commands start with / — e.g. /create character <description>, /variant c:<ref> <description>",
        ));
    }
    let operation = slash::parse_slash(input)
        .map_err(|error| ReportedError::plain(ErrorCode::AppCommandInvalid, error.to_string()))?;
    // Static validation against the registry (ref resolution, kinds, active
    // images) so errors surface before any backend work starts.
    ops::compile(registry, &operation, defaults).map_err(|error| error.report())?;
    Ok(operation)
}

/// The one input path for slash commands: parse, statically validate, then
/// run through [`run_operations`]. Parse/validation failures are returned
/// (the caller decides how to surface them); execution is fire-and-forget —
/// completion is observed through `ui.activity`.
pub fn run_slash_command(
    app_state: Signal<AppState>,
    backend: CreativeBackend,
    defaults: GenerationDefaults,
    ui: ExecutionUi,
    input: &str,
) -> Result<(), ReportedError> {
    let operation = parse_command(input, &app_state.read().registry, &defaults)?;
    run_operations(app_state, backend, vec![operation], defaults, ui);
    Ok(())
}

/// Applies one or more operations: snapshot for undo, then execute each in
/// order against the backend on a blocking worker (its own tokio runtime)
/// so a long generation never ties up the UI's async executor.
pub fn run_operations(
    mut app_state: Signal<AppState>,
    backend: CreativeBackend,
    operations: Vec<Operation>,
    defaults: GenerationDefaults,
    mut ui: ExecutionUi,
) {
    let summary = stack_summary(&operations);
    if app_state.read().operation_in_progress() {
        ui.error_report.set(Some(ReportedError::plain(
            ErrorCode::AppCommandInvalid,
            "Wait for the current artwork operation to finish.",
        )));
        return;
    }
    app_state.write().begin_operation();
    ui.activity.set(OpActivity::Running { summary });

    let registry = app_state.read().registry.clone();
    let baseline = registry.clone();
    spawn(async move {
        let outcome = tokio::task::spawn_blocking(move || {
            let runtime = tokio::runtime::Runtime::new().expect("op worker runtime");
            run_stack(&runtime, registry, &backend, &operations, &defaults)
        })
        .await;

        // The worker moved the pre-op registry; a worker that dies without
        // reporting leaves the current state untouched.
        let result = match outcome {
            Ok((finished, result)) => {
                let mut state = app_state.write();
                state.finish_operation(&baseline, finished);
                drop(state);
                Some(result)
            }
            Err(_) => {
                app_state.write().cancel_operation();
                None
            }
        };
        ui.activity.set(OpActivity::Idle);
        match result {
            Some(Ok(outcome)) => {
                if let Some(id) = outcome.artifact_id {
                    app_state.write().selected_artifact_id = Some(id);
                }
            }
            // The single log point for op results: the worker attached the
            // request context, this is where the line is emitted.
            Some(Err(report)) => ui.error_report.set(Some(report.log())),
            None => ui.error_report.set(Some(
                ReportedError::plain(
                    ErrorCode::AppStackEmpty,
                    "the operation worker exited without a result",
                )
                .log(),
            )),
        }
    });
}

/// Executes the stack on a blocking worker, returning the (possibly partially
/// applied) registry and the outcome of the last operation attempted. A fresh
/// seed per visual op keeps regenerate from being an exact replay; pass
/// `--seed` on the CLI for golden runs.
fn run_stack(
    runtime: &tokio::runtime::Runtime,
    mut registry: ArtifactRegistry,
    backend: &CreativeBackend,
    operations: &[Operation],
    defaults: &GenerationDefaults,
) -> (ArtifactRegistry, Result<ops::OpOutcome, ReportedError>) {
    let mut result: Result<ops::OpOutcome, ReportedError> = Err(ReportedError::plain(
        ErrorCode::AppStackEmpty,
        "stack was empty",
    ));
    for operation in operations {
        let options = ExecuteOptions {
            backend: Some(backend),
            run: RunOptions {
                work_dir: std::env::temp_dir().join(format!("svs-canvas-{}", Uuid::new_v4())),
                approvals: ApprovalPolicy::auto(),
                seed: Some(random_seed()),
            },
            generation: GenerationOverrides {
                // The configured default step count (Settings → Image
                // generation) shapes every canvas op.
                steps: Some(defaults.steps),
                ..Default::default()
            },
            defaults,
            manual_text: None,
            origin: OpOrigin::User,
        };
        match runtime.block_on(ops::execute(&mut registry, operation, &options)) {
            Ok(outcome) => {
                if outcome.status == ops::OpStatus::Rejected {
                    result = Err(ReportedError::plain(
                        ErrorCode::AppCheckpointRejected,
                        "operation rejected at a checkpoint",
                    )
                    .with_context("operation", operation.summary()));
                    break;
                }
                result = Ok(outcome);
            }
            Err(error) => {
                // Attach what the op was and how it was run (seed and
                // work_dir let the failure be reproduced/inspected).
                let mut report = error
                    .report()
                    .with_context("operation", operation.summary())
                    .with_context("work_dir", options.run.work_dir.display().to_string());
                if let Some(seed) = options.run.seed {
                    report = report.with_context("seed", seed.to_string());
                }
                result = Err(report);
                break;
            }
        }
    }
    (registry, result)
}

/// The running-state label for an operation or stack.
fn stack_summary(operations: &[Operation]) -> String {
    match operations {
        [single] => single.summary(),
        [first, rest @ ..] => format!("{} (+{} more)", first.summary(), rest.len()),
        [] => "no operations".to_owned(),
    }
}

fn random_seed() -> u64 {
    let id = Uuid::new_v4();
    u64::from_be_bytes(id.as_bytes()[..8].try_into().expect("eight bytes"))
}

/// Creates the database file for a new story (unique slug path) and saves
/// the fresh registry into it before the screen switches to the canvas.
/// Returns the created path.
pub fn create_project(
    project_dir: &Path,
    story_name: &str,
    mut app_state: Signal<AppState>,
) -> Result<PathBuf, ReportedError> {
    ensure_project_idle(app_state)?;
    // An unbound Writer draft becomes this project's first source; switching
    // away from a saved project first persists its outstanding changes.
    let (registry, source) = if app_state.read().project_path.is_none() {
        let state = app_state.read();
        (state.registry.clone(), state.story.source().to_owned())
    } else {
        if app_state.read().has_unsaved_changes {
            save_project(app_state)?;
        }
        (ArtifactRegistry::default(), String::new())
    };
    std::fs::create_dir_all(project_dir)
        .map_err(|error| ReportedError::plain(ErrorCode::StoreSave, error.to_string()))?;
    let path = unique_project_path(project_dir, story_name);
    let db = ProjectDb::open(&path).map_err(|error| error.report())?;
    db.rename_project(story_name)
        .map_err(|error| error.report())?;
    db.save_registry(&registry)
        .map_err(|error| error.report())?;
    let mut stored = db.load().map_err(|error| error.report())?;
    stored.story.set_source(source);
    stored.story.save().map_err(|error| error.report())?;
    app_state.write().open_project(stored, path.clone());
    Ok(path)
}

/// Opens a project database or its `.sf` source in place. A standalone `.sf`
/// gets a companion database; the source file is never imported into SQLite.
pub fn open_project(path: &Path, mut app_state: Signal<AppState>) -> Result<(), ReportedError> {
    ensure_project_idle(app_state)?;
    if app_state.read().has_unsaved_changes {
        save_project(app_state)?;
    }
    let path = path
        .canonicalize()
        .map_err(|error| ReportedError::plain(ErrorCode::StoreOpen, error.to_string()))?;
    let project_path = if path.extension().is_some_and(|extension| extension == "sf") {
        path.with_extension("db")
    } else {
        path
    };
    let stored = ProjectDb::open(&project_path)
        .and_then(|db| db.load())
        .map_err(|error| error.report())?;
    app_state.write().open_project(stored, project_path);
    Ok(())
}

/// Bind an unbound draft to the exact destination chosen in the native dialog.
pub fn save_new_project(path: &Path, mut app_state: Signal<AppState>) -> Result<(), ReportedError> {
    ensure_project_idle(app_state)?;
    if app_state.read().project_path.is_some()
        || path.exists()
        || path.with_extension("sf").exists()
    {
        return Err(ReportedError::plain(
            ErrorCode::StoreSave,
            "Choose a new project file; existing projects and story sources are never overwritten.",
        ));
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| ReportedError::plain(ErrorCode::StoreSave, error.to_string()))?;
    }
    let registry = app_state.read().registry.clone();
    let source = app_state.read().story.source().to_owned();
    let db = ProjectDb::open(path).map_err(|error| error.report())?;
    db.rename_project(
        path.file_stem()
            .and_then(|name| name.to_str())
            .unwrap_or("Untitled"),
    )
    .map_err(|error| error.report())?;
    db.save_registry(&registry)
        .map_err(|error| error.report())?;
    let mut stored = db.load().map_err(|error| error.report())?;
    stored.story.set_source(source);
    stored.story.save().map_err(|error| error.report())?;
    app_state.write().open_project(stored, path.to_owned());
    Ok(())
}

fn ensure_project_idle(app_state: Signal<AppState>) -> Result<(), ReportedError> {
    if app_state.read().has_description_drafts() {
        return Err(ReportedError::plain(
            ErrorCode::AppCommandInvalid,
            "Apply or discard the description drafts in Canvas before switching projects.",
        ));
    }
    if app_state.read().operation_in_progress() {
        return Err(ReportedError::plain(
            ErrorCode::AppCommandInvalid,
            "Wait for artwork generation before switching projects.",
        ));
    }
    Ok(())
}

/// Saves the current registry to the open project file; clears the dirty
/// flag on success. The one persistence seam: the autosave effect, the
/// Cmd/Ctrl+S shortcut, and the driver's check-mode exit all use it.
/// An unbound dirty draft must be assigned a project before it can be saved.
pub fn save_project(mut app_state: Signal<AppState>) -> Result<(), ReportedError> {
    let Some(path) = app_state.read().project_path.clone() else {
        if app_state.read().has_unsaved_changes {
            return Err(storyfella_studio_core::story::StoryFileError::Unbound.report());
        }
        return Ok(());
    };
    let mut story = app_state.read().story.clone();
    story.save().map_err(|error| error.report())?;
    // A later registry-save failure must not make the successful source save
    // look like an external edit on the next attempt.
    if app_state.read().story != story {
        app_state.write().adopt_story_document(story);
    }
    let registry = app_state.read().registry.clone();
    ProjectDb::open(&path)
        .and_then(|db| db.save_registry(&registry))
        .map_err(|error| error.report())?;
    app_state.write().mark_saved();
    Ok(())
}

/// Called only after the Writer's explicit discard confirmation when dirty.
pub fn reload_story(mut app_state: Signal<AppState>) -> Result<(), ReportedError> {
    let mut story = app_state.read().story.clone();
    story.reload().map_err(|error| error.report())?;
    app_state.write().adopt_story_document(story);
    Ok(())
}

/// `"My Story"` → `my-story.svs-project.db`; an existing file gets a `-2`,
/// `-3`, … suffix.
pub fn unique_project_path(project_dir: &Path, name: &str) -> PathBuf {
    let slug: String = name
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() {
                character.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();
    let slug = slug.trim_matches('-').to_owned();
    let slug = if slug.is_empty() {
        "story".to_owned()
    } else {
        slug
    };
    let mut candidate = project_dir.join(format!("{slug}.svs-project.db"));
    let mut index = 2;
    while candidate.exists() || storyfella_studio_core::story::source_path(&candidate).exists() {
        candidate = project_dir.join(format!("{slug}-{index}.svs-project.db"));
        index += 1;
    }
    candidate
}

#[cfg(test)]
mod tests {
    use super::unique_project_path;
    use dioxus::prelude::*;
    use std::path::PathBuf;

    use storyfella_studio_core::registry::{ops::Operation, ArtifactKind, Ref, VariantAxis};

    #[component]
    fn PersistenceProbe(directory: PathBuf) -> Element {
        let mut state = use_signal(crate::state::AppState::default);
        let mut unbound = use_signal(crate::state::AppState::default);
        use_hook(move || {
            unbound
                .write()
                .set_story_source("scene exact\n  end\n".into());
            let exact = directory.join("Exact chosen filename.db");
            super::save_new_project(&exact, unbound).unwrap();
            assert_eq!(
                unbound.read().project_path.as_deref(),
                Some(exact.as_path())
            );
            assert_eq!(
                std::fs::read_to_string(exact.with_extension("sf")).unwrap(),
                "scene exact\n  end\n"
            );
            assert!(!unbound.read().has_unsaved_changes);
            assert!(super::save_new_project(&exact, state).is_err());
            let collision = directory.join("Existing source.db");
            let existing_source = collision.with_extension("sf");
            std::fs::write(&existing_source, "scene existing\n  end\n").unwrap();
            assert!(super::save_new_project(&collision, state).is_err());
            assert!(!collision.exists());
            assert_eq!(
                std::fs::read_to_string(existing_source).unwrap(),
                "scene existing\n  end\n"
            );
            // Starting a project preserves an unbound Writer draft.
            state
                .write()
                .set_story_source("scene draft\r\n   unfinished 🌊\r\n".into());
            let project = super::create_project(&directory, "Draft", state).unwrap();
            let source = state.read().story.path().unwrap().to_owned();
            assert_eq!(
                std::fs::read_to_string(&source).unwrap(),
                state.read().story.source()
            );
            assert!(!state.read().has_unsaved_changes);

            // An external edit prevents both saving and switching projects.
            state.write().set_story_source("local edits".into());
            std::fs::write(&source, "external edits").unwrap();
            let another = directory.join("another.sf");
            std::fs::write(&another, "scene another\n  end\n").unwrap();
            assert!(super::save_project(state).is_err());
            assert!(super::open_project(&another, state).is_err());
            assert_eq!(state.read().story.source(), "local edits");
            assert!(state.read().has_unsaved_changes);
            assert!(!another.with_extension("db").exists());
            super::reload_story(state).unwrap();
            assert_eq!(state.read().story.source(), "external edits");
            super::save_project(state).unwrap();

            let id = state
                .write()
                .registry
                .create_artifact(
                    storyfella_studio_core::registry::ArtifactKind::Character,
                    storyfella_studio_core::registry::CreateArtifact {
                        name: "draft protected".into(),
                        description: "Canon".into(),
                        ..Default::default()
                    },
                )
                .unwrap();
            state
                .write()
                .set_description_draft(id, "Still editing".into());
            assert!(super::open_project(&another, state).is_err());
            assert_eq!(state.read().description_draft(id), Some("Still editing"));
            state.write().discard_description_draft(id);

            // Source success followed by DB failure must retain the new disk
            // snapshot, keep the project dirty, and allow a normal retry.
            let backup = directory.join("registry-backup.db");
            std::fs::rename(&project, &backup).unwrap();
            std::fs::create_dir(&project).unwrap();
            state
                .write()
                .set_story_source("scene saved\n  end\n".into());
            assert!(super::save_project(state).is_err());
            assert_eq!(
                std::fs::read_to_string(&source).unwrap(),
                "scene saved\n  end\n"
            );
            assert!(!state.read().story.is_dirty());
            assert!(state.read().has_unsaved_changes);
            std::fs::remove_dir(&project).unwrap();
            std::fs::rename(&backup, &project).unwrap();
            super::save_project(state).unwrap();
            assert!(!state.read().has_unsaved_changes);

            state
                .write()
                .set_story_source("scene harbor\n  beat arrival\n    prop: lantern\n".into());
            super::save_project(state).unwrap();
            let saved_scene = state.read().registry.story_scenes[0].clone();
            state.write().set_story_source("scene port\n".into());
            super::reload_story(state).unwrap();
            assert_eq!(
                state.read().registry.story_scenes[0],
                saved_scene,
                "reload reattaches saved identities after an unsaved rename"
            );
            state
                .write()
                .create_pending_asset("lantern", storyfella_language::AssetKind::Prop)
                .unwrap();
            super::save_project(state).unwrap();
            std::fs::write(
                &source,
                "scene harbor\n  beat light\n  beat arrival\n    prop: lantern\n",
            )
            .unwrap();
            super::save_project(state).unwrap();
            assert_eq!(
                state.read().registry.story_scenes[0].beats[1].artifact_id,
                saved_scene.beats[0].artifact_id
            );
            let reopened = storyfella_studio_core::persistence::ProjectDb::open(&project)
                .unwrap()
                .load()
                .unwrap();
            assert_eq!(reopened.registry, state.read().registry);
            state.write().begin_operation();
            assert!(super::open_project(&another, state).is_err());
            state.write().cancel_operation();
            super::open_project(&another, state).unwrap();
            assert_eq!(state.read().story.source(), "scene another\n  end\n");
            assert!(another.with_extension("db").exists());
        });
        rsx! {}
    }

    #[test]
    fn project_commands_preserve_drafts_conflicts_and_partial_save_retries() {
        let directory = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../.svs-e2e/tests")
            .join(format!("project-commands-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&directory).unwrap();
        let mut dom = VirtualDom::new_with_props(
            PersistenceProbe,
            PersistenceProbeProps {
                directory: directory.clone(),
            },
        );
        dom.rebuild_in_place();
        drop(dom);
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn slugs_are_safe_filename_components() {
        let dir = std::path::Path::new("/tmp/projects");
        assert_eq!(
            unique_project_path(dir, "The Lighthouse!"),
            dir.join("the-lighthouse.svs-project.db")
        );
        assert_eq!(
            unique_project_path(dir, "   "),
            dir.join("story.svs-project.db")
        );
    }

    #[test]
    fn existing_files_get_a_suffix() {
        let dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../.svs-e2e/tests")
            .join(format!("svs-slug-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("story.svs-project.db"), b"x").unwrap();
        assert_eq!(
            unique_project_path(&dir, "Story"),
            dir.join("story-2.svs-project.db")
        );
        std::fs::write(dir.join("story-2.svs-project.sf"), b"source only").unwrap();
        assert_eq!(
            unique_project_path(&dir, "Story"),
            dir.join("story-3.svs-project.db")
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn stack_summary_handles_single_ops_and_stacks() {
        let create = Operation::Create {
            kind: ArtifactKind::Character,
            description: "Mia".into(),
            name: Some("mia".into()),
            size: None,
            steps: None,
            loras: Vec::new(),
        };
        let variant = Operation::Variant {
            target: Ref::new("c:mia"),
            description: "older".into(),
            name: None,
            axis: Some(VariantAxis::Age),
            size: None,
            steps: None,
            loras: Vec::new(),
        };
        assert_eq!(super::stack_summary(&[]), "no operations");
        assert!(super::stack_summary(std::slice::from_ref(&create)).starts_with("create"));
        let combined = super::stack_summary(&[create, variant]);
        assert!(combined.contains("(+1 more)"), "{combined}");
    }
}
