//! Draws a [`Frame`]: the backdrop, actors, placeholders and scene label
//! behind the reading card, and the passage inside it.
//!
//! Nothing here decides *what* appears; `studio_core::playback::stage` does.
//! This module only decides how it looks, so the panel and a headless capture
//! of the same frame cannot disagree.
use gpui_kit::{
    div, img, linear_color_stop, linear_gradient, prelude::*, px, relative, rgb, rgba, AnyElement,
    Div, FontWeight, Hsla, ObjectFit, Pixels, StyledImage,
};
use storyfella_compiler::ChoiceOption;
use storyfella_language::AssetKind;
use storyfella_studio_core::playback::{
    stage::{Frame, Passage, Portrait},
    PlaybackSession,
};

use crate::images::StageImages;

pub(crate) const INK: u32 = 0xeeeae2;
pub(crate) const MUTED: u32 = 0xb2b7bc;
pub(crate) const ACCENT: u32 = 0xdfb875;
pub(crate) const GROUND: u32 = 0x121619;

/// Running text and headings grow with the reader's text size setting.
#[derive(Clone, Copy)]
pub struct TextScale(pub f32);

impl TextScale {
    fn of(self, size: f32) -> Pixels {
        px(size * self.0)
    }
}

/// Everything drawn behind the reading card, as absolutely placed layers.
pub fn backdrop_layers(
    frame: &Frame,
    session: &PlaybackSession,
    images: &StageImages,
) -> Vec<AnyElement> {
    let mut layers = Vec::new();
    let backdrop = frame
        .backdrop
        .and_then(|backdrop| Some((backdrop, images.get(session, backdrop.visual)?)));
    if let Some((backdrop, image)) = backdrop {
        // A close shot crops in by drawing the backdrop 12% larger.
        let (size, offset) = if frame.effects.close_shot {
            (1.12, -0.06)
        } else {
            (1., 0.)
        };
        layers.push(
            div()
                .absolute()
                .inset_0()
                .overflow_hidden()
                .child(
                    img(image.image.clone())
                        .absolute()
                        .left(relative(offset))
                        .top(relative(offset))
                        .w(relative(size))
                        .h(relative(size))
                        .object_fit(if backdrop.contained {
                            ObjectFit::Contain
                        } else {
                            ObjectFit::Cover
                        })
                        .grayscale(frame.effects.monochrome),
                )
                .into_any_element(),
        );
    } else if frame.backdrop.is_some() || frame.backdrop_unavailable {
        layers.push(
            div()
                .absolute()
                .inset_0()
                .flex()
                .items_center()
                .justify_center()
                .bg(rgb(0x050607))
                .text_color(rgb(0x687078))
                .text_size(px(12.))
                .child("ARTWORK UNAVAILABLE")
                .into_any_element(),
        );
    }
    let (top, bottom) = if frame.effects.tense {
        (rgba(0x30101488), rgba(0x10090bf2))
    } else {
        (rgba(0x080d1240), rgba(0x080d12f2))
    };
    layers.push(
        div()
            .absolute()
            .inset_0()
            .bg(linear_gradient(
                180.,
                linear_color_stop(top, 0.),
                linear_color_stop(bottom, 1.),
            ))
            .into_any_element(),
    );
    layers.push(actors(frame, session, images).into_any_element());
    if !frame.placeholders.is_empty() {
        layers.push(placeholders(frame).into_any_element());
    }
    layers.push(scene_label(frame).into_any_element());
    layers
}

/// A zero-width anchor at an actor's centre line. Its child is centred on
/// it and may overflow both sides, since GPUI has no translate transform.
fn anchor(left: f32, bottom: f32, height: f32) -> Div {
    div()
        .absolute()
        .left(relative(left / 100.))
        .bottom(relative(bottom / 100.))
        .h(relative(height / 100.))
        .w(px(0.))
        .flex()
        .justify_center()
}

fn actors(frame: &Frame, session: &PlaybackSession, images: &StageImages) -> Div {
    let mut actors = frame.actors.clone();
    // Children paint in order, so deeper actors go first.
    actors.sort_by_key(|actor| actor.depth);
    div()
        .absolute()
        .left_0()
        .right_0()
        .top(relative(0.04))
        .bottom(relative(0.18))
        .overflow_hidden()
        .children(actors.into_iter().map(|actor| {
            let image = actor.visual.and_then(|visual| images.get(session, visual));
            let figure = match image {
                Some(image) => div().flex_none().h_full().aspect_ratio(image.aspect).child(
                    img(image.image.clone())
                        .size_full()
                        .object_fit(ObjectFit::Contain)
                        .grayscale(frame.effects.monochrome),
                ),
                None => div()
                    .flex_none()
                    .w(px(120.))
                    .h_full()
                    .p(px(8.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .border_1()
                    .border_dashed()
                    .border_color(rgb(ACCENT))
                    .bg(rgba(0x172128b8))
                    .text_color(rgb(INK))
                    .child(
                        div()
                            .w_full()
                            .text_center()
                            .child(format!("{} · missing artwork", actor.name)),
                    ),
            };
            anchor(actor.left, actor.bottom, actor.height).child(figure)
        }))
}

fn placeholders(frame: &Frame) -> Div {
    div()
        .absolute()
        .left_0()
        .right_0()
        .top(relative(0.08))
        .bottom(relative(0.12))
        .overflow_hidden()
        .children(frame.placeholders.iter().map(|placeholder| {
            let character = placeholder.kind == AssetKind::Character;
            let (width, height) = if character { (220., 78.) } else { (110., 32.) };
            anchor(placeholder.left, 0., height).child(
                div()
                    .flex_none()
                    .w(px(width))
                    .h_full()
                    .pt(if character {
                        relative(0.25)
                    } else {
                        px(20.).into()
                    })
                    .px(px(18.))
                    .flex()
                    .flex_col()
                    .gap(px(3.))
                    .items_center()
                    .border_1()
                    .border_color(rgba(0xc7d3d02b))
                    .when(!character, |this| this.rounded(px(12.)))
                    .bg(rgba(0xb7c8c31c))
                    .text_color(rgb(0xe3e8e5))
                    .text_center()
                    .child(
                        div()
                            .text_size(px(11.))
                            .text_color(rgb(0xaabbb7))
                            .child(if character { "Character" } else { "Prop" }),
                    )
                    .child(
                        div()
                            .text_size(px(13.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(placeholder.name.to_owned()),
                    ),
            )
        }))
}

fn scene_label(frame: &Frame) -> Div {
    let label = &frame.label;
    let heading = match label.beat {
        Some(beat) => format!("{} · {beat}", label.scene),
        None => label.scene.to_owned(),
    };
    let note = |text: &'static str| {
        div()
            .mt(px(4.))
            .text_size(px(11.))
            .text_color(rgb(0xd5bd8e))
            .child(text)
    };
    div()
        .absolute()
        .top(px(24.))
        .left(px(28.))
        .max_w(relative(0.8))
        .px(px(14.))
        .py(px(10.))
        .rounded(px(8.))
        .border_1()
        .border_color(rgba(0xffffff18))
        .bg(rgba(0x10191dc9))
        .text_size(px(12.))
        .text_color(rgb(0xd0d4d6))
        .child(heading)
        .when(label.missing_artwork, |this| {
            this.child(note("Missing artwork · positions are approximate"))
        })
        .when(label.ignored_directions, |this| {
            this.child(note("Unsupported visual directions are ignored in preview"))
        })
}

/// The reading card's surface. The caller adds the passage and controls.
pub fn reading_card(choice: bool) -> Div {
    div()
        .relative()
        .mt_auto()
        .mx_auto()
        .mb(px(30.))
        .w(relative(0.9))
        .max_w(px(if choice { 640. } else { 860. }))
        .px(px(32.))
        .pt(px(28.))
        .pb(px(20.))
        .flex()
        .flex_col()
        .rounded(px(14.))
        .border_1()
        .border_color(rgba(0xffffff20))
        .bg(rgba(0x11191eea))
        .text_color(rgb(INK))
}

pub fn heading(text: impl Into<gpui_kit::SharedString>, scale: TextScale) -> Div {
    div()
        .mb(px(16.))
        .text_size(scale.of(26.))
        .line_height(relative(1.4))
        .child(text.into())
}

pub fn secondary(text: impl Into<gpui_kit::SharedString>) -> Div {
    div()
        .text_size(px(14.))
        .line_height(relative(1.65))
        .text_color(rgb(MUTED))
        .child(text.into())
}

/// The passage inside the reading card. `choice` draws one option of a choice.
pub fn passage(
    passage: Passage,
    session: &PlaybackSession,
    images: &StageImages,
    scale: TextScale,
    monochrome: bool,
    mut choice: impl FnMut(usize, &ChoiceOption) -> AnyElement,
) -> Vec<AnyElement> {
    let story_text = |text: &str| {
        div()
            .text_size(scale.of(22.))
            .line_height(relative(1.45))
            .child(text.to_owned())
    };
    match passage {
        Passage::Complete => vec![heading("Story complete", scale).into_any_element()],
        Passage::Budget => vec![
            heading("A moment to pause", scale).into_any_element(),
            secondary(
                "Automatic transitions reached their limit. Continue another batch, or choose a scene in Options.",
            )
            .into_any_element(),
        ],
        Passage::Speech {
            speaker,
            thought,
            text,
            portrait,
        } => {
            let portrait = match portrait {
                Portrait::Image(visual) => images.get(session, visual).map(|image| {
                    img(image.image.clone())
                        .flex_none()
                        .mt(px(3.))
                        .size(px(48.))
                        .rounded_full()
                        .object_fit(ObjectFit::Cover)
                        .grayscale(monochrome)
                        .into_any_element()
                }),
                Portrait::Missing => Some(
                    div()
                        .flex_none()
                        .mt(px(3.))
                        .size(px(48.))
                        .rounded_full()
                        .flex()
                        .items_center()
                        .justify_center()
                        .border_1()
                        .border_color(rgba(0xffffff1c))
                        .bg(rgb(0x050607))
                        .text_color(rgb(0x687078))
                        .text_size(px(18.))
                        .child("?")
                        .into_any_element(),
                ),
                Portrait::None => None,
            };
            let name = if thought {
                format!("{speaker} · thought")
            } else {
                speaker.to_owned()
            };
            vec![div()
                .flex()
                .items_start()
                .gap(px(20.))
                .children(portrait)
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .child(
                            div()
                                .mb(px(6.))
                                .text_size(px(11.))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(rgb(0xdfbd82))
                                .child(name.to_uppercase()),
                        )
                        .child(story_text(text).when(thought, |this| this.italic())),
                )
                .into_any_element()]
        }
        Passage::Narration(text) => vec![story_text(text).into_any_element()],
        Passage::Choice { prompt, options } => vec![
            heading(prompt.to_owned(), scale)
                .mb(px(28.))
                .text_center()
                .into_any_element(),
            div()
                .flex()
                .flex_col()
                .gap(px(8.))
                .children(
                    options
                        .iter()
                        .enumerate()
                        .map(|(index, option)| choice(index, option)),
                )
                .into_any_element(),
        ],
        Passage::Silent => Vec::new(),
    }
}

pub(crate) fn hex(color: u32) -> Hsla {
    rgb(color).into()
}
