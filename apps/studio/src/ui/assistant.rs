//! The assistant column docked at Write's right: the conversation shown, the
//! project's saved [`conversations`] beside it, and a prompt that asks the
//! story agent for a change. The
//! agent edits the story as it works — each edit lands in the editor as one
//! undoable change — and what it changed waits in [`changes`] for the writer
//! to keep or undo, change by change.
//!
//! A request that runs out of steps says what is left and offers to
//! continue.
use std::{
    collections::{BTreeMap, HashSet},
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
        code::{Diff, LineKind},
    },
    ButtonKind, Glyph, InlineEdit, InlineEditEvent, Menu, MenuButton, MenuItem, PromptBar,
    SplitSide, SplitState, SplitView, TextArea, TextAreaEvent, Theme, TypeScale, Typography,
};
use gpui_kit::{prelude::*, *};
use storyfella_studio_core::{
    errors::{ErrorCode, ReportedError},
    planner::PlannerBackend,
    workflows::{
        agent::AgentRequest,
        assistant::{Answer, Asked, Conversation, Done, FileChange, Outcome, Step, Turn},
        Cancellation, CutOff, Feedback, RunStore,
    },
};

use super::Studio;
use crate::assistant::{self, Engine, Heard, Request, Scope};
pub(super) use changes::Changes;
use conversations::Confirm;

mod changes;
mod conversations;

pub(super) struct Assistant {
    /// The project's conversations, newest first. The one shown may be a
    /// new conversation not saved yet; there is always one.
    conversations: Vec<Conversation>,
    /// The id of the conversation shown.
    current: String,
    /// The project the conversations belong to.
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
    /// What the assistant changed that the writer has not kept or undone.
    pub(super) changes: Changes,
    /// Why the last request could not be sent, shown under the prompt.
    pub(super) notice: Option<String>,
    open_activity: HashSet<usize>,
    /// The shown conversation's title, in the column's header.
    title: Entity<InlineEdit>,
    /// Whether the saved conversations show beside the transcript.
    pub(super) conversations_open: bool,
    conversations_split: SplitState,
    /// The listed conversation whose menu is open, and where.
    menu: Option<(usize, Point<Pixels>)>,
    /// The last question asked, kept while its dialog fades out.
    confirm: Option<Confirm>,
    confirm_open: bool,
    /// Stands in for the Settings provider in tests.
    #[cfg(test)]
    pub(super) model: Option<storyfella_studio_core::workflows::framework::SharedStreamingModel>,
}

struct Running {
    /// The conversation it answers in, which need not be the one shown.
    conversation: String,
    cancel: Cancellation,
    step_started: Instant,
    /// The story when the request was sent, to count what it changed.
    started_with: storyfella_compiler::SourceWorkspace,
    /// The request, to carry on from when it runs out of steps.
    request: AgentRequest,
    _task: Task<()>,
    /// Redraws every second so the running step's time moves.
    _tick: Task<()>,
}

impl Assistant {
    /// A reply is being written.
    pub(super) fn is_running(&self) -> bool {
        self.running.is_some()
    }

    /// A reply is being written in the conversation `id`.
    fn running_in(&self, id: &str) -> bool {
        self.running
            .as_ref()
            .is_some_and(|running| running.conversation == id)
    }

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
        let conversation = Conversation::new();
        let title = cx.new(|cx| {
            InlineEdit::new(
                conversation.title.clone(),
                "Conversation title",
                theme,
                window,
                cx,
            )
        });
        cx.subscribe(&title, |studio, _, event: &InlineEditEvent, cx| {
            let InlineEditEvent::Committed(text) = event;
            studio.rename_conversation(text.to_string(), cx);
        })
        .detach();
        Self {
            current: conversation.id.clone(),
            conversations: vec![conversation],
            owner: None,
            transcript: TranscriptState::new(0),
            prompt,
            scope: Scope::default(),
            choosing_scope: false,
            keep_facts: true,
            running: None,
            streamed: String::new(),
            reasoned: 0,
            changes: Changes::default(),
            notice: None,
            open_activity: HashSet::new(),
            title,
            conversations_open: false,
            conversations_split: SplitState::new(px(180.)).limits(px(140.), px(360.)),
            menu: None,
            confirm: None,
            confirm_open: false,
            #[cfg(test)]
            model: None,
        }
    }

    pub(super) fn set_theme(&mut self, theme: Theme, cx: &mut Context<Studio>) {
        self.prompt
            .update(cx, |prompt, cx| prompt.set_theme(theme, cx));
        self.title
            .update(cx, |title, cx| title.set_theme(theme, cx));
        self.changes.set_theme(theme, cx);
    }

    /// The conversation shown.
    pub(super) fn current(&self) -> &Conversation {
        self.conversations
            .iter()
            .find(|conversation| conversation.id == self.current)
            .expect("the shown conversation is listed")
    }

    fn current_mut(&mut self) -> &mut Conversation {
        let id = self.current.clone();
        self.conversation_mut(&id)
            .expect("the shown conversation is listed")
    }

    fn conversation_mut(&mut self, id: &str) -> Option<&mut Conversation> {
        self.conversations
            .iter_mut()
            .find(|conversation| conversation.id == id)
    }

    #[cfg(test)]
    pub(super) fn conversations(&self) -> &[Conversation] {
        &self.conversations
    }

    /// The reply at `turn` of the conversation `id`.
    fn answer_mut(&mut self, id: &str, turn: usize) -> Option<&mut Answer> {
        match self.conversation_mut(id)?.turns.get_mut(turn) {
            Some(Turn::Answered(answer)) => Some(answer),
            _ => None,
        }
    }

    /// The project's run store and the project, when a project is open.
    fn store(&self) -> Option<anyhow::Result<(RunStore, PathBuf)>> {
        let project = self.owner.clone()?;
        Some(
            RunStore::open(&project.with_extension("workflows.sqlite3"))
                .map(|store| (store, project)),
        )
    }

    /// Saves the conversation `id`; a new one is saved once it has a
    /// question.
    fn save(&self, id: &str) -> Result<(), String> {
        let Some(conversation) = self
            .conversations
            .iter()
            .find(|conversation| conversation.id == id && !conversation.turns.is_empty())
        else {
            return Ok(());
        };
        let Some(store) = self.store() else {
            return Ok(());
        };
        store
            .and_then(|(store, project)| store.save_conversation(&project, conversation))
            .map_err(|error| format!("Could not save the conversation: {error}"))
    }

    /// Saves the assistant's changes nobody has decided on, so the bar and
    /// review outlive Studio.
    pub(super) fn save_undecided(
        &self,
        undecided: &BTreeMap<String, Option<String>>,
    ) -> Result<(), String> {
        let Some(project) = &self.owner else {
            return Ok(());
        };
        RunStore::open(&project.with_extension("workflows.sqlite3"))
            .and_then(|store| store.save_undecided(project, undecided))
            .map_err(|error| format!("Could not save the assistant's changes: {error}"))
    }
}

/// Lines a change adds and removes.
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
    /// Loads the open project's conversations when the project changes and
    /// shows the newest. What the assistant changed in the last project is
    /// the writer's now.
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
        self.assistant.streamed.clear();
        self.assistant.open_activity.clear();
        let saved = project.as_ref().map(|project| {
            RunStore::open(&project.with_extension("workflows.sqlite3"))
                .and_then(|store| Ok((store.conversations(project)?, store.undecided(project)?)))
        });
        let (conversations, undecided) = match saved {
            Some(Ok(saved)) => saved,
            Some(Err(error)) => {
                self.fail(ErrorCode::AppConversation, error.to_string(), cx);
                Default::default()
            }
            None => Default::default(),
        };
        self.restore_changes(undecided);
        self.assistant.owner = project;
        self.assistant.conversations = conversations;
        self.show_newest(cx);
    }

    fn engine(&self) -> Result<Engine, String> {
        #[cfg(test)]
        if let Some(model) = &self.assistant.model {
            return Ok(Engine::Model(model.clone()));
        }
        let backend = PlannerBackend::new(&self.config);
        if backend.has_llm() {
            Ok(Engine::Provider(Box::new(backend)))
        } else {
            Err("Choose a text provider in Settings to use the assistant.".into())
        }
    }

    /// Sends the prompt about what the scope names around the caret.
    pub(super) fn ask(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let text = self.assistant.prompt.read(cx).text(cx).trim().to_owned();
        if text.is_empty() || self.assistant.is_running() {
            return;
        }
        if self.assistant.changes.review.is_some() {
            return self.refuse("Finish the review before asking again.".into(), cx);
        }
        let engine = match self.engine() {
            Ok(engine) => engine,
            Err(message) => return self.refuse(message, cx),
        };
        let Some(file) = self.write.file().map(str::to_owned) else {
            return;
        };
        let selection = self.write.editor.read(cx).selection(cx);
        let request = match assistant::capture(
            &self.state,
            &file,
            self.assistant.scope,
            selection,
            &text,
            self.assistant.keep_facts,
        ) {
            Ok(request) => request,
            Err(message) => return self.refuse(message, cx),
        };
        self.assistant.notice = None;
        self.assistant
            .prompt
            .update(cx, |prompt, cx| prompt.set_text("", window, cx));
        self.send(text, request, engine, cx);
    }

    /// Carries on the request at `turn`, which ran out of steps.
    pub(super) fn continue_request(&mut self, turn: usize, cx: &mut Context<Self>) {
        if self.assistant.is_running() {
            return;
        }
        let turns = &self.assistant.current().turns;
        let Some((Turn::Asked(asked), Turn::Answered(answer))) = turn
            .checked_sub(1)
            .and_then(|asked| turns.get(asked))
            .zip(turns.get(turn))
        else {
            return;
        };
        let Outcome::Done(Done {
            continuation: Some(agent),
            ..
        }) = &answer.outcome
        else {
            return;
        };
        let request = Request {
            agent: AgentRequest::clone(agent),
            describe: asked.scope.clone(),
            place: asked.place.clone(),
        };
        let engine = match self.engine() {
            Ok(engine) => engine,
            Err(message) => return self.refuse(message, cx),
        };
        self.send("Continue".into(), request, engine, cx);
    }

    /// Records the request in the shown conversation and runs it.
    fn send(&mut self, text: String, request: Request, engine: Engine, cx: &mut Context<Self>) {
        let conversation = self.assistant.current_mut();
        let turn = conversation.ask(Asked {
            text,
            scope: request.describe,
            place: request.place,
        });
        let title = conversation.title.clone();
        self.assistant
            .title
            .update(cx, |field, cx| field.set_value(title, cx));
        self.assistant.transcript.push(2);
        self.assistant.transcript.scroll_to_end();
        self.assistant.streamed.clear();
        self.run_request(turn, request.agent, engine, cx);
    }

    /// Sends the request of the reply at `turn` again, about what the scope
    /// names now.
    pub(super) fn retry(&mut self, turn: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(Turn::Asked(asked)) = turn
            .checked_sub(1)
            .and_then(|asked| self.assistant.current().turns.get(asked))
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

    fn run_request(
        &mut self,
        turn: usize,
        request: AgentRequest,
        engine: Engine,
        cx: &mut Context<Self>,
    ) {
        let Some(store) = assistant::store_path(&self.state) else {
            return;
        };
        let started_with = match self.state.workspace_snapshot() {
            Ok(workspace) => workspace,
            Err(message) => return self.refuse(message, cx),
        };
        let conversation = self.assistant.current.clone();
        let cancel: Cancellation = Arc::new(AtomicBool::new(false));
        let (send, mut receive) = tokio::sync::mpsc::unbounded_channel();
        let worker = assistant::spawn(
            request.clone(),
            self.config.assistant,
            engine,
            self.state.registry.clone(),
            store,
            cancel.clone(),
            send,
        );
        // The channel closes when the worker is done with it, after which
        // its outcome is ready.
        let task = cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            while let Some(heard) = receive.recv().await {
                let alive = this.update(cx, |studio, cx| studio.heard(turn, heard, cx));
                if alive.is_err() {
                    return;
                }
            }
            let result = match worker {
                Ok(answer) => answer.await.unwrap_or_else(|_| {
                    Err(anyhow::anyhow!("The assistant stopped unexpectedly."))
                }),
                Err(error) => Err(anyhow::anyhow!("The assistant could not start: {error}")),
            };
            this.update(cx, |studio, cx| studio.answered(turn, result, cx))
                .ok();
        });
        let shown = conversation.clone();
        let tick = cx.spawn(
            async move |this: WeakEntity<Self>, cx: &mut AsyncApp| loop {
                cx.background_executor().timer(Duration::from_secs(1)).await;
                let alive = this.update(cx, |studio, cx| {
                    studio.assistant_changed(&shown, turn);
                    cx.notify();
                });
                if alive.is_err() {
                    return;
                }
            },
        );
        self.assistant.running = Some(Running {
            conversation: conversation.clone(),
            cancel,
            step_started: Instant::now(),
            started_with,
            request,
            _task: task,
            _tick: tick,
        });
        self.save_conversation(&conversation, cx);
        cx.notify();
    }

    /// The reply at `turn` of the conversation `id` changed size, which
    /// matters only while it shows.
    fn assistant_changed(&self, id: &str, turn: usize) {
        if self.assistant.current == id {
            self.assistant.transcript.changed(turn);
        }
    }

    /// Progress from the agent, or its call on the story.
    fn heard(&mut self, turn: usize, heard: Heard, cx: &mut Context<Self>) {
        let event = match heard {
            Heard::Draft(call) => return self.draft_call(call, cx),
            Heard::Feedback(event) => event,
        };
        let Some(running) = &self.assistant.running else {
            return;
        };
        let (id, started) = (running.conversation.clone(), running.step_started);
        let now = Instant::now();
        let streamed = streamed_detail(&self.assistant.streamed, self.assistant.reasoned, false);
        match event {
            Feedback::Status(label) => {
                if let Some(answer) = self.assistant.answer_mut(&id, turn) {
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
                    .answer_mut(&id, turn)
                    .and_then(|answer| answer.steps.last_mut())
                {
                    step.detail = Some(detail);
                }
            }
            Feedback::Text(text) => self.assistant.streamed.push_str(&text),
            Feedback::Reasoning(text) => self.assistant.reasoned += text.chars().count(),
            Feedback::Event(_) => return,
        }
        self.assistant_changed(&id, turn);
        cx.notify();
    }

    fn answered(&mut self, turn: usize, result: assistant::Finished, cx: &mut Context<Self>) {
        let Some(running) = self.assistant.running.take() else {
            return;
        };
        let id = running.conversation.clone();
        let running = Some(running);
        let outcome = match result {
            Ok(Some(outcome)) => {
                let now = self.state.workspace_snapshot().ok();
                let files = outcome
                    .changed
                    .iter()
                    .filter_map(|file| {
                        let before = running
                            .as_ref()
                            .and_then(|running| running.started_with.files.get(file))
                            .map_or("", String::as_str);
                        let after = now.as_ref()?.files.get(file)?;
                        let (added, removed) = line_counts(&Diff::new(before, after));
                        (added + removed > 0).then(|| FileChange {
                            file: file.clone(),
                            added,
                            removed,
                        })
                    })
                    .collect();
                let continuation =
                    running
                        .as_ref()
                        .filter(|_| outcome.out_of_steps)
                        .map(|running| {
                            Box::new(AgentRequest {
                                scope: outcome.scope.clone(),
                                editor: None,
                                continuing: Some(outcome.reply.clone()),
                                ..running.request.clone()
                            })
                        });
                Outcome::Done(Done {
                    reply: outcome.reply,
                    files,
                    out_of_steps: outcome.out_of_steps,
                    continuation,
                })
            }
            Ok(None) => Outcome::Stopped,
            Err(error) if error.is::<CutOff>() => Outcome::CutOff(format!("{error:#}")),
            Err(error) => Outcome::Failed(format!("{error:#}")),
        };
        let streamed = streamed_detail(&self.assistant.streamed, self.assistant.reasoned, false);
        if let Some(answer) = self.assistant.answer_mut(&id, turn) {
            close_step(
                answer,
                running.map_or(Duration::ZERO, |running| running.step_started.elapsed()),
                streamed,
            );
            answer.outcome = outcome;
        }
        if let Some(conversation) = self.assistant.conversation_mut(&id) {
            conversation.touch();
        }
        self.assistant.streamed.clear();
        self.assistant.reasoned = 0;
        self.assistant_changed(&id, turn);
        self.save_conversation(&id, cx);
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

    fn save_conversation(&mut self, id: &str, cx: &mut Context<Self>) {
        if let Err(message) = self.assistant.save(id) {
            self.fail(ErrorCode::AppConversation, message, cx);
        }
    }

    pub(super) fn render_assistant(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let c = self.theme;
        let studio = cx.entity().downgrade();
        let turns = self.assistant.current().turns.clone();
        let turns_empty = turns.is_empty();
        let running_here = self.assistant.running_in(&self.assistant.current);
        let running_elsewhere = self.assistant.is_running() && !running_here;
        let running_turn = running_here.then(|| turns.len().saturating_sub(1));
        let streamed = self.assistant.streamed.clone();
        let reasoned = self.assistant.reasoned;
        let started = self
            .assistant
            .running
            .as_ref()
            .map(|running| running.step_started);
        let reviewing = self.assistant.changes.review.is_some();
        let last = turns.len().saturating_sub(1);
        let idle = !self.assistant.is_running();
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
                        match asked.place.clone().filter(|_| !reviewing) {
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
                        browsing: !reviewing,
                        can_continue: index == last && idle,
                    },
                    open.contains(&index),
                    studio.clone(),
                ),
            }
        };
        let empty = self.assistant.prompt.read(cx).text(cx).trim().is_empty();
        let configured = self.engine().is_ok();
        let action = if running_here {
            c.icon_action("assistant-stop", Glyph::Stop, "Stop", ButtonKind::Secondary)
                .on_click(cx.listener(|studio, _, _, cx| studio.stop_assistant(cx)))
        } else {
            // Send takes the accent only when there is something to send.
            let ready = !empty && configured && !running_elsewhere;
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
        if !reviewing {
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
            bar = bar.option(c.chip(
                "assistant-reviewing",
                "Finish the review to ask again",
                true,
            ));
        }
        bar = bar.action(action);
        if !configured {
            bar = bar.message("Choose a text provider in Settings to use the assistant.");
        } else if running_elsewhere {
            bar = bar.message(
                "A reply is being written in another conversation. Wait for it, or stop it there.",
            );
        } else if let Some(notice) = &self.assistant.notice {
            bar = bar.message(notice.clone());
        }
        let shown = div()
            .size_full()
            .flex()
            .flex_col()
            .child(div().flex_1().min_h_0().child(if turns_empty {
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
                            .child("Ask for a change to the story"),
                    )
                    .child(
                        "Say what should change. The assistant edits the story as it \
                         works; keep or undo each change in the editor afterwards. \
                         Choose a passage below to limit what it may touch.",
                    )
                    .into_any_element()
            } else {
                Transcript::new("assistant-transcript", c, &self.assistant.transcript, item)
                    .into_any_element()
            }))
            .child(div().p(px(10.)).child(bar));
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
                    .gap(px(4.))
                    .pl(px(4.))
                    .pr(px(12.))
                    .border_b_1()
                    .border_color(c.hairline)
                    .child(self.render_conversations_toggle(cx))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .type_scale(TypeScale::Small)
                            .child(self.assistant.title.clone()),
                    )
                    .child(
                        c.icon_action(
                            "assistant-new",
                            Glyph::Plus,
                            "New conversation",
                            ButtonKind::Quiet,
                        )
                        .disabled(turns_empty)
                        .on_click(
                            cx.listener(|studio, _, window, cx| {
                                studio.new_conversation(window, cx)
                            }),
                        ),
                    )
                    .child(
                        c.switch("keep-facts", "Keep facts", self.assistant.keep_facts)
                            .on_click(cx.listener(|studio, _, _, cx| {
                                studio.assistant.keep_facts = !studio.assistant.keep_facts;
                                cx.notify();
                            })),
                    ),
            )
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .child(if self.assistant.conversations_open {
                        SplitView::new(
                            "assistant-conversations-split",
                            c,
                            &self.assistant.conversations_split,
                            self.render_conversation_column(cx),
                            shown,
                        )
                        .sized(SplitSide::Start)
                        .rest(px(240.))
                        .into_any_element()
                    } else {
                        shown.into_any_element()
                    }),
            )
            .child(self.render_conversation_confirm(cx))
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
/// the running step began.
struct Live<'a> {
    streamed: &'a str,
    reasoned: usize,
    step_started: Instant,
}

/// How a reply stands toward the rest of the column.
#[derive(Clone, Copy)]
struct Shown {
    /// No review holds the editor, so files can be opened.
    browsing: bool,
    /// It is the latest reply and nothing runs, so it can be carried on.
    can_continue: bool,
}

fn render_answer(
    index: usize,
    answer: &Answer,
    c: Theme,
    live: Option<&Live>,
    shown: Shown,
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
                (Outcome::Failed(_) | Outcome::CutOff(_), true) => ToolStatus::Failed("".into()),
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
                let preview = agentkit_ai::reasoning::visible(live.streamed);
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
        Outcome::Done(done) => {
            if !done.reply.is_empty() {
                message = message.markdown(done.reply.clone());
            }
            if !done.files.is_empty() || done.out_of_steps {
                message = message.block(done_card(index, done, c, shown, studio));
            }
        }
        Outcome::Failed(details) | Outcome::CutOff(details) => {
            let code = match &answer.outcome {
                Outcome::CutOff(_) => ErrorCode::WritingCutOff,
                _ => ErrorCode::WritingFailed,
            };
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

/// The files a request changed, each opening in the editor, and a way to
/// carry on when it ran out of steps.
fn done_card(
    index: usize,
    done: &Done,
    c: Theme,
    Shown {
        browsing,
        can_continue,
    }: Shown,
    studio: WeakEntity<Studio>,
) -> impl IntoElement {
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
    div()
        .flex()
        .flex_col()
        .gap(px(8.))
        .p(px(10.))
        .rounded(c.radius_control)
        .border_1()
        .border_color(c.hairline)
        .bg(c.raised)
        .children(done.files.iter().map(|change| {
            div()
                .flex()
                .items_center()
                .gap(px(6.))
                .type_scale(TypeScale::Small)
                .child(name(&change.file))
                .child(div().flex_1())
                .child(counts(change))
        }))
        .when(done.out_of_steps, |card| {
            card.child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .child(
                        div()
                            .flex_1()
                            .type_scale(TypeScale::Small)
                            .text_color(c.text_muted)
                            .child("It used all its steps and may not be finished."),
                    )
                    .when(can_continue && done.continuation.is_some(), |row| {
                        row.child(
                            c.button(("continue", index), "Continue", ButtonKind::Primary)
                                .on_click(move |_, _, cx| {
                                    studio
                                        .update(cx, |studio, cx| studio.continue_request(index, cx))
                                        .ok();
                                }),
                        )
                    }),
            )
        })
}
