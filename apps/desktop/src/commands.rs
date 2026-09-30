//! The intent seam — the app-owned action language.
//!
//! Everything a user can *do* to a project (create/open it, run slash
//! operations) lives here, implemented once against the registry, state,
//! and persistence layers. The canvas composer, the operation log rerun
//! action, and the automation driver (`crate::driver`) all call the same
//! entry points, so an action driven by the driver is indistinguishable
//! from the same action typed by a user.
//!
//! Execution model: operations run on a worker thread (own tokio runtime)
//! so mask checkpoints can block on the user without freezing the UI; the
//! worker forwards checkpoint requests over a channel to the supervisor
//! (UI thread), which surfaces them through [`ExecutionUi`] and sends the
//! user's decision back. The registry is written back on success **and**
//! on failure (a failed revision is recorded; intermediates are kept); the
//! stack stops at the first failure or checkpoint rejection (fail-fast,
//! ops already applied stay).

use std::{
    cell::RefCell,
    path::{Path, PathBuf},
    rc::Rc,
    sync::{mpsc, Arc, Mutex},
    time::Duration,
};

use dioxus::prelude::*;
use uuid::Uuid;

use svs_core::errors::{CodedError, ErrorCode, ReportedError};
use svs_core::persistence::ProjectDb;
use svs_core::registry::{
    backend::CreativeBackend,
    ops::{self, ExecuteOptions, OpOrigin, Operation},
    pipeline::{
        auto_decision, ApprovalPolicy, BlockedOn, Decision, GenerationOverrides, InputPurpose,
        RunOptions,
    },
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

/// One segmentation candidate shown in the mask-confirmation modal.
#[derive(Clone, Debug, PartialEq)]
pub struct MaskCandidateUi {
    pub path: String,
    pub score: f64,
    pub area_pixels: u64,
}

impl From<svs_core::registry::pipeline::MaskCandidate> for MaskCandidateUi {
    fn from(candidate: svs_core::registry::pipeline::MaskCandidate) -> Self {
        Self {
            path: candidate.path.display().to_string(),
            score: candidate.score,
            area_pixels: candidate.area_pixels,
        }
    }
}

/// The mask checkpoint waiting on the user: which image to judge the
/// candidates against, plus the candidates themselves.
#[derive(Clone, Debug, PartialEq)]
pub struct PendingMaskApproval {
    pub image: Option<String>,
    pub description: String,
    pub candidates: Vec<MaskCandidateUi>,
}

/// The checkpoint waiting on the user, and which kind of answer it needs.
#[derive(Clone, Debug, PartialEq)]
pub enum PendingApproval {
    /// Segment candidates to choose from (the mask checkpoint).
    Mask(PendingMaskApproval),
    /// The LLM could not plan the edit; the user supplies the split.
    PlannedEdit { description: String },
}

/// Messages the op worker thread sends to the supervisor (UI thread).
enum OpEvent {
    PendingMask(PendingMaskApproval),
    /// The LLM edit plan degraded; the worker only sends this for
    /// `InputPurpose::PlannedEdit` (the canvas cannot answer other kinds).
    PendingInput {
        description: String,
    },
    Done {
        registry: ArtifactRegistry,
        result: Result<ops::OpOutcome, ReportedError>,
    },
}

/// The UI-facing surface an operation run reports through: activity,
/// checkpoints, and errors. The canvas owns one instance; the driver owns
/// another (its supervisor auto-rejects checkpoints, since a procedural
/// run cannot answer them).
#[derive(Clone)]
pub struct ExecutionUi {
    pub activity: Signal<OpActivity>,
    pub pending_approval: Signal<Option<PendingApproval>>,
    pub reply_slot: Rc<RefCell<Option<mpsc::Sender<Decision>>>>,
    pub error_report: Signal<Option<ReportedError>>,
}

impl ExecutionUi {
    /// A self-contained surface for non-interactive drivers: checkpoints
    /// are automatically rejected (the op is logged rejected and the run
    /// fails) — a plan cannot answer a question only a user can answer.
    pub fn for_driver() -> Self {
        Self {
            activity: use_signal(OpActivity::default),
            pending_approval: use_signal(|| None::<PendingApproval>),
            reply_slot: Rc::new(RefCell::new(None::<mpsc::Sender<Decision>>)),
            error_report: use_signal(|| None::<ReportedError>),
        }
    }

    /// Sends the pending checkpoint a rejection and clears it. Used by the
    /// driver's supervisor loop to keep a non-interactive run moving.
    pub fn reject_pending(&mut self) {
        if self.pending_approval.read().is_some() {
            if let Some(sender) = self.reply_slot.borrow_mut().take() {
                let _ = sender.send(Decision::Reject);
            }
            self.pending_approval.set(None);
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
/// The flag that stands in for the canvas brush's output until a command
/// leaves the UI.
pub const PAINTED_MASK_FLAG: &str = "--mask painted";

pub fn run_slash_command(
    app_state: Signal<AppState>,
    backend: CreativeBackend,
    defaults: GenerationDefaults,
    ui: ExecutionUi,
    input: &str,
) -> Result<(), ReportedError> {
    // `--mask painted` names the canvas brush's output. Resolved here, in
    // the one place every surface funnels through, so the composer and the
    // automation driver cannot disagree about what it means — and so the
    // slash parser only ever sees a real path.
    let input = match (
        input.contains(PAINTED_MASK_FLAG),
        app_state.read().painted_mask.clone(),
    ) {
        (true, Some(path)) => input.replace(PAINTED_MASK_FLAG, &format!("--mask-file {path}")),
        (true, None) => {
            return Err(ReportedError::plain(
                ErrorCode::OpIncompleteModify,
                "Nothing is painted on the canvas. Pick the brush (b), paint the region to edit, then run this again.",
            ))
        }
        (false, _) => input.to_owned(),
    };
    let operation = parse_command(&input, &app_state.read().registry, &defaults)?;
    run_operations(app_state, backend, vec![operation], defaults, ui);
    Ok(())
}

/// Applies one or more operations: snapshot for undo, then execute each in
/// order against the backend on a worker thread (its own tokio runtime) so
/// checkpoints can block on the user without freezing the UI.
///
/// Handshake: the worker sends [`OpEvent`]s over a std channel; the supervisor
/// task on the UI thread forwards checkpoints to `ui.pending_approval` and
/// applies the finished registry. The user's decision travels back on a
/// second channel whose UI end lives in `ui.reply_slot` (UI thread only —
/// signals are not cross-thread safe, so the worker never touches them).
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

    let mut registry = app_state.read().registry.clone();
    let (event_tx, event_rx) = mpsc::channel::<OpEvent>();
    let (reply_tx, reply_rx) = mpsc::channel::<Decision>();
    *ui.reply_slot.borrow_mut() = Some(reply_tx);
    // Shared across the per-op policies (the receiver is not Clone); the
    // worker is single-threaded, so the lock never contends.
    let reply_rx = Arc::new(Mutex::new(reply_rx));

    spawn(async move {
        let mut ui = ui;
        std::thread::spawn(move || {
            let runtime = tokio::runtime::Runtime::new().expect("op worker runtime");
            let mut result: Result<ops::OpOutcome, ReportedError> = Err(ReportedError::plain(
                ErrorCode::AppStackEmpty,
                "stack was empty",
            ));
            for operation in &operations {
                // The image a mask checkpoint on this op is judged against,
                // resolved from the worker's registry — which earlier ops in
                // the stack may have changed.
                let approval_image = operation
                    .target_ref()
                    .and_then(|reference| registry.resolve(reference).ok())
                    .and_then(|id| registry.latest_image(id))
                    .and_then(|revision| revision.asset_path.clone());
                let event_tx_for_policy = event_tx.clone();
                let reply_rx_for_policy = reply_rx.clone();
                let approvals =
                    ApprovalPolicy::Interactive(Box::new(move |blocked| match blocked {
                        BlockedOn::SelectMask {
                            description,
                            candidates,
                        } => {
                            let event = OpEvent::PendingMask(PendingMaskApproval {
                                image: approval_image.clone(),
                                description,
                                candidates: candidates
                                    .into_iter()
                                    .map(MaskCandidateUi::from)
                                    .collect(),
                            });
                            if event_tx_for_policy.send(event).is_err() {
                                return Decision::Reject;
                            }
                            // Block until the modal answers (or the app closes).
                            reply_rx_for_policy
                                .lock()
                                .expect("reply lock should not contend")
                                .recv()
                                .unwrap_or(Decision::Reject)
                        }
                        BlockedOn::ProvideInput {
                            description,
                            purpose,
                        } => {
                            // Only the edit-plan split is surfaceable here;
                            // other degraded steps keep the auto behavior.
                            if purpose != InputPurpose::PlannedEdit {
                                return Decision::Reject;
                            }
                            let event = OpEvent::PendingInput { description };
                            if event_tx_for_policy.send(event).is_err() {
                                return Decision::Reject;
                            }
                            reply_rx_for_policy
                                .lock()
                                .expect("reply lock should not contend")
                                .recv()
                                .unwrap_or(Decision::Reject)
                        }
                        other => auto_decision(other),
                    }));
                let options = ExecuteOptions {
                    backend: Some(&backend),
                    run: RunOptions {
                        work_dir: std::env::temp_dir()
                            .join(format!("svs-canvas-{}", Uuid::new_v4())),
                        approvals,
                        // A fresh seed per visual op so regenerate is never an
                        // exact replay; pass --seed on the CLI for golden runs.
                        seed: Some(random_seed()),
                    },
                    generation: GenerationOverrides {
                        // The configured default step count (Settings →
                        // Image generation) shapes every canvas op.
                        steps: Some(defaults.steps),
                        ..Default::default()
                    },
                    defaults: &defaults,
                    manual_text: None,
                    origin: OpOrigin::User,
                };
                match runtime.block_on(ops::execute(&mut registry, operation, &options)) {
                    Ok(outcome) => {
                        if outcome.status == svs_core::registry::ops::OpStatus::Rejected {
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
                        // work_dir let the failure be reproduced/inspected);
                        // the supervisor logs the report once.
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
            let _ = event_tx.send(OpEvent::Done { registry, result });
        });

        // Supervisor: forward checkpoints to the modal, then apply the
        // finished registry. The poll loop keeps the dioxus executor free
        // (std channels have no async recv without tokio's sync feature).
        let mut result: Result<ops::OpOutcome, ReportedError> = Err(ReportedError::plain(
            ErrorCode::AppStackEmpty,
            "op worker exited without a result",
        ));
        // The worker moved the pre-op registry; `None` until it reports back
        // (a worker that dies without reporting leaves the current state).
        let mut finished_registry: Option<ArtifactRegistry> = None;
        loop {
            match event_rx.recv_timeout(Duration::from_millis(50)) {
                Ok(OpEvent::PendingMask(approval)) => {
                    ui.pending_approval
                        .set(Some(PendingApproval::Mask(approval)));
                }
                Ok(OpEvent::PendingInput { description }) => {
                    ui.pending_approval
                        .set(Some(PendingApproval::PlannedEdit { description }));
                }
                Ok(OpEvent::Done {
                    registry: finished,
                    result: outcome,
                }) => {
                    finished_registry = Some(finished);
                    result = outcome;
                    break;
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    tokio::time::sleep(Duration::from_millis(50)).await;
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            }
        }
        ui.pending_approval.set(None);

        let mut state = app_state.write();
        state.registry = finished_registry.unwrap_or_else(|| state.registry.clone());
        state.has_unsaved_changes = true;
        match result {
            Ok(outcome) => {
                if let Some(id) = outcome.artifact_id {
                    state.selected_artifact_id = Some(id);
                }
                drop(state);
                ui.activity.set(OpActivity::Idle);
            }
            Err(report) => {
                drop(state);
                ui.activity.set(OpActivity::Idle);
                // The single log point for op results: the worker attached
                // the request context, this is where the line is emitted.
                ui.error_report.set(Some(report.log()));
            }
        }
    });
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
