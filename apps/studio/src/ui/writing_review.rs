//! CodeMirror renders the diff. This bridge keeps all draft mutations in Rust.
use super::writer::highlight_span;
use crate::state::AppState;
use dioxus::prelude::*;
use serde::{Deserialize, Serialize};
use storyfella_language::{analyze, Span};
use storyfella_studio_core::{
    authoring::AssetCatalog,
    workflows::writing_review::{ReviewChange, WritingReview},
};

#[derive(Clone, Copy, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewMode {
    #[default]
    Inline,
    SideBySide,
}

#[derive(Clone)]
pub struct EditorBridge {
    session: String,
    evaluator: document::Eval,
}

#[derive(Clone, Copy)]
pub struct ReviewController {
    pub review: Signal<Option<WritingReview>>,
    pub mode: Signal<ReviewMode>,
    pub error: Signal<Option<String>>,
    pub busy: Signal<bool>,
    pub changes: Signal<Vec<ReviewChange>>,
    pub selected: Signal<usize>,
    pub marked: Signal<Vec<usize>>,
    pub sequence: Signal<u64>,
    pub ready: Signal<bool>,
    pub retry: Signal<u64>,
    pub bridge: Signal<Option<EditorBridge>>,
}

impl ReviewController {
    pub fn owner_error(self, state: &AppState) -> Option<String> {
        let review = self.review.read();
        let review = review.as_ref()?;
        let Some((project, path)) = state.project_path.as_deref().zip(state.story.path()) else {
            return Some("Open this suggestion's story to continue reviewing.".into());
        };
        review
            .check_owner(project, path, state.story.source())
            .err()
            .map(|e| e.to_string())
    }

    pub fn begin(mut self, review: WritingReview) {
        self.changes.set(Vec::new());
        self.selected.set(0);
        self.marked.set(Vec::new());
        self.sequence.set(0);
        self.ready.set(false);
        self.error.set(None);
        self.review.set(Some(review));
    }

    pub fn command(mut self, command: &str, action: &str, all: bool, group: bool) {
        if (self.busy)() {
            return;
        }
        let result = self.bridge.read().as_ref().map(|bridge| {
            bridge.evaluator.send(
                serde_json::json!({"command":command, "action":action, "all":all, "group":group}),
            )
        });
        if let Some(Err(error)) = result {
            self.error
                .set(Some(format!("The review editor did not respond: {error}")));
        }
    }

    fn receive(mut self, mut state: Signal<AppState>, event: EditorEvent) {
        let Some(mut review) = (self.review)() else {
            return;
        };
        if event.session != review.proposal.run_id {
            return;
        }
        if event.kind == "error" {
            self.error.set(Some(event.message));
            self.ready.set(false);
            self.bridge.set(None);
            return;
        }
        if event.sequence < (self.sequence)() {
            return;
        }
        if event.sequence != (self.sequence)() {
            self.sequence.set(event.sequence);
        }
        if !(self.ready)() {
            self.ready.set(true);
        }
        if *self.changes.peek() != event.changes {
            self.changes.set(event.changes.clone());
        }
        if *self.selected.peek() != event.selected {
            self.selected.set(event.selected);
        }
        if *self.marked.peek() != event.marked {
            self.marked.set(event.marked.clone());
        }
        if event.kind == "selection" || event.kind == "state" {
            return;
        }
        // Every action includes the editor's latest text and sequence. An older
        // render can never accept a stale copy of newer native keystrokes.
        review.edit(&event.text);
        if event.kind == "action" {
            let result = (|| -> anyhow::Result<()> {
                if let Some(error) = self.owner_error(&state.read()) {
                    anyhow::bail!(error);
                }
                let group = event
                    .marked
                    .iter()
                    .map(|index| {
                        event
                            .changes
                            .get(*index)
                            .cloned()
                            .ok_or_else(|| anyhow::anyhow!("Select the changes again."))
                    })
                    .collect::<anyhow::Result<Vec<_>>>()?;
                let change = if event.all || event.group {
                    None
                } else {
                    Some(
                        event
                            .changes
                            .get(event.selected)
                            .ok_or_else(|| anyhow::anyhow!("Select a change first."))?,
                    )
                };
                if event.action == "keep" {
                    if !review.finished() {
                        let proposal = if event.group {
                            review.acceptance_group(&group)?
                        } else {
                            review.acceptance(change)?
                        };
                        state.write().apply_writing_proposal(&proposal)
                            .map_err(|error| anyhow::anyhow!("These edits need correction or related changes kept together. {error}"))?;
                        review.draft = state.read().story.source().to_owned();
                    }
                } else if event.action == "discard" {
                    if event.group {
                        review.discard_group(&group)?;
                    } else {
                        review.discard(change)?;
                    }
                } else {
                    anyhow::bail!("Unknown review action");
                }
                Ok(())
            })();
            match result {
                Ok(()) => {
                    self.marked.set(Vec::new());
                    self.error.set(None);
                    if review.finished() {
                        self.review.set(None);
                        return;
                    }
                }
                Err(error) => self.error.set(Some(error.to_string())),
            }
        } else {
            self.error.set(None);
        }
        if self.review.peek().as_ref() != Some(&review) {
            self.review.set(Some(review));
        }
        if event.kind == "retry" {
            self.retry += 1;
        }
    }
}

#[derive(Deserialize)]
struct EditorEvent {
    kind: String,
    session: String,
    #[serde(default)]
    sequence: u64,
    #[serde(default)]
    text: String,
    #[serde(default)]
    changes: Vec<ReviewChange>,
    #[serde(default)]
    selected: usize,
    #[serde(default)]
    marked: Vec<usize>,
    #[serde(default)]
    action: String,
    #[serde(default)]
    all: bool,
    #[serde(default)]
    group: bool,
    #[serde(default)]
    message: String,
}

#[derive(Clone, PartialEq, Serialize)]
struct TokenStyle {
    from: usize,
    to: usize,
    class: &'static str,
}

#[derive(Clone, PartialEq, Serialize)]
struct EditorConfig {
    session: String,
    sequence: u64,
    selected: usize,
    marked: Vec<usize>,
    mode: ReviewMode,
    draft: String,
    proposed: String,
    disabled: bool,
    draft_tokens: Vec<TokenStyle>,
    proposed_tokens: Vec<TokenStyle>,
}

fn config(controller: ReviewController, state: &AppState) -> Option<EditorConfig> {
    let review = (controller.review)()?;
    let catalog = AssetCatalog::from_registry(&state.registry);
    let styles = |source: &str| {
        let analysis = analyze(source, &catalog.assets);
        let mut offset = 0;
        highlight_span(&analysis, Span::new(0, source.len()))
            .into_iter()
            .filter_map(|piece| {
                let from = offset;
                offset += piece.text.encode_utf16().count();
                (!piece.class.is_empty()).then_some(TokenStyle {
                    from,
                    to: offset,
                    class: piece.class,
                })
            })
            .collect()
    };
    let draft = review.draft.replace("\r\n", "\n");
    let proposed = review.proposed.replace("\r\n", "\n");
    Some(EditorConfig {
        session: review.proposal.run_id.clone(),
        selected: (controller.selected)(),
        marked: (controller.marked)(),
        sequence: (controller.sequence)(),
        mode: (controller.mode)(),
        disabled: (controller.busy)() || controller.owner_error(state).is_some(),
        draft_tokens: styles(&draft),
        proposed_tokens: styles(&proposed),
        draft,
        proposed,
    })
}

#[component]
pub fn WritingReviewEditor(app_state: Signal<AppState>) -> Element {
    let mut controller = use_context::<ReviewController>();
    let local_bridge = use_signal(|| None::<document::Eval>);
    let initial = config(controller, &app_state.read());
    let Some(initial) = initial else {
        return rsx! {};
    };
    let session = initial.session.clone();
    let mount_session = session.clone();
    let stale = controller.owner_error(&app_state.read());
    let mut local = local_bridge;
    use_effect(move || {
        if let Some(config) = config(controller, &app_state.read()) {
            if let Some(bridge) = controller.bridge.read().as_ref() {
                if bridge.session == config.session {
                    let _ = bridge.evaluator.send(config);
                }
            }
        }
    });
    use_drop(move || {
        if let Some(evaluator) = *local_bridge.peek() {
            let _ = evaluator.send(serde_json::json!({"close":true}));
        }
        if controller
            .bridge
            .peek()
            .as_ref()
            .is_some_and(|bridge| bridge.session == session)
        {
            controller.bridge.set(None);
        }
    });
    rsx! {
        document::Script { src: asset!("/assets/writing-diff.js") }
        div { class: "studio-review-workspace",
            if let Some(error) = stale {
                div { class: "studio-review-status", role: "status",
                    p { "{error}" }
                    button { class: "studio-writing-link", title: "Return to draft", onclick: move |_| controller.review.set(None), "Return to draft" }
                }
            }
            if (controller.mode)() == ReviewMode::SideBySide {
                div { class: "studio-diff-headings", span { "Draft" } span { "Suggested · editable" } }
            }
            div { id: "studio-writing-diff", class: "studio-diff-host", aria_label: "Story change review",
                onmounted: move |_| {
                    let mut evaluator = document::eval(r#"
                        const initial = await dioxus.recv();
                        let editor;
                        try {
                            const deadline = Date.now() + 8000;
                            while (!window.StoryfellaDiff) {
                                if (Date.now() > deadline) throw new Error("The diff editor could not load. Reopen Writing help.");
                                await new Promise(resolve => setTimeout(resolve, 25));
                            }
                            editor = window.StoryfellaDiff.mount(document.getElementById("studio-writing-diff"), initial, event => dioxus.send(event));
                            while (true) {
                                const update = await dioxus.recv();
                                if (update.close) break;
                                editor.update(update);
                            }
                        } catch (error) {
                            dioxus.send({kind:"error",session:initial.session,message:String(error)});
                            await dioxus.recv();
                        } finally { editor?.destroy(); }
                    "#);
                    let _ = evaluator.send(initial.clone());
                    local.set(Some(evaluator));
                    controller.bridge.set(Some(EditorBridge {session:mount_session.clone(), evaluator}));
                    spawn(async move {
                        while let Ok(event) = evaluator.recv::<EditorEvent>().await { controller.receive(app_state, event); }
                    });
                }
            }
        }
    }
}
