//! The assistant column docked at Write's right: the project's saved
//! conversation, and a prompt that asks the writing workflow for an edit to
//! a passage. A suggestion arrives as a card; reviewing it turns the editor
//! into a read-only diff whose changes are kept or reset one by one, and the
//! story changes only when the review ends.
//!
//! Asking again while a review is open refines it: the changes kept so far
//! are applied, and the same passage is asked for again with both requests.
use std::{
    collections::HashSet,
    path::PathBuf,
    sync::{atomic::AtomicBool, Arc},
    time::{Duration, Instant},
};

use design_system::{
    kits::{
        chat::{
            AssistantMessage, ToolActivity, ToolCall, ToolStatus, Transcript, TranscriptState,
            UserMessage,
        },
        code::{Diff, DiffView, DiffViewEvent, HunkState, LineKind},
    },
    ButtonKind, Glyph, PromptBar, TextArea, TextAreaEvent, Theme, TypeScale, Typography,
};
use gpui_kit::{prelude::*, *};
use storyfella_studio_core::{
    errors::{ErrorCode, ReportedError},
    planner::PlannerBackend,
    workflows::{
        assistant::{Answer, Asked, Conversation, Decision, Outcome, Proposed, Step, Turn},
        writing::WritingRequest,
        Cancellation, Feedback, RunStore,
    },
};

use super::Studio;
use crate::{
    assistant::{self, Engine, Scope},
    host::Place,
    writing,
};

pub(super) struct Assistant {
    conversation: Conversation,
    /// The project the conversation belongs to.
    owner: Option<PathBuf>,
    transcript: TranscriptState,
    pub(super) prompt: Entity<TextArea>,
    pub(super) scope: Scope,
    keep_facts: bool,
    running: Option<Running>,
    /// The suggestion as it streams in, before it is checked.
    streamed: String,
    pub(super) review: Option<Review>,
    /// Why the last request could not be sent, shown under the prompt.
    pub(super) notice: Option<String>,
    open_activity: HashSet<usize>,
    /// Stands in for the Settings provider in tests.
    #[cfg(test)]
    pub(super) model: Option<storyfella_studio_core::workflows::framework::SharedStreamingModel>,
}

struct Running {
    cancel: Cancellation,
    step_started: Instant,
    _task: Task<()>,
}

/// A suggestion under review in the editor's place.
pub(super) struct Review {
    turn: usize,
    pub(super) file: String,
    /// The file as the suggestion was written against.
    base: String,
    pub(super) diff: Entity<DiffView>,
}

impl Assistant {
    pub(super) fn new(theme: Theme, window: &mut Window, cx: &mut Context<Studio>) -> Self {
        let prompt = cx.new(|cx| {
            TextArea::new("Ask about this story…", theme, window, cx)
                .bare(true)
                .rows(1)
                .max_rows(6)
        });
        cx.subscribe_in(
            &prompt,
            window,
            |studio, _, event, window, cx| match event {
                TextAreaEvent::Submitted(_) => studio.ask(window, cx),
                TextAreaEvent::Changed(_) => {
                    studio.assistant.notice = None;
                    cx.notify()
                }
            },
        )
        .detach();
        Self {
            conversation: Conversation::default(),
            owner: None,
            transcript: TranscriptState::new(0),
            prompt,
            scope: Scope::default(),
            keep_facts: true,
            running: None,
            streamed: String::new(),
            review: None,
            notice: None,
            open_activity: HashSet::new(),
            #[cfg(test)]
            model: None,
        }
    }

    pub(super) fn set_theme(&mut self, theme: Theme, cx: &mut Context<Studio>) {
        self.prompt
            .update(cx, |prompt, cx| prompt.set_theme(theme, cx));
        if let Some(review) = &self.review {
            review.diff.update(cx, |diff, cx| diff.set_theme(theme, cx));
        }
    }

    #[cfg(test)]
    pub(super) fn conversation(&self) -> &Conversation {
        &self.conversation
    }

    fn answer_mut(&mut self, turn: usize) -> Option<&mut Answer> {
        match self.conversation.turns.get_mut(turn) {
            Some(Turn::Answered(answer)) => Some(answer),
            _ => None,
        }
    }

    fn save(&self) -> Result<(), String> {
        let Some(project) = &self.owner else {
            return Ok(());
        };
        RunStore::open(&project.with_extension("workflows.sqlite3"))
            .and_then(|store| store.save_conversation(project, &self.conversation))
            .map_err(|error| format!("Could not save the conversation: {error}"))
    }
}

/// Lines a suggestion adds and removes.
fn line_counts(diff: &Diff) -> (usize, usize) {
    diff.lines()
        .iter()
        .fold((0, 0), |(added, removed), line| match line.kind {
            LineKind::Added => (added + 1, removed),
            LineKind::Removed => (added, removed + 1),
            LineKind::Context => (added, removed),
        })
}

/// The whole file with the suggestion applied.
fn suggested(proposal: &storyfella_studio_core::workflows::writing::WritingProposal) -> String {
    let request = &proposal.request;
    let mut text = request.source.clone();
    text.replace_range(
        request.range.start..request.range.end,
        &proposal.replacement,
    );
    text
}

impl Studio {
    /// Loads the open project's conversation when the project changes.
    pub(super) fn follow_project_in_assistant(&mut self, cx: &mut Context<Self>) {
        let project = self.state.project_path.clone();
        if project == self.assistant.owner {
            return;
        }
        if let Some(running) = self.assistant.running.take() {
            running
                .cancel
                .store(true, std::sync::atomic::Ordering::Relaxed);
        }
        self.assistant.review = None;
        self.assistant.streamed.clear();
        self.assistant.open_activity.clear();
        self.assistant.conversation = match &project {
            Some(project) => RunStore::open(&project.with_extension("workflows.sqlite3"))
                .and_then(|store| store.conversation(project))
                .unwrap_or_else(|error| {
                    self.fail(ErrorCode::AppConversation, error.to_string(), cx);
                    Conversation::default()
                }),
            None => Conversation::default(),
        };
        self.assistant.owner = project;
        self.assistant
            .transcript
            .reset(self.assistant.conversation.turns.len());
    }

    fn engine(&self) -> Result<Engine, String> {
        #[cfg(test)]
        if let Some(model) = &self.assistant.model {
            return Ok(Engine::Model(model.clone()));
        }
        let backend = PlannerBackend::new(&self.config);
        if backend.has_llm() {
            Ok(Engine::Provider(backend))
        } else {
            Err("Choose a text provider in Settings to use the assistant.".into())
        }
    }

    /// Sends the prompt: a new request, or a refinement of the open review.
    pub(super) fn ask(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let text = self.assistant.prompt.read(cx).text(cx).trim().to_owned();
        if text.is_empty() || self.assistant.running.is_some() {
            return;
        }
        let engine = match self.engine() {
            Ok(engine) => engine,
            Err(message) => return self.refuse(message, cx),
        };
        let captured = match self.assistant.review.as_ref().map(|review| review.turn) {
            Some(turn) => self.refinement(turn, &text, cx),
            None => {
                let Some(file) = self.write.file().map(str::to_owned) else {
                    return;
                };
                let selection = self.write.editor.read(cx).selection(cx);
                assistant::capture(
                    &self.state,
                    &file,
                    self.assistant.scope,
                    selection,
                    &text,
                    self.assistant.keep_facts,
                )
                .map(|request| (file, self.assistant.scope, request))
            }
        };
        let (file, scope, request) = match captured {
            Ok(captured) => captured,
            Err(message) => return self.refuse(message, cx),
        };
        self.assistant.notice = None;
        self.assistant
            .prompt
            .update(cx, |prompt, cx| prompt.set_text("", window, cx));
        let turns = &mut self.assistant.conversation.turns;
        turns.push(Turn::Asked(Asked {
            text,
            scope: assistant::describe(scope, &file, &request.source, request.range),
        }));
        turns.push(Turn::Answered(Answer::default()));
        let turn = turns.len() - 1;
        self.assistant.transcript.push(2);
        self.assistant.transcript.scroll_to_end();
        self.assistant.streamed.clear();
        self.run_request(turn, file, request, engine, cx);
    }

    /// Keeps the prompt as typed and says why it was not sent.
    fn refuse(&mut self, message: String, cx: &mut Context<Self>) {
        self.assistant.notice = Some(message);
        cx.notify();
    }

    /// Ends the open review, keeping what was kept, and asks again for the
    /// same passage with both requests.
    fn refinement(
        &mut self,
        turn: usize,
        text: &str,
        cx: &mut Context<Self>,
    ) -> Result<(String, Scope, WritingRequest), String> {
        let Some(Turn::Answered(Answer {
            outcome: Outcome::Proposed(proposed),
            ..
        })) = self.assistant.conversation.turns.get(turn).cloned()
        else {
            return Err("There is no suggestion to refine.".into());
        };
        let kept_before = writing::source(&self.state, &proposed.file).map(str::len);
        self.finish_review(None, cx)?;
        let request = proposed.proposal.request;
        let grown = writing::source(&self.state, &proposed.file)
            .map(str::len)
            .zip(kept_before)
            .map_or(0, |(now, before)| now as isize - before as isize);
        let end = (request.range.end as isize + grown).max(request.range.start as isize) as usize;
        let mut refined = assistant::capture(
            &self.state,
            &proposed.file,
            Scope::Selection,
            request.range.start..end,
            &format!("{}\n\nThen: {text}", request.instructions),
            request.preserve_facts,
        )?;
        refined.action = request.action;
        Ok((proposed.file, Scope::Selection, refined))
    }

    fn run_request(
        &mut self,
        turn: usize,
        file: String,
        request: WritingRequest,
        engine: Engine,
        cx: &mut Context<Self>,
    ) {
        let Some(store) = assistant::store_path(&self.state) else {
            return;
        };
        let cancel: Cancellation = Arc::new(AtomicBool::new(false));
        let (send, mut receive) = tokio::sync::mpsc::unbounded_channel();
        let registry = self.state.registry.clone();
        let worker = cx.background_spawn({
            let cancel = cancel.clone();
            async move {
                assistant::suggest(request, engine, registry, store, cancel, move |event| {
                    let _ = send.send(event);
                })
            }
        });
        let task = cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            while let Some(event) = receive.recv().await {
                let alive = this.update(cx, |studio, cx| studio.heard(turn, event, cx));
                if alive.is_err() {
                    return;
                }
            }
            let result = worker.await;
            this.update(cx, |studio, cx| studio.answered(turn, file, result, cx))
                .ok();
        });
        self.assistant.running = Some(Running {
            cancel,
            step_started: Instant::now(),
            _task: task,
        });
        self.save_conversation(cx);
        cx.notify();
    }

    /// Progress from the workflow: a new phase, or more of the suggestion.
    fn heard(&mut self, turn: usize, event: Feedback, cx: &mut Context<Self>) {
        let now = Instant::now();
        let started = self
            .assistant
            .running
            .as_ref()
            .map_or(now, |running| running.step_started);
        match event {
            Feedback::Status(label) => {
                if let Some(answer) = self.assistant.answer_mut(turn) {
                    close_step(answer, now.duration_since(started));
                    answer.steps.push(Step {
                        label: label.trim_end_matches('…').to_owned(),
                        millis: None,
                    });
                }
                if let Some(running) = &mut self.assistant.running {
                    running.step_started = now;
                }
                self.assistant.streamed.clear();
            }
            Feedback::Text(text) => self.assistant.streamed.push_str(&text),
            Feedback::Event(_) => return,
        }
        self.assistant.transcript.changed(turn);
        cx.notify();
    }

    fn answered(
        &mut self,
        turn: usize,
        file: String,
        result: anyhow::Result<Option<storyfella_studio_core::workflows::writing::WritingProposal>>,
        cx: &mut Context<Self>,
    ) {
        let started = self
            .assistant
            .running
            .take()
            .map(|running| running.step_started);
        let outcome = match result {
            Ok(Some(proposal)) => {
                let after = suggested(&proposal);
                if after == proposal.request.source {
                    Outcome::Unchanged
                } else {
                    let (added, removed) =
                        line_counts(&Diff::new(&proposal.request.source, &after));
                    Outcome::Proposed(Box::new(Proposed {
                        file,
                        proposal,
                        added,
                        removed,
                        decision: Decision::Pending,
                    }))
                }
            }
            Ok(None) => Outcome::Stopped,
            Err(error) => Outcome::Failed(format!("{error:#}")),
        };
        if let Some(answer) = self.assistant.answer_mut(turn) {
            close_step(
                answer,
                started.map_or(Duration::ZERO, |started| started.elapsed()),
            );
            answer.outcome = outcome;
        }
        self.assistant.streamed.clear();
        self.assistant.transcript.changed(turn);
        self.save_conversation(cx);
        cx.notify();
    }

    pub(super) fn stop_assistant(&mut self, cx: &mut Context<Self>) {
        if let Some(running) = &self.assistant.running {
            running
                .cancel
                .store(true, std::sync::atomic::Ordering::Relaxed);
        }
        cx.notify();
    }

    fn save_conversation(&mut self, cx: &mut Context<Self>) {
        if let Err(message) = self.assistant.save() {
            self.fail(ErrorCode::AppConversation, message, cx);
        }
    }

    /// Opens the suggestion of `turn` for review in the editor's place.
    pub(super) fn start_review(
        &mut self,
        turn: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(Turn::Answered(Answer {
            outcome: Outcome::Proposed(proposed),
            ..
        })) = self.assistant.conversation.turns.get(turn)
        else {
            return;
        };
        let base = proposed.proposal.request.source.clone();
        if writing::source(&self.state, &proposed.file) != Some(base.as_str()) {
            self.refuse(
                format!(
                    "{} changed since this suggestion was written. Ask again for a fresh one.",
                    proposed.file
                ),
                cx,
            );
            return;
        }
        let file = proposed.file.clone();
        let diff = Diff::new(&base, &suggested(&proposed.proposal));
        let theme = self.theme;
        let view =
            cx.new(|_| DiffView::new(diff, theme).label(format!("Suggested edit to {file}")));
        cx.subscribe_in(&view, window, |studio, view, event, _, cx| {
            let DiffViewEvent::Decided { .. } = event;
            let decided = view
                .read(cx)
                .states()
                .iter()
                .all(|state| *state != HunkState::Pending);
            if decided {
                if let Err(message) = studio.finish_review(None, cx) {
                    studio.refuse(message, cx);
                }
            }
            cx.notify();
        })
        .detach();
        self.assistant.review = Some(Review {
            turn,
            file: file.clone(),
            base,
            diff: view,
        });
        self.open_file(&file, None, cx);
        self.show(Place::Write, window, cx);
    }

    /// Ends the open review. `Some(true)` keeps the changes not yet decided,
    /// `Some(false)` discards them, and `None` discards them; a change decided
    /// one by one keeps its decision. What is kept becomes one undoable edit.
    pub(super) fn finish_review(
        &mut self,
        all: Option<bool>,
        cx: &mut Context<Self>,
    ) -> Result<(), String> {
        let Some(review) = self.assistant.review.take() else {
            return Ok(());
        };
        let (merged, kept, of) = review.diff.update(cx, |view, cx| {
            if let Some(all) = all {
                let state = if all {
                    HunkState::Accepted
                } else {
                    HunkState::Rejected
                };
                // Changes already decided one by one keep their decision.
                let states: Vec<_> = view
                    .states()
                    .iter()
                    .map(|decided| {
                        if *decided == HunkState::Pending {
                            state
                        } else {
                            *decided
                        }
                    })
                    .collect();
                view.set_states(&states, cx);
            }
            let kept = view
                .states()
                .iter()
                .filter(|state| **state == HunkState::Accepted)
                .count();
            (view.merged(), kept, view.states().len())
        });
        let decision = if kept == 0 {
            Decision::Discarded
        } else {
            if writing::source(&self.state, &review.file) != Some(review.base.as_str()) {
                self.assistant.review = Some(review);
                return Err(
                    "The file changed during the review; its changes were not kept.".into(),
                );
            }
            writing::set_source(&mut self.state, &review.file, merged)?;
            Decision::Kept { kept, of }
        };
        if let Some(Turn::Answered(Answer {
            outcome: Outcome::Proposed(proposed),
            ..
        })) = self.assistant.conversation.turns.get_mut(review.turn)
        {
            proposed.decision = decision;
        }
        self.assistant.transcript.changed(review.turn);
        self.save_conversation(cx);
        self.changed(cx);
        Ok(())
    }

    fn discard_proposal(&mut self, turn: usize, cx: &mut Context<Self>) {
        if self.assistant.review.as_ref().map(|review| review.turn) == Some(turn) {
            if let Err(message) = self.finish_review(Some(false), cx) {
                self.refuse(message, cx);
            }
            return;
        }
        if let Some(Turn::Answered(Answer {
            outcome: Outcome::Proposed(proposed),
            ..
        })) = self.assistant.conversation.turns.get_mut(turn)
        {
            proposed.decision = Decision::Discarded;
        }
        self.assistant.transcript.changed(turn);
        self.save_conversation(cx);
        cx.notify();
    }

    /// The bar above a suggestion under review.
    pub(super) fn render_review_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let c = self.theme;
        let (decided, of) = self.assistant.review.as_ref().map_or((0, 0), |review| {
            let states = review.diff.read(cx).states();
            (
                states
                    .iter()
                    .filter(|state| **state != HunkState::Pending)
                    .count(),
                states.len(),
            )
        });
        let finish = |all: bool| {
            cx.listener(move |studio: &mut Studio, _, _, cx| {
                if let Err(message) = studio.finish_review(Some(all), cx) {
                    studio.refuse(message, cx);
                }
            })
        };
        div()
            .h(px(36.))
            .flex_none()
            .flex()
            .items_center()
            .gap(px(6.))
            .px(px(12.))
            .border_b_1()
            .border_color(c.hairline)
            .bg(c.surface)
            .child(Glyph::Sparkle.render(c.accent, px(12.)))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .type_scale(TypeScale::Small)
                    .child(format!(
                        "Reviewing a suggestion · {decided} of {of} changes decided · read-only"
                    )),
            )
            .child(
                c.button("discard-all", "Discard all", ButtonKind::Secondary)
                    .on_click(finish(false)),
            )
            .child(
                c.button("keep-all", "Keep all", ButtonKind::Primary)
                    .on_click(finish(true)),
            )
    }

    pub(super) fn render_assistant(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let c = self.theme;
        let studio = cx.entity().downgrade();
        let turns = self.assistant.conversation.turns.clone();
        let running_turn = self
            .assistant
            .running
            .as_ref()
            .map(|_| turns.len().saturating_sub(1));
        let streamed = self.assistant.streamed.clone();
        let reviewing = self.assistant.review.as_ref().map(|review| review.turn);
        let open = self.assistant.open_activity.clone();
        let item = move |index: usize, _: &mut Window, _: &mut App| -> AnyElement {
            match &turns[index] {
                Turn::Asked(asked) => div()
                    .flex()
                    .flex_col()
                    .items_end()
                    .gap(px(4.))
                    .child(UserMessage::new(("asked", index), c, asked.text.clone()))
                    .child(
                        div()
                            .type_scale(TypeScale::Caption)
                            .text_color(c.text_muted)
                            .child(asked.scope.clone()),
                    )
                    .into_any_element(),
                Turn::Answered(answer) => render_answer(
                    index,
                    answer,
                    c,
                    running_turn == Some(index),
                    &streamed,
                    reviewing == Some(index),
                    open.contains(&index),
                    studio.clone(),
                ),
            }
        };
        let running = self.assistant.running.is_some();
        let empty = self.assistant.prompt.read(cx).text(cx).trim().is_empty();
        let configured = self.engine().is_ok();
        let action = if running {
            c.icon_action("assistant-stop", Glyph::Stop, "Stop", ButtonKind::Secondary)
                .on_click(cx.listener(|studio, _, _, cx| studio.stop_assistant(cx)))
        } else {
            c.icon_action(
                "assistant-send",
                Glyph::ArrowUp,
                "Send (⌘Return)",
                ButtonKind::Primary,
            )
            .disabled(empty || !configured)
            .on_click(cx.listener(|studio, _, window, cx| studio.ask(window, cx)))
        };
        let mut bar = PromptBar::new("assistant-prompt", c, self.assistant.prompt.clone());
        if reviewing.is_none() {
            for scope in Scope::ALL {
                bar = bar.option(
                    c.chip(
                        ("assistant-scope", scope as usize),
                        scope.label(),
                        self.assistant.scope == scope,
                    )
                    .on_click(cx.listener(move |studio, _, _, cx| {
                        studio.assistant.scope = scope;
                        cx.notify();
                    })),
                );
            }
        } else {
            bar = bar.option(c.chip("assistant-refining", "Refining the suggestion", true));
        }
        bar = bar.action(action);
        if !configured {
            bar = bar.message("Choose a text provider in Settings to use the assistant.");
        } else if let Some(notice) = &self.assistant.notice {
            bar = bar.message(notice.clone());
        }
        div()
            .w(px(340.))
            .flex_none()
            .flex()
            .flex_col()
            .border_l_1()
            .border_color(c.hairline)
            .bg(c.surface)
            .child(
                div()
                    .h(px(36.))
                    .flex_none()
                    .flex()
                    .items_center()
                    .px(px(12.))
                    .border_b_1()
                    .border_color(c.hairline)
                    .child(
                        div()
                            .flex_1()
                            .type_scale(TypeScale::Caption)
                            .text_color(c.text_muted)
                            .child("Assistant"),
                    )
                    .child(
                        c.switch("keep-facts", "Keep facts", self.assistant.keep_facts)
                            .on_click(cx.listener(|studio, _, _, cx| {
                                studio.assistant.keep_facts = !studio.assistant.keep_facts;
                                cx.notify();
                            })),
                    ),
            )
            .child(div().flex_1().min_h_0().child(
                if self.assistant.conversation.turns.is_empty() {
                    div()
                        .size_full()
                        .flex()
                        .flex_col()
                        .justify_center()
                        .px(px(24.))
                        .gap(px(6.))
                        .text_color(c.text_muted)
                        .type_scale(TypeScale::Small)
                        .child(
                            div()
                                .text_color(c.text)
                                .child("Ask for an edit to the story"),
                        )
                        .child(
                            "Pick the passage below, say what should change, and review the \
                             suggestion before anything is written.",
                        )
                        .into_any_element()
                } else {
                    Transcript::new("assistant-transcript", c, &self.assistant.transcript, item)
                        .into_any_element()
                },
            ))
            .child(div().p(px(10.)).child(bar))
    }
}

/// Ends the running step, if any, with how long it took.
fn close_step(answer: &mut Answer, took: Duration) {
    if let Some(step) = answer
        .steps
        .iter_mut()
        .rev()
        .find(|step| step.millis.is_none())
    {
        step.millis = Some(took.as_millis() as u64);
    }
}

#[allow(clippy::too_many_arguments)]
fn render_answer(
    index: usize,
    answer: &Answer,
    c: Theme,
    running: bool,
    streamed: &str,
    reviewing: bool,
    open: bool,
    studio: WeakEntity<Studio>,
) -> AnyElement {
    let mut message = AssistantMessage::new(("answer", index), c);
    if !answer.steps.is_empty() {
        let mut activity = ToolActivity::new(("activity", index), c).expanded(open);
        let mut total = Duration::ZERO;
        for (step, phase) in answer.steps.iter().enumerate() {
            let status = match (phase.millis, &answer.outcome) {
                (None, Outcome::Working) => ToolStatus::Running,
                (None, _) | (Some(_), Outcome::Stopped) if step + 1 == answer.steps.len() => {
                    ToolStatus::Cancelled
                }
                _ => ToolStatus::Done,
            };
            let mut call = ToolCall::new(
                ("step", index * 100 + step),
                c,
                "Studio",
                phase.label.clone(),
                status,
            );
            if let Some(millis) = phase.millis {
                let took = Duration::from_millis(millis);
                total += took;
                call = call.elapsed(took);
            }
            activity = activity.call(call);
        }
        let toggle = studio.clone();
        message = message.block(activity.elapsed(total).on_toggle(move |_, cx| {
            toggle
                .update(cx, |studio, cx| {
                    let open = &mut studio.assistant.open_activity;
                    if !open.remove(&index) {
                        open.insert(index);
                    }
                    studio.assistant.transcript.changed(index);
                    cx.notify();
                })
                .ok();
        }));
    }
    match &answer.outcome {
        Outcome::Working => {
            let preview = if streamed.is_empty() {
                "Working on it…".to_owned()
            } else {
                let tail: String = streamed
                    .chars()
                    .rev()
                    .take(600)
                    .collect::<Vec<_>>()
                    .into_iter()
                    .rev()
                    .collect();
                format!("```\n{tail}\n```")
            };
            message = message.markdown(preview).streaming(running);
        }
        Outcome::Proposed(proposed) => {
            message = message
                .markdown(format!("Suggested an edit to **{}**.", proposed.file))
                .block(proposal_card(index, proposed, c, reviewing, studio));
        }
        Outcome::Unchanged => {
            message = message.markdown(
                "The suggestion matched the passage as it stands, so there is nothing to \
                 review. Try more specific instructions.",
            );
        }
        Outcome::Failed(details) => {
            let code = ErrorCode::WritingProposalFailed;
            let details = details.clone();
            message = message
                .error(code.message())
                .on_error_details(move |_, _, cx| {
                    studio
                        .update(cx, |studio, cx| {
                            studio.show_details(ReportedError::plain(code, details.clone()), cx)
                        })
                        .ok();
                });
        }
        Outcome::Stopped => message = message.stopped(true),
        Outcome::Interrupted => {
            message = message.error("Studio closed before this reply finished. Ask again to retry.")
        }
    }
    message.into_any_element()
}

fn proposal_card(
    index: usize,
    proposed: &Proposed,
    c: Theme,
    reviewing: bool,
    studio: WeakEntity<Studio>,
) -> impl IntoElement {
    let (status, pending) = match proposed.decision {
        Decision::Pending if reviewing => ("Reviewing".to_owned(), false),
        Decision::Pending => ("Makes changes".to_owned(), true),
        Decision::Kept { kept, of } if kept == of => ("Kept".to_owned(), false),
        Decision::Kept { kept, of } => (format!("Kept {kept} of {of}"), false),
        Decision::Discarded => ("Discarded".to_owned(), false),
    };
    let review = studio.clone();
    div()
        .flex()
        .flex_col()
        .gap(px(8.))
        .p(px(10.))
        .rounded(c.radius_control)
        .border_1()
        .border_color(if pending || reviewing {
            c.accent
        } else {
            c.hairline
        })
        .bg(c.raised)
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(6.))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .font_weight(FontWeight::MEDIUM)
                        .child(format!("Edit {}", proposed.file)),
                )
                .child(
                    div()
                        .type_scale(TypeScale::Caption)
                        .text_color(if pending { c.accent } else { c.text_muted })
                        .child(status),
                ),
        )
        .child(
            div()
                .type_scale(TypeScale::Small)
                .text_color(c.text_muted)
                .child(format!("+{} −{} lines", proposed.added, proposed.removed)),
        )
        .when(pending, |card| {
            card.child(
                div()
                    .flex()
                    .gap(px(6.))
                    .child(
                        c.button(("review", index), "Review", ButtonKind::Primary)
                            .on_click(move |_, window, cx| {
                                review
                                    .update(cx, |studio, cx| studio.start_review(index, window, cx))
                                    .ok();
                            }),
                    )
                    .child(
                        c.button(("discard", index), "Discard", ButtonKind::Secondary)
                            .on_click(move |_, _, cx| {
                                studio
                                    .update(cx, |studio, cx| studio.discard_proposal(index, cx))
                                    .ok();
                            }),
                    ),
            )
        })
}
