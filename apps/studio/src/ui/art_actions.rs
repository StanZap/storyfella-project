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
    #[props(default)] oncreated: Option<EventHandler<uuid::Uuid>>,
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
    let is_variation = matches!(action, ArtAction::Variant(_));
    let title = if is_variation {
        "New variation"
    } else {
        "New asset"
    };
    rsx! {
        Dialog {
            title,
            on_close: move |_| onclose.call(()),
            form {
                class: "studio-task-form",
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
                        Ok(id) => { if let Some(oncreated) = oncreated { oncreated.call(id); } onclose.call(()); },
                        Err(message) => error.set(Some(message)),
                    }
                },
                header { class: "flex items-center gap-3",
                    h2 { class: "flex-1 text-lg font-semibold", "{title}" }
                    super::components::IconButton { label: "Close", icon: super::icons::IconName::Close, onclick: move |_| onclose.call(()) }
                }
                if matches!(action, ArtAction::Create(_)) {
                    label {
                        class: "studio-field", span { "Kind" }
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
                div { class: "studio-field",
                label { r#for: name_id.clone(), "Reference name" }
                TextInput {
                    id: Some(name_id.clone()), label: "Reference name", value: name(),
                    placeholder: if is_variation { "" } else { "e.g. mara" }, autofocus: true,
                    oninput: move |value| name.set(value),
                }
                }
                label { r#for: description_id.clone(), class: "studio-field", span { "Description" }
                textarea {
                    id: description_id, onmounted: description_mount,
                    class: "setting-control min-h-28 w-full",
                    oninput: move |event| description.set(event.value()),
                }
                }
                if stale { p { role: "alert", "The open project changed. Close this form and try again." } }
                if let Some(message) = error() { p { role: "alert", class: "studio-inline-error", "{message}" } }
                div {
                    class: "flex justify-end gap-2",
                    button {
                        r#type: "button", class: "studio-button", title: "Cancel",
                        onclick: move |_| onclose.call(()), "Cancel"
                    }
                    button {
                        r#type: "submit", class: "studio-button studio-button-primary",
                        title: if is_variation { "Create variation" } else { "Create asset" },
                        disabled: stale || name().trim().is_empty() || app_state.read().operation_in_progress(),
                        if is_variation { "Create variation" } else { "Create asset" }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[component]
    fn VariationDialogProbe() -> Element {
        let app_state = use_signal(AppState::default);
        rsx! { ArtActionDialog { action: ArtAction::Variant(uuid::Uuid::nil()), app_state, onclose: move |_| {} } }
    }

    #[test]
    fn variation_dialog_is_named_for_the_action() {
        let mut dom = VirtualDom::new(VariationDialogProbe);
        let rendered = format!("{:?}", dom.rebuild_to_vec());
        assert!(rendered.contains("New variation"));
        assert!(!rendered.contains("New asset"));
    }
}
