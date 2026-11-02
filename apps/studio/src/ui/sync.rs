//! The refresh owner stays mounted on every screen. Reads run off the UI thread;
//! errors and decisions stay local without stealing focus from authored text.
use dioxus::prelude::*;
use storyfella_studio_core::{
    catalog::sync::Resolution,
    persistence::{ProjectDb, StoreError},
    story::StoryFileError,
};

use super::{components::IconButton, dialog::Dialog, icons::IconName};
use crate::state::AppState;

#[component]
pub fn CatalogSync(mut app_state: Signal<AppState>) -> Element {
    let mut error = use_signal(|| None::<String>);
    let mut reviewing = use_signal(|| false);
    let mut retry = use_signal(|| 0u64);
    let mut refresh_now = use_signal(|| true);
    let mut source_recovery = use_signal(|| false);
    let mut review_source = use_signal(|| false);
    let mut recovering = use_signal(|| false);
    let mut preserved_source = use_signal(|| None::<std::path::PathBuf>);
    use_effect(move || {
        let _ = retry();
        refresh_now.set(true);
    });
    use_hook(move || {
        spawn(async move {
            loop {
                let path = app_state.peek().project_path.clone();
                if let Some(path) = path.filter(|_| !app_state.peek().operation_in_progress()) {
                    let captured_path = path.clone();
                    let result = tokio::task::spawn_blocking(move || {
                        ProjectDb::open_existing(&path).and_then(|db| db.load())
                    })
                    .await;
                    if app_state.peek().project_path.as_ref() == Some(&captured_path) {
                        match result {
                            Ok(Ok(stored)) => {
                                let mut next = app_state.peek().clone();
                                let result = next.refresh_catalog(stored, Resolution::Check);
                                let message =
                                    result.err().filter(|_| next.catalog_conflict.is_none());
                                if *app_state.peek() != next {
                                    app_state.set(next);
                                }
                                if *error.peek() != message {
                                    error.set(message);
                                }
                                if *source_recovery.peek() {
                                    source_recovery.set(false);
                                }
                            }
                            result => {
                                let needs_recovery = matches!(
                                    &result,
                                    Ok(Err(StoreError::Story(StoryFileError::Conflict { .. })))
                                );
                                if *source_recovery.peek() != needs_recovery {
                                    source_recovery.set(needs_recovery);
                                }
                                let message = match result {
                                    Ok(Err(error)) => error.to_string(),
                                    Err(error) => error.to_string(),
                                    Ok(Ok(_)) => unreachable!(),
                                };
                                if error.peek().as_ref() != Some(&message) {
                                    error.set(Some(message));
                                }
                            }
                        }
                    }
                } else if app_state.peek().project_path.is_none() && error.peek().is_some() {
                    error.set(None);
                }
                refresh_now.set(false);
                // A short wait keeps Retry responsive without opening another
                // read concurrently or queuing reads behind generation.
                for _ in 0..20 {
                    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                    if *refresh_now.peek() {
                        break;
                    }
                }
            }
        });
    });
    let conflict = app_state.read().catalog_conflict.clone();
    rsx! {
        if conflict.is_some() || error.read().is_some() {
            div { class: "flex shrink-0 items-center gap-3 border-b border-amber-400/20 bg-amber-400/5 px-4 py-2 text-sm",
                span { role: "status", class: "flex-1 text-amber-200",
                    if conflict.is_some() { "Catalog changes need your review. Your edits are retained." }
                    else { "Could not refresh assets. Your work is retained." }
                }
                if conflict.is_some() {
                    button { class: "studio-button", onclick: move |_| reviewing.set(true), "Review changes" }
                }
                if source_recovery() {
                    button { class: "studio-button", onclick: move |_| review_source.set(true), "Recover interrupted save" }
                }
                IconButton { icon: IconName::Refresh, label: "Retry catalog refresh", onclick: move |_| { let next = *retry.peek() + 1; retry.set(next); } }
            }
        }
        if review_source() && source_recovery() {
            Dialog { title: "Recover interrupted save", on_close: move |_| review_source.set(false),
                h2 { class: "text-lg font-semibold", "Recover interrupted save" }
                p { class: "text-sm text-zinc-400", "The catalog was saved, but its story file changed before the matching text could be written. Keep a separate copy of the current file, then finish the interrupted save. Your unsaved editor text stays intact." }
                button { class: "studio-button studio-button-primary", disabled: recovering(), onclick: move |_| {
                    let Some(path) = app_state.peek().project_path.clone() else { return };
                    recovering.set(true);
                    spawn(async move {
                        let expected_path = path.clone();
                        let result = tokio::task::spawn_blocking(move || ProjectDb::open_existing(path).and_then(|db| db.preserve_source_and_recover())).await;
                        recovering.set(false);
                        if app_state.peek().project_path.as_ref() != Some(&expected_path) { return; }
                        match result {
                            Ok(Ok(path)) => { preserved_source.set(path); source_recovery.set(false); review_source.set(false); refresh_now.set(true); }
                            Ok(Err(problem)) => error.set(Some(problem.to_string())),
                            Err(problem) => error.set(Some(problem.to_string())),
                        }
                    });
                }, "Keep file copy and finish save" }
                button { class: "studio-button", disabled: recovering(), onclick: move |_| review_source.set(false), "Cancel" }
            }
        }
        if reviewing() {
            if let Some(conflict) = conflict {
                Dialog { title: "Catalog changes", on_close: move |_| reviewing.set(false),
                    h2 { class: "text-lg font-semibold", "Catalog changes" }
                    p { class: "text-sm text-zinc-400", "Another editor saved changes to the same items. Nothing has been overwritten. Your story text stays in Studio. Your choice also applies to conflicting description drafts." }
                    ul { class: "max-h-64 space-y-3 overflow-auto text-sm text-zinc-300",
                        for field in &conflict.fields { li {
                            p { class: "font-medium", "{field.label}" }
                            if !conflict.requires_reload {
                                div { class: "grid grid-cols-2 gap-3",
                                    div { p { class: "text-xs text-zinc-400", "Yours" } pre { class: "max-h-32 overflow-auto whitespace-pre-wrap break-words text-xs", "{field.local}" } }
                                    div { p { class: "text-xs text-zinc-400", "Saved" } pre { class: "max-h-32 overflow-auto whitespace-pre-wrap break-words text-xs", "{field.saved}" } }
                                }
                            }
                        } }
                    }
                    if conflict.requires_reload {
                        p { class: "text-sm text-amber-200", "Reloading the saved catalog discards unsaved catalog changes and project undo history. Story text and description drafts for retained items stay in Studio." }
                        button { class: "studio-button", disabled: app_state.read().operation_in_progress(), onclick: move |_| {
                            match app_state.write().use_saved_catalog() {
                                Ok(()) => reviewing.set(false), Err(message) => error.set(Some(message)),
                            }
                        }, "Reload saved catalog" }
                    } else {
                        p { class: "text-sm text-zinc-400", "Choose which overlapping fields to keep. Independent changes and artwork history from both editors are retained." }
                        div { class: "flex flex-wrap gap-2",
                            for (label, resolution) in [("Keep my conflicting edits", Resolution::Local), ("Use saved conflicting edits", Resolution::Saved)] {
                                button { class: "studio-button", disabled: app_state.read().operation_in_progress(), onclick: move |_| {
                                    match app_state.write().resolve_catalog_conflict(resolution) {
                                        Ok(()) => reviewing.set(false), Err(message) => error.set(Some(message)),
                                    }
                                }, "{label}" }
                            }
                        }
                    }
                    button { class: "studio-button", onclick: move |_| reviewing.set(false), "Decide later" }
                }
            }
        }
        if let Some(message) = error() {
            details { class: "shrink-0 px-4 py-1 text-xs text-zinc-400", summary { "Refresh details" } pre { class: "max-h-24 overflow-auto whitespace-pre-wrap", "{message}" } }
        }
        if let Some(path) = preserved_source() {
            div { role: "status", class: "flex items-center gap-2 px-4 py-2 text-xs text-zinc-300",
                span { class: "min-w-0 flex-1 break-words", "Source copy saved at {path.display()}" }
                IconButton { icon: IconName::Close, label: "Dismiss source copy location", onclick: move |_| preserved_source.set(None) }
            }
        }
    }
}
