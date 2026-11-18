//! An open story: the bar that switches between Write, Play and Board, the
//! view itself, and the status bar beneath.
use design_system::{ButtonKind, Glyph, Segment, SegmentedControl, TypeScale, Typography};
use gpui_kit::{prelude::*, *};

use super::Studio;
use crate::{commands::OpActivity, host::Place};

const VIEWS: [Segment; 3] = [
    Segment::new("write", "Write"),
    Segment::new("play", "Play"),
    Segment::new("board", "Board"),
];
const PLACES: [Place; 3] = [Place::Write, Place::Play, Place::Board];

/// Room for the window's traffic lights at the bar's leading edge.
fn leading_inset() -> Pixels {
    if cfg!(target_os = "macos") {
        px(84.)
    } else {
        px(12.)
    }
}

impl Studio {
    pub(super) fn render_workspace(
        &mut self,
        view: Place,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        div()
            .size_full()
            .flex()
            .flex_col()
            .child(self.render_bar(view, cx))
            .child(
                div().flex_1().min_h_0().child(match view {
                    Place::Play => div()
                        .size_full()
                        .child(self.player.clone())
                        .into_any_element(),
                    Place::Write => self.render_write(window, cx).into_any_element(),
                    _ => self
                        .render_unported("Board", "the asset board")
                        .into_any_element(),
                }),
            )
            .child(self.render_status(view, cx))
    }

    fn render_bar(&self, view: Place, cx: &mut Context<Self>) -> impl IntoElement {
        let c = self.theme;
        let selected = PLACES.iter().position(|place| *place == view).unwrap_or(0);
        let studio = cx.entity().downgrade();
        let name = if self.state.project_name.is_empty() {
            "Untitled".to_owned()
        } else {
            self.state.project_name.clone()
        };
        div()
            .h(c.metrics.bar + px(8.))
            .flex_none()
            .flex()
            .items_center()
            .gap(c.metrics.space * 2.)
            .pl(leading_inset())
            .pr(px(12.))
            .border_b_1()
            .border_color(c.hairline)
            .bg(c.surface)
            .child(
                c.icon_action("home", Glyph::ArrowLeft, "Home", ButtonKind::Quiet)
                    .on_click(
                        cx.listener(|studio, _, window, cx| studio.show(Place::Home, window, cx)),
                    ),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .items_center()
                    .gap(c.metrics.space)
                    .child(div().type_scale(TypeScale::Title).truncate().child(name))
                    .when(self.state.has_unsaved_changes, |row| {
                        row.child(
                            div()
                                .type_scale(TypeScale::Small)
                                .text_color(c.text_muted)
                                .child("Edited"),
                        )
                    }),
            )
            .child(SegmentedControl::new(
                "views",
                "View",
                &VIEWS,
                selected,
                c,
                move |index, window, cx| {
                    studio
                        .update(cx, |studio, cx| studio.show(PLACES[index], window, cx))
                        .ok();
                },
            ))
            .child(div().flex_1())
            .child(
                c.icon_action("commands", Glyph::Search, "Commands", ButtonKind::Quiet)
                    .on_click(
                        cx.listener(|studio, _, window, cx| studio.set_palette(true, window, cx)),
                    ),
            )
            .child(
                c.icon_action("settings", Glyph::Settings, "Settings", ButtonKind::Quiet)
                    .on_click(cx.listener(|studio, _, window, cx| {
                        studio.show(Place::Settings, window, cx)
                    })),
            )
    }

    fn render_unported(&self, view: &str, what: &str) -> Div {
        let c = self.theme;
        div()
            .size_full()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap(px(6.))
            .text_color(c.text_muted)
            .child(
                div()
                    .type_scale(TypeScale::Title)
                    .text_color(c.text)
                    .child(view.to_owned()),
            )
            .child(format!("This build of Studio does not include {what} yet."))
    }

    fn render_status(&self, view: Place, cx: &mut Context<Self>) -> impl IntoElement {
        let c = self.theme;
        let (compile, compile_color) = match &self.problems {
            None => ("Checking…".to_owned(), c.text_muted),
            Some(Ok(problems)) if problems.errors > 0 => (problems.summary(), c.error),
            Some(Ok(problems)) if problems.warnings > 0 => (problems.summary(), c.warning),
            Some(Ok(problems)) => (problems.summary(), c.text_muted),
            Some(Err(message)) => (message.clone(), c.error),
        };
        let saved = if self.state.project_path.is_none() {
            "Not saved"
        } else if self.state.has_unsaved_changes {
            "Unsaved changes"
        } else {
            "Saved"
        };
        let item = || div().flex_none().flex().items_center().gap(c.metrics.space);
        div()
            .h(px(26.))
            .flex_none()
            .flex()
            .items_center()
            .gap(px(16.))
            .px(px(12.))
            .border_t_1()
            .border_color(c.hairline)
            .bg(c.surface)
            .type_scale(TypeScale::Caption)
            .text_color(c.text_muted)
            .child(item().text_color(compile_color).child(compile))
            .child(item().child(saved))
            .when_some(
                match &self.activity {
                    OpActivity::Running { summary } => Some(summary.clone()),
                    OpActivity::Idle => None,
                },
                |bar, summary| bar.child(item().text_color(c.accent).child(summary)),
            )
            .when_some(self.error.clone(), |bar, error| {
                let studio = cx.entity().downgrade();
                let shown = error.clone();
                bar.child(super::details::failure_indication(
                    "error",
                    c,
                    &error,
                    move |_, cx| {
                        studio
                            .update(cx, |studio, cx| studio.show_details(shown.clone(), cx))
                            .ok();
                    },
                ))
            })
            .when_some(self.hint.clone(), |bar, hint| {
                bar.child(
                    item()
                        .min_w_0()
                        .child(Glyph::Info.render(c.text_muted, px(12.)))
                        .child(div().truncate().child(hint)),
                )
            })
            .child(div().flex_1())
            .when(view == Place::Write, |bar| {
                bar.child(item().child(format!(
                    "Ln {}, Col {}",
                    self.caret.line + 1,
                    self.caret.character + 1
                )))
            })
            .when_some(self.state.project_path.as_ref(), |bar, path| {
                bar.child(div().min_w_0().truncate().child(path.display().to_string()))
            })
    }
}
