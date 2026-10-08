//! The assistant card — the visible surface of a plan run.
//!
//! While a plan executes, a floating card (bottom-right, above every
//! screen) narrates the steps: the current message, the step index, and
//! the operation summary while an op is in flight. The controls pause and
//! adjust narration and typing speed and stop the run. When the run finishes
//! it shows the per-step verdict. The runner and controls share the session signal.

use dioxus::prelude::*;

use crate::driver::{GuideSession, GuideState, StepStatus};

use super::icons::{Icon, IconName};

/// The narration-speed buttons: `(rate, active class)` pairs.
fn speed_options(current: u32) -> [(u32, &'static str); 3] {
    [
        (
            1,
            if current == 1 {
                "bg-violet-500/20 text-violet-200"
            } else {
                "text-zinc-600 hover:text-zinc-300"
            },
        ),
        (
            2,
            if current == 2 {
                "bg-violet-500/20 text-violet-200"
            } else {
                "text-zinc-600 hover:text-zinc-300"
            },
        ),
        (
            4,
            if current == 4 {
                "bg-violet-500/20 text-violet-200"
            } else {
                "text-zinc-600 hover:text-zinc-300"
            },
        ),
    ]
}

#[component]
pub fn GuideCard(mut session: Signal<Option<GuideSession>>) -> Element {
    let Some(active) = session.read().clone() else {
        return rsx! {};
    };
    let mode_label = if active.mode.as_str() == "guide" {
        "Guide"
    } else {
        "Check"
    };
    let step_count = active.plan.steps.len();
    let current = active.current.min(step_count.saturating_sub(1));
    let step_message = active
        .plan
        .steps
        .get(current)
        .and_then(|step| step.message.clone())
        .unwrap_or_else(|| "…".to_owned());
    let busy = active.busy.clone().unwrap_or_else(|| "working".to_owned());
    let progress_pct = if step_count == 0 {
        100.0
    } else {
        (current as f32 / step_count as f32) * 100.0
    };

    match &active.state {
        GuideState::Running => {
            let mut paused = active.paused;
            let speed = active.speed;
            let typing_cps = active.typing_cps;
            rsx! {
                div { class: "fixed bottom-6 right-6 z-40 w-[360px] overflow-hidden rounded-2xl bg-[#101014] shadow-[0_30px_90px_rgba(0,0,0,.55)] ring-1 ring-white/[0.09]",
                    div { class: "flex items-center justify-between border-b border-white/[0.06] px-4 py-3",
                        div { class: "flex min-w-0 items-center gap-2.5",
                            span { class: "size-2 shrink-0 animate-pulse rounded-full bg-violet-400" }
                            div { class: "min-w-0",
                                p { class: "truncate text-[11px] font-semibold text-zinc-100", "{active.plan.name}" }
                                p { class: "text-[9px] uppercase tracking-[0.16em] text-zinc-600", "{mode_label} · step {current + 1} of {step_count}" }
                            }
                        }
                        button {
                            class: "grid size-7 place-items-center rounded-lg text-zinc-600 transition hover:bg-white/[0.06] hover:text-zinc-300",
                            aria_label: "Stop after the current operation",
                            title: "Stop the plan. The current generation finishes in the background.",
                            onclick: move |_| session.set(None),
                            Icon { name: IconName::Close, class: "size-3.5" }
                        }
                    }
                    div { class: "px-4 py-4",
                        if active.busy.is_some() {
                            p { class: "mb-2 flex items-center gap-2 text-[10px] font-medium text-violet-300",
                                span { class: "size-2 animate-pulse rounded-full bg-violet-400" }
                                "{busy}"
                            }
                        }
                        p { class: "text-[13px] leading-6 text-zinc-200", "{step_message}" }
                    }
                    div { class: "h-0.5 w-full bg-white/[0.05]",
                        div { class: "h-full bg-violet-500/60 transition-all duration-300", width: "{progress_pct}%" }
                    }
                    div { class: "flex items-center justify-between border-t border-white/[0.06] px-4 py-2.5",
                    div { class: "flex items-center gap-1",
                        button {
                            class: "grid size-7 place-items-center rounded-lg text-zinc-500 transition hover:bg-white/[0.06] hover:text-zinc-200",
                            title: if paused { "Resume" } else { "Pause" },
                            onclick: move |_| {
                                paused = !paused;
                                session.with_mut(|slot| {
                                    if let Some(s) = slot { s.paused = paused; }
                                });
                            },
                            Icon { name: if paused { IconName::Play } else { IconName::Pause }, class: "size-3.5" }
                        }
                        for (rate, active_class) in speed_options(speed) {
                            button {
                                class: "rounded-md px-1.5 py-1 font-mono text-[9px] transition {active_class}",
                                title: "Narration speed",
                                onclick: move |_| {
                                    session.with_mut(|slot| {
                                        if let Some(s) = slot { s.speed = rate; }
                                    });
                                },
                                "{rate}x"
                            }
                        }
                    }
                        p { class: "font-mono text-[9px] text-zinc-700", "artificial user · {mode_label}" }
                    }
                    div { class: "flex items-center gap-3 border-t border-white/[0.06] px-4 py-2.5",
                        label { class: "shrink-0 text-[10px] text-zinc-400", r#for: "guide-typing-speed", "Typing" }
                        input {
                            id: "guide-typing-speed",
                            class: "min-w-0 flex-1 accent-violet-400",
                            r#type: "range",
                            min: "1", max: "200", step: "1",
                            value: "{typing_cps}",
                            oninput: move |event| {
                                if let Ok(rate @ 1..=200) = event.value().parse::<u16>() {
                                    session.with_mut(|slot| {
                                        if let Some(s) = slot { s.typing_cps = rate; }
                                    });
                                }
                            },
                        }
                        span { class: "w-16 shrink-0 text-right font-mono text-[10px] text-zinc-400", "{typing_cps} char/s" }
                    }
                }
            }
        }
        GuideState::Done(report) => {
            let passed = report.passed();
            let failed = report.failed();
            let verdict_class = if failed == 0 {
                "text-emerald-300"
            } else {
                "text-rose-300"
            };
            let rows = report.steps.iter().map(|(label, status)| {
                let (icon, icon_class, text_class) = match status {
                    StepStatus::Passed => (IconName::Check, "text-emerald-400", "text-zinc-400"),
                    StepStatus::Failed { .. } => (IconName::Close, "text-rose-400", "text-rose-200"),
                    StepStatus::Warned { .. } => (IconName::Refresh, "text-amber-400", "text-amber-200"),
                };
                let detail = match status {
                    StepStatus::Failed { reason } | StepStatus::Warned { reason } => Some(reason.as_str()),
                    StepStatus::Passed => None,
                };
                rsx! {
                    div { class: "flex items-start gap-2.5 border-b border-white/[0.04] py-2 last:border-b-0",
                        Icon { name: icon, class: "mt-0.5 size-3 shrink-0 {icon_class}" }
                        div { class: "min-w-0",
                            p { class: "truncate text-[11px] {text_class}", "{label}" }
                            if let Some(detail) = detail {
                                p { class: "mt-0.5 text-[10px] leading-4 text-zinc-600", "{detail}" }
                            }
                        }
                    }
                }
            });
            rsx! {
                div { class: "fixed bottom-6 right-6 z-40 w-[360px] overflow-hidden rounded-2xl bg-[#101014] shadow-[0_30px_90px_rgba(0,0,0,.55)] ring-1 ring-white/[0.09]",
                    div { class: "flex items-center justify-between border-b border-white/[0.06] px-4 py-3",
                        div { class: "min-w-0",
                            p { class: "truncate text-[11px] font-semibold text-zinc-100", "{active.plan.name}" }
                            p { class: "text-[9px] uppercase tracking-[0.16em] text-zinc-600", "{mode_label} · finished in {report.duration_ms / 1000}s" }
                        }
                        button {
                            class: "grid size-7 place-items-center rounded-lg text-zinc-600 transition hover:bg-white/[0.06] hover:text-zinc-300",
                            title: "Dismiss",
                            onclick: move |_| session.set(None),
                            Icon { name: IconName::Close, class: "size-3.5" }
                        }
                    }
                    div { class: "max-h-64 overflow-y-auto px-4 py-1", {rows} }
                    div { class: "flex items-center justify-between border-t border-white/[0.06] px-4 py-2.5",
                        p { class: "text-[11px] font-semibold {verdict_class}", "{passed} passed · {failed} failed" }
                        p { class: "font-mono text-[9px] text-zinc-700", "check mode · state-level" }
                    }
                }
            }
        }
    }
}
