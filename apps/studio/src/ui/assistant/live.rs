//! A project request's files while they are written: each file's state in
//! the running reply, and one of them watched in the editor's place as a
//! diff that fills in as its fragments arrive. Nothing here changes the
//! story; the files become a suggestion when the run ends, or stops.
use design_system::{
    kits::code::{Diff, DiffView},
    Theme, TypeScale, Typography,
};
use gpui_kit::{prelude::*, *};
use storyfella_studio_core::workflows::FileState;

use super::{line_counts, Studio};
use crate::host::Place;

/// One file of the running request.
#[derive(Clone)]
pub(super) struct LiveFile {
    pub(super) file: String,
    /// The file as the request found it.
    before: String,
    text: String,
    pub(super) state: FileState,
    added: usize,
    removed: usize,
}

/// The file shown being written.
pub(in crate::ui) struct Watch {
    pub(in crate::ui) file: String,
    pub(in crate::ui) view: Entity<DiffView>,
}

impl Studio {
    /// A file's progress: its text so far and how it stands.
    pub(in crate::ui) fn heard_file(
        &mut self,
        file: String,
        text: String,
        state: FileState,
        cx: &mut Context<Self>,
    ) {
        let live = &mut self.assistant.live;
        let index = match live.iter().position(|item| item.file == file) {
            Some(index) => index,
            None => {
                // A file's first report is its text before anything is written.
                live.push(LiveFile {
                    file: file.clone(),
                    before: text.clone(),
                    text: String::new(),
                    state: state.clone(),
                    added: 0,
                    removed: 0,
                });
                live.len() - 1
            }
        };
        let item = &mut live[index];
        let diff = Diff::new(&item.before, &text);
        (item.added, item.removed) = line_counts(&diff);
        item.text = text;
        item.state = state;
        if let Some(watch) = &self.assistant.watching {
            if watch.file == file {
                watch.view.update(cx, |view, cx| view.set_diff(diff, cx));
            }
        }
    }

    /// Shows `file` being written in the editor's place.
    pub(in crate::ui) fn watch(&mut self, file: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(item) = self.assistant.live.iter().find(|item| item.file == file) else {
            return;
        };
        let diff = Diff::new(&item.before, &item.text);
        let label = format!("{file} as it is being written");
        let theme = self.theme;
        let view = cx.new(|_| DiffView::new(diff, theme).label(label).decisions(false));
        self.assistant.watching = Some(Watch {
            file: file.to_owned(),
            view,
        });
        self.open_file(file, None, cx);
        self.show(Place::Write, window, cx);
        cx.notify();
    }

    pub(super) fn stop_watching(&mut self, cx: &mut Context<Self>) {
        self.assistant.watching = None;
        cx.notify();
    }

    /// The bar above a file being watched.
    pub(in crate::ui) fn render_watch_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let c = self.theme;
        let Some(watch) = &self.assistant.watching else {
            return div();
        };
        let state = self
            .assistant
            .live
            .iter()
            .find(|item| item.file == watch.file)
            .map_or_else(String::new, |item| {
                format!(" · {}", state_label(&item.state))
            });
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
            .child(design_system::Glyph::Sparkle.render(c.accent, px(12.)))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .type_scale(TypeScale::Small)
                    .child(format!(
                        "Watching {} being written{state} · nothing is kept until you review it",
                        watch.file
                    )),
            )
            .child(
                c.button(
                    "stop-watching",
                    "Back to the file",
                    design_system::ButtonKind::Secondary,
                )
                .on_click(cx.listener(|studio, _, _, cx| studio.stop_watching(cx))),
            )
    }
}

/// How a file stands, in a few words.
fn state_label(state: &FileState) -> String {
    match state {
        FileState::Writing { parts: 1, .. } => "Writing".into(),
        FileState::Writing { part, parts } => format!("Writing part {part} of {parts}"),
        FileState::Fixing => "Fixing".into(),
        FileState::Compiles => "Compiles".into(),
        FileState::Problems(problems) => match problems.len() {
            1 => "1 problem".into(),
            count => format!("{count} problems"),
        },
    }
}

/// The running request's files, each opening its live diff from its name
/// while no review holds the editor.
pub(super) fn live_files(
    files: &[LiveFile],
    c: Theme,
    browsing: bool,
    studio: WeakEntity<Studio>,
) -> impl IntoElement {
    div()
        .flex()
        .flex_col()
        .gap(px(2.))
        .p(px(10.))
        .rounded(c.radius_control)
        .border_1()
        .border_color(c.hairline)
        .bg(c.raised)
        .type_scale(TypeScale::Small)
        .children(files.iter().map(|item| {
            let tone = match &item.state {
                FileState::Compiles => c.success,
                FileState::Problems(_) => c.warning,
                _ => c.text_muted,
            };
            let name = if browsing {
                let (studio, file) = (studio.clone(), item.file.clone());
                c.link(
                    SharedString::from(format!("watch-{}", item.file)),
                    item.file.clone(),
                )
                .text_color(c.text)
                .on_click(move |_, window, cx| {
                    studio
                        .update(cx, |studio, cx| studio.watch(&file, window, cx))
                        .ok();
                })
                .into_any_element()
            } else {
                div()
                    .min_w_0()
                    .truncate()
                    .child(item.file.clone())
                    .into_any_element()
            };
            div()
                .flex()
                .items_center()
                .gap(px(6.))
                .child(name)
                .child(div().flex_1())
                .child(
                    div()
                        .flex_none()
                        .type_scale(TypeScale::Caption)
                        .text_color(tone)
                        .child(state_label(&item.state)),
                )
                .child(
                    div()
                        .flex_none()
                        .flex()
                        .gap(px(4.))
                        .type_scale(TypeScale::Caption)
                        .code_text()
                        .child(
                            div()
                                .text_color(c.success)
                                .child(format!("+{}", item.added)),
                        )
                        .child(
                            div()
                                .text_color(c.error)
                                .child(format!("−{}", item.removed)),
                        ),
                )
        }))
}
