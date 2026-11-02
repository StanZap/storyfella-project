//! Catalog definitions are authoring operations; image production belongs to Atelier.
use super::{actions::ArtAction, dialog::Dialog, inputs::TextInput};
use crate::state::AppState;
use dioxus::prelude::*;
use storyfella_studio_core::{
    catalog::{CatalogEdit, KINDS},
    registry::ArtifactKind,
};

#[component]
pub fn ArtActionDialog(
    action: ArtAction,
    app_state: Signal<AppState>,
    onclose: EventHandler<()>,
) -> Element {
    let mut app_state = app_state;
    // The selector belongs only to new definitions. Variations resolve their
    // base's current kind at submission instead of retaining a stale copy.
    let initial = match action {
        ArtAction::Create(kind) => kind,
        ArtAction::Variant(_) => ArtifactKind::Character,
    };
    let mut kind = use_signal(|| initial);
    let mut name = use_signal(String::new);
    let name_id = use_hook(|| format!("new-asset-name-{}", uuid::Uuid::new_v4()));
    let mut description = use_signal(String::new);
    let mut error = use_signal(|| None::<String>);
    let (description_id, description_mount) =
        super::text_input::use_text_value(description(), None);
    let project = use_hook(|| app_state.peek().project_path.clone());
    let stale = project.is_none() || project != app_state.read().project_path;
    rsx! {
        Dialog {
            title: "New catalog entry",
            on_close: move |_| onclose.call(()),
            form {
                class: "space-y-4",
                onsubmit: move |event| {
                    event.prevent_default();
                    if stale { return; }
                    let (kind, variant_of) = match action {
                        ArtAction::Variant(id) => {
                            let base = app_state.read().registry.artifact(id).map(|item| item.kind);
                            let Some(kind) = base else {
                                error.set(Some("The base item no longer exists.".into()));
                                return;
                            };
                            (kind, Some(id))
                        }
                        ArtAction::Create(_) => (kind(), None),
                    };
                    let edit = CatalogEdit::Create {
                        kind, name: name(), description: description(), variant_of,
                    };
                    match app_state.write().edit_catalog(edit) {
                        Ok(_) => onclose.call(()),
                        Err(message) => error.set(Some(message)),
                    }
                },
                h2 { class: "text-lg font-semibold", "New catalog entry" }
                if matches!(action, ArtAction::Create(_)) {
                    label {
                        class: "block text-xs text-zinc-400", "Kind"
                        select {
                            class: "setting-control w-full",
                            value: kind().as_str(),
                            oninput: move |event| {
                                if let Ok(value) = event.value().parse::<ArtifactKind>() {
                                    kind.set(value);
                                }
                            },
                            for option in KINDS {
                                option {
                                    value: option.as_str(), selected: kind() == option,
                                    "{option}"
                                }
                            }
                        }
                    }
                }
                label { r#for: name_id.clone(), class: "block text-xs text-zinc-400", "Reference name" }
                TextInput {
                    id: Some(name_id.clone()), label: "Reference name", value: name(),
                    placeholder: "e.g. mara", autofocus: true,
                    oninput: move |value| name.set(value),
                }
                label { r#for: description_id.clone(), class: "block text-xs text-zinc-400", "Description" }
                textarea {
                    id: description_id, onmounted: description_mount,
                    class: "setting-control min-h-28 w-full",
                    oninput: move |event| description.set(event.value()),
                }
                if stale { p { role: "alert", "The open project changed. Close this form and try again." } }
                if let Some(message) = error() { p { role: "alert", class: "studio-inline-error", "{message}" } }
                div {
                    class: "flex justify-end gap-2",
                    button {
                        r#type: "button", class: "studio-button",
                        onclick: move |_| onclose.call(()), "Cancel"
                    }
                    button {
                        r#type: "submit", class: "studio-button studio-button-primary",
                        disabled: stale || name().trim().is_empty() || app_state.read().operation_in_progress(),
                        "Create catalog entry"
                    }
                }
            }
        }
    }
}
