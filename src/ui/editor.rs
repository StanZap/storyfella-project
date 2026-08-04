use dioxus::prelude::*;

use crate::state::AppState;

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

#[component]
pub fn Studio(app_state: Signal<AppState>) -> Element {
    let mut view = use_signal(|| StudioView::Canvas);
    let prompt = use_signal(String::new);
    let project = app_state.read().project.clone();
    let frame_count = project.storyboard.len();

    rsx! {
        section { class: "grid min-h-0 grid-cols-[minmax(0,1fr)_292px] grid-rows-[minmax(0,1fr)_156px]",
            div { class: "relative min-h-0 overflow-hidden bg-[radial-gradient(circle_at_50%_42%,rgba(67,56,115,.16),transparent_36%)]",
                div { class: "absolute left-1/2 top-4 z-10 flex -translate-x-1/2 items-center gap-1 rounded-xl bg-zinc-950/70 p-1 ring-1 ring-inset ring-white/[0.07] backdrop-blur-xl",
                    ViewButton { label: "Canvas", icon: IconName::Canvas, active: view() == StudioView::Canvas, onclick: move |_| view.set(StudioView::Canvas) }
                    ViewButton { label: "Storyboard", icon: IconName::Grid, active: view() == StudioView::Storyboard, onclick: move |_| view.set(StudioView::Storyboard) }
                    ViewButton { label: "Timeline", icon: IconName::Timeline, active: view() == StudioView::Timeline, onclick: move |_| view.set(StudioView::Timeline) }
                }

                match view() {
                    StudioView::Canvas => rsx! { CanvasView { app_state, prompt } },
                    StudioView::Storyboard => rsx! { StoryboardView { app_state } },
                    StudioView::Timeline => rsx! { TimelineView { app_state } },
                }
            }

            aside { class: "min-h-0 overflow-y-auto border-l border-white/[0.055] bg-zinc-950/35",
                div { class: "flex h-14 items-center justify-between border-b border-white/[0.055] px-5",
                    h2 { class: "text-xs font-medium text-zinc-300", "Properties" }
                    button { class: "text-zinc-600 transition hover:text-zinc-300", aria_label: "More properties", Icon { name: IconName::More, class: "size-4" } }
                }
                if frame_count == 0 {
                    div { class: "grid h-[calc(100%-3.5rem)] min-h-60 place-items-center px-8",
                        EmptyVisual {
                            icon: IconName::Layers,
                            title: "Nothing selected",
                            description: "Select a generated frame or timeline beat to adjust its details."
                        }
                    }
                } else {
                    Inspector { app_state }
                }
            }

            StoryStrip { app_state, on_open_storyboard: move |_| view.set(StudioView::Storyboard) }
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
fn CanvasView(app_state: Signal<AppState>, mut prompt: Signal<String>) -> Element {
    let frame_count = app_state.read().project.storyboard.len();
    let mut submit_prompt = move || {
        let value = prompt();
        if !value.trim().is_empty() {
            app_state.write().add_storyboard_beat(value);
            prompt.set(String::new());
        }
    };

    rsx! {
        div { class: "flex h-full min-h-[420px] flex-col items-center justify-center px-8 pb-16 pt-20",
            if frame_count == 0 {
                div { class: "mb-10 text-center",
                    div { class: "mx-auto mb-5 grid size-12 place-items-center text-violet-300",
                        Icon { name: IconName::Sparkles, class: "size-7" }
                    }
                    h2 { class: "text-[22px] font-medium tracking-[-0.03em] text-zinc-100", "What should we create?" }
                    p { class: "mt-2 text-[13px] text-zinc-600", "Describe a scene. The planner will turn it into a visual sequence." }
                }
            } else {
                div { class: "mb-8 flex items-center gap-3 text-xs text-zinc-500",
                    span { class: "grid size-7 place-items-center rounded-lg bg-violet-500/10 text-violet-300", "{frame_count}" }
                    span { "story beats ready to generate" }
                }
            }

            div { class: "w-full max-w-2xl rounded-[22px] bg-zinc-900/75 p-2 shadow-[0_24px_80px_rgba(0,0,0,.35)] ring-1 ring-inset ring-white/[0.09] backdrop-blur-xl focus-within:ring-violet-400/35",
                textarea {
                    class: "block min-h-20 w-full resize-none bg-transparent px-3 py-2.5 text-[13px] leading-6 text-zinc-100 outline-none placeholder:text-zinc-600",
                    placeholder: "A quiet lighthouse above a silver sea at dusk…",
                    value: "{prompt}",
                    oninput: move |event| prompt.set(event.value()),
                    onkeydown: move |event| {
                        if event.key() == Key::Enter && !event.modifiers().shift() {
                            event.prevent_default();
                            submit_prompt();
                        }
                    }
                }
                div { class: "flex items-center justify-between px-1 pb-1",
                    div { class: "flex items-center gap-1",
                        button { class: "grid size-8 place-items-center rounded-lg text-zinc-500 transition hover:bg-white/[0.05] hover:text-zinc-200", title: "Add reference", aria_label: "Add reference", Icon { name: IconName::Add, class: "size-4" } }
                        button { class: "flex h-8 items-center gap-1.5 rounded-lg px-2 text-[11px] text-zinc-500 transition hover:bg-white/[0.05] hover:text-zinc-200",
                            Icon { name: IconName::Layers, class: "size-3.5" }
                            "Sequence"
                        }
                    }
                    button {
                        class: "flex h-9 items-center gap-2 rounded-xl bg-zinc-100 px-3.5 text-[11px] font-semibold text-zinc-950 transition hover:bg-white disabled:cursor-not-allowed disabled:opacity-35",
                        disabled: prompt.read().trim().is_empty(),
                        onclick: move |_| submit_prompt(),
                        Icon { name: IconName::Generate, class: "size-3.5" }
                        "Plan scene"
                    }
                }
            }
            p { class: "mt-3 text-[10px] text-zinc-700", "Enter to plan · Shift + Enter for a new line" }
        }
    }
}

#[component]
fn StoryboardView(app_state: Signal<AppState>) -> Element {
    let frames = app_state.read().project.storyboard.clone();
    rsx! {
        div { class: "h-full overflow-y-auto px-8 pb-8 pt-20",
            div { class: "mx-auto max-w-5xl",
                div { class: "mb-6 flex items-end justify-between",
                    div {
                        h2 { class: "text-lg font-medium tracking-tight text-zinc-100", "Storyboard" }
                        p { class: "mt-1 text-xs text-zinc-600", "The visual beats that shape this sequence." }
                    }
                    span { class: "text-[11px] text-zinc-600", "{frames.len()} beats" }
                }
                if frames.is_empty() {
                    div { class: "grid min-h-72 place-items-center border-y border-white/[0.05]",
                        EmptyVisual { icon: IconName::Grid, title: "Your story is still open", description: "Return to Canvas and describe the first scene to begin." }
                    }
                } else {
                    div { class: "grid grid-cols-[repeat(auto-fill,minmax(190px,1fr))] gap-x-4 gap-y-7",
                        for (index, frame) in frames.iter().enumerate() {
                            article { class: "group min-w-0",
                                div { class: "relative aspect-video overflow-hidden rounded-xl bg-[linear-gradient(145deg,rgba(124,58,237,.13),rgba(24,24,27,.8))] ring-1 ring-inset ring-white/[0.07]",
                                    div { class: "absolute inset-0 grid place-items-center text-zinc-700", Icon { name: IconName::Sparkles, class: "size-5" } }
                                    span { class: "absolute left-2.5 top-2.5 rounded-md bg-black/45 px-1.5 py-1 text-[9px] font-medium text-zinc-400 backdrop-blur", "{index + 1}" }
                                    button { class: "absolute bottom-2.5 right-2.5 grid size-7 translate-y-1 place-items-center rounded-lg bg-zinc-100 text-zinc-950 opacity-0 shadow-lg transition group-hover:translate-y-0 group-hover:opacity-100", aria_label: "Generate beat", Icon { name: IconName::Play, class: "size-3.5" } }
                                }
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
fn TimelineView(app_state: Signal<AppState>) -> Element {
    let clips = app_state.read().project.timeline.clips.clone();
    rsx! {
        div { class: "h-full overflow-auto px-8 pb-8 pt-20",
            div { class: "mx-auto min-w-[640px] max-w-5xl",
                div { class: "mb-5 flex items-center justify-between",
                    div { h2 { class: "text-lg font-medium tracking-tight text-zinc-100", "Timeline" } p { class: "mt-1 text-xs text-zinc-600", "Arrange story beats in time." } }
                    button { class: "flex items-center gap-2 rounded-lg bg-white/[0.05] px-3 py-2 text-[11px] text-zinc-400 hover:text-zinc-100", Icon { name: IconName::Play, class: "size-3" } "Preview" }
                }
                div { class: "border-y border-white/[0.055] py-5",
                    div { class: "mb-3 grid grid-cols-8 text-[9px] text-zinc-700",
                        for second in [0, 5, 10, 15, 20, 25, 30, 35] { span { "{second}s" } }
                    }
                    div { class: "relative h-20 bg-[linear-gradient(90deg,rgba(255,255,255,.035)_1px,transparent_1px)] bg-[size:12.5%_100%]",
                        if clips.is_empty() {
                            p { class: "absolute inset-0 grid place-items-center text-xs text-zinc-700", "No beats on the timeline" }
                        }
                        for (index, clip) in clips.iter().enumerate() {
                            div {
                                class: "absolute top-2 flex h-14 items-center overflow-hidden rounded-lg bg-violet-500/15 px-3 text-[10px] text-violet-200 ring-1 ring-inset ring-violet-400/20",
                                style: "left: {clip.start_seconds * 2.5}%; width: {clip.duration_seconds * 2.5}%; min-width: 72px; top: {8 + (index % 2) * 4}px",
                                "{clip.label}"
                            }
                        }
                    }
                }
            }
        }
    }
}

#[component]
fn Inspector(app_state: Signal<AppState>) -> Element {
    let frame = app_state.read().project.storyboard.last().cloned();
    rsx! {
        if let Some(frame) = frame {
            div { class: "space-y-7 p-5",
                div {
                    p { class: "mb-2 text-[9px] font-semibold uppercase tracking-[0.16em] text-zinc-600", "Prompt" }
                    p { class: "text-xs leading-5 text-zinc-400", "{frame.prompt}" }
                }
                div {
                    p { class: "mb-3 text-[9px] font-semibold uppercase tracking-[0.16em] text-zinc-600", "Generation" }
                    dl { class: "space-y-3 text-[11px]",
                        div { class: "flex justify-between", dt { class: "text-zinc-600", "Model" } dd { class: "text-zinc-300", "Krea 2 Turbo" } }
                        div { class: "flex justify-between", dt { class: "text-zinc-600", "Quality" } dd { class: "text-zinc-300", "Draft · Q2" } }
                        div { class: "flex justify-between", dt { class: "text-zinc-600", "Format" } dd { class: "text-zinc-300", "16:9" } }
                    }
                }
                button { class: "flex h-9 w-full items-center justify-center gap-2 rounded-xl bg-white/[0.06] text-[11px] font-medium text-zinc-300 transition hover:bg-white/[0.09] hover:text-white",
                    Icon { name: IconName::Generate, class: "size-3.5" }
                    "Generate frame"
                }
            }
        }
    }
}

#[component]
fn StoryStrip(
    app_state: Signal<AppState>,
    on_open_storyboard: EventHandler<MouseEvent>,
) -> Element {
    let frames = app_state.read().project.storyboard.clone();
    rsx! {
        footer { class: "col-span-2 min-w-0 border-t border-white/[0.055] bg-zinc-950/45",
            div { class: "flex h-9 items-center justify-between border-b border-white/[0.04] px-4",
                button { class: "flex items-center gap-2 text-[10px] font-medium text-zinc-400 hover:text-zinc-100", onclick: on_open_storyboard,
                    Icon { name: IconName::Grid, class: "size-3" }
                    "Story beats"
                    span { class: "text-zinc-700", "{frames.len()}" }
                }
                div { class: "flex items-center gap-3 text-[9px] text-zinc-700", span { "00:00" } span { "—" } span { "00:{frames.len() * 5:02}" } }
            }
            div { class: "flex h-[116px] items-center gap-2 overflow-x-auto px-4",
                for (index, frame) in frames.iter().enumerate() {
                    button { class: "group flex w-36 shrink-0 items-center gap-2 rounded-xl p-1.5 text-left transition hover:bg-white/[0.04]",
                        div { class: "grid aspect-video w-20 shrink-0 place-items-center rounded-lg bg-violet-500/[0.08] text-[9px] text-violet-300 ring-1 ring-inset ring-white/[0.06]", "{index + 1}" }
                        p { class: "line-clamp-3 text-[9px] leading-4 text-zinc-600 group-hover:text-zinc-400", "{frame.prompt}" }
                    }
                }
                button { class: "grid aspect-video w-20 shrink-0 place-items-center rounded-lg border border-dashed border-white/[0.09] text-zinc-700 transition hover:border-white/[0.16] hover:text-zinc-400", aria_label: "Add story beat", Icon { name: IconName::Add, class: "size-4" } }
            }
        }
    }
}
