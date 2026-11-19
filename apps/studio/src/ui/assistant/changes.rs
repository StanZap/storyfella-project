//! What the assistant changed and the writer has not decided on yet. Its
//! edits are in the story as soon as it makes them — saved like any other
//! edit — and the story's state remembers how each file was before the
//! assistant first touched it (`AppState::assistant_changes`), whether the
//! edit came from this column or a live MCP job. A bar above the editor says
//! so; Review shows each changed file as a diff against that, whose changes
//! are kept or undone one by one. Undoing every change to a file the
//! assistant created removes the file.
use std::collections::BTreeMap;

use design_system::{
    kits::code::{Diff, DiffView, DiffViewEvent, HunkState},
    ButtonKind, Glyph, Theme, TypeScale, Typography,
};
use gpui_kit::{prelude::*, *};
use storyfella_compiler::SourceWorkspace;

use storyfella_studio_core::errors::ErrorCode;

use super::Studio;
use crate::{assistant::DraftCall, host::Place};

#[derive(Default)]
pub(in crate::ui) struct Changes {
    /// The files the assistant changed that still differ, as of the last
    /// change to the story; reading them is not cheap enough to do on every
    /// frame.
    pub(in crate::ui) differing: Vec<String>,
    /// What was last saved to the run store, so the bar outlives Studio.
    saved: BTreeMap<String, Option<String>>,
    pub(in crate::ui) review: Option<Review>,
}

impl Changes {
    pub(super) fn set_theme(&self, theme: Theme, cx: &mut App) {
        if let Some(review) = &self.review {
            for (_, _, diff) in &review.files {
                diff.update(cx, |diff, cx| diff.set_theme(theme, cx));
            }
        }
    }
}

/// The changes under review in the editor's place, one file at a time.
pub(in crate::ui) struct Review {
    /// Each file, whether it existed before the assistant, and its diff.
    files: Vec<(String, bool, Entity<DiffView>)>,
    /// The file shown, as an index into `files`.
    current: usize,
    /// The story as the review opened on it.
    base: SourceWorkspace,
}

impl Review {
    /// The file shown and its diff.
    pub(in crate::ui) fn shown(&self) -> (&str, &Entity<DiffView>) {
        let (file, _, diff) = &self.files[self.current];
        (file, diff)
    }

    /// Changes decided, and changes in all, across every file.
    fn decided(&self, cx: &App) -> (usize, usize) {
        self.files
            .iter()
            .fold((0, 0), |(decided, of), (_, _, diff)| {
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
                    .2
                    .read(cx)
                    .states()
                    .contains(&HunkState::Pending)
            })
    }
}

impl Studio {
    /// Answers the running agent's call on the story: what it says now, or
    /// one of its changes, applied as one undoable change.
    pub(super) fn draft_call(&mut self, call: DraftCall, cx: &mut Context<Self>) {
        if call.answer(&mut self.state) {
            self.changed(cx);
        }
    }

    /// Each file the assistant changed, as it was and as it is — `None`
    /// where the story does not have it — leaving out files that are back to
    /// how they were.
    fn changed_files(&self) -> Vec<(String, Option<String>, Option<String>)> {
        let Ok(now) = self.state.workspace_snapshot() else {
            return Vec::new();
        };
        self.state
            .assistant_changes
            .iter()
            .filter_map(|(file, before)| {
                let after = now.files.get(file);
                (before.as_ref() != after).then(|| (file.clone(), before.clone(), after.cloned()))
            })
            .collect()
    }

    /// Follows a change to the story: a file put back as it was, by the
    /// writer or an undo, no longer waits for a decision. What still waits
    /// is saved with the project's conversation.
    pub(in crate::ui) fn refresh_changes(&mut self, cx: &mut Context<Self>) {
        let differing: Vec<String> = self
            .changed_files()
            .into_iter()
            .map(|(file, _, _)| file)
            .collect();
        self.state
            .assistant_changes
            .retain(|file, _| differing.contains(file));
        self.assistant.changes.differing = differing;
        if self.assistant.changes.saved != self.state.assistant_changes {
            let undecided = self.state.assistant_changes.clone();
            match self.assistant.save_undecided(&undecided) {
                Ok(()) => self.assistant.changes.saved = undecided,
                Err(message) => self.fail(ErrorCode::AppConversation, message, cx),
            }
        }
    }

    /// Loads what the assistant changed in the open project and nobody has
    /// decided on, from before Studio last closed.
    pub(super) fn restore_changes(&mut self, undecided: BTreeMap<String, Option<String>>) {
        self.assistant.changes = Changes {
            saved: undecided.clone(),
            ..Changes::default()
        };
        self.state.assistant_changes = undecided;
    }

    /// Opens the assistant's changes for review in the editor's place,
    /// starting with the first changed file.
    pub(in crate::ui) fn start_review(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.assistant.is_running() {
            return;
        }
        let texts = self.changed_files();
        if texts.is_empty() {
            self.refresh_changes(cx);
            return cx.notify();
        }
        let base = match self.state.workspace_snapshot() {
            Ok(base) => base,
            Err(message) => return self.refuse(message, cx),
        };
        let theme = self.theme;
        let files: Vec<_> = texts
            .into_iter()
            .map(|(file, before, after)| {
                let label = match (&before, &after) {
                    (None, _) => format!("{file}, which the assistant created"),
                    (_, None) => format!("{file}, which the assistant removed"),
                    _ => format!("The assistant's changes to {file}"),
                };
                let existed = before.is_some();
                let diff = Diff::new(
                    before.as_deref().unwrap_or_default(),
                    after.as_deref().unwrap_or_default(),
                );
                let view = cx.new(|_| DiffView::new(diff, theme).label(label));
                cx.subscribe_in(&view, window, |studio, _, event, window, cx| {
                    let DiffViewEvent::Decided { .. } = event;
                    studio.decided(window, cx);
                })
                .detach();
                (file, existed, view)
            })
            .collect();
        let first = files[0].0.clone();
        self.assistant.changes.review = Some(Review {
            files,
            current: 0,
            base,
        });
        self.open_file(&first, None, cx);
        self.show(Place::Write, window, cx);
    }

    /// A change was kept or undone: the review moves to the next file with
    /// something undecided, and ends once nothing is.
    fn decided(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(review) = &self.assistant.changes.review else {
            return;
        };
        let (decided, of) = review.decided(cx);
        if decided == of {
            if let Err(message) = self.finish_review(None, cx) {
                self.refuse(message, cx);
            }
        } else if !review
            .shown()
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
        let Some(review) = &mut self.assistant.changes.review else {
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

    /// Ends the open review. `Some(true)` keeps the changes not yet decided
    /// and `Some(false)` undoes them; a change decided one by one keeps its
    /// decision. What is undone, in every file, is one undoable edit, and the
    /// reviewed files are the writer's again. A file the assistant created
    /// that is left empty is removed, with its last change undone.
    pub(in crate::ui) fn finish_review(
        &mut self,
        all: Option<bool>,
        cx: &mut Context<Self>,
    ) -> Result<(), String> {
        let Some(review) = self.assistant.changes.review.take() else {
            return Ok(());
        };
        let mut texts = BTreeMap::new();
        for (file, existed, diff) in &review.files {
            let merged = diff.update(cx, |view, cx| {
                if let Some(all) = all {
                    let state = if all {
                        HunkState::Accepted
                    } else {
                        HunkState::Rejected
                    };
                    let states: Vec<_> = view
                        .states()
                        .iter()
                        .map(|decided| match decided {
                            HunkState::Pending => state,
                            decided => *decided,
                        })
                        .collect();
                    view.set_states(&states, cx);
                }
                view.merged()
            });
            let merged = (*existed || !merged.is_empty()).then_some(merged);
            if review.base.files.get(file) != merged.as_ref() {
                texts.insert(file.clone(), merged);
            }
        }
        if !texts.is_empty() {
            if let Err(message) = self.state.set_sources(&review.base, texts) {
                self.assistant.changes.review = Some(review);
                return Err(format!("{message}; nothing was undone."));
            }
        }
        for (file, _, _) in &review.files {
            self.state.assistant_changes.remove(file);
        }
        self.changed(cx);
        Ok(())
    }

    /// Keeps every change the assistant made: they are the writer's now.
    pub(in crate::ui) fn keep_all_changes(&mut self, cx: &mut Context<Self>) {
        if self.assistant.is_running() {
            return;
        }
        if self.assistant.changes.review.is_some() {
            if let Err(message) = self.finish_review(Some(true), cx) {
                self.refuse(message, cx);
            }
            return;
        }
        self.state.assistant_changes.clear();
        self.refresh_changes(cx);
        cx.notify();
    }

    /// Puts every file the assistant changed back as it was, as one
    /// undoable edit.
    pub(in crate::ui) fn undo_all_changes(&mut self, cx: &mut Context<Self>) {
        if self.assistant.is_running() {
            return;
        }
        if self.assistant.changes.review.is_some() {
            if let Err(message) = self.finish_review(Some(false), cx) {
                self.refuse(message, cx);
            }
            return;
        }
        let base = match self.state.workspace_snapshot() {
            Ok(base) => base,
            Err(message) => return self.refuse(message, cx),
        };
        let texts: BTreeMap<_, _> = self
            .changed_files()
            .into_iter()
            .map(|(file, before, _)| (file, before))
            .collect();
        if let Err(message) = self.state.set_sources(&base, texts) {
            return self.refuse(format!("{message}; nothing was undone."), cx);
        }
        self.state.assistant_changes.clear();
        self.changed(cx);
    }

    /// The bar above the editor while the assistant's changes wait: a summary
    /// with Review, or the review's own place and progress.
    pub(in crate::ui) fn render_changes_bar(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let c = self.theme;
        let running = self.assistant.is_running();
        let (summary, review) = match &self.assistant.changes.review {
            Some(review) => {
                let (decided, of) = review.decided(cx);
                let (shown, _) = review.shown();
                let count = review.files.len();
                let place = if count > 1 {
                    format!("{shown} ({} of {count})", review.current + 1)
                } else {
                    shown.to_owned()
                };
                (
                    format!(
                        "Reviewing the assistant's changes to {place} · {decided} of {of} decided"
                    ),
                    Some((review.current, count)),
                )
            }
            None => {
                let files = &self.assistant.changes.differing;
                if files.is_empty() {
                    return None;
                }
                let summary = match (files.as_slice(), running) {
                    ([file], true) => format!("The assistant is changing {file}"),
                    (files, true) => format!("The assistant is changing {} files", files.len()),
                    ([file], false) => format!("The assistant changed {file}"),
                    (files, false) => format!("The assistant changed {} files", files.len()),
                };
                (summary, None)
            }
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
        Some(
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
                        .child(summary),
                )
                .when_some(
                    review.filter(|(_, count)| *count > 1),
                    |bar, (current, count)| {
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
                    },
                )
                .when(review.is_none(), |bar| {
                    bar.child(
                        c.button("review-changes", "Review", ButtonKind::Secondary)
                            .disabled(running)
                            .on_click(
                                cx.listener(|studio, _, window, cx| {
                                    studio.start_review(window, cx)
                                }),
                            ),
                    )
                })
                .child(
                    c.button("undo-all", "Undo all", ButtonKind::Secondary)
                        .disabled(running)
                        .on_click(cx.listener(|studio, _, _, cx| studio.undo_all_changes(cx))),
                )
                .child(
                    c.button("keep-all", "Keep all", ButtonKind::Primary)
                        .disabled(running)
                        .on_click(cx.listener(|studio, _, _, cx| studio.keep_all_changes(cx))),
                )
                .into_any_element(),
        )
    }
}
