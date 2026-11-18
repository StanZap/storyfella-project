//! How Studio says something went wrong. Where it happened shows only an
//! indication: the error code's short label, or a sentence of guidance. The
//! explanation — what to do, what happened, the code — opens on request in
//! one dialog, whichever window or surface the indication is on.
use std::rc::Rc;

use design_system::{ButtonKind, Dialog, Theme, TypeScale, Typography};
use gpui_kit::{prelude::*, *};
use storyfella_studio_core::errors::{ErrorCode, ReportedError};

use super::Studio;

/// The details of `error`, open while it is `Some`. `close` runs on Close and
/// on dismissal.
pub(super) fn details_dialog(
    theme: Theme,
    error: Option<&ReportedError>,
    close: impl Fn(&mut App) + 'static,
) -> Dialog {
    let c = theme;
    let close = Rc::new(close);
    let dismiss = close.clone();
    let title = error.map_or("", |error| error.code.message());
    let mut dialog = Dialog::new("error-details", title, c)
        .open(error.is_some())
        .width(px(520.));
    if let Some(error) = error {
        let copy = error.copy_text();
        let lines: Vec<String> = error
            .details
            .lines()
            .map(str::to_owned)
            .chain(
                error
                    .context
                    .iter()
                    .map(|(key, value)| format!("{key}: {value}")),
            )
            .collect();
        dialog = dialog
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(10.))
                    .child(error.code.hint())
                    .child(
                        div()
                            .id("error-details-text")
                            .max_h(px(220.))
                            .overflow_y_scroll()
                            .p(px(10.))
                            .rounded(c.radius_control)
                            .bg(c.surface_deep)
                            .code_text()
                            .type_scale(TypeScale::Small)
                            .whitespace_normal()
                            .children(lines.into_iter().map(|line| div().child(line))),
                    )
                    .child(
                        div()
                            .type_scale(TypeScale::Caption)
                            .text_color(c.text_muted)
                            .child(format!("Code {}", error.code.code())),
                    ),
            )
            .action(
                c.button("copy-error", "Copy details", ButtonKind::Secondary)
                    .on_click(move |_, _, cx| {
                        cx.write_to_clipboard(ClipboardItem::new_string(copy.clone()))
                    }),
            );
    }
    dialog
        .action(
            c.button("close-error", "Close", ButtonKind::Primary)
                .on_click(move |_, _, cx| close(cx)),
        )
        .on_dismiss(move |_, cx| dismiss(cx))
}

/// The indication of a failure: its short label, opening the details.
pub(super) fn failure_indication(
    id: impl Into<ElementId>,
    theme: Theme,
    error: &ReportedError,
    open: impl Fn(&mut Window, &mut App) + 'static,
) -> Stateful<Div> {
    let c = theme;
    div()
        .id(id)
        .flex()
        .min_w_0()
        .items_center()
        .gap(c.metrics.space)
        .text_color(c.error)
        .cursor_pointer()
        .child(design_system::Glyph::Warning.render(c.error, px(12.)))
        .child(div().truncate().child(error.code.message()))
        .on_click(move |_, window, cx| open(window, cx))
}

impl Studio {
    /// Reports a failure: logged once with its code, indicated in the status
    /// bar, explained in the details dialog.
    pub(super) fn report(&mut self, error: ReportedError, cx: &mut Context<Self>) {
        self.hint = None;
        self.error = Some(error.log());
        cx.notify();
    }

    pub(super) fn fail(
        &mut self,
        code: ErrorCode,
        details: impl Into<String>,
        cx: &mut Context<Self>,
    ) {
        self.report(ReportedError::plain(code, details), cx);
    }

    /// Guidance on what to do next, such as where to put the caret. It is a
    /// sentence, not a failure, so it has no details.
    pub(super) fn hint(&mut self, text: impl Into<String>, cx: &mut Context<Self>) {
        self.hint = Some(text.into());
        cx.notify();
    }

    /// Opens the details of `error` from wherever it is indicated.
    pub(super) fn show_details(&mut self, error: ReportedError, cx: &mut Context<Self>) {
        self.details = Some(error);
        cx.notify();
    }

    /// Closes the details. Reading the status bar's failure clears it.
    pub(super) fn close_details(&mut self, cx: &mut Context<Self>) {
        if let Some(shown) = self.details.take() {
            if self
                .error
                .as_ref()
                .is_some_and(|error| error.code == shown.code && error.details == shown.details)
            {
                self.error = None;
            }
        }
        cx.notify();
    }

    pub(super) fn render_details(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let studio = cx.entity().downgrade();
        details_dialog(self.theme, self.details.as_ref(), move |cx| {
            studio
                .update(cx, |studio, cx| studio.close_details(cx))
                .ok();
        })
    }
}
