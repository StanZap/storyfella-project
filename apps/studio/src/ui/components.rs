use dioxus::prelude::*;

use storyfella_studio_core::{errors::ReportedError, runtime::HealthStatus};

use super::dialog::Dialog;
use super::icons::{Icon, IconName};

#[derive(Clone, Copy)]
pub struct ErrorDetails(pub Signal<Option<ReportedError>>);

#[component]
pub fn ErrorDetailsHost() -> Element {
    let mut details = use_context::<ErrorDetails>().0;
    let report = details.read().clone();
    rsx! {
        if let Some(error) = report {
            ErrorDetailsModal { error, on_close: move |_| details.set(None) }
        }
    }
}

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
    let row_id = use_hook(|| format!("setting-{}", uuid::Uuid::new_v4()));
    let label_id = format!("{row_id}-label");
    let description_id = format!("{row_id}-description");
    let mount_id = row_id.clone();
    let border = if last {
        ""
    } else {
        "border-b border-white/[0.055]"
    };
    rsx! {
        div { id: row_id, role: "group", aria_label: title.clone(), class: "grid min-h-20 grid-cols-[minmax(180px,1fr)_minmax(260px,1.2fr)] items-center gap-8 py-4 {border}",
            onmounted: move |_| { document::eval(&format!(
                "document.getElementById('{mount_id}').querySelectorAll('input,textarea,select,button[role=switch]').forEach(control => {{ if (!control.hasAttribute('aria-label')) control.setAttribute('aria-labelledby', '{mount_id}-label'); control.setAttribute('aria-describedby', '{mount_id}-description'); }});"
            )); },
            div {
                p { id: label_id, class: "text-[13px] font-medium text-zinc-200", "{title}" }
                p { id: description_id, class: "mt-1 text-xs leading-5 text-zinc-600", "{description}" }
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
    let mut details = use_context::<ErrorDetails>().0;
    let report = error_report.read().clone();
    match report {
        None => rsx! { Fragment {} },
        Some(report) => rsx! {
            div { class: "fixed right-4 top-14 z-40 w-[min(384px,calc(100vw-32px))]", role: "alert",
                div { class: "flex items-start gap-3 rounded-xl bg-zinc-900/95 p-4 shadow-[0_18px_60px_rgba(0,0,0,.5)] ring-1 ring-inset ring-rose-400/25 backdrop-blur-xl",
                    span { class: "mt-1 size-1.5 shrink-0 rounded-full bg-rose-400" }
                    div { class: "min-w-0 flex-1",
                        p { class: "font-mono text-[9px] font-medium text-rose-300/80", "{report.code.code()}" }
                        p { class: "mt-0.5 text-[11px] font-medium leading-4 text-zinc-200", "{report.code.message()}" }
                    }
                    button {
                        class: "shrink-0 rounded-lg bg-white/[0.06] px-2.5 py-1.5 text-[9px] font-medium text-zinc-400 transition hover:bg-white/[0.1] hover:text-white",
                        onclick: move |_| details.set(Some(report.clone())),
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
        },
    }
}

/// The full error report: code chip, the error text, the remediation hint,
/// and an explicit Copy action. Diagnostic text stays selectable.
#[component]
fn ErrorDetailsModal(error: ReportedError, on_close: EventHandler<()>) -> Element {
    let mut copy_status = use_signal(|| "Copy".to_owned());
    let copy_text = error.copy_text();
    let copy_action = move |_| {
        let text = copy_text.clone();
        spawn(async move {
            copy_status.set(
                if copy_to_clipboard(text).await {
                    "Copied"
                } else {
                    "Copy failed — retry"
                }
                .to_owned(),
            );
        });
    };
    rsx! {
        Dialog { title: "Error details", on_close,
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
                    class: "max-h-44 overflow-y-auto whitespace-pre-wrap rounded-lg bg-white/[0.035] px-3 py-2.5 font-mono text-xs leading-5 text-zinc-300 ring-1 ring-inset ring-white/[0.06] select-text",
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
                        if copy_status() == "Copied" {
                            Icon { name: IconName::Check, class: "size-3" }
                        } else {
                            Icon { name: IconName::Copy, class: "size-3" }
                        }
                        "{copy_status}"
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

/// Opens the shared report directly from an inline error row.
#[component]
pub fn ErrorDetailsButton(error: ReportedError) -> Element {
    let mut details = use_context::<ErrorDetails>().0;
    rsx! {
        button {
            class: "shrink-0 rounded-lg bg-white/[0.06] px-2.5 py-1.5 text-[9px] font-medium text-zinc-400 transition hover:bg-white/[0.1] hover:text-white",
            onclick: move |_| details.set(Some(error.clone())),
            "Details"
        }
    }
}

/// Puts `text` on the clipboard through the webview: `navigator.clipboard`
/// first, with the execCommand fallback for webviews without the async API.
async fn copy_to_clipboard(text: String) -> bool {
    let escaped = serde_json::to_string(&text).unwrap_or_else(|_| "\"\"".to_owned());
    let mut evaluation = dioxus::document::eval(&format!(
        r#"(async () => {{
            const value = {escaped};
            const fallback = () => {{
                const ta = document.createElement('textarea');
                ta.value = value;
                ta.style.position = 'fixed';
                ta.style.opacity = '0';
                document.body.appendChild(ta);
                ta.select();
                const success = document.execCommand('copy');
                document.body.removeChild(ta);
                return success;
            }};
            if (navigator.clipboard && navigator.clipboard.writeText) {{
                try {{ await navigator.clipboard.writeText(value); dioxus.send(true); }}
                catch (_) {{ dioxus.send(fallback()); }}
            }} else {{
                dioxus.send(fallback());
            }}
        }})()"#,
    ));
    evaluation.recv::<bool>().await.unwrap_or(false)
}

/// Case-insensitive substring match over one or more candidates — the app's
/// single "type to filter" rule. The command palette and [`SearchSelect`] both
/// use it, so filtering cannot mean two different things in two places.
///
/// An empty query matches everything, which is what lets a caller hand the
/// unfiltered list straight to it.
pub fn matches_query(query: &str, candidates: &[&str]) -> bool {
    let query = query.trim().to_ascii_lowercase();
    if query.is_empty() {
        return true;
    }
    candidates
        .iter()
        .any(|candidate| candidate.to_ascii_lowercase().contains(&query))
}

/// The entries of `options` matching `query`, in their original order.
pub fn matching(options: &[String], query: &str) -> Vec<String> {
    options
        .iter()
        .filter(|option| matches_query(query, &[option.as_str()]))
        .cloned()
        .collect()
}

/// A searchable select: a control showing the current value that opens a
/// filterable list.
///
/// A native `select` is right for a handful of fixed options (`Autosave`); it
/// is wrong for a list discovered at runtime that can run to dozens of entries
/// — the LLM endpoint's model ids, a LoRA directory. The interaction is deliberately
/// the command palette's: type to filter, arrows to move, Enter to pick, Escape
/// or Cancel to dismiss. One search idiom, not two.
///
/// The component stays dumb about *where* the options come from: a caller with a
/// loading or failed discovery renders its own spinner or error row instead of
/// mounting this.
#[component]
pub fn SearchSelect(
    #[props(default = "Choose a value".to_owned())] label: String,
    options: Vec<String>,
    value: String,
    /// Shown in the list when nothing matches — or when there are no options
    /// at all, which is how a caller says "nothing discovered yet".
    #[props(default)]
    empty_message: Option<String>,
    on_select: EventHandler<String>,
) -> Element {
    let mut open = use_signal(|| false);
    let mut query = use_signal(String::new);
    let mut highlight = use_signal(|| 0usize);
    let id = use_hook(|| format!("select-{}", uuid::Uuid::new_v4()));
    let list_id = format!("{id}-list");
    let filtered_count = matching(&options, &query.read()).len();
    let active_id = if filtered_count > 0 {
        format!(
            "{id}-option-{}",
            (*highlight.read()).min(filtered_count - 1)
        )
    } else {
        String::new()
    };
    let scroll_id = active_id.clone();
    use_effect(use_reactive(&scroll_id, move |id| {
        document::eval(&format!(
            "document.getElementById('{id}')?.scrollIntoView({{block:'nearest'}})"
        ));
    }));

    let matched = matching(&options, &query.read());
    let total = options.len();
    let count = matched.len();
    // The highlight can outlive the list it pointed into (a keystroke narrows
    // the matches), so it is clamped on every render rather than trusted.
    let index = (*highlight.read()).min(count.saturating_sub(1));
    let empty_message = empty_message.unwrap_or_else(|| "Nothing matches.".to_owned());

    let mut dismiss = move || {
        open.set(false);
        query.set(String::new());
        highlight.set(0);
    };

    rsx! {
        div { class: "relative min-w-0",
            button {
                class: "setting-control flex items-center justify-between gap-2 text-left",
                title: "{value}",
                aria_label: label.clone(),
                aria_haspopup: "dialog",
                aria_expanded: open(),
                onclick: move |_| {
                    if open() {
                        dismiss();
                    } else {
                        open.set(true);
                    }
                },
                span { class: "truncate font-mono text-[11px]", "{value}" }
                // The one chevron the icon set has, turned to point down.
                Icon { name: IconName::Arrow, class: "size-3 shrink-0 rotate-90 text-zinc-600" }
            }

            if open() {
                // Use the shared searchable-dialog behavior and focus contract.
                Dialog { title: label.clone(), on_close: move |_| dismiss(),
                div { class: "w-[min(420px,90vw)] overflow-hidden rounded-xl bg-[#17171d]",
                    h2 { class: "px-3.5 pt-3 text-sm font-semibold", "{label}" }
                    input {
                        aria_label: "Filter {label}",
                        role: "combobox",
                        aria_expanded: "true",
                        aria_controls: list_id.clone(),
                        aria_activedescendant: active_id,
                        aria_autocomplete: "list",
                        class: "w-full border-b border-white/[0.06] bg-transparent px-3.5 py-2.5 font-mono text-[11.5px] text-zinc-100 outline-none placeholder:text-zinc-700",
                        placeholder: "Search…",
                        autofocus: true,
                        autocomplete: "off",
                        spellcheck: "false",
                        oninput: move |event| {
                            query.set(event.value());
                            highlight.set(0);
                        },
                        onkeydown: move |event| {
                            match event.key() {
                                Key::Escape => {
                                    event.prevent_default();
                                    dismiss();
                                }
                                Key::ArrowDown => {
                                    event.prevent_default();
                                    highlight.set(index.saturating_add(1).min(count.saturating_sub(1)));
                                }
                                Key::ArrowUp => {
                                    event.prevent_default();
                                    highlight.set(index.saturating_sub(1));
                                }
                                Key::Enter => {
                                    event.prevent_default();
                                    if let Some(choice) = matched.get(index) {
                                        on_select.call(choice.clone());
                                        dismiss();
                                    }
                                }
                                _ => {}
                            }
                        },
                    }
                    div { id: list_id, role: "listbox", aria_label: label.clone(), class: "max-h-64 overflow-y-auto p-1.5",
                        if count == 0 {
                            p { class: "px-3 py-4 text-center text-[11px] text-zinc-600", "{empty_message}" }
                        }
                        for (slot , option) in matched.iter().enumerate() {
                            button {
                                id: "{id}-option-{slot}",
                                role: "option",
                                aria_selected: slot == index,
                                tabindex: "-1",
                                key: "{option}",
                                class: if slot == index { "flex w-full items-center gap-2 rounded-lg bg-violet-400/15 px-3 py-1.5 text-left" } else { "flex w-full items-center gap-2 rounded-lg px-3 py-1.5 text-left hover:bg-white/[0.04]" },
                                onclick: {
                                    let option = option.clone();
                                    move |_| {
                                        on_select.call(option.clone());
                                        dismiss();
                                    }
                                },
                                span { class: "min-w-0 flex-1 truncate font-mono text-[11.5px] text-zinc-100", "{option}" }
                                if option == &value {
                                    Icon { name: IconName::Check, class: "size-3 shrink-0 text-violet-300" }
                                }
                            }
                        }
                    }
                    p { class: "flex items-center justify-between gap-2 border-t border-white/[0.06] bg-white/[0.015] px-3.5 py-1.5 text-[9.5px] text-zinc-600",
                        span { if count == total { "{total} shown" } else { "{count} of {total}" } }
                        span { "↑↓ move · ↵ pick · esc close" }
                    }
                    button { class: "w-full px-3 py-2 text-xs", onclick: move |_| dismiss(), "Cancel" }
                }
                }
            }
        }
    }
}

/// Open the resolved path using the desktop's file manager/default application.
pub fn open_path(path: &std::path::Path) -> Result<(), String> {
    if !path.exists() {
        return Err(format!("Path does not exist: {}", path.display()));
    }
    let program = if cfg!(target_os = "macos") {
        "open"
    } else if cfg!(target_os = "windows") {
        "explorer"
    } else {
        "xdg-open"
    };
    let status = std::process::Command::new(program)
        .arg(path)
        .status()
        .map_err(|error| error.to_string())?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("Could not open {} ({status})", path.display()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn models() -> Vec<String> {
        vec![
            "prism-ml/bonsai-27b".to_owned(),
            "google/gemma-4-e4b".to_owned(),
            "qwen3-vl-4b".to_owned(),
        ]
    }

    #[test]
    fn matching_is_case_insensitive_and_keeps_the_source_order() {
        assert_eq!(matching(&models(), "GEMMA"), vec!["google/gemma-4-e4b"]);
    }

    #[test]
    fn matching_is_a_substring_not_a_prefix() {
        // Model ids lead with an organisation, so the useful part is often in
        // the middle — a prefix match would find almost nothing.
        assert_eq!(matching(&models(), "27b"), vec!["prism-ml/bonsai-27b"]);
    }

    #[test]
    fn an_empty_query_keeps_every_option() {
        assert_eq!(matching(&models(), ""), models());
        assert_eq!(matching(&models(), "   "), models());
    }

    #[test]
    fn a_query_is_trimmed_before_matching() {
        assert_eq!(matching(&models(), "  gemma  "), vec!["google/gemma-4-e4b"]);
    }

    #[test]
    fn a_query_matching_nothing_returns_nothing() {
        assert!(matching(&models(), "zzznope").is_empty());
    }

    #[test]
    fn candidates_beyond_the_first_are_searched_too() {
        // The palette matches an entry's hint as well as its label.
        assert!(matches_query("harbor", &["open mara", "lighthouse harbor"]));
        assert!(!matches_query("harbor", &["open mara"]));
    }
}
