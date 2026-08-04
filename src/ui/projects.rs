use dioxus::prelude::*;

use crate::state::AppState;

use super::icons::{Icon, IconName};

#[component]
pub fn Projects(app_state: Signal<AppState>, on_open: EventHandler<MouseEvent>) -> Element {
    let mut name = use_signal(String::new);
    let current_project = app_state.read().project.clone();

    rsx! {
        section { class: "h-full overflow-y-auto px-10 py-10",
            div { class: "mx-auto max-w-5xl",
                header { class: "mb-12 flex items-end justify-between",
                    div {
                        p { class: "mb-2 text-[10px] font-semibold uppercase tracking-[0.18em] text-violet-400", "Workspace" }
                        h1 { class: "text-3xl font-semibold tracking-[-0.04em] text-zinc-100", "Your stories" }
                        p { class: "mt-2 text-sm text-zinc-600", "Create, continue, and shape visual sequences." }
                    }
                }

                div { class: "grid grid-cols-[minmax(0,1.4fr)_minmax(260px,.8fr)] gap-10",
                    div {
                        h2 { class: "mb-4 text-[11px] font-medium text-zinc-500", "Recent" }
                        button { class: "group flex w-full items-center gap-5 border-y border-white/[0.06] py-5 text-left", onclick: on_open,
                            div { class: "grid aspect-video w-32 shrink-0 place-items-center rounded-xl bg-[linear-gradient(145deg,rgba(124,58,237,.16),rgba(24,24,27,.7))] text-violet-300 ring-1 ring-inset ring-white/[0.07]",
                                Icon { name: IconName::Sparkles, class: "size-5" }
                            }
                            div { class: "min-w-0 flex-1",
                                h3 { class: "truncate text-sm font-medium text-zinc-200 group-hover:text-white", "{current_project.name}" }
                                p { class: "mt-1 text-[11px] text-zinc-600", "{current_project.storyboard.len()} beats · Edited in this session" }
                            }
                            Icon { name: IconName::Arrow, class: "size-4 text-zinc-700 transition group-hover:translate-x-1 group-hover:text-zinc-300" }
                        }
                    }

                    aside { class: "border-l border-white/[0.06] pl-10",
                        div { class: "mb-5 grid size-10 place-items-center rounded-xl bg-white/[0.045] text-zinc-400", Icon { name: IconName::Add, class: "size-4" } }
                        h2 { class: "text-sm font-medium text-zinc-200", "Start a new story" }
                        p { class: "mt-2 text-xs leading-5 text-zinc-600", "Begin with an idea. Structure and visuals can evolve as you work." }
                        input {
                            class: "mt-6 h-10 w-full border-b border-white/[0.1] bg-transparent text-sm text-zinc-100 outline-none transition placeholder:text-zinc-700 focus:border-violet-400/60",
                            placeholder: "Story name",
                            value: "{name}",
                            oninput: move |event| name.set(event.value()),
                        }
                        button {
                            class: "mt-5 flex h-10 w-full items-center justify-center gap-2 rounded-xl bg-zinc-100 text-xs font-semibold text-zinc-950 transition hover:bg-white disabled:opacity-35",
                            disabled: name.read().trim().is_empty(),
                            onclick: move |event| {
                                app_state.write().create_project(name());
                                name.set(String::new());
                                on_open.call(event);
                            },
                            "Create story"
                            Icon { name: IconName::Arrow, class: "size-3.5" }
                        }
                    }
                }
            }
        }
    }
}
