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

fn has_unsent_request(state: &AppState, request: &str) -> bool {
    if request.trim().is_empty() {
        return false;
    }
    // Searches are transient navigation; explicit commands and assistant
    // instructions are drafts that must not disappear on an ordinary quit.
    if request.trim_start().starts_with('/') {
        return true;
    }
    let actions = super::palette::quick_actions(state, request);
    actions.entries.is_empty() && actions.prerequisite.is_none()
}

fn needs_close_confirmation(state: &AppState, request: &str) -> bool {
    state.has_unsaved_changes
        || state.has_description_drafts()
        || state.operation_in_progress()
        || has_unsent_request(state, request)
}

#[component]
pub fn CloseGuard(
    config: AppConfig,
    app_state: Signal<AppState>,
    composer: Signal<String>,
) -> Element {
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
            if needs_close_confirmation(&app_state.peek(), &composer.peek()) {
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
            if needs_close_confirmation(&app_state.peek(), &composer.peek()) {
                confirm.set(true);
            } else {
                menu_window.set_close_behavior(WindowCloseBehaviour::WindowCloses);
                menu_window.close();
            }
        }
    });
    let save_window = desktop.clone();
    let working = app_state.read().operation_in_progress();
    let unsent = has_unsent_request(&app_state.read(), &composer.read());
    let title = if working || unsent {
        "Quit Studio?"
    } else {
        "Save before quitting?"
    };
    rsx! {
        if confirm() {
            Dialog { title, on_close: move |_| confirm.set(false),
                div { class: "studio-task-form",
                    h2 { class: "text-xl font-semibold", "{title}" }
                    if !working && !unsent { p { class: "text-sm text-zinc-300", "Save your changes before quitting." } }
                    if working { p { role: "status", class: "text-sm text-amber-300", "Wait for the artwork operation to finish before saving." } }
                    if unsent { p { role: "status", class: "text-sm text-amber-300", "You have an unsent request. Cancel to submit or clear it." } }
                    if let Some(message) = error() { p { role: "alert", class: "studio-inline-error", "{message}" } }
                    footer { class: "flex flex-wrap justify-end gap-2",
                        button { class: "studio-button", autofocus: true, onclick: move |_| confirm.set(false), "Cancel" }
                        button { class: "studio-button studio-button-danger", onclick: move |_| { desktop.set_close_behavior(WindowCloseBehaviour::WindowCloses); desktop.close(); }, "Discard and quit" }
                        button { class: "studio-button studio-button-primary", disabled: working || unsent, onclick: move |_| {
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
    fn action_searches_do_not_block_quit_but_requests_and_commands_do() {
        let state = AppState::default();
        for search in ["", "  ", "edit"] {
            assert!(!has_unsent_request(&state, search), "{search}");
            assert!(!needs_close_confirmation(&state, search), "{search}");
        }
        let mut state = state;
        state.project_path = Some("test-project.db".into());
        assert!(!needs_close_confirmation(&state, "new story"));
        for request in [
            "Give the harbor golden evening light",
            "/create",
            "/modify harbor_dock make it blue",
        ] {
            assert!(has_unsent_request(&state, request), "{request}");
            assert!(needs_close_confirmation(&state, request), "{request}");
        }
    }

    #[test]
    fn searches_still_protect_project_changes() {
        let mut state = AppState::default();
        state.has_unsaved_changes = true;
        assert!(needs_close_confirmation(&state, "edit"));
        state.has_unsaved_changes = false;
        state.set_description_draft(uuid::Uuid::new_v4(), "An unfinished description".into());
        assert!(needs_close_confirmation(&state, "edit"));
        let mut state = AppState::default();
        state.begin_operation();
        assert!(needs_close_confirmation(&state, "edit"));
    }
}
