use super::dialog::Dialog;
use crate::{commands, state::AppState};
use dioxus::desktop::{
    tao::event::{Event, WindowEvent},
    WindowCloseBehaviour,
};
use dioxus::prelude::*;
use storyfella_studio_core::{
    config::AppConfig,
    errors::{ErrorCode, ReportedError},
};

fn needs_close_confirmation(state: &AppState) -> bool {
    state.has_unsaved_changes || state.has_description_drafts() || state.operation_in_progress()
}

#[component]
pub fn CloseGuard(config: AppConfig, app_state: Signal<AppState>) -> Element {
    let mut confirm = use_signal(|| false);
    let mut error = use_signal(|| None::<String>);
    let desktop = dioxus::desktop::use_window();
    let event_window = desktop.clone();
    dioxus::desktop::use_wry_event_handler(move |event, _| {
        if let Event::WindowEvent {
            window_id,
            event: WindowEvent::CloseRequested,
            ..
        } = event
        {
            if *window_id != event_window.window.id() {
                return;
            }
            if needs_close_confirmation(&app_state.peek()) {
                event_window.set_close_behavior(WindowCloseBehaviour::WindowHides);
                confirm.set(true);
                let window = event_window.clone();
                spawn(async move {
                    window.window.set_visible(true);
                    window.window.set_focus();
                });
            } else {
                event_window.set_close_behavior(WindowCloseBehaviour::WindowCloses);
            }
        }
    });
    let menu_window = desktop.clone();
    dioxus::desktop::use_muda_event_handler(move |event| {
        if event.id.0 == super::QUIT_MENU_ID {
            if needs_close_confirmation(&app_state.peek()) {
                confirm.set(true);
            } else {
                menu_window.set_close_behavior(WindowCloseBehaviour::WindowCloses);
                menu_window.close();
            }
        }
    });
    let save_window = desktop.clone();
    let working = app_state.read().operation_in_progress();
    let title = if working {
        "Quit Studio?"
    } else {
        "Save before quitting?"
    };
    rsx! {
        if confirm() {
            Dialog { title, on_close: move |_| confirm.set(false),
                div { class: "studio-task-form",
                    h2 { class: "text-xl font-semibold", "{title}" }
                    if !working { p { class: "text-sm text-zinc-300", "Save your changes before quitting." } }
                    if working { p { role: "status", class: "text-sm text-amber-300", "Wait for the current operation to finish before saving." } }
                    if let Some(message) = error() { p { role: "alert", class: "studio-inline-error", "{message}" } }
                    footer { class: "flex flex-wrap justify-end gap-2",
                        button { class: "studio-button", autofocus: true, onclick: move |_| confirm.set(false), "Cancel" }
                        button { class: "studio-button studio-button-danger", onclick: move |_| { desktop.set_close_behavior(WindowCloseBehaviour::WindowCloses); desktop.close(); }, "Discard and quit" }
                        button { class: "studio-button studio-button-primary", disabled: working, onclick: move |_| {
                            let applied = app_state.write().apply_description_drafts().map_err(|message| ReportedError::plain(ErrorCode::AppCommandInvalid, message));
                            let result = applied.and_then(|_| {
                                    if app_state.read().project_path.is_none() {
                                        let Some(path) = rfd::FileDialog::new().set_title("Save new project").set_directory(&config.project_dir).add_filter("Storyfella project", &["db"]).set_file_name("Untitled.db").save_file() else { return Err(ReportedError::plain(ErrorCode::AppCommandInvalid, "Save cancelled.")); };
                                        let path = if path.extension().is_none() { path.with_extension("db") } else { path };
                                        commands::save_new_project(&path, app_state)
                                    } else { commands::save_project(app_state) }
                                });
                            match result {
                                Ok(()) => { save_window.set_close_behavior(WindowCloseBehaviour::WindowCloses); save_window.close(); }
                                Err(report) => error.set(Some(report.details)),
                            }
                        }, "Save and quit" }
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
    fn quitting_protects_source_drafts_and_active_operations() {
        let mut state = AppState::default();
        assert!(!needs_close_confirmation(&state));
        state.set_story_source("// unsaved writing".into());
        assert!(needs_close_confirmation(&state));
        let mut state = AppState::default();
        state.set_description_draft(uuid::Uuid::new_v4(), "Unfinished description".into());
        assert!(needs_close_confirmation(&state));
        let mut state = AppState::default();
        state.begin_operation();
        assert!(needs_close_confirmation(&state));
    }
}
