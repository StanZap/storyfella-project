use crate::{
    app::{DrawingApp, Page, Panel, Picker},
    document::{Brush, Tool},
    providers::Provider,
    viewport::Viewport,
};
use gpui_kit::{
    component::{
        button::*,
        input::{Input, InputState, NumberInput, Textarea, TextareaState},
        select::Select,
        *,
    },
    prelude::FluentBuilder,
    *,
};
use std::time::Duration;

pub(crate) const WINE: u32 = 0x790e26;
const INK: u32 = 0x201d26;
const MUTED: u32 = 0x756b79;
/// Pending / in-progress state; never the only signal.
const AMBER: u32 = 0x9a6b06;
/// A completed request.
const GREEN: u32 = 0x2f6b3a;
pub(crate) const BORDER: u32 = 0xe1dce5;
const SURFACE: u32 = 0xf7f6f8;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum PromptComposerMode {
    Artboards,
    Drawing,
    Image,
}

fn prompt_composer_frame(
    prompt: &Entity<TextareaState>,
    controls: impl IntoElement,
    error: Option<&String>,
) -> impl IntoElement {
    panel()
        .occlude()
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .w(px(720.))
        .max_w_full()
        .p_3()
        .shadow_md()
        .child(Textarea::new(prompt).h(px(62.)))
        .child(controls)
        .when_some(error, |composer, error| {
            composer.child(label(error.clone()).text_color(rgb(WINE)))
        })
}

pub fn panel() -> Div {
    div()
        .bg(rgb(0xffffff))
        .border_1()
        .border_color(rgb(BORDER))
        .rounded_lg()
}
pub(crate) fn row() -> Div {
    div().flex().items_center().gap_2()
}
pub(crate) fn column() -> Div {
    div().flex().flex_col().gap_3()
}
pub(crate) fn label(text: impl Into<SharedString>) -> Div {
    div().text_xs().text_color(rgb(MUTED)).child(text.into())
}

fn creation_recipe_button(
    id: &'static str,
    label: &'static str,
    prompt: &'static str,
    width: u32,
    height: u32,
    cx: &mut Context<DrawingApp>,
) -> impl IntoElement {
    Button::new(id)
        .ghost()
        .small()
        .label(label)
        .on_click(cx.listener(move |this, _, window, cx| {
            this.apply_creation_recipe(prompt, width, height, window, cx)
        }))
}

/// The label for a queued request's state. Colour never carries the meaning
/// alone; the text always names the state.
fn job_state_label(state: &crate::queue::JobState) -> Div {
    use crate::queue::JobState;
    let color = match state {
        JobState::Queued | JobState::Cancelled => MUTED,
        JobState::Running { .. } => AMBER,
        JobState::Done => GREEN,
        JobState::Failed(_) => WINE,
    };
    label(state.label()).text_color(rgb(color))
}
fn separator() -> Div {
    div().h(px(1.)).w_full().bg(rgb(BORDER)).my_2()
}
pub(crate) fn icon(name: &str) -> Icon {
    let path=match name {
        "pencil"=>"<path d='m15 4 5 5M4 15l11-11a3 3 0 0 1 5 5L9 20l-6 1z'/>",
        "brush"=>"<path d='m14 3 7 7-9 9-7-7zM5 12l-3 9 9-3M14 3l-3 3'/>",
        "eraser"=>"<path d='m15 3 6 6-10 12H6l-4-4zM7 12l6 6M11 21h10'/>",
        "hand"=>"<path d='M8 13V6a2 2 0 0 1 4 0v6-8a2 2 0 0 1 4 0v8-6a2 2 0 0 1 4 0v10c0 5-3 7-7 7-3 0-5-2-7-5l-3-5a2 2 0 0 1 3-2z'/>",
        "image"=>"<rect x='3' y='3' width='18' height='18' rx='3'/><circle cx='8' cy='8' r='1'/><path d='m3 17 6-6 4 4 3-3 5 5'/>",
        "timer"=>"<circle cx='12' cy='14' r='8'/><path d='M9 2h6M12 6V2m6 5 2-2M12 10v5'/>",
        "save"=>"<path d='M5 3h12l4 4v14H3V3zM7 3v6h10V3M7 21v-7h10v7'/>",
        "export"=>"<path d='M12 16V3m-4 4 4-4 4 4M5 13H3v8h18v-8h-2'/>",
        "download"=>"<path d='M12 3v13m-4-4 4 4 4-4M3 16v5h18v-5'/>",
        "back"=>"<path d='m10 5-7 7 7 7M3 12h18'/>",
        "layers"=>"<path d='m12 3 10 5-10 5L2 8zM2 12l10 5 10-5M2 16l10 5 10-5'/>",
        "package"=>"<path d='m12 2 9 5v10l-9 5-9-5V7zM3 7l9 5 9-5M12 12v10M7 4l10 5'/>",
        "compare"=>"<rect x='3' y='3' width='18' height='18' rx='2'/><path d='M12 3v18M6 12h3M15 12h3'/>",
        "mask"=>"<rect x='3' y='3' width='18' height='18' rx='2' stroke-dasharray='3 3'/><path d='m7 16 9-9M7 11l4-4M12 17l5-5'/>",
        "mask-rect"=>"<rect x='4' y='5' width='16' height='14' rx='1' stroke-dasharray='3 3'/>",
        "reference"=>"<rect x='3' y='3' width='13' height='13' rx='2'/><path d='M8 21h13V8M3 12l4-4 4 4 2-2 3 3'/>",
        "info"=>"<circle cx='12' cy='12' r='9'/><path d='M12 11v6M12 7h.01'/>",
        "open"=>"<path d='M3 7V4h6l3 3h9v3M3 10h18l-3 11H3z'/>",
        "more"=>"<circle cx='5' cy='12' r='1'/><circle cx='12' cy='12' r='1'/><circle cx='19' cy='12' r='1'/>",
        "plus"=>"<path d='M12 5v14M5 12h14'/>","minus"=>"<path d='M5 12h14'/>",
        "undo"=>"<path d='m9 4-6 6 6 6M3 10h12a6 6 0 0 1 0 12'/>",
        "redo"=>"<path d='m15 4 6 6-6 6M21 10H9a6 6 0 0 0 0 12'/>",
        "rotate"=>"<path d='M20 10a8 8 0 1 0-2 8M20 3v7h-7'/>",
        "flip"=>"<path d='M12 3v18M8 6 3 12l5 6zM16 6l5 6-5 6z'/>",
        "fit"=>"<path d='M3 9V3h6M15 3h6v6M21 15v6h-6M9 21H3v-6'/>",
        "grid"=>"<rect x='3' y='3' width='18' height='18' rx='2'/><path d='M9 3v18M15 3v18M3 9h18M3 15h18'/>",
        "values"=>"<circle cx='12' cy='12' r='9'/><path d='M12 3v18' /><path d='M12 3a9 9 0 0 1 0 18' fill='currentColor'/>",
        "smooth"=>"<path d='M2 15c3 0 3-6 6-6s3 6 6 6 3-6 6-6h2'/>",
        "eye"=>"<path d='M2 12s4-7 10-7 10 7 10 7-4 7-10 7S2 12 2 12z'/><circle cx='12' cy='12' r='3'/>",
        "eye-off"=>"<path d='m3 3 18 18M9 5h3c6 0 10 7 10 7l-3 4M6 6l-4 6s4 7 10 7h3'/>",
        "heart"=>"<path d='M12 21 3 12a6 6 0 0 1 9-8 6 6 0 0 1 9 8z'/>",
        "settings"=>"<path d='M4 6h16M4 12h16M4 18h16'/><circle cx='8' cy='6' r='2' fill='white'/><circle cx='16' cy='12' r='2' fill='white'/><circle cx='10' cy='18' r='2' fill='white'/>",
        "panel"=>"<rect x='3' y='3' width='18' height='18' rx='2'/><path d='M15 3v18'/>",
        "play"=>"<path d='m7 3 14 9-14 9z'/>","pause"=>"<path d='M8 4v16M16 4v16'/>",
        "trash"=>"<path d='M3 6h18M9 6V3h6v3M5 6l1 15h12l1-15M10 10v7M14 10v7'/>",
        "up"=>"<path d='m6 14 6-6 6 6'/>","down"=>"<path d='m6 10 6 6 6-6'/>",
        "check"=>"<path d='m4 12 5 5L20 6'/>","close"=>"<path d='m5 5 14 14M19 5 5 19'/>",
        "sparkle"=>"<path d='m12 3 3 6 6 3-6 3-3 6-3-6-6-3 6-3z'/>",
        "help"=>"<circle cx='12' cy='12' r='9'/><path d='M9 9a3 3 0 1 1 5 2c-2 1-2 1-2 3M12 17h.01'/>",
        _=>"<circle cx='12' cy='12' r='8'/>",
    };
    Icon::default().data(format!("<svg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 24 24' fill='none' stroke='currentColor' stroke-width='1.6' stroke-linecap='round' stroke-linejoin='round'>{path}</svg>").as_bytes())
}
pub(crate) fn tool_button(id: impl Into<ElementId>, name: &str, tooltip: &'static str) -> Button {
    Button::new(id)
        .ghost()
        .small()
        .icon(icon(name))
        .accessibility_label(tooltip)
        .tooltip(tooltip)
}

pub(crate) trait AtelierSelection {
    fn atelier_selected(self, selected: bool, cx: &App) -> Self;
}
impl AtelierSelection for Button {
    fn atelier_selected(self, selected: bool, cx: &App) -> Self {
        if selected {
            self.custom(
                ButtonCustomVariant::new(cx)
                    .color(rgb(0xf1e6eb).into())
                    .foreground(rgb(WINE).into())
                    .hover(rgb(0xe9d6df).into())
                    .active(rgb(0xf1e6eb).into()),
            )
            .selected(true)
            .toggled(true)
        } else {
            self
        }
    }
}

fn field(name: &'static str, input: &Entity<InputState>) -> Div {
    column().gap_1().child(label(name)).child(Input::new(input))
}

/// A labelled numeric stepper, the numeric counterpart to [`field`].
fn number_field(name: &'static str, input: &Entity<InputState>) -> Div {
    column()
        .gap_1()
        .child(label(name))
        .child(NumberInput::new(input).small())
}

/// The status line for the machine's shared local image server.
fn server_status_label(status: Option<&crate::native_images::ServerStatus>) -> String {
    use crate::native_images::ServerStatus;
    match status {
        Some(ServerStatus::Running { model: Some(model) }) => {
            format!("Local server: running · {model}")
        }
        Some(ServerStatus::Running { model: None }) => "Local server: running".to_string(),
        Some(ServerStatus::Stopped) | None => {
            "Local server: stopped · starts on demand".to_string()
        }
        Some(ServerStatus::NotLocal) => String::new(),
    }
}

impl DrawingApp {
    fn projects_workspace(&self, cx: &mut Context<Self>) -> Div {
        let mut cards = div()
            .id("project-gallery")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .flex()
            .flex_wrap()
            .content_start()
            .gap_4();
        for (i, project) in self.projects.iter().enumerate() {
            let path = project.path.clone();
            cards = cards.child(
                panel()
                    .w(px(240.))
                    .overflow_hidden()
                    .child(
                        img(project.image.clone())
                            .w_full()
                            .h(px(180.))
                            .object_fit(ObjectFit::Contain),
                    )
                    .child(
                        column()
                            .gap_2()
                            .p_3()
                            .child(div().truncate().child(project.title.clone()))
                            .when(!project.note.is_empty(), |p| {
                                p.child(
                                    label(project.note.chars().take(80).collect::<String>())
                                        .truncate(),
                                )
                            })
                            .child(
                                Button::new(format!("open-project-{i}"))
                                    .ghost()
                                    .small()
                                    .label("Open project")
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        this.open_project(path.clone(), window, cx);
                                        cx.notify();
                                    })),
                            ),
                    ),
            );
        }
        if self.projects.is_empty() {
            cards = cards.child(
                column()
                    .w_full()
                    .items_center()
                    .justify_center()
                    .h(px(240.))
                    .child(if self.projects_loading {
                        "Loading projects…"
                    } else {
                        "No projects yet"
                    })
                    .when(!self.projects_loading, |empty| {
                        empty.child(
                            Button::new("empty-new-project")
                                .primary()
                                .icon(icon("plus"))
                                .label("New project")
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.new_document(window, cx);
                                    cx.notify();
                                })),
                        )
                    }),
            );
        }
        column()
            .flex_1()
            .min_w_0()
            .h_full()
            .p_6()
            .child(
                row()
                    .child(
                        div()
                            .text_xl()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("Projects"),
                    )
                    .child(div().flex_1())
                    .child(
                        Button::new("refresh-projects")
                            .ghost()
                            .small()
                            .icon(icon("rotate"))
                            .label("Refresh")
                            .disabled(self.projects_loading)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.refresh_projects();
                                cx.notify();
                            })),
                    ),
            )
            .child(cards)
    }
    pub fn render_workspace(&mut self, _: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let main = match self.page {
            Page::Projects => self.projects_workspace(cx).into_any_element(),
            Page::Draw => self.draw_workspace(cx).into_any_element(),
            Page::Library => self.library(cx).into_any_element(),
            Page::Image => self.image_workspace(cx).into_any_element(),
            Page::Artboards => self.artboards_workspace(cx).into_any_element(),
        };
        let overlay = match self.panel {
            Panel::Brushes => Some(self.brush_settings(cx)),
            Panel::Canvas => Some(self.canvas_settings(cx)),
            Panel::Settings => Some(self.provider_settings(cx)),
            Panel::Help => Some(self.help(cx)),
            Panel::Queue => Some(self.queue_panel(cx)),
            Panel::References => Some(self.reference_picker(cx)),
            Panel::None => None,
        };
        div()
            .id("atelier")
            .track_focus(&self.focus)
            .size_full()
            .flex()
            .flex_col()
            .relative()
            .bg(rgb(SURFACE))
            .text_color(rgb(INK))
            .text_sm()
            .on_key_down(cx.listener(Self::key_down))
            .on_action(cx.listener(|this, _: &crate::Quit, _, cx| {
                this.request_close(cx);
                cx.stop_propagation();
            }))
            .on_action(cx.listener(|this, _: &crate::SaveProject, window, cx| {
                this.save(false, window, cx);
                cx.stop_propagation();
            }))
            .on_action(cx.listener(|this, _: &crate::SaveProjectAs, window, cx| {
                this.save(true, window, cx);
                cx.stop_propagation();
            }))
            .on_action(cx.listener(|this, _: &crate::OpenProject, window, cx| {
                this.pick(Picker::Open, window, cx);
                cx.stop_propagation();
            }))
            .on_action(cx.listener(|this, _: &crate::OpenPrompt, window, cx| {
                this.open_prompt(window, cx);
                cx.stop_propagation();
            }))
            .child(self.header(cx))
            .child(self.project_tab_bar(cx))
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .when(
                        self.page != Page::Projects && self.page != Page::Artboards,
                        |layout| layout.child(self.rail(cx)),
                    )
                    .child(main),
            )
            .child(self.status_bar())
            .when_some(overlay, |layout, overlay| layout.child(overlay))
            .when(self.switching || self.closing, |layout| {
                layout.child(
                    div()
                        .id("switching-shield")
                        .absolute()
                        .size_full()
                        .bg(rgba(0xf7f6f8ee))
                        .flex()
                        .items_center()
                        .justify_center()
                        .child("Opening project…")
                        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation()),
                )
            })
            .into_any_element()
    }
    fn header(&self, cx: &mut Context<Self>) -> Div {
        row()
            .h(px(44.))
            .px_4()
            .border_b_1()
            .border_color(rgb(BORDER))
            .bg(rgb(0xffffff))
            .child(
                div()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(rgb(WINE))
                    .child("Atelier"),
            )
            .child(div().w(px(24.)))
            .child(
                Button::new("projects")
                    .ghost()
                    .small()
                    .label("Projects")
                    .atelier_selected(
                        matches!(self.page, Page::Projects | Page::Artboards | Page::Draw)
                            || (self.page == Page::Image
                                && self.image_return_page != Page::Library),
                        cx,
                    )
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.finish_stroke();
                        this.finish_mask();
                        this.page = Page::Projects;
                        this.refresh_projects();
                        cx.notify();
                    })),
            )
            .child(
                Button::new("library")
                    .ghost()
                    .small()
                    .label("Library")
                    .atelier_selected(self.page == Page::Library, cx)
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.finish_stroke();
                        this.finish_mask();
                        this.page = Page::Library;
                        cx.notify();
                    })),
            )
            .child(div().flex_1())
            .when(self.page != Page::Projects, |header| {
                header
                    .child(
                        Button::new("project-title")
                            .ghost()
                            .small()
                            .label(self.doc.title.clone())
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.layer_name.update(cx, |s, cx| {
                                    s.set_value(
                                        this.doc.layers[this.doc.active_layer].name.clone(),
                                        window,
                                        cx,
                                    )
                                });
                                this.panel = if this.panel == Panel::Canvas {
                                    Panel::None
                                } else {
                                    Panel::Canvas
                                };
                                cx.notify();
                            })),
                    )
                    .child(label(if self.dirty { "Edited" } else { "Saved" }))
            })
            .child(div().flex_1())
            .child(
                tool_button("new", "plus", "New project").on_click(cx.listener(
                    |this, _, window, cx| {
                        this.new_document(window, cx);
                        cx.notify();
                    },
                )),
            )
            .child(
                tool_button("open", "open", "Open project · ⌘O").on_click(
                    cx.listener(|this, _, window, cx| this.pick(Picker::Open, window, cx)),
                ),
            )
            .when(self.page != Page::Projects, |header| {
                header
                    .child(
                        tool_button("save", "save", "Save project · ⌘S").on_click(
                            cx.listener(|this, _, window, cx| this.save(false, window, cx)),
                        ),
                    )
                    .child(
                        tool_button("export", "export", "Export PNG / JPG").on_click(cx.listener(
                            |this, _, window, cx| {
                                this.layer_name.update(cx, |s, cx| {
                                    s.set_value(
                                        this.doc.layers[this.doc.active_layer].name.clone(),
                                        window,
                                        cx,
                                    )
                                });
                                this.panel = Panel::Canvas;
                                cx.notify();
                            },
                        )),
                    )
            })
            .when(self.page != Page::Projects, |header| {
                header.child(div().w(px(8.))).child(self.queue_button(cx))
            })
    }
    fn project_tab_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let mut tabs: Vec<_> = self
            .project_tabs
            .iter()
            .map(|tab| (tab.epoch, tab.doc.title.clone(), tab.dirty))
            .collect();
        tabs.push((self.epoch, self.doc.title.clone(), self.dirty));
        tabs.sort_by_key(|tab| tab.0);
        div()
            .id("project-tabs")
            .overflow_x_scroll()
            .flex()
            .gap_1()
            .px_4()
            .py_1()
            .children(tabs.into_iter().map(|(epoch, title, dirty)| {
                Button::new(format!("project-tab-{epoch}"))
                    .ghost()
                    .small()
                    .label(format!("{title}{}", if dirty { " •" } else { "" }))
                    .atelier_selected(
                        epoch == self.epoch && !matches!(self.page, Page::Library | Page::Projects),
                        cx,
                    )
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.activate_project_tab(epoch, window, cx)
                    }))
            }))
    }
    /// The header's queue control: an icon with a count badge while requests are
    /// outstanding, opening the queue panel.
    fn queue_button(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let outstanding = self.queue.outstanding();
        div()
            .relative()
            .child(
                tool_button(
                    "open-queue",
                    "layers",
                    if outstanding == 0 {
                        "Generation queue"
                    } else {
                        "Generation queue · requests waiting"
                    },
                )
                .atelier_selected(self.panel == Panel::Queue, cx)
                .on_click(cx.listener(|this, _, _, cx| {
                    this.panel = if this.panel == Panel::Queue {
                        Panel::None
                    } else {
                        Panel::Queue
                    };
                    cx.notify();
                })),
            )
            .when(outstanding > 0, |element| {
                element.child(
                    div()
                        .absolute()
                        .top(px(-5.))
                        .right(px(-5.))
                        .min_w(px(16.))
                        .h(px(16.))
                        .px(px(4.))
                        .rounded_full()
                        .bg(rgb(WINE))
                        .text_color(rgb(0xffffff))
                        .text_xs()
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(outstanding.to_string()),
                )
            })
    }
    fn rail(&self, cx: &mut Context<Self>) -> Div {
        let mut rail = column()
            .items_center()
            .w(px(if self.page == Page::Draw { 56. } else { 48. }))
            .flex_shrink_0()
            .py_3()
            .gap_1()
            .bg(rgb(0xffffff))
            .border_r_1()
            .border_color(rgb(BORDER));
        for (tool, name, tooltip) in [
            (Tool::Pencil, "pencil", "Pencil · B"),
            (Tool::Eraser, "eraser", "Eraser · E"),
            (Tool::Hand, "hand", "Pan · H / middle drag"),
        ] {
            rail = rail.child(
                tool_button(format!("tool-{name}"), name, tooltip)
                    .atelier_selected(self.tool == tool, cx)
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.finish_stroke();
                        this.tool = tool;
                        this.moving_image = false;
                        this.page = Page::Draw;
                        this.restore_prompt(window, cx);
                        window.focus(&this.focus, cx);
                        cx.notify();
                    })),
            );
        }
        if self.page == Page::Draw {
            rail = rail
                .child(separator().w(px(40.)))
                .child(self.brush_rail_controls(cx));
        }
        rail.child(separator().w(px(30.)))
            .child(
                tool_button("rail-library", "image", "Reference library")
                    .atelier_selected(self.page == Page::Library, cx)
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.finish_stroke();
                        this.page = Page::Library;
                        cx.notify();
                    })),
            )
            .child(
                tool_button("rail-timer", "timer", "Study timer")
                    .atelier_selected(self.panel == Panel::Help || self.timer_running, cx)
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.panel = if this.panel == Panel::Help {
                            Panel::None
                        } else {
                            Panel::Help
                        };
                        cx.notify();
                    })),
            )
            .child(div().flex_1())
            .child(
                tool_button("rail-settings", "settings", "Provider settings")
                    .atelier_selected(self.panel == Panel::Settings, cx)
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.panel = if this.panel == Panel::Settings {
                            Panel::None
                        } else {
                            Panel::Settings
                        };
                        cx.notify();
                    })),
            )
            .child(
                tool_button("rail-help", "help", "Practice and shortcuts").on_click(cx.listener(
                    |this, _, _, cx| {
                        this.panel = if this.panel == Panel::Help {
                            Panel::None
                        } else {
                            Panel::Help
                        };
                        cx.notify();
                    },
                )),
            )
    }
    fn draw_workspace(&self, cx: &mut Context<Self>) -> Div {
        row()
            .items_start()
            .flex_1()
            .min_w_0()
            .h_full()
            .gap_0()
            .child(
                column()
                    .gap_0()
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .child(self.canvas_toolbar(cx))
                    .child(self.canvas(cx)),
            )
            .when(self.dock, |layout| layout.child(self.dock_panel(cx)))
    }
    fn canvas_toolbar(&self, cx: &mut Context<Self>) -> Div {
        row()
            .h(px(46.))
            .px_4()
            .child(
                tool_button("canvas-back", "back", "Back to artboards").on_click(cx.listener(
                    |this, _, window, cx| {
                        this.finish_stroke();
                        this.page = Page::Artboards;
                        this.editing_image = false;
                        this.restore_prompt(window, cx);
                        cx.notify();
                    },
                )),
            )
            .child(
                tool_button("undo", "undo", "Undo · ⌘Z")
                    .disabled(!self.history.can_undo())
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.undo(window, cx);
                        cx.notify();
                    })),
            )
            .child(
                tool_button("redo", "redo", "Redo · ⇧⌘Z")
                    .disabled(!self.history.can_redo())
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.redo(window, cx);
                        cx.notify();
                    })),
            )
            .child(
                tool_button(
                    "generate-from-drawing",
                    "sparkle",
                    "Create / image prompt · ⌘P",
                )
                .atelier_selected(self.show_drawing_composer, cx)
                .on_click(cx.listener(|this, _, window, cx| {
                    this.open_from_drawing(window, cx);
                })),
            )
            .child(div().flex_1())
            .child(
                tool_button("zoom-out", "minus", "Zoom out").on_click(cx.listener(
                    |this, _, _, cx| {
                        this.view.zoom = (this.view.zoom / 1.2).clamp(0.2, 8.);
                        cx.notify();
                    },
                )),
            )
            .child(label(format!(
                "{}%",
                (self.view.zoom * 100.).round() as u32
            )))
            .child(
                tool_button("zoom-in", "plus", "Zoom in").on_click(cx.listener(
                    |this, _, _, cx| {
                        this.view.zoom = (this.view.zoom * 1.2).clamp(0.2, 8.);
                        cx.notify();
                    },
                )),
            )
            .child(
                tool_button("rotate-view", "rotate", "Rotate view 90° · R").on_click(cx.listener(
                    |this, _, _, cx| {
                        this.view.turns = (this.view.turns + 1) % 4;
                        this.view_changed();
                        cx.notify();
                    },
                )),
            )
            .child(
                tool_button("mirror-view", "flip", "Mirror view · F")
                    .atelier_selected(self.view.mirror, cx)
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.view.mirror = !this.view.mirror;
                        this.view_changed();
                        cx.notify();
                    })),
            )
            .child(
                tool_button("fit", "fit", "Reset / fit view · 0").on_click(cx.listener(
                    |this, _, _, cx| {
                        this.view = Viewport::default();
                        this.view_changed();
                        cx.notify();
                    },
                )),
            )
            .child(
                tool_button("canvas-guides", "grid", "Canvas proportion guides")
                    .atelier_selected(self.guides, cx)
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.guides = !this.guides;
                        cx.notify();
                    })),
            )
            .child(
                tool_button("dock", "panel", "Toggle panels")
                    .atelier_selected(self.dock, cx)
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.dock = !this.dock;
                        cx.notify();
                    })),
            )
    }
    fn canvas(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let capture = self.bounds.clone();
        let tiles: Vec<_> = self
            .tiles
            .values()
            .map(|tile| (tile.x, tile.y, tile.width, tile.height, tile.image.clone()))
            .collect();
        let view = self.view;
        let w = self.doc.width;
        let h = self.doc.height;
        let hover = self.hover;
        let brush = self.size();
        let tool = self.tool;
        let guides = self.guides;
        let region = self.generation_region;
        let gizmo = (self.moving_image && self.tool == Tool::Hand)
            .then(|| self.doc.layer_bounds())
            .flatten();

        div()
            .id("canvas-viewport")
            .relative()
            .flex_1()
            .min_h_0()
            .overflow_hidden()
            .cursor(
                if self.tool == Tool::Hand && !self.selecting_generation_region {
                    CursorStyle::OpenHand
                } else {
                    CursorStyle::Crosshair
                },
            )
            .on_mouse_down(MouseButton::Left, cx.listener(Self::pointer_down))
            .on_mouse_down(MouseButton::Middle, cx.listener(Self::pointer_down))
            .on_mouse_move(cx.listener(|this, event, _, cx| this.pointer_move(event, cx)))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _, window, cx| {
                    this.finish_stroke();
                    this.finish_drawing_region(cx);
                    this.restore_prompt(window, cx);
                    cx.notify();
                }),
            )
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(|this, _, window, cx| {
                    this.finish_stroke();
                    this.finish_drawing_region(cx);
                    this.restore_prompt(window, cx);
                    cx.notify();
                }),
            )
            .on_scroll_wheel(cx.listener(|this, event: &ScrollWheelEvent, _, cx| {
                this.finish_stroke();
                let delta = event.delta.pixel_delta(px(20.));
                if event.modifiers.platform || event.modifiers.control {
                    this.view.zoom =
                        (this.view.zoom * (f32::from(delta.y) * 0.003).exp()).clamp(0.2, 8.);
                } else {
                    this.view.pan.x += f32::from(delta.x);
                    this.view.pan.y += f32::from(delta.y);
                }
                cx.notify();
            }))
            .on_pinch(cx.listener(|this, event: &PinchEvent, _, cx| {
                this.finish_stroke();
                this.view.zoom = (this.view.zoom * (1. + event.delta)).clamp(0.2, 8.);
                cx.notify();
            }))
            .child(
                canvas(
                    move |bounds, _, _| {
                        capture.set(bounds);
                    },
                    move |bounds, _, window, _| {
                        let scale = view.scale(
                            (bounds.size.width.into(), bounds.size.height.into()),
                            (w, h),
                        );
                        let (vw, vh) = view.dimensions(w, h);
                        let paper = Bounds::new(
                            point(
                                bounds.origin.x
                                    + (bounds.size.width - px(vw * scale)) / 2.
                                    + px(view.pan.x),
                                bounds.origin.y
                                    + (bounds.size.height - px(vh * scale)) / 2.
                                    + px(view.pan.y),
                            ),
                            size(px(vw * scale), px(vh * scale)),
                        );
                        window.paint_quad(fill(paper, rgb(0xfffdf8)));
                        for (x, y, width, height, image) in &tiles {
                            let tile_bounds = Bounds::new(
                                point(
                                    paper.origin.x + px(*x as f32 * scale),
                                    paper.origin.y + px(*y as f32 * scale),
                                ),
                                size(px(*width as f32 * scale), px(*height as f32 * scale)),
                            );
                            if tile_bounds.intersects(&bounds) {
                                match image {
                                    #[cfg(target_os = "macos")]
                                    crate::app::CanvasImage::Surface(surface) => {
                                        window.paint_surface(tile_bounds, surface.buffer())
                                    }
                                    #[cfg(any(test, not(target_os = "macos")))]
                                    crate::app::CanvasImage::Image(image) => {
                                        let _ = window.paint_image(
                                            bounds,
                                            tile_bounds,
                                            Corners::all(px(0.)),
                                            image.clone(),
                                            0,
                                            false,
                                        );
                                    }
                                }
                            }
                        }
                        if guides {
                            let mut path = PathBuilder::stroke(px(1.));
                            let convert = |p: crate::document::Point| {
                                let p = view.to_view(p, w as f32, h as f32);
                                point(
                                    paper.origin.x + px(p.x * scale),
                                    paper.origin.y + px(p.y * scale),
                                )
                            };
                            for fraction in [1. / 3., 2. / 3.] {
                                path.move_to(convert(crate::document::Point {
                                    x: w as f32 * fraction,
                                    y: 0.,
                                }));
                                path.line_to(convert(crate::document::Point {
                                    x: w as f32 * fraction,
                                    y: h as f32,
                                }));
                                path.move_to(convert(crate::document::Point {
                                    x: 0.,
                                    y: h as f32 * fraction,
                                }));
                                path.line_to(convert(crate::document::Point {
                                    x: w as f32,
                                    y: h as f32 * fraction,
                                }));
                            }
                            if let Ok(path) = path.build() {
                                window.paint_path(path, rgba(0x790e2640));
                            }
                        }
                        if let Some((a, b)) = region {
                            let corners = [
                                crate::document::Point { x: a.x, y: a.y },
                                crate::document::Point { x: b.x, y: a.y },
                                crate::document::Point { x: b.x, y: b.y },
                                crate::document::Point { x: a.x, y: b.y },
                            ];
                            let (mut min_x, mut min_y) = (f32::MAX, f32::MAX);
                            let (mut max_x, mut max_y) = (f32::MIN, f32::MIN);
                            for corner in corners {
                                let view_point = view.to_view(corner, w as f32, h as f32);
                                min_x = min_x.min(view_point.x);
                                min_y = min_y.min(view_point.y);
                                max_x = max_x.max(view_point.x);
                                max_y = max_y.max(view_point.y);
                            }
                            let rect = Bounds::new(
                                point(
                                    paper.origin.x + px(min_x * scale),
                                    paper.origin.y + px(min_y * scale),
                                ),
                                size(px((max_x - min_x) * scale), px((max_y - min_y) * scale)),
                            );
                            window.paint_quad(fill(rect, rgba(0x790e2620)));
                            let mut border = PathBuilder::stroke(px(1.5));
                            border.move_to(rect.origin);
                            border.line_to(point(rect.origin.x + rect.size.width, rect.origin.y));
                            border.line_to(point(
                                rect.origin.x + rect.size.width,
                                rect.origin.y + rect.size.height,
                            ));
                            border.line_to(point(rect.origin.x, rect.origin.y + rect.size.height));
                            border.close();
                            if let Ok(border) = border.build() {
                                window.paint_path(border, rgb(WINE));
                            }
                        }
                        if let Some((x, y, r, b)) = gizmo {
                            let convert = |x, y| {
                                let p = view.to_view(
                                    crate::document::Point { x, y },
                                    w as f32,
                                    h as f32,
                                );
                                point(
                                    paper.origin.x + px(p.x * scale),
                                    paper.origin.y + px(p.y * scale),
                                )
                            };
                            let mut path = PathBuilder::stroke(px(1.5));
                            path.move_to(convert(x, y));
                            path.line_to(convert(r, y));
                            path.line_to(convert(r, b));
                            path.line_to(convert(x, b));
                            path.close();
                            if let Ok(path) = path.build() {
                                window.paint_path(path, rgb(WINE));
                            }
                            let handle = convert(r, b);
                            window.paint_quad(fill(
                                Bounds::new(
                                    point(handle.x - px(6.), handle.y - px(6.)),
                                    size(px(12.), px(12.)),
                                ),
                                rgb(WINE),
                            ));
                        }
                        if tool != Tool::Hand {
                            if let Some(p) = hover.filter(|p| paper.contains(p)) {
                                let radius = (brush * scale / 2.).max(2.);
                                let mut ring = PathBuilder::stroke(px(1.));
                                for i in 0..=40 {
                                    let a = i as f32 / 40. * std::f32::consts::TAU;
                                    let p = point(
                                        p.x + px(radius * a.cos()),
                                        p.y + px(radius * a.sin()),
                                    );
                                    if i == 0 {
                                        ring.move_to(p);
                                    } else {
                                        ring.line_to(p);
                                    }
                                }
                                if let Ok(ring) = ring.build() {
                                    window.paint_path(ring, rgb(WINE));
                                }
                            }
                        }
                    },
                )
                .size_full(),
            )
            .when(self.show_drawing_composer, |canvas| {
                canvas.child(self.drawing_composer(cx))
            })
    }
    fn brush_rail_controls(&self, cx: &mut Context<Self>) -> Div {
        column()
            .w_full()
            .items_center()
            .gap_1()
            .child(
                tool_button("brush-picker", "brush", "Brush presets")
                    .atelier_selected(self.panel == Panel::Brushes, cx)
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.finish_stroke();
                        this.panel = if this.panel == Panel::Brushes {
                            Panel::None
                        } else {
                            Panel::Brushes
                        };
                        cx.notify();
                    })),
            )
            .child(
                tool_button("brush-smaller", "minus", "Smaller brush · [").on_click(cx.listener(
                    |this, _, _, cx| {
                        this.brush_size(0.8);
                        cx.notify();
                    },
                )),
            )
            .child(
                div()
                    .w(px(48.))
                    .h(px(28.))
                    .rounded_md()
                    .bg(rgb(0xf7f3f5))
                    .text_sm()
                    .font_weight(FontWeight::MEDIUM)
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(format!("{} px", self.size().round() as u32)),
            )
            .child(
                tool_button("brush-larger", "plus", "Larger brush · ]").on_click(cx.listener(
                    |this, _, _, cx| {
                        this.brush_size(1.25);
                        cx.notify();
                    },
                )),
            )
            .child(
                Button::new("brush-opacity")
                    .ghost()
                    .small()
                    .label(format!("{}%", (self.opacity * 100.) as u32))
                    .tooltip("Cycle pencil opacity")
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.opacity = if this.opacity >= 0.99 {
                            0.2
                        } else {
                            (this.opacity + 0.2).min(1.)
                        };
                        cx.notify();
                    })),
            )
            .child(
                tool_button("stabilize", "smooth", "Smooth strokes")
                    .atelier_selected(self.stabilize, cx)
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.stabilize = !this.stabilize;
                        cx.notify();
                    })),
            )
    }
    fn dock_panel(&self, cx: &mut Context<Self>) -> impl IntoElement {
        column()
            .id("drawing-dock")
            .w(px(256.))
            .flex_shrink_0()
            .h_full()
            .overflow_y_scroll()
            .bg(rgb(0xffffff))
            .border_l_1()
            .border_color(rgb(BORDER))
            .p_4()
            .gap_4()
            .child(self.layer_panel(cx))
            .child(separator())
            .child(self.reference_panel(cx))
    }

    fn reference_panel(&self, cx: &mut Context<Self>) -> Div {
        let has_reference =
            self.doc.reference.is_some() || !self.doc.artboard_references.is_empty();
        let mut reference = column().gap_2().child(
            row()
                .child(
                    div()
                        .flex_1()
                        .font_weight(FontWeight::MEDIUM)
                        .child("References"),
                )
                .when(has_reference, |header| {
                    header
                        .child(
                            tool_button(
                                "references-visible",
                                if self.doc.references_visible {
                                    "eye"
                                } else {
                                    "eye-off"
                                },
                                "Show or hide references",
                            )
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.doc.references_visible = !this.doc.references_visible;
                                this.tracing = false;
                                this.changed();
                                cx.notify();
                            })),
                        )
                        .when(self.doc.references_visible, |header| {
                            header.child(
                                tool_button("reference-tools", "more", "Reference options")
                                    .atelier_selected(self.show_reference_tools, cx)
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.show_reference_tools = !this.show_reference_tools;
                                        cx.notify();
                                    })),
                            )
                        })
                        .child(
                            tool_button("reference-import", "plus", "Choose references").on_click(
                                cx.listener(|this, _, _, cx| {
                                    this.panel = Panel::References;
                                    this.refresh_library();
                                    cx.notify();
                                }),
                            ),
                        )
                }),
        );
        if let Some(i) = self
            .doc
            .reference
            .filter(|i| self.doc.references_visible && *i < self.references.len())
        {
            let image = self.references[i].clone();
            let grid = self.grid;
            let grayscale = self.grayscale;
            let zoom = self.ref_zoom;
            let pan = self.ref_pan;
            reference = reference
                .child(
                    div()
                        .id("reference-viewport")
                        .h(px(180.))
                        .w_full()
                        .overflow_hidden()
                        .cursor(CursorStyle::OpenHand)
                        .on_scroll_wheel(cx.listener(|this, event: &ScrollWheelEvent, _, cx| {
                            let delta = event.delta.pixel_delta(px(20.));
                            this.ref_zoom =
                                (this.ref_zoom * (f32::from(delta.y) * 0.003).exp()).clamp(1., 8.);
                            cx.notify();
                        }))
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(|this, event: &MouseDownEvent, _, cx| {
                                this.ref_drag = Some(event.position);
                                cx.notify();
                            }),
                        )
                        .on_mouse_move(cx.listener(|this, event: &MouseMoveEvent, _, cx| {
                            if let Some(last) = this.ref_drag {
                                if event.dragging() {
                                    this.ref_pan.x += f32::from(event.position.x - last.x);
                                    this.ref_pan.y += f32::from(event.position.y - last.y);
                                    this.ref_drag = Some(event.position);
                                } else {
                                    this.ref_drag = None;
                                }
                                cx.notify();
                            }
                        }))
                        .on_mouse_up(
                            MouseButton::Left,
                            cx.listener(|this, _, _, _| {
                                this.ref_drag = None;
                            }),
                        )
                        .on_mouse_up_out(
                            MouseButton::Left,
                            cx.listener(|this, _, _, _| {
                                this.ref_drag = None;
                            }),
                        )
                        .child(
                            canvas(
                                |_, _, _| {},
                                move |bounds, _, window, _| {
                                    let image_size = image.size(0);
                                    let iw = image_size.width.0 as f32;
                                    let ih = image_size.height.0 as f32;
                                    let scale = (f32::from(bounds.size.width) / iw)
                                        .min(f32::from(bounds.size.height) / ih)
                                        * zoom;
                                    let fitted = Bounds::new(
                                        point(
                                            bounds.origin.x
                                                + (bounds.size.width - px(iw * scale)) / 2.
                                                + px(pan.x),
                                            bounds.origin.y
                                                + (bounds.size.height - px(ih * scale)) / 2.
                                                + px(pan.y),
                                        ),
                                        size(px(iw * scale), px(ih * scale)),
                                    );
                                    let _ = window.paint_image(
                                        bounds,
                                        fitted,
                                        Corners::all(px(4.)),
                                        image.clone(),
                                        0,
                                        grayscale,
                                    );
                                    if grid {
                                        let mut path = PathBuilder::stroke(px(1.));
                                        for fraction in [1. / 3., 2. / 3.] {
                                            let x = fitted.origin.x + fitted.size.width * fraction;
                                            let y = fitted.origin.y + fitted.size.height * fraction;
                                            path.move_to(point(x, fitted.origin.y));
                                            path.line_to(point(
                                                x,
                                                fitted.origin.y + fitted.size.height,
                                            ));
                                            path.move_to(point(fitted.origin.x, y));
                                            path.line_to(point(
                                                fitted.origin.x + fitted.size.width,
                                                y,
                                            ));
                                        }
                                        if let Ok(path) = path.build() {
                                            window.paint_path(path, rgba(0xffffffaa));
                                        }
                                    }
                                },
                            )
                            .size_full(),
                        ),
                )
                .child(label(self.doc.references[i].name.clone()).truncate())
                .when(self.show_reference_tools, |reference| {
                    reference.child(
                        row()
                            .justify_center()
                            .child(
                                tool_button("reference-grid", "grid", "Reference grid")
                                    .atelier_selected(self.grid, cx)
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.grid = !this.grid;
                                        cx.notify();
                                    })),
                            )
                            .child(
                                tool_button("reference-values", "values", "Grayscale values")
                                    .atelier_selected(self.grayscale, cx)
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.grayscale = !this.grayscale;
                                        cx.notify();
                                    })),
                            )
                            .child(
                                tool_button("reference-trace", "image", "Assisted study overlay")
                                    .atelier_selected(self.tracing, cx)
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.tracing = !this.tracing;
                                        this.view_changed();
                                        cx.notify();
                                    })),
                            )
                            .child(
                                tool_button(
                                    "reference-fit",
                                    "fit",
                                    "Reset reference zoom · scroll to zoom, drag to pan",
                                )
                                .on_click(cx.listener(
                                    |this, _, _, cx| {
                                        this.ref_zoom = 1.;
                                        this.ref_pan = crate::document::Point { x: 0., y: 0. };
                                        cx.notify();
                                    },
                                )),
                            )
                            .child(
                                tool_button("reference-favorite", "heart", "Favorite reference")
                                    .atelier_selected(self.doc.references[i].favorite, cx)
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.toggle_favorite(i);
                                        cx.notify();
                                    })),
                            ),
                    )
                })
                .when(self.tracing, |panel| panel.child(label("Assisted study")));
        } else if self.doc.references_visible {
            reference = reference.child(
                Button::new("choose-reference")
                    .ghost()
                    .small()
                    .icon(icon("plus"))
                    .label("Add reference")
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.panel = Panel::References;
                        this.refresh_library();
                        cx.notify();
                    })),
            );
        } else {
            reference = reference.child(label("Hidden while you work"));
        }
        if self.doc.references_visible && self.doc.artboard_references.len() > 1 {
            for id in &self.doc.artboard_references {
                if let Some(i) = self
                    .doc
                    .references
                    .iter()
                    .position(|r| crate::library::id(r) == *id)
                {
                    reference = reference.child(
                        Button::new(format!("show-reference-{i}"))
                            .ghost()
                            .small()
                            .label(self.doc.references[i].name.clone())
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.select_reference(i, window, cx)
                            })),
                    );
                }
            }
        }
        if self.show_reference_tools && self.doc.references_visible && self.doc.reference.is_some()
        {
            reference = reference.child(
                Button::new("reference-remove-project")
                    .ghost()
                    .small()
                    .icon(icon("trash"))
                    .label("Remove reference")
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.remove_reference(window, cx);
                        cx.notify();
                    })),
            );
        }

        reference
    }

    fn layer_panel(&self, cx: &mut Context<Self>) -> Div {
        let selected = &self.doc.layers[self.doc.active_layer];
        let mut layers = column().gap_2().child(
            row()
                .child(div().font_weight(FontWeight::MEDIUM).child("Layers"))
                .child(label(self.doc.layers.len().to_string()))
                .child(div().flex_1())
                .child(
                    tool_button("add-layer", "plus", "Add layer").on_click(cx.listener(
                        |this, _, window, cx| {
                            this.add_layer();
                            this.sync_layer_controls(window, cx);
                            cx.notify();
                        },
                    )),
                ),
        );
        let mut list = column()
            .id("layer-list")
            .gap_1()
            .max_h(px(232.))
            .overflow_y_scroll();
        for (i, layer) in self.doc.layers.iter().enumerate().rev() {
            list = list.child(
                row()
                    .gap_1()
                    .rounded_md()
                    .when(i == self.doc.active_layer, |r| r.bg(rgb(0xf1e6eb)))
                    .child(
                        tool_button(
                            format!("layer-visible-{i}"),
                            if layer.visible { "eye" } else { "eye-off" },
                            "Toggle layer visibility",
                        )
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.checkpoint();
                            this.doc.layers[i].visible = !this.doc.layers[i].visible;
                            this.changed();
                            cx.notify();
                        })),
                    )
                    .child(
                        div().flex_1().min_w_0().overflow_hidden().child(
                            Button::new(format!("layer-{i}"))
                                .ghost()
                                .small()
                                .w_full()
                                .label(layer.name.clone())
                                .atelier_selected(i == self.doc.active_layer, cx)
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    this.finish_stroke();
                                    this.doc.active_layer = i;
                                    this.sync_layer_controls(window, cx);
                                    this.layer_name.update(cx, |s, cx| {
                                        s.set_value(this.doc.layers[i].name.clone(), window, cx)
                                    });
                                    cx.notify();
                                })),
                        ),
                    )
                    .when(layer.candidate, |r| {
                        r.child(label("Review").text_color(rgb(AMBER)).pr_2())
                    }),
            );
        }
        layers = layers.child(list).child(
            row()
                .child(
                    Button::new("move-image-layer")
                        .ghost()
                        .small()
                        .icon(icon("hand"))
                        .label("Move")
                        .tooltip("Drag the layer to move; drag its corner to scale")
                        .disabled(self.doc.layer_bounds().is_none())
                        .atelier_selected(self.moving_image && self.tool == Tool::Hand, cx)
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.finish_stroke();
                            this.moving_image = !this.moving_image;
                            this.tool = Tool::Hand;
                            this.selecting_generation_region = false;
                            cx.notify();
                        })),
                )
                .child(div().flex_1())
                .child(
                    Button::new("layer-properties")
                        .ghost()
                        .small()
                        .icon(icon("settings"))
                        .label(format!("{}%", (selected.opacity * 100.).round() as u32))
                        .accessibility_label("Layer properties")
                        .tooltip("Opacity and precise layer values")
                        .atelier_selected(self.show_layer_properties, cx)
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.show_layer_properties = !this.show_layer_properties;
                            if this.show_layer_properties {
                                this.sync_layer_controls(window, cx);
                            }
                            cx.notify();
                        })),
                ),
        );
        if selected.candidate {
            layers = layers.child(
                column()
                    .gap_2()
                    .pt_2()
                    .child(label("Pending · excluded from export").text_color(rgb(AMBER)))
                    .child(
                        row()
                            .child(
                                Button::new("candidate-accept")
                                    .primary()
                                    .small()
                                    .label("Accept")
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.review_candidate(
                                            crate::workflow::Review::Accept,
                                            window,
                                            cx,
                                        )
                                    })),
                            )
                            .child(
                                Button::new("candidate-reference")
                                    .ghost()
                                    .small()
                                    .label("Save as reference")
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.review_candidate(
                                            crate::workflow::Review::Reference,
                                            window,
                                            cx,
                                        )
                                    })),
                            ),
                    )
                    .child(
                        row()
                            .child(
                                Button::new("candidate-hide")
                                    .ghost()
                                    .small()
                                    .icon(icon(if selected.visible { "eye-off" } else { "eye" }))
                                    .label(if selected.visible { "Hide" } else { "Show" })
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.checkpoint();
                                        let layer = &mut this.doc.layers[this.doc.active_layer];
                                        layer.visible = !layer.visible;
                                        this.changed();
                                        cx.notify();
                                    })),
                            )
                            .child(div().flex_1())
                            .child(
                                Button::new("candidate-discard")
                                    .ghost()
                                    .small()
                                    .icon(icon("trash"))
                                    .label("Discard")
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.review_candidate(
                                            crate::workflow::Review::Discard,
                                            window,
                                            cx,
                                        )
                                    })),
                            ),
                    ),
            );
        }
        if self.show_layer_properties {
            let mut properties = column()
                .gap_3()
                .pt_2()
                .child(
                    row()
                        .child(field("Move X (px)", &self.layer_dx).flex_1())
                        .child(field("Move Y (px)", &self.layer_dy).flex_1()),
                )
                .child(
                    row()
                        .child(field("Scale by (%)", &self.layer_scale).flex_1())
                        .child(field("Opacity (%)", &self.layer_opacity).flex_1()),
                )
                .child(
                    row()
                        .child(
                            tool_button("layer-up", "up", "Move layer up")
                                .disabled(self.doc.active_layer + 1 >= self.doc.layers.len())
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.move_layer(true);
                                    cx.notify();
                                })),
                        )
                        .child(
                            tool_button("layer-down", "down", "Move layer down")
                                .disabled(self.doc.active_layer == 0)
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.move_layer(false);
                                    cx.notify();
                                })),
                        )
                        .child(
                            tool_button("delete-layer", "trash", "Delete layer · undoable")
                                .disabled(self.doc.layers.len() == 1)
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.delete_layer();
                                    this.sync_layer_controls(window, cx);
                                    cx.notify();
                                })),
                        )
                        .child(div().flex_1())
                        .child(
                            Button::new("apply-layer-values")
                                .small()
                                .label("Apply")
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.apply_layer_controls(window, cx)
                                })),
                        ),
                );
            if let Some(reference) = selected.image.as_ref().map(|image| image.reference.clone()) {
                properties = properties.child(
                    Button::new("inspect-layer-image")
                        .ghost()
                        .small()
                        .icon(icon("info"))
                        .label("Image and versions")
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.open_image(reference.clone(), cx)
                        })),
                );
                if !selected.candidate {
                    properties = properties.child(
                        Button::new("layer-variation")
                            .ghost()
                            .small()
                            .icon(icon("plus"))
                            .label("New variation artboard")
                            .on_click(
                                cx.listener(|this, _, window, cx| this.layer_variation(window, cx)),
                            ),
                    );
                }
            }
            layers = layers.child(properties);
        }
        layers
    }
    fn study_timer(&self, cx: &mut Context<Self>) -> Div {
        let seconds = self.timer_remaining.as_secs();
        panel()
            .p_3()
            .flex()
            .flex_col()
            .gap_2()
            .child(
                row()
                    .child(icon("timer"))
                    .child(
                        div()
                            .flex_1()
                            .child(format!("{:02}:{:02}", seconds / 60, seconds % 60)),
                    )
                    .child(
                        tool_button(
                            "timer-toggle",
                            if self.timer_running { "pause" } else { "play" },
                            "Start / pause study",
                        )
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.timer_toggle();
                            cx.notify();
                        })),
                    )
                    .child(
                        tool_button("timer-reset", "rotate", "Reset timer").on_click(cx.listener(
                            |this, _, _, cx| {
                                this.timer_running = false;
                                this.timer_remaining =
                                    Duration::from_secs(this.timer_seconds as u64);
                                cx.notify();
                            },
                        )),
                    ),
            )
            .child(
                row().children([30, 120, 300, 600].into_iter().map(|seconds| {
                    Button::new(format!("timer-{seconds}"))
                        .ghost()
                        .small()
                        .label(if seconds == 30 {
                            "30s".into()
                        } else {
                            format!("{}m", seconds / 60)
                        })
                        .atelier_selected(self.timer_seconds == seconds, cx)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.timer_seconds = seconds;
                            this.timer_remaining = Duration::from_secs(seconds as u64);
                            this.timer_running = false;
                            cx.notify();
                        }))
                })),
            )
    }
    fn status_bar(&self) -> Div {
        if self.page == Page::Projects {
            return row()
                .h(px(28.))
                .px_4()
                .border_t_1()
                .border_color(rgb(BORDER))
                .text_xs()
                .text_color(rgb(MUTED))
                .child(format!("{} projects", self.projects.len()))
                .child(div().flex_1())
                .child("Stored locally");
        }
        if self.page == Page::Artboards {
            return if self.error {
                row()
                    .h(px(28.))
                    .px_6()
                    .text_xs()
                    .text_color(rgb(WINE))
                    .child(self.status.clone())
            } else {
                div()
            };
        }
        row()
            .h(px(28.))
            .px_4()
            .border_t_1()
            .border_color(rgb(BORDER))
            .text_xs()
            .text_color(rgb(MUTED))
            .child(format!("{} × {}", self.doc.width, self.doc.height))
            .when(self.timer_running, |r| {
                r.child(format!(
                    "Study · {:02}:{:02}",
                    self.timer_remaining.as_secs() / 60,
                    self.timer_remaining.as_secs() % 60
                ))
            })
            .when(self.tracing, |r| r.child("· Assisted"))
            .child(self.render_backend)
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .overflow_hidden()
                    .text_color(rgb(if self.error { WINE } else { MUTED }))
                    .child(self.status.clone()),
            )
            .child(if self.dirty {
                "Autosave enabled"
            } else {
                "Local project"
            })
    }
    fn library(&self, cx: &mut Context<Self>) -> Div {
        column()
            .flex_1()
            .min_w_0()
            .h_full()
            .p_6()
            .child(
                row()
                    .child(div().flex_1().text_xl().child("Reference library"))
                    .child(
                        Button::new("library-import")
                            .small()
                            .label("Import images")
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.pick(Picker::Import, window, cx)
                            })),
                    ),
            )
            .child(label(
                "Shared across all projects · originals remain independent of artwork layers",
            ))
            .child(
                Button::new("library-favorites")
                    .ghost()
                    .small()
                    .label("Favorites")
                    .atelier_selected(self.favorite_filter, cx)
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.favorite_filter = !this.favorite_filter;
                        cx.notify();
                    })),
            )
            .child(Input::new(&self.search))
            .child(self.reference_cards(false, cx))
    }

    fn reference_cards(&self, picker: bool, cx: &mut Context<Self>) -> impl IntoElement {
        let query = self.search.read(cx).value().to_lowercase();
        let mut items = self.library.clone();
        if picker {
            for (reference, image) in self
                .doc
                .references
                .iter()
                .cloned()
                .zip(self.references.iter().cloned())
            {
                if !items.iter().any(|(r, _)| r.image == reference.image) {
                    items.push((reference, image));
                }
            }
        }
        let mut cards = div()
            .id(if picker {
                "picker-cards"
            } else {
                "library-cards"
            })
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .flex()
            .flex_wrap()
            .content_start()
            .gap_3();
        for (i, (reference, image)) in items
            .into_iter()
            .filter(|(r, _)| {
                (!self.favorite_filter || r.favorite)
                    && format!("{} {}", r.name, r.tags)
                        .to_lowercase()
                        .contains(&query)
            })
            .enumerate()
        {
            let select = reference.clone();
            let open = reference.clone();
            let favorite = reference.clone();
            cards = cards.child(
                panel()
                    .w(px(if picker { 190. } else { 218. }))
                    .p_2()
                    .child(
                        img(image)
                            .w_full()
                            .h(px(130.))
                            .object_fit(ObjectFit::Contain),
                    )
                    .child(label(reference.name.clone()).truncate())
                    .child(
                        tool_button(
                            format!("library-favorite-{i}"),
                            "heart",
                            "Favorite reference",
                        )
                        .atelier_selected(reference.favorite, cx)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if let Some(index) = this
                                .doc
                                .references
                                .iter()
                                .position(|r| r.image == favorite.image)
                            {
                                this.toggle_favorite(index);
                            } else {
                                let mut reference = (*favorite).clone();
                                reference.favorite = !reference.favorite;
                                let _ = this.workers.storage.send(
                                    crate::worker::Storage::LibraryUpdate(
                                        this.data_dir.join("library"),
                                        std::sync::Arc::new(reference),
                                        crate::library::Update::Favorite,
                                    ),
                                );
                            }
                            cx.notify();
                        })),
                    )
                    .child(
                        Button::new(format!("reference-select-{i}"))
                            .small()
                            .label(if picker { "Select reference" } else { "Open" })
                            .on_click(cx.listener(move |this, _, window, cx| {
                                if picker {
                                    this.choose_reference(select.clone(), window, cx);
                                } else {
                                    this.open_image(open.clone(), cx);
                                }
                                cx.notify();
                            })),
                    ),
            );
        }
        cards
            .when(self.library_loading, |c| c.child(label("Loading library…")))
            .when_some(self.library_error.as_ref(), |c, error| {
                c.child(label(error.clone()).text_color(rgb(WINE)))
            })
    }

    fn reference_picker(&self, cx: &mut Context<Self>) -> AnyElement {
        let limit = crate::providers::reference_limit(&self.settings).map(|count| format!("{}: {count} image input(s) including the source · references are checked before queueing",self.settings.provider.name()))
            .unwrap_or_else(|error|format!("Choose an editing model: {error}"));
        let content = column().h(px(500.))
            .child(label(limit))
            .child(label("Project references and the shared library · selecting a reference never adds artwork"))
            .child(Input::new(&self.search))
            .child(Button::new("picker-import").small().label("Import into library and select…")
                .on_click(cx.listener(|this,_,window,cx| this.pick(Picker::Import,window,cx))))
            .child(self.reference_cards(true,cx));
        self.overlay("References", content, cx)
    }
    fn provider_choices(&self, settings: bool) -> Div {
        let state = if settings {
            &self.settings_provider_select
        } else {
            &self.generation_provider_select
        };
        column()
            .gap_2()
            .when(settings, |p| p.child(label("Image provider")))
            .child(
                Select::new(state)
                    .accessibility_label("Image provider")
                    .cleanable(false)
                    .w_full(),
            )
    }

    pub(crate) fn prompt_composer(
        &self,
        mode: PromptComposerMode,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let drawing = mode == PromptComposerMode::Drawing;
        let selection = drawing && self.region_bounds().is_some();
        let separate = selection && self.selection_mode == crate::workflow::SelectionMode::Separate;
        let setup_needed = self.provider_readiness.starts_with("Setup needed");
        let caption = if drawing {
            if separate {
                "Separate image · new artboard"
            } else if selection {
                "Blend into artboard · review as a layer"
            } else {
                "Artboard edit · review as a layer"
            }
        } else if mode == PromptComposerMode::Image {
            "Image variation · new artboard"
        } else {
            "New composition · new artboard"
        };

        let context = row()
            .child(label(caption).flex_1())
            .when(drawing && !separate, |r| {
                let (x, y, w, h) =
                    self.region_bounds()
                        .unwrap_or((0, 0, self.doc.width, self.doc.height));
                r.child(label(if selection {
                    format!("{w} × {h} at {x}, {y}")
                } else {
                    format!("{w} × {h} px")
                }))
            });
        let mut controls = column().gap_2().child(context);
        if drawing {
            controls = controls.child(
                row()
                    .gap_1()
                    .child(
                        Button::new("drawing-full")
                            .ghost()
                            .small()
                            .label("Full artboard")
                            .atelier_selected(!selection && !self.selecting_generation_region, cx)
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.use_full_canvas(cx);
                                this.restore_prompt(window, cx);
                            })),
                    )
                    .child(
                        Button::new("drawing-area")
                            .ghost()
                            .small()
                            .icon(icon("mask-rect"))
                            .label(if selection {
                                "Selected area"
                            } else {
                                "Select area"
                            })
                            .atelier_selected(self.selecting_generation_region || selection, cx)
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.toggle_box_select(cx);
                                this.restore_prompt(window, cx);
                            })),
                    ),
            );
        }
        if selection {
            controls = controls.child(
                row().gap_1().children(
                    [
                        (
                            "blend-selection",
                            "Blend into artboard",
                            crate::workflow::SelectionMode::Blend,
                        ),
                        (
                            "separate-selection",
                            "Create separate image",
                            crate::workflow::SelectionMode::Separate,
                        ),
                    ]
                    .into_iter()
                    .map(|(id, name, choice)| {
                        Button::new(id)
                            .ghost()
                            .small()
                            .label(name)
                            .atelier_selected(self.selection_mode == choice, cx)
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.selection_mode = choice;
                                this.restore_prompt(window, cx);
                                cx.notify();
                            }))
                    }),
                ),
            );
        }
        controls = controls.child(
            row()
                .flex_wrap()
                .gap_1()
                .child(
                    div().w(px(164.)).child(
                        Select::new(&self.generation_provider_select)
                            .small()
                            .accessibility_label("Image provider")
                            .cleanable(false)
                            .w_full(),
                    ),
                )
                .when(!drawing || separate, |r| {
                    r.child(
                        div().w(px(144.)).child(
                            Select::new(&self.artboard_aspect_select)
                                .small()
                                .accessibility_label("Aspect ratio")
                                .cleanable(false)
                                .w_full(),
                        ),
                    )
                })
                .child(
                    Button::new("composer-references")
                        .ghost()
                        .small()
                        .icon(icon("reference"))
                        .label(self.input_references().len().to_string())
                        .tooltip("Choose references")
                        .accessibility_label("Choose references")
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.panel = Panel::References;
                            this.refresh_library();
                            cx.notify();
                        })),
                )
                .child(div().flex_1())
                .child(
                    tool_button("composer-options", "more", "Image options")
                        .atelier_selected(self.show_prompt_details, cx)
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.show_prompt_details = !this.show_prompt_details;
                            cx.notify();
                        })),
                )
                .child(
                    Button::new("composer-submit")
                        .primary()
                        .small()
                        .label(if drawing && !separate {
                            "Queue edit"
                        } else {
                            "Create"
                        })
                        .on_click(cx.listener(move |this, _, window, cx| {
                            if drawing {
                                this.generate_from_drawing(window, cx);
                            } else {
                                this.editing_image = mode == PromptComposerMode::Image;
                                this.enqueue_generation(window, cx);
                            }
                        })),
                ),
        );
        let mut references = row().flex_wrap().gap_1();
        for (i, reference) in self.input_references().iter().enumerate() {
            references = references.child(
                row()
                    .gap_1()
                    .px_2()
                    .rounded_md()
                    .bg(rgb(0xf5f3f6))
                    .max_w(px(224.))
                    .child(
                        label(format!(
                            "{} · {}",
                            i + if drawing || mode == PromptComposerMode::Image {
                                2
                            } else {
                                1
                            },
                            reference.name
                        ))
                        .truncate()
                        .flex_1(),
                    )
                    .child(
                        tool_button(
                            format!("composer-remove-ref-{i}"),
                            "close",
                            "Remove generation reference",
                        )
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.checkpoint();
                            this.doc.artboard_references.remove(i);
                            this.changed();
                            cx.notify();
                        })),
                    ),
            );
        }
        if !self.doc.artboard_references.is_empty() {
            controls = controls.child(references);
        }
        if self.show_prompt_details || separate {
            let options = row()
                .items_end()
                .flex_wrap()
                .when(!drawing || separate, |r| {
                    r.child(number_field("Width · px", &self.generation_width).w(px(112.)))
                        .child(label("×").mb_1())
                        .child(number_field("Height · px", &self.generation_height).w(px(112.)))
                })
                .when(self.show_prompt_details && (!drawing || separate), |r| {
                    r.child(
                        column()
                            .gap_1()
                            .w(px(136.))
                            .child(label("Size preset"))
                            .child(
                                Select::new(&self.artboard_size_select)
                                    .small()
                                    .accessibility_label("Resolution")
                                    .cleanable(false)
                                    .w_full(),
                            ),
                    )
                })
                .when(
                    self.show_prompt_details
                        && self.settings.provider.native_architecture().is_some(),
                    |r| {
                        r.child(
                            column().gap_1().w(px(120.)).child(label("Sampling")).child(
                                Select::new(&self.artboard_steps_select)
                                    .small()
                                    .accessibility_label("Sampling steps")
                                    .cleanable(false)
                                    .w_full(),
                            ),
                        )
                    },
                )
                .when(self.show_prompt_details, |r| {
                    r.child(
                        tool_button("composer-settings", "settings", "Image settings")
                            .ghost()
                            .small()
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.panel = Panel::Settings;
                                cx.notify();
                            })),
                    )
                });
            controls = controls.child(options);
        }
        if self.show_prompt_details {
            if !drawing && mode != PromptComposerMode::Image {
                controls = controls.child(row().gap_1().flex_wrap().child(label("Start with").mr_1()).children([
                    ("recipe-character","Character","Full-body character design: [describe character, outfit, personality and palette]. Clear silhouette, expressive face, neutral background, no text.",1024,1536),
                    ("recipe-object","Object","Prop concept: [describe object and story role]. Single object, three-quarter view, clear silhouette, no text.",1024,1024),
                    ("recipe-background","Background","Illustrated environment: [describe place, mood, lighting and composition]. No text.",1536,1024),
                    ("recipe-illustration","Illustration","Illustration: [describe subject, medium, mood and composition]. No text.",1536,1024)
                ].into_iter().map(|(id,name,prompt,w,h)| creation_recipe_button(id,name,prompt,w,h,cx))));
            }
            if mode == PromptComposerMode::Image {
                if let Some(source) = self.edit_source().filter(|r| !r.prompt.is_empty()) {
                    controls = controls.child(
                        row().child(
                            Button::new("reuse-source-prompt")
                                .ghost()
                                .small()
                                .label("Reuse source prompt")
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    this.prompt.update(cx, |s, cx| {
                                        s.set_value(source.prompt.clone(), window, cx)
                                    });
                                    this.prompt.update(cx, |s, cx| s.focus(window, cx));
                                    cx.notify();
                                })),
                        ),
                    );
                }
            }
        }
        if setup_needed || self.show_prompt_details {
            controls =
                controls.child(
                    row()
                        .gap_1()
                        .pt_2()
                        .border_t_1()
                        .border_color(rgb(BORDER))
                        .child(
                            label(
                                self.provider_readiness
                                    .strip_prefix("Setup needed: ")
                                    .unwrap_or(&self.provider_readiness)
                                    .to_owned(),
                            )
                            .text_color(rgb(if setup_needed { WINE } else { MUTED }))
                            .min_w_0()
                            .flex_1(),
                        )
                        .child(
                            Button::new("composer-clipforge")
                                .ghost()
                                .small()
                                .label("Open Clipforge")
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.open_clipforge();
                                    cx.notify();
                                })),
                        )
                        .child(
                            tool_button("composer-refresh", "undo", "Refresh provider readiness")
                                .tooltip(server_status_label(self.server_status.as_ref()))
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.reload_clipforge(window, cx)
                                })),
                        ),
                );
        }
        prompt_composer_frame(&self.prompt, controls, self.generation_error.as_ref())
    }
    /// The queue panel, opened from the header badge or the generation rail.
    fn queue_panel(&self, cx: &mut Context<Self>) -> AnyElement {
        let mut content = column().child(
            row()
                .child(
                    label(format!(
                        "{} waiting · {} finished",
                        self.queue.pending(),
                        self.queue.finished()
                    ))
                    .flex_1(),
                )
                .when(!self.queue.is_empty(), |row| {
                    let paused = self.queue.is_paused();
                    row.child(
                        Button::new("queue-pause")
                            .ghost()
                            .small()
                            .icon(icon(if paused { "play" } else { "pause" }))
                            .label(if paused { "Resume" } else { "Pause" })
                            .accessibility_label(if paused {
                                "Resume the queue"
                            } else {
                                "Pause the queue"
                            })
                            .tooltip(if paused {
                                "Resume the queue"
                            } else {
                                "Pause the queue"
                            })
                            .atelier_selected(paused, cx)
                            .on_click(cx.listener(|this, _, _, cx| this.queue_pause_toggle(cx))),
                    )
                })
                .when(self.generation.is_some(), |row| {
                    row.child(
                        Button::new("cancel-generation")
                            .ghost()
                            .small()
                            .icon(icon("close"))
                            .label("Stop")
                            .accessibility_label("Stop the running image")
                            .tooltip("Stop the running image")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.cancel_generation();
                                cx.notify();
                            })),
                    )
                })
                .when(self.queue.finished() > 0, |row| {
                    row.child(
                        Button::new("queue-clear")
                            .ghost()
                            .small()
                            .icon(icon("trash"))
                            .label("Clear finished")
                            .accessibility_label("Clear finished requests")
                            .tooltip("Clear finished requests")
                            .on_click(cx.listener(|this, _, _, cx| this.queue_clear_finished(cx))),
                    )
                }),
        );
        if self.queue.is_empty() {
            content = content.child(self.queue_empty_state());
        } else {
            for item in self.queue.items() {
                content = content.child(self.queue_row(item, cx));
            }
        }
        self.overlay("Generation queue", content, cx)
    }
    /// One queue entry, oldest first. A queued request can be removed; a running
    /// or finished one is left in place so its outcome stays visible.
    fn queue_row(&self, item: &crate::queue::QueueItem, cx: &mut Context<Self>) -> Div {
        use crate::queue::JobState;
        let id = item.id;
        let prompt = item
            .request
            .prompt
            .lines()
            .next()
            .unwrap_or("")
            .trim()
            .to_string();
        let mut detail = column()
            .gap_1()
            .flex_1()
            .min_w_0()
            .child(div().text_sm().truncate().child(prompt))
            .child(label(item.request.summary()));
        match &item.state {
            JobState::Running { progress } if !progress.is_empty() => {
                detail = detail.child(label(progress.clone()));
            }
            JobState::Failed(message) => {
                detail = detail.child(label(message.clone()).text_color(rgb(WINE)));
            }
            _ => {}
        }
        let mut entry = row()
            .items_start()
            .gap_3()
            .child(
                div()
                    .w(px(66.))
                    .flex_shrink_0()
                    .child(job_state_label(&item.state)),
            )
            .child(detail);
        if item.state == JobState::Queued {
            entry = entry.child(
                Button::new(format!("queue-remove-{id}"))
                    .ghost()
                    .small()
                    .label("Remove")
                    .accessibility_label("Remove from queue")
                    .tooltip("Remove from queue")
                    .on_click(cx.listener(move |this, _, _, cx| this.queue_remove(id, cx))),
            );
        }
        if matches!(item.state, JobState::Failed(_)) {
            entry = entry.child(
                Button::new(format!("queue-retry-{id}"))
                    .ghost()
                    .small()
                    .label("Retry")
                    .on_click(cx.listener(move |this, _, _, cx| this.queue_retry(id, cx))),
            );
        }
        if matches!(item.state, JobState::Done) && item.result_id.is_some() {
            entry = entry.child(
                Button::new(format!("queue-open-{id}"))
                    .ghost()
                    .small()
                    .label("Open")
                    .accessibility_label("Open generated result")
                    .tooltip("Open generated result")
                    .on_click(cx.listener(move |this, _, _, cx| this.open_queue_result(id, cx))),
            );
        }
        panel().w_full().p_3().child(entry)
    }
    fn queue_empty_state(&self) -> Div {
        column().items_center().justify_center().py_8().child(
            div()
                .text_sm()
                .text_color(rgb(MUTED))
                .child("Nothing queued"),
        )
    }
    fn drawing_composer(&self, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .id("floating-drawing-composer")
            .absolute()
            .bottom(px(18.))
            .left(px(0.))
            .right(px(0.))
            .flex()
            .justify_center()
            .child(self.prompt_composer(PromptComposerMode::Drawing, cx))
    }

    pub(crate) fn reference_details(&self, cx: &mut Context<Self>) -> Div {
        let mut details = column()
            .child(field("Name", &self.ref_name))
            .child(field("Tags", &self.ref_tags))
            .child(
                row()
                    .child(
                        tool_button("reference-details", "save", "Save image details").on_click(
                            cx.listener(|this, _, _, cx| {
                                this.edit_reference(cx);
                                cx.notify();
                            }),
                        ),
                    )
                    .when(self.page != Page::Image, |p| {
                        p.child(
                            tool_button("remove-reference", "trash", "Remove reference · undoable")
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.remove_reference(window, cx);
                                    cx.notify();
                                })),
                        )
                    }),
            );
        if let Some(reference) = self.edit_source() {
            details = details.child(label(reference.provenance.clone()));
        }
        details
    }
    fn overlay(&self, title: &'static str, content: Div, cx: &mut Context<Self>) -> AnyElement {
        let settings = title == "Settings";
        let rail_brushes = title == "Brushes" && self.page == Page::Draw;
        let rail_settings = settings && self.page == Page::Draw;
        let rail_help = title == "Practice" && self.page == Page::Draw;
        let rail_popup = rail_brushes || rail_settings || rail_help;
        let panel = panel()
            .id(format!("overlay-{title}"))
            // Floating panels must block hitboxes underneath, including canvas focus.
            .occlude()
            .absolute()
            .when(rail_popup, |p| p.left(px(64.)))
            .when(!rail_popup, |p| p.right(px(16.)))
            .when(rail_brushes, |p| p.top(px(170.)))
            .when(rail_settings, |p| p.top(px(64.)).bottom(px(80.)))
            .when(rail_help, |p| p.bottom(px(40.)))
            .when(!rail_popup, |p| p.top(px(64.)))
            .w(px(if title == "References" {
                470.
            } else if settings {
                360.
            } else if title == "Generation queue" {
                336.
            } else {
                288.
            }))
            .when(settings, |p| {
                p.when(!rail_settings, |p| p.bottom(px(16.)))
                    .flex()
                    .flex_col()
                    .overflow_hidden()
            })
            .when(!settings, |p| p.max_h(px(660.)).overflow_y_scroll())
            .p_5()
            .shadow_lg()
            .child(
                row()
                    .flex_shrink_0()
                    .mb_4()
                    .child(div().flex_1().text_lg().child(title))
                    .child(
                        tool_button("close-overlay", "close", "Close panel").on_click(cx.listener(
                            |this, _, window, cx| {
                                this.panel = Panel::None;
                                window.focus(&this.focus, cx);
                                cx.notify();
                            },
                        )),
                    ),
            )
            .child(if settings {
                div()
                    .id("settings-scroll-body")
                    .flex_1()
                    .min_h(px(0.))
                    .overflow_y_scroll()
                    .pr_2()
                    .child(content)
                    .into_any_element()
            } else {
                content.into_any_element()
            })
            .when(settings, |p| {
                p.child(
                    div()
                        .flex_shrink_0()
                        .border_t_1()
                        .border_color(rgb(BORDER))
                        .mt_4()
                        .pt_4()
                        .when_some(self.settings_save_error.as_ref(), |p, error| {
                            p.child(label(error.clone()).text_color(rgb(WINE)).mb_2())
                        })
                        .child(
                            Button::new("save-settings")
                                .primary()
                                .w_full()
                                .label(if self.settings_save.is_some() {
                                    "Saving…"
                                } else {
                                    "Save settings"
                                })
                                .disabled(self.settings_save.is_some())
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.apply_settings(cx);
                                    cx.notify();
                                })),
                        ),
                )
            });
        if self.settings.reduced_motion {
            panel.into_any_element()
        } else {
            panel
                .with_animation(
                    format!("reveal-{title}"),
                    Animation::new(Duration::from_millis(150)),
                    |panel, delta| panel.opacity(0.75 + delta * 0.25),
                )
                .into_any_element()
        }
    }
    fn brush_settings(&self, cx: &mut Context<Self>) -> AnyElement {
        let mut content = column();
        for (index, brush) in Brush::ALL.into_iter().enumerate() {
            content = content.child(
                row()
                    .child(
                        Button::new(format!("brush-{brush:?}"))
                            .ghost()
                            .label(brush.name())
                            .icon(icon(if brush == Brush::Graphite {
                                "pencil"
                            } else {
                                "brush"
                            }))
                            .atelier_selected(self.brush == brush && self.tool == Tool::Pencil, cx)
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.finish_stroke();
                                this.brush = brush;
                                this.tool = Tool::Pencil;
                                this.pencil_size = brush.size();
                                this.panel = Panel::None;
                                cx.notify();
                            })),
                    )
                    .child(
                        img(self.brush_previews[index].clone())
                            .w(px(120.))
                            .h(px(32.))
                            .object_fit(ObjectFit::Contain),
                    ),
            );
        }
        content = content
            .child(separator())
            .child(
                Button::new("pen-pressure")
                    .ghost()
                    .label("Pen pressure")
                    .atelier_selected(self.pressure_enabled, cx)
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.finish_stroke();
                        this.pressure_enabled = !this.pressure_enabled;
                        cx.notify();
                    })),
            )
            .child(
                row().children(
                    [(0.65f32, "Light"), (1., "Natural"), (1.65, "Firm")]
                        .into_iter()
                        .map(|(curve, name)| {
                            Button::new(format!("pressure-{name}"))
                                .ghost()
                                .small()
                                .label(name)
                                .atelier_selected(self.pressure_curve == curve, cx)
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.finish_stroke();
                                    this.pressure_curve = curve;
                                    cx.notify();
                                }))
                        }),
                ),
            )
            .child(label(
                "Pressure controls width and pigment. Mouse uses full pressure.",
            ));
        self.overlay("Brushes", content, cx)
    }
    fn canvas_settings(&self, cx: &mut Context<Self>) -> AnyElement {
        let content = column()
            .child(field("Project title", &self.title))
            .child(field("Active layer name", &self.layer_name))
            .child(
                row().children(
                    [
                        ("Square", 1800, 1800),
                        ("Portrait", 1800, 2400),
                        ("Landscape", 2400, 1800),
                        ("A4", 2480, 3508),
                    ]
                    .into_iter()
                    .map(|(name, w, h)| {
                        Button::new(format!("preset-{name}"))
                            .ghost()
                            .small()
                            .label(name)
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.width
                                    .update(cx, |s, cx| s.set_value(w.to_string(), window, cx));
                                this.height
                                    .update(cx, |s, cx| s.set_value(h.to_string(), window, cx));
                                cx.notify();
                            }))
                    }),
                ),
            )
            .child(
                row()
                    .child(field("Width (px)", &self.width).flex_1())
                    .child(field("Height (px)", &self.height).flex_1()),
            )
            .child(
                Button::new("scale-art")
                    .ghost()
                    .small()
                    .label(if self.scale_art {
                        "Scale artwork with canvas ✓"
                    } else {
                        "Resize canvas bounds"
                    })
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.scale_art = !this.scale_art;
                        cx.notify();
                    })),
            )
            .child(
                Button::new("apply-canvas")
                    .primary()
                    .label("Apply")
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.apply_canvas(window, cx);
                        cx.notify();
                    })),
            )
            .child(
                row()
                    .child(
                        Button::new("rotate-document")
                            .ghost()
                            .small()
                            .icon(icon("rotate"))
                            .label("Rotate artwork")
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.checkpoint();
                                this.doc.rotate();
                                this.changed();
                                this.view = Viewport::default();
                                this.width.update(cx, |s, cx| {
                                    s.set_value(this.doc.width.to_string(), window, cx)
                                });
                                this.height.update(cx, |s, cx| {
                                    s.set_value(this.doc.height.to_string(), window, cx)
                                });
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new("flip-document")
                            .ghost()
                            .small()
                            .icon(icon("flip"))
                            .label("Flip artwork")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.checkpoint();
                                this.doc.flip();
                                this.changed();
                                cx.notify();
                            })),
                    ),
            )
            .child(separator())
            .child("Export")
            .child(
                row()
                    .child(
                        Button::new("export-png")
                            .ghost()
                            .small()
                            .label("PNG")
                            .atelier_selected(!self.jpeg, cx)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.jpeg = false;
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new("export-jpg")
                            .ghost()
                            .small()
                            .label("JPG")
                            .atelier_selected(self.jpeg, cx)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.jpeg = true;
                                cx.notify();
                            })),
                    ),
            )
            .when(!self.jpeg, |p| {
                p.child(
                    Button::new("transparent")
                        .ghost()
                        .small()
                        .label("Transparent background")
                        .atelier_selected(self.transparent, cx)
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.transparent = !this.transparent;
                            cx.notify();
                        })),
                )
            })
            .when(self.jpeg, |p| {
                p.child(
                    Button::new("jpg-quality")
                        .ghost()
                        .small()
                        .label(format!(
                            "Quality {}% · warm paper background",
                            self.jpeg_quality
                        ))
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.jpeg_quality = if this.jpeg_quality == 100 {
                                70
                            } else {
                                this.jpeg_quality + 10
                            };
                            cx.notify();
                        })),
                )
            })
            .child(
                Button::new("export-file")
                    .icon(icon("export"))
                    .label("Export file…")
                    .on_click(
                        cx.listener(|this, _, window, cx| this.pick(Picker::Export, window, cx)),
                    ),
            )
            .child(
                Button::new("canvas-export-studio")
                    .label("Export canvas for Studio…")
                    .on_click(
                        cx.listener(|this, _, window, cx| this.pick(Picker::Artwork, window, cx)),
                    ),
            )
            .child(
                Button::new("save-as")
                    .ghost()
                    .small()
                    .label("Save project as…")
                    .on_click(cx.listener(|this, _, window, cx| this.save(true, window, cx))),
            )
            .child(
                Button::new("sessions")
                    .ghost()
                    .small()
                    .label("Open session archive")
                    .on_click(cx.listener(|this, _, _, cx| {
                        cx.reveal_path(&this.data_dir.join("sessions"));
                    })),
            );
        self.overlay("Canvas & project", content, cx)
    }
    fn lora_editor(&self, cx: &mut Context<Self>) -> Div {
        let discovered = crate::native_images::available_loras(&self.settings);
        let addable: Vec<String> = discovered
            .iter()
            .filter(|name| !self.settings.loras.iter().any(|lora| lora.name == **name))
            .cloned()
            .collect();
        let mut content = column().child(label("LoRA adapters")).child(
            Select::new(&self.lora_select)
                .placeholder("Add adapter…")
                .search_placeholder("Search adapters")
                .cleanable(false)
                .disabled(addable.is_empty())
                .w_full(),
        );
        if discovered.is_empty() {
            content = content.child(label("No adapters found in this model's lora directory."));
        } else if self.settings.loras.is_empty() {
            content = content.child(label("No adapters applied."));
        }
        for lora in &self.settings.loras {
            let name = lora.name.clone();
            let mut entry = row().child(div().flex_1().min_w_0().truncate().child(name.clone()));
            if let Some(input) = self.lora_weights.get(&name) {
                entry = entry.child(NumberInput::new(input).small().w(px(84.)));
            }
            entry = entry.child(
                tool_button(format!("lora-remove-{name}"), "close", "Remove adapter").on_click(
                    cx.listener(move |this, _, window, cx| {
                        this.remove_lora(&name, window, cx);
                        cx.notify();
                    }),
                ),
            );
            content = content.child(entry);
        }
        content
    }
    fn provider_settings(&self, cx: &mut Context<Self>) -> AnyElement {
        let provider = self.settings.provider;
        let mut content = column()
            .child(
                div()
                    .text_sm()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child("Provider"),
            )
            .child(self.provider_choices(true));
        if provider.native_architecture().is_some() {
            content = content
                .child(label(if provider.supports_edit() {
                    "Generation and editing · one source image"
                } else {
                    "Generation only"
                }))
                .child(label(
                    "Size, steps and LoRA adapters are set per request in the generation rail.",
                ));
        } else if provider != Provider::Comfy {
            content = content
                .child(separator())
                .child(
                    div()
                        .text_sm()
                        .font_weight(FontWeight::SEMIBOLD)
                        .child("Models"),
                )
                .child(label("Model presets"))
                .child(column().children(provider.presets().into_iter().map(
                    |(name, model, edit)| {
                        Button::new(format!("preset-{model}-{edit}"))
                            .ghost()
                            .small()
                            .label(name)
                            .atelier_selected(
                                self.model.read(cx).value() == model
                                    && self.edit_model.read(cx).value() == edit,
                                cx,
                            )
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.select_image_preset(model, edit, window, cx)
                            }))
                    },
                )))
                .child(field("Generation model", &self.model))
                .child(field("Editing model", &self.edit_model))
                .child(label(
                    "Leave editing model blank for models that only create images.",
                ));
            content = content.child(separator())
                .child(label("Provider connections, credentials and secrets are managed in Clipforge. Request model presets stay here."));
        } else {
            content = content
                .child(separator())
                .child(
                    div()
                        .text_sm()
                        .font_weight(FontWeight::SEMIBOLD)
                        .child("ComfyUI"),
                )
                .child(field("Server address", &self.endpoint))
                .child(
                    Button::new("load-workflow")
                        .small()
                        .icon(icon("open"))
                        .label(if self.settings.workflow.is_null() {
                            "Load API workflow…"
                        } else {
                            "Replace API workflow…"
                        })
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.pick(Picker::Workflow, window, cx)
                        })),
                )
                .child(
                    row()
                        .child(field("Prompt node", &self.positive_node).flex_1())
                        .child(field("Size node", &self.latent_node).flex_1())
                        .child(field("Seed node", &self.seed_node).flex_1()),
                )
                .child(label(
                    "Leave size / seed nodes blank to keep workflow values",
                ));
        }
        let content = content
            .when(
                self.settings.provider.native_architecture().is_some(),
                |p| p.child(self.lora_editor(cx)),
            )
            .child(
                Button::new("settings-clipforge")
                    .small()
                    .label("Manage providers in Clipforge")
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.open_clipforge();
                        cx.notify();
                    })),
            )
            .child(separator())
            .child(
                div()
                    .text_sm()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child("Accessibility"),
            )
            .child(
                Button::new("reduced-motion")
                    .ghost()
                    .small()
                    .label("Reduce motion")
                    .atelier_selected(self.settings.reduced_motion, cx)
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.settings.reduced_motion = !this.settings.reduced_motion;
                        cx.notify();
                    })),
            );
        self.overlay("Settings", content, cx)
    }
    fn help(&self, cx: &mut Context<Self>) -> AnyElement {
        let exercises = [
            (
                "Contour",
                "Follow the silhouette. Look at the reference more than your pencil.",
            ),
            (
                "Negative space",
                "Draw the shapes around the subject before filling it in.",
            ),
            (
                "Proportions",
                "Mark the largest shapes and compare their width and height.",
            ),
            (
                "Values",
                "Use three tones to separate light, middle and shadow.",
            ),
            (
                "Gesture",
                "Capture direction and weight with a few loose lines.",
            ),
        ];
        let (title, description) = exercises[self.exercise % exercises.len()];
        let content = column()
            .child(self.study_timer(cx))
            .child(div().text_lg().child(title))
            .child(description)
            .child(
                Button::new("next-exercise")
                    .ghost()
                    .small()
                    .label("Next prompt")
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.exercise = (this.exercise + 1) % 5;
                        cx.notify();
                    })),
            )
            .child(separator())
            .child("Reflection")
            .child(Textarea::new(&self.note).h(px(100.)))
            .child(
                Button::new("save-note")
                    .small()
                    .label("Save reflection")
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.save_note(cx);
                        cx.notify();
                    })),
            )
            .child(separator())
            .child("Canvas shortcuts")
            .children(
                [
                    "B Pencil · E Eraser · H Pan",
                    "[ / ] Brush size · R Rotate view · F Mirror",
                    "0 Fit view · Tab Move keyboard focus",
                    "⌘Z Undo · ⇧⌘Z Redo · ⌘S Save",
                    "⌘O Open · ⇧⌘S Save as",
                    "Scroll Pan · ⌘Scroll / pinch Zoom",
                ]
                .map(label),
            );
        self.overlay("Practice", content, cx)
    }
}
