//! The canvas — the artifact workspace (roadmap §12 item 4).
//!
//! Layout: an artifact sidebar grouped by kind, a detail pane for the
//! selection (image, description, variants, revisions, children, operation
//! log), and a slash-command composer at the bottom (`/create`, `/variant`,
//! `/regenerate`, `/modify` — the same typed vocabulary as the `svs` CLI).
//! Context actions (New variant, Regenerate, Modify) prefill the composer so
//! there is exactly one input path.

use dioxus::prelude::*;
use uuid::Uuid;

use crate::{
    registry::{
        backend::CreativeBackend,
        ops::{self, ExecuteOptions, OpOrigin, Operation},
        pipeline::{ApprovalPolicy, RunOptions},
        slash::{self, is_slash_command},
        Artifact, ArtifactKind, ArtifactRevision, RevisionStatus,
    },
    state::AppState,
};

use super::{
    components::EmptyVisual,
    generated_asset_url,
    icons::{Icon, IconName},
};

/// The kinds that carry images and take variants.
const VISUAL_KINDS: [ArtifactKind; 3] = [
    ArtifactKind::Character,
    ArtifactKind::Environment,
    ArtifactKind::Object,
];

/// The sidebar group order (visual kinds first — the creation loop).
const GROUP_ORDER: [ArtifactKind; 6] = [
    ArtifactKind::Character,
    ArtifactKind::Environment,
    ArtifactKind::Object,
    ArtifactKind::Scene,
    ArtifactKind::Story,
    ArtifactKind::Beat,
];

#[derive(Clone, Debug, Default, PartialEq)]
enum CanvasActivity {
    #[default]
    Idle,
    Running {
        summary: String,
    },
    Failed(String),
}

impl CanvasActivity {
    fn is_busy(&self) -> bool {
        matches!(self, Self::Running { .. })
    }
}

#[component]
pub fn Canvas(app_state: Signal<AppState>, backend: CreativeBackend) -> Element {
    let mut composer = use_signal(String::new);
    let filter = use_signal(String::new);
    let mut activity = use_signal(CanvasActivity::default);
    let selected = app_state.read().selected_artifact().cloned();
    let busy = activity.read().is_busy();

    let backend_for_run = backend.clone();
    let submit = move |_| {
        if busy {
            return;
        }
        let input = composer().trim().to_owned();
        if input.is_empty() {
            return;
        }
        let operation = match parse_composer(&input, &app_state.read().registry) {
            Ok(operation) => operation,
            Err(message) => {
                activity.set(CanvasActivity::Failed(message));
                return;
            }
        };
        composer.set(String::new());
        run_operation(app_state, backend_for_run.clone(), operation, activity);
    };

    rsx! {
        section { class: "grid h-full min-h-0 grid-cols-[248px_minmax(0,1fr)] grid-rows-[minmax(0,1fr)_auto]",
            Sidebar {
                app_state,
                filter,
                composer,
                disabled: busy,
            }
            Detail {
                app_state,
                artifact: selected,
                composer,
                activity,
                disabled: busy,
            }
            Composer {
                value: composer,
                activity,
                disabled: busy,
                onsubmit: submit,
            }
        }
    }
}

/// Parses composer text into an operation; non-slash input is rejected with
/// a hint rather than silently ignored.
fn parse_composer(
    input: &str,
    registry: &crate::registry::ArtifactRegistry,
) -> Result<Operation, String> {
    if !is_slash_command(input) {
        return Err(
            "Commands start with / — e.g. /create character <description>, /variant c:<ref> <description>"
                .to_owned(),
        );
    }
    let operation = slash::parse_slash(input).map_err(|error| error.to_string())?;
    // Static validation against the registry (ref resolution, kinds, active
    // images) so errors surface before any backend work starts.
    ops::compile(registry, &operation).map_err(|error| error.to_string())?;
    Ok(operation)
}

/// Applies one operation: snapshot for undo, then execute (model-only ops
/// apply instantly; visual ops run their pipeline against the backend in a
/// spawned task). The registry is written back on success **and** on failure
/// (a failed revision is recorded; intermediates are kept).
fn run_operation(
    mut app_state: Signal<AppState>,
    backend: CreativeBackend,
    operation: Operation,
    mut activity: Signal<CanvasActivity>,
) {
    let summary = operation.summary();
    app_state.write().snapshot_for_undo();
    activity.set(CanvasActivity::Running {
        summary: summary.clone(),
    });

    spawn(async move {
        let mut registry = app_state.read().registry.clone();
        let options = ExecuteOptions {
            backend: Some(&backend),
            run: RunOptions {
                work_dir: std::env::temp_dir().join(format!("svs-canvas-{}", Uuid::new_v4())),
                approvals: ApprovalPolicy::auto(),
                // A fresh seed per visual op so regenerate is never an
                // exact replay; pass --seed on the CLI for golden runs.
                seed: Some(random_seed()),
            },
            generation: Default::default(),
            manual_text: None,
            origin: OpOrigin::User,
        };
        let result = ops::execute(&mut registry, &operation, &options).await;

        let mut state = app_state.write();
        state.registry = registry;
        state.has_unsaved_changes = true;
        match result {
            Ok(outcome) => {
                if let Some(id) = outcome.artifact_id {
                    state.selected_artifact_id = Some(id);
                }
                let rejected = outcome.status == crate::registry::ops::OpStatus::Rejected;
                drop(state);
                if rejected {
                    activity.set(CanvasActivity::Failed(
                        "operation rejected at a checkpoint".to_owned(),
                    ));
                } else {
                    activity.set(CanvasActivity::Idle);
                }
            }
            Err(error) => {
                drop(state);
                activity.set(CanvasActivity::Failed(error.to_string()));
            }
        }
    });
}

fn random_seed() -> u64 {
    let id = Uuid::new_v4();
    u64::from_be_bytes(id.as_bytes()[..8].try_into().expect("eight bytes"))
}

fn is_visual(artifact: &Artifact) -> bool {
    VISUAL_KINDS.contains(&artifact.kind)
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

// ---------------------------------------------------------------- sidebar

#[component]
fn Sidebar(
    app_state: Signal<AppState>,
    filter: Signal<String>,
    mut composer: Signal<String>,
    disabled: bool,
) -> Element {
    let artifacts = app_state.read().registry.artifacts.clone();
    let selected_id = app_state.read().selected_artifact_id;
    let query = filter.read().trim().to_ascii_lowercase();
    let registry = app_state.read().registry.clone();

    let groups = GROUP_ORDER.iter().map(|kind| {
        let mut group_artifacts: Vec<Artifact> = artifacts
            .iter()
            .filter(|artifact| artifact.kind == *kind)
            .cloned()
            .collect();
        if !query.is_empty() {
            group_artifacts.retain(|artifact| {
                artifact.name.to_ascii_lowercase().contains(&query)
                    || artifact.description.to_ascii_lowercase().contains(&query)
            });
        }
        // Bases first, then their variants, alphabetically within each.
        group_artifacts.sort_by(|a, b| {
            let a_key = (a.variant_of.is_some(), a.name.to_ascii_lowercase());
            let b_key = (b.variant_of.is_some(), b.name.to_ascii_lowercase());
            a_key.cmp(&b_key)
        });

        let kind = *kind;
        let mut group_composer = composer;
        rsx! {
            KindGroup {
                kind,
                artifacts: group_artifacts,
                selected_id,
                registry: registry.clone(),
                on_select: move |id| { app_state.write().select_artifact(id); },
                on_create: move |_event: MouseEvent| {
                    group_composer.set(format!("/create {kind} "));
                },
                disabled,
            }
        }
    });

    rsx! {
        aside { class: "min-h-0 overflow-y-auto border-r border-white/[0.055] bg-zinc-950/35",
            div { class: "sticky top-0 z-10 border-b border-white/[0.05] bg-zinc-950/80 px-4 py-3 backdrop-blur-xl",
                p { class: "mb-2.5 text-[10px] font-semibold uppercase tracking-[0.18em] text-zinc-600", "Artifacts" }
                input {
                    class: "h-8 w-full rounded-lg bg-white/[0.045] px-3 text-[11px] text-zinc-200 outline-none ring-1 ring-inset ring-white/[0.06] transition placeholder:text-zinc-700 focus:ring-violet-400/40",
                    placeholder: "Filter…",
                    value: "{filter}",
                    oninput: move |event| filter.set(event.value()),
                }
            }
            div { class: "space-y-6 px-3 py-4",
                {groups}
                if app_state.read().registry.artifacts.is_empty() {
                    div { class: "px-2 pt-2 text-[10px] leading-4 text-zinc-700",
                        "No artifacts yet. Create one below with /create character, /create environment, or /create object."
                    }
                }
            }
        }
    }
}

#[component]
fn KindGroup(
    kind: ArtifactKind,
    artifacts: Vec<Artifact>,
    selected_id: Option<Uuid>,
    registry: crate::registry::ArtifactRegistry,
    on_select: EventHandler<Uuid>,
    on_create: EventHandler<MouseEvent>,
    disabled: bool,
) -> Element {
    if artifacts.is_empty() {
        return rsx! { Fragment {} };
    }
    rsx! {
        div {
            div { class: "mb-1.5 flex items-center justify-between px-2",
                p { class: "text-[9px] font-semibold uppercase tracking-[0.16em] text-zinc-600", "{kind}s" }
                button {
                    class: "grid size-5 place-items-center rounded-md text-zinc-700 transition hover:bg-white/[0.06] hover:text-zinc-300 disabled:opacity-35",
                    aria_label: "Create {kind}",
                    title: "/create {kind}",
                    disabled,
                    onclick: on_create,
                    Icon { name: IconName::Add, class: "size-3" }
                }
            }
            div { class: "space-y-0.5",
                for artifact in artifacts {
                    ArtifactRow {
                        artifact: artifact.clone(),
                        registry: registry.clone(),
                        selected: selected_id == Some(artifact.id),
                        onclick: {
                            let id = artifact.id;
                            move |_| on_select.call(id)
                        },
                    }
                }
            }
        }
    }
}

#[component]
fn ArtifactRow(
    artifact: Artifact,
    registry: crate::registry::ArtifactRegistry,
    selected: bool,
    onclick: EventHandler<MouseEvent>,
) -> Element {
    let class = if selected {
        "flex min-w-0 items-center gap-2 rounded-lg bg-white/[0.07] px-2.5 py-2 text-left ring-1 ring-inset ring-white/[0.09]"
    } else {
        "flex min-w-0 items-center gap-2 rounded-lg px-2.5 py-2 text-left transition hover:bg-white/[0.04]"
    };
    let title = if artifact.name.trim().is_empty() {
        registry.short_id(artifact.id)
    } else {
        registry.ref_of(artifact.id)
    };
    rsx! {
        button { class, onclick,
            if let Some(image) = registry.latest_image(artifact.id).and_then(|revision| revision.asset_path.as_deref()) {
                if let Some(url) = generated_asset_url(image) {
                    img { class: "size-7 shrink-0 rounded-md object-cover ring-1 ring-inset ring-white/[0.08]", src: "{url}", alt: "{artifact.name}" }
                }
            } else {
                div { class: "grid size-7 shrink-0 place-items-center rounded-md bg-white/[0.045] text-zinc-700 ring-1 ring-inset ring-white/[0.06]",
                    Icon { name: IconName::Layers, class: "size-3.5" }
                }
            }
            div { class: "min-w-0 flex-1",
                p { class: "truncate text-[11px] font-medium text-zinc-300", if artifact.name.trim().is_empty() { "Unnamed" } else { "{artifact.name}" } }
                p { class: "truncate text-[9px] text-zinc-700", "{title}" }
            }
            if artifact.variant_of.is_some() {
                span { class: "shrink-0 rounded bg-violet-400/10 px-1 py-0.5 text-[8px] font-medium text-violet-300", if let Some(axis) = artifact.variant_axis { "{axis}" } else { "variant" } }
            }
        }
    }
}

// ---------------------------------------------------------------- detail

#[component]
fn Detail(
    app_state: Signal<AppState>,
    artifact: Option<Artifact>,
    mut composer: Signal<String>,
    activity: Signal<CanvasActivity>,
    disabled: bool,
) -> Element {
    let count = app_state.read().registry.artifacts.len();
    rsx! {
        div { class: "min-h-0 overflow-y-auto",
            match artifact {
                Some(artifact) => rsx! {
                    ArtifactDetail {
                        app_state,
                        artifact: artifact.clone(),
                        registry: app_state.read().registry.clone(),
                        composer,
                        activity,
                        disabled,
                    }
                },
                None => rsx! {
                    div { class: "grid h-full min-h-[420px] place-items-center px-10",
                        EmptyVisual {
                            icon: IconName::Layers,
                            title: if count == 0 { "Start with an artifact" } else { "Select an artifact" },
                            description: if count == 0 { "Create your first character, environment, or object with the prompt bar below." } else { "Pick one from the sidebar, or run /create for a new one." }
                        }
                    }
                },
            }
        }
    }
}

#[component]
fn ArtifactDetail(
    app_state: Signal<AppState>,
    artifact: Artifact,
    registry: crate::registry::ArtifactRegistry,
    mut composer: Signal<String>,
    activity: Signal<CanvasActivity>,
    disabled: bool,
) -> Element {
    let image = app_state.read().display_image(&artifact).map(str::to_owned);
    let is_visual = is_visual(&artifact);
    // The size this artifact's images use: the explicit `--size` from
    // creation, or the kind default.
    let size = artifact
        .default_size
        .or_else(|| artifact.kind.default_size());
    let variants: Vec<Artifact> = registry
        .artifacts
        .iter()
        .filter(|candidate| candidate.variant_of == Some(artifact.id))
        .cloned()
        .collect();
    let children: Vec<Artifact> = registry
        .artifacts
        .iter()
        .filter(|candidate| candidate.parent_id == Some(artifact.id))
        .cloned()
        .collect();
    let log_entries: Vec<crate::registry::ops::OperationLogEntry> =
        registry.log_for(artifact.id).cloned().collect();
    // Context actions prefill the composer; the prefixes are computed once
    // so the closures capture strings, not the registry.
    let ref_label = registry.ref_of(artifact.id);
    let variant_prefix = format!("/variant {ref_label} ");
    let regenerate_prefix = format!("/regenerate {ref_label} ");
    let modify_prefix = format!("/modify {ref_label} ");
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
        })
        .collect();

    rsx! {
        div { class: "mx-auto max-w-4xl px-10 py-8",
            // Header: identity + provenance links.
            div { class: "mb-7 flex flex-wrap items-start justify-between gap-4",
                div { class: "min-w-0",
                    div { class: "mb-2 flex flex-wrap items-center gap-2",
                        span { class: "rounded-md bg-white/[0.05] px-2 py-1 text-[9px] font-medium uppercase tracking-[0.14em] text-zinc-400 ring-1 ring-inset ring-white/[0.07]", "{artifact.kind}" }
                        span { class: "rounded-md bg-zinc-900 px-2 py-1 font-mono text-[10px] text-violet-300 ring-1 ring-inset ring-violet-400/20", "{registry.ref_of(artifact.id)}" }
                        if let Some((width, height)) = size {
                            span { class: "rounded-md bg-white/[0.05] px-2 py-1 text-[9px] font-medium text-zinc-500 ring-1 ring-inset ring-white/[0.07]", title: if artifact.default_size.is_some() { "Size set at creation" } else { format!("Default size for {}", artifact.kind) }, "{width}×{height}" }
                        }
                        if let Some(axis) = artifact.variant_axis {
                            span { class: "rounded-md bg-violet-400/10 px-2 py-1 text-[9px] font-medium text-violet-300", "variant · {axis}" }
                        }
                    }
                    h2 { class: "truncate text-xl font-semibold tracking-[-0.02em] text-zinc-100", if artifact.name.trim().is_empty() { "Unnamed" } else { "{artifact.name}" } }
                    p { class: "mt-1.5 max-w-2xl text-xs leading-5 text-zinc-500", "{artifact.description}" }
                    if let Some(base_id) = artifact.variant_of {
                        ProvenanceLink { label: "Base", registry: registry.clone(), target: base_id, on_select: move |id| app_state.write().select_artifact(id) }
                    }
                    if let Some(parent_id) = artifact.parent_id {
                        ProvenanceLink { label: "Parent", registry: registry.clone(), target: parent_id, on_select: move |id| app_state.write().select_artifact(id) }
                    }
                }
                if is_visual {
                    div { class: "flex shrink-0 items-center gap-2",
                        ActionButton {
                            label: "New variant",
                            icon: IconName::Add,
                            disabled,
                            title: "Create a new version (axis: outfit, age, weather, …)",
                            onclick: move |_| composer.set(variant_prefix.clone()),
                        }
                        ActionButton {
                            label: "Regenerate",
                            icon: IconName::Refresh,
                            disabled,
                            title: "New revision of the active image",
                            onclick: move |_| composer.set(regenerate_prefix.clone()),
                        }
                        if image.is_some() {
                            ActionButton {
                                label: "Modify",
                                icon: IconName::Generate,
                                disabled,
                                title: "Mask-guided regional edit (needs --mask and --inpaint, or an LLM)",
                                onclick: move |_| composer.set(modify_prefix.clone()),
                            }
                        }
                    }
                }
            }

            // Visual: the active image, or an empty state.
            div { class: "mb-8 grid place-items-center overflow-hidden rounded-2xl bg-black/25 ring-1 ring-inset ring-white/[0.07]",
                match image {
                    Some(path) => rsx! {
                        if let Some(url) = generated_asset_url(&path) {
                            img { class: "max-h-[460px] w-full object-contain", src: "{url}", alt: artifact.name }
                        }
                    },
                    None => rsx! {
                        div { class: "py-16",
                            EmptyVisual {
                                icon: IconName::Generate,
                                title: if is_visual { "No image yet" } else { "Not a visual artifact" },
                                description: if is_visual { "Run /regenerate to generate the first image (Krea must be provisioned)." } else { "This artifact carries text only — its images come from the layers that use it." }
                            }
                        }
                    },
                }
            }

            if !variants.is_empty() {
                Section { title: format!("Variants ({})", variants.len()),
                    div { class: "grid grid-cols-[repeat(auto-fill,minmax(150px,1fr))] gap-3",
                        for variant in variants {
                            button {
                                class: "group min-w-0 text-left",
                                onclick: {
                                    let id = variant.id;
                                    move |_| app_state.write().select_artifact(id)
                                },
                                div { class: "relative aspect-square overflow-hidden rounded-xl bg-white/[0.04] ring-1 ring-inset ring-white/[0.07]",
                                    if let Some(image) = registry.latest_image(variant.id).and_then(|revision| revision.asset_path.as_deref()) {
                                        if let Some(url) = generated_asset_url(image) { img { class: "h-full w-full object-cover", src: "{url}", alt: "{variant.name}" } }
                                    } else {
                                        div { class: "absolute inset-0 grid place-items-center text-zinc-700", Icon { name: IconName::Layers, class: "size-4" } }
                                    }
                                    if let Some(axis) = variant.variant_axis {
                                        span { class: "absolute bottom-1.5 left-1.5 rounded bg-black/60 px-1.5 py-0.5 text-[8px] font-medium text-violet-200 backdrop-blur", "{axis}" }
                                    }
                                }
                                p { class: "mt-1.5 truncate text-[10px] text-zinc-400 group-hover:text-zinc-200", "{variant.name}" }
                            }
                        }
                    }
                }
            }

            if !artifact.revisions.is_empty() {
                Section { title: format!("Revisions ({})", artifact.revisions.len()),
                    div { class: "space-y-1",
                        for revision in revision_rows {
                            button {
                                class: if artifact.active_revision_id == Some(revision.id) { "flex w-full items-center gap-3 rounded-lg bg-white/[0.06] px-3 py-2 text-left" } else { "flex w-full items-center gap-3 rounded-lg px-3 py-2 text-left transition hover:bg-white/[0.035] disabled:opacity-40" },
                                disabled: !revision.has_asset,
                                title: if revision.has_asset { "Show this revision" } else { "No image for this revision" },
                                onclick: {
                                    let artifact_id = artifact.id;
                                    let revision_id = revision.id;
                                    move |_| { app_state.write().activate_revision(artifact_id, revision_id); }
                                },
                                span { class: "{status_dot_class(revision.status)}" }
                                div { class: "min-w-0 flex-1",
                                    p { class: "truncate text-[11px] text-zinc-300", "{revision.prompt}" }
                                    p { class: "mt-0.5 text-[9px] text-zinc-700", "{revision.meta}" }
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
                            }
                        }
                    }
                }
            }
        }
    }
}

fn log_status_class(status: crate::registry::ops::OpStatus) -> &'static str {
    match status {
        crate::registry::ops::OpStatus::Applied => "bg-emerald-400",
        crate::registry::ops::OpStatus::Proposed => "bg-amber-400",
        crate::registry::ops::OpStatus::Rejected | crate::registry::ops::OpStatus::Reverted => {
            "bg-rose-400"
        }
    }
}

fn log_status_label(status: crate::registry::ops::OpStatus) -> &'static str {
    match status {
        crate::registry::ops::OpStatus::Applied => "applied",
        crate::registry::ops::OpStatus::Proposed => "proposed",
        crate::registry::ops::OpStatus::Rejected => "rejected",
        crate::registry::ops::OpStatus::Reverted => "reverted",
    }
}

#[component]
fn ProvenanceLink(
    label: String,
    registry: crate::registry::ArtifactRegistry,
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
        div { class: "mb-8",
            h3 { class: "mb-3 text-[9px] font-semibold uppercase tracking-[0.16em] text-zinc-600", "{title}" }
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
    mut value: Signal<String>,
    activity: Signal<CanvasActivity>,
    disabled: bool,
    onsubmit: EventHandler<()>,
) -> Element {
    let empty = value.read().trim().is_empty();
    let busy = activity.read().is_busy();
    let summary = match &*activity.read() {
        CanvasActivity::Running { summary } => Some(summary.clone()),
        _ => None,
    };
    rsx! {
        footer { class: "col-span-2 border-t border-white/[0.055] bg-zinc-950/45 px-6 py-4",
            div { class: "mx-auto max-w-3xl",
                if let CanvasActivity::Failed(error) = &*activity.read() {
                    p { class: "mb-2 text-[10px] leading-4 text-red-300/80", "{error}" }
                }
                div { class: "flex items-center gap-3 rounded-[18px] bg-zinc-900/80 p-2 pl-4 shadow-[0_18px_60px_rgba(0,0,0,.3)] ring-1 ring-inset ring-white/[0.09] backdrop-blur-xl focus-within:ring-violet-400/35",
                    if let Some(summary) = summary {
                        div { class: "flex shrink-0 items-center gap-2 text-[10px] text-zinc-400",
                            span { class: "size-3 animate-spin rounded-full border border-zinc-700 border-t-violet-300" }
                            span { class: "max-w-56 truncate font-mono", "{summary}" }
                        }
                    }
                    textarea {
                        class: "block min-h-9 w-full resize-none bg-transparent py-1.5 text-[12px] leading-5 text-zinc-100 outline-none placeholder:text-zinc-700 disabled:opacity-40",
                        placeholder: "/create character <description> [--name <name>] [--size WxH]   ·   /variant c:<ref> <description> [--axis <axis>]   ·   /regenerate c:<ref> [<change>]   ·   /modify c:<ref> <change> --mask <region> --inpaint <look>",
                        value: "{value}",
                        disabled,
                        // WebKit rewrites `--` to an em dash and straight to
                        // curly quotes in editable text; slash syntax needs
                        // the literal characters (the parser also normalizes
                        // smart punctuation as a fallback).
                        autocorrect: "off",
                        spellcheck: "false",
                        oninput: move |event| value.set(event.value()),
                        onkeydown: move |event| {
                            if event.key() == Key::Enter && !event.modifiers().contains(Modifiers::SHIFT) {
                                event.prevent_default();
                                onsubmit.call(());
                            }
                        },
                    }
                    button {
                        class: "flex h-9 shrink-0 items-center gap-2 rounded-xl bg-zinc-100 px-3.5 text-[11px] font-semibold text-zinc-950 transition hover:bg-white disabled:cursor-not-allowed disabled:opacity-35",
                        disabled: disabled || busy || empty,
                        onclick: move |_| onsubmit.call(()),
                        Icon { name: IconName::Arrow, class: "size-3.5" }
                        "Run"
                    }
                }
            }
        }
    }
}
