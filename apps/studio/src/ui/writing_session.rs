//! Autosave the writing workspace independently of explicit story saves.
use super::writing_review::{ReviewController, ReviewMode};
use crate::state::AppState;
use dioxus::prelude::*;
use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};
use storyfella_studio_core::errors::ReportedError;
use storyfella_studio_core::workflows::{
    writing::{WritingAction, WritingRequest},
    writing_session::WritingSession,
    RunStore,
};

#[derive(Clone, Copy)]
pub struct SessionFields {
    pub scope: Signal<String>,
    pub action: Signal<WritingAction>,
    pub instructions: Signal<String>,
    pub preserve: Signal<bool>,
    pub request: Signal<Option<WritingRequest>>,
    pub request_open: Signal<bool>,
    pub streamed: Signal<String>,
    pub phase: Signal<String>,
    pub cancellation: Signal<Arc<AtomicBool>>,
    pub interrupted: Signal<bool>,
    pub message: Signal<Option<String>>,
    pub failure: Signal<Option<ReportedError>>,
}

impl SessionFields {
    pub fn restore(mut self, mut controller: ReviewController, session: Option<&WritingSession>) {
        self.cancellation.peek().store(true, Ordering::Relaxed);
        self.cancellation.set(Arc::new(AtomicBool::new(false)));
        controller.busy.set(false);
        self.scope
            .set(session.map_or_else(|| "beat".into(), |s| s.scope.clone()));
        self.action
            .set(session.map_or(WritingAction::default(), |s| s.action));
        self.instructions
            .set(session.map_or_else(String::new, |s| s.instructions.clone()));
        self.preserve.set(session.is_none_or(|s| s.preserve_facts));
        self.request.set(session.and_then(|s| s.request.clone()));
        self.request_open
            .set(session.is_none_or(|s| s.request_open));
        self.streamed
            .set(session.map_or_else(String::new, |s| s.streamed.clone()));
        self.phase
            .set(session.map_or_else(String::new, |s| s.phase.clone()));
        self.interrupted.set(session.is_some_and(|s| s.generating));
        self.message.set(None);
        self.failure.set(None);
        controller
            .review
            .set(session.and_then(|s| s.review.clone()));
        controller
            .mode
            .set(if session.is_some_and(|s| s.side_by_side) {
                ReviewMode::SideBySide
            } else {
                ReviewMode::Inline
            });
        controller.selected.set(session.map_or(0, |s| s.selected));
        controller
            .marked
            .set(session.map_or_else(Vec::new, |s| s.marked.clone()));
        controller.sequence.set(0);
        controller.ready.set(false);
        controller.changes.set(Vec::new());
        controller.error.set(None);
    }
}

#[derive(Clone, Copy)]
pub struct SessionRecovery {
    pub conflict: Signal<Option<WritingSession>>,
    pub error: Signal<Option<String>>,
}

type Binding = (PathBuf, PathBuf, Arc<RunStore>);

pub fn use_writing_session(
    mut state: Signal<AppState>,
    controller: ReviewController,
    fields: SessionFields,
    mut open: Signal<bool>,
) -> SessionRecovery {
    let mut attempted = use_signal(|| None::<(PathBuf, PathBuf)>);
    let mut binding = use_signal(|| None::<Binding>);
    let mut last_saved = use_signal(|| None::<WritingSession>);
    let mut recovery = SessionRecovery {
        conflict: use_signal(|| None),
        error: use_signal(|| None),
    };
    use_effect(move || {
        let owner = {
            let state = state.read();
            state
                .project_path
                .clone()
                .zip(state.story.path().map(PathBuf::from))
        };
        if *attempted.peek() == owner {
            return;
        }
        attempted.set(owner.clone());
        binding.set(None);
        last_saved.set(None);
        recovery.conflict.set(None);
        recovery.error.set(None);
        fields.restore(controller, None);
        let Some((project, path)) = owner else {
            return;
        };
        let loaded = (|| -> anyhow::Result<_> {
            let store = Arc::new(RunStore::open(
                &project.with_extension("workflows.sqlite3"),
            )?);
            let session = store.writing_session(&project, &path)?;
            Ok((store, session))
        })();
        match loaded {
            Ok((store, session)) => {
                if let Some(session) = session {
                    if session.request.is_some()
                        || session.review.is_some()
                        || session.generating
                        || !session.instructions.is_empty()
                    {
                        open.set(true);
                    }
                    if session.can_restore(state.peek().story.source()) {
                        if session.has_source_context() {
                            state.write().set_story_source(session.draft.clone());
                        }
                        fields.restore(controller, Some(&session));
                    } else {
                        open.set(true);
                        recovery.conflict.set(Some(session));
                    }
                }
                binding.set(Some((project, path, store)));
            }
            Err(error) => {
                open.set(true);
                recovery
                    .error
                    .set(Some(format!("Couldn't open writing recovery: {error}")));
            }
        }
    });
    use_effect(move || {
        let Some((project, path, store)) = binding() else {
            return;
        };
        if recovery.conflict.read().is_some() {
            return;
        }
        let state = state.read();
        if state.project_path.as_ref() != Some(&project)
            || state.story.path() != Some(path.as_path())
        {
            return;
        }
        let snapshot = WritingSession {
            project,
            source_path: path,
            saved_source: state.story.saved_source().into(),
            draft: state.story.source().into(),
            scope: (fields.scope)(),
            action: (fields.action)(),
            instructions: (fields.instructions)(),
            preserve_facts: (fields.preserve)(),
            request: (fields.request)(),
            request_open: (fields.request_open)(),
            review: (controller.review)(),
            generating: (controller.busy)() || (fields.interrupted)(),
            streamed: (fields.streamed)(),
            phase: (fields.phase)(),
            side_by_side: (controller.mode)() == ReviewMode::SideBySide,
            selected: (controller.selected)(),
            marked: (controller.marked)(),
        };
        if last_saved.peek().as_ref() == Some(&snapshot) {
            return;
        }
        match store.save_writing_session(&snapshot) {
            Ok(()) => {
                last_saved.set(Some(snapshot));
                recovery.error.set(None);
            }
            Err(error) => recovery
                .error
                .set(Some(format!("Couldn't save writing recovery: {error}"))),
        }
    });
    recovery
}
