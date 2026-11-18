//! Play: the story's source beside its stage, or the stage alone.
//!
//! The player follows the caret. Moving it, by clicking a line or with the
//! keys, plays from the start of that scene up to the line. An edit plays the
//! story as it now stands, unsaved drafts included, once typing pauses. The
//! line on stage is lit in the source while its file is the one open.
use std::time::Duration;

use design_system::{ButtonKind, SplitSide, SplitState, SplitView, TypeScale, Typography};
use gpui_kit::{prelude::*, *};
use storyfella_language::{Source, Span};
use storyfella_studio_core::playback::PlaybackSession;

use super::{key, Studio, CONTEXT};
use crate::{host::Place, writing};

actions!(play, [ToggleFullWindow]);

pub(super) fn bind_keys(cx: &mut App) {
    cx.bind_keys([KeyBinding::new(
        key!("shift-enter"),
        ToggleFullWindow,
        Some(CONTEXT),
    )]);
}

/// How long typing must pause before the edited story plays.
const SETTLE: Duration = Duration::from_millis(250);
/// How long the caret must rest before the player follows it.
const REST: Duration = Duration::from_millis(60);

pub(super) struct Play {
    /// The stage has the whole window.
    pub(super) full: bool,
    /// The width of the source column, as the person dragged it.
    split: SplitState,
    /// The story changed since the player last compiled it.
    stale: bool,
    following: Option<Task<()>>,
}

impl Play {
    pub(super) fn new() -> Self {
        Self {
            full: false,
            split: SplitState::new(px(520.)).limits(px(280.), px(1100.)),
            stale: true,
            following: None,
        }
    }
}

/// The line on stage: the file it is written in and where.
pub(super) struct OnStage {
    pub(super) file: String,
    pub(super) span: Span,
}

impl Studio {
    /// Shows Play with the story as it stands, played to the caret.
    pub(super) fn open_play(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.place = Place::Play;
        self.replay(cx);
        if self.shown() == Place::Play {
            let focus = self.player.focus_handle(cx);
            window.focus(&focus, cx);
        }
    }

    pub(super) fn toggle_full_window(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.shown() != Place::Play {
            return;
        }
        self.play.full = !self.play.full;
        if !self.play.full && self.play.stale {
            self.replay(cx);
        }
        self.light_stage(cx);
        let focus = self.player.focus_handle(cx);
        window.focus(&focus, cx);
        cx.notify();
    }

    /// Compiles the story as it stands, drafts included, and plays it to
    /// the caret. A draft that does not compile leaves the player where it
    /// was, showing why.
    fn replay(&mut self, cx: &mut Context<Self>) {
        if self.state.project_path.is_none() {
            return;
        }
        self.play.stale = false;
        let session = crate::preview::compile(&self.state);
        let compiled = session.is_ok();
        self.install_preview(session, cx);
        if compiled {
            self.play_to_caret(cx);
        }
        self.light_stage(cx);
    }

    pub(super) fn install_preview(
        &mut self,
        session: Result<PlaybackSession, String>,
        cx: &mut Context<Self>,
    ) {
        self.player
            .update(cx, |player, cx| player.install(session, cx));
    }

    fn play_to_caret(&mut self, cx: &mut Context<Self>) {
        let (Some(file), Some(entry)) = (
            self.write.file().map(str::to_owned),
            writing::entry(&self.state),
        ) else {
            return;
        };
        let offset = self.write.editor.read(cx).cursor(cx);
        self.player.update(cx, |player, cx| {
            player.update_model(
                |model| {
                    model.play_to(&file, offset, &entry);
                },
                cx,
            )
        });
    }

    /// The caret moved, or the story changed: play there once it settles.
    pub(super) fn follow_caret(&mut self, story_changed: bool, cx: &mut Context<Self>) {
        self.play.stale |= story_changed;
        if self.shown() != Place::Play || self.play.full {
            return;
        }
        let wait = if self.play.stale { SETTLE } else { REST };
        self.play.following = Some(cx.spawn(
            async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
                cx.background_executor().timer(wait).await;
                this.update(cx, |studio, cx| {
                    if studio.play.stale {
                        studio.replay(cx);
                    } else {
                        studio.play_to_caret(cx);
                    }
                })
                .ok();
            },
        ));
    }

    /// The line on stage, while the player has one.
    pub(super) fn on_stage(&self, cx: &App) -> Option<OnStage> {
        let entry = writing::entry(&self.state)?;
        let player = self.player.read(cx);
        let (file, span) = player.model().session.as_ref()?.on_stage(&entry)?;
        Some(OnStage {
            file: file.to_owned(),
            span,
        })
    }

    /// "chapter.sf, line 18": where the line on stage is written.
    pub(super) fn stage_place(&self, cx: &App) -> Option<String> {
        if self.play.stale {
            return None;
        }
        let stage = self.on_stage(cx)?;
        let text = writing::source(&self.state, &stage.file)?;
        let line = Source::new(text).position(stage.span.start)?.line + 1;
        Some(format!("{}, line {line}", stage.file))
    }

    /// Lights the line on stage in the source beside it, and nothing
    /// anywhere else. While an edit waits to play, the light stays where
    /// the editor moved it.
    pub(super) fn light_stage(&mut self, cx: &mut Context<Self>) {
        let beside = self.shown() == Place::Play && !self.play.full;
        if beside && self.play.stale {
            return;
        }
        let lit = beside
            .then(|| self.on_stage(cx))
            .flatten()
            .filter(|stage| self.write.file() == Some(stage.file.as_str()))
            .map(|stage| stage.span.start..stage.span.end);
        if self.write.editor.read(cx).spotlight() != lit {
            self.write
                .editor
                .update(cx, |editor, cx| editor.set_spotlight(lit, cx));
        }
    }

    pub(super) fn render_play(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        if self.play.full {
            return div()
                .size_full()
                .child(self.player.clone())
                .into_any_element();
        }
        self.write.sync(&self.state, window, cx);
        self.light_stage(cx);
        SplitView::new(
            "play-source",
            self.theme,
            &self.play.split,
            div()
                .size_full()
                .flex()
                .flex_col()
                .child(self.render_source_bar(cx))
                .child(div().flex_1().min_h_0().child(self.write.editor.clone())),
            div().size_full().child(self.player.clone()),
        )
        .sized(SplitSide::Start)
        .rest(px(360.))
        .into_any_element()
    }

    /// The open file's name, and a way to it when the line on stage is
    /// written in another file.
    fn render_source_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let c = self.theme;
        let elsewhere = self
            .on_stage(cx)
            .filter(|stage| !self.play.stale && self.write.file() != Some(stage.file.as_str()));
        div()
            .h(px(36.))
            .flex_none()
            .flex()
            .items_center()
            .gap(px(2.))
            .pl(px(12.))
            .pr(px(4.))
            .border_b_1()
            .border_color(c.hairline)
            .bg(c.surface)
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .type_scale(TypeScale::Small)
                    .child(self.write.file().unwrap_or_default().to_owned()),
            )
            .when_some(elsewhere, |bar, stage| {
                bar.child(
                    c.button(
                        "show-stage-line",
                        format!("On stage in {}", stage.file),
                        ButtonKind::Quiet,
                    )
                    .on_click(cx.listener(move |studio, _, _, cx| {
                        studio.open_file(&stage.file, Some(stage.span.start), cx)
                    })),
                )
            })
    }
}
