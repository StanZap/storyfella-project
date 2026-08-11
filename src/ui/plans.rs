//! The Plans screen — the plan library.
//!
//! Every `*.yaml` in `config/plans/` is a runnable plan: a sequence of
//! steps in the app's own language (see `crate::driver`). From here a plan
//! runs in Guide mode (narrated, visible — the tutoring experience) or
//! Check mode (fast regression; failures are UX issues).

use std::path::Path;

use dioxus::prelude::*;

use crate::driver::{self, GuideMode, Plan};

use super::icons::{Icon, IconName};

/// The run-mode segmented control.
#[component]
fn ModeToggle(mode: GuideMode, mut on_mode: EventHandler<GuideMode>) -> Element {
    let button = |label: &'static str, value: GuideMode| {
        let active = mode == value;
        let active_class = if active {
            "bg-violet-500/20 text-violet-200"
        } else {
            "text-zinc-500 hover:text-zinc-300"
        };
        rsx! {
            button {
                class: "rounded-md px-2.5 py-1 text-[10px] font-medium transition {active_class}",
                onclick: move |_| on_mode.call(value),
                "{label}"
            }
        }
    };
    rsx! {
        div { class: "flex items-center gap-0.5 rounded-lg bg-white/[0.04] p-0.5 ring-1 ring-inset ring-white/[0.06]",
            {button("Guide", GuideMode::Guide)}
            {button("Check", GuideMode::Check)}
        }
    }
}

#[component]
pub fn Plans(on_run: EventHandler<(Plan, GuideMode)>) -> Element {
    let mut files = use_signal(|| driver::discover(Path::new(driver::PLANS_DIR)));
    let mode = use_signal(GuideMode::default);

    let list = files.read().clone();
    let empty = list.is_empty();
    let cards = list.iter().map(|file| {
        let plan = file.plan.clone();
        let error = file.error.clone();
        let path = file.path.display().to_string();
        let mut mode = mode;
        let run_mode = mode();
        let steps_label = format!("{} steps", plan.steps.len());
        let plan_name = plan.name.clone();
        let description = plan.description.clone();
        rsx! {
            div { class: "rounded-2xl bg-white/[0.025] p-5 ring-1 ring-inset ring-white/[0.07] transition hover:ring-white/[0.12]",
                div { class: "flex items-start justify-between gap-6",
                    div { class: "min-w-0",
                        h3 { class: "text-sm font-medium text-zinc-100", "{plan_name}" }
                        p { class: "mt-1 max-w-xl text-[11px] leading-5 text-zinc-600",
                            if description.is_empty() {
                                "No description."
                            } else {
                                "{description}"
                            }
                        }
                        p { class: "mt-1 font-mono text-[9px] text-zinc-700", "{path}" }
                    }
                    div { class: "flex shrink-0 items-center gap-3",
                        ModeToggle { mode: run_mode, on_mode: move |value| mode.set(value) }
                        if error.is_none() {
                            button {
                                class: "flex h-8 items-center gap-2 rounded-lg bg-violet-500/15 px-4 text-[10px] font-semibold text-violet-200 ring-1 ring-inset ring-violet-400/25 transition hover:bg-violet-500/25 hover:text-violet-100",
                                onclick: move |_| on_run.call((plan.clone(), mode())),
                                Icon { name: IconName::Play, class: "size-3" }
                                "Run"
                            }
                        }
                    }
                }
                div { class: "mt-4 flex items-center gap-4 text-[10px] text-zinc-600",
                    if let Some(error) = error {
                        div { class: "rounded-lg bg-rose-400/10 px-2.5 py-1 font-mono text-rose-300 ring-1 ring-inset ring-rose-400/20",
                            "{error}"
                        }
                    } else {
                        span { "{steps_label}" }
                        span { "·" }
                        span { if run_mode == GuideMode::Guide { "guide — watch it narrate the steps" } else { "check — fast, pass/fail verdict" } }
                    }
                }
            }
        }
    });

    rsx! {
        section { class: "h-full overflow-y-auto px-10 py-10",
            div { class: "mx-auto max-w-5xl",
                header { class: "mb-8 flex items-end justify-between",
                    div {
                        p { class: "mb-2 text-[10px] font-semibold uppercase tracking-[0.18em] text-violet-400", "Automation" }
                        h1 { class: "text-3xl font-semibold tracking-[-0.04em] text-zinc-100", "Plans" }
                        p { class: "mt-2 max-w-xl text-sm text-zinc-600",
                            "Reusable scripts in the app's own language — run them as narrated guides or as regression checks. Plans live in config/plans/."
                        }
                    }
                    button {
                        class: "flex h-9 items-center gap-2 rounded-xl bg-white/[0.05] px-4 text-xs font-medium text-zinc-300 transition hover:bg-white/[0.09] hover:text-white",
                        title: "Rescan the plans directory",
                        onclick: move |_| files.set(driver::discover(Path::new(driver::PLANS_DIR))),
                        Icon { name: IconName::Refresh, class: "size-3.5" }
                        "Rescan"
                    }
                }

                if empty {
                    div { class: "grid min-h-64 place-items-center rounded-2xl border-y border-white/[0.05]",
                        div { class: "text-center",
                            Icon { name: IconName::List, class: "mx-auto mb-3 size-6 text-zinc-700" }
                            p { class: "text-xs text-zinc-600", "No plans yet — drop a .yaml file into config/plans/." }
                        }
                    }
                } else {
                    div { class: "space-y-3", {cards} }
                }
            }
        }
    }
}
