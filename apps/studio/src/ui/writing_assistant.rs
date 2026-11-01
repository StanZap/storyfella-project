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
        writing::{
            self, ProposalReview, WritingAction, WritingProposal, WritingRequest, WritingScope,
        },
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

fn action_label(action: WritingAction) -> &'static str {
    match action {
        WritingAction::Rewrite => "Rewrite",
        WritingAction::Expand => "Expand",
    }
}

#[component]
pub fn WritingAssistant(
    mut app_state: Signal<AppState>,
    backend: Signal<CreativeBackend>,
    onclose: EventHandler<()>,
) -> Element {
    let mut screen = use_context::<Signal<super::AppScreen>>();
    let text_configured = backend.read().planner_config().is_some();
    let mut scope = use_signal(|| "beat".to_owned());
    let mut action = use_signal(WritingAction::default);
    let mut instructions = use_signal(String::new);
    let mut preserve = use_signal(|| true);
    let mut request = use_signal(|| None::<WritingRequest>);
    let mut proposal = use_signal(|| None::<WritingProposal>);
    let mut request_open = use_signal(|| true);
    let mut show_original = use_signal(|| false);
    let mut busy = use_signal(|| false);
    let mut message = use_signal(|| None::<String>);
    let mut failure = use_signal(|| None::<ReportedError>);
    let mut streamed = use_signal(String::new);
    let mut phase = use_signal(|| "Writing a suggestion…".to_owned());
    let mut cancellation = use_signal(|| Arc::new(AtomicBool::new(false)));
    let (instructions_id, instructions_mount) =
        super::text_input::use_text_value(instructions(), None);
    use_drop(move || cancellation.peek().store(true, Ordering::Relaxed));

    let mut start = move |retry: bool| {
        if busy() {
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
        busy.set(true);
        message.set(None);
        failure.set(None);
        streamed.set(String::new());
        phase.set("Writing a suggestion…".into());
        let cancel = Arc::new(AtomicBool::new(false));
        cancellation.set(cancel.clone());
        let backend = backend.read().clone();
        spawn(async move {
            let captured = if retry {
                previous.ok_or_else(|| anyhow::anyhow!("Choose a passage first."))
            } else {
                capture_request(app_state, &selected_scope)
                    .await
                    .map(|mut captured| {
                        captured.action = selected_action;
                        captured.instructions = selected_instructions;
                        captured.preserve_facts = selected_preserve;
                        captured
                    })
            };
            let captured = match captured {
                Ok(captured) => captured,
                Err(error) => {
                    message.set(Some(error.to_string()));
                    busy.set(false);
                    return;
                }
            };
            request.set(Some(captured.clone()));
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
                let run = writing::run(captured, config, &registry, &store, cancel, move |event| { let _ = send.send(event); });
                tokio::pin!(run);
                loop {
                    tokio::select! {
                        result = &mut run => break result,
                        Some(event) = receive.recv() => match event {
                            Feedback::Text(text) => streamed.write().push_str(&text),
                            Feedback::Status(status) => { phase.set(status); streamed.set(String::new()); }
                            Feedback::Event(_) => {}
                        },
                    }
                }
            }.await;
            match result {
                Ok(Some(value)) => {
                    proposal.set(Some(value));
                    request_open.set(false);
                    show_original.set(false);
                }
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

    let current = proposal();
    let mut mismatch = None;
    let mut review = ProposalReview::default();
    if let Some(value) = &current {
        let state = app_state.read();
        mismatch = match state.project_path.as_deref().zip(state.story.path()) {
            Some((project, path)) => value.apply(project, path, state.story.source()).err().map(|_| "The story has changed since this request. Your suggestion is kept; edit the request to work from the current draft.".to_owned()),
            None => Some("Open this suggestion's story before accepting it.".into()),
        };
        match value.review(&state.registry) {
            Ok(value) => review = value,
            Err(error) => mismatch = Some(error.to_string()),
        }
    }

    rsx! {
        aside { class: "studio-writing-panel", aria_label: "Writing help",
            header { class: "studio-writing-header",
                h2 { class: "text-sm font-semibold", "Writing help" }
                div { class: "ml-auto flex items-center gap-1",
                    if busy() { button { class: "studio-writing-link", onclick: move |_| cancellation.read().store(true, Ordering::Relaxed), "Cancel" } }
                    if request().is_some() && !busy() {
                        button { class: "studio-writing-link", aria_expanded: request_open(), onclick: move |_| request_open.toggle(), if request_open() { "Hide request" } else { "Edit request" } }
                    }
                    IconButton { label: "Close Writing help", icon: IconName::Close, onclick: move |_| onclose.call(()) }
                }
            }
            if !text_configured {
                div { class: "px-4 pt-3",
                    RecoveryNotice { title: "Connect a text model", message: "Choose a text provider to start writing together. Your story stays here.",
                        button { class: "studio-button", onclick: move |_| screen.set(super::AppScreen::Settings), "Open Settings" }
                    }
                }
            }
            div { class: "studio-writing-request", hidden: !request_open() || busy(),
                div { class: "grid grid-cols-2 gap-2",
                    select { aria_label: "Writing scope", value: scope(), oninput: move |event| scope.set(event.value()),
                        option { value: "selection", "Selection" } option { value: "beat", "Current beat" } option { value: "scene", "Current scene" }
                    }
                    select { aria_label: "Writing action", value: if action() == WritingAction::Expand { "expand" } else { "rewrite" }, oninput: move |event| action.set(if event.value() == "expand" { WritingAction::Expand } else { WritingAction::Rewrite }),
                        option { value: "rewrite", "Rewrite" } option { value: "expand", "Expand" }
                    }
                }
                textarea { id: instructions_id, onmounted: instructions_mount, aria_label: "Writing instructions", class: "studio-writing-prompt", rows: 3, placeholder: "How should this passage change?", oninput: move |event| instructions.set(event.value()) }
                div { class: "flex flex-wrap items-center justify-between gap-2",
                    label { class: "flex items-center gap-2 text-xs text-zinc-400",
                        input { r#type: "checkbox", class: "accent-violet-500", checked: !preserve(), onchange: move |_| preserve.toggle() }
                        "Allow new details"
                    }
                    button { class: "studio-button studio-button-primary", disabled: !text_configured, onclick: move |_| start(false), if action() == WritingAction::Expand { "Suggest expansion" } else { "Suggest rewrite" } }
                }
            }
            if let Some(captured) = request() {
                div { class: "studio-writing-brief", hidden: request_open() && !busy(),
                    span { class: "text-xs font-medium text-zinc-400", "{action_label(captured.action)}" }
                    if !captured.instructions.trim().is_empty() { p { class: "mt-1 text-sm leading-6 text-zinc-200", "{captured.instructions}" } }
                }
            }
            div { class: "studio-writing-reading",
                if let Some(report) = failure() {
                    RecoveryNotice { title: "Couldn't create a suggestion", message: "Your story and request are unchanged. Check the text model or try again.", alert: true,
                        button { class: "studio-button", disabled: busy() || !text_configured, onclick: move |_| start(true), "Try again" }
                        button { class: "studio-writing-link", onclick: move |_| screen.set(super::AppScreen::Settings), "Model settings" }
                        ErrorDetailsButton { error: report }
                    }
                }
                if let Some(text) = message() { p { role: "status", class: "mb-4 text-sm leading-6 text-zinc-400", "{text}" } }
                if busy() {
                    div { class: "mb-4 flex items-center gap-3",
                        p { role: "status", class: "text-sm text-zinc-400", "{phase}" }
                    }
                    pre { class: "studio-writing-source", "{streamed}" }
                } else if let Some(value) = current.as_ref() {
                    div { class: "studio-writing-review-nav", aria_label: "Compare suggestion",
                        button { aria_pressed: !show_original(), class: if !show_original() { "is-selected" } else { "" }, onclick: move |_| show_original.set(false), "Suggestion" }
                        button { aria_pressed: show_original(), class: if show_original() { "is-selected" } else { "" }, onclick: move |_| show_original.set(true), "Original" }
                        div { class: "ml-auto",
                            IconButton { label: "Another version", icon: IconName::Refresh, disabled: !text_configured, onclick: move |_| start(true) }
                        }
                    }
                    pre { class: "studio-writing-source", role: "region", aria_label: if show_original() { "Original passage" } else { "Suggested passage" }, if show_original() { "{value.request.original()}" } else { "{value.replacement}" } }
                    if !review.diagnostics.is_empty() {
                        details { class: "studio-writing-notes",
                            summary { if review.has_errors { "Source issues to resolve ({review.diagnostics.len()})" } else { "Notes for this suggestion ({review.diagnostics.len()})" } }
                            for diagnostic in &review.diagnostics { p { class: "mt-2 text-xs leading-5 text-zinc-400", "{diagnostic.message}" } }
                        }
                    }
                } else if failure().is_none() && message().is_none() {
                    p { class: "text-sm leading-6 text-zinc-500", "Select a passage or place the cursor in a beat or scene. Suggestions appear here for review." }
                }
            }
            if current.is_some() && !busy() {
                footer { class: "studio-writing-actions",
                    if let Some(text) = mismatch.as_ref() { p { role: "status", class: "mb-3 text-xs leading-5 text-zinc-400", "{text}" } }
                    if review.has_errors { p { role: "status", class: "mb-3 text-xs text-rose-300", "Story validation changed. This suggestion is kept, but cannot be applied safely." } }
                    div { class: "flex items-center gap-2",
                        button { class: "studio-button studio-button-primary flex-1", disabled: mismatch.is_some() || review.has_errors, onclick: move |_| {
                            if let Some(value) = proposal() {
                                match app_state.write().apply_writing_proposal(&value) {
                                    Ok(()) => { proposal.set(None); message.set(None); failure.set(None); request.set(None); request_open.set(true); onclose.call(()); }
                                    Err(error) => message.set(Some(error)),
                                }
                            }
                        }, "Accept suggestion" }
                        button { class: "studio-button", onclick: move |_| { proposal.set(None); failure.set(None); message.set(None); request_open.set(true); }, "Discard" }
                    }
                }
            }
        }
    }
}
