use dioxus::prelude::*;

use super::icons::{Icon, IconName};

#[component]
pub fn RailButton(
    label: String,
    icon: IconName,
    active: bool,
    onclick: EventHandler<MouseEvent>,
) -> Element {
    let classes = if active {
        "group relative grid size-10 place-items-center rounded-xl bg-white/[0.09] text-white transition"
    } else {
        "group relative grid size-10 place-items-center rounded-xl text-zinc-500 transition hover:bg-white/[0.05] hover:text-zinc-200"
    };
    rsx! {
        button { class: classes, title: "{label}", aria_label: "{label}", onclick,
            if active {
                span { class: "absolute -left-[13px] h-5 w-0.5 rounded-full bg-violet-400" }
            }
            Icon { name: icon, class: "size-[19px]" }
            span { class: "pointer-events-none absolute left-12 z-50 hidden whitespace-nowrap rounded-lg border border-white/10 bg-zinc-900 px-2.5 py-1.5 text-[11px] text-zinc-200 shadow-xl group-hover:block", "{label}" }
        }
    }
}

#[component]
pub fn StatusDot(online: bool) -> Element {
    let class = if online {
        "size-1.5 rounded-full bg-emerald-400 shadow-[0_0_8px_rgba(52,211,153,.55)]"
    } else {
        "size-1.5 rounded-full bg-zinc-600"
    };
    rsx! { span { class } }
}

#[component]
pub fn SectionHeading(eyebrow: String, title: String, description: String) -> Element {
    rsx! {
        header { class: "mb-7 max-w-2xl",
            p { class: "mb-2 text-[10px] font-semibold uppercase tracking-[0.18em] text-violet-400", "{eyebrow}" }
            h2 { class: "text-2xl font-semibold tracking-[-0.025em] text-zinc-100", "{title}" }
            p { class: "mt-2 text-sm leading-6 text-zinc-500", "{description}" }
        }
    }
}

#[component]
pub fn SettingRow(
    title: String,
    description: String,
    #[props(default)] last: bool,
    children: Element,
) -> Element {
    let border = if last {
        ""
    } else {
        "border-b border-white/[0.055]"
    };
    rsx! {
        div { class: "grid min-h-20 grid-cols-[minmax(180px,1fr)_minmax(260px,1.2fr)] items-center gap-8 py-4 {border}",
            div {
                p { class: "text-[13px] font-medium text-zinc-200", "{title}" }
                p { class: "mt-1 text-xs leading-5 text-zinc-600", "{description}" }
            }
            {children}
        }
    }
}

#[component]
pub fn EmptyVisual(icon: IconName, title: String, description: String) -> Element {
    rsx! {
        div { class: "mx-auto flex max-w-sm flex-col items-center text-center",
            div { class: "mb-5 grid size-12 place-items-center rounded-2xl bg-white/[0.045] text-zinc-500 ring-1 ring-inset ring-white/[0.06]",
                Icon { name: icon, class: "size-5" }
            }
            h3 { class: "text-sm font-medium text-zinc-200", "{title}" }
            p { class: "mt-2 text-xs leading-5 text-zinc-600", "{description}" }
        }
    }
}
