use crate::PlaybackModel;
use dioxus::prelude::*;
use storyfella_compiler::SceneId;
use storyfella_runtime::Command;

#[derive(Clone, Copy, PartialEq)]
pub(crate) enum ExtraPanel {
    Options,
    Artwork,
    Source,
}

pub(crate) fn close_panel(mut panel: Signal<Option<ExtraPanel>>) {
    panel.set(None);
    let _ = document::eval("document.querySelector('.sf-options-trigger')?.focus()");
}

pub(crate) fn focus_stage() {
    let _ = document::eval("document.querySelector('.sf-stage')?.focus()");
}

/// Button Enter/Space stays native rather than bubbling into pause/advance.
pub(crate) fn control_keys(event: KeyboardEvent) {
    if event.key() == Key::Enter || event.key() == Key::Character(" ".into()) {
        event.stop_propagation();
    }
}

pub(crate) fn save_state(mut model: Signal<PlaybackModel>) {
    let Some(saved) = model.read().session.as_ref().map(|s| s.runtime.save()) else {
        return;
    };
    let window = dioxus::desktop::window().window.clone();
    // Native modal dialogs must not re-enter the desktop event loop while
    // Dioxus is dispatching a click or holding a render/signal borrow.
    spawn(async move {
        if let Some(file) = rfd::AsyncFileDialog::new()
            .set_parent(window.as_ref())
            .set_file_name("story-state.json")
            .save_file()
            .await
        {
            model.write().error = serde_json::to_vec_pretty(&saved)
                .map_err(|e| e.to_string())
                .and_then(|bytes| std::fs::write(file.path(), bytes).map_err(|e| e.to_string()))
                .err();
        }
    });
}

pub(crate) fn load_state(mut model: Signal<PlaybackModel>) {
    let window = dioxus::desktop::window().window.clone();
    spawn(async move {
        if let Some(file) = rfd::AsyncFileDialog::new()
            .set_parent(window.as_ref())
            .add_filter("Saved state", &["json"])
            .pick_file()
            .await
        {
            let result = std::fs::read(file.path())
                .map_err(|e| e.to_string())
                .and_then(|bytes| serde_json::from_slice(&bytes).map_err(|e| e.to_string()));
            match result {
                Ok(saved) => model.write().restore(saved),
                Err(error) => model.write().error = Some(error),
            }
        }
    });
}

#[component]
pub(crate) fn PlaybackOptions(
    mut model: Signal<PlaybackModel>,
    mut panel: Signal<Option<ExtraPanel>>,
    on_save: EventHandler<()>,
    on_load: EventHandler<()>,
    #[props(default)] on_open: Option<EventHandler<()>>,
) -> Element {
    let snapshot = model.read();
    rsx! {
        nav { class: "sf-option-list", aria_label: "Playback actions",
            if let Some(on_open) = on_open {
                button { onclick: move |_| { close_panel(panel); on_open.call(()); }, "Open story…" }
            }
            if let Some(session) = snapshot.session.as_ref() {
                button { onclick: move |_| { model.write().dispatch(Command::Restart); close_panel(panel); }, "Restart story" }
                button { onclick: move |_| { model.write().dispatch(Command::End); close_panel(panel); }, "End story" }
                label { class: "sf-jump",
                    span { "Jump to scene" }
                    select { aria_label: "Jump to scene", value: "", disabled: session.runtime.state().paused,
                        onchange: move |event| {
                            if !event.value().is_empty() {
                                model.write().dispatch(Command::Jump { scene: SceneId(event.value()) });
                                close_panel(panel);
                            }
                        },
                        option { value: "", "Choose…" }
                        for scene in &session.runtime.plan().scenes {
                            option { value: "{scene.id.0}", "{scene.id.0}" }
                        }
                    }
                }
                div { class: "sf-option-divider" }
                button { onclick: move |_| { model.write().bookmark(); close_panel(panel); }, "Bookmark" }
                button { disabled: snapshot.bookmark.is_none(), onclick: move |_| { model.write().restore_bookmark(); close_panel(panel); }, "Restore bookmark" }
                button { onclick: move |_| { close_panel(panel); on_save.call(()); }, "Save state…" }
                button { onclick: move |_| { close_panel(panel); on_load.call(()); }, "Load state…" }
                div { class: "sf-option-divider" }
                button { onclick: move |_| panel.set(Some(ExtraPanel::Artwork)), "Artwork" }
                button { onclick: move |_| panel.set(Some(ExtraPanel::Source)), "Source snapshot" }
            }
            p { class: "sf-key-help", "Right arrow to continue · Space to pause" }
        }
    }
}
