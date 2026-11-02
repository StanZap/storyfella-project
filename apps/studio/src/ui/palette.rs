//! Navigation and authoring actions. Visual production is reached through Atelier.
use super::{actions::ArtAction, components::matches_query, AppScreen};
use crate::state::AppState;
use dioxus::prelude::*;
use uuid::Uuid;

#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    Navigate(AppScreen),
    Task(ArtAction),
    Save,
    Undo,
    Redo,
    Open(Uuid),
}
#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    pub label: String,
    pub hint: String,
    pub action: Action,
    pub enabled: bool,
}

pub fn entries(state: &AppState, query: &str) -> Vec<Entry> {
    let available = state.project_path.is_some() && !state.operation_in_progress();
    let mut entries = Vec::new();
    for screen in AppScreen::ALL {
        if screen.available(state.project_path.is_some()) {
            entries.push(Entry {
                label: format!("Go to {}", screen.label()),
                hint: "Navigation".into(),
                action: Action::Navigate(screen),
                enabled: true,
            });
        }
    }
    for (label, action, enabled) in [
        (
            "New asset definition",
            Action::Task(ArtAction::Create(
                storyfella_studio_core::registry::ArtifactKind::Character,
            )),
            available,
        ),
        ("Save project", Action::Save, available),
        ("Undo", Action::Undo, state.can_undo()),
        ("Redo", Action::Redo, state.can_redo()),
    ] {
        entries.push(Entry {
            label: label.into(),
            hint: if enabled {
                String::new()
            } else if state.project_path.is_none() {
                "Open a project first".into()
            } else {
                "Unavailable during the current task or history is empty".into()
            },
            action,
            enabled,
        });
    }
    if let Some(item) = state
        .selected_artifact()
        .filter(|item| item.kind.supports_variants())
    {
        entries.push(Entry {
            label: "New variation definition".into(),
            hint: item.name.clone(),
            action: Action::Task(ArtAction::Variant(item.variant_of.unwrap_or(item.id))),
            enabled: available,
        });
    }
    for item in &state.registry.artifacts {
        entries.push(Entry {
            label: format!("Open {}", item.name),
            hint: item.kind.as_str().into(),
            action: Action::Open(item.id),
            enabled: true,
        });
    }
    entries.retain(|entry| matches_query(query, &[entry.label.as_str(), entry.hint.as_str()]));
    entries
}

#[component]
pub fn Palette(
    app_state: Signal<AppState>,
    mut open: Signal<bool>,
    onpick: EventHandler<Action>,
) -> Element {
    let mut query = use_signal(String::new);
    let mut selected = use_signal(|| 0usize);
    let found = entries(&app_state.read(), &query());
    let results_id = use_hook(|| format!("palette-results-{}", Uuid::new_v4()));
    let scroll_id = results_id.clone();
    use_effect(move || {
        let index = selected();
        document::eval(&format!(
            "document.getElementById('{scroll_id}-{index}')?.scrollIntoView({{block:'nearest'}});"
        ));
    });
    rsx! {
        super::dialog::Dialog {
            title: "Commands and navigation",
            on_close: move |_| open.set(false),
            form {
                class: "space-y-3",
                onsubmit: move |event| {
                event.prevent_default();
                    let found = entries(&app_state.peek(), &query.peek());
                    if let Some(entry) = found.get(*selected.peek()).filter(|entry| entry.enabled) {
                        onpick.call(entry.action.clone());
                        open.set(false);
                    }
                },
                super::inputs::SearchInput {
                    label: "Search commands and assets",
                    value: query(),
                    autofocus: true,
                    controls: results_id.clone(),
                    oninput: move |value| {
                        query.set(value);
                        selected.set(0);
                    },
                    oncancel: move |_| open.set(false),
                    onkeydown: move |event: KeyboardEvent| {
                        match event.key() {
                            Key::ArrowDown => {
                                event.prevent_default();
                                let count = entries(&app_state.peek(), &query.peek()).len();
                                let next = (*selected.peek() + 1).min(count.saturating_sub(1));
                                selected.set(next);
                            }
                            Key::ArrowUp => {
                                event.prevent_default();
                                let next = selected.peek().saturating_sub(1);
                                selected.set(next);
                            }
                            _ => {}
                        }
                    },
                }
                button { r#type: "submit", hidden: true, "Choose selected action" }
                div {
                    id: results_id.clone(),
                    class: "max-h-80 overflow-auto",
                    if found.is_empty() { p { role: "status", "Nothing matches." } }
                    for (index, entry) in found.into_iter().enumerate() {
                        button {
                            id: "{results_id}-{index}",
                            r#type: "button",
                            class: if index == selected() { "studio-button w-full bg-violet-400/15 text-left" } else { "studio-button w-full text-left" },
                            disabled: !entry.enabled,
                            onfocus: move |_| selected.set(index),
                            onclick: move |_| {
                                onpick.call(entry.action.clone());
                                open.set(false);
                            },
                            span { "{entry.label}" }
                            span { class: "ml-auto text-xs text-zinc-400", "{entry.hint}" }
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn palette_exposes_authoring_and_navigation_without_visual_production_commands() {
        let mut state = AppState::default();
        assert!(!entries(&state, "new asset")[0].enabled);
        state.project_path = Some("story.db".into());
        assert!(entries(&state, "new asset")[0].enabled);
        for query in [
            "regenerate",
            "modify",
            "attach adapter",
            "/create",
            "render",
            "import artwork",
        ] {
            assert!(entries(&state, query).is_empty(), "{query}");
        }
        assert!(entries(&state, "story")
            .iter()
            .any(|entry| matches!(entry.action, Action::Navigate(AppScreen::Writer))));
    }
}
