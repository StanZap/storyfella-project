//! The window Studio opens instead when its configuration cannot load: what
//! went wrong, where the files are, and a way to try again once fixed.
use design_system::{ButtonKind, Density, Glyph, Theme, TypeScale, Typography};
use gpui_kit::{prelude::*, *};
use storyfella_studio_core::{config::ConfigError, errors::CodedError, paths};

use crate::driver::PlanLaunch;

struct Startup {
    error: String,
    code: String,
    launch: PlanLaunch,
}

pub fn open(error: ConfigError, launch: PlanLaunch, options: WindowOptions, cx: &mut App) {
    let report = error.report().log();
    cx.open_window(options, |_, cx| {
        cx.new(|_| Startup {
            error: report.details,
            code: report.code.code().to_owned(),
            launch,
        })
    })
    .expect("open the startup window");
    cx.on_window_closed(|cx, _| {
        if cx.windows().is_empty() {
            cx.quit();
        }
    })
    .detach();
}

impl Startup {
    /// Opens Studio again, or another of these, then closes this window, so
    /// the app always has a window open.
    fn retry(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let launch = self.launch.clone();
        let this = window.window_handle();
        cx.defer(move |cx| {
            super::open(launch, cx);
            this.update(cx, |_, window, _| window.remove_window()).ok();
        });
    }
}

impl Render for Startup {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let c = if matches!(
            window.appearance(),
            WindowAppearance::Light | WindowAppearance::VibrantLight
        ) {
            Theme::light()
        } else {
            Theme::dark()
        }
        .with_density(Density::Compact);
        let files = [
            ("Studio settings", paths::config_path()),
            ("Shared providers", paths::clipforge_config_path()),
        ];
        div()
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .bg(c.surface_deep)
            .text_color(c.text)
            .type_scale(TypeScale::Body)
            .child(
                div()
                    .w(px(520.))
                    .flex()
                    .flex_col()
                    .gap(px(16.))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(10.))
                            .child(Glyph::Warning.render(c.warning, px(20.)))
                            .child(
                                div()
                                    .type_scale(TypeScale::Title)
                                    .child("Studio's settings could not be read"),
                            ),
                    )
                    .child(div().child(self.error.clone()))
                    .child(
                        div()
                            .type_scale(TypeScale::Small)
                            .text_color(c.text_muted)
                            .child(format!("Code {}", self.code)),
                    )
                    .children(files.into_iter().filter_map(|(label, path)| {
                        let path = path?;
                        Some(
                            div()
                                .flex()
                                .flex_col()
                                .gap(px(2.))
                                .child(
                                    div()
                                        .type_scale(TypeScale::Small)
                                        .text_color(c.text_muted)
                                        .child(label),
                                )
                                .child(div().child(path.display().to_string())),
                        )
                    }))
                    .child(
                        div()
                            .flex()
                            .gap(px(8.))
                            .child(
                                c.button("reveal", "Show settings folder", ButtonKind::Secondary)
                                    .on_click(|_, _, cx| {
                                        if let Some(folder) =
                                            paths::config_path().and_then(|path| {
                                                path.parent().map(|parent| parent.to_path_buf())
                                            })
                                        {
                                            cx.reveal_path(&folder);
                                        }
                                    }),
                            )
                            .child(
                                c.button("retry", "Try again", ButtonKind::Primary)
                                    .on_click(cx.listener(|startup, _, window, cx| {
                                        startup.retry(window, cx)
                                    })),
                            ),
                    ),
            )
    }
}
