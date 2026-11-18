//! The stage as it stands, without the panel's header, controls or
//! overlays: what a capture of a preview shows. It draws with the same
//! [`stage`](crate::stage) pieces as the panel, so the two cannot disagree.
use gpui_kit::{div, prelude::*, px, rgb, Context, IntoElement, Render, Window};
use storyfella_studio_core::playback::{
    stage::{Frame, Passage, Viewer},
    PlaybackSession,
};

use crate::{
    images::StageImages,
    stage::{self, TextScale, GROUND, INK},
};

pub struct Still {
    session: PlaybackSession,
    images: StageImages,
    viewer: Viewer,
}

impl Still {
    pub fn new(session: PlaybackSession, viewer: Viewer) -> Self {
        Self {
            images: StageImages::decode(&session),
            session,
            viewer,
        }
    }
}

impl Render for Still {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let frame = Frame::new(&self.session, self.viewer);
        let scale = TextScale(self.viewer.settings.text_percent as f32 / 100.);
        let card = stage::reading_card(matches!(frame.passage, Passage::Choice { .. })).children(
            stage::passage(
                frame.passage,
                &self.session,
                &self.images,
                scale,
                frame.effects.monochrome,
                |_, option| {
                    stage::choice_row(div(), option.label.value.clone().into(), scale)
                        .into_any_element()
                },
            ),
        );
        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(rgb(GROUND))
            .text_color(rgb(INK))
            .text_size(px(14.))
            .child(stage::surface(
                stage::backdrop_layers(&frame, &self.session, &self.images),
                card,
            ))
    }
}
