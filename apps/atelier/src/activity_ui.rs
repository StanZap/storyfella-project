use crate::{
    activity::{PromptValidation, Report, Scope, Task},
    app::{DrawingApp, Page, Panel, Picker},
    queue::JobState,
    ui::{column, icon, label, row, tool_button, BORDER, WINE},
};
use gpui_kit::{
    component::{button::*, input::Textarea, *},
    prelude::FluentBuilder,
    *,
};

impl DrawingApp {
    pub fn report_error(&mut self, scope: Scope, task: Task, details: String) -> u64 {
        let name = match scope {
            Scope::Project(epoch) if epoch == self.epoch => Some(self.doc.title.clone()),
            Scope::Project(epoch) => self
                .project_tabs
                .iter()
                .find(|tab| tab.epoch == epoch)
                .map(|tab| tab.doc.title.clone()),
            _ => None,
        };
        self.activity.record(scope, task, None, name, details)
    }

    pub(crate) fn prompt_issue(&mut self, message: impl Into<String>) {
        self.prompt_validation = Some(PromptValidation {
            epoch: self.epoch,
            target: self.prompt_target.clone(),
            message: message.into(),
        });
    }

    pub(crate) fn generation_failed(&mut self, id: u64, details: String) {
        let Some(item) = self.queue.get(id) else {
            return;
        };
        let epoch = item.epoch;
        let context = item.context.clone();
        let retry_of = item.retry_of;
        let report_id = if let Some(report) = retry_of.filter(|id| self.activity.get(*id).is_some())
        {
            self.activity.repeat_generation(report, id, details);
            report
        } else {
            let name = if epoch == self.epoch {
                Some(self.doc.title.clone())
            } else {
                self.project_tabs
                    .iter()
                    .find(|tab| tab.epoch == epoch)
                    .map(|tab| tab.doc.title.clone())
            };
            self.activity.record(
                Scope::Project(epoch),
                Task::Generation(id),
                context,
                name,
                details,
            )
        };
        self.queue.get_mut(id).expect("Failed request exists").state = JobState::Failed(report_id);
        self.status.clear();
    }

    pub fn open_report(&mut self, id: u64, cx: &mut Context<Self>) {
        self.panel = Panel::Activity;
        self.activity_selection = Some(id);
        self.activity_details = false;
        self.activity_copy = None;
        cx.notify();
    }

    pub(crate) fn report_hint(
        &self,
        id: u64,
        text: &'static str,
        cx: &mut Context<Self>,
    ) -> Button {
        Button::new(format!("report-hint-{id}"))
            .ghost()
            .small()
            .icon(icon("alert"))
            .label(text)
            .accessibility_label(text)
            .tooltip("View in Activity")
            .on_click(cx.listener(move |this, _, _, cx| this.open_report(id, cx)))
    }

    pub(crate) fn generation_report(&self) -> Option<&Report> {
        self.activity
            .latest(&Scope::Project(self.epoch), Some(&self.prompt_target))
            .filter(|report| matches!(report.task, Task::Generation(_)))
    }

    fn report_action(&self, report: &Report) -> Option<(&'static str, bool)> {
        let project_open = match report.scope {
            Scope::Project(epoch) => self.project_tab_order.contains(&epoch),
            _ => true,
        };
        match &report.task {
            Task::Generation(_) => {
                let available =
                    project_open
                        && report
                            .retry_request
                            .is_some_and(|id| self.queue.get(id).is_some())
                        && !self.queue.items().iter().any(|item| {
                            item.retry_of == Some(report.id) && !item.state.is_finished()
                        });
                Some(("Retry", available))
            }
            Task::Save(_) => Some(("Save again", project_open && !self.manual_pending)),
            Task::Recovery => Some(("Retry", project_open)),
            Task::Import => Some(("Import again", project_open)),
            Task::Export if matches!(report.scope, Scope::Project(_)) => {
                Some(("Review export", project_open))
            }
            Task::Export => None,
            Task::Library => Some(("Refresh", !self.library_loading)),
            Task::Settings => Some(("Review settings", true)),
            Task::Open => Some(("Open project", true)),
            Task::Clipforge => Some(("Try again", true)),
            Task::Mask(id) => Some((
                "Save mask",
                self.opened_image
                    .as_ref()
                    .is_some_and(|(reference, _, _, _)| crate::library::id(reference) == *id),
            )),
            Task::Other => None,
        }
    }

    fn retry_report(&mut self, id: u64, window: &mut Window, cx: &mut Context<Self>) {
        let Some(report) = self.activity.get(id) else {
            return;
        };
        if self
            .report_action(report)
            .is_none_or(|(_, enabled)| !enabled)
        {
            return;
        }
        let scope = report.scope.clone();
        let task = report.task.clone();
        let request = report.retry_request;
        if let Scope::Project(epoch) = scope {
            if !matches!(task, Task::Generation(_)) {
                self.activate_project_tab(epoch, window, cx);
                if self.epoch != epoch {
                    return;
                }
            }
        }
        match task {
            Task::Generation(_) => {
                if let Some(request) = request {
                    self.queue_retry(request, cx);
                }
            }
            Task::Save(path) => {
                self.capture_prompt(cx);
                self.queue_save(path, false);
            }
            Task::Recovery if scope == Scope::App => self.persist_open_projects(),
            Task::Recovery => {
                self.capture_prompt(cx);
                self.queue_save(self.data_dir.join("recovery.atelier"), true);
            }
            Task::Import => {
                if scope == Scope::Library {
                    self.page = Page::Library;
                } else {
                    self.panel = Panel::References;
                }
                self.pick(Picker::Import, window, cx);
            }
            Task::Export => self.panel = Panel::Canvas,
            Task::Library => self.refresh_library(),
            Task::Settings => self.panel = Panel::Settings,
            Task::Open => self.pick(Picker::Open, window, cx),
            Task::Clipforge => self.open_clipforge(),
            Task::Mask(_) => self.save_mask(),
            Task::Other => {}
        }
        cx.notify();
    }

    fn activity_report_row(&self, report: &Report, cx: &mut Context<Self>) -> Div {
        let id = report.id;
        let selected = self.activity_selection == Some(id);
        let mut content = column()
            .gap_2()
            .py_3()
            .border_b_1()
            .border_color(rgb(BORDER))
            .child(
                row()
                    .child(
                        Button::new(format!("activity-report-{id}"))
                            .ghost()
                            .small()
                            .custom(ButtonCustomVariant::new(cx))
                            .flex_1()
                            .min_w_0()
                            .h_auto()
                            .p_0()
                            .accessibility_label(report.task.title())
                            .child(
                                row()
                                    .w_full()
                                    .items_start()
                                    .child(
                                        icon(if report.resolved { "check" } else { "alert" })
                                            .text_color(rgb(WINE)),
                                    )
                                    .child(
                                        div()
                                            .flex_1()
                                            .min_w_0()
                                            .text_left()
                                            .whitespace_normal()
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .child(report.task.title()),
                                    ),
                            )
                            .on_click(cx.listener(move |this, _, _, cx| this.open_report(id, cx))),
                    )
                    .child(
                        tool_button(
                            format!("dismiss-report-{id}"),
                            "close",
                            "Dismiss notification",
                        )
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if let Some(report) = this.activity.get_mut(id) {
                                report.dismissed = true;
                            }
                            if this.activity_selection == Some(id) {
                                this.activity_selection = None;
                                this.activity_details = false;
                            }
                            cx.notify();
                        })),
                    ),
            )
            .when(
                report.project_name.is_some() || report.attempts > 1,
                |content| {
                    content.child(
                        row()
                            .flex_wrap()
                            .gap_1()
                            .when_some(report.project_name.as_ref(), |meta, name| {
                                meta.child(label(name.clone()).truncate())
                            })
                            .when(report.attempts > 1, |meta| {
                                meta.when(report.project_name.is_some(), |meta| {
                                    meta.child(label("·"))
                                })
                                .child(label(format!("{} attempts", report.attempts)))
                            }),
                    )
                },
            );
        if selected {
            content = content
                .child(div().text_sm().child(if report.resolved {
                    "This issue has been resolved."
                } else {
                    report.explanation()
                }))
                .child(
                    row()
                        .flex_wrap()
                        .gap_1()
                        .when(!report.resolved, |actions| {
                            if let Some((title, enabled)) = self.report_action(report) {
                                actions.child(
                                    Button::new(format!("retry-report-{id}"))
                                        .primary()
                                        .small()
                                        .label(title)
                                        .disabled(!enabled)
                                        .on_click(cx.listener(move |this, _, window, cx| {
                                            this.retry_report(id, window, cx)
                                        })),
                                )
                            } else {
                                actions
                            }
                        })
                        .child(
                            Button::new(format!("report-details-{id}"))
                                .ghost()
                                .small()
                                .label(if self.activity_details {
                                    "Hide details"
                                } else {
                                    "Details"
                                })
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    this.activity_details = !this.activity_details;
                                    this.activity_copy = None;
                                    if this.activity_details {
                                        if let Some(report) = this.activity.get(id) {
                                            let text = report.diagnostics.join("\n\n");
                                            this.activity_text.update(cx, |state, cx| {
                                                state.set_value(text, window, cx)
                                            });
                                        }
                                    }
                                    cx.notify();
                                })),
                        )
                        .when(self.activity_details, |actions| {
                            actions.child(
                                Button::new(format!("copy-report-{id}"))
                                    .ghost()
                                    .small()
                                    .icon(icon("copy"))
                                    .label(self.activity_copy.unwrap_or("Copy report"))
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        let Some(report) = this.activity.get(id) else {
                                            return;
                                        };
                                        let text = report.text();
                                        cx.write_to_clipboard(ClipboardItem::new_string(
                                            text.clone(),
                                        ));
                                        this.activity_copy = Some(
                                            if cx
                                                .read_from_clipboard()
                                                .and_then(|item| item.text())
                                                .as_deref()
                                                == Some(&text)
                                            {
                                                "Copied"
                                            } else {
                                                "Copy failed · retry"
                                            },
                                        );
                                        cx.notify();
                                    })),
                            )
                        }),
                );
            if self.activity_details {
                content = content.child(
                    div().rounded_md().bg(rgb(0xf7f6f8)).p_2().child(
                        Textarea::new(&self.activity_text)
                            .readonly(true)
                            .appearance(false)
                            .bordered(false)
                            .small()
                            .h(px(112.))
                            .aria_label("Technical report"),
                    ),
                );
            }
        }
        content
    }

    pub(crate) fn activity_panel(&self, cx: &mut Context<Self>) -> Div {
        let mut body = column().gap_0();
        let mut has_content = false;
        for report in self
            .activity
            .reports()
            .iter()
            .rev()
            .filter(|report| !report.resolved || self.activity_selection == Some(report.id))
        {
            body = body.child(self.activity_report_row(report, cx));
            has_content = true;
        }
        let queue = self
            .queue
            .items()
            .iter()
            .filter(|item| !matches!(item.state, JobState::Failed(_)));
        for item in queue {
            body = body.child(self.queue_row(item, cx));
            has_content = true;
        }
        if !has_content {
            body = body.child(label("Nothing needs attention").py_6());
        }
        let content = column()
            .gap_2()
            .child(
                row()
                    .when(self.queue.outstanding() > 0, |row| {
                        row.child(
                            Button::new("activity-pause")
                                .ghost()
                                .small()
                                .label(if self.queue.is_paused() {
                                    "Resume"
                                } else {
                                    "Pause"
                                })
                                .on_click(
                                    cx.listener(|this, _, _, cx| this.queue_pause_toggle(cx)),
                                ),
                        )
                    })
                    .when(self.generation.is_some(), |row| {
                        row.child(
                            Button::new("activity-stop")
                                .ghost()
                                .small()
                                .label("Stop")
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.cancel_generation();
                                    cx.notify();
                                })),
                        )
                    })
                    .when(
                        self.queue.items().iter().any(|item| match item.state {
                            JobState::Done | JobState::Cancelled => true,
                            JobState::Failed(id) => self
                                .activity
                                .get(id)
                                .is_none_or(|report| !report.needs_attention()),
                            _ => false,
                        }),
                        |row| {
                            row.child(
                                Button::new("activity-clear")
                                    .ghost()
                                    .small()
                                    .label("Clear completed")
                                    .on_click(
                                        cx.listener(|this, _, _, cx| this.queue_clear_finished(cx)),
                                    ),
                            )
                        },
                    ),
            )
            .child(body);
        self.side_panel("Activity", content, cx)
    }
}
