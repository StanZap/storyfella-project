//! Source-owned asset metadata. Visual production settings live in Atelier.
use dioxus::prelude::*;
use storyfella_studio_core::{catalog::CatalogEdit, registry::Artifact};

use super::editable::{EditableMultiline, EditableText};
use crate::state::AppState;

#[component]
pub fn Inspector(
    artifact: Artifact,
    disabled: bool,
    #[props(default)] rename_requested: bool,
    #[props(default)] on_rename_started: Option<EventHandler<()>>,
) -> Element {
    let mut app_state = use_context::<Signal<AppState>>();
    let target = artifact.id;
    let draft = app_state
        .read()
        .description_draft(target)
        .map(str::to_owned);
    rsx! {
        aside { class: "w-72 shrink-0 overflow-y-auto border-l border-white/10 p-4 space-y-5",
            if storyfella_studio_core::catalog::KINDS.contains(&artifact.kind) {
                EditableText {
                    key: "name-{target}", label: "Reference name", value: artifact.name, disabled,
                    edit_requested: rename_requested, on_edit_started: on_rename_started,
                    onconfirm: move |name: String| {
                        let description = app_state.read().registry.artifact(target)
                            .map(|item| item.description.clone())
                            .ok_or_else(|| "This catalog item no longer exists.".to_owned())?;
                        app_state.write().edit_catalog(CatalogEdit::Describe {
                            id: target, name, description,
                        }).map(|_| ())
                    },
                }
            }
            EditableMultiline {
                key: "description-{target}", label: "Description", value: artifact.description,
                disabled, draft, placeholder: "No description yet",
                oninput: move |value| app_state.write().set_description_draft(target, value),
                onconfirm: move |_| app_state.write().apply_description_draft(target),
                oncancel: move |_| app_state.write().discard_description_draft(target),
            }
        }
    }
}
