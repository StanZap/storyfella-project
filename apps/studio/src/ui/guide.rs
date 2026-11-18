//! The card a running plan narrates itself in, with its pacing controls.
use design_system::{ButtonKind, Glyph, TypeScale, Typography};
use gpui_kit::{prelude::*, *};

use super::Studio;
use crate::driver::{GuideMode, GuideSession, GuideState};

const SPEEDS: [u32; 3] = [1, 2, 4];

impl Studio {
    fn update_guide(
        &mut self,
        act: impl FnOnce(&mut Option<GuideSession>),
        cx: &mut Context<Self>,
    ) {
        act(&mut self.guide);
        cx.notify();
    }

    pub(super) fn render_guide(&self, cx: &mut Context<Self>) -> Option<impl IntoElement> {
        let c = self.theme;
        let session = self.guide.as_ref()?;
        let total = session.plan.steps.len();
        let card = div()
            .id("guide")
            .absolute()
            .right(px(16.))
            .top(px(112.))
            .w(px(320.))
            .p(px(14.))
            .flex()
            .flex_col()
            .gap(px(10.))
            .rounded(c.radius_panel)
            .border_1()
            .border_color(c.hairline)
            .bg(c.surface)
            .shadow_lg()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(c.metrics.space)
                    .child(Glyph::Sparkle.render(c.accent, c.metrics.icon))
                    .child(
                        div()
                            .flex_1()
                            .font_weight(FontWeight::MEDIUM)
                            .truncate()
                            .child(session.plan.name.clone()),
                    ),
            );
        let card = match &session.state {
            GuideState::Running => {
                let message = session
                    .plan
                    .steps
                    .get(session.current)
                    .and_then(|step| step.message.clone())
                    .unwrap_or_else(|| "…".to_owned());
                card.child(
                    div()
                        .type_scale(TypeScale::Small)
                        .text_color(c.text_muted)
                        .child(format!(
                            "Step {} of {total}{}",
                            (session.current + 1).min(total),
                            if session.paused { " · paused" } else { "" }
                        )),
                )
                .child(div().child(message))
                .when_some(session.busy.clone(), |card, busy| {
                    card.child(
                        div()
                            .type_scale(TypeScale::Small)
                            .text_color(c.accent)
                            .child(busy),
                    )
                })
                .child(self.render_guide_controls(session, cx))
            }
            GuideState::Done(report) => {
                let failed = report.failed();
                let (verdict, color) = if failed == 0 {
                    (format!("All {} steps passed", report.passed()), c.success)
                } else {
                    (
                        format!("{failed} of {} steps failed", report.steps.len()),
                        c.error,
                    )
                };
                card.child(div().text_color(color).child(verdict)).child(
                    div().flex().justify_end().child(
                        c.button("guide-close", "Close", ButtonKind::Secondary)
                            .on_click(cx.listener(|studio, _, _, cx| {
                                studio.update_guide(|guide| *guide = None, cx)
                            })),
                    ),
                )
            }
        };
        Some(card)
    }

    fn render_guide_controls(
        &self,
        session: &GuideSession,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let c = self.theme;
        let paused = session.paused;
        div()
            .flex()
            .items_center()
            .gap(c.metrics.space)
            .child(
                c.icon_action(
                    "guide-pause",
                    if paused { Glyph::Play } else { Glyph::Pause },
                    if paused { "Resume" } else { "Pause" },
                    ButtonKind::Secondary,
                )
                .on_click(cx.listener(|studio, _, _, cx| {
                    studio.update_guide(
                        |guide| {
                            if let Some(guide) = guide {
                                guide.paused = !guide.paused;
                            }
                        },
                        cx,
                    )
                })),
            )
            .when(session.mode == GuideMode::Guide, |row| {
                row.children(SPEEDS.into_iter().map(|speed| {
                    let kind = if session.speed == speed {
                        ButtonKind::Secondary
                    } else {
                        ButtonKind::Quiet
                    };
                    c.button(("guide-speed", speed as usize), format!("{speed}×"), kind)
                        .on_click(cx.listener(move |studio, _, _, cx| {
                            studio.update_guide(
                                |guide| {
                                    if let Some(guide) = guide {
                                        guide.speed = speed;
                                    }
                                },
                                cx,
                            )
                        }))
                }))
            })
            .child(div().flex_1())
            .child(
                c.icon_action("guide-stop", Glyph::Stop, "Stop", ButtonKind::Quiet)
                    .on_click(cx.listener(|studio, _, _, cx| {
                        studio.update_guide(|guide| *guide = None, cx)
                    })),
            )
    }
}
