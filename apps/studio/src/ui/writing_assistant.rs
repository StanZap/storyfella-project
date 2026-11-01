//! A compact request composer and a suggestion-first review, beside the story.
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

use dioxus::prelude::*;
use storyfella_language::{Position, Span};
use storyfella_studio_core::{
    errors::{ErrorCode, ReportedError},
    registry::backend::CreativeBackend,
    workflows::{
        writing::{self, ProposalReview, WritingAction, WritingRequest, WritingScope},
        writing_review::WritingReview,
        Feedback, RunStore,
    },
};

use super::{
    components::{ErrorDetailsButton, IconButton, RecoveryNotice},
    icons::IconName,
};
use crate::state::AppState;

#[derive(serde::Deserialize)]
struct EditorSelection {
    text: String,
    positions: [Position; 2],
}

async fn capture_request(
    app_state: Signal<AppState>,
    scope: &str,
) -> anyhow::Result<WritingRequest> {
    let mut evaluation = document::eval(
        r#"
        const el = document.getElementById('svs-writer');
        const text = el?.value ?? '';
        const pos = n => { const lines = text.slice(0,n).split('\n'); return {line:lines.length-1, character:lines.at(-1).length}; };
        dioxus.send({text, positions:[pos(el?.selectionStart ?? 0), pos(el?.selectionEnd ?? 0)]});
        await dioxus.recv();
    "#,
    );
    // Keep the desktop query alive until Rust consumes the snapshot.
    let selected = tokio::time::timeout(
        std::time::Duration::from_secs(3),
        evaluation.recv::<EditorSelection>(),
    )
    .await;
    let _ = evaluation.send(());
    let selected =
        selected.map_err(|_| anyhow::anyhow!("The editor did not respond. Try again."))??;
    let state = app_state.read();
    anyhow::ensure!(
        selected.text == state.story.source().replace("\r\n", "\n"),
        "The editor is still receiving changes. Try again."
    );
    let source = storyfella_language::Source::new(state.story.source());
    let start = source
        .offset(selected.positions[0])
        .ok_or_else(|| anyhow::anyhow!("Select the passage again."))?;
    let end = source
        .offset(selected.positions[1])
        .ok_or_else(|| anyhow::anyhow!("Select the passage again."))?;
    let scope = match scope {
        "file" => WritingScope::File,
        "selection" => WritingScope::Selection(Span::new(start, end)),
        "scene" => WritingScope::Scene(start),
        _ => WritingScope::Beat(start),
    };
    WritingRequest::capture(
        state
            .project_path
            .as_deref()
            .ok_or_else(|| anyhow::anyhow!("Open a story first."))?,
        state
            .story
            .path()
            .ok_or_else(|| anyhow::anyhow!("Save the story first."))?,
        state.story.source(),
        scope,
        &state.registry,
    )
}

#[component]
pub fn WritingAssistant(
    mut app_state: Signal<AppState>,
    backend: Signal<CreativeBackend>,
    open: Signal<bool>,
    onclose: EventHandler<()>,
) -> Element {
    let mut screen = use_context::<Signal<super::AppScreen>>();
    let mut controller = use_context::<super::writing_review::ReviewController>();
    let text_configured = backend.read().planner_config().is_some();
    let mut scope = use_signal(|| "beat".to_owned());
    let mut action = use_signal(WritingAction::default);
    let mut instructions = use_signal(String::new);
    let mut preserve = use_signal(|| true);
    let mut request = use_signal(|| None::<WritingRequest>);
    let mut request_open = use_signal(|| true);
    let mut busy = controller.busy;
    let mut message = use_signal(|| None::<String>);
    let mut failure = use_signal(|| None::<ReportedError>);
    let mut streamed = use_signal(String::new);
    let mut phase = use_signal(|| "Writing a suggestion…".to_owned());
    let mut cancellation = use_signal(|| Arc::new(AtomicBool::new(false)));
    let mut interrupted = use_signal(|| false);
    let fields = super::writing_session::SessionFields {
        scope,
        action,
        instructions,
        preserve,
        request,
        request_open,
        streamed,
        phase,
        cancellation,
        interrupted,
        message,
        failure,
    };
    let mut recovery =
        super::writing_session::use_writing_session(app_state, controller, fields, open);
    let (instructions_id, instructions_mount) =
        super::text_input::use_text_value(instructions(), None);
    use_drop(move || cancellation.peek().store(true, Ordering::Relaxed));
    let mut had_review = use_signal(|| None::<std::path::PathBuf>);
    use_effect(move || {
        let reviewing = controller.review.read().is_some();
        let owner = app_state.read().project_path.clone();
        if had_review.peek().is_some()
            && *had_review.peek() == owner
            && !reviewing
            && !interrupted()
        {
            request_open.set(true);
            request.set(None);
            failure.set(None);
            message.set(Some(
                "Review finished. You can start another request.".into(),
            ));
            controller.error.set(None);
        }
        had_review.set(if reviewing { owner } else { None });
    });

    let mut start = move |retry: bool| {
        if busy() || recovery.conflict.read().is_some() {
            return;
        }
        let Some(config) = backend.read().planner_config() else {
            return;
        };
        let selected_scope = scope();
        let selected_action = action();
        let selected_instructions = instructions();
        let selected_preserve = preserve();
        let previous = request();
        let existing_review = (controller.review)();
        let previous_request_open = request_open();
        busy.set(true);
        interrupted.set(false);
        message.set(None);
        failure.set(None);
        streamed.set(String::new());
        phase.set("Writing a suggestion…".into());
        let cancel = Arc::new(AtomicBool::new(false));
        cancellation.set(cancel.clone());
        let backend = backend.read().clone();
        spawn(async move {
            let captured = if let Some(review) = &existing_review {
                if let Some(error) = controller.owner_error(&app_state.read()) {
                    message.set(Some(error));
                    busy.set(false);
                    return;
                }
                if selected_scope == "file" {
                    WritingRequest::capture(
                        &review.proposal.request.project,
                        &review.proposal.request.source_path,
                        &review.draft,
                        WritingScope::File,
                        &app_state.read().registry,
                    )
                } else {
                    review.retry_request(&controller.changes.read(), &app_state.read().registry)
                }
            } else if retry {
                previous.ok_or_else(|| anyhow::anyhow!("Choose a passage first."))
            } else {
                capture_request(app_state, &selected_scope).await
            }
            .map(|mut captured| {
                captured.action = selected_action;
                captured.instructions = selected_instructions;
                captured.preserve_facts = selected_preserve;
                captured
            });
            if !Arc::ptr_eq(&cancel, &cancellation.peek()) {
                return;
            }
            let captured = match captured {
                Ok(captured) => captured,
                Err(error) => {
                    message.set(Some(error.to_string()));
                    busy.set(false);
                    return;
                }
            };
            request.set(Some(captured.clone()));
            let owner = (captured.project.clone(), captured.source_path.clone());
            let registry = app_state.read().registry.clone();
            let result = async {
                let store = RunStore::open(&captured.project.with_extension("workflows.sqlite3"))?;
                let readiness = backend.ensure_planner_ready();
                tokio::pin!(readiness);
                loop {
                    tokio::select! {
                        result = &mut readiness => { result?; break; }
                        _ = tokio::time::sleep(std::time::Duration::from_millis(25)) => {
                            if cancel.load(Ordering::Relaxed) { return Ok(None); }
                        }
                    }
                }
                let (send, mut receive) = tokio::sync::mpsc::unbounded_channel();
                let run = writing::run(captured, config, &registry, &store, cancel.clone(), move |event| { let _ = send.send(event); });
                tokio::pin!(run);
                loop {
                    tokio::select! {
                        result = &mut run => break result,
                        Some(event) = receive.recv() => {
                            if Arc::ptr_eq(&cancel, &cancellation.peek()) && !cancel.load(Ordering::Relaxed) {
                                match event {
                                    Feedback::Text(text) => streamed.write().push_str(&text),
                                    Feedback::Status(status) => { phase.set(status); streamed.set(String::new()); }
                                    Feedback::Event(_) => {}
                                }
                            }
                        },
                    }
                }
            }.await;
            // Project switching cancels the old task. Its late completion must
            // never replace the newly opened project's restored workspace.
            if !Arc::ptr_eq(&cancel, &cancellation.peek())
                || app_state.read().project_path.as_ref() != Some(&owner.0)
                || app_state.read().story.path() != Some(owner.1.as_path())
            {
                return;
            }
            let retrying_review = existing_review.is_some();
            match result {
                Ok(Some(value)) => match if let Some(previous) = existing_review {
                    previous.retry_review(value, &app_state.read().registry)
                } else {
                    WritingReview::new(value, &app_state.read().registry)
                } {
                    Ok(review) if !review.finished() => {
                        controller.begin(review);
                        request_open.set(false);
                    }
                    Ok(_) => {
                        message.set(Some(
                            "No changes suggested. Try more specific instructions.".into(),
                        ));
                        request_open.set(true);
                    }
                    Err(error) => {
                        message.set(Some(error.to_string()));
                        request_open.set(previous_request_open || !retrying_review);
                    }
                },
                Ok(None) => {
                    message.set(Some("Cancelled. Your story is unchanged.".into()));
                    request_open.set(true);
                }
                Err(error) => {
                    failure.set(Some(
                        ReportedError::plain(ErrorCode::WritingProposalFailed, error.to_string())
                            .with_context("operation", "Writing suggestion")
                            .log(),
                    ));
                    request_open.set(false);
                }
            }
            busy.set(false);
        });
    };

    let mut last_retry = use_signal(|| 0);
    use_effect(move || {
        let retry = (controller.retry)();
        if retry != *last_retry.peek() {
            last_retry.set(retry);
            start(true);
        }
    });

    let current = (controller.review)();
    let remaining = controller.changes.read().len();
    let selected = (controller.selected)().min(remaining.saturating_sub(1));
    let marked = controller.marked.read().len();
    let ready = (controller.ready)();
    let mismatch = controller.owner_error(&app_state.read());
    let mut review = ProposalReview::default();
    if let Some(value) = &current {
        let state = app_state.read();
        match value
            .acceptance(None)
            .and_then(|proposal| proposal.review(&state.registry))
        {
            Ok(value) => review = value,
            Err(_) => review.has_errors = true,
        }
    }
    let can_keep_selection = current.as_ref().is_some_and(|value| {
        let changes = controller.changes.read();
        let marked = controller.marked.read();
        let proposal = if marked.is_empty() {
            changes
                .get(selected)
                .map(|change| value.acceptance(Some(change)))
        } else {
            marked
                .iter()
                .map(|index| changes.get(*index).cloned())
                .collect::<Option<Vec<_>>>()
                .map(|changes| value.acceptance_group(&changes))
        };
        proposal.is_some_and(|proposal| {
            proposal
                .and_then(|proposal| proposal.review(&app_state.read().registry))
                .is_ok_and(|review| !review.has_errors)
        })
    });
    let reviewing = current.is_some();
    let can_retry = text_configured
        && mismatch.is_none()
        && recovery.conflict.read().is_none()
        && (!reviewing || !review.has_errors)
        && if current.is_some() {
            ready
        } else {
            let state = app_state.read();
            request.read().as_ref().is_some_and(|request| {
                state.project_path.as_ref() == Some(&request.project)
                    && state.story.path() == Some(request.source_path.as_path())
                    && state.story.source() == request.source
            })
        };

    rsx! {
        aside { class: "studio-writing-panel", aria_label: "Writing help",
            header { class: "studio-writing-header",
                h2 { class: "text-sm font-semibold", "Writing help" }
                div { class: "ml-auto flex items-center gap-1",
                    if busy() { IconButton { label: "Cancel writing request", icon: IconName::Close, onclick: move |_| cancellation.read().store(true, Ordering::Relaxed) } }
                    if request().is_some() && !busy() {
                        IconButton { label: if request_open() { "Hide request" } else { "Edit request" }, icon: IconName::Edit, active: request_open(), expanded: request_open(), onclick: move |_| request_open.toggle() }
                        IconButton { label: "Try again", icon: IconName::Refresh, tooltip: if reviewing && review.has_errors { "Correct the proposed source errors before generating another version" } else { "Generate another version using this request" }, disabled: !can_retry, onclick: move |_| {
                            if reviewing { controller.command("retry", "", false, false); }
                            else { start(true); }
                        } }
                    }
                    IconButton { label: "Close Writing help", icon: IconName::Close, onclick: move |_| onclose.call(()) }
                }
            }
            if let Some(saved) = (recovery.conflict)() {
                div { class: "px-4 pt-3",
                    RecoveryNotice { title: "Your story changed while you were away", message: "Your saved writing session is safe. Restore it as an unsaved draft to continue reviewing, or keep the current story and discard that session.",
                        IconButton { label: "Restore writing session", icon: IconName::Undo, show_label: true, tooltip: "Restore the saved writing workspace as an editable draft", onclick: move |_| {
                            app_state.write().set_story_source(saved.draft.clone());
                            fields.restore(controller, Some(&saved));
                            recovery.conflict.set(None);
                        } }
                        IconButton { label: "Discard saved session", icon: IconName::Close, show_label: true, tooltip: "Keep the current story and discard the saved writing workspace", onclick: move |_| recovery.conflict.set(None) }
                    }
                }
            }
            if let Some(error) = (recovery.error)() {
                p { role: "alert", class: "px-4 pt-3 text-xs leading-5 text-rose-300", "{error}" }
            }
            if !text_configured {
                div { class: "px-4 pt-3",
                    RecoveryNotice { title: "Connect a text model", message: "Choose a text provider to start writing together. Your story stays here.",
                        IconButton { label: "Open Settings", icon: IconName::Settings, show_label: true, onclick: move |_| screen.set(super::AppScreen::Settings) }
                    }
                }
            }
            div { class: "studio-writing-request", hidden: !request_open() || busy(),
                div { class: "grid grid-cols-2 gap-2",
                    select { aria_label: "Writing scope", disabled: current.is_some(), value: scope(), oninput: move |event| scope.set(event.value()),
                        option { value: "selection", "Selection" } option { value: "beat", "Current beat" } option { value: "scene", "Current scene" } option { value: "file", "Entire file" }
                    }
                    select { aria_label: "Writing action", value: if action() == WritingAction::Expand { "expand" } else { "rewrite" }, oninput: move |event| action.set(if event.value() == "expand" { WritingAction::Expand } else { WritingAction::Rewrite }),
                        option { value: "rewrite", "Rewrite" } option { value: "expand", "Expand" }
                    }
                }
                if current.is_some() { p { class: "text-xs leading-5 text-zinc-400", "Finish reviewing this passage before choosing another scope." } }
                textarea { id: instructions_id, onmounted: instructions_mount, aria_label: "Writing instructions", class: "studio-writing-prompt", rows: 3, placeholder: "How should this passage change?", oninput: move |event| instructions.set(event.value()) }
                div { class: "flex flex-wrap items-center justify-between gap-2",
                    label { class: "flex items-center gap-2 text-xs text-zinc-400",
                        input { r#type: "checkbox", class: "accent-violet-500", checked: !preserve(), onchange: move |_| preserve.toggle() }
                        "Allow new details"
                    }
                    IconButton { label: if action() == WritingAction::Expand { "Suggest expansion" } else { "Suggest rewrite" }, icon: IconName::Sparkles, show_label: true, primary: true, tooltip: "Generate a suggestion for the chosen passage", disabled: !text_configured || recovery.conflict.read().is_some(), onclick: move |_| start(false) }
                }
            }
            if let Some(captured) = request() {
                if !captured.instructions.trim().is_empty() {
                    div { class: "studio-writing-brief", hidden: request_open() && !busy(),
                        p { class: "text-sm leading-6 text-zinc-200", "{captured.instructions}" }
                    }
                }
            }
            div { class: "studio-writing-reading",
                if interrupted() && !busy() {
                    RecoveryNotice { title: "Writing was interrupted", message: "Your request and the response received so far were restored. Try again when you're ready; no new request has been sent." }
                    if !streamed.read().is_empty() {
                        details { class: "studio-writing-notes", summary { "Recovered partial response" }
                            pre { class: "studio-writing-source", "{streamed}" }
                        }
                    }
                }
                if let Some(report) = failure() {
                    RecoveryNotice { title: "Couldn't create a suggestion", message: "Your story and request are unchanged. Check the text model or try again.", alert: true,
                        IconButton { label: "Model settings", icon: IconName::Settings, show_label: true, onclick: move |_| screen.set(super::AppScreen::Settings) }
                        ErrorDetailsButton { error: report }
                    }
                }
                if let Some(text) = message() { p { role: "status", class: "mb-4 text-sm leading-6 text-zinc-400", "{text}" } }
                if busy() {
                    div { class: "mb-4 flex items-center gap-3",
                        p { role: "status", class: "text-sm text-zinc-400", "{phase}" }
                    }
                    pre { class: "studio-writing-source", "{streamed}" }
                } else if current.is_some() {
                    if !ready && controller.error.read().is_none() { p { role: "status", class: "text-xs text-zinc-400", "Opening review…" } }
                    if remaining > 0 {
                        div { class: "studio-writing-selection",
                            div { class: "flex items-center justify-between gap-2",
                                p { class: "text-xs text-zinc-400", if marked > 0 { "{marked} selected" } else { "Change {selected + 1} of {remaining}" } }
                                div { class: "flex gap-1",
                                    IconButton { label: "Previous change", icon: IconName::ChevronUp, tooltip: "Go to the previous change · Shift+F7", disabled: !ready || selected == 0, onclick: move |_| controller.command("previous", "", false, false) }
                                    IconButton { label: "Next change", icon: IconName::ChevronDown, tooltip: "Go to the next change · F7", disabled: !ready || selected + 1 >= remaining, onclick: move |_| controller.command("next", "", false, false) }
                                }
                            }
                            div { class: "mt-2 flex gap-2",
                                IconButton { label: if marked > 0 { "Keep selected" } else { "Keep change" }, icon: IconName::Check, show_label: true, tooltip: if ready && mismatch.is_none() && !can_keep_selection { "Correct these edits or select their related changes before keeping them".into() } else if marked > 0 { format!("Keep {marked} selected changes in the draft") } else { "Keep this change in the draft".into() }, disabled: !ready || mismatch.is_some() || !can_keep_selection, onclick: move |_| controller.command("action", "keep", false, marked > 0) }
                                IconButton { label: if marked > 0 { "Reset selected" } else { "Reset change" }, icon: IconName::Undo, tooltip: if marked > 0 { format!("Restore {marked} selected changes to the draft text") } else { "Restore this change to the draft text".into() }, disabled: !ready || mismatch.is_some(), onclick: move |_| controller.command("action", "discard", false, marked > 0) }
                            }
                        }
                    }
                    if !review.diagnostics.is_empty() {
                        details { class: "studio-writing-notes",
                            summary { if review.has_errors { "Source issues to resolve ({review.diagnostics.len()})" } else { "Notes for this suggestion ({review.diagnostics.len()})" } }
                            for diagnostic in &review.diagnostics { p { class: "mt-2 text-xs leading-5 text-zinc-400", "{diagnostic.message}" } }
                        }
                    }
                } else if failure().is_none() && message().is_none() && !interrupted() && request.read().is_none() && recovery.conflict.read().is_none() {
                    p { class: "text-sm leading-6 text-zinc-500", "Choose a passage, the current beat or scene, or the entire file." }
                }
            }
            if current.is_some() && !busy() {
                footer { class: "studio-writing-actions",
                    if let Some(text) = mismatch.as_ref() { p { role: "status", class: "mb-3 text-xs leading-5 text-zinc-400", "{text}" } }
                    if let Some(error) = (controller.error)() { p { role: "status", class: "mb-3 text-xs leading-5 text-rose-300", "{error}" } }
                    else if review.has_errors { p { role: "status", class: "mb-3 text-xs text-rose-300", "Fix the proposed text before keeping all changes. Your draft is unchanged by pending edits." } }
                    div { class: "flex flex-wrap items-center gap-2",
                        IconButton { label: if remaining == 0 { "Done" } else { "Keep all" }, icon: IconName::Check, show_label: true, primary: true, tooltip: "Keep all remaining changes and finish reviewing", disabled: !ready || mismatch.is_some() || review.has_errors, onclick: move |_| controller.command("action", "keep", true, false) }
                        IconButton { label: "Discard all", icon: IconName::Close, show_label: true, tooltip: "Discard all pending changes; keep previously accepted edits", disabled: mismatch.is_some() || ready && remaining == 0, onclick: move |_| {
                            if ready { controller.command("action", "discard", true, false); }
                            else { controller.review.set(None); }
                        } }
                    }
                }
            }
        }
    }
}
