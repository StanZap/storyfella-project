use super::{
    components::{ErrorDetailsButton, IconButton},
    icons::IconName,
};
use crate::{commands, state::AppState};
use dioxus::prelude::*;
use storyfella_studio_core::{
    atelier::{self, HandoffReply, Mailbox, OpenRequest},
    errors::{ErrorCode, ReportedError},
};

#[component]
pub fn OpenInAtelier(
    target: Option<uuid::Uuid>,
    #[props(default = true)] show_label: bool,
) -> Element {
    let mut state = use_context::<Signal<AppState>>();
    let mut busy = use_signal(|| false);
    let mut error = use_signal(|| None::<ReportedError>);
    let shared = state.read().project_path.as_ref().is_some_and(|path| {
        path.file_name().is_some_and(|name| name == "story.db")
            && path.parent().is_some_and(|parent| {
                parent
                    .join(storyfella_studio_core::project::MANIFEST)
                    .is_file()
            })
    });
    rsx! {
        div { class: "flex flex-wrap items-center gap-2",
            IconButton { icon: IconName::Image, label: if busy() { "Opening Atelier…" } else { "Open in Atelier" },
                show_label, disabled: busy() || target.is_none() || !shared || state.read().operation_in_progress(),
                tooltip: if !shared { "Copy this story to a project folder from Projects first" } else { "Save story and open this item in Atelier" },
                onclick: move |_| {
                    let Some(id) = target else { return };
                    let result = state.write().apply_description_drafts();
                    if let Err(message) = result { error.set(Some(ReportedError::plain(ErrorCode::StoreSave, message))); return; }
                    if let Err(problem) = commands::save_project(state) { error.set(Some(problem)); return; }
                    let project = state.peek().project_path.clone();
                    let source = state.peek().story.source().to_owned();
                    let Some(project) = project else { return };
                    busy.set(true); error.set(None);
                    spawn(async move {
                        let result = tokio::task::spawn_blocking(move || -> anyhow::Result<OpenRequest> {
                            let request = OpenRequest::prepare(&project, &source, id)?;
                            atelier::send_and_launch(&request)?;
                            Ok(request)
                        }).await;
                        let result = match result {
                            Ok(Ok(request)) => {
                                let mailbox = Mailbox::new(&atelier::data_dir());
                                let mut outcome = Err("Atelier did not respond. Check that the rebuilt app is running, then retry.".to_owned());
                                for _ in 0..300 {
                                    match mailbox.reply(request.id) {
                                        Ok(Some(HandoffReply::Opened)) => { outcome = Ok(()); break; }
                                        Ok(Some(HandoffReply::Failed(message))) => { outcome = Err(message); break; }
                                        Err(problem) => { outcome = Err(format!("{problem:#}")); break; }
                                        Ok(None) => tokio::time::sleep(std::time::Duration::from_millis(100)).await,
                                    }
                                }
                                if outcome.is_err() { mailbox.cancel(request.id); }
                                outcome
                            }
                            Ok(Err(problem)) => Err(format!("{problem:#}")),
                            Err(problem) => Err(problem.to_string()),
                        };
                        busy.set(false);
                        if let Err(message) = result { error.set(Some(ReportedError::plain(ErrorCode::AppCommandInvalid, message))); }
                    });
                }
            }
            if let Some(problem) = error() {
                span { role: "alert", class: "text-xs text-amber-200", "Could not open Atelier" }
                ErrorDetailsButton { error: problem }
            }
        }
    }
}
