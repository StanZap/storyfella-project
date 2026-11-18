//! Review and apply one coordinated writing proposal across the novel.
use crate::state::AppState;
use dioxus::prelude::*;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use storyfella_studio_core::{
    planner::PlannerBackend,
    workflows::{
        novel::{self, NovelProposal, NovelRequest},
        writing::WritingAction,
        Feedback, RunStore,
    },
};

#[component]
pub fn NovelAssistant(
    mut app_state: Signal<AppState>,
    backend: Signal<PlannerBackend>,
    action: Signal<WritingAction>,
    instructions: Signal<String>,
    preserve: Signal<bool>,
    mut busy: Signal<bool>,
    mut proposal: Signal<Option<NovelProposal>>,
    mut cancellation: Signal<Arc<AtomicBool>>,
) -> Element {
    let mut message = use_signal(|| None::<String>);
    let mut status = use_signal(String::new);
    use_drop(move || cancellation.peek().store(true, Ordering::Relaxed));
    let start = move |_| {
        if busy() || !backend.read().has_llm() {
            return;
        }
        let snapshot = {
            let state = app_state.read();
            state
                .project_path
                .clone()
                .ok_or_else(|| "Open a project first.".to_owned())
                .and_then(|project| {
                    state
                        .workspace_snapshot()
                        .map(|workspace| (project, workspace, state.registry.clone()))
                })
        };
        let (project, workspace, registry) = match snapshot {
            Ok(value) => value,
            Err(error) => {
                message.set(Some(error));
                return;
            }
        };
        let catalog_digest = match novel::catalog_digest(&registry) {
            Ok(value) => value,
            Err(error) => {
                message.set(Some(error.to_string()));
                return;
            }
        };
        let request = NovelRequest {
            project: project.clone(),
            workspace,
            action: action(),
            instructions: instructions(),
            preserve_facts: preserve(),
            catalog_digest,
        };
        let selected = backend.read().clone();
        let cancel = Arc::new(AtomicBool::new(false));
        cancellation.set(cancel.clone());
        busy.set(true);
        proposal.set(None);
        message.set(None);
        status.set("Writing a novel suggestion…".into());
        spawn(async move {
            let result = async {
                let store = RunStore::open(&project.with_extension("workflows.sqlite3"))?;
                let (send, mut receive) = tokio::sync::mpsc::unbounded_channel();
                let run = novel::run_with_backend(
                    request,
                    &selected,
                    &registry,
                    &store,
                    cancel.clone(),
                    move |event| {
                        let _ = send.send(event);
                    },
                );
                tokio::pin!(run);
                loop {
                    tokio::select! {
                        result = &mut run => break result,
                        Some(Feedback::Status(text)) = receive.recv() => status.set(text),
                    }
                }
            }
            .await;
            if !Arc::ptr_eq(&cancel, &cancellation.peek()) {
                return;
            }
            busy.set(false);
            if app_state.read().project_path.as_ref() != Some(&project) {
                return;
            }
            match result {
                Ok(Some(value)) => {
                    status.set("Review all proposed files before applying.".into());
                    proposal.set(Some(value));
                }
                Ok(None) => {
                    status.set(String::new());
                    message.set(Some("Cancelled. Your novel is unchanged.".into()));
                }
                Err(error) => {
                    status.set(String::new());
                    message.set(Some(error.to_string()));
                }
            }
        });
    };
    rsx! {
        div { class: "px-4 pb-4",
            if busy() {
                p { role: "status", class: "text-xs text-zinc-400", "{status}" }
                button { onclick: move |_| cancellation.read().store(true, Ordering::Relaxed), "Cancel" }
            } else if let Some(current) = proposal() {
                p { class: "mb-2 text-xs text-zinc-400", "Review {current.files.len()} changed files in the Writer before applying." }
                button { class: "rounded bg-violet-400/20 px-3 py-2 text-xs text-violet-100", onclick: move |_| {
                    let Some(current) = proposal() else { return; };
                    let result = app_state.write().apply_novel_proposal(&current);
                    match result {
                        Ok(()) => {
                            let mut state = app_state.write();
                            let active = state.active_source.as_deref().unwrap_or(&current.request.workspace.entry);
                            if !current.files.contains_key(active) {
                                if let Some(first) = current.files.keys().next() {
                                    state.active_source = (first != &current.request.workspace.entry).then(|| first.clone());
                                }
                            }
                            drop(state);
                            proposal.set(None);
                            message.set(Some("Applied as one undoable change. Save the project to persist it.".into()));
                        }
                        Err(error) => message.set(Some(error)),
                    }
                }, "Apply all changes" }
                button { class: "ml-2 px-3 py-2 text-xs text-zinc-400", onclick: move |_| proposal.set(None), "Discard" }
            } else {
                p { class: "mb-2 text-xs text-zinc-400", "Writing help can change included story files. Creating or deleting files is not yet supported." }
                button { class: "rounded bg-violet-400/20 px-3 py-2 text-xs text-violet-100", disabled: !backend.read().has_llm(), onclick: start, "Suggest changes to the novel" }
            }
            if let Some(error) = message() { p { role: "status", class: "mt-2 text-xs text-amber-300", "{error}" } }
        }
    }
}
