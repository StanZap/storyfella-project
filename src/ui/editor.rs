use std::path::Path;

use dioxus::prelude::*;

use crate::{
    models::{RevisionStatus, StoryboardFrame},
    runtime::{CreativeRuntime, CreativeRuntimeError},
    state::AppState,
    vision::{ComputeDevice, GenerateRequest},
};

use super::{
    components::EmptyVisual,
    icons::{Icon, IconName},
};

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum StudioView {
    Canvas,
    Storyboard,
    Timeline,
}

#[derive(Clone, Debug, Default, PartialEq)]
enum GenerationActivity {
    #[default]
    Idle,
    Starting {
        frame_id: uuid::Uuid,
    },
    Generating {
        frame_id: uuid::Uuid,
    },
    Completed,
    Failed(String),
}

impl GenerationActivity {
    fn is_busy(&self) -> bool {
        matches!(self, Self::Starting { .. } | Self::Generating { .. })
    }

    fn frame_id(&self) -> Option<uuid::Uuid> {
        match self {
            Self::Starting { frame_id } | Self::Generating { frame_id } => Some(*frame_id),
            _ => None,
        }
    }
}

#[component]
pub fn Studio(app_state: Signal<AppState>, runtime: CreativeRuntime) -> Element {
    let mut view = use_signal(|| StudioView::Canvas);
    let prompt = use_signal(String::new);
    let activity = use_signal(GenerationActivity::default);
    let frame_count = app_state.read().project.storyboard.len();
    let has_selection = app_state.read().selected_frame().is_some();

    rsx! {
        section { class: "grid min-h-0 grid-cols-[minmax(0,1fr)_292px] grid-rows-[minmax(0,1fr)_156px]",
            div { class: "relative min-h-0 overflow-hidden bg-[radial-gradient(circle_at_50%_42%,rgba(67,56,115,.16),transparent_36%)]",
                div { class: "absolute left-1/2 top-4 z-20 flex -translate-x-1/2 items-center gap-1 rounded-xl bg-zinc-950/70 p-1 ring-1 ring-inset ring-white/[0.07] backdrop-blur-xl",
                    ViewButton { label: "Canvas", icon: IconName::Canvas, active: view() == StudioView::Canvas, onclick: move |_| view.set(StudioView::Canvas) }
                    ViewButton { label: "Storyboard", icon: IconName::Grid, active: view() == StudioView::Storyboard, onclick: move |_| view.set(StudioView::Storyboard) }
                    ViewButton { label: "Timeline", icon: IconName::Timeline, active: view() == StudioView::Timeline, onclick: move |_| view.set(StudioView::Timeline) }
                }

                match view() {
                    StudioView::Canvas => rsx! { CanvasView { app_state, runtime: runtime.clone(), prompt, activity } },
                    StudioView::Storyboard => rsx! { StoryboardView { app_state, on_open_canvas: move |_| view.set(StudioView::Canvas) } },
                    StudioView::Timeline => rsx! { TimelineView { app_state } },
                }
            }

            aside { class: "min-h-0 overflow-y-auto border-l border-white/[0.055] bg-zinc-950/35",
                div { class: "flex h-14 items-center justify-between border-b border-white/[0.055] px-5",
                    h2 { class: "text-xs font-medium text-zinc-300", "Properties" }
                    button { class: "text-zinc-600 transition hover:text-zinc-300", aria_label: "More properties", Icon { name: IconName::More, class: "size-4" } }
                }
                if !has_selection {
                    div { class: "grid h-[calc(100%-3.5rem)] min-h-60 place-items-center px-8",
                        EmptyVisual {
                            icon: IconName::Layers,
                            title: if frame_count == 0 { "Start with a scene" } else { "New story beat" },
                            description: if frame_count == 0 { "Describe the first image in the Canvas." } else { "Describe another scene, or select an existing beat below." }
                        }
                    }
                } else {
                    Inspector { app_state, activity }
                }
            }

            StoryStrip {
                app_state,
                on_open_storyboard: move |_| view.set(StudioView::Storyboard),
                on_open_canvas: move |_| view.set(StudioView::Canvas),
            }
        }
    }
}

#[component]
fn ViewButton(
    label: String,
    icon: IconName,
    active: bool,
    onclick: EventHandler<MouseEvent>,
) -> Element {
    let class = if active {
        "flex h-8 items-center gap-2 rounded-lg bg-white/[0.08] px-3 text-[11px] font-medium text-zinc-100 shadow-sm"
    } else {
        "flex h-8 items-center gap-2 rounded-lg px-3 text-[11px] font-medium text-zinc-500 transition hover:text-zinc-200"
    };
    rsx! { button { class, onclick, Icon { name: icon, class: "size-3.5" } "{label}" } }
}

#[component]
fn CanvasView(
    app_state: Signal<AppState>,
    runtime: CreativeRuntime,
    mut prompt: Signal<String>,
    activity: Signal<GenerationActivity>,
) -> Element {
    let selected_frame = app_state.read().selected_frame().cloned();
    let selected_frame_id = selected_frame.as_ref().map(|frame| frame.id);
    let is_busy = activity.read().is_busy();
    let selected_is_busy = activity.read().frame_id() == selected_frame_id;
    let has_image = selected_frame
        .as_ref()
        .and_then(|frame| frame.asset_path.as_ref())
        .is_some();

    let mut submit_new_beat = move || {
        let value = prompt();
        if !value.trim().is_empty() {
            app_state.write().add_storyboard_beat(value);
            prompt.set(String::new());
        }
    };
    let generate_runtime = runtime.clone();
    let mut generate = move || {
        start_generation(app_state, generate_runtime.clone(), activity, prompt());
        prompt.set(String::new());
    };

    rsx! {
        div { class: "flex h-full min-h-[420px] flex-col px-8 pb-5 pt-20",
            if let Some(frame) = selected_frame {
                div { class: "mx-auto flex min-h-0 w-full max-w-4xl flex-1 flex-col items-center justify-center",
                    div { class: "mb-3 flex w-full max-w-3xl items-center justify-between",
                        div { class: "min-w-0",
                            p { class: "text-[10px] font-medium uppercase tracking-[0.14em] text-zinc-600", "Selected beat" }
                            p { class: "mt-1 truncate text-xs text-zinc-400", "{frame.prompt}" }
                        }
                        button {
                            class: "flex h-8 shrink-0 items-center gap-2 rounded-lg px-3 text-[10px] text-zinc-500 transition hover:bg-white/[0.045] hover:text-zinc-200",
                            onclick: move |_| {
                                app_state.write().begin_new_beat();
                                prompt.set(String::new());
                            },
                            Icon { name: IconName::Add, class: "size-3" }
                            "New beat"
                        }
                    }
                    div { class: "relative flex min-h-0 w-full max-w-3xl flex-1 items-center justify-center overflow-hidden rounded-2xl bg-black/30 ring-1 ring-inset ring-white/[0.07]",
                        if let Some(asset_path) = &frame.asset_path {
                            if let Some(url) = generated_asset_url(asset_path) {
                                img { class: "h-full w-full object-contain", src: "{url}", alt: "Generated story frame" }
                            }
                        } else if selected_is_busy {
                            div { class: "flex flex-col items-center text-center",
                                span { class: "mb-4 size-6 animate-spin rounded-full border-2 border-zinc-800 border-t-violet-300" }
                                p { class: "text-xs font-medium text-zinc-300", if matches!(*activity.read(), GenerationActivity::Starting { .. }) { "Starting local models…" } else { "Generating image…" } }
                                p { class: "mt-2 text-[10px] text-zinc-700", "Krea remains loaded for the next request" }
                            }
                        } else {
                            EmptyVisual { icon: IconName::Generate, title: "Ready to generate", description: "Create the first image for this beat, then describe changes below." }
                        }
                        if has_image && selected_is_busy {
                            div { class: "absolute inset-x-0 bottom-0 flex items-center gap-3 bg-black/65 px-4 py-3 backdrop-blur",
                                span { class: "size-3 animate-spin rounded-full border border-zinc-600 border-t-white" }
                                p { class: "text-[10px] text-zinc-300", "Creating a new revision in the background…" }
                            }
                        }
                    }
                }

                Composer {
                    prompt,
                    placeholder: if has_image { "Describe a change: make it wider, warmer, more dramatic…" } else { "Optional: refine the scene before generating…" },
                    action_label: if has_image { "Apply change" } else { "Generate image" },
                    disabled: is_busy,
                    require_prompt: false,
                    onsubmit: move |_| generate(),
                }
                if let GenerationActivity::Failed(error) = &*activity.read() {
                    p { class: "mx-auto mt-2 max-w-2xl text-[10px] leading-4 text-red-300/70", "{error}" }
                }
            } else {
                div { class: "flex flex-1 flex-col items-center justify-center pb-4",
                    div { class: "mb-9 text-center",
                        div { class: "mx-auto mb-5 grid size-12 place-items-center text-violet-300", Icon { name: IconName::Sparkles, class: "size-7" } }
                        h2 { class: "text-[22px] font-medium tracking-[-0.03em] text-zinc-100", "What happens next?" }
                        p { class: "mt-2 text-[13px] text-zinc-600", "Describe one visual beat. You can generate and refine it immediately." }
                    }
                    Composer {
                        prompt,
                        placeholder: "A quiet lighthouse above a silver sea at dusk…",
                        action_label: "Add beat",
                        disabled: false,
                        require_prompt: true,
                        onsubmit: move |_| submit_new_beat(),
                    }
                }
            }
        }
    }
}

#[component]
fn Composer(
    mut prompt: Signal<String>,
    placeholder: String,
    action_label: String,
    disabled: bool,
    require_prompt: bool,
    onsubmit: EventHandler<MouseEvent>,
) -> Element {
    let action_disabled = disabled || (require_prompt && prompt.read().trim().is_empty());
    rsx! {
        div { class: "mx-auto mt-4 w-full max-w-2xl rounded-[20px] bg-zinc-900/80 p-2 shadow-[0_18px_60px_rgba(0,0,0,.3)] ring-1 ring-inset ring-white/[0.09] backdrop-blur-xl focus-within:ring-violet-400/35",
            textarea {
                class: "block min-h-12 w-full resize-none bg-transparent px-3 py-2 text-[12px] leading-5 text-zinc-100 outline-none placeholder:text-zinc-600 disabled:opacity-40",
                placeholder,
                value: "{prompt}",
                disabled,
                oninput: move |event| prompt.set(event.value()),
            }
            div { class: "flex items-center justify-between px-1 pb-1",
                p { class: "pl-2 text-[9px] text-zinc-700", "Krea 2 · local · 16:9" }
                button {
                    class: "flex h-9 items-center gap-2 rounded-xl bg-zinc-100 px-3.5 text-[11px] font-semibold text-zinc-950 transition hover:bg-white disabled:cursor-not-allowed disabled:opacity-35",
                    disabled: action_disabled,
                    onclick: onsubmit,
                    Icon { name: IconName::Generate, class: "size-3.5" }
                    "{action_label}"
                }
            }
        }
    }
}

#[component]
fn StoryboardView(
    app_state: Signal<AppState>,
    on_open_canvas: EventHandler<MouseEvent>,
) -> Element {
    let frames = app_state.read().project.storyboard.clone();
    let selected_id = app_state.read().selected_frame_id;
    rsx! {
        div { class: "h-full overflow-y-auto px-8 pb-8 pt-20",
            div { class: "mx-auto max-w-5xl",
                div { class: "mb-6 flex items-end justify-between",
                    div { h2 { class: "text-lg font-medium tracking-tight text-zinc-100", "Storyboard" } p { class: "mt-1 text-xs text-zinc-600", "Select a beat to generate or refine its image." } }
                    span { class: "text-[11px] text-zinc-600", "{frames.len()} beats" }
                }
                if frames.is_empty() {
                    div { class: "grid min-h-72 place-items-center border-y border-white/[0.05]",
                        EmptyVisual { icon: IconName::Grid, title: "Your story is still open", description: "Return to Canvas and describe the first scene to begin." }
                    }
                } else {
                    div { class: "grid grid-cols-[repeat(auto-fill,minmax(190px,1fr))] gap-x-4 gap-y-7",
                        for (index, frame) in frames.iter().enumerate() {
                            button {
                                class: if selected_id == Some(frame.id) { "group min-w-0 text-left" } else { "group min-w-0 text-left opacity-75 hover:opacity-100" },
                                onclick: {
                                    let frame_id = frame.id;
                                    move |event| { app_state.write().select_frame(frame_id); on_open_canvas.call(event); }
                                },
                                FrameThumbnail { frame: frame.clone(), index }
                                p { class: "mt-2.5 line-clamp-2 text-xs leading-5 text-zinc-400", "{frame.prompt}" }
                            }
                        }
                    }
                }
            }
        }
    }
}

#[component]
fn FrameThumbnail(frame: StoryboardFrame, index: usize) -> Element {
    rsx! {
        div { class: "relative aspect-video overflow-hidden rounded-xl bg-[linear-gradient(145deg,rgba(124,58,237,.13),rgba(24,24,27,.8))] ring-1 ring-inset ring-white/[0.07]",
            if let Some(asset_path) = &frame.asset_path {
                if let Some(url) = generated_asset_url(asset_path) {
                    img { class: "h-full w-full object-cover", src: "{url}", alt: "Generated story frame" }
                }
            } else {
                div { class: "absolute inset-0 grid place-items-center text-zinc-700", Icon { name: IconName::Sparkles, class: "size-5" } }
            }
            span { class: "absolute left-2.5 top-2.5 rounded-md bg-black/55 px-1.5 py-1 text-[9px] font-medium text-zinc-300 backdrop-blur", "{index + 1}" }
            if let Some(revision) = frame.revisions.last() {
                if matches!(revision.status, RevisionStatus::Queued | RevisionStatus::Generating) {
                    span { class: "absolute bottom-2.5 right-2.5 size-4 animate-spin rounded-full border border-zinc-500 border-t-white" }
                }
            }
        }
    }
}

#[component]
fn TimelineView(app_state: Signal<AppState>) -> Element {
    let clips = app_state.read().project.timeline.clips.clone();
    rsx! {
        div { class: "h-full overflow-auto px-8 pb-8 pt-20",
            div { class: "mx-auto min-w-[640px] max-w-5xl",
                div { class: "mb-5 flex items-center justify-between",
                    div { h2 { class: "text-lg font-medium tracking-tight text-zinc-100", "Timeline" } p { class: "mt-1 text-xs text-zinc-600", "Arrange story beats in time." } }
                    button { class: "flex items-center gap-2 rounded-lg bg-white/[0.05] px-3 py-2 text-[11px] text-zinc-400", disabled: true, title: "Preview playback is the next presentation slice", Icon { name: IconName::Play, class: "size-3" } "Preview" }
                }
                div { class: "border-y border-white/[0.055] py-5",
                    div { class: "mb-3 grid grid-cols-8 text-[9px] text-zinc-700",
                        for second in [0, 5, 10, 15, 20, 25, 30, 35] { span { "{second}s" } }
                    }
                    div { class: "relative h-20 bg-[linear-gradient(90deg,rgba(255,255,255,.035)_1px,transparent_1px)] bg-[size:12.5%_100%]",
                        if clips.is_empty() { p { class: "absolute inset-0 grid place-items-center text-xs text-zinc-700", "No beats on the timeline" } }
                        for (index, clip) in clips.iter().enumerate() {
                            div { class: "absolute top-2 flex h-14 items-center overflow-hidden rounded-lg bg-violet-500/15 px-3 text-[10px] text-violet-200 ring-1 ring-inset ring-violet-400/20", style: "left: {clip.start_seconds * 2.5}%; width: {clip.duration_seconds * 2.5}%; min-width: 72px; top: {8 + (index % 2) * 4}px", "{clip.label}" }
                        }
                    }
                }
            }
        }
    }
}

#[component]
fn Inspector(app_state: Signal<AppState>, activity: Signal<GenerationActivity>) -> Element {
    let frame = app_state.read().selected_frame().cloned();
    rsx! {
        if let Some(frame) = frame {
            div { class: "space-y-7 p-5",
                div { p { class: "mb-2 text-[9px] font-semibold uppercase tracking-[0.16em] text-zinc-600", "Prompt" } p { class: "text-xs leading-5 text-zinc-400", "{frame.prompt}" } }
                div {
                    p { class: "mb-3 text-[9px] font-semibold uppercase tracking-[0.16em] text-zinc-600", "Generation" }
                    dl { class: "space-y-3 text-[11px]",
                        div { class: "flex justify-between", dt { class: "text-zinc-600", "Model" } dd { class: "text-zinc-300", "Krea 2 Turbo" } }
                        div { class: "flex justify-between", dt { class: "text-zinc-600", "Revisions" } dd { class: "text-zinc-300", "{frame.revisions.len()}" } }
                        div { class: "flex justify-between", dt { class: "text-zinc-600", "Format" } dd { class: "text-zinc-300", "16:9" } }
                    }
                }
                if !frame.revisions.is_empty() {
                    div {
                        p { class: "mb-3 text-[9px] font-semibold uppercase tracking-[0.16em] text-zinc-600", "Revision history" }
                        div { class: "space-y-1.5",
                            for (index, revision) in frame.revisions.iter().enumerate().rev() {
                                button {
                                    class: if frame.active_revision_id == Some(revision.id) { "flex h-8 w-full items-center justify-between rounded-lg bg-white/[0.06] px-2.5 text-[10px] text-zinc-200" } else { "flex h-8 w-full items-center justify-between rounded-lg px-2.5 text-[10px] text-zinc-600 transition hover:bg-white/[0.035] hover:text-zinc-300 disabled:opacity-40" },
                                    disabled: revision.asset_path.is_none(),
                                    onclick: {
                                        let revision_id = revision.id;
                                        move |_| app_state.write().activate_revision(frame.id, revision_id)
                                    },
                                    span { "Revision {index + 1}" }
                                    span { class: "text-[9px]", "{revision_status_label(revision.status)}" }
                                }
                            }
                        }
                    }
                }
                div {
                    p { class: "mb-3 text-[9px] font-semibold uppercase tracking-[0.16em] text-zinc-600", "Status" }
                    p { class: "text-[11px] text-zinc-400",
                        match &*activity.read() {
                            GenerationActivity::Idle => "Ready".to_owned(),
                            GenerationActivity::Starting { frame_id } if *frame_id == frame.id => "Loading resident models…".to_owned(),
                            GenerationActivity::Generating { frame_id } if *frame_id == frame.id => "Generating revision…".to_owned(),
                            GenerationActivity::Starting { .. } | GenerationActivity::Generating { .. } => "Another beat is generating".to_owned(),
                            GenerationActivity::Completed => "Latest revision complete".to_owned(),
                            GenerationActivity::Failed(_) => "Generation needs attention".to_owned(),
                        }
                    }
                }
            }
        }
    }
}

#[component]
fn StoryStrip(
    app_state: Signal<AppState>,
    on_open_storyboard: EventHandler<MouseEvent>,
    on_open_canvas: EventHandler<MouseEvent>,
) -> Element {
    let frames = app_state.read().project.storyboard.clone();
    let selected_id = app_state.read().selected_frame_id;
    rsx! {
        footer { class: "col-span-2 min-w-0 border-t border-white/[0.055] bg-zinc-950/45",
            div { class: "flex h-9 items-center justify-between border-b border-white/[0.04] px-4",
                button { class: "flex items-center gap-2 text-[10px] font-medium text-zinc-400 hover:text-zinc-100", onclick: on_open_storyboard,
                    Icon { name: IconName::Grid, class: "size-3" } "Story beats" span { class: "text-zinc-700", "{frames.len()}" }
                }
                div { class: "flex items-center gap-3 text-[9px] text-zinc-700", span { "00:00" } span { "—" } span { "00:{frames.len() * 5:02}" } }
            }
            div { class: "flex h-[116px] items-center gap-2 overflow-x-auto px-4",
                for (index, frame) in frames.iter().enumerate() {
                    button {
                        class: if selected_id == Some(frame.id) { "group flex w-36 shrink-0 items-center gap-2 rounded-xl bg-white/[0.045] p-1.5 text-left ring-1 ring-inset ring-white/[0.07]" } else { "group flex w-36 shrink-0 items-center gap-2 rounded-xl p-1.5 text-left transition hover:bg-white/[0.04]" },
                        onclick: {
                            let frame_id = frame.id;
                            move |event| { app_state.write().select_frame(frame_id); on_open_canvas.call(event); }
                        },
                        div { class: "relative grid aspect-video w-20 shrink-0 place-items-center overflow-hidden rounded-lg bg-violet-500/[0.08] text-[9px] text-violet-300 ring-1 ring-inset ring-white/[0.06]",
                            if let Some(path) = &frame.asset_path {
                                if let Some(url) = generated_asset_url(path) { img { class: "h-full w-full object-cover", src: "{url}", alt: "Beat {index + 1}" } }
                            } else { "{index + 1}" }
                        }
                        p { class: "line-clamp-3 text-[9px] leading-4 text-zinc-600 group-hover:text-zinc-400", "{frame.prompt}" }
                    }
                }
                button {
                    class: "grid aspect-video w-20 shrink-0 place-items-center rounded-lg border border-dashed border-white/[0.09] text-zinc-700 transition hover:border-white/[0.16] hover:text-zinc-400",
                    aria_label: "Add story beat",
                    onclick: move |event| { app_state.write().begin_new_beat(); on_open_canvas.call(event); },
                    Icon { name: IconName::Add, class: "size-4" }
                }
            }
        }
    }
}

fn start_generation(
    mut app_state: Signal<AppState>,
    runtime: CreativeRuntime,
    mut activity: Signal<GenerationActivity>,
    follow_up: String,
) {
    if activity.read().is_busy() {
        return;
    }
    let Some(frame) = app_state.read().selected_frame().cloned() else {
        return;
    };
    let generation_prompt = if follow_up.trim().is_empty() {
        frame.prompt.clone()
    } else {
        format!(
            "{}\n\nRequested revision: {}",
            frame.prompt,
            follow_up.trim()
        )
    };
    let reference_image_path = frame.asset_path.clone();
    let Some(revision_id) = app_state
        .write()
        .start_revision(frame.id, generation_prompt.clone())
    else {
        return;
    };
    activity.set(GenerationActivity::Starting { frame_id: frame.id });

    spawn(async move {
        let result = async {
            runtime.ensure_ready().await?;
            app_state.write().update_revision(
                frame.id,
                revision_id,
                RevisionStatus::Generating,
                None,
                None,
            );
            activity.set(GenerationActivity::Generating { frame_id: frame.id });
            generate_image(runtime, generation_prompt, reference_image_path).await
        }
        .await;
        match result {
            Ok(asset_path) => {
                app_state.write().update_revision(
                    frame.id,
                    revision_id,
                    RevisionStatus::Completed,
                    Some(asset_path),
                    None,
                );
                activity.set(GenerationActivity::Completed);
            }
            Err(error) => {
                let message = error.to_string();
                app_state.write().update_revision(
                    frame.id,
                    revision_id,
                    RevisionStatus::Failed,
                    None,
                    Some(message.clone()),
                );
                activity.set(GenerationActivity::Failed(message));
            }
        }
    });
}

async fn generate_image(
    runtime: CreativeRuntime,
    prompt: String,
    reference_image_path: Option<String>,
) -> Result<String, CreativeRuntimeError> {
    let mut request = GenerateRequest::new(prompt);
    request.reference_image_path = reference_image_path;
    request.width = Some(768);
    request.height = Some(448);
    request.steps = Some(4);
    request.seed = Some(0);
    request.model = Some(runtime.profile().profile_id().to_owned());
    request.device = Some(ComputeDevice::Auto);
    let job = runtime
        .vision_client()
        .submit_generation(&request, false)
        .await?;
    let source = runtime.wait_for_job(job).await?;
    runtime.import_asset(source).await
}

fn generated_asset_url(path: &str) -> Option<String> {
    Path::new(path)
        .file_name()
        .and_then(|filename| filename.to_str())
        .map(|filename| format!("/generated/{filename}"))
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
