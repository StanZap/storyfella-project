//! The assistant column docked at Write's right: the project's saved
//! conversation, and a prompt that asks the writing workflows for an edit to
//! a passage or to the whole project. A suggestion arrives as a card;
//! reviewing it turns the editor into a read-only diff of each changed file
//! in turn, whose changes are kept or reset one by one, and the story changes
//! only when the review ends — every file at once, as one undoable edit.
//!
//! Asking again while a review is open refines it: the changes kept so far
//! are applied, and the same passage — or project — is asked for again with
//! both requests.
use std::{
    collections::HashSet,
    path::PathBuf,
    sync::{atomic::AtomicBool, Arc},
    time::{Duration, Instant},
};

use design_system::{
    kits::{
        chat::{
            AssistantMessage, Draft, Step as Phase, Steps, ToolStatus, Transcript, TranscriptState,
            UserMessage,
        },
        code::{Diff, DiffView, DiffViewEvent, HunkState, LineKind},
    },
    ButtonKind, Glyph, Menu, MenuButton, MenuItem, PromptBar, TextArea, TextAreaEvent, Theme,
    TypeScale, Typography,
};
use gpui_kit::{prelude::*, *};
use storyfella_studio_core::{
    errors::{ErrorCode, ReportedError},
    planner::PlannerBackend,
    workflows::{
        assistant::{
            Answer, Asked, Conversation, Decision, FileChange, Outcome, Proposed, Step, Suggestion,
            Turn,
        },
        writing as core_writing, Cancellation, Feedback, RunStore,
    },
};

use super::Studio;
use crate::{
    assistant::{self, Answered, Engine, Request, Scope},
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
    /// Whether the scope's menu is open.
    choosing_scope: bool,
    keep_facts: bool,
    running: Option<Running>,
    /// The answer as it streams in, before it is read.
    streamed: String,
    /// How much reasoning the model has streamed apart from its answer.
    reasoned: usize,
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
    /// Redraws every second so the running phase's time moves.
    _tick: Task<()>,
}

/// A suggestion under review in the editor's place, one file at a time.
pub(super) struct Review {
    turn: usize,
    files: Vec<(String, Entity<DiffView>)>,
    /// The file shown, as an index into `files`.
    current: usize,
    /// The story as the suggestion was written against.
    base: storyfella_compiler::SourceWorkspace,
}

impl Review {
    /// The file shown and its diff.
    pub(super) fn shown(&self) -> (&str, &Entity<DiffView>) {
        let (file, diff) = &self.files[self.current];
        (file, diff)
    }

    /// Changes decided, and changes in all, across every file.
    fn decided(&self, cx: &App) -> (usize, usize) {
        self.files.iter().fold((0, 0), |(decided, of), (_, diff)| {
            let states = diff.read(cx).states();
            (
                decided
                    + states
                        .iter()
                        .filter(|state| **state != HunkState::Pending)
                        .count(),
                of + states.len(),
            )
        })
    }

    /// The next file, after the one shown, with a change still undecided.
    fn next_undecided(&self, cx: &App) -> Option<usize> {
        (1..self.files.len())
            .map(|step| (self.current + step) % self.files.len())
            .find(|&index| {
                self.files[index]
                    .1
                    .read(cx)
                    .states()
                    .contains(&HunkState::Pending)
            })
    }
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
            choosing_scope: false,
            keep_facts: true,
            running: None,
            streamed: String::new(),
            reasoned: 0,
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
            for (_, diff) in &review.files {
                diff.update(cx, |diff, cx| diff.set_theme(theme, cx));
            }
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
                .map(|request| (self.assistant.scope, request))
            }
        };
        let (scope, request) = match captured {
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
            scope: request.describe(scope),
            place: request.place(),
        }));
        turns.push(Turn::Answered(Answer::default()));
        let turn = turns.len() - 1;
        self.assistant.transcript.push(2);
        self.assistant.transcript.scroll_to_end();
        self.assistant.streamed.clear();
        self.run_request(turn, request, engine, cx);
    }

    /// Sends the request of the reply at `turn` again, about what the scope
    /// names now.
    pub(super) fn retry(&mut self, turn: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(Turn::Asked(asked)) = turn
            .checked_sub(1)
            .and_then(|asked| self.assistant.conversation.turns.get(asked))
        else {
            return;
        };
        let text = asked.text.clone();
        self.assistant
            .prompt
            .update(cx, |prompt, cx| prompt.set_text(&text, window, cx));
        self.ask(window, cx);
    }

    /// Keeps the prompt as typed and says why it was not sent.
    fn refuse(&mut self, message: String, cx: &mut Context<Self>) {
        self.assistant.notice = Some(message);
        cx.notify();
    }

    /// Ends the open review, keeping what was kept, and asks again for the
    /// same passage — or the whole project — with both requests.
    fn refinement(
        &mut self,
        turn: usize,
        text: &str,
        cx: &mut Context<Self>,
    ) -> Result<(Scope, Request), String> {
        let Some(Turn::Answered(Answer {
            outcome: Outcome::Proposed(proposed),
            ..
        })) = self.assistant.conversation.turns.get(turn).cloned()
        else {
            return Err("There is no suggestion to refine.".into());
        };
        match proposed.suggestion {
            Suggestion::Passage { file, proposal } => {
                let kept_before = writing::source(&self.state, &file).map(str::len);
                self.finish_review(None, cx)?;
                let request = proposal.request;
                let grown = writing::source(&self.state, &file)
                    .map(str::len)
                    .zip(kept_before)
                    .map_or(0, |(now, before)| now as isize - before as isize);
                let end =
                    (request.range.end as isize + grown).max(request.range.start as isize) as usize;
                let mut refined = assistant::capture_passage(
                    &self.state,
                    &file,
                    Scope::Selection,
                    request.range.start..end,
                    &format!("{}\n\nThen: {text}", request.instructions),
                    request.preserve_facts,
                )?;
                refined.action = request.action;
                Ok((
                    Scope::Selection,
                    Request::Passage {
                        file,
                        request: refined,
                    },
                ))
            }
            Suggestion::Project(proposal) => {
                self.finish_review(None, cx)?;
                let request = proposal.request;
                let mut refined = assistant::capture_project(
                    &self.state,
                    &format!("{}\n\nThen: {text}", request.instructions),
                    request.preserve_facts,
                )?;
                refined.action = request.action;
                Ok((Scope::Project, Request::Project(refined)))
            }
        }
    }

    fn run_request(
        &mut self,
        turn: usize,
        request: Request,
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
            this.update(cx, |studio, cx| studio.answered(turn, result, cx))
                .ok();
        });
        let tick = cx.spawn(
            async move |this: WeakEntity<Self>, cx: &mut AsyncApp| loop {
                cx.background_executor().timer(Duration::from_secs(1)).await;
                let alive = this.update(cx, |studio, cx| {
                    studio.assistant.transcript.changed(turn);
                    cx.notify();
                });
                if alive.is_err() {
                    return;
                }
            },
        );
        self.assistant.running = Some(Running {
            cancel,
            step_started: Instant::now(),
            _task: task,
            _tick: tick,
        });
        self.save_conversation(cx);
        cx.notify();
    }

    /// Progress from the workflow: a new phase, what it found, or more of
    /// the answer.
    fn heard(&mut self, turn: usize, event: Feedback, cx: &mut Context<Self>) {
        let now = Instant::now();
        let started = self
            .assistant
            .running
            .as_ref()
            .map_or(now, |running| running.step_started);
        let streamed = streamed_detail(&self.assistant.streamed, self.assistant.reasoned, false);
        match event {
            Feedback::Status(label) => {
                if let Some(answer) = self.assistant.answer_mut(turn) {
                    close_step(answer, now.duration_since(started), streamed);
                    answer.steps.push(Step {
                        label: label.trim_end_matches('…').to_owned(),
                        millis: None,
                        detail: None,
                    });
                }
                if let Some(running) = &mut self.assistant.running {
                    running.step_started = now;
                }
                self.assistant.streamed.clear();
                self.assistant.reasoned = 0;
            }
            Feedback::Detail(detail) => {
                if let Some(step) = self
                    .assistant
                    .answer_mut(turn)
                    .and_then(|answer| answer.steps.last_mut())
                {
                    step.detail = Some(detail);
                }
            }
            Feedback::Text(text) => self.assistant.streamed.push_str(&text),
            Feedback::Reasoning(text) => self.assistant.reasoned += text.chars().count(),
            Feedback::Event(_) => return,
        }
        self.assistant.transcript.changed(turn);
        cx.notify();
    }

    fn answered(
        &mut self,
        turn: usize,
        result: anyhow::Result<Option<Answered>>,
        cx: &mut Context<Self>,
    ) {
        let started = self
            .assistant
            .running
            .take()
            .map(|running| running.step_started);
        let outcome = match result {
            Ok(Some(Answered::Edit(suggestion))) => {
                let files: Vec<FileChange> = suggestion
                    .texts()
                    .into_iter()
                    .filter(|(_, before, after)| before != after)
                    .map(|(file, before, after)| {
                        let (added, removed) = line_counts(&Diff::new(&before, &after));
                        FileChange {
                            file,
                            added,
                            removed,
                        }
                    })
                    .collect();
                if files.is_empty() {
                    Outcome::Unchanged
                } else {
                    Outcome::Proposed(Box::new(Proposed {
                        suggestion: *suggestion,
                        files,
                        decision: Decision::Pending,
                    }))
                }
            }
            Ok(Some(Answered::Reply(text))) => Outcome::Replied(text),
            Ok(None) => Outcome::Stopped,
            Err(error) => Outcome::Failed(format!("{error:#}")),
        };
        let streamed = streamed_detail(&self.assistant.streamed, self.assistant.reasoned, false);
        if let Some(answer) = self.assistant.answer_mut(turn) {
            close_step(
                answer,
                started.map_or(Duration::ZERO, |started| started.elapsed()),
                streamed,
            );
            answer.outcome = outcome;
        }
        self.assistant.streamed.clear();
        self.assistant.reasoned = 0;
        self.assistant.transcript.changed(turn);
        self.save_conversation(cx);
        cx.notify();
    }

    /// Starts over: the project's saved conversation is emptied. The story
    /// is not touched, and nothing runs or is under review while it clears.
    pub(super) fn new_conversation(&mut self, cx: &mut Context<Self>) {
        if self.assistant.running.is_some() || self.assistant.review.is_some() {
            return;
        }
        self.assistant.conversation = Conversation::default();
        self.assistant.open_activity.clear();
        self.assistant.notice = None;
        self.assistant.transcript.reset(0);
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

    /// Opens the suggestion of `turn` for review in the editor's place,
    /// starting with its first file.
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
        let texts: Vec<_> = proposed
            .suggestion
            .texts()
            .into_iter()
            .filter(|(file, _, _)| proposed.files.iter().any(|change| change.file == *file))
            .collect();
        // Every file the suggestion changes, opened or not, as it is now.
        let base = match self.state.workspace_snapshot() {
            Ok(base) => base,
            Err(message) => return self.refuse(message, cx),
        };
        if let Some((file, _, _)) = texts
            .iter()
            .find(|(file, before, _)| base.files.get(file) != Some(before))
        {
            let message = format!(
                "{file} changed since this suggestion was written. Ask again for a fresh one."
            );
            return self.refuse(message, cx);
        }
        let theme = self.theme;
        let files: Vec<_> = texts
            .into_iter()
            .map(|(file, before, after)| {
                let label = format!("Suggested edit to {file}");
                let view =
                    cx.new(|_| DiffView::new(Diff::new(&before, &after), theme).label(label));
                cx.subscribe_in(&view, window, |studio, _, event, window, cx| {
                    let DiffViewEvent::Decided { .. } = event;
                    studio.decided(window, cx);
                })
                .detach();
                (file, view)
            })
            .collect();
        let first = files[0].0.clone();
        self.assistant.review = Some(Review {
            turn,
            files,
            current: 0,
            base,
        });
        self.open_file(&first, None, cx);
        self.show(Place::Write, window, cx);
    }

    /// A change was kept or reset: the review moves to the next file with
    /// something undecided, and ends once nothing is.
    pub(super) fn decided(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(review) = &self.assistant.review else {
            return;
        };
        let (decided, of) = review.decided(cx);
        if decided == of {
            if let Err(message) = self.finish_review(None, cx) {
                self.refuse(message, cx);
            }
        } else if !review.files[review.current]
            .1
            .read(cx)
            .states()
            .contains(&HunkState::Pending)
        {
            if let Some(next) = review.next_undecided(cx) {
                self.show_review_file(next, window, cx);
            }
        }
        cx.notify();
    }

    /// Shows the review's file at `index`.
    fn show_review_file(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(review) = &mut self.assistant.review else {
            return;
        };
        if index >= review.files.len() {
            return;
        }
        review.current = index;
        let file = review.files[index].0.clone();
        self.open_file(&file, None, cx);
        self.show(Place::Write, window, cx);
    }

    /// Ends the open review. `Some(true)` keeps the changes not yet decided,
    /// `Some(false)` discards them, and `None` discards them; a change decided
    /// one by one keeps its decision. What is kept, in every file, becomes
    /// one undoable edit.
    pub(super) fn finish_review(
        &mut self,
        all: Option<bool>,
        cx: &mut Context<Self>,
    ) -> Result<(), String> {
        let Some(review) = self.assistant.review.take() else {
            return Ok(());
        };
        let mut texts = std::collections::BTreeMap::new();
        let (mut kept, mut of) = (0, 0);
        for (file, diff) in &review.files {
            let (merged, file_kept, file_of) = diff.update(cx, |view, cx| {
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
            kept += file_kept;
            of += file_of;
            if file_kept > 0 {
                texts.insert(file.clone(), merged);
            }
        }
        let decision = if kept == 0 {
            Decision::Discarded
        } else {
            if let Err(message) = self.state.set_sources(&review.base, texts) {
                self.assistant.review = Some(review);
                return Err(format!("{message}; its changes were not kept."));
            }
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
        let Some(review) = &self.assistant.review else {
            return div();
        };
        let (decided, of) = review.decided(cx);
        let (shown, _) = review.shown();
        let count = review.files.len();
        let current = review.current;
        let place = if count > 1 {
            format!("{shown} ({} of {count})", current + 1)
        } else {
            shown.to_owned()
        };
        let finish = |all: bool| {
            cx.listener(move |studio: &mut Studio, _, _, cx| {
                if let Err(message) = studio.finish_review(Some(all), cx) {
                    studio.refuse(message, cx);
                }
            })
        };
        let step = |id: &'static str, glyph: Glyph, label: &'static str, to: Option<usize>| {
            c.icon_action(id, glyph, label, ButtonKind::Quiet)
                .disabled(to.is_none())
                .on_click(cx.listener(move |studio: &mut Studio, _, window, cx| {
                    if let Some(to) = to {
                        studio.show_review_file(to, window, cx);
                    }
                }))
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
                        "Reviewing {place} · {decided} of {of} changes decided · read-only"
                    )),
            )
            .when(count > 1, |bar| {
                bar.child(step(
                    "review-previous-file",
                    Glyph::ChevronLeft,
                    "Previous file",
                    current.checked_sub(1),
                ))
                .child(step(
                    "review-next-file",
                    Glyph::ChevronRight,
                    "Next file",
                    (current + 1 < count).then_some(current + 1),
                ))
            })
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
        let reasoned = self.assistant.reasoned;
        let started = self
            .assistant
            .running
            .as_ref()
            .map(|running| running.step_started);
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
                        // A way back to the passage, except while a review
                        // holds the editor.
                        match asked.place.clone().filter(|_| reviewing.is_none()) {
                            Some(place) => {
                                let studio = studio.clone();
                                c.link(("asked-place", index), asked.scope.clone())
                                    .max_w_full()
                                    .type_scale(TypeScale::Caption)
                                    .on_click(move |_, _, cx| {
                                        studio
                                            .update(cx, |studio, cx| {
                                                studio.open_file(
                                                    &place.file,
                                                    Some(place.offset),
                                                    cx,
                                                )
                                            })
                                            .ok();
                                    })
                                    .into_any_element()
                            }
                            None => div()
                                .type_scale(TypeScale::Caption)
                                .text_color(c.text_muted)
                                .child(asked.scope.clone())
                                .into_any_element(),
                        },
                    )
                    .into_any_element(),
                Turn::Answered(answer) => render_answer(
                    index,
                    answer,
                    c,
                    started
                        .filter(|_| running_turn == Some(index))
                        .map(|step_started| Live {
                            streamed: &streamed,
                            reasoned,
                            step_started,
                        })
                        .as_ref(),
                    Shown {
                        reviewing: reviewing == Some(index),
                        browsing: reviewing.is_none(),
                    },
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
            // Send takes the accent only when there is something to send.
            let ready = !empty && configured;
            c.icon_action(
                "assistant-send",
                Glyph::ArrowUp,
                "Send (⌘Return)",
                if ready {
                    ButtonKind::Primary
                } else {
                    ButtonKind::Secondary
                },
            )
            .disabled(!ready)
            .on_click(cx.listener(|studio, _, window, cx| studio.ask(window, cx)))
        };
        let mut bar = PromptBar::new("assistant-prompt", c, self.assistant.prompt.clone());
        if reviewing.is_none() {
            let entity = cx.entity();
            let menu = self.assistant.choosing_scope.then(|| {
                Scope::ALL.into_iter().fold(
                    Menu::new("assistant-scopes", c).width(px(160.)),
                    |menu, scope| {
                        let entity = entity.clone();
                        menu.item(
                            MenuItem::new(scope.label())
                                .checked(self.assistant.scope == scope)
                                .on_click(move |_, cx| {
                                    entity.update(cx, |studio, cx| {
                                        studio.assistant.scope = scope;
                                        studio.assistant.choosing_scope = false;
                                        cx.notify();
                                    });
                                }),
                        )
                    },
                )
            });
            let dismiss = entity.clone();
            bar = bar.option(
                MenuButton::new(
                    "assistant-scope",
                    c.chip("assistant-scope-chip", self.assistant.scope.label(), false)
                        .aria_label("What the request is about")
                        .aria_expanded(self.assistant.choosing_scope)
                        .child(Glyph::ChevronDown.render(c.text_muted, px(12.)))
                        .on_click(cx.listener(|studio, _, _, cx| {
                            studio.assistant.choosing_scope = !studio.assistant.choosing_scope;
                            cx.notify();
                        })),
                )
                .above()
                .menu(menu)
                .on_dismiss(move |_, cx| {
                    dismiss.update(cx, |studio, cx| {
                        studio.assistant.choosing_scope = false;
                        cx.notify();
                    });
                }),
            );
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
            .size_full()
            .flex()
            .flex_col()
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
                        c.icon_action(
                            "assistant-new",
                            Glyph::Plus,
                            "New conversation",
                            ButtonKind::Quiet,
                        )
                        .disabled(
                            running
                                || reviewing.is_some()
                                || self.assistant.conversation.turns.is_empty(),
                        )
                        .on_click(cx.listener(|studio, _, _, cx| studio.new_conversation(cx))),
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

/// Ends the running step, if any, with how long it took. A phase that
/// streamed an answer and said nothing else keeps how much arrived.
fn close_step(answer: &mut Answer, took: Duration, streamed: Option<String>) {
    if let Some(step) = answer
        .steps
        .iter_mut()
        .rev()
        .find(|step| step.millis.is_none())
    {
        step.millis = Some(took.as_millis() as u64);
        if step.detail.is_none() {
            step.detail = streamed;
        }
    }
}

/// `n` with thousands separated, as the column counts characters.
fn grouped(n: usize) -> String {
    let digits = n.to_string();
    let mut grouped = String::new();
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            grouped.push(',');
        }
        grouped.push(digit);
    }
    grouped
}

/// How much of an answer has arrived, for the phase receiving it: in the
/// present while it is `arriving`, and as a record once the phase ends.
fn streamed_detail(streamed: &str, reasoned: usize, arriving: bool) -> Option<String> {
    let (written, reasoned) = (grouped(streamed.chars().count()), grouped(reasoned));
    match (streamed.is_empty(), reasoned.as_str(), arriving) {
        (true, "0", _) => None,
        (true, _, true) => Some(format!("Thinking · {reasoned} characters")),
        (true, _, false) => Some(format!("Thought for {reasoned} characters")),
        (false, "0", true) => Some(format!("Writing · {written} characters")),
        (false, "0", false) => Some(format!("Wrote {written} characters")),
        (false, _, true) => Some(format!(
            "Writing · {written} characters after {reasoned} of thinking"
        )),
        (false, _, false) => Some(format!(
            "Wrote {written} characters after {reasoned} of thinking"
        )),
    }
}

/// The view of a run the column draws from: what is streaming now and when
/// the running phase began.
struct Live<'a> {
    streamed: &'a str,
    reasoned: usize,
    step_started: Instant,
}

fn render_answer(
    index: usize,
    answer: &Answer,
    c: Theme,
    live: Option<&Live>,
    Shown {
        reviewing,
        browsing,
    }: Shown,
    open: bool,
    studio: WeakEntity<Studio>,
) -> AnyElement {
    let mut message = AssistantMessage::new(("answer", index), c);
    if !answer.steps.is_empty() {
        let last = answer.steps.len() - 1;
        let mut steps = Steps::new(("steps", index), c).expanded(open || live.is_some());
        for (number, phase) in answer.steps.iter().enumerate() {
            let status = match (&answer.outcome, number == last) {
                (Outcome::Working, true) if phase.millis.is_none() => ToolStatus::Running,
                // The message's error card says what failed.
                (Outcome::Failed(_), true) => ToolStatus::Failed("".into()),
                (Outcome::Stopped | Outcome::Interrupted, true) => ToolStatus::Cancelled,
                _ => ToolStatus::Done,
            };
            let mut step = Phase::new(phase.label.clone(), status.clone());
            match (phase.millis, live) {
                (Some(millis), _) => step = step.elapsed(Duration::from_millis(millis)),
                (None, Some(live)) => step = step.elapsed(live.step_started.elapsed()),
                (None, None) => {}
            }
            let detail = match (&status, live) {
                (ToolStatus::Running, Some(live)) => phase
                    .detail
                    .clone()
                    .or_else(|| streamed_detail(live.streamed, live.reasoned, true)),
                _ => phase.detail.clone(),
            };
            if let Some(detail) = detail {
                step = step.detail(detail);
            }
            steps = steps.step(step);
        }
        let toggle = studio.clone();
        message = message.block(steps.on_toggle(move |_, cx| {
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
            if let Some(live) = live {
                let preview = core_writing::preview(live.streamed);
                if !preview.text.trim().is_empty() {
                    message = message.block(
                        Draft::new(("draft", index), c, preview.text.clone())
                            .caption(format!(
                                "Draft · {} characters",
                                grouped(preview.text.chars().count())
                            ))
                            .lines(8),
                    );
                }
            }
        }
        Outcome::Proposed(proposed) => {
            if let Some(note) = proposed.suggestion.note() {
                message = message.markdown(note.to_owned());
            }
            message = message.block(proposal_card(
                index,
                proposed,
                c,
                Shown {
                    reviewing,
                    browsing,
                },
                studio,
            ));
        }
        Outcome::Replied(text) => message = message.markdown(text.clone()),
        Outcome::Unchanged => {
            message = message.markdown(
                "The suggestion matched the passage as it stands, so there is nothing to \
                 review. Try more specific instructions.",
            );
        }
        Outcome::Failed(details) => {
            let code = ErrorCode::WritingProposalFailed;
            let details = details.clone();
            let retry = studio.clone();
            message = message
                .error(code.message())
                .on_retry(move |_, window, cx| {
                    retry
                        .update(cx, |studio, cx| studio.retry(index, window, cx))
                        .ok();
                })
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
            message = message
                .error("Studio closed before this reply finished.")
                .on_retry(move |_, window, cx| {
                    studio
                        .update(cx, |studio, cx| studio.retry(index, window, cx))
                        .ok();
                });
        }
    }
    message.into_any_element()
}

/// How a reply stands toward the review: whether its own suggestion is under
/// review, and whether no review holds the editor, so files can be opened.
#[derive(Clone, Copy)]
struct Shown {
    reviewing: bool,
    browsing: bool,
}

fn proposal_card(
    index: usize,
    proposed: &Proposed,
    c: Theme,
    Shown {
        reviewing,
        browsing,
    }: Shown,
    studio: WeakEntity<Studio>,
) -> impl IntoElement {
    let (status, pending) = match proposed.decision {
        Decision::Pending if reviewing => ("Reviewing".to_owned(), false),
        Decision::Pending => ("Ready to review".to_owned(), true),
        Decision::Kept { kept, of } if kept == of => ("Kept".to_owned(), false),
        Decision::Kept { kept, of } => (format!("Kept {kept} of {of}"), false),
        Decision::Discarded => ("Discarded".to_owned(), false),
    };
    let review = studio.clone();
    let counts = |change: &FileChange| {
        div()
            .flex_none()
            .flex()
            .gap(px(4.))
            .type_scale(TypeScale::Caption)
            .code_text()
            .child(
                div()
                    .text_color(c.success)
                    .child(format!("+{}", change.added)),
            )
            .child(
                div()
                    .text_color(c.error)
                    .child(format!("−{}", change.removed)),
            )
    };
    // A file opens in the editor from its name, while no review holds it.
    let name = |file: &str| {
        if !browsing {
            return div()
                .min_w_0()
                .truncate()
                .child(file.to_owned())
                .into_any_element();
        }
        let (open, file) = (studio.clone(), file.to_owned());
        c.link(
            SharedString::from(format!("open-{index}-{file}")),
            file.clone(),
        )
        .text_color(c.text)
        .on_click(move |_, _, cx| {
            open.update(cx, |studio, cx| studio.open_file(&file, None, cx))
                .ok();
        })
        .into_any_element()
    };
    let single = match proposed.files.as_slice() {
        [change] => Some(change),
        _ => None,
    };
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
                        .min_w_0()
                        .font_weight(FontWeight::MEDIUM)
                        .child(match single {
                            Some(change) => name(&change.file),
                            None => format!("{} files", proposed.files.len()).into_any_element(),
                        }),
                )
                .children(single.map(counts))
                .child(div().flex_1())
                .child(
                    div()
                        .flex_none()
                        .type_scale(TypeScale::Caption)
                        .text_color(if pending { c.accent } else { c.text_muted })
                        .child(status),
                ),
        )
        .when(single.is_none(), |card| {
            card.child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(2.))
                    .type_scale(TypeScale::Small)
                    .text_color(c.text_muted)
                    .children(proposed.files.iter().map(|change| {
                        div()
                            .flex()
                            .items_center()
                            .gap(px(6.))
                            .child(name(&change.file))
                            .child(div().flex_1())
                            .child(counts(change))
                    })),
            )
        })
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
