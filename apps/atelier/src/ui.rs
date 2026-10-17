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
}

fn prompt_composer_frame(
    prompt: &Entity<TextareaState>,
    controls: impl IntoElement,
    error: Option<&String>,
) -> impl IntoElement {
    panel()
        .w(px(720.))
        .max_w_full()
        .p_3()
        .shadow_lg()
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
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.open_project(path.clone(), cx);
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
                        this.page = Page::Draw;
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
                tool_button("rail-timer", "timer", "Start / pause study")
                    .atelier_selected(self.timer_running, cx)
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.timer_toggle();
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
                    "Prompt from drawing · ⌘P",
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
                cx.listener(|this, _, _, cx| {
                    this.finish_stroke();
                    this.finish_drawing_region(cx);
                    cx.notify();
                }),
            )
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                    this.finish_stroke();
                    this.finish_drawing_region(cx);
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
        let mut dock = column()
            .id("drawing-dock")
            .w(px(256.))
            .flex_shrink_0()
            .h_full()
            .overflow_y_scroll()
            .p_3()
            .border_l_1()
            .border_color(rgb(BORDER));
        let mut reference = panel().p_3().flex().flex_col().gap_2().child(
            row().child(div().flex_1().child("Reference")).child(
                tool_button("reference-import", "plus", "Import references").on_click(
                    cx.listener(|this, _, window, cx| this.pick(Picker::Import, window, cx)),
                ),
            ),
        );
        if let Some(i) = self.doc.reference.filter(|i| *i < self.references.len()) {
            let image = self.references[i].clone();
            let grid = self.grid;
            let grayscale = self.grayscale;
            let zoom = self.ref_zoom;
            let pan = self.ref_pan;
            reference = reference
                .child(
                    div()
                        .id("reference-viewport")
                        .h(px(250.))
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
                .child(
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
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.ref_zoom = 1.;
                                this.ref_pan = crate::document::Point { x: 0., y: 0. };
                                cx.notify();
                            })),
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
                .when(self.tracing, |panel| panel.child(label("Assisted study")));
        } else {
            reference = reference.child(
                div()
                    .h(px(180.))
                    .flex()
                    .flex_col()
                    .items_center()
                    .justify_center()
                    .gap_3()
                    .child(icon("image").text_color(rgb(MUTED)))
                    .child(
                        Button::new("choose-reference")
                            .ghost()
                            .label("Choose reference")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.page = Page::Library;
                                cx.notify();
                            })),
                    ),
            );
        }
        dock = dock.child(reference);
        let mut layers = panel().p_3().flex().flex_col().gap_2().child(
            row().child(div().flex_1().child("Layers")).child(
                tool_button("add-layer", "plus", "Add layer").on_click(cx.listener(
                    |this, _, _, cx| {
                        this.add_layer();
                        cx.notify();
                    },
                )),
            ),
        );
        for (i, layer) in self.doc.layers.iter().enumerate().rev() {
            layers = layers.child(
                row()
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
                        Button::new(format!("layer-{i}"))
                            .ghost()
                            .small()
                            .label(layer.name.clone())
                            .atelier_selected(i == self.doc.active_layer, cx)
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.finish_stroke();
                                this.doc.active_layer = i;
                                this.layer_name.update(cx, |s, cx| {
                                    s.set_value(this.doc.layers[i].name.clone(), window, cx)
                                });
                                cx.notify();
                            })),
                    )
                    .child(div().flex_1()),
            );
        }
        layers = layers.child(
            row()
                .child(
                    tool_button("layer-up", "up", "Move layer up").on_click(cx.listener(
                        |this, _, _, cx| {
                            this.move_layer(true);
                            cx.notify();
                        },
                    )),
                )
                .child(
                    tool_button("layer-down", "down", "Move layer down").on_click(cx.listener(
                        |this, _, _, cx| {
                            this.move_layer(false);
                            cx.notify();
                        },
                    )),
                )
                .child(
                    Button::new("layer-opacity")
                        .ghost()
                        .small()
                        .label(format!(
                            "{}%",
                            (self.doc.layers[self.doc.active_layer].opacity * 100.) as u32
                        ))
                        .tooltip("Cycle layer opacity")
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.checkpoint();
                            let l = &mut this.doc.layers[this.doc.active_layer];
                            l.opacity = if l.opacity < 0.3 {
                                1.
                            } else {
                                l.opacity - 0.25
                            };
                            this.changed();
                            cx.notify();
                        })),
                )
                .child(
                    tool_button("delete-layer", "trash", "Delete layer · undoable")
                        .disabled(self.doc.layers.len() == 1)
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.delete_layer();
                            cx.notify();
                        })),
                ),
        );
        if self.doc.layers[self.doc.active_layer].image.is_some() {
            layers = layers.child(label("Image layer · move, scale, draw or erase"))
                .child(row().flex_wrap()
                    .child(Button::new("move-image-layer").ghost().small().label("Move layer").atelier_selected(self.moving_image && self.tool == Tool::Hand,cx)
                        .on_click(cx.listener(|this,_,_,cx| { this.finish_stroke(); this.moving_image = !this.moving_image; this.tool = Tool::Hand; cx.notify(); })))
                    .children([("smaller-image-layer","Smaller",0.8),("larger-image-layer","Larger",1.25)].into_iter().map(|(id,label,scale)| Button::new(id).small().label(label)
                        .on_click(cx.listener(move |this,_,_,cx| { this.checkpoint(); match this.doc.reposition_image_layer(0.,0.,scale) { Ok(()) => this.changed(), Err(error) => this.message(error.to_string(),true) } cx.notify(); })))))
                .child(label("Move layer: drag on the canvas. Scale keeps painted marks aligned. Undo restores the previous placement."));
        }
        dock.child(layers).child(self.practice(cx))
    }
    fn practice(&self, cx: &mut Context<Self>) -> Div {
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
            .child(
                Button::new("practice-guide")
                    .ghost()
                    .small()
                    .label("Practice prompts")
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.panel = Panel::Help;
                        cx.notify();
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
    fn library_navigation(&self, cx: &mut Context<Self>) -> Div {
        row()
            .child(
                Button::new("library-shared")
                    .ghost()
                    .small()
                    .label("All projects")
                    .atelier_selected(self.shared_library, cx)
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.shared_library = true;
                        this.refresh_library();
                        cx.notify();
                    })),
            )
            .child(
                Button::new("library-references")
                    .ghost()
                    .small()
                    .label("This project")
                    .atelier_selected(!self.shared_library, cx)
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.shared_library = false;
                        cx.notify();
                    })),
            )
    }
    fn library(&self, cx: &mut Context<Self>) -> Div {
        let query = self.search.read(cx).value().to_lowercase();
        let mut cards = div()
            .id("reference-cards")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .flex()
            .flex_wrap()
            .content_start()
            .gap_4();
        let items: Vec<_> = if self.shared_library {
            self.library
                .iter()
                .map(|(reference, image)| (reference.clone(), image.clone()))
                .collect()
        } else {
            self.doc
                .references
                .iter()
                .cloned()
                .zip(self.references.iter().cloned())
                .collect()
        };
        for (i, (reference, image)) in items.iter().enumerate().filter(|(_, (r, _))| {
            (!self.favorite_filter || r.favorite)
                && format!("{} {}", r.name, r.tags)
                    .to_lowercase()
                    .contains(&query)
        }) {
            {
                let open = reference.clone();
                cards = cards.child(
                    panel()
                        .w(px(218.))
                        .overflow_hidden()
                        .when(!self.shared_library && self.doc.reference == Some(i), |p| {
                            p.border_color(rgb(WINE))
                        })
                        .child(
                            div()
                                .id(format!("reference-image-{i}"))
                                .h(px(190.))
                                .w_full()
                                .overflow_hidden()
                                .cursor_pointer()
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    if !this.shared_library {
                                        this.select_reference(i, window, cx);
                                    }
                                    this.open_image(open.clone(), cx);
                                    cx.notify();
                                }))
                                .child(
                                    img(image.clone())
                                        .size_full()
                                        .object_fit(ObjectFit::Contain),
                                ),
                        )
                        .child(
                            column()
                                .gap_1()
                                .child(
                                    row()
                                        .px_2()
                                        .py_2()
                                        .child(
                                            div()
                                                .flex_1()
                                                .overflow_hidden()
                                                .child(reference.name.clone()),
                                        )
                                        .child(
                                            tool_button(
                                                format!("favorite-{i}"),
                                                "heart",
                                                "Favorite image",
                                            )
                                            .atelier_selected(reference.favorite, cx)
                                            .on_click(
                                                cx.listener(move |this, _, _, cx| {
                                                    if this.shared_library {
                                                        this.toggle_library_favorite(i);
                                                    } else {
                                                        this.toggle_favorite(i);
                                                    }
                                                    cx.notify();
                                                }),
                                            ),
                                        ),
                                )
                                .child(
                                    row()
                                        .px_2()
                                        .pb_2()
                                        .child(
                                            Button::new(format!("open-image-{i}"))
                                                .ghost()
                                                .small()
                                                .icon(icon("open"))
                                                .label("Open")
                                                .accessibility_label("Open image")
                                                .tooltip("Open image")
                                                .on_click(cx.listener({
                                                    let reference = reference.clone();
                                                    move |this, _, _, cx| {
                                                        this.open_image(reference.clone(), cx)
                                                    }
                                                })),
                                        )
                                        .child(
                                            Button::new(format!("input-reference-{i}"))
                                                .ghost()
                                                .small()
                                                .icon(icon("reference"))
                                                .label("Reference")
                                                .accessibility_label("Add as an AI reference")
                                                .tooltip("Add as an AI reference")
                                                .on_click(cx.listener({
                                                    let reference = reference.clone();
                                                    move |this, _, _, cx| {
                                                        this.add_input_reference(reference.clone());
                                                        cx.notify();
                                                    }
                                                })),
                                        )
                                        .when(self.shared_library, |row| {
                                            row.child(
                                                Button::new(format!("use-library-{i}"))
                                                    .ghost()
                                                    .small()
                                                    .icon(icon("plus"))
                                                    .label("Use")
                                                    .accessibility_label("Use in this project")
                                                    .tooltip("Use in this project")
                                                    .on_click(cx.listener(
                                                        move |this, _, window, cx| {
                                                            this.use_library_image(i, window, cx);
                                                            cx.notify();
                                                        },
                                                    )),
                                            )
                                        }),
                                ),
                        ),
                );
            }
        }
        if items.is_empty() && !self.library_loading {
            cards = cards.child(
                column()
                    .w_full()
                    .items_center()
                    .justify_center()
                    .h(px(300.))
                    .child(icon("image"))
                    .child("Your reference collection starts here")
                    .child(Button::new("empty-import").label("Import images").on_click(
                        cx.listener(|this, _, window, cx| this.pick(Picker::Import, window, cx)),
                    )),
            );
        }
        row()
            .items_start()
            .gap_0()
            .flex_1()
            .min_w_0()
            .h_full()
            .child(
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
                                    .child("Image library"),
                            )
                            .child(div().flex_1())
                            .child(
                                tool_button("favorites-filter", "heart", "Show favorites")
                                    .atelier_selected(self.favorite_filter, cx)
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.favorite_filter = !this.favorite_filter;
                                        cx.notify();
                                    })),
                            )
                            .child(tool_button("import", "plus", "Import images").on_click(
                                cx.listener(|this, _, window, cx| {
                                    this.pick(Picker::Import, window, cx)
                                }),
                            )),
                    )
                    .child(self.library_navigation(cx))
                    .child(Input::new(&self.search))
                    .when(self.library_loading, |p| p.child(label("Loading library…")))
                    .when_some(self.library_error.as_ref(), |p, error| {
                        p.child(label(error.clone()).text_color(rgb(WINE)))
                    })
                    .when(self.library_error.is_some(), |p| {
                        p.child(
                            Button::new("library-retry")
                                .small()
                                .label("Retry library")
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.refresh_library();
                                    cx.notify();
                                })),
                        )
                    })
                    .child(cards)
                    .child(label(format!("{} images", items.len()))),
            )
            .child(self.generate_panel(cx))
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
        let prompt = match mode {
            PromptComposerMode::Artboards => &self.prompt,
            PromptComposerMode::Drawing => &self.from_drawing_prompt,
        };
        prompt_composer_frame(
            prompt,
            row()
                .pt_1()
                .items_center()
                .gap_2()
                .when(mode == PromptComposerMode::Drawing, |row| {
                    row.child(
                        Button::new("drawing-area")
                            .ghost()
                            .small()
                            .icon(icon("mask-rect"))
                            .label(if self.selecting_generation_region {
                                "Area"
                            } else {
                                "Full"
                            })
                            .tooltip("Drag on the canvas to select an area")
                            .atelier_selected(self.selecting_generation_region, cx)
                            .on_click(cx.listener(|this, _, _, cx| this.toggle_box_select(cx))),
                    )
                })
                .child(
                    div()
                        .w(px(if mode == PromptComposerMode::Drawing {
                            130.
                        } else {
                            155.
                        }))
                        .flex_shrink_0()
                        .child(
                            Select::new(&self.generation_provider_select)
                                .small()
                                .accessibility_label("Image provider")
                                .cleanable(false)
                                .w_full(),
                        ),
                )
                .child(
                    div()
                        .w(px(if mode == PromptComposerMode::Drawing {
                            145.
                        } else {
                            160.
                        }))
                        .flex_shrink_0()
                        .child(
                            Select::new(&self.artboard_aspect_select)
                                .small()
                                .accessibility_label("Aspect ratio")
                                .cleanable(false)
                                .w_full(),
                        ),
                )
                .child(
                    div()
                        .w(px(if mode == PromptComposerMode::Drawing {
                            130.
                        } else {
                            145.
                        }))
                        .flex_shrink_0()
                        .child(
                            Select::new(&self.artboard_size_select)
                                .small()
                                .accessibility_label("Resolution")
                                .cleanable(false)
                                .w_full(),
                        ),
                )
                .when(
                    self.settings.provider.native_architecture().is_some(),
                    |row| {
                        row.child(
                            div()
                                .w(px(if mode == PromptComposerMode::Drawing {
                                    95.
                                } else {
                                    125.
                                }))
                                .flex_shrink_0()
                                .child(
                                    Select::new(&self.artboard_steps_select)
                                        .small()
                                        .accessibility_label("Sampling steps")
                                        .cleanable(false)
                                        .w_full(),
                                ),
                        )
                    },
                )
                .child(div().flex_1())
                .child(
                    Button::new(match mode {
                        PromptComposerMode::Artboards => "artboards-create-image",
                        PromptComposerMode::Drawing => "from-drawing-generate",
                    })
                    .primary()
                    .icon(icon("sparkle"))
                    .accessibility_label(match mode {
                        PromptComposerMode::Artboards => "Create image",
                        PromptComposerMode::Drawing => "Create from drawing",
                    })
                    .tooltip(match mode {
                        PromptComposerMode::Artboards => "Create image",
                        PromptComposerMode::Drawing => "Create from drawing",
                    })
                    .disabled(
                        mode == PromptComposerMode::Drawing
                            && !self.settings.provider.supports_edit(),
                    )
                    .on_click(cx.listener(move |this, _, window, cx| match mode {
                        PromptComposerMode::Artboards => {
                            this.editing_image = false;
                            this.enqueue_generation(window, cx);
                        }
                        PromptComposerMode::Drawing => this.generate_from_drawing(window, cx),
                    })),
                ),
            if mode == PromptComposerMode::Artboards {
                self.generation_error.as_ref()
            } else {
                None
            },
        )
    }

    pub(crate) fn generate_panel(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let can_edit = self.settings.provider.supports_edit()
            && self.edit_source().is_some()
            && (self.settings.provider.native_architecture().is_some()
                || !self.edit_model.read(cx).value().trim().is_empty());
        let mut content = column()
            .id("generate-panel")
            .w(px(320.))
            .flex_shrink_0()
            .h_full()
            .overflow_y_scroll()
            .p_3()
            .bg(rgb(0xffffff))
            .border_l_1()
            .border_color(rgb(BORDER))
            .child(
                row()
                    .child(self.provider_choices(false).flex_1().min_w_0())
                    .child(label("Generation")),
            )
            .child(
                row()
                    .child(label("Prompt").flex_1())
                    .when_some(
                        self.edit_source().filter(|_| self.editing_image),
                        |row, reference| {
                            row.child(
                                tool_button("reuse-prompt", "undo", "Reuse this version's prompt")
                                    .disabled(reference.prompt.is_empty())
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        this.prompt.update(cx, |s, cx| {
                                            s.set_value(reference.prompt.clone(), window, cx)
                                        });
                                        this.prompt.update(cx, |s, cx| s.focus(window, cx));
                                        cx.notify();
                                    })),
                            )
                        },
                    )
                    .child(
                        Button::new("create-image")
                            .ghost()
                            .small()
                            .icon(icon("image"))
                            .label("Create")
                            .accessibility_label("Create a new image")
                            .tooltip("Create a new image")
                            .atelier_selected(!self.editing_image, cx)
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.editing_image = false;
                                this.restore_prompt(window, cx);
                                this.prompt.update(cx, |s, cx| s.focus(window, cx));
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new("edit-selected-image")
                            .ghost()
                            .small()
                            .icon(icon("pencil"))
                            .label("Edit")
                            .accessibility_label("Edit selected image")
                            .tooltip(if can_edit {
                                "Edit selected image"
                            } else {
                                "Select an image and editing provider first"
                            })
                            .atelier_selected(self.editing_image, cx)
                            .disabled(!can_edit)
                            .on_click(cx.listener(|this, _, window, cx| {
                                if this.page == Page::Image {
                                    this.edit_opened_image(window, cx);
                                } else {
                                    this.editing_image = true;
                                    this.restore_prompt(window, cx);
                                    this.prompt.update(cx, |s, cx| s.focus(window, cx));
                                }
                                cx.notify();
                            })),
                    ),
            )
            .when(!self.editing_image, |panel| {
                panel
                    .child(label("Start with a game-art brief"))
                    .child(row().children([
                        creation_recipe_button(
                            "recipe-character",
                            "Character",
                            "Full-body character design for a visual novel: [describe the character, outfit, personality, and palette]. Clear readable silhouette, expressive face, neutral studio background, no text.",
                            1024,
                            1536,
                            cx,
                        ),
                        creation_recipe_button(
                            "recipe-object",
                            "Object",
                            "Game prop concept for [describe the object and its story role]. Single object, clear silhouette, three-quarter view, restrained backdrop, finely rendered materials, no text.",
                            1024,
                            1024,
                            cx,
                        ),
                    ]))
                    .child(row().children([
                        creation_recipe_button(
                            "recipe-background",
                            "Background",
                            "Visual novel background of [describe the place, time, and mood]. Wide establishing composition, inviting depth, clear area for character dialogue, no people, no text.",
                            1536,
                            1024,
                            cx,
                        ),
                        creation_recipe_button(
                            "recipe-key-art",
                            "Illustration",
                            "Narrative game illustration of [describe the moment, characters, and emotion]. Strong focal point, cinematic composition, deliberate color palette, no lettering.",
                            1024,
                            1536,
                            cx,
                        ),
                    ]))
            })
            .child(Textarea::new(&self.prompt).h(px(180.)))
            .when(self.settings.provider.hosted().is_some(), |p| {
                p.child(
                    label("Hosted generation may incur provider charges.").text_color(rgb(AMBER)),
                )
            })
            .when(
                self.settings.provider.native_architecture().is_some(),
                |p| {
                    p.child(
                        row()
                            .child(label(server_status_label(self.server_status.as_ref())).flex_1())
                            .child(
                                tool_button(
                                    "refresh-server-status",
                                    "undo",
                                    "Check the local server",
                                )
                                .on_click(cx.listener(
                                    |this, _, _, cx| {
                                        this.refresh_server_status();
                                        cx.notify();
                                    },
                                )),
                            ),
                    )
                },
            )
            .child(
                row().children(
                    [
                        ("1:1", 1024, 1024),
                        ("3:2", 1536, 1024),
                        ("2:3", 1024, 1536),
                    ]
                    .into_iter()
                    .enumerate()
                    .map(|(i, (name, w, h))| {
                        Button::new(format!("aspect-{i}"))
                            .ghost()
                            .small()
                            .label(name)
                            .atelier_selected(
                                self.settings.width == w && self.settings.height == h,
                                cx,
                            )
                            .disabled(self.editing_image && self.use_mask)
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.generation_width
                                    .update(cx, |s, cx| s.set_value(w.to_string(), window, cx));
                                this.generation_height
                                    .update(cx, |s, cx| s.set_value(h.to_string(), window, cx));
                                this.settings.width = w;
                                this.settings.height = h;
                                this.sync_artboard_selects(window, cx);
                                cx.notify();
                            }))
                    }),
                ),
            )
            .child(separator())
            .child(label("Output size"))
            .child(
                row()
                    .child(number_field("Width", &self.generation_width).flex_1())
                    .child(number_field("Height", &self.generation_height).flex_1()),
            )
            .when(
                self.settings.provider.native_architecture().is_some(),
                |p| {
                    p.child(number_field("Steps", &self.steps))
                        .child(self.lora_editor(cx))
                },
            )
            .child(
                row()
                    .child(
                        Button::new("enqueue-generation")
                            .primary()
                            .icon(icon("sparkle"))
                            .label(if self.editing_image {
                                "Queue edit"
                            } else {
                                "Queue image"
                            })
                            .tooltip(if self.settings.provider.hosted().is_none() {
                                "Queue an image with the settings above"
                            } else {
                                "Queue an image · API charges apply"
                            })
                            .disabled(self.editing_image && !can_edit)
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.enqueue_generation(window, cx);
                            })),
                    )
                    .child(div().flex_1())
                    .when(self.generation.is_some(), |p| {
                        p.child(
                            tool_button("cancel-generation", "close", "Stop the running image")
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.cancel_generation();
                                    cx.notify();
                                })),
                        )
                    }),
            )
            .when(!self.queue.is_empty(), |p| {
                p.child(
                    Button::new("queue-summary")
                        .ghost()
                        .small()
                        .w_full()
                        .icon(icon("layers"))
                        .label(format!(
                            "{} waiting · {} finished",
                            self.queue.pending(),
                            self.queue.finished()
                        ))
                        .tooltip("Open the generation queue")
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.panel = Panel::Queue;
                            cx.notify();
                        })),
                )
            })
            .when_some(self.generation_error.as_ref(), |p, error| {
                p.child(label(error.clone()).text_color(rgb(WINE)))
            })
            .when(self.generation.is_some(), |p| {
                p.child(label(self.status.clone()))
            });
        if !self.input_references.is_empty() {
            content = content.child(separator());
            for (i, reference) in self.input_references.iter().enumerate() {
                content = content.child(
                    row()
                        .child(label(format!("{} · {}", i + 2, reference.name)).flex_1())
                        .child(
                            tool_button(
                                format!("remove-input-{i}"),
                                "close",
                                "Remove AI reference",
                            )
                            .on_click(cx.listener(
                                move |this, _, _, cx| {
                                    this.input_references.remove(i);
                                    cx.notify();
                                },
                            )),
                        ),
                );
            }
        }
        if let Some(reference) = self.edit_source() {
            content = content
                .child(separator())
                .child(
                    row()
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .truncate()
                                .child(reference.name.clone()),
                        )
                        .when(self.page != Page::Image, |p| {
                            p.child(
                                tool_button("open-selected-image", "open", "Open selected image")
                                    .on_click(cx.listener({
                                        let reference = reference.clone();
                                        move |this, _, _, cx| this.open_image(reference.clone(), cx)
                                    })),
                            )
                            .child(
                                tool_button("use-reference", "brush", "Draw this reference")
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.page = Page::Draw;
                                        this.dock = true;
                                        window.focus(&this.focus, cx);
                                        cx.notify();
                                    })),
                            )
                        })
                        .child(
                            tool_button("image-details", "info", "Image details")
                                .atelier_selected(self.image_details_open, cx)
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.image_details_open = !this.image_details_open;
                                    cx.notify();
                                })),
                        ),
                )
                .when(self.image_details_open, |p| {
                    p.child(self.reference_details(cx))
                });
        }
        content
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
            .occlude()
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(self.prompt_composer(PromptComposerMode::Drawing, cx))
    }
    fn reference_details(&self, cx: &mut Context<Self>) -> Div {
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
            .w(px(if settings {
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
            if provider == Provider::Alibaba {
                content = content
                    .child(separator())
                    .child(div().text_sm().font_weight(FontWeight::SEMIBOLD).child("Connection"))
                    .child(field("Regional API base URL", &self.endpoint))
                    .child(label("Use your workspace's regional URL ending in /compatible-mode/v1. The key and models must belong to that region."));
            }
            content = content
                .child(separator())
                .child(
                    div()
                        .text_sm()
                        .font_weight(FontWeight::SEMIBOLD)
                        .child("Credentials"),
                )
                .child(label(format!(
                    "Stored in the shared clipforge.toml provider entry; set {} only if you prefer an environment credential.",
                    provider.key_env()
                )))
                .child(field("API key", &self.key))
                .child(label(
                    "Save updates Clipforge's shared provider registry. Leave blank to keep the existing shared key.",
                ));
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
