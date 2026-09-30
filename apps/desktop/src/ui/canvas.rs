//! The canvas — the artifact workspace (roadmap §12 item 4).
//!
//! Two states, not a split. At rest the screen is the artifact grid
//! (`super::gallery`); opening one replaces it with the detail pane (image,
//! description, variants, revisions, children, operation log). The tokenized
//! command bar (`super::cmdbar`) sits under both. Context actions (New
//! variant, Regenerate, Modify) prefill it, so there is exactly one input
//! path — the same typed vocabulary as the `svs` CLI.
//!
//! `modify` runs its pipeline on a worker thread (own tokio runtime) so the
//! mask checkpoint can block on the user without freezing the UI: the worker
//! forwards the segmentation candidates through a channel, the supervisor
//! task (UI thread) shows them in a modal, and the user's choice is sent back
//! to the worker. Stored masks are listed under the active revision.

use std::{cell::RefCell, rc::Rc, sync::mpsc};

use dioxus::prelude::*;
use uuid::Uuid;

use svs_core::errors::{ErrorCode, ReportedError};
use svs_core::registry::{
    backend::CreativeBackend,
    completion::RefCandidate,
    ops::{self, Operation},
    pipeline::{Decision, PlannedEdit},
    slash::is_slash_command,
    Artifact, ArtifactRegistry, ArtifactRevision, RevisionStatus, StoredMask,
};

use crate::{
    commands::{
        self, ExecutionUi, MaskCandidateUi, OpActivity, PendingApproval, PendingMaskApproval,
    },
    state::AppState,
};

use super::{
    cmdbar::CommandBar,
    components::EmptyVisual,
    gallery::Gallery,
    generated_asset_url,
    icons::{Icon, IconName},
    inspector::Inspector,
    mask_asset_url,
    palette::{self, Palette},
    shortcuts,
};

/// The active canvas tool, in context for the same reason the panel toggles
/// are: the key handler lives in the workspace and the rail lives in the
/// detail pane, and a shortcut that does not reach the rail is a shortcut
/// the tooltip lies about.
#[derive(Clone, Copy)]
pub struct ActiveTool(pub Signal<super::tools::Tool>);

/// The strokes painted on the open image, in its own pixel coordinates.
///
/// In context alongside [`ActiveTool`] so the automation driver can paint a
/// demonstration the same way a hand does — through the state the rail and
/// the overlay already read, rather than through a second path that could
/// diverge from the real one.
#[derive(Clone, Copy)]
pub struct PaintedStrokes(pub Signal<Vec<svs_core::registry::pipeline::Stroke>>);

/// Collapse state for the detail panels, in context so the key handler and
/// the pane share one source of truth.
#[derive(Clone, Copy)]
pub struct HistoryOpen(pub Signal<bool>);
#[derive(Clone, Copy)]
pub struct VariantsOpen(pub Signal<bool>);

#[component]
pub fn Canvas(
    app_state: Signal<AppState>,
    backend: Signal<CreativeBackend>,
    defaults: Signal<svs_core::registry::GenerationDefaults>,
) -> Element {
    // Owned by the Workspace: the composer's text is the same signal the
    // automation driver types into (a `compose` step writes it visibly).
    let mut composer = use_context::<Signal<String>>();
    let filter = use_signal(String::new);
    let activity = use_signal(OpActivity::default);
    let mut error_report = use_context::<Signal<Option<ReportedError>>>();
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

    // The mask checkpoint handshake. `reply_slot` is the UI-side end of the
    // channel the op worker blocks on; the modal's buttons send the choice.
    // Both stay on the UI thread (signals are not cross-thread safe).
    let mut pending_approval = use_signal(|| None::<PendingApproval>);
    let reply_slot = use_hook(|| Rc::new(RefCell::new(None::<mpsc::Sender<Decision>>)));
    // The rerun action re-executes an operation from the log; it needs its
    // own reply-slot clone (the closure below moves the original).
    let reply_slot_for_rerun = reply_slot.clone();
    // A free-form message proposed as an operation stack, awaiting approval.
    let mut proposal = use_signal(|| None::<StackProposal>);
    let reviewing = proposal.read().is_some();
    let disabled = busy || reviewing;

    // The inspector's controls are commands: they take the same path a
    // typed one does, so the rail and the bar cannot diverge.
    let reply_slot_for_rail = reply_slot.clone();
    let run_command = move |input: String| {
        if disabled {
            return;
        }
        let ui = ExecutionUi {
            activity,
            pending_approval,
            reply_slot: reply_slot_for_rail.clone(),
            error_report,
        };
        if let Err(report) =
            commands::run_slash_command(app_state, backend(), defaults.read().clone(), ui, &input)
        {
            error_report.set(Some(report.log()));
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
    let active_tool = use_context::<ActiveTool>().0;
    // The grid's keyboard cursor. Kept here rather than inside Gallery so
    // the key handler and the grid share one source of truth.
    let mut grid_cursor = use_context::<super::GridCursor>().0;

    // Keyboard navigation. The listener lives in the Workspace; this
    // observes what it broadcasts, so the canvas gets keys whether or not
    // it happened to mount first.
    let viewing = selected.is_some();
    let filter_query = filter.read().clone();
    let last_key = use_context::<Signal<Option<super::keys::KeyEvent>>>();
    use_effect(move || {
        let Some((_, press)) = last_key() else {
            return;
        };
        if *palette_open.peek() {
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
            "n" if !viewing => composer.set("/create character ".to_owned()),
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
            // Tool keys. `from_key` refuses the ones that are listed but
            // not implemented, so a dimmed rail button and a dead key agree.
            key if viewing
                && key.chars().count() == 1
                && super::tools::Tool::from_key(key.chars().next().unwrap_or(' ')).is_some() =>
            {
                let Some(tool) = super::tools::Tool::from_key(key.chars().next().unwrap_or(' '))
                else {
                    return;
                };
                let mut active = active_tool;
                active.set(tool);
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
                // Prefill rather than run: every one of these needs an
                // argument, so the caret goes to the bar.
                let reference = app_state
                    .peek()
                    .selected_artifact()
                    .map(|artifact| app_state.peek().registry.ref_of(artifact.id));
                if let Some(reference) = reference {
                    let op = match key {
                        "v" => "variant",
                        "r" => "regenerate",
                        _ => "modify",
                    };
                    composer.set(format!("/{op} {reference} "));
                    document::eval(
                        "const el = document.getElementById(\"svs-cmdbar\");
                         if (el) { el.focus(); el.setSelectionRange(el.value.length, el.value.length); }",
                    );
                }
            }
            "/" => {
                document::eval(
                    "const el = document.getElementById(\"svs-cmdbar\");
                     if (el) { el.focus(); }",
                );
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

    let reply_slot_for_run = reply_slot.clone();
    let submit = move |_| {
        if disabled {
            return;
        }
        let input = composer().trim().to_owned();
        if input.is_empty() {
            return;
        }
        composer.set(String::new());
        let defaults = defaults.read().clone();
        if is_slash_command(&input) {
            let ui = ExecutionUi {
                activity,
                pending_approval,
                reply_slot: reply_slot_for_run.clone(),
                error_report,
            };
            if let Err(report) =
                commands::run_slash_command(app_state, backend(), defaults, ui, &input)
            {
                error_report.set(Some(report.log()));
            }
        } else {
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

    // The active modal, built before the rsx (a `let` inside a rsx block is
    // not allowed). The checkpoint modals and the proposal modal are
    // mutually exclusive (the composer is disabled while busy).
    let modal = {
        let pending = pending_approval.read().clone();
        match pending {
            Some(PendingApproval::Mask(approval)) => {
                let reply = reply_slot.clone();
                let on_decide = move |decision: Decision| {
                    if let Some(sender) = reply.borrow_mut().take() {
                        let _ = sender.send(decision);
                    }
                    pending_approval.set(None);
                };
                Some(rsx! {
                    MaskApprovalModal {
                        approval,
                        on_decide,
                    }
                })
            }
            Some(PendingApproval::PlannedEdit { description }) => {
                let reply = reply_slot.clone();
                let on_decide = move |decision: Decision| {
                    if let Some(sender) = reply.borrow_mut().take() {
                        let _ = sender.send(decision);
                    }
                    pending_approval.set(None);
                };
                Some(rsx! {
                    PlanEditModal {
                        description,
                        on_decide,
                    }
                })
            }
            None => None,
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
                            proposal.set(None);
                            let ui = ExecutionUi {
                                activity,
                                pending_approval,
                                reply_slot: reply_slot.clone(),
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
                    composer,
                    disabled,
                }
            } else {
                Detail {
                    app_state,
                    artifact: selected,
                    composer,
                    activity,
                    disabled,
                    onrun: run_command,
                    on_rerun: move |op: Operation| {
                    if disabled {
                        return;
                    }
                    let ui = ExecutionUi {
                        activity,
                        pending_approval,
                        reply_slot: reply_slot_for_rerun.clone(),
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
                activity,
                disabled,
                target_hint,
                ref_candidates,
                onsubmit: submit,
            }
        }
        if palette_open() {
            Palette {
                app_state,
                open: palette_open,
                onpick: move |action| {
                    match action {
                        palette::Action::Compose(command) => {
                            composer.set(command);
                            // The palette hands the caret to the bar, so
                            // the completion for the first argument opens
                            // without another keystroke.
                            document::eval(
                                "const el = document.getElementById(\"svs-cmdbar\");
                                 if (el) { el.focus(); el.setSelectionRange(el.value.length, el.value.length); }",
                            );
                        }
                        palette::Action::Open(id) => app_state.write().select_artifact(id),
                    }
                },
            }
        }
        {modal}
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
/// without LM Studio this degrades to a hint, never a hard failure.
fn propose_operations(
    app_state: Signal<AppState>,
    backend: CreativeBackend,
    message: String,
    defaults: svs_core::registry::GenerationDefaults,
    mut activity: Signal<OpActivity>,
    mut proposal: Signal<Option<StackProposal>>,
    mut error_report: Signal<Option<ReportedError>>,
) {
    if !backend.has_llm() {
        error_report.set(Some(
            ReportedError::plain(
                ErrorCode::AppLlmUnavailable,
                "LM Studio is not configured or reachable — use slash commands instead \
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
    defaults: &svs_core::registry::GenerationDefaults,
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

/// The mask checkpoint modal: the segmentation candidates overlaid on the
/// artifact's image; picking one resumes the `modify` pipeline, Cancel
/// rejects the checkpoint (revision cancelled, op logged rejected).
#[component]
fn MaskApprovalModal(approval: PendingMaskApproval, on_decide: EventHandler<Decision>) -> Element {
    let image_url = approval.image.as_deref().and_then(generated_asset_url);
    let description = approval.description.clone();
    let candidate_count = approval.candidates.len();
    // Score labels computed here: RSX text nodes cannot nest format calls.
    let candidates: Vec<(MaskCandidateUi, String)> = approval
        .candidates
        .iter()
        .map(|candidate| (candidate.clone(), format!("score {:.3}", candidate.score)))
        .collect();
    rsx! {
        div { class: "fixed inset-0 z-50 grid place-items-center bg-black/60 p-6 backdrop-blur-sm",
            div { class: "w-full max-w-3xl rounded-2xl bg-zinc-950 p-6 shadow-[0_30px_90px_rgba(0,0,0,.5)] ring-1 ring-white/[0.09]",
                div { class: "mb-1 flex items-center gap-2",
                    span { class: "size-2 animate-pulse rounded-full bg-amber-400" }
                    p { class: "text-[10px] font-semibold uppercase tracking-[0.18em] text-zinc-500", "Checkpoint" }
                }
                h2 { class: "mb-1 text-base font-semibold text-zinc-100", "{description}" }
                p { class: "mb-5 text-[10px] leading-4 text-zinc-600",
                    "{candidate_count} candidate masks — pick the one that covers the right region, or cancel."
                }
                if candidate_count == 0 {
                    p { class: "mb-5 rounded-lg bg-rose-400/10 px-3 py-2 text-[11px] text-rose-300 ring-1 ring-inset ring-rose-400/20",
                        "Segmentation produced no masks for this prompt."
                    }
                }
                div { class: "grid grid-cols-[repeat(auto-fill,minmax(170px,1fr))] gap-3",
                    for (index, (candidate, score_label)) in candidates.iter().enumerate() {
                        button {
                            class: "group min-w-0 rounded-xl text-left transition hover:ring-2 hover:ring-violet-400/50",
                            title: "Use this mask",
                            onclick: move |_| on_decide.call(Decision::SelectMask(index)),
                            div { class: "relative aspect-square overflow-hidden rounded-xl bg-zinc-900 ring-1 ring-inset ring-white/[0.08]",
                                if let Some(url) = &image_url {
                                    img { class: "absolute inset-0 h-full w-full object-cover", src: "{url}", alt: "artifact" }
                                }
                                if let Some(mask_url) = mask_asset_url(&candidate.path) {
                                    img { class: "absolute inset-0 h-full w-full object-cover opacity-80 mix-blend-screen", src: "{mask_url}", alt: "candidate mask" }
                                }
                                span { class: "absolute left-1.5 top-1.5 rounded bg-black/60 px-1.5 py-0.5 font-mono text-[9px] text-violet-200 backdrop-blur",
                                    "{index + 1}"
                                }
                            }
                            p { class: "mt-1.5 truncate font-mono text-[10px] text-zinc-400", "{score_label}" }
                            p { class: "truncate text-[9px] text-zinc-700", "{candidate.area_pixels} px" }
                        }
                    }
                }
                div { class: "mt-5 flex justify-end gap-2",
                    button {
                        class: "flex h-8 items-center rounded-lg bg-white/[0.05] px-3.5 text-[10px] font-medium text-zinc-300 ring-1 ring-inset ring-white/[0.07] transition hover:bg-white/[0.09] hover:text-white",
                        onclick: move |_| on_decide.call(Decision::Reject),
                        "Cancel"
                    }
                }
            }
        }
    }
}

/// The degraded-LLM fallback for `modify` without explicit prompts: the
/// edit plan (mask region + inpaint look) could not be produced by the LLM,
/// so the user supplies the split manually — the same degradation the CLI
/// offers (`--approve interactive`).
#[component]
fn PlanEditModal(description: String, on_decide: EventHandler<Decision>) -> Element {
    let mut mask_prompt = use_signal(String::new);
    let mut inpaint_prompt = use_signal(String::new);
    let ready = !mask_prompt.read().trim().is_empty() && !inpaint_prompt.read().trim().is_empty();
    rsx! {
        div { class: "fixed inset-0 z-50 grid place-items-center bg-black/60 p-6 backdrop-blur-sm",
            div { class: "w-full max-w-lg rounded-2xl bg-zinc-950 p-6 shadow-[0_30px_90px_rgba(0,0,0,.5)] ring-1 ring-white/[0.09]",
                div { class: "mb-1 flex items-center gap-2",
                    span { class: "size-2 animate-pulse rounded-full bg-amber-400" }
                    p { class: "text-[10px] font-semibold uppercase tracking-[0.18em] text-zinc-500", "Manual plan" }
                }
                h2 { class: "mb-1 text-base font-semibold text-zinc-100", "The edit needs a plan" }
                p { class: "mb-5 text-[10px] leading-4 text-zinc-600",
                    "The LLM is unavailable, so describe the edit yourself: what region of the image to change, and what it should look like. \"{description}\""
                }
                div { class: "space-y-3",
                    div {
                        p { class: "mb-1 text-[9px] font-semibold uppercase tracking-[0.14em] text-zinc-600", "Mask region" }
                        input {
                            class: "h-9 w-full rounded-lg bg-white/[0.045] px-3 text-[11px] text-zinc-200 outline-none ring-1 ring-inset ring-white/[0.06] transition placeholder:text-zinc-700 focus:ring-violet-400/40",
                            placeholder: "e.g. her hair",
                            value: "{mask_prompt}",
                            oninput: move |event| mask_prompt.set(event.value()),
                        }
                    }
                    div {
                        p { class: "mb-1 text-[9px] font-semibold uppercase tracking-[0.14em] text-zinc-600", "Inpaint look" }
                        input {
                            class: "h-9 w-full rounded-lg bg-white/[0.045] px-3 text-[11px] text-zinc-200 outline-none ring-1 ring-inset ring-white/[0.06] transition placeholder:text-zinc-700 focus:ring-violet-400/40",
                            placeholder: "e.g. a bob cut",
                            value: "{inpaint_prompt}",
                            oninput: move |event| inpaint_prompt.set(event.value()),
                        }
                    }
                }
                div { class: "mt-6 flex justify-end gap-2",
                    button {
                        class: "flex h-8 items-center rounded-lg bg-white/[0.05] px-3.5 text-[10px] font-medium text-zinc-300 ring-1 ring-inset ring-white/[0.07] transition hover:bg-white/[0.09] hover:text-white",
                        onclick: move |_| on_decide.call(Decision::Reject),
                        "Cancel"
                    }
                    button {
                        class: "flex h-8 items-center rounded-lg bg-zinc-100 px-3.5 text-[10px] font-semibold text-zinc-950 transition hover:bg-white disabled:cursor-not-allowed disabled:opacity-35",
                        disabled: !ready,
                        title: "Segment the mask region, then inpaint",
                        onclick: move |_| {
                            let plan = PlannedEdit {
                                mask_prompt: mask_prompt().trim().to_owned(),
                                inpaint_prompt: inpaint_prompt().trim().to_owned(),
                            };
                            on_decide.call(Decision::ProvidePlan(plan));
                        },
                        "Plan the edit"
                    }
                }
            }
        }
    }
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
        div { class: "fixed inset-0 z-50 grid place-items-center bg-black/60 p-6 backdrop-blur-sm",
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
    origin: svs_core::registry::RevisionOrigin,
    /// The `/describe` that would make this experiment canonical.
    promote: String,
}

/// "Revision 3 · krea-2-turbo-q2 · Ready" — the 1-based index plus model
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
    mut composer: Signal<String>,
    activity: Signal<OpActivity>,
    disabled: bool,
    on_rerun: EventHandler<Operation>,
    /// Runs a command exactly as the command bar would.
    onrun: EventHandler<String>,
) -> Element {
    let count = app_state.read().registry.artifacts.len();
    let rail = artifact.as_ref().map(|artifact| {
        let registry = app_state.read().registry.clone();
        let reference = registry.ref_of(artifact.id);
        let bound_loras: Vec<svs_core::registry::LoraEntry> = registry
            .loras_bound_to(artifact.id)
            .into_iter()
            .cloned()
            .collect();
        rsx! {
            Inspector {
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
                        composer,
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
        {rail}
        }
    }
}

#[component]
fn ArtifactDetail(
    app_state: Signal<AppState>,
    artifact: Artifact,
    registry: svs_core::registry::ArtifactRegistry,
    mut composer: Signal<String>,
    activity: Signal<OpActivity>,
    disabled: bool,
    onrun: EventHandler<String>,
    on_rerun: EventHandler<Operation>,
) -> Element {
    let mut show_history = use_context::<HistoryOpen>().0;
    let mut show_variants = use_context::<VariantsOpen>().0;
    let image = app_state.read().display_image(&artifact).map(str::to_owned);
    let is_visual = is_visual(&artifact);
    // The active canvas tool. Pointer is the resting state, so opening an
    // artifact never lands you in a mode you did not ask for.
    let tool = use_context::<ActiveTool>().0;
    let mut strokes = use_context::<PaintedStrokes>().0;
    let mut painting = use_signal(|| false);
    // In pane pixels, so the brush stays the size it looks.
    let brush_radius = use_signal(|| 24.0f64);
    // The render pane's size, measured once it exists. Until then there is
    // no mapping from pointer to pixel, so painting is inert rather than
    // wrong.
    let mut pane = use_signal(|| None::<(f64, f64)>);
    // The image's own pixel dimensions, which the mapping needs and the
    // artifact's default size only guesses at.
    let image_for_size: Option<String> = image.clone();
    let image_size = use_memo(move || {
        image_for_size
            .as_deref()
            .map(std::path::Path::new)
            .and_then(|path| svs_core::registry::image_ops::dimensions(path).ok())
    })();
    // The mask file is written by whatever changed the strokes — the
    // pointer handler here, or the driver's paint step — never by a
    // reactive echo of them. An effect would re-fire on any re-render and
    // rewrite the file against whatever `image_size` had resolved to by
    // then, which is how a plan ended up editing through a mask sized for
    // the artifact it had open a moment earlier.

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
    let log_entries: Vec<svs_core::registry::ops::OperationLogEntry> =
        registry.log_for(artifact.id).cloned().collect();
    // Context actions prefill the composer; the prefixes are computed once
    // so the closures capture strings, not the registry.
    let ref_label = registry.ref_of(artifact.id);
    let variant_prefix = format!("/variant {ref_label} ");
    let regenerate_prefix = format!("/regenerate {ref_label} ");
    let modify_prefix = format!("/modify {ref_label} ");
    let variant_prefix_strip = variant_prefix.clone();
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
    // The masks of the revision currently on screen (the active one).
    let active_masks: Vec<StoredMask> = artifact
        .active_revision_id
        .and_then(|id| artifact.revisions.iter().find(|revision| revision.id == id))
        .map(|revision| revision.masks.clone())
        .unwrap_or_default();

    rsx! {
        // Three regions with their own space, so the page never scrolls:
        // the render takes what is left, the filmstrip is a fixed row, and
        // the history scrolls inside itself. Scrolling *past* the picture to
        // reach the revisions was the thing that made this feel like a
        // document rather than a workspace.
        div { class: "relative flex h-full min-h-0 flex-col",

            // Floats over the render: no background, no border, so the
            // picture reads as the surface and this reads as chrome on it.
            div { class: "pointer-events-none absolute inset-x-0 top-0 z-20 flex items-center gap-2 px-4 py-2.5",
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
                        IconAction {
                            icon: IconName::Add,
                            title: shortcuts::tooltip("New variant", "new-variant"),
                            disabled,
                            onclick: move |_| composer.set(variant_prefix.clone()),
                        }
                        IconAction {
                            icon: IconName::Refresh,
                            title: shortcuts::tooltip("Regenerate", "regenerate"),
                            disabled,
                            onclick: move |_| composer.set(regenerate_prefix.clone()),
                        }
                        if image.is_some() {
                            IconAction {
                                icon: IconName::Generate,
                                title: shortcuts::tooltip("Modify", "modify"),
                                disabled,
                                onclick: move |_| composer.set(modify_prefix.clone()),
                            }
                        }
                        span { class: "mx-1 h-4 w-px bg-white/[0.08]" }
                        IconAction {
                            icon: IconName::Layers,
                            title: shortcuts::tooltip(
                                if show_variants() { "Hide variants" } else { "Show variants" },
                                "toggle-variants",
                            ),
                            active: show_variants(),
                            onclick: move |_| { let now = show_variants(); show_variants.set(!now); },
                        }
                    }
                    IconAction {
                        icon: IconName::List,
                        title: shortcuts::tooltip(
                            if show_history() { "Hide history" } else { "Show history" },
                            "toggle-history",
                        ),
                        active: show_history(),
                        onclick: move |_| { let now = show_history(); show_history.set(!now); },
                    }
                }
            }

            // The render takes whatever the other two regions leave.
            div { class: "relative min-h-0 flex-1 overflow-hidden bg-black/25",
                if is_visual && image.is_some() {
                    ToolRail {
                        tool,
                        disabled,
                        brush_radius,
                        painted: !strokes.read().is_empty(),
                        on_clear: move |_| {
                            strokes.write().clear();
                            write_painted_mask(app_state, &[], None);
                        },
                    }
                }
                match image {
                    Some(path) => rsx! {
                        if let Some(url) = generated_asset_url(&path) {
                            // The picture and the painting surface are the
                            // same box: `Fitted` maps pointer positions in it
                            // to image pixels, so both must measure the same
                            // element.
                            div {
                                class: if tool().paints() { "absolute inset-3 cursor-crosshair" } else { "absolute inset-3" },
                                onmounted: move |event| async move {
                                    if let Ok(rect) = event.get_client_rect().await {
                                        pane.set(Some((rect.size.width, rect.size.height)));
                                    }
                                },
                                onmousedown: move |event| {
                                    if tool().paints() {
                                        painting.set(true);
                                        let at = event.element_coordinates();
                                        paint_at((at.x, at.y), pane(), image_size, brush_radius, tool(), strokes, app_state);
                                    }
                                },
                                onmousemove: move |event| {
                                    if painting() && tool().paints() {
                                        let at = event.element_coordinates();
                                        paint_at((at.x, at.y), pane(), image_size, brush_radius, tool(), strokes, app_state);
                                    }
                                },
                                onmouseup: move |_| painting.set(false),
                                onmouseleave: move |_| painting.set(false),
                                img { class: "pointer-events-none h-full w-full object-contain", src: "{url}", alt: artifact.name }
                                if !strokes.read().is_empty() {
                                    MaskOverlay { strokes, pane: pane(), image_size }
                                }
                            }
                        }
                    },
                    None => rsx! {
                        div { class: "grid h-full place-items-center",
                            EmptyVisual {
                                icon: IconName::Generate,
                                title: if is_visual { "No image yet" } else { "Not a visual artifact" },
                                description: if is_visual { "The first image is generated with the create — this one failed or predates it. Run /regenerate to retry (Krea must be provisioned)." } else { "This artifact carries text only — its images come from the layers that use it." }
                            }
                        }
                    },
                }
            }

            if is_visual && show_variants() {
                // Floats over the render, pinned left. Always the base plus
                // its variants, whichever member is open.
                div { class: "pointer-events-none absolute bottom-3 left-0 z-20 flex max-w-[60%] items-end gap-2 overflow-x-auto px-4",
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
                        onclick: move |_| composer.set(variant_prefix_strip.clone()),
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
            div { class: "min-h-0 max-h-[38%] shrink-0 overflow-y-auto border-t border-white/[0.05] px-5 pb-24 pt-3",
            if !active_masks.is_empty() {
                Section { title: format!("Masks ({})", active_masks.len()),
                    p { class: "mb-3 -mt-2 text-[9px] leading-4 text-zinc-700",
                        "The mask that confined the edit on the active revision — follow-up modifications can reuse it."
                    }
                    div { class: "grid grid-cols-[repeat(auto-fill,minmax(140px,1fr))] gap-3",
                        for mask in &active_masks {
                            div { class: "min-w-0",
                                div { class: "relative aspect-square overflow-hidden rounded-xl bg-white/[0.04] ring-1 ring-inset ring-white/[0.07]",
                                    if let Some(url) = mask_asset_url(&mask.asset_path) {
                                        img { class: "h-full w-full object-cover", src: "{url}", alt: "mask" }
                                    } else {
                                        div { class: "absolute inset-0 grid place-items-center text-zinc-700", Icon { name: IconName::Generate, class: "size-4" } }
                                    }
                                }
                                p { class: "mt-1.5 truncate text-[10px] text-zinc-400",
                                    if let Some(prompt) = &mask.prompt { "{prompt}" } else { "mask" }
                                }
                                p { class: "truncate text-[9px] text-zinc-700",
                                    if let Some(score) = mask.score {
                                        "auto · score {score:.3}"
                                    } else {
                                        "auto"
                                    }
                                }
                            }
                        }
                    }
                }
            }

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
                                if revision.origin == svs_core::registry::RevisionOrigin::Experiment {
                                    span { class: "shrink-0 rounded bg-amber-400/15 px-1.5 py-0.5 text-[9px] font-bold uppercase tracking-[0.05em] text-amber-300",
                                        "experiment"
                                    }
                                    button {
                                        class: "shrink-0 rounded-md bg-amber-400/15 px-2 py-1 text-[10px] font-semibold text-amber-300 opacity-0 transition hover:bg-amber-400/25 group-hover:opacity-100 disabled:opacity-30",
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
                    p { class: "mb-2 -mt-2 text-[9px] leading-4 text-zinc-700",
                        "Re-run an applied operation with the refresh icon — a fresh seed, same prompt (regenerate/modify/draft only)."
                    }
                    div { class: "space-y-0.5",
                        for entry in log_entries.iter().rev() {
                            div { class: "flex items-center gap-3 rounded-lg px-2 py-1.5",
                                span { class: "size-1.5 shrink-0 rounded-full {log_status_class(entry.status)}" }
                                p { class: "min-w-0 flex-1 truncate font-mono text-[10px] text-zinc-500", "{entry.op.summary()}" }
                                span { class: "shrink-0 text-[9px] uppercase tracking-wider text-zinc-700", "{log_status_label(entry.status)}" }
                                RerunButton {
                                    operation: entry.op.clone(),
                                    applied: entry.status == svs_core::registry::ops::OpStatus::Applied,
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

/// A square icon button. `title` carries the tooltip, which the callers
/// build from [`shortcuts::tooltip`] so a key is never hardcoded twice.
#[component]
fn IconAction(
    icon: IconName,
    title: String,
    #[props(default = false)] disabled: bool,
    #[props(default = false)] active: bool,
    onclick: EventHandler<()>,
) -> Element {
    rsx! {
        button {
            class: if active { "grid size-7 place-items-center rounded-lg bg-violet-400/15 text-violet-300 transition" } else { "grid size-7 place-items-center rounded-lg text-zinc-500 transition hover:bg-white/[0.07] hover:text-zinc-100 disabled:opacity-30" },
            title,
            disabled,
            onclick: move |_| onclick.call(()),
            Icon { name: icon, class: "size-3.5" }
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

fn log_status_class(status: svs_core::registry::ops::OpStatus) -> &'static str {
    match status {
        svs_core::registry::ops::OpStatus::Applied => "bg-emerald-400",
        svs_core::registry::ops::OpStatus::Proposed => "bg-amber-400",
        svs_core::registry::ops::OpStatus::Rejected
        | svs_core::registry::ops::OpStatus::Reverted => "bg-rose-400",
    }
}

fn log_status_label(status: svs_core::registry::ops::OpStatus) -> &'static str {
    match status {
        svs_core::registry::ops::OpStatus::Applied => "applied",
        svs_core::registry::ops::OpStatus::Proposed => "proposed",
        svs_core::registry::ops::OpStatus::Rejected => "rejected",
        svs_core::registry::ops::OpStatus::Reverted => "reverted",
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
    registry: svs_core::registry::ArtifactRegistry,
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
    activity: Signal<OpActivity>,
    disabled: bool,
    target_hint: Option<String>,
    /// The artifacts the bar offers as `c:` targets, and the set it treats
    /// as resolving.
    ref_candidates: Vec<RefCandidate>,
    onsubmit: EventHandler<()>,
) -> Element {
    let empty = value.read().trim().is_empty();
    let busy = activity.read().is_busy();
    let summary = match &*activity.read() {
        OpActivity::Running { summary } => Some(summary.clone()),
        _ => None,
    };
    rsx! {
        footer { class: "pointer-events-none absolute inset-x-0 bottom-0 z-30 px-6 pb-5",
            div { class: "pointer-events-auto mx-auto w-full max-w-2xl",
                // One opaque box: the target chip rides inside it at the top,
                // above the field rather than beside it, so the field still
                // owns its full width when the text wraps.
                div { class: "flex flex-col gap-1 rounded-lg bg-zinc-900 p-2 shadow-[0_18px_60px_rgba(0,0,0,.55)] ring-1 ring-inset ring-white/[0.09] focus-within:ring-violet-400/35",
                    if target_hint.is_some() || summary.is_some() {
                        div { class: "flex items-center gap-2 pl-1.5",
                            if let Some(target) = target_hint {
                                span { class: "rounded bg-violet-400/10 px-1.5 py-0.5 font-mono text-[9px] text-violet-300 ring-1 ring-inset ring-violet-400/20",
                                    title: "Free-form changes apply to this artifact unless you name another",
                                    "→ {target}"
                                }
                            }
                            if let Some(summary) = summary {
                                span { class: "flex items-center gap-1.5 text-[10px] text-zinc-400",
                                    span { class: "size-2.5 animate-spin rounded-full border border-zinc-700 border-t-violet-300" }
                                    span { class: "max-w-56 truncate font-mono", "{summary}" }
                                }
                            }
                        }
                    }
                    div { class: "flex items-end gap-2 pl-1.5",
                        CommandBar {
                            value,
                            refs: ref_candidates,
                            disabled,
                            onsubmit: move |_| onsubmit.call(()),
                        }
                        button {
                            class: "flex h-8 shrink-0 items-center gap-1.5 rounded-md bg-zinc-100 px-3 text-[11px] font-semibold text-zinc-950 transition hover:bg-white disabled:cursor-not-allowed disabled:opacity-35",
                            disabled: disabled || busy || empty,
                            onclick: move |_| onsubmit.call(()),
                            Icon { name: IconName::Arrow, class: "size-3" }
                            "Run"
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use svs_core::registry::{ArtifactKind, CreateArtifact, Ref};

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
            mask_prompt: Some("her hair".into()),
            mask_file: None,
            plan: false,
            inpaint_prompt: Some("a bob cut".into()),
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
            &svs_core::registry::GenerationDefaults {
                sizes: Default::default(),
                steps: 4,
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

/// The canvas tool rail: one icon per tool, floating over the render.
///
/// Tools that are planned but unimplemented are shown disabled rather than
/// hidden — the shape of the toolset is information, and a tool that appears
/// only once it works looks like a tool that was never going to exist.
#[component]
fn ToolRail(
    mut tool: Signal<super::tools::Tool>,
    disabled: bool,
    mut brush_radius: Signal<f64>,
    /// Whether anything is painted, which is what makes clearing meaningful.
    painted: bool,
    on_clear: EventHandler<()>,
) -> Element {
    use super::tools::Tool;
    rsx! {
        div { class: "pointer-events-none absolute left-3 top-3 z-20 flex flex-col gap-1 rounded-xl bg-black/45 p-1 backdrop-blur ring-1 ring-inset ring-white/[0.07]",
            div { class: "pointer-events-auto flex flex-col gap-0.5",
                for item in Tool::ALL {
                    IconAction {
                        key: "{item.action()}",
                        icon: item.icon(),
                        title: if item.available() {
                            shortcuts::tooltip(item.label(), item.action())
                        } else {
                            format!("{} — not yet implemented", item.label())
                        },
                        disabled: disabled || !item.available(),
                        active: tool() == item,
                        onclick: move |_| tool.set(item),
                    }
                }
            }
            // Brush settings and the mask's own controls appear only while a
            // painting tool is held, so the rail stays a rail.
            if tool().paints() {
                div { class: "pointer-events-auto flex flex-col items-center gap-1 border-t border-white/[0.08] pt-1",
                    input {
                        r#type: "range",
                        class: "h-16 w-6 cursor-pointer accent-violet-400",
                        style: "writing-mode: vertical-lr; direction: rtl;",
                        min: "4",
                        max: "96",
                        value: "{brush_radius()}",
                        title: "Brush size",
                        oninput: move |event| {
                            if let Ok(value) = event.value().parse::<f64>() {
                                brush_radius.set(value);
                            }
                        },
                    }
                    IconAction {
                        icon: IconName::Close,
                        title: "Clear the painted mask".to_owned(),
                        disabled: !painted,
                        onclick: move |_| on_clear.call(()),
                    }
                }
            }
        }
    }
}

/// Rasterises the painted strokes and records the file on the app state.
///
/// Both surfaces that can paint call this — the brush's pointer handler and
/// the automation driver's `paint` step — because the mask has to exist by
/// the time a command runs, and a component effect only fires when that
/// component re-renders. Tying the file to a render is how a plan can paint
/// and then find nothing painted.
pub fn write_painted_mask(
    mut app_state: Signal<AppState>,
    strokes: &[svs_core::registry::pipeline::Stroke],
    image_size: Option<(u32, u32)>,
) {
    match (strokes.is_empty(), image_size) {
        (false, Some((width, height))) => {
            let output = std::env::temp_dir().join(format!("svs-painted-{}.png", Uuid::new_v4()));
            match svs_core::registry::image_ops::strokes_to_mask(width, height, strokes, &output) {
                Ok(()) => app_state.write().painted_mask = Some(output.display().to_string()),
                Err(error) => tracing::warn!(%error, "could not write the painted mask"),
            }
        }
        _ => app_state.write().painted_mask = None,
    }
}

/// Draws the painted mask over the picture, in the picture's own box.
#[component]
fn MaskOverlay(
    strokes: Signal<Vec<svs_core::registry::pipeline::Stroke>>,
    pane: Option<(f64, f64)>,
    image_size: Option<(u32, u32)>,
) -> Element {
    let (Some(pane), Some(image_size)) = (pane, image_size) else {
        return rsx! {};
    };
    let Some(fitted) = super::tools::Fitted::contain(pane, image_size) else {
        return rsx! {};
    };
    // The overlay is drawn in image coordinates and scaled by the same fit
    // the pointer mapping uses, so what is drawn and what is masked are the
    // same shape by construction rather than by two agreeing calculations.
    let dabs: Vec<(f64, f64, f64, f64)> = strokes
        .read()
        .iter()
        .map(|stroke| {
            (
                stroke.center_x,
                stroke.center_y,
                stroke.radius_x,
                stroke.radius_y,
            )
        })
        .collect();
    rsx! {
        svg {
            class: "pointer-events-none absolute",
            style: "left: {fitted.left}px; top: {fitted.top}px; width: {fitted.width}px; height: {fitted.height}px;",
            view_box: "0 0 {image_size.0} {image_size.1}",
            preserve_aspect_ratio: "none",
            for (index, (x, y, rx, ry)) in dabs.into_iter().enumerate() {
                ellipse {
                    key: "{index}",
                    cx: "{x}",
                    cy: "{y}",
                    rx: "{rx}",
                    ry: "{ry}",
                    fill: "rgb(167 139 250)",
                    fill_opacity: "0.45",
                }
            }
        }
    }
}

/// Adds one brush dab at a pointer position, or removes what it touches.
///
/// Inert until the pane has been measured: without a fit there is no
/// mapping from pointer to pixel, and a guessed one paints somewhere other
/// than where the cursor is.
fn paint_at(
    at: (f64, f64),
    pane: Option<(f64, f64)>,
    image_size: Option<(u32, u32)>,
    brush_radius: Signal<f64>,
    tool: super::tools::Tool,
    mut strokes: Signal<Vec<svs_core::registry::pipeline::Stroke>>,
    app_state: Signal<AppState>,
) {
    let (Some(pane), Some(image_size)) = (pane, image_size) else {
        return;
    };
    let Some(fitted) = super::tools::Fitted::contain(pane, image_size) else {
        return;
    };
    let Some((x, y)) = fitted.to_image(at.0, at.1, image_size) else {
        return; // in the letterbox, not on the picture
    };
    let radius = fitted.radius_to_image(brush_radius(), image_size);
    match tool {
        super::tools::Tool::Eraser => {
            // Erasing drops whole dabs rather than subtracting shapes: the
            // mask is a list of strokes, and a stroke is the unit a person
            // painted.
            strokes.write().retain(|stroke| {
                let dx = stroke.center_x - x;
                let dy = stroke.center_y - y;
                (dx * dx + dy * dy).sqrt() > radius
            });
        }
        _ => strokes.write().push(svs_core::registry::pipeline::Stroke {
            center_x: x,
            center_y: y,
            radius_x: radius,
            radius_y: radius,
        }),
    }
    let painted = strokes.read().clone();
    write_painted_mask(app_state, &painted, Some(image_size));
}
