//! Home: start a story, open one, or pick up a recent one.
use std::time::SystemTime;

use design_system::{
    ButtonKind, Dialog, Glyph, ListBox, ListRow, TextField, TextFieldEvent, Theme, TypeScale,
    Typography,
};
use gpui_kit::{prelude::*, *};
use storyfella_studio_core::config::AppConfig;

use super::{Studio, TITLE};
use crate::recents::RecentProject;

pub struct Home {
    search: Entity<TextField>,
    name: Entity<TextField>,
    /// Naming a new story, in place of the New story button.
    creating: bool,
    pub(super) recents: Vec<RecentProject>,
    /// Projects that could not be read, and why.
    notices: Vec<String>,
    scanned: bool,
    highlighted: Option<usize>,
    /// The unreadable project whose details are open.
    pub(super) trouble: Option<RecentProject>,
    scroll: UniformListScrollHandle,
    list_focus: FocusHandle,
    scanning: Option<Task<()>>,
}

impl Home {
    pub fn new(_: &AppConfig, theme: Theme, window: &mut Window, cx: &mut Context<Studio>) -> Self {
        let search = cx.new(|cx| TextField::search("Search stories", theme, window, cx));
        let name = cx.new(|cx| TextField::new("Story name", theme, window, cx));
        cx.subscribe_in(&search, window, |studio, _, event, _, cx| {
            if let TextFieldEvent::Changed(_) = event {
                studio.home.highlighted = None;
                cx.notify();
            }
        })
        .detach();
        cx.subscribe_in(&name, window, |studio, _, event, window, cx| {
            if let TextFieldEvent::Submitted(text) = event {
                studio.create_named(text.to_string(), window, cx);
            }
        })
        .detach();
        Self {
            search,
            name,
            creating: false,
            recents: Vec::new(),
            notices: Vec::new(),
            scanned: false,
            highlighted: None,
            trouble: None,
            scroll: UniformListScrollHandle::new(),
            list_focus: cx.focus_handle(),
            scanning: None,
        }
    }

    /// Reads the projects directory again, off the UI thread.
    pub fn rescan(&mut self, config: &AppConfig, cx: &mut Context<Studio>) {
        let directory = config.project_dir.clone();
        self.scanning = Some(
            cx.spawn(async move |this: WeakEntity<Studio>, cx: &mut AsyncApp| {
                let (recents, notices) = cx
                    .background_spawn(async move { crate::recents::scan_projects(&directory) })
                    .await;
                this.update(cx, |studio, cx| {
                    let home = &mut studio.home;
                    home.recents = recents;
                    home.notices = notices;
                    home.scanned = true;
                    home.scanning = None;
                    cx.notify();
                })
                .ok();
            }),
        );
    }

    pub fn set_theme(&mut self, theme: Theme, cx: &mut Context<Studio>) {
        self.search
            .update(cx, |field, cx| field.set_theme(theme, cx));
        self.name.update(cx, |field, cx| field.set_theme(theme, cx));
    }

    pub fn start_creating(&mut self, window: &mut Window, cx: &mut Context<Studio>) {
        self.creating = true;
        self.name.update(cx, |field, cx| {
            field.set_error(None, cx);
            field.focus(window, cx);
        });
        cx.notify();
    }

    /// The recent projects the search matches, as indices into `recents`.
    fn matching(&self, cx: &App) -> Vec<usize> {
        let query = self.search.read(cx).text(cx);
        self.recents
            .iter()
            .enumerate()
            .filter(|(_, project)| project.matches_query(&query))
            .map(|(index, _)| index)
            .collect()
    }
}

/// "just now", "5 minutes ago", "3 days ago".
fn ago(when: SystemTime) -> String {
    let seconds = SystemTime::now()
        .duration_since(when)
        .map_or(0, |elapsed| elapsed.as_secs());
    let (count, unit) = match seconds {
        0..60 => return "just now".into(),
        60..3_600 => (seconds / 60, "minute"),
        3_600..86_400 => (seconds / 3_600, "hour"),
        86_400..2_592_000 => (seconds / 86_400, "day"),
        2_592_000..31_536_000 => (seconds / 2_592_000, "month"),
        _ => (seconds / 31_536_000, "year"),
    };
    format!("{count} {unit}{} ago", if count == 1 { "" } else { "s" })
}

fn detail(project: &RecentProject) -> String {
    let assets = match project.assets {
        1 => "1 asset".to_owned(),
        count => format!("{count} assets"),
    };
    match project.edited {
        Some(edited) => format!("{assets} · edited {}", ago(edited)),
        None => assets,
    }
}

impl Studio {
    fn create_named(&mut self, name: String, window: &mut Window, cx: &mut Context<Self>) {
        match self.create_project(&name, cx) {
            Ok(()) => {
                self.home.creating = false;
                self.home
                    .name
                    .update(cx, |field, cx| field.set_text("", window, cx));
                window.focus(&self.focus, cx);
            }
            Err(message) => self
                .home
                .name
                .update(cx, |field, cx| field.set_error(Some(message.into()), cx)),
        }
    }

    pub(super) fn render_home(
        &mut self,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let c = self.theme;
        let home = &self.home;
        let matching = home.matching(cx);
        let start = if home.creating {
            div()
                .flex()
                .gap(c.metrics.space * 2.)
                .items_start()
                .child(div().w(px(280.)).child(home.name.clone()))
                .child(
                    c.button("create", "Create", ButtonKind::Primary)
                        .on_click(cx.listener(|studio, _, window, cx| {
                            let name = studio.home.name.read(cx).text(cx).to_string();
                            studio.create_named(name, window, cx);
                        })),
                )
                .child(
                    c.button("cancel-create", "Cancel", ButtonKind::Quiet)
                        .on_click(cx.listener(|studio, _, window, cx| {
                            studio.home.creating = false;
                            window.focus(&studio.focus, cx);
                            cx.notify();
                        })),
                )
                .into_any_element()
        } else {
            div()
                .flex()
                .gap(c.metrics.space * 2.)
                .child(
                    c.button("new-story", "New story", ButtonKind::Primary)
                        .on_click(cx.listener(|studio, _, window, cx| {
                            studio.home.start_creating(window, cx)
                        })),
                )
                .child(
                    c.button("open-story", "Open…", ButtonKind::Secondary)
                        .on_click(cx.listener(|studio, _, _, cx| studio.prompt_open(cx))),
                )
                .into_any_element()
        };
        let list = if matching.is_empty() {
            let message = if !home.scanned {
                "Looking for stories…"
            } else if home.recents.is_empty() {
                "No stories yet. Start one, or open a story folder."
            } else {
                "No story matches the search."
            };
            div()
                .py(px(40.))
                .flex()
                .justify_center()
                .text_color(c.text_muted)
                .child(message)
                .into_any_element()
        } else {
            let rows: Vec<RecentProject> = matching
                .iter()
                .map(|&index| home.recents[index].clone())
                .collect();
            let len = rows.len();
            let theme = c;
            let opened = rows.clone();
            let studio = cx.entity().downgrade();
            let highlight = studio.clone();
            let details = studio.clone();
            ListBox::new(
                "recents",
                "Recent stories",
                c,
                &home.scroll,
                len,
                move |index, _, _| {
                    let project = &rows[index];
                    if let Some(trouble) = &project.trouble {
                        let details = details.clone();
                        let shown = rows[index].clone();
                        return ListRow::new(project.name.clone())
                            .glyph(Glyph::Folder)
                            .detail(if trouble.missing {
                                "Not found"
                            } else {
                                "Cannot open"
                            })
                            .trailing(
                                theme
                                    .icon_action(
                                        ("trouble", index),
                                        Glyph::Warning,
                                        "Why this story cannot open",
                                        ButtonKind::Quiet,
                                    )
                                    .on_click(move |_, _, cx| {
                                        cx.stop_propagation();
                                        let shown = shown.clone();
                                        details
                                            .update(cx, |studio, cx| studio.show_trouble(shown, cx))
                                            .ok();
                                    }),
                            );
                    }
                    let (status, color) = match &project.problems {
                        Ok(problems) if problems.errors > 0 => (problems.summary(), theme.error),
                        Ok(problems) if problems.warnings > 0 => {
                            (problems.summary(), theme.warning)
                        }
                        Ok(problems) => (problems.summary(), theme.text_muted),
                        Err(_) => ("Cannot read story".into(), theme.warning),
                    };
                    ListRow::new(project.name.clone())
                        .glyph(Glyph::Folder)
                        .detail(detail(project))
                        .trailing(
                            div()
                                .type_scale(TypeScale::Small)
                                .text_color(color)
                                .child(status),
                        )
                },
            )
            .focus(&home.list_focus)
            .highlighted(home.highlighted)
            .on_highlight(move |index, _, cx| {
                highlight
                    .update(cx, |studio, cx| {
                        studio.home.highlighted = Some(index);
                        cx.notify();
                    })
                    .ok();
            })
            .on_activate(move |index, _, cx| {
                let project = opened[index].clone();
                studio
                    .update(cx, |studio, cx| {
                        if project.trouble.is_some() {
                            studio.show_trouble(project, cx);
                        } else {
                            studio.open_project(&project.path, cx);
                        }
                    })
                    .ok();
            })
            .into_any_element()
        };
        div()
            .id("home")
            .size_full()
            .flex()
            .flex_col()
            .items_center()
            .child(
                div()
                    .w_full()
                    .h_full()
                    .max_w(px(720.))
                    .px(px(24.))
                    .pt(px(88.))
                    .pb(px(32.))
                    .flex()
                    .flex_col()
                    .gap(px(28.))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(6.))
                            .child(div().type_scale(TypeScale::Heading).child(TITLE))
                            .child(
                                div()
                                    .text_color(c.text_muted)
                                    .child("Write a story, play it, and give it art."),
                            ),
                    )
                    .child(start)
                    .child(
                        div()
                            .flex_1()
                            .min_h_0()
                            .flex()
                            .flex_col()
                            .gap(c.metrics.space * 2.)
                            .child(
                                div()
                                    .type_scale(TypeScale::Small)
                                    .text_color(c.text_muted)
                                    .child("Recent"),
                            )
                            .child(home.search.clone())
                            .child(
                                div()
                                    .flex_1()
                                    .min_h_0()
                                    .rounded(c.radius_panel)
                                    .border_1()
                                    .border_color(c.hairline)
                                    .bg(c.surface)
                                    .overflow_hidden()
                                    .child(list),
                            )
                            .children(home.notices.iter().map(|notice| {
                                div()
                                    .flex()
                                    .gap(c.metrics.space)
                                    .items_center()
                                    .type_scale(TypeScale::Small)
                                    .text_color(c.warning)
                                    .child(Glyph::Warning.render(c.warning, c.metrics.icon))
                                    .child(div().truncate().child(notice.clone()))
                            })),
                    ),
            )
    }

    pub(super) fn show_trouble(&mut self, project: RecentProject, cx: &mut Context<Self>) {
        self.home.trouble = Some(project);
        cx.notify();
    }

    pub(super) fn forget(&mut self, path: &std::path::Path, cx: &mut Context<Self>) {
        match crate::recents::forget_project(&self.config.project_dir, path) {
            Ok(()) => {
                self.home.trouble = None;
                self.home.highlighted = None;
                self.home.rescan(&self.config, cx);
            }
            Err(message) => self.report_message(message, cx),
        }
        cx.notify();
    }

    /// Why a listed story cannot open, drawn over the window.
    pub(super) fn render_trouble(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let c = self.theme;
        let studio = cx.entity().downgrade();
        let close = studio.clone();
        let dismiss = studio.clone();
        let project = self.home.trouble.clone();
        let title = project.as_ref().map_or_else(String::new, |project| {
            format!("{} cannot open", project.name)
        });
        let mut dialog = Dialog::new("story-trouble", title, c)
            .open(project.is_some())
            .width(px(560.));
        if let Some((project, trouble)) = project.and_then(|project| {
            let trouble = project.trouble.clone()?;
            Some((project, trouble))
        }) {
            let path = project.path.clone();
            dialog = dialog.child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(10.))
                    .child(if trouble.missing {
                        "Nothing is at this path any more. It may have been moved or deleted."
                    } else {
                        "Studio could not read this story. It may have been saved by an older version of Studio."
                    })
                    .child(
                        div()
                            .type_scale(TypeScale::Small)
                            .text_color(c.text_muted)
                            .child(path.display().to_string()),
                    )
                    .when(!trouble.missing, |body| {
                        body.child(
                            div()
                                .id("trouble-message")
                                .max_h(px(220.))
                                .overflow_y_scroll()
                                .p(px(10.))
                                .rounded(c.radius_control)
                                .bg(c.surface_deep)
                                .code_text()
                                .type_scale(TypeScale::Small)
                                .whitespace_normal()
                                .children(
                                    trouble
                                        .message
                                        .lines()
                                        .map(|line| div().child(line.to_owned()))
                                        .collect::<Vec<_>>(),
                                ),
                        )
                    }),
            );
            if trouble.remembered {
                let forget = studio.clone();
                let path = path.clone();
                dialog = dialog.action(
                    c.button("forget-story", "Remove from list", ButtonKind::Secondary)
                        .on_click(move |_, _, cx| {
                            forget
                                .update(cx, |studio, cx| studio.forget(&path, cx))
                                .ok();
                        }),
                );
            }
            if !trouble.missing {
                dialog = dialog.action(
                    c.button("reveal-story", "Show in Finder", ButtonKind::Secondary)
                        .on_click(move |_, _, cx| cx.reveal_path(&path)),
                );
            }
        }
        dialog
            .action(
                c.button("close-trouble", "Close", ButtonKind::Primary)
                    .on_click(move |_, _, cx| {
                        close
                            .update(cx, |studio, cx| {
                                studio.home.trouble = None;
                                cx.notify();
                            })
                            .ok();
                    }),
            )
            .on_dismiss(move |_, cx| {
                dismiss
                    .update(cx, |studio, cx| {
                        studio.home.trouble = None;
                        cx.notify();
                    })
                    .ok();
            })
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, SystemTime};

    use super::ago;

    #[test]
    fn edits_read_as_elapsed_time() {
        let now = SystemTime::now();
        assert_eq!(ago(now), "just now");
        assert_eq!(ago(now - Duration::from_secs(60)), "1 minute ago");
        assert_eq!(ago(now - Duration::from_secs(7_200)), "2 hours ago");
        assert_eq!(ago(now - Duration::from_secs(3 * 86_400)), "3 days ago");
    }
}
