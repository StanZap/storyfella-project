//! The canvas — the artifact workspace (roadmap §12 item 4).
//!
//! Two states, not a split. At rest the screen is the artifact grid
//! (`super::gallery`); opening one replaces it with the detail pane (image,
//! description, variants, revisions, children, operation log). The tokenized
//! command bar (`super::cmdbar`) sits under both. Context actions use visual
//! forms, compiled to the same typed operations as the bar and `svs` CLI.
//!
//! `modify` is an instruction edit — a reference image plus a grounded
//! instruction — so it runs unattended and this screen has no checkpoint to
//! answer.

use dioxus::prelude::*;
use uuid::Uuid;

use storyfella_studio_core::errors::{ErrorCode, ReportedError};
use storyfella_studio_core::registry::{
    backend::CreativeBackend,
    completion::RefCandidate,
    ops::{self, Operation},
    slash::is_slash_command,
    Artifact, ArtifactRegistry, ArtifactRevision, RevisionStatus,
};

use crate::{
    commands::{self, ExecutionUi, OpActivity},
    state::AppState,
};

use super::{
    actions::{ActionRequest, ArtAction, StudioAction},
    art_actions::ArtActionDialog,
    cmdbar::{self, CommandBar},
    components::{EmptyVisual, IconButton},
    gallery::Gallery,
    generated_asset_url,
    icons::{Icon, IconName},
    inspector::Inspector,
    shortcuts,
};

/// Collapse state for the detail panels, in context so the key handler and
/// the pane share one source of truth.
#[derive(Clone, Copy)]
pub struct HistoryOpen(pub Signal<bool>);
#[derive(Clone, Copy)]
pub struct VariantsOpen(pub Signal<bool>);
#[derive(Clone, Copy)]
pub struct InspectorOpen(pub Signal<bool>);

#[component]
pub fn Canvas(
    app_state: Signal<AppState>,
    backend: Signal<CreativeBackend>,
    defaults: Signal<storyfella_studio_core::registry::GenerationDefaults>,
    asset_directory: std::path::PathBuf,
) -> Element {
    // Owned by the Workspace: the composer's text is the same signal the
    // automation driver types into (a `compose` step writes it visibly).
    let composer = use_context::<Signal<String>>();
    let mut requests = use_signal(Vec::<(String, Option<Uuid>)>::new);
    let project_path = app_state.read().project_path.clone();
    use_effect(use_reactive(&project_path, move |_| {
        requests.set(Vec::new())
    }));
    let filter = use_signal(String::new);
    let activity = use_signal(OpActivity::default);
    let mut error_report = use_context::<Signal<Option<ReportedError>>>();
    let mut command_error = use_signal(|| None::<ReportedError>);
    let mut action_request = use_context::<ActionRequest>().0;
    let mut pending_action = use_signal(|| None::<(ArtAction, Option<std::path::PathBuf>)>);
    let mut import_trigger = use_signal(|| 0_u64);
    let selected = app_state.read().selected_artifact().cloned();
    let busy = activity.read().is_busy();
    // The chip the composer shows: free-form changes apply to the viewed
    // artifact (computed before the rsx — `selected` is moved into Detail).
    let target_hint = selected
        .as_ref()
        .filter(|artifact| artifact.kind.is_visual())
        .map(|artifact| app_state.read().registry.ref_of(artifact.id));

    // Every artifact is offerable as a `c:` target; the hint is its kind, so
    // `c:` completion reads as a typed list rather than bare names.
    let ref_candidates: Vec<RefCandidate> = app_state
        .read()
        .registry
        .artifacts
        .iter()
        .map(|artifact| RefCandidate::new(artifact.name.clone(), artifact.kind.as_str()))
        .collect();

    // A free-form message proposed as an operation stack, awaiting approval.
    let mut proposal = use_signal(|| None::<StackProposal>);
    let reviewing = proposal.read().is_some();
    let disabled = busy || reviewing || project_path.is_none();
    use_effect(move || {
        let Some(action) = action_request() else {
            return;
        };
        action_request.set(None);
        match action {
            StudioAction::FocusCommand => cmdbar::focus_field(),
            StudioAction::ImportArtwork if !disabled => {
                let next = import_trigger.peek().wrapping_add(1);
                import_trigger.set(next);
            }
            StudioAction::Art(action) if !disabled => {
                pending_action.set(Some((action, app_state.peek().project_path.clone())));
            }
            _ => {}
        }
    });

    // The inspector's controls are commands: they take the same path a
    // typed one does, so the rail and the bar cannot diverge.
    let run_command = move |input: String| {
        if disabled {
            return;
        }
        let ui = ExecutionUi {
            activity,
            error_report,
        };
        if let Err(report) =
            commands::run_slash_command(app_state, backend(), defaults.read().clone(), ui, &input)
        {
            command_error.set(Some(report.clone()));
            error_report.set(Some(report.log()));
        } else {
            command_error.set(None);
        }
    };

    let palette_open = use_context::<Signal<bool>>();
    // Collapse state for the detail pane's two panels. Provided here so the
    // keyboard can toggle them; the tooltips promise `e` and `f`, and a
    // promised key that does nothing is worse than no key.
    let show_history = use_signal(|| false);
    use_context_provider(|| HistoryOpen(show_history));
    let show_variants = use_signal(|| true);
    use_context_provider(|| VariantsOpen(show_variants));
    let show_inspector = use_signal(|| true);
    use_context_provider(|| InspectorOpen(show_inspector));
    // The grid's keyboard cursor. Kept here rather than inside Gallery so
    // the key handler and the grid share one source of truth.
    let mut grid_cursor = use_context::<super::GridCursor>().0;

    // Keyboard navigation. The listener lives in the Workspace; this
    // observes what it broadcasts, so the canvas gets keys whether or not
    // it happened to mount first.
    let viewing = selected.is_some();
    let filter_query = filter.read().clone();
    let last_key = use_context::<Signal<Option<super::keys::KeyEvent>>>();
    let screen = use_context::<Signal<super::AppScreen>>();
    use_effect(move || {
        let Some((_, press)) = last_key() else {
            return;
        };
        if *palette_open.peek() || *screen.peek() != super::AppScreen::Canvas {
            return;
        }
        let ids = super::gallery::ordered_ids(
            &app_state.peek().registry,
            &filter_query.to_ascii_lowercase(),
        );
        match press.key.as_str() {
            "Escape" if viewing => app_state.write().clear_selection(),
            "Enter" if !viewing => {
                if let Some(id) = ids.get(*grid_cursor.peek()) {
                    app_state.write().select_artifact(*id);
                }
            }
            "n" if !viewing => action_request.set(Some(StudioAction::Art(ArtAction::Create(
                storyfella_studio_core::registry::ArtifactKind::Character,
            )))),
            // In detail, `[` and `]` walk the variant filmstrip — the same
            // set the strip shows, so the keys and the row agree.
            key @ ("[" | "]") if viewing => {
                let state = app_state.peek();
                let Some(open) = state.selected_artifact() else {
                    return;
                };
                let base = open.variant_of.unwrap_or(open.id);
                let mut family = vec![base];
                family.extend(
                    state
                        .registry
                        .artifacts
                        .iter()
                        .filter(|candidate| candidate.variant_of == Some(base))
                        .map(|candidate| candidate.id),
                );
                let at = family.iter().position(|id| *id == open.id).unwrap_or(0);
                let next = if key == "]" {
                    (at + 1).min(family.len() - 1)
                } else {
                    at.saturating_sub(1)
                };
                let target = family[next];
                drop(state);
                app_state.write().select_artifact(target);
            }
            "e" if viewing => {
                let mut open = show_history;
                let now = *open.peek();
                open.set(!now);
            }
            "f" if viewing => {
                let mut open = show_variants;
                let now = *open.peek();
                open.set(!now);
            }
            key @ ("v" | "r" | "m") if viewing => {
                if let Some(artifact) = app_state
                    .peek()
                    .selected_artifact()
                    .filter(|a| a.kind.is_visual())
                {
                    let action = match key {
                        "v" => ArtAction::Variant(artifact.variant_of.unwrap_or(artifact.id)),
                        "r" => ArtAction::Regenerate(artifact.id),
                        _ => ArtAction::Modify(artifact.id),
                    };
                    action_request.set(Some(StudioAction::Art(action)));
                }
            }
            key if !viewing => {
                let at = *grid_cursor.peek();
                if let Some(next) = super::keys::move_cursor(key, at, ids.len(), press.columns) {
                    grid_cursor.set(next);
                    document::eval(
                        "const el = document.getElementById(\"svs-focused-tile\");
                         if (el) { el.scrollIntoView({ block: \"nearest\" }); }",
                    );
                }
            }
            _ => {}
        }
    });

    let submit = move |_| {
        if disabled {
            return;
        }
        let input = composer().trim().to_owned();
        if input.is_empty() {
            return;
        }
        let defaults = defaults.read().clone();
        if is_slash_command(&input) {
            let ui = ExecutionUi {
                activity,
                error_report,
            };
            if let Err(report) =
                commands::run_slash_command(app_state, backend(), defaults, ui, &input)
            {
                command_error.set(Some(report.clone()));
                error_report.set(Some(report.log()));
            } else {
                command_error.set(None);
                requests
                    .write()
                    .insert(0, (composer(), app_state.read().selected_artifact_id));
                requests.write().truncate(10);
                cmdbar::clear(composer);
            }
        } else {
            command_error.set(None);
            // Free text: the LLM proposes the operations; the user approves.
            propose_operations(
                app_state,
                backend(),
                input,
                defaults,
                activity,
                proposal,
                error_report,
            );
        }
    };

    let proposal_modal = {
        let pending = proposal.read().clone();
        match pending {
            Some(pending) => {
                let operations = pending.operations.clone();
                Some(rsx! {
                    StackProposalModal {
                        operations,
                        on_approve: move |selected: Vec<Operation>| {
                            requests.write().insert(0, (composer(), app_state.read().selected_artifact_id));
                            requests.write().truncate(10);
                            cmdbar::clear(composer);
                            proposal.set(None);
                            let ui = ExecutionUi {
                                activity,
                                error_report,
                            };
                            commands::run_operations(
                                app_state,
                                backend(),
                                selected,
                                defaults.read().clone(),
                                ui,
                            );
                        },
                        on_cancel: move |_| proposal.set(None),
                    }
                })
            }
            None => None,
        }
    };

    rsx! {
        // Two states, not a split: the grid is the canvas at rest, and
        // opening an artifact replaces it. The sidebar is gone — the picture
        // is what you navigate by.
        section { class: "flex h-full min-h-0 w-full flex-col",
            if selected.is_none() {
                Gallery {
                    app_state,
                    filter,
                    cursor: grid_cursor,
                    disabled,
                }
            } else {
                Detail {
                    app_state,
                    artifact: selected,
                    activity,
                    disabled,
                    onrun: run_command,
                    on_rerun: move |op: Operation| {
                    if disabled {
                        return;
                    }
                    let ui = ExecutionUi {
                        activity,
                        error_report,
                    };
                    commands::run_operations(
                        app_state,
                        backend(),
                        vec![op],
                        defaults.read().clone(),
                        ui,
                    );
                },
                }
            }
            Composer {
                value: composer,
                app_state,
                activity,
                disabled,
                target_hint,
                ref_candidates,
                onsubmit: submit,
                requests,
                error: command_error(),
                on_restore: move |index: usize| {
                    if let Some((request, target)) = requests.read().get(index) {
                        cmdbar::prefill(composer, request.clone());
                        if let Some(target) = target { app_state.write().select_artifact(*target); }
                    }
                },
            }
        }
        super::artwork_import::ArtworkImport { app_state, asset_directory, disabled, trigger: import_trigger }
        if let Some((action, project)) = pending_action() {
            ArtActionDialog {
                action, project, app_state, defaults, disabled,
                onrun: move |operation| {
                    commands::run_operations(app_state, backend(), vec![operation], defaults.read().clone(), ExecutionUi { activity, error_report });
                },
                onclose: move |_| pending_action.set(None),
            }
        }
        {proposal_modal}
    }
}

/// One LLM-proposed operation, validated against the registry before the
/// approval modal is shown.
#[derive(Clone, Debug, PartialEq)]
struct ProposedOp {
    operation: Operation,
    /// Why the op would fail at compile time (shown, but not runnable).
    invalid_reason: Option<String>,
    /// A semantic warning for ops that run but do not do what the request
    /// sounds like (e.g. a `draft` on a visual artifact never changes the
    /// image — weak models misplan these).
    note: Option<String>,
}

/// A free-form message parsed into a proposed operation stack.
#[derive(Clone, Debug, PartialEq)]
struct StackProposal {
    operations: Vec<ProposedOp>,
}

/// Free-form composer text → the LLM proposes an operation stack against
/// the closed vocabulary; the proposal is validated and shown for approval
/// (LLM proposals are gated, per the execution model). A soft dependency:
/// without the endpoint this degrades to a hint, never a hard failure.
fn propose_operations(
    app_state: Signal<AppState>,
    backend: CreativeBackend,
    message: String,
    defaults: storyfella_studio_core::registry::GenerationDefaults,
    mut activity: Signal<OpActivity>,
    mut proposal: Signal<Option<StackProposal>>,
    mut error_report: Signal<Option<ReportedError>>,
) {
    if !backend.has_llm() {
        error_report.set(Some(
            ReportedError::plain(
                ErrorCode::AppLlmUnavailable,
                "The LLM endpoint is not configured or reachable — use slash commands instead \
                 (/create, /variant, /regenerate, /modify, /describe, /attach-lora)",
            )
            .with_context("message", message)
            .log(),
        ));
        return;
    }
    activity.set(OpActivity::Running {
        summary: "proposing operations to the LLM…".to_owned(),
    });
    spawn(async move {
        let registry = app_state.read().registry.clone();
        let viewing = app_state.read().selected_artifact().cloned();
        match backend
            .propose_stack(&registry, &defaults, viewing.as_ref(), &message)
            .await
        {
            Ok(stack) => {
                let operations = validate_proposal(&registry, stack.operations, &defaults);
                activity.set(OpActivity::Idle);
                proposal.set(Some(StackProposal { operations }));
            }
            Err(error) => {
                activity.set(OpActivity::Idle);
                // The user message is the context: it is what the LLM was
                // asked to turn into operations.
                error_report.set(Some(
                    ReportedError::plain(ErrorCode::AppProposalFailed, error)
                        .with_context("message", message)
                        .log(),
                ));
            }
        }
    });
}

/// Static validation of a proposed stack against the registry: ops that
/// would fail to compile are marked so the approval modal shows (but does
/// not run) them; ops that compile but are semantically suspect for the
/// request get a warning note.
fn validate_proposal(
    registry: &ArtifactRegistry,
    operations: Vec<Operation>,
    defaults: &storyfella_studio_core::registry::GenerationDefaults,
) -> Vec<ProposedOp> {
    operations
        .into_iter()
        .map(|operation| {
            let invalid_reason = ops::compile(registry, &operation, defaults)
                .err()
                .map(|error| error.to_string());
            let note = draft_on_visual_artifact(registry, &operation);
            ProposedOp {
                invalid_reason,
                note,
                operation,
            }
        })
        .collect()
}

/// A `draft` resolves to a visual artifact: the op compiles and runs, but
/// it only appends text — it never changes the image. The modal warns about
/// it so a misplanned stack is visible before approval.
fn draft_on_visual_artifact(registry: &ArtifactRegistry, operation: &Operation) -> Option<String> {
    let Operation::Draft { target, .. } = operation else {
        return None;
    };
    let id = registry.resolve(target).ok()?;
    let artifact = registry.artifact(id)?;
    artifact.kind.is_visual().then(|| {
        "a draft is a text note — it never changes the image. \
         Use /modify (change the image), /describe (change what the artifact \
         IS, which every later edit is measured against), /variant (new look), \
         or /regenerate (redo)."
            .to_owned()
    })
}

fn is_visual(artifact: &Artifact) -> bool {
    artifact.kind.is_visual()
}

/// The LLM proposal review: a free-form message parsed into an operation
/// stack, each op toggleable. Approving runs the selected ops in order
/// through the same worker-thread machinery as slash commands; ops that
/// would fail static validation are shown but not runnable.
#[component]
fn StackProposalModal(
    operations: Vec<ProposedOp>,
    on_approve: EventHandler<Vec<Operation>>,
    on_cancel: EventHandler<()>,
) -> Element {
    let mut selected = use_signal(|| {
        operations
            .iter()
            .map(|op| op.invalid_reason.is_none())
            .collect::<Vec<bool>>()
    });
    let selected_count = selected.read().iter().filter(|&&picked| picked).count();
    let run_label = format!(
        "Run {selected_count} operation{}",
        if selected_count == 1 { "" } else { "s" }
    );
    rsx! {
        super::dialog::Dialog { title: "Review proposed operations", on_close: on_cancel,
            div { class: "w-full max-w-2xl rounded-2xl bg-zinc-950 p-6 shadow-[0_30px_90px_rgba(0,0,0,.5)] ring-1 ring-white/[0.09]",
                div { class: "mb-1 flex items-center gap-2",
                    span { class: "size-2 animate-pulse rounded-full bg-violet-400" }
                    p { class: "text-[10px] font-semibold uppercase tracking-[0.18em] text-zinc-500", "LLM proposal" }
                }
                h2 { class: "mb-1 text-base font-semibold text-zinc-100", "I interpreted your request as these operations" }
                p { class: "mb-5 text-[10px] leading-4 text-zinc-600",
                    "Uncheck anything you don't want, then run the rest in order. Amber notes warn about ops that run but may not do what you asked; checkpoints inside an op still ask you."
                }
                div { class: "max-h-72 space-y-1 overflow-y-auto pr-1",
                    for (index, op) in operations.iter().enumerate() {
                        div {
                            class: if op.invalid_reason.is_some() {
                                "flex items-start gap-3 rounded-lg bg-rose-400/[0.05] px-3 py-2.5 ring-1 ring-inset ring-rose-400/15"
                            } else {
                                "flex items-start gap-3 rounded-lg bg-white/[0.03] px-3 py-2.5 ring-1 ring-inset ring-white/[0.05]"
                            },
                            input {
                                aria_label: "Run {op.operation.summary()}",
                                r#type: "checkbox",
                                class: "mt-0.5 size-3.5 accent-violet-500",
                                checked: selected.read().get(index).copied().unwrap_or(false),
                                disabled: op.invalid_reason.is_some(),
                                onchange: move |_| {
                                    let mut selection = selected.write();
                                    if let Some(state) = selection.get_mut(index) {
                                        *state = !*state;
                                    }
                                },
                            }
                            div { class: "min-w-0 flex-1",
                                p { class: "truncate font-mono text-[10px] text-zinc-300", "{op.operation.summary()}" }
                                if let Some(reason) = &op.invalid_reason {
                                    p { class: "mt-0.5 text-[9px] leading-4 text-rose-300/80", "will fail: {reason}" }
                                } else if let Some(note) = &op.note {
                                    p { class: "mt-0.5 text-[9px] leading-4 text-amber-300/80", "{note}" }
                                }
                            }
                        }
                    }
                }
                div { class: "mt-6 flex justify-end gap-2",
                    button {
                        class: "flex h-8 items-center rounded-lg bg-white/[0.05] px-3.5 text-[10px] font-medium text-zinc-300 ring-1 ring-inset ring-white/[0.07] transition hover:bg-white/[0.09] hover:text-white",
                        onclick: move |_| on_cancel.call(()),
                        "Discard"
                    }
                    button {
                        class: "flex h-8 items-center rounded-lg bg-zinc-100 px-3.5 text-[10px] font-semibold text-zinc-950 transition hover:bg-white disabled:cursor-not-allowed disabled:opacity-35",
                        disabled: selected_count == 0,
                        onclick: move |_| {
                            let chosen: Vec<Operation> = operations
                                .iter()
                                .enumerate()
                                .filter(|(index, _)| selected.read().get(*index).copied().unwrap_or(false))
                                .map(|(_, op)| op.operation.clone())
                                .collect();
                            on_approve.call(chosen);
                        },
                        "{run_label}"
                    }
                }
            }
        }
    }
}

fn revision_status_label(status: RevisionStatus) -> &'static str {
    match status {
        RevisionStatus::Queued => "Queued",
        RevisionStatus::Generating => "Generating",
        RevisionStatus::Completed => "Ready",
        RevisionStatus::Failed => "Failed",
        RevisionStatus::Cancelled => "Cancelled",
    }
}

/// One row of the revisions list — computed before the rsx so the loop
/// body stays pure element markup.
struct RevisionRow {
    id: Uuid,
    prompt: String,
    status: RevisionStatus,
    has_asset: bool,
    meta: String,
    /// Whether this rendered the description or an experiment against it.
    origin: storyfella_studio_core::registry::RevisionOrigin,
    /// The `/describe` that would make this experiment canonical.
    promote: String,
}

/// "Revision 3 · qwen-image-2.1 · Ready" — the 1-based index plus model
/// and status.
fn revision_meta_label(index: usize, revision: &ArtifactRevision) -> String {
    let model = revision
        .model
        .as_deref()
        .map(|model| format!(" · {model}"))
        .unwrap_or_default();
    format!(
        "Revision {} ·{model} · {}",
        index + 1,
        revision_status_label(revision.status)
    )
}

fn status_dot_class(status: RevisionStatus) -> &'static str {
    match status {
        RevisionStatus::Completed => "size-1.5 rounded-full bg-emerald-400",
        RevisionStatus::Failed | RevisionStatus::Cancelled => "size-1.5 rounded-full bg-rose-400",
        RevisionStatus::Queued | RevisionStatus::Generating => "size-1.5 rounded-full bg-amber-400",
    }
}

// ---------------------------------------------------------------- detail

#[component]
fn Detail(
    app_state: Signal<AppState>,
    artifact: Option<Artifact>,
    activity: Signal<OpActivity>,
    disabled: bool,
    on_rerun: EventHandler<Operation>,
    /// Runs a command exactly as the command bar would.
    onrun: EventHandler<String>,
) -> Element {
    let show_inspector = use_context::<InspectorOpen>().0;
    let count = app_state.read().registry.artifacts.len();
    let rail = artifact.as_ref().map(|artifact| {
        let registry = app_state.read().registry.clone();
        let reference = registry.ref_of(artifact.id);
        let bound_loras: Vec<storyfella_studio_core::registry::LoraEntry> = registry
            .loras_bound_to(artifact.id)
            .into_iter()
            .cloned()
            .collect();
        rsx! {
            Inspector {
                key: "{artifact.id}",
                artifact: artifact.clone(),
                reference,
                bound_loras,
                disabled,
                onrun,
            }
        }
    });
    rsx! {
        div { class: "flex min-h-0 w-full flex-1",
        div { class: "min-h-0 flex-1 overflow-y-auto",
            match artifact {
                Some(artifact) => rsx! {
                    ArtifactDetail {
                        app_state,
                        artifact: artifact.clone(),
                        registry: app_state.read().registry.clone(),
                        activity,
                        disabled,
                        onrun,
                        on_rerun,
                    }
                },
                None => rsx! {
                    div { class: "grid h-full min-h-[420px] place-items-center px-10",
                        EmptyVisual {
                            icon: IconName::Layers,
                            title: if count == 0 { "Start with an artifact" } else { "Select an artifact" },
                            description: if count == 0 { "Create your first character, environment, or object with the prompt bar below." } else { "Pick one from the grid, or run /create for a new one." }
                        }
                    }
                },
            }
        }
        if show_inspector() { {rail} }
        }
    }
}

#[component]
fn ArtifactDetail(
    app_state: Signal<AppState>,
    artifact: Artifact,
    registry: storyfella_studio_core::registry::ArtifactRegistry,
    activity: Signal<OpActivity>,
    disabled: bool,
    onrun: EventHandler<String>,
    on_rerun: EventHandler<Operation>,
) -> Element {
    let actions = use_context::<ActionRequest>();
    let mut show_inspector = use_context::<InspectorOpen>().0;
    let mut show_history = use_context::<HistoryOpen>().0;
    let mut show_variants = use_context::<VariantsOpen>().0;
    let image = app_state.read().display_image(&artifact).map(str::to_owned);
    let is_visual = is_visual(&artifact);
    let mut screen = use_context::<Signal<super::AppScreen>>();
    let caret = use_context::<super::WriterCaret>();
    let analysis = storyfella_language::analyze(
        app_state.read().story.source(),
        &storyfella_studio_core::authoring::AssetCatalog::from_registry(&registry).assets,
    );
    let links = storyfella_studio_core::authoring::source_links(&registry, &analysis, artifact.id);
    let positioned_links: Vec<_> = links
        .into_iter()
        .filter_map(|link| {
            analysis
                .document
                .source
                .position(link.span.start)
                .map(|position| (link.label, position))
        })
        .collect();
    let detached = matches!(
        artifact.kind,
        storyfella_studio_core::registry::ArtifactKind::Scene
            | storyfella_studio_core::registry::ArtifactKind::Beat
    ) && storyfella_studio_core::authoring::declaration_key(&registry, artifact.id)
        .is_none();

    // Size and bound adapters are the inspector rail's business now.
    // Always the base's family, never the open artifact's children —
    // otherwise opening a variant empties the strip you navigated with.
    let base_id = artifact.variant_of.unwrap_or(artifact.id);
    let variants: Vec<Artifact> = registry
        .artifacts
        .iter()
        .filter(|candidate| candidate.variant_of == Some(base_id))
        .cloned()
        .collect();
    let children: Vec<Artifact> = registry
        .artifacts
        .iter()
        .filter(|candidate| candidate.parent_id == Some(artifact.id))
        .cloned()
        .collect();
    let log_entries: Vec<storyfella_studio_core::registry::ops::OperationLogEntry> =
        registry.log_for(artifact.id).cloned().collect();
    // Commands emitted by history use identity, independent of display names.
    let ref_label = format!("c:{}", artifact.id);
    // The strip shows the base and its variants, so it stays the same set
    // whether you opened the base or one of its variants.
    let base_thumb = registry
        .latest_image(base_id)
        .and_then(|revision| revision.asset_path.as_deref())
        .and_then(generated_asset_url);
    let revision_rows: Vec<RevisionRow> = artifact
        .revisions
        .iter()
        .enumerate()
        .rev()
        .map(|(index, revision)| RevisionRow {
            id: revision.id,
            prompt: revision.prompt.clone(),
            status: revision.status,
            has_asset: revision.asset_path.is_some(),
            meta: revision_meta_label(index, revision),
            origin: revision.origin,
            promote: format!("/describe {ref_label} {:?}", revision.prompt.trim()),
        })
        .collect();

    rsx! {
        // Three regions with their own space, so the page never scrolls:
        // the render takes what is left, the filmstrip is a fixed row, and
        // the history scrolls inside itself. Scrolling *past* the picture to
        // reach the revisions was the thing that made this feel like a
        // document rather than a workspace.
        div { class: "relative flex h-full min-h-0 flex-col",
            if detached {
                p { class: "mt-12 px-4 py-2 text-[11px] text-amber-300", "Detached artwork · add story structure in Writer" }
            }
            if !positioned_links.is_empty() {
                div { class: "mt-12 flex flex-wrap items-center gap-2 px-4 py-2",
                    span { class: "text-[10px] text-zinc-500", "Appears in" }
                    for (label, position) in positioned_links {
                        button {
                            class: "rounded bg-white/[0.05] px-2 py-1 font-mono text-[11px] text-violet-300 hover:text-white",
                            onclick: move |_| { caret.navigate(position); screen.set(super::AppScreen::Writer); },
                            "{label} · line {position.line + 1}"
                        }
                    }
                }
            }

            // Keep actions above the render so they never cover the image.
            div { class: "flex shrink-0 items-center gap-2 border-b border-white/[0.05] px-4 py-2.5",
                div { class: "pointer-events-auto flex min-w-0 items-center gap-1.5",
                    button {
                        class: "-ml-1.5 flex items-center gap-1 rounded-lg px-1.5 py-1 text-[11px] font-medium text-zinc-500 transition hover:bg-white/[0.06] hover:text-zinc-200",
                        title: shortcuts::tooltip("All artifacts", "back"),
                        onclick: move |_| app_state.write().clear_selection(),
                        span { class: "text-[13px] leading-none", "‹" }
                        "All artifacts"
                    }
                    span { class: "text-zinc-700", "/" }
                    span { class: "truncate text-[12px] font-semibold text-zinc-200",
                        if artifact.name.trim().is_empty() { "Unnamed" } else { "{artifact.name}" }
                    }
                    if let Some(axis) = artifact.variant_axis {
                        span { class: "shrink-0 rounded bg-violet-400/15 px-1.5 py-0.5 text-[9px] font-medium text-violet-300", "{axis}" }
                    }
                }

                div { class: "pointer-events-auto ml-auto flex shrink-0 items-center gap-0.5",
                    if is_visual {
                        IconButton {
                            icon: IconName::Add,
                            label: shortcuts::tooltip("New variant", "new-variant"),
                            disabled,
                            onclick: move |_| actions.send(StudioAction::Art(ArtAction::Variant(base_id))),
                        }
                        IconButton {
                            icon: IconName::Refresh,
                            label: shortcuts::tooltip("Regenerate", "regenerate"),
                            disabled,
                            onclick: move |_| actions.send(StudioAction::Art(ArtAction::Regenerate(artifact.id))),
                        }
                        if image.is_some() {
                            IconButton {
                                icon: IconName::Edit,
                                label: shortcuts::tooltip("Edit image", "modify"),
                                disabled,
                                onclick: move |_| actions.send(StudioAction::Art(ArtAction::Modify(artifact.id))),
                            }
                        }
                        span { class: "mx-1 h-4 w-px bg-white/[0.08]" }
                        IconButton {
                            icon: IconName::Layers,
                            label: shortcuts::tooltip(
                                if show_variants() { "Hide variants" } else { "Show variants" },
                                "toggle-variants",
                            ),
                            active: show_variants(),
                            onclick: move |_| { let now = show_variants(); show_variants.set(!now); },
                        }
                    }
                    IconButton {
                        icon: IconName::List,
                        label: shortcuts::tooltip(
                            if show_history() { "Hide history" } else { "Show history" },
                            "toggle-history",
                        ),
                        active: show_history(),
                        onclick: move |_| { let now = show_history(); show_history.set(!now); },
                    }
                    IconButton {
                        icon: IconName::Sliders,
                        label: if show_inspector() { "Hide inspector" } else { "Show inspector" },
                        active: show_inspector(),
                        onclick: move |_| { let open = show_inspector(); show_inspector.set(!open); },
                    }
                }
            }

            // The render takes whatever the other two regions leave.
            div { class: "relative min-h-0 flex-1 overflow-hidden bg-black/25",
                match image {
                    Some(path) => rsx! {
                        if let Some(url) = generated_asset_url(&path) {
                            div { class: "absolute inset-3",
                                img { class: "h-full w-full object-contain", src: "{url}", alt: artifact.name }
                            }
                        }
                    },
                    None => rsx! {
                        div { class: "grid h-full place-items-center",
                            EmptyVisual {
                                icon: IconName::Generate,
                                title: if is_visual { "No image yet" } else { "Not a visual artifact" },
                                description: if is_visual { "Regenerate to create an image." } else { "" }
                            }
                        }
                    },
                }
            }

            if is_visual && show_variants() {
                // Keep the strip below the render. Always the base plus
                // its variants, whichever member is open.
                div { class: "flex shrink-0 items-end gap-2 overflow-x-auto border-t border-white/[0.05] px-4 py-2",
                    div { class: "pointer-events-auto flex items-start gap-2 rounded-xl bg-black/45 p-2 backdrop-blur ring-1 ring-inset ring-white/[0.07]",
                    VariantChip {
                        label: "base".to_owned(),
                        image: base_thumb.clone(),
                        active: artifact.variant_of.is_none(),
                        target: base_id,
                        onpick: move |id| { app_state.write().select_artifact(id); },
                    }
                    for (index , variant) in variants.iter().enumerate() {
                        VariantChip {
                            key: "{variant.id}",
                            label: format!("v{}", index + 1),
                            image: registry
                                .latest_image(variant.id)
                                .and_then(|revision| revision.asset_path.as_deref())
                                .and_then(generated_asset_url),
                            active: variant.id == artifact.id,
                            target: variant.id,
                            onpick: move |id| { app_state.write().select_artifact(id); },
                        }
                    }
                    button {
                        class: "shrink-0",
                        disabled,
                        title: shortcuts::tooltip("New variant", "new-variant"),
                        onclick: move |_| actions.send(StudioAction::Art(ArtAction::Variant(base_id))),
                        div { class: "grid h-16 w-12 place-items-center rounded-lg border border-dashed border-white/[0.12] text-lg font-light text-zinc-700 transition hover:text-zinc-300",
                            "+"
                        }
                        // Matches the chips' label line so every item in the
                        // row is the same height.
                        p { class: "mt-1 text-center font-mono text-[9px] text-transparent", "new" }
                    }
                    }
                }
            }

            // Collapsed by default: the picture is the subject, and the
            // history is reference you open when you want it.
            if show_history() {
            div { class: "min-h-0 max-h-[38%] shrink-0 overflow-y-auto border-t border-white/[0.05] px-5 pb-4 pt-3",
            if !artifact.revisions.is_empty() {
                Section { title: format!("Revisions ({})", artifact.revisions.len()),
                    // Canon renders the description; an experiment renders a
                    // delta against it. Because edits ground against the
                    // description rather than the current image, experiments
                    // never accumulate — Promote is what makes one stick.
                    div { class: "space-y-0.5",
                        for revision in revision_rows {
                            div {
                                class: if artifact.active_revision_id == Some(revision.id) { "flex w-full items-center gap-2.5 rounded-md bg-white/[0.06] px-2.5 py-1" } else { "group flex w-full items-center gap-2.5 rounded-md px-2.5 py-1 transition hover:bg-white/[0.035]" },
                                span { class: "{status_dot_class(revision.status)}" }
                                button {
                                    class: "min-w-0 flex-1 text-left disabled:opacity-40",
                                    disabled: !revision.has_asset,
                                    title: if revision.has_asset { "Show this revision" } else { "No image for this revision" },
                                    onclick: {
                                        let artifact_id = artifact.id;
                                        let revision_id = revision.id;
                                        move |_| { app_state.write().activate_revision(artifact_id, revision_id); }
                                    },
                                    p { class: "truncate text-[11px] leading-4 text-zinc-300", "{revision.prompt}" }
                                    p { class: "truncate text-[9px] leading-3 text-zinc-700", "{revision.meta}" }
                                }
                                if revision.origin == storyfella_studio_core::registry::RevisionOrigin::Experiment {
                                    span { class: "shrink-0 rounded bg-amber-400/15 px-1.5 py-0.5 text-[9px] font-bold uppercase tracking-[0.05em] text-amber-300",
                                        "experiment"
                                    }
                                    button {
                                        class: "shrink-0 rounded-md bg-amber-400/15 px-2 py-1 text-[10px] font-semibold text-amber-300 transition hover:bg-amber-400/25 disabled:opacity-30",
                                        disabled,
                                        title: "Make this the artifact's description, so later edits build on it",
                                        onclick: {
                                            let command = revision.promote.clone();
                                            move |_| onrun.call(command.clone())
                                        },
                                        "Promote"
                                    }
                                } else {
                                    span { class: "shrink-0 rounded bg-violet-400/15 px-1.5 py-0.5 text-[9px] font-bold uppercase tracking-[0.05em] text-violet-300",
                                        "canon"
                                    }
                                }
                            }
                        }
                    }
                }
            }

            if !children.is_empty() {
                Section { title: format!("Children ({})", children.len()),
                    div { class: "flex flex-wrap gap-2",
                        for child in children {
                            button {
                                class: "flex items-center gap-2 rounded-lg bg-white/[0.045] px-3 py-1.5 text-[10px] text-zinc-400 transition hover:bg-white/[0.07] hover:text-zinc-200",
                                onclick: {
                                    let id = child.id;
                                    move |_| app_state.write().select_artifact(id)
                                },
                                span { class: "text-zinc-700", "{child.kind}" }
                                "{child.name}"
                            }
                        }
                    }
                }
            }

            if !artifact.drafts.is_empty() {
                Section { title: format!("Story text ({})", artifact.drafts.len()),
                    div { class: "space-y-3",
                        for draft in &artifact.drafts {
                            div { class: "rounded-lg bg-white/[0.03] p-3 ring-1 ring-inset ring-white/[0.06]",
                                p { class: "text-[9px] uppercase tracking-[0.14em] text-zinc-700", "{draft.request}" }
                                p { class: "mt-1.5 whitespace-pre-wrap text-[11px] leading-5 text-zinc-400", "{draft.text}" }
                            }
                        }
                    }
                }
            }

            if !log_entries.is_empty() {
                Section { title: format!("Operation log ({})", log_entries.len()),
                    div { class: "space-y-0.5",
                        for entry in log_entries.iter().rev() {
                            div { class: "flex items-center gap-3 rounded-lg px-2 py-1.5",
                                span { class: "size-1.5 shrink-0 rounded-full {log_status_class(entry.status)}" }
                                p { class: "min-w-0 flex-1 truncate font-mono text-[10px] text-zinc-500", "{entry.op.summary()}" }
                                span { class: "shrink-0 text-[9px] uppercase tracking-wider text-zinc-700", "{log_status_label(entry.status)}" }
                                RerunButton {
                                    operation: entry.op.clone(),
                                    applied: entry.status == storyfella_studio_core::registry::ops::OpStatus::Applied,
                                    disabled,
                                    on_rerun,
                                }
                            }
                        }
                    }
                }
            }
            }
            }
        }
    }
}

/// One frame in the variant filmstrip.
#[component]
fn VariantChip(
    label: String,
    image: Option<String>,
    active: bool,
    target: Uuid,
    onpick: EventHandler<Uuid>,
) -> Element {
    rsx! {
        button {
            class: "group shrink-0",
            onclick: move |_| onpick.call(target),
            div {
                class: if active { "relative h-16 w-12 overflow-hidden rounded-lg bg-white/[0.04] ring-2 ring-violet-400" } else { "relative h-16 w-12 overflow-hidden rounded-lg bg-white/[0.04] ring-1 ring-inset ring-white/[0.09] transition group-hover:ring-violet-400/50" },
                if let Some(url) = image {
                    img { class: "h-full w-full object-cover", src: "{url}", alt: "{label}" }
                } else {
                    div { class: "absolute inset-0 grid place-items-center text-zinc-700",
                        Icon { name: IconName::Layers, class: "size-3.5" }
                    }
                }
            }
            p {
                class: if active { "mt-1 text-center font-mono text-[9px] text-violet-300" } else { "mt-1 text-center font-mono text-[9px] text-zinc-600" },
                "{label}"
            }
        }
    }
}

fn log_status_class(status: storyfella_studio_core::registry::ops::OpStatus) -> &'static str {
    match status {
        storyfella_studio_core::registry::ops::OpStatus::Applied => "bg-emerald-400",
        storyfella_studio_core::registry::ops::OpStatus::Proposed => "bg-amber-400",
        storyfella_studio_core::registry::ops::OpStatus::Rejected
        | storyfella_studio_core::registry::ops::OpStatus::Reverted => "bg-rose-400",
    }
}

fn log_status_label(status: storyfella_studio_core::registry::ops::OpStatus) -> &'static str {
    match status {
        storyfella_studio_core::registry::ops::OpStatus::Applied => "applied",
        storyfella_studio_core::registry::ops::OpStatus::Proposed => "proposed",
        storyfella_studio_core::registry::ops::OpStatus::Rejected => "rejected",
        storyfella_studio_core::registry::ops::OpStatus::Reverted => "reverted",
    }
}

/// Whether re-running an operation is meaningful: only ops that produce a
/// *new* visual/text result without duplicating state. `create`/`variant`/
/// `compose` would mint duplicate artifacts; the LoRA ops are registry
/// mutations that re-applying would redo.
fn is_rerunnable(operation: &Operation) -> bool {
    matches!(
        operation,
        Operation::Regenerate { .. }
            | Operation::Resize { .. }
            | Operation::Modify { .. }
            | Operation::Draft { .. }
    )
}

/// The re-run action for one operation-log row: re-executes the same op
/// with a fresh seed. Disabled (with a reason) for ops that would duplicate
/// state or that were never applied.
#[component]
fn RerunButton(
    operation: Operation,
    applied: bool,
    disabled: bool,
    on_rerun: EventHandler<Operation>,
) -> Element {
    let rerunnable = applied && is_rerunnable(&operation);
    let title = if !applied {
        "Only applied operations can be re-run"
    } else if !is_rerunnable(&operation) {
        "Re-running this would duplicate state — use /regenerate or /modify instead"
    } else {
        "Re-run this operation (fresh seed)"
    };
    rsx! {
        button {
            class: "grid size-6 shrink-0 place-items-center rounded-md text-zinc-700 transition hover:bg-white/[0.06] hover:text-zinc-200 disabled:cursor-not-allowed disabled:opacity-30",
            disabled: disabled || !rerunnable,
            title,
            aria_label: "Re-run",
            onclick: move |_| on_rerun.call(operation.clone()),
            Icon { name: IconName::Refresh, class: "size-2.5" }
        }
    }
}

#[component]
fn ProvenanceLink(
    label: String,
    registry: storyfella_studio_core::registry::ArtifactRegistry,
    target: Uuid,
    on_select: EventHandler<Uuid>,
) -> Element {
    let Some(artifact) = registry.artifact(target) else {
        return rsx! { Fragment {} };
    };
    rsx! {
        button {
            class: "mt-2 flex items-center gap-1.5 text-[10px] text-zinc-600 transition hover:text-violet-300",
            onclick: move |_| on_select.call(target),
            span { class: "uppercase tracking-[0.14em] text-zinc-700", "{label}" }
            span { class: "truncate font-mono", "{registry.ref_of(target)}" }
            span { class: "text-zinc-700", "({artifact.kind})" }
        }
    }
}

#[component]
fn Section(title: String, children: Element) -> Element {
    rsx! {
        div { class: "mb-4 last:mb-0",
            h3 { class: "mb-1.5 text-[9px] font-semibold uppercase tracking-[0.16em] text-zinc-600", "{title}" }
            {children}
        }
    }
}

#[component]
fn ActionButton(
    label: String,
    icon: IconName,
    disabled: bool,
    title: String,
    onclick: EventHandler<MouseEvent>,
) -> Element {
    rsx! {
        button {
            class: "flex h-8 items-center gap-1.5 rounded-lg bg-white/[0.05] px-3 text-[10px] font-medium text-zinc-300 ring-1 ring-inset ring-white/[0.07] transition hover:bg-white/[0.09] hover:text-white disabled:cursor-not-allowed disabled:opacity-35",
            disabled,
            title,
            onclick,
            Icon { name: icon, class: "size-3" }
            "{label}"
        }
    }
}

// ---------------------------------------------------------------- composer

#[component]
fn Composer(
    value: Signal<String>,
    app_state: Signal<AppState>,
    activity: Signal<OpActivity>,
    disabled: bool,
    target_hint: Option<String>,
    /// The artifacts the bar offers as `c:` targets, and the set it treats
    /// as resolving.
    ref_candidates: Vec<RefCandidate>,
    onsubmit: EventHandler<()>,
    requests: Signal<Vec<(String, Option<Uuid>)>>,
    on_restore: EventHandler<usize>,
    error: Option<ReportedError>,
) -> Element {
    let summary = match &*activity.read() {
        OpActivity::Running { summary } => Some(summary.clone()),
        _ => None,
    };
    rsx! {
        footer { class: "studio-composer",
            div { class: "mx-auto w-full max-w-4xl",
                div { class: "studio-command-surface",
                    Icon { name: IconName::Search, class: "size-4 shrink-0 text-zinc-400" }
                    CommandBar { value, app_state, refs: ref_candidates, disabled, onsubmit }
                    if !requests.read().is_empty() {
                        select { aria_label: "Restore a submitted request", title: "Recent requests", disabled, class: "max-w-[120px] shrink-0",
                            value: "", onchange: move |event| { if let Ok(index) = event.value().parse::<usize>() { on_restore.call(index); } },
                            option { value: "", "Recent" }
                            for (index, (request, _)) in requests.read().iter().enumerate() { option { value: "{index}", "{request}" } }
                        }
                    }
                    if let Some(target) = target_hint {
                        span { class: "max-w-[140px] truncate border-l border-white/10 pl-2 text-xs text-zinc-400",
                            title: "Change requests target {target}", {target.trim_start_matches("c:")} }
                    }
                }
                if let Some(summary) = summary { p { role: "status", class: "mt-2 flex items-center gap-2 text-xs text-zinc-400",
                    span { class: "size-2.5 animate-spin rounded-full border border-zinc-700 border-t-violet-300" }
                    "{summary}"
                } }
                if let Some(report) = error {
                    div { role: "alert", class: "studio-inline-error mt-2 flex items-center gap-2",
                        span { class: "min-w-0 flex-1", "{report.code.message()}" }
                        super::components::ErrorDetailsButton { error: report }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use storyfella_studio_core::registry::{ArtifactKind, CreateArtifact, Ref};

    fn registry_with(name: &str, kind: ArtifactKind) -> ArtifactRegistry {
        let mut registry = ArtifactRegistry::new();
        registry
            .create_artifact(
                kind,
                CreateArtifact {
                    name: name.into(),
                    description: "a test artifact".into(),
                    ..Default::default()
                },
            )
            .expect("artifact should create");
        registry
    }

    #[test]
    fn validate_proposal_marks_unresolvable_ops() {
        let mut registry = registry_with("mia", ArtifactKind::Character);
        registry
            .create_artifact(
                ArtifactKind::Story,
                CreateArtifact {
                    name: "story".into(),
                    description: "a test story".into(),
                    ..Default::default()
                },
            )
            .expect("story should create");
        let valid = Operation::Create {
            kind: ArtifactKind::Object,
            description: "a lantern".into(),
            name: None,
            size: None,
            steps: None,
            loras: Vec::new(),
        };
        let unknown_ref = Operation::Variant {
            target: Ref::new("c:ghost"),
            description: "a variant of nobody".into(),
            name: None,
            axis: None,
            size: None,
            steps: None,
            loras: Vec::new(),
        };
        let modify_without_image = Operation::Modify {
            target: Ref::new("c:mia"),
            description: "change her hair".into(),
            size: None,
            steps: None,
            loras: Vec::new(),
        };
        let draft_on_visual = Operation::Draft {
            target: Ref::new("c:mia"),
            request: "add glasses".into(),
        };
        let draft_on_story = Operation::Draft {
            target: Ref::new("c:story"),
            request: "note the theme".into(),
        };
        let ops = validate_proposal(
            &registry,
            vec![
                valid,
                unknown_ref,
                modify_without_image,
                draft_on_visual,
                draft_on_story,
            ],
            &storyfella_studio_core::registry::GenerationDefaults {
                sizes: Default::default(),
                steps: 4,
                cfg_scale: clipforge_inference::sd::DEFAULT_CFG_SCALE,
                sample_method: clipforge_inference::sd::DEFAULT_SAMPLE_METHOD.to_owned(),
                loras: Default::default(),
            },
        );
        assert!(ops[0].invalid_reason.is_none());
        assert!(
            ops[1]
                .invalid_reason
                .as_deref()
                .unwrap()
                .contains("c:ghost"),
            "unknown refs surface as invalid: {:?}",
            ops[1].invalid_reason
        );
        assert!(
            ops[2].invalid_reason.is_some(),
            "modify without a completed image cannot compile"
        );
        assert!(
            ops[3].note.as_deref().unwrap().contains("text note"),
            "a draft on a visual artifact warns it never changes the image: {:?}",
            ops[3].note
        );
        assert!(ops[3].invalid_reason.is_none(), "the draft still runs");
        assert!(
            ops[4].note.is_none(),
            "a draft on a text artifact is a legitimate request"
        );
    }
}
