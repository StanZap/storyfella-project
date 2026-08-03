use dioxus::prelude::*;

use crate::{app::AppConfig, state::AppState};

#[component]
pub fn Workspace(config: AppConfig, app_state: Signal<AppState>) -> Element {
    let project_name = app_state.read().project_name.clone();

    rsx! {
        main { class: "grid min-h-screen min-w-[720px] grid-cols-[minmax(0,1fr)_280px] grid-rows-[84px_minmax(0,1fr)_116px] bg-[radial-gradient(circle_at_45%_20%,#20283a_0,#11151f_38%,#0b0e14_100%)] font-sans text-slate-100",
            header { class: "col-span-2 flex items-center justify-between border-b border-slate-700/70 bg-slate-950/85 px-6 py-4 backdrop-blur",
                div {
                    p { class: "m-0 text-[10px] font-medium tracking-[0.16em] text-slate-400", "SMART VISUAL SEQUENCER" }
                    h1 { class: "mt-1 text-xl font-semibold tracking-tight text-slate-100", "{project_name}" }
                }
                span { class: "rounded-full border border-emerald-800 bg-emerald-950/30 px-3 py-1.5 text-xs font-medium text-emerald-200", "Bootstrap ready" }
            }
            section { class: "grid place-items-center p-8",
                div { class: "max-w-lg text-center text-slate-400",
                    div { class: "mx-auto mb-5 grid size-14 place-items-center rounded-2xl border border-slate-700/80 bg-slate-900/70 text-2xl shadow-xl shadow-black/20", "✦" }
                    h2 { class: "mb-2 text-2xl font-semibold tracking-tight text-slate-100", "Your visual workspace starts here" }
                    p { class: "text-sm leading-6", "The desktop shell, runtime boundary, and API clients are ready. AI pipeline orchestration comes next." }
                }
            }
            aside { class: "overflow-hidden border-l border-slate-700/70 bg-slate-950/70 p-5 backdrop-blur",
                div { class: "mb-5 flex items-center justify-between",
                    h2 { class: "text-sm font-semibold text-slate-100", "Runtime" }
                    span { class: "size-2 rounded-full bg-emerald-400 shadow-[0_0_10px] shadow-emerald-400/60" }
                }
                dl { class: "space-y-4",
                    div {
                        dt { class: "text-[10px] font-medium uppercase tracking-wider text-slate-500", "LM Studio" }
                        dd { class: "mt-1 truncate text-xs text-slate-300", title: "{config.lm_studio.base_url}", "{config.lm_studio.base_url}" }
                    }
                    div {
                        dt { class: "text-[10px] font-medium uppercase tracking-wider text-slate-500", "Python" }
                        dd { class: "mt-1 truncate text-xs text-slate-300", title: "{config.python_runtime.display()}", "{config.python_runtime.display()}" }
                    }
                    div {
                        dt { class: "text-[10px] font-medium uppercase tracking-wider text-slate-500", "Models" }
                        dd { class: "mt-1 truncate text-xs text-slate-300", title: "{config.model_dir.display()}", "{config.model_dir.display()}" }
                    }
                }
            }
            footer { class: "col-span-2 flex items-start gap-6 border-t border-slate-700/70 bg-slate-950 px-6 py-4 text-xs text-slate-500",
                strong { class: "font-semibold text-slate-200", "Timeline" }
                span { class: "rounded-md border border-dashed border-slate-700 px-3 py-1.5", "No clips yet" }
            }
        }
    }
}
