//! Project persistence and guarded authoring operations for Studio guides.
//! Definitions and source metadata stay in Studio; visual operations remain
//! in shared core/CLI and are rejected at this host boundary. Workers execute
//! allowed model-only operations with no image backend.

use std::path::{Path, PathBuf};

use uuid::Uuid;

use storyfella_studio_core::errors::{CodedError, ErrorCode, ReportedError};
use storyfella_studio_core::persistence::ProjectDb;
use storyfella_studio_core::project::ProjectFolder;
use storyfella_studio_core::registry::{
    ops::{self, ExecuteOptions, OpOrigin, Operation},
    pipeline::{ApprovalPolicy, RunOptions},
    slash::{self, is_slash_command},
    ArtifactRegistry, GenerationDefaults,
};

use crate::state::AppState;

/// The running state of an authoring operation.
#[derive(Clone, Debug, Default, PartialEq)]
pub enum OpActivity {
    #[default]
    Idle,
    Running {
        summary: String,
    },
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
            "Authoring commands start with / — e.g. /describe c:mara A harbourmaster",
        ));
    }
    let operation = slash::parse_slash(input)
        .map_err(|error| ReportedError::plain(ErrorCode::AppCommandInvalid, error.to_string()))?;
    validate_studio_operations(std::slice::from_ref(&operation))?;
    // Resolve references and kinds before starting the worker.
    ops::compile(registry, &operation, defaults).map_err(|error| error.report())?;
    Ok(operation)
}

/// Authoring operations that have been checked and marked as running on the
/// state they will merge back into. [`run`](Self::run) blocks, so a host calls
/// it off its UI thread and hands the result to [`finish_operations`].
pub struct PendingOperations {
    registry: ArtifactRegistry,
    baseline: ArtifactRegistry,
    operations: Vec<Operation>,
    defaults: GenerationDefaults,
    summary: String,
}

/// The worker's registry and outcome, waiting to merge.
pub struct FinishedOperations {
    baseline: ArtifactRegistry,
    registry: ArtifactRegistry,
    result: Result<ops::OpOutcome, ReportedError>,
}

/// Validates operations and marks `state` busy. The one way into an
/// authoring run: slash commands and plan steps both start here.
pub fn begin_operations(
    state: &mut AppState,
    operations: Vec<Operation>,
    defaults: GenerationDefaults,
) -> Result<PendingOperations, ReportedError> {
    validate_studio_operations(&operations)?;
    if state.operation_in_progress() {
        return Err(ReportedError::plain(
            ErrorCode::AppCommandInvalid,
            "Wait for the current operation to finish.",
        ));
    }
    state.begin_operation();
    Ok(PendingOperations {
        summary: stack_summary(&operations),
        registry: state.registry.clone(),
        baseline: state.registry.clone(),
        operations,
        defaults,
    })
}

/// Parses a slash command and begins it.
pub fn begin_slash_command(
    state: &mut AppState,
    defaults: GenerationDefaults,
    input: &str,
) -> Result<PendingOperations, ReportedError> {
    let operation = parse_command(input, &state.registry, &defaults)?;
    begin_operations(state, vec![operation], defaults)
}

impl PendingOperations {
    /// The running-state label for these operations.
    pub fn summary(&self) -> &str {
        &self.summary
    }

    /// Executes the operations on a private runtime. Blocks.
    pub fn run(self) -> FinishedOperations {
        let runtime = tokio::runtime::Runtime::new().expect("op worker runtime");
        let (registry, result) =
            run_stack(&runtime, self.registry, &self.operations, &self.defaults);
        FinishedOperations {
            baseline: self.baseline,
            registry,
            result,
        }
    }
}

/// Merges a finished run through AppState, so concurrent source edits and
/// undo stay coordinated. `None` is a worker that died without reporting:
/// the state is left as it was before the run.
pub fn finish_operations(
    state: &mut AppState,
    finished: Option<FinishedOperations>,
) -> Result<(), ReportedError> {
    let Some(finished) = finished else {
        state.cancel_operation();
        // The single log point for op results.
        return Err(ReportedError::plain(
            ErrorCode::AppStackEmpty,
            "the operation worker exited without a result",
        )
        .log());
    };
    state.finish_operation(&finished.baseline, finished.registry);
    let outcome = finished.result.map_err(ReportedError::log)?;
    if let Some(id) = outcome.artifact_id {
        state.selected_artifact_id = Some(id);
    }
    Ok(())
}

/// Execute authoring operations only, with no inference backend. Validate the
/// entire batch first so a forbidden visual step cannot leave partial edits.
fn run_stack(
    runtime: &tokio::runtime::Runtime,
    mut registry: ArtifactRegistry,
    operations: &[Operation],
    defaults: &GenerationDefaults,
) -> (ArtifactRegistry, Result<ops::OpOutcome, ReportedError>) {
    if let Err(error) = validate_studio_operations(operations) {
        return (registry, Err(error));
    }
    let mut result: Result<ops::OpOutcome, ReportedError> = Err(ReportedError::plain(
        ErrorCode::AppStackEmpty,
        "stack was empty",
    ));
    for operation in operations {
        let options = ExecuteOptions {
            backend: None,
            run: RunOptions {
                work_dir: storyfella_studio_core::paths::root()
                    .join(".svs-e2e/authoring")
                    .join(Uuid::new_v4().to_string()),
                approvals: ApprovalPolicy::auto(),
                seed: None,
            },
            generation: Default::default(),
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

/// Studio owns authoring metadata. CLI/core retain their independent visual
/// operation vocabulary; a Studio guide cannot bypass this host boundary.
pub(crate) fn validate_studio_operations(operations: &[Operation]) -> Result<(), ReportedError> {
    if operations.iter().all(|operation| {
        matches!(
            operation,
            Operation::Describe { .. }
                | Operation::Create {
                    kind: storyfella_studio_core::registry::ArtifactKind::Story
                        | storyfella_studio_core::registry::ArtifactKind::Scene
                        | storyfella_studio_core::registry::ArtifactKind::Beat,
                    ..
                }
        )
    }) {
        return Ok(());
    }
    Err(ReportedError::plain(ErrorCode::AppCommandInvalid, "Visual production belongs to Atelier. Use Open in Atelier for image creation, editing and generation settings."))
}

/// Creates a portable folder for a new story (unique slug path) and saves
/// the fresh registry into it before the screen switches to Assets.
/// Returns the created path.
pub fn create_project(
    project_dir: &Path,
    story_name: &str,
    state: &mut AppState,
) -> Result<PathBuf, ReportedError> {
    ensure_project_idle(state)?;
    // An unbound Writer draft becomes this project's first source; switching
    // away from a saved project first persists its outstanding changes.
    let (registry, source) = if state.project_path.is_none() {
        (state.registry.clone(), state.story.source().to_owned())
    } else {
        if state.has_unsaved_changes {
            save_project(state)?;
        }
        (ArtifactRegistry::default(), String::new())
    };
    std::fs::create_dir_all(project_dir)
        .map_err(|error| ReportedError::plain(ErrorCode::StoreSave, error.to_string()))?;
    let folder = ProjectFolder::create(
        &unique_project_path(project_dir, story_name),
        story_name,
        &registry,
        &source,
    )
    .map_err(|error| ReportedError::plain(ErrorCode::StoreSave, format!("{error:#}")))?;
    install_shared_project(&folder, state)?;
    Ok(folder.database())
}

/// Opens a project database or its `.sf` source in place. A standalone `.sf`
/// gets a companion database; the source file is never imported into SQLite.
pub fn open_project(path: &Path, state: &mut AppState) -> Result<(), ReportedError> {
    ensure_project_idle(state)?;
    if state.has_unsaved_changes {
        save_project(state)?;
    }
    if let Some(folder) = ProjectFolder::locate_for_authoring(path).map_err(shared_open_error)? {
        return install_shared_project(&folder, state);
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
    state.open_project(stored, project_path, None);
    Ok(())
}

fn shared_open_error(error: anyhow::Error) -> ReportedError {
    ReportedError::plain(ErrorCode::StoreOpen, format!("{error:#}"))
}

fn install_shared_project(
    folder: &ProjectFolder,
    state: &mut AppState,
) -> Result<(), ReportedError> {
    let existing = state.authoring_lease.clone();
    let lease = match existing.filter(|lease| lease.belongs_to(folder)) {
        Some(lease) => lease,
        None => folder.claim_authoring().map_err(shared_open_error)?,
    };
    lease.verify().map_err(shared_open_error)?;
    let stored = folder.load().map_err(shared_open_error)?;
    state.open_project(stored, folder.database(), Some(lease));
    Ok(())
}

/// Bind an unbound draft to the exact destination chosen in the native dialog.
pub fn save_new_project(path: &Path, state: &mut AppState) -> Result<(), ReportedError> {
    ensure_project_idle(state)?;
    if state.project_path.is_some() || path.exists() || path.with_extension("sf").exists() {
        return Err(ReportedError::plain(
            ErrorCode::StoreSave,
            "Choose a new project file; existing projects and story sources are never overwritten.",
        ));
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| ReportedError::plain(ErrorCode::StoreSave, error.to_string()))?;
    }
    let registry = state.registry.clone();
    let source = state.story.source().to_owned();
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
    state.open_project(stored, path.to_owned(), None);
    Ok(())
}

fn ensure_project_idle(state: &AppState) -> Result<(), ReportedError> {
    if state.has_description_drafts() {
        return Err(ReportedError::plain(
            ErrorCode::AppCommandInvalid,
            "Apply or discard the description drafts in Assets before switching projects.",
        ));
    }
    if state.operation_in_progress() {
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
pub fn save_project(state: &mut AppState) -> Result<(), ReportedError> {
    let Some(path) = state.project_path.clone() else {
        if state.has_unsaved_changes {
            return Err(storyfella_studio_core::story::StoryFileError::Unbound.report());
        }
        return Ok(());
    };
    if state.operation_in_progress() {
        return Err(ReportedError::plain(
            ErrorCode::StoreSave,
            "Wait for the current operation before saving.",
        ));
    }
    if let Some(lease) = &state.authoring_lease {
        lease
            .verify()
            .map_err(|error| ReportedError::plain(ErrorCode::StoreSave, format!("{error:#}")))?;
    }
    let db = ProjectDb::open_existing(&path).map_err(|error| error.report())?;
    let stored = db.load().map_err(|error| error.report())?;
    let mut refreshed = state.clone();
    let result = refreshed.refresh_catalog(
        stored,
        storyfella_studio_core::catalog::sync::Resolution::Check,
    );
    if *state != refreshed {
        *state = refreshed;
    }
    result.map_err(|message| {
        ReportedError::plain(
            ErrorCode::StoreSave,
            format!("{message}. Review the catalog conflict before saving."),
        )
    })?;
    if let Some(sources) = state.additional_sources.as_mut() {
        sources.save().map_err(|error| error.report())?;
    }
    let mut story = state.story.clone();
    let registry = state.registry.clone();
    let version = state.registry_version;
    let version = db
        .save_authoring_checked(&registry, version, &mut story)
        .map_err(|error| error.report())?;
    if state.story != story {
        state.adopt_story_document(story);
    }
    state.mark_saved(version);
    Ok(())
}

/// Called only after the Writer's explicit discard confirmation when dirty.
pub fn reload_story(state: &mut AppState) -> Result<(), ReportedError> {
    let mut story = state.story.clone();
    story.reload().map_err(|error| error.report())?;
    state.adopt_story_document(story);
    Ok(())
}

/// `"My Story"` → `my-story/`; an existing folder gets a `-2`,
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
    let mut candidate = project_dir.join(&slug);
    let mut index = 2;
    while candidate.exists() {
        candidate = project_dir.join(format!("{slug}-{index}"));
        index += 1;
    }
    candidate
}

#[cfg(test)]
mod tests {
    #[test]
    fn studio_blocks_visual_operations_before_any_authoring_batch_is_applied() {
        use storyfella_studio_core::registry::{
            ops, slash, ArtifactKind, ArtifactRegistry, GenerationDefaults,
        };
        let defaults = GenerationDefaults {
            sizes: Default::default(),
            steps: 25,
            cfg_scale: 6.0,
            sample_method: "euler".into(),
            loras: Default::default(),
        };
        let visual = slash::parse_slash("/create character --name mara A harbourmaster").unwrap();
        // The independent CLI/core capability still exists.
        assert!(matches!(
            ops::compile(&ArtifactRegistry::default(), &visual, &defaults).unwrap(),
            ops::CompiledOp::Pipeline(_)
        ));
        assert!(super::parse_command(
            "/create character --name mara A harbourmaster",
            &ArtifactRegistry::default(),
            &defaults
        )
        .unwrap_err()
        .details
        .contains("Atelier"));
        let metadata = ops::Operation::Create {
            kind: ArtifactKind::Story,
            name: Some("premise".into()),
            description: "A story".into(),
            size: None,
            steps: None,
            loras: vec![],
        };
        assert!(super::validate_studio_operations(std::slice::from_ref(&metadata)).is_ok());
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let registry = ArtifactRegistry::default();
        let (after, result) =
            super::run_stack(&runtime, registry.clone(), &[metadata, visual], &defaults);
        assert!(result.is_err());
        assert_eq!(after, registry);
        for command in [
            "/regenerate c:mara",
            "/modify c:mara a blue coat",
            "/resize c:mara 512x768",
            "/set c:mara --steps 8",
            "/attach-lora c:mara example",
            "/revert c:mara",
        ] {
            let operation = slash::parse_slash(command).unwrap();
            assert!(
                super::validate_studio_operations(&[operation]).is_err(),
                "{command}"
            );
        }
    }
    use super::unique_project_path;
    use std::path::{Path, PathBuf};

    use crate::state::AppState;

    use storyfella_studio_core::registry::{ops::Operation, ArtifactKind, Ref, VariantAxis};

    fn persistence_probe(directory: &Path) {
        let mut state = AppState::default();
        let mut unbound = AppState::default();
        let mut empty = AppState::default();
        {
            super::create_project(directory, "Blank", &mut empty).unwrap();
            assert_eq!(empty.story.source(), "");
            assert!(empty.registry.artifacts.is_empty());
            assert_eq!(
                std::fs::read_to_string(empty.story.path().unwrap()).unwrap(),
                ""
            );
            unbound.set_story_source("scene exact\n  end\n".into());
            let exact = directory.join("Exact chosen filename.db");
            super::save_new_project(&exact, &mut unbound).unwrap();
            assert_eq!(unbound.project_path.as_deref(), Some(exact.as_path()));
            assert_eq!(
                std::fs::read_to_string(exact.with_extension("sf")).unwrap(),
                "scene exact\n  end\n"
            );
            assert!(!unbound.has_unsaved_changes);
            assert!(super::save_new_project(&exact, &mut state).is_err());
            let collision = directory.join("Existing source.db");
            let existing_source = collision.with_extension("sf");
            std::fs::write(&existing_source, "scene existing\n  end\n").unwrap();
            assert!(super::save_new_project(&collision, &mut state).is_err());
            assert!(!collision.exists());
            assert_eq!(
                std::fs::read_to_string(existing_source).unwrap(),
                "scene existing\n  end\n"
            );
            // Starting a project preserves an unbound Writer draft.
            state.set_story_source("scene draft\r\n   unfinished 🌊\r\n".into());
            let project = super::create_project(directory, "Draft", &mut state).unwrap();
            let source = state.story.path().unwrap().to_owned();
            assert_eq!(
                std::fs::read_to_string(&source).unwrap(),
                state.story.source()
            );
            assert!(!state.has_unsaved_changes);

            // Every way of opening the shared project must meet the same
            // owner check, even before the first Studio saves its draft.
            let folder = project.parent().unwrap();
            let manifest = folder.join(storyfella_studio_core::project::MANIFEST);
            state.set_story_source("scene still_editing\n  end\n".into());
            for alias in [
                folder,
                manifest.as_path(),
                project.as_path(),
                source.as_path(),
            ] {
                let error = super::open_project(alias, &mut unbound).unwrap_err();
                assert!(error.details.contains("open for editing in another Studio"));
            }
            #[cfg(unix)]
            {
                let alias = directory.join("shared-alias.sf");
                std::fs::write(&alias, "scene alias\n  end\n").unwrap();
                std::os::unix::fs::symlink(&project, alias.with_extension("db")).unwrap();
                let error = super::open_project(&alias, &mut unbound).unwrap_err();
                assert!(error.details.contains("open for editing in another Studio"));
            }
            assert_eq!(unbound.project_path.as_deref(), Some(exact.as_path()));
            assert!(state.story.is_dirty());
            assert_ne!(
                std::fs::read_to_string(&source).unwrap(),
                state.story.source()
            );
            super::open_project(&manifest, &mut state).unwrap();
            assert_eq!(state.story.source(), "scene still_editing\n  end\n");

            // An external edit prevents both saving and switching projects.
            state.set_story_source("local edits".into());
            std::fs::write(&source, "external edits").unwrap();
            let another = directory.join("another.sf");
            std::fs::write(&another, "scene another\n  end\n").unwrap();
            assert!(super::save_project(&mut state).is_err());
            assert!(super::open_project(&another, &mut state).is_err());
            assert_eq!(state.story.source(), "local edits");
            assert!(state.has_unsaved_changes);
            assert!(!another.with_extension("db").exists());
            super::reload_story(&mut state).unwrap();
            assert_eq!(state.story.source(), "external edits");
            super::save_project(&mut state).unwrap();

            let id = state
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
            state.set_description_draft(id, "Still editing".into());
            assert!(super::open_project(&another, &mut state).is_err());
            assert_eq!(state.description_draft(id), Some("Still editing"));
            state.discard_description_draft(id);

            // A DB failure must not publish source ahead of its catalog.
            let backup = directory.join("registry-backup.db");
            std::fs::rename(&project, &backup).unwrap();
            std::fs::create_dir(&project).unwrap();
            state.set_story_source("scene saved\n  end\n".into());
            assert!(super::save_project(&mut state).is_err());
            assert_eq!(std::fs::read_to_string(&source).unwrap(), "external edits");
            assert!(state.story.is_dirty());
            assert!(state.has_unsaved_changes);
            std::fs::remove_dir(&project).unwrap();
            std::fs::rename(&backup, &project).unwrap();
            super::save_project(&mut state).unwrap();
            assert!(!state.has_unsaved_changes);

            state.set_story_source("scene harbor\n  beat arrival\n    prop: lantern\n".into());
            super::save_project(&mut state).unwrap();
            let saved_scene = state.registry.story_scenes[0].clone();
            state.set_story_source("scene port\n".into());
            super::reload_story(&mut state).unwrap();
            assert_eq!(
                state.registry.story_scenes[0], saved_scene,
                "reload reattaches saved identities after an unsaved rename"
            );
            assert!(state
                .registry
                .artifacts
                .iter()
                .any(|asset| { asset.name == "lantern" && asset.kind == ArtifactKind::Object }));
            super::save_project(&mut state).unwrap();
            std::fs::write(
                &source,
                "scene harbor\n  beat light\n  beat arrival\n    prop: lantern\n",
            )
            .unwrap();
            super::save_project(&mut state).unwrap();
            assert_eq!(
                state.registry.story_scenes[0].beats[1].artifact_id,
                saved_scene.beats[0].artifact_id
            );
            let reopened = storyfella_studio_core::persistence::ProjectDb::open(&project)
                .unwrap()
                .load()
                .unwrap();
            assert_eq!(reopened.registry, state.registry);
            state.begin_operation();
            assert!(super::open_project(&another, &mut state).is_err());
            state.cancel_operation();
            super::open_project(&another, &mut state).unwrap();
            assert_eq!(state.story.source(), "scene another\n  end\n");
            assert!(another.with_extension("db").exists());
            super::open_project(&source, &mut unbound).unwrap();
            assert_eq!(unbound.project_path.as_deref(), Some(project.as_path()));
            assert_eq!(unbound.registry, reopened.registry);
        }
    }

    #[test]
    fn project_commands_preserve_drafts_conflicts_and_partial_save_retries() {
        let directory = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../.svs-e2e/tests")
            .join(format!("project-commands-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&directory).unwrap();
        persistence_probe(&directory);
        std::fs::remove_dir_all(directory).unwrap();
    }

    fn multi_file_probe(directory: &Path) {
        let mut state = AppState::default();
        {
            super::create_project(directory, "Multi", &mut state).unwrap();
            let root = state.story.path().unwrap().parent().unwrap().to_owned();
            let chapter = root.join("chapter.sf");
            state.create_additional_source(&chapter).unwrap();
            state
                .set_additional_source("chapter.sf", "scene first\n  end\n".into())
                .unwrap();
            super::save_project(&mut state).unwrap();
            assert_eq!(
                std::fs::read_to_string(&chapter).unwrap(),
                "scene first\n  end\n"
            );
            assert!(!state.has_unsaved_changes);
            state
                .set_additional_source("chapter.sf", "scene local\n  end\n".into())
                .unwrap();
            std::fs::write(&chapter, "scene external\n  end\n").unwrap();
            assert!(super::save_project(&mut state).is_err());
            assert_eq!(
                std::fs::read_to_string(&chapter).unwrap(),
                "scene external\n  end\n"
            );
            assert_eq!(
                state
                    .additional_sources
                    .as_ref()
                    .unwrap()
                    .get("chapter.sf")
                    .unwrap()
                    .source(),
                "scene local\n  end\n"
            );
            assert!(state.has_unsaved_changes);
        }
    }

    #[test]
    fn multi_file_save_preserves_conflicted_chapter_drafts() {
        let directory = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../.svs-e2e/tests")
            .join(format!("multi-file-commands-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&directory).unwrap();
        multi_file_probe(&directory);
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn slugs_are_safe_filename_components() {
        let dir = std::path::Path::new("projects");
        assert_eq!(
            unique_project_path(dir, "The Lighthouse!"),
            dir.join("the-lighthouse")
        );
        assert_eq!(unique_project_path(dir, "   "), dir.join("story"));
    }

    #[test]
    fn existing_files_get_a_suffix() {
        let dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../.svs-e2e/tests")
            .join(format!("svs-slug-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("story"), b"x").unwrap();
        assert_eq!(unique_project_path(&dir, "Story"), dir.join("story-2"));
        std::fs::create_dir(dir.join("story-2")).unwrap();
        assert_eq!(unique_project_path(&dir, "Story"), dir.join("story-3"));
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
