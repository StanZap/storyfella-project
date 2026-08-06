use std::rc::Rc;

use dioxus::prelude::*;

use crate::{errors::ReportedError, runtime::HealthStatus};

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
pub fn StatusDot(status: HealthStatus) -> Element {
    let class = match status {
        HealthStatus::Online => {
            "size-1.5 rounded-full bg-emerald-400 shadow-[0_0_8px_rgba(52,211,153,.55)]"
        }
        HealthStatus::Degraded => {
            "size-1.5 rounded-full bg-amber-400 shadow-[0_0_8px_rgba(251,191,36,.45)]"
        }
        HealthStatus::Idle => "size-1.5 rounded-full bg-zinc-600",
        HealthStatus::Offline => "size-1.5 rounded-full bg-rose-400",
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

/// The app-wide error surface: a fixed banner (code + short message) with
/// a Details action opening the full report modal. Owned by the Workspace,
/// fed by every error-surfacing site through the shared
/// `Signal<Option<ReportedError>>` context.
#[component]
pub fn ErrorToast(error_report: Signal<Option<ReportedError>>) -> Element {
    let mut show_details = use_signal(|| false);
    let report = error_report.read().clone();
    match report {
        None => rsx! { Fragment {} },
        Some(report) => rsx! {
            div { class: "fixed bottom-6 right-6 z-40 w-full max-w-sm",
                div { class: "flex items-start gap-3 rounded-xl bg-zinc-900/95 p-4 shadow-[0_18px_60px_rgba(0,0,0,.5)] ring-1 ring-inset ring-rose-400/25 backdrop-blur-xl",
                    span { class: "mt-1 size-1.5 shrink-0 rounded-full bg-rose-400" }
                    div { class: "min-w-0 flex-1",
                        p { class: "font-mono text-[9px] font-medium text-rose-300/80", "{report.code.code()}" }
                        p { class: "mt-0.5 text-[11px] font-medium leading-4 text-zinc-200", "{report.code.message()}" }
                    }
                    button {
                        class: "shrink-0 rounded-lg bg-white/[0.06] px-2.5 py-1.5 text-[9px] font-medium text-zinc-400 transition hover:bg-white/[0.1] hover:text-white",
                        onclick: move |_| show_details.set(true),
                        "Details"
                    }
                    button {
                        class: "shrink-0 rounded-lg p-1.5 text-zinc-600 transition hover:bg-white/[0.06] hover:text-zinc-300",
                        aria_label: "Dismiss error",
                        title: "Dismiss",
                        onclick: move |_| error_report.set(None),
                        Icon { name: IconName::Close, class: "size-3" }
                    }
                }
            }
            if show_details() {
                ErrorDetailsModal {
                    error: report,
                    on_close: move |_| show_details.set(false),
                }
            }
        },
    }
}

/// The full error report: code chip, the error text, the remediation hint,
/// and copy — the Copy button and a click on the error text both put the
/// whole report on the clipboard.
#[component]
fn ErrorDetailsModal(error: ReportedError, on_close: EventHandler<()>) -> Element {
    let mut copied = use_signal(|| false);
    // `Rc` keeps the closure `Clone` so one handler can serve both the copy
    // button and the click-to-copy on the error text.
    let copy_text = Rc::new(error.copy_text());
    let copy_action = move |_| {
        copy_to_clipboard((*copy_text).clone());
        copied.set(true);
    };
    let copied_label = if copied() { "Copied" } else { "Copy" };
    rsx! {
        div {
            class: "fixed inset-0 z-50 grid place-items-center bg-black/60 p-6 backdrop-blur-sm",
            onclick: move |_| on_close.call(()),
            div {
                class: "w-full max-w-xl rounded-2xl bg-zinc-950 p-6 shadow-[0_30px_90px_rgba(0,0,0,.5)] ring-1 ring-white/[0.09]",
                onclick: move |event| event.stop_propagation(),
                div { class: "mb-3 flex items-center gap-2",
                    span { class: "size-2 rounded-full bg-rose-400" }
                    p { class: "text-[10px] font-semibold uppercase tracking-[0.18em] text-zinc-500", "Error details" }
                    span { class: "rounded-md bg-rose-400/10 px-2 py-0.5 font-mono text-[9px] text-rose-300 ring-1 ring-inset ring-rose-400/20", "{error.code.code()}" }
                }
                h2 { class: "mb-1 text-base font-semibold text-zinc-100", "{error.code.message()}" }
                p {
                    class: "max-h-44 cursor-pointer overflow-y-auto whitespace-pre-wrap rounded-lg bg-white/[0.035] px-3 py-2.5 font-mono text-[10px] leading-4 text-zinc-300 ring-1 ring-inset ring-white/[0.06] transition hover:ring-rose-400/30",
                    title: "Click to copy the error",
                    onclick: copy_action.clone(),
                    "{error.details}"
                }
                if !error.context.is_empty() {
                    div { class: "mt-3 rounded-lg bg-white/[0.035] px-3 py-2.5 ring-1 ring-inset ring-white/[0.06]",
                        p { class: "mb-1.5 text-[9px] font-semibold uppercase tracking-[0.14em] text-zinc-600", "Context" }
                        for (key, value) in &error.context {
                            p { class: "truncate font-mono text-[9px] leading-4 text-zinc-500", title: "{value}", "{key}: {value}" }
                        }
                    }
                }
                p { class: "mt-3 rounded-lg bg-amber-400/[0.06] px-3 py-2 text-[10px] leading-4 text-amber-200/80 ring-1 ring-inset ring-amber-300/15",
                    "{error.code.hint()}"
                }
                div { class: "mt-5 flex justify-end gap-2",
                    button {
                        class: "flex h-8 items-center gap-2 rounded-lg bg-white/[0.05] px-3.5 text-[10px] font-medium text-zinc-300 ring-1 ring-inset ring-white/[0.07] transition hover:bg-white/[0.09] hover:text-white",
                        onclick: copy_action,
                        if copied() {
                            Icon { name: IconName::Check, class: "size-3" }
                        } else {
                            Icon { name: IconName::Copy, class: "size-3" }
                        }
                        "{copied_label}"
                    }
                    button {
                        class: "flex h-8 items-center rounded-lg bg-zinc-100 px-3.5 text-[10px] font-semibold text-zinc-950 transition hover:bg-white",
                        onclick: move |_| on_close.call(()),
                        "Close"
                    }
                }
            }
        }
    }
}

/// Pushes a report onto the shared error surface (the Workspace's
/// `Signal<Option<ReportedError>>` context): the toast appears and its
/// Details modal opens the full report. Used by inline error rows that
/// keep a report locally (Settings) while the modal stays global.
#[component]
pub fn ErrorDetailsButton(error: ReportedError) -> Element {
    let mut error_report = use_context::<Signal<Option<ReportedError>>>();
    rsx! {
        button {
            class: "shrink-0 rounded-lg bg-white/[0.06] px-2.5 py-1.5 text-[9px] font-medium text-zinc-400 transition hover:bg-white/[0.1] hover:text-white",
            onclick: move |_| error_report.set(Some(error.clone())),
            "Details"
        }
    }
}

/// Puts `text` on the clipboard through the webview: `navigator.clipboard`
/// first, with the execCommand fallback for webviews without the async API.
fn copy_to_clipboard(text: String) {
    let escaped = serde_json::to_string(&text).unwrap_or_else(|_| "\"\"".to_owned());
    dioxus::document::eval(&format!(
        r#"(() => {{
            const value = {escaped};
            const fallback = () => {{
                const ta = document.createElement('textarea');
                ta.value = value;
                ta.style.position = 'fixed';
                ta.style.opacity = '0';
                document.body.appendChild(ta);
                ta.select();
                document.execCommand('copy');
                document.body.removeChild(ta);
            }};
            if (navigator.clipboard && navigator.clipboard.writeText) {{
                navigator.clipboard.writeText(value).catch(fallback);
            }} else {{
                fallback();
            }}
        }})()"#,
    ));
}
