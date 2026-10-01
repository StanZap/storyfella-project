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

use svs_core::errors::{CodedError, ErrorCode, ReportedError};
use svs_core::persistence::ProjectDb;
use svs_core::registry::{
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
    app_state.write().snapshot_for_undo();
    ui.activity.set(OpActivity::Running { summary });

    let registry = app_state.read().registry.clone();
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
                state.registry = finished;
                state.has_unsaved_changes = true;
                drop(state);
                Some(result)
            }
            Err(_) => None,
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
    std::fs::create_dir_all(project_dir)
        .map_err(|error| ReportedError::plain(ErrorCode::StoreSave, error.to_string()))?;
    let path = unique_project_path(project_dir, story_name);
    let db = ProjectDb::open(&path).map_err(|error| error.report())?;
    app_state.write().create_project(story_name);
    let registry = app_state.read().registry.clone();
    db.rename_project(story_name)
        .map_err(|error| error.report())?;
    db.save_registry(&registry)
        .map_err(|error| error.report())?;
    app_state.write().project_path = Some(path.clone());
    app_state.write().has_unsaved_changes = false;
    Ok(path)
}

/// Opens any `.svs-project.db` file in place.
pub fn open_project(path: &Path, mut app_state: Signal<AppState>) -> Result<(), ReportedError> {
    let stored = ProjectDb::open(path)
        .and_then(|db| db.load())
        .map_err(|error| error.report())?;
    app_state.write().open_project(stored, path.to_path_buf());
    Ok(())
}

/// Saves the current registry to the open project file; clears the dirty
/// flag on success. The one persistence seam: the autosave effect, the
/// Cmd/Ctrl+S shortcut, and the driver's check-mode exit all use it.
/// No-op when no project file is open.
pub fn save_project(mut app_state: Signal<AppState>) -> Result<(), ReportedError> {
    let Some(path) = app_state.read().project_path.clone() else {
        return Ok(());
    };
    let registry = app_state.read().registry.clone();
    let story_source = app_state.read().story_source.clone();
    ProjectDb::open(&path)
        .and_then(|db| {
            db.save_registry(&registry)?;
            db.save_story_source(&story_source)
        })
        .map_err(|error| error.report())?;
    app_state.write().has_unsaved_changes = false;
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
    while candidate.exists() {
        candidate = project_dir.join(format!("{slug}-{index}.svs-project.db"));
        index += 1;
    }
    candidate
}

#[cfg(test)]
mod tests {
    use super::unique_project_path;

    use svs_core::registry::{ops::Operation, ArtifactKind, Ref, VariantAxis};

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
        let dir = std::env::temp_dir().join(format!("svs-slug-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("story.svs-project.db"), b"x").unwrap();
        assert_eq!(
            unique_project_path(&dir, "Story"),
            dir.join("story-2.svs-project.db")
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
