use crate::{
    app::{DockView, DrawingApp, Page, Panel, Picker},
    document::{Brush, Tool},
    providers::Provider,
    viewport::Viewport,
};
use gpui_kit::{
    component::{
        button::*,
        input::{Input, InputState, NumberInput, Textarea, TextareaState},
        menu::{ContextMenuExt, DropdownMenu, PopupMenu, PopupMenuItem},
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

fn selection_context_menu(
    menu: PopupMenu,
    app: Entity<DrawingApp>,
    cx: &mut Context<PopupMenu>,
) -> PopupMenu {
    let (
        available,
        has_selection,
        locked,
        can_crop,
        can_group,
        can_ungroup,
        can_forward,
        can_backward,
    ) = {
        let state = app.read(cx);
        let available = state.image_crop.is_none();
        let can_crop = available
            && state.selected_layers.len() == 1
            && state.selected_group_name().is_none()
            && state
                .doc
                .layers
                .get(state.selected_layers[0])
                .is_some_and(|layer| layer.image.is_some() && !state.doc.layer_is_locked(layer));
        (
            available,
            available && !state.selected_layers.is_empty(),
            state.selection_is_locked(),
            can_crop,
            available
                && state.selected_layers.len() >= 2
                && state.selected_layers.iter().all(|index| {
                    state
                        .doc
                        .layers
                        .get(*index)
                        .is_some_and(|layer| layer.group.is_none())
                }),
            available && state.selected_group_name().is_some(),
            available
                && state
                    .selected_move_range()
                    .is_some_and(|(_, end)| end < state.doc.layers.len()),
            available
                && state
                    .selected_move_range()
                    .is_some_and(|(start, _)| start > 0),
        )
    };
    let duplicate = app.clone();
    let crop = app.clone();
    let group = app.clone();
    let ungroup = app.clone();
    let zoom = app.clone();
    let forward = app.clone();
    let backward = app.clone();
    let front = app.clone();
    let back = app.clone();
    let flip_horizontal = app.clone();
    let flip_vertical = app.clone();
    let paste = app.clone();
    menu.item(
        PopupMenuItem::new("Paste image")
            .disabled(!available)
            .on_click(move |_, _, cx| paste.update(cx, |this, cx| this.paste_artwork(cx))),
    )
    .item(
        PopupMenuItem::new("Duplicate selection")
            .disabled(!has_selection || locked)
            .on_click(move |_, window, cx| {
                duplicate.update(cx, |this, cx| this.duplicate_selected_layers(window, cx))
            }),
    )
    .item(
        PopupMenuItem::new("Crop image")
            .disabled(!can_crop)
            .on_click(move |_, window, cx| {
                crop.update(cx, |this, cx| {
                    this.start_image_crop(cx);
                    window.focus(&this.focus, cx);
                })
            }),
    )
    .item(
        PopupMenuItem::new("Group layers")
            .disabled(!can_group)
            .on_click(move |_, window, cx| {
                group.update(cx, |this, cx| {
                    this.group_selected_layers();
                    this.sync_layer_controls(window, cx);
                    cx.notify();
                })
            }),
    )
    .item(
        PopupMenuItem::new("Ungroup")
            .disabled(!can_ungroup)
            .on_click(move |_, window, cx| {
                ungroup.update(cx, |this, cx| {
                    this.ungroup_selected_layers();
                    this.sync_layer_controls(window, cx);
                    cx.notify();
                })
            }),
    )
    .item(
        PopupMenuItem::new("Move forward")
            .disabled(!can_forward || locked)
            .on_click(move |_, window, cx| {
                forward.update(cx, |this, cx| {
                    this.move_layer(true);
                    this.sync_layer_controls(window, cx);
                    cx.notify();
                })
            }),
    )
    .item(
        PopupMenuItem::new("Move backward")
            .disabled(!can_backward || locked)
            .on_click(move |_, window, cx| {
                backward.update(cx, |this, cx| {
                    this.move_layer(false);
                    this.sync_layer_controls(window, cx);
                    cx.notify();
                })
            }),
    )
    .item(
        PopupMenuItem::new("Bring to front")
            .disabled(!can_forward || locked)
            .on_click(move |_, window, cx| {
                front.update(cx, |this, cx| this.move_selection_to_edge(true, window, cx))
            }),
    )
    .item(
        PopupMenuItem::new("Send to back")
            .disabled(!can_backward || locked)
            .on_click(move |_, window, cx| {
                back.update(cx, |this, cx| {
                    this.move_selection_to_edge(false, window, cx)
                })
            }),
    )
    .item(
        PopupMenuItem::new("Flip horizontally")
            .disabled(!has_selection || locked)
            .on_click(move |_, window, cx| {
                flip_horizontal.update(cx, |this, cx| this.flip_selection(true, false, window, cx))
            }),
    )
    .item(
        PopupMenuItem::new("Flip vertically")
            .disabled(!has_selection || locked)
            .on_click(move |_, window, cx| {
                flip_vertical.update(cx, |this, cx| this.flip_selection(false, true, window, cx))
            }),
    )
    .item(
        PopupMenuItem::new("Zoom to selection")
            .disabled(!has_selection)
            .on_click(move |_, _, cx| zoom.update(cx, |this, cx| this.zoom_to_selection(cx))),
    )
    .item(
        PopupMenuItem::new("Delete selection")
            .disabled(!has_selection || locked)
            .on_click(move |_, window, cx| {
                app.update(cx, |this, cx| {
                    let revision = this.revision;
                    this.delete_layer();
                    if this.revision != revision {
                        this.sync_inputs(window, cx);
                    }
                    cx.notify();
                })
            }),
    )
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum PromptComposerMode {
    Artboards,
    Drawing,
    Image,
}

#[derive(Clone)]
pub(crate) struct DraggedLayerRows {
    pub epoch: u64,
    pub artboard: u32,
    pub revision: u64,
    pub start: usize,
    pub end: usize,
    pub label: String,
}

impl Render for DraggedLayerRows {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        panel().px_3().py_2().text_sm().child(self.label.clone())
    }
}

fn prompt_composer_frame(
    prompt: &Entity<TextareaState>,
    controls: impl IntoElement,
    error: Option<&str>,
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
            composer.child(label(error.to_owned()).text_color(rgb(WINE)))
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
        JobState::Running { .. } | JobState::Aborting => AMBER,
        JobState::Done => GREEN,
        JobState::Failed(_) => WINE,
    };
    label(state.label()).text_color(rgb(color))
}
fn separator() -> Div {
    div().h(px(1.)).w_full().bg(rgb(BORDER)).my_2()
}
pub(crate) fn icon(name: &str) -> Icon {
    let path = match name {
        "pencil" => "<path d='m15 4 5 5M4 15l11-11a3 3 0 0 1 5 5L9 20l-6 1z'/>",
        "select" => "<path d='M5 3v16l4-4 3 6 3-1-3-6h6z'/>",
        "brush" => "<path d='m14 3 7 7-9 9-7-7zM5 12l-3 9 9-3M14 3l-3 3'/>",
        "eraser" => "<path d='m15 3 6 6-10 12H6l-4-4zM7 12l6 6M11 21h10'/>",
        "hand" => {
            "<path d='M8 13V6a2 2 0 0 1 4 0v6-8a2 2 0 0 1 4 0v8-6a2 2 0 0 1 4 0v10c0 5-3 7-7 7-3 0-5-2-7-5l-3-5a2 2 0 0 1 3-2z'/>"
        }
        "image" => {
            "<rect x='3' y='3' width='18' height='18' rx='3'/><circle cx='8' cy='8' r='1'/><path d='m3 17 6-6 4 4 3-3 5 5'/>"
        }
        "timer" => "<circle cx='12' cy='14' r='8'/><path d='M9 2h6M12 6V2m6 5 2-2M12 10v5'/>",
        "save" => "<path d='M5 3h12l4 4v14H3V3zM7 3v6h10V3M7 21v-7h10v7'/>",
        "export" => "<path d='M12 16V3m-4 4 4-4 4 4M5 13H3v8h18v-8h-2'/>",
        "download" => "<path d='M12 3v13m-4-4 4 4 4-4M3 16v5h18v-5'/>",
        "back" => "<path d='m10 5-7 7 7 7M3 12h18'/>",
        "layers" => "<path d='m12 3 10 5-10 5L2 8zM2 12l10 5 10-5M2 16l10 5 10-5'/>",
        "package" => "<path d='m12 2 9 5v10l-9 5-9-5V7zM3 7l9 5 9-5M12 12v10M7 4l10 5'/>",
        "compare" => {
            "<rect x='3' y='3' width='18' height='18' rx='2'/><path d='M12 3v18M6 12h3M15 12h3'/>"
        }
        "mask" => {
            "<rect x='3' y='3' width='18' height='18' rx='2' stroke-dasharray='3 3'/><path d='m7 16 9-9M7 11l4-4M12 17l5-5'/>"
        }
        "mask-rect" => "<rect x='4' y='5' width='16' height='14' rx='1' stroke-dasharray='3 3'/>",
        "reference" => {
            "<rect x='3' y='3' width='13' height='13' rx='2'/><path d='M8 21h13V8M3 12l4-4 4 4 2-2 3 3'/>"
        }
        "info" => "<circle cx='12' cy='12' r='9'/><path d='M12 11v6M12 7h.01'/>",
        "open" => "<path d='M3 7V4h6l3 3h9v3M3 10h18l-3 11H3z'/>",
        "more" => {
            "<circle cx='5' cy='12' r='1'/><circle cx='12' cy='12' r='1'/><circle cx='19' cy='12' r='1'/>"
        }
        "plus" => "<path d='M12 5v14M5 12h14'/>",
        "minus" => "<path d='M5 12h14'/>",
        "undo" => "<path d='m9 4-6 6 6 6M3 10h12a6 6 0 0 1 0 12'/>",
        "redo" => "<path d='m15 4 6 6-6 6M21 10H9a6 6 0 0 0 0 12'/>",
        "rotate" => "<path d='M20 10a8 8 0 1 0-2 8M20 3v7h-7'/>",
        "flip" => "<path d='M12 3v18M8 6 3 12l5 6zM16 6l5 6-5 6z'/>",
        "fit" => "<path d='M3 9V3h6M15 3h6v6M21 15v6h-6M9 21H3v-6'/>",
        "search" => "<circle cx='10' cy='10' r='6'/><path d='m15 15 5 5'/>",
        "grid" => {
            "<rect x='3' y='3' width='18' height='18' rx='2'/><path d='M9 3v18M15 3v18M3 9h18M3 15h18'/>"
        }
        "values" => {
            "<circle cx='12' cy='12' r='9'/><path d='M12 3v18' /><path d='M12 3a9 9 0 0 1 0 18' fill='currentColor'/>"
        }
        "smooth" => "<path d='M2 15c3 0 3-6 6-6s3 6 6 6 3-6 6-6h2'/>",
        "eye" => {
            "<path d='M2 12s4-7 10-7 10 7 10 7-4 7-10 7S2 12 2 12z'/><circle cx='12' cy='12' r='3'/>"
        }
        "eye-off" => "<path d='m3 3 18 18M9 5h3c6 0 10 7 10 7l-3 4M6 6l-4 6s4 7 10 7h3'/>",
        "heart" => "<path d='M12 21 3 12a6 6 0 0 1 9-8 6 6 0 0 1 9 8z'/>",
        "settings" => {
            "<path d='M4 6h16M4 12h16M4 18h16'/><circle cx='8' cy='6' r='2' fill='white'/><circle cx='16' cy='12' r='2' fill='white'/><circle cx='10' cy='18' r='2' fill='white'/>"
        }
        "lock" => {
            "<rect x='4' y='10' width='16' height='11' rx='2'/><path d='M8 10V7a4 4 0 1 1 8 0v3M12 14v3'/>"
        }
        "unlock" => {
            "<rect x='4' y='10' width='16' height='11' rx='2'/><path d='M8 10V7a4 4 0 0 1 8 0M12 14v3'/>"
        }
        "panel" => "<rect x='3' y='3' width='18' height='18' rx='2'/><path d='M15 3v18'/>",
        "align-left" => "<path d='M5 3v18M9 7h10v4H9zM9 15h7v4H9z'/>",
        "align-center" => "<path d='M12 3v18M5 7h14v4H5zM8 15h8v4H8z'/>",
        "align-right" => "<path d='M19 3v18M5 7h10v4H5zM8 15h7v4H8z'/>",
        "align-top" => "<path d='M3 5h18M7 9h4v10H7zM15 9h4v7h-4z'/>",
        "align-middle" => "<path d='M3 12h18M7 5h4v14H7zM15 8h4v8h-4z'/>",
        "align-bottom" => "<path d='M3 19h18M7 5h4v10H7zM15 8h4v7h-4z'/>",
        "distribute-x" => "<path d='M4 4v16M20 4v16M8 7h3v10H8zM13 7h3v10h-3z'/>",
        "distribute-y" => "<path d='M4 4h16M4 20h16M7 8h10v3H7zM7 13h10v3H7z'/>",
        "play" => "<path d='m7 3 14 9-14 9z'/>",
        "pause" => "<path d='M8 4v16M16 4v16'/>",
        "trash" => "<path d='M3 6h18M9 6V3h6v3M5 6l1 15h12l1-15M10 10v7M14 10v7'/>",
        "up" => "<path d='m6 14 6-6 6 6'/>",
        "down" => "<path d='m6 10 6 6 6-6'/>",
        "check" => "<path d='m4 12 5 5L20 6'/>",
        "close" => "<path d='m5 5 14 14M19 5 5 19'/>",
        "copy" => "<rect x='8' y='8' width='13' height='13' rx='2'/><path d='M16 8V3H3v13h5'/>",
        "sparkle" => "<path d='m12 3 3 6 6 3-6 3-3 6-3-6-6-3 6-3z'/>",
        "alert" => "<path d='m12 3 10 18H2z'/><path d='M12 9v5M12 17h.01'/>",
        "activity" => "<circle cx='12' cy='12' r='9'/><path d='M12 7v5l3 2'/>",
        "help" => {
            "<circle cx='12' cy='12' r='9'/><path d='M9 9a3 3 0 1 1 5 2c-2 1-2 1-2 3M12 17h.01'/>"
        }
        _ => "<circle cx='12' cy='12' r='8'/>",
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

pub(crate) fn field(name: &'static str, input: &Entity<InputState>) -> Div {
    column()
        .gap_1()
        .child(label(name))
        .child(Input::new(input).aria_label(name))
}

fn property_field(name: &'static str, input: &Entity<InputState>, disabled: bool) -> Div {
    column()
        .gap_1()
        .child(label(name))
        .child(Input::new(input).aria_label(name).disabled(disabled))
}

pub(crate) fn side_panel_frame(title: &'static str, content: Div, close: Button) -> Div {
    column()
        .key_context("SidePanel")
        .w(px(360.))
        .flex_shrink_0()
        .h_full()
        .min_h_0()
        .gap_2()
        .bg(rgb(0xffffff))
        .border_l_1()
        .border_color(rgb(BORDER))
        .p_4()
        .occlude()
        .child(
            row()
                .flex_shrink_0()
                .child(div().flex_1().text_lg().child(title))
                .child(close),
        )
        .child(
            div()
                .id(format!("side-panel-scroll-{title}"))
                .flex_1()
                .min_h_0()
                .overflow_y_scroll()
                .child(content),
        )
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
        Some(ServerStatus::Stopped) => "Local server: stopped · starts on demand".to_string(),
        Some(ServerStatus::Unavailable) => {
            "Local server: unavailable · check provider configuration in Clipforge".to_string()
        }
        None => "Local server: checking…".to_string(),
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
                Button::new(format!("open-project-{i}"))
                    .custom(
                        ButtonCustomVariant::new(cx)
                            .color(rgb(0xffffff).into())
                            .foreground(rgb(INK).into())
                            .hover(rgb(SURFACE).into())
                            .active(rgb(0xf1e6eb).into()),
                    )
                    .w(px(240.))
                    .h_auto()
                    .p_0()
                    .border_1()
                    .border_color(rgb(BORDER))
                    .rounded_lg()
                    .overflow_hidden()
                    .cursor_pointer()
                    .accessibility_label(format!("Open {}", project.title))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.open_project(path.clone(), window, cx);
                        cx.notify();
                    }))
                    .child(
                        column()
                            .gap_0()
                            .w_full()
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
                                    .w_full()
                                    .text_left()
                                    .child(div().truncate().child(project.title.clone()))
                                    .when(!project.note.is_empty(), |p| {
                                        p.child(
                                            label(
                                                project.note.chars().take(80).collect::<String>(),
                                            )
                                            .truncate(),
                                        )
                                    }),
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
                        Button::new("home-library")
                            .ghost()
                            .small()
                            .icon(icon("image"))
                            .label("Library")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.page = Page::Library;
                                cx.notify();
                            })),
                    )
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
        let side_panel = match self.panel {
            Panel::StoryContext => Some(
                side_panel_frame(
                    "Story context",
                    column().gap_3().child(label(
                        self.doc
                            .story_context
                            .get(&self.doc.canvas_id)
                            .cloned()
                            .unwrap_or_default(),
                    )),
                    tool_button("story-context-close", "close", "Close story context").on_click(
                        cx.listener(|this, _, _, cx| {
                            this.panel = Panel::None;
                            cx.notify();
                        }),
                    ),
                )
                .into_any_element(),
            ),
            Panel::Catalog => Some(self.catalog_panel(cx)),
            Panel::Brushes => Some(self.brush_settings(cx)),
            Panel::Canvas => Some(self.artboard_settings(cx)),
            Panel::Export => Some(self.export_settings(cx)),
            Panel::Settings => Some(self.provider_settings(cx)),
            Panel::Help => Some(self.help(cx)),
            Panel::Activity => Some(self.activity_panel(cx).into_any_element()),
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
            .on_action(cx.listener(|this, _: &crate::OpenSettings, _, cx| {
                this.panel = Panel::Settings;
                this.refresh_server_status();
                cx.notify();
                cx.stop_propagation();
            }))
            .on_action(cx.listener(|this, _: &crate::ClosePanel, window, cx| {
                if this.cancel_layer_input(window, cx) {
                    cx.stop_propagation();
                } else if this.panel != Panel::None {
                    this.panel = Panel::None;
                    window.focus(&this.focus, cx);
                    cx.notify();
                    cx.stop_propagation();
                } else if this.dock {
                    this.dock = false;
                    window.focus(&this.focus, cx);
                    cx.notify();
                    cx.stop_propagation();
                }
            }))
            .on_action(cx.listener(|this, _: &crate::CloseProject, _, cx| {
                this.close_project_tab(this.epoch, cx);
                cx.stop_propagation();
            }))
            .on_action(cx.listener(|this, _: &crate::MoveProjectLeft, _, cx| {
                this.move_project_tab(-1, cx);
                cx.stop_propagation();
            }))
            .on_action(cx.listener(|this, _: &crate::MoveProjectRight, _, cx| {
                this.move_project_tab(1, cx);
                cx.stop_propagation();
            }))
            .child(self.header(cx))
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .when(matches!(self.page, Page::Draw | Page::Image), |layout| {
                        layout.child(self.rail(cx))
                    })
                    .child(div().flex_1().min_w_0().h_full().flex().child(main))
                    .when_some(side_panel, |layout, panel| layout.child(panel)),
            )
            .child(self.status_bar())
            .when(
                self.switching || self.closing || self.closing_project.is_some(),
                |layout| {
                    layout.child(
                        div()
                            .id("switching-shield")
                            .absolute()
                            .size_full()
                            .bg(rgba(0xf7f6f8ee))
                            .flex()
                            .items_center()
                            .justify_center()
                            .child(if self.closing_project.is_some() {
                                "Saving project before closing…"
                            } else if self.closing {
                                "Saving session…"
                            } else {
                                "Opening project…"
                            })
                            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation()),
                    )
                },
            )
            .into_any_element()
    }
    fn header(&self, cx: &mut Context<Self>) -> Div {
        let project_context = !self.project_tab_order.is_empty()
            && !matches!(self.page, Page::Projects | Page::Library);
        row()
            .h(px(44.))
            .flex_shrink_0()
            .px_4()
            .border_b_1()
            .border_color(rgb(BORDER))
            .bg(rgb(0xffffff))
            .child(
                Button::new("atelier-home")
                    .ghost()
                    .small()
                    .label("Atelier")
                    .tooltip("Home · Projects and Library")
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(rgb(WINE))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.finish_stroke();
                        this.finish_mask();
                        this.page = Page::Projects;
                        this.refresh_projects();
                        cx.notify();
                    })),
            )
            .child(self.project_tab_bar(cx))
            .when(project_context, |header| {
                header.when_some(self.activity.save_failure(self.epoch), |header, report| {
                    header.child(self.report_hint(report.id, "Not saved · View", cx))
                })
            })
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
            .when(project_context, |header| {
                header
                    .child(
                        tool_button("save", "save", "Save project · ⌘S").on_click(
                            cx.listener(|this, _, window, cx| this.save(false, window, cx)),
                        ),
                    )
                    .child(
                        tool_button("export", "export", "Export PNG / JPG").on_click(cx.listener(
                            |this, _, _, cx| {
                                this.panel = Panel::Export;
                                cx.notify();
                            },
                        )),
                    )
            })
            .child(div().w(px(8.)))
            .child(self.queue_button(cx))
    }
    fn project_tab_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let tabs = self.project_tab_order.iter().filter_map(|epoch| {
            if *epoch == self.epoch {
                Some((*epoch, self.doc.title.clone(), self.dirty))
            } else {
                self.project_tabs
                    .iter()
                    .find(|tab| tab.epoch == *epoch)
                    .map(|tab| (tab.epoch, tab.doc.title.clone(), tab.dirty))
            }
        });
        div()
            .id("project-tabs")
            .flex_1()
            .min_w_0()
            .overflow_x_scroll()
            .flex()
            .gap_1()
            .children(tabs.map(|(epoch, title, dirty)| {
                let active =
                    epoch == self.epoch && !matches!(self.page, Page::Library | Page::Projects);
                row()
                    .id(format!("project-tab-container-{epoch}"))
                    .gap_1()
                    .h(px(30.))
                    .pl_3()
                    .pr_1()
                    .max_w(px(240.))
                    .flex_shrink_0()
                    .border_1()
                    .border_color(gpui_kit::transparent_black())
                    .rounded_md()
                    .bg(rgb(if active { 0xf1e6eb } else { 0xffffff }))
                    .text_color(rgb(if active { WINE } else { MUTED }))
                    .hover(|style| style.bg(rgb(0xf1e6eb)))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.activate_project_tab(epoch, window, cx)
                    }))
                    .drag_over::<crate::project_tabs::DraggedProject>(|tab, _, _, _| {
                        tab.border_color(rgb(WINE))
                    })
                    .on_drag(
                        crate::project_tabs::DraggedProject {
                            epoch,
                            title: title.clone(),
                        },
                        |drag, _, _, cx| cx.new(|_| drag.clone()),
                    )
                    .on_drop(cx.listener(
                        move |this, drag: &crate::project_tabs::DraggedProject, _, cx| {
                            this.reorder_project_tab(drag.epoch, epoch, cx);
                        },
                    ))
                    .child(
                        Button::new(format!("project-tab-{epoch}"))
                            .custom(
                                ButtonCustomVariant::new(cx)
                                    .foreground(rgb(if active { WINE } else { MUTED }).into()),
                            )
                            .small()
                            .px_0()
                            .min_w_0()
                            .flex_1()
                            .label(title.clone())
                            .tooltip(format!("{title} · drag to reorder"))
                            .selected(active)
                            .toggled(active)
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.activate_project_tab(epoch, window, cx);
                                cx.stop_propagation();
                            })),
                    )
                    .when(dirty, |tab| {
                        tab.child(div().flex_shrink_0().text_xs().child("•"))
                    })
                    .child(
                        Button::new(format!("close-project-{epoch}"))
                            .custom(ButtonCustomVariant::new(cx).foreground(rgb(MUTED).into()))
                            .small()
                            .group("project-tab-close")
                            .child(
                                div()
                                    .flex()
                                    .group_hover("project-tab-close", |style| {
                                        style.text_color(rgb(WINE))
                                    })
                                    .child(icon("close")),
                            )
                            .tooltip("Save and close project")
                            .disabled(self.closing_project.is_some())
                            .accessibility_label(format!("Save and close {title}"))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.close_project_tab(epoch, cx);
                                cx.stop_propagation();
                            })),
                    )
            }))
    }
    /// The header's queue control: an icon with a count badge while requests are
    /// outstanding, opening the queue panel.
    fn queue_button(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let outstanding = self.queue.outstanding();
        let failures = self.activity.count();
        let count = if failures > 0 { failures } else { outstanding };
        div()
            .relative()
            .child(
                tool_button(
                    "open-queue",
                    if failures > 0 { "alert" } else { "activity" },
                    "Activity",
                )
                .accessibility_label(if failures > 0 {
                    format!(
                        "Activity · {failures} {}",
                        if failures == 1 { "issue" } else { "issues" }
                    )
                } else if outstanding > 0 {
                    format!(
                        "Activity · {outstanding} {} pending",
                        if outstanding == 1 {
                            "request"
                        } else {
                            "requests"
                        }
                    )
                } else {
                    "Activity".into()
                })
                .atelier_selected(self.panel == Panel::Activity, cx)
                .on_click(cx.listener(|this, _, _, cx| {
                    this.panel = if this.panel == Panel::Activity {
                        Panel::None
                    } else {
                        if this.activity_selection.is_none_or(|id| {
                            this.activity.get(id).is_none_or(|report| report.resolved)
                        }) {
                            this.activity_selection = this
                                .activity
                                .reports()
                                .iter()
                                .rev()
                                .find(|report| report.needs_attention())
                                .map(|report| report.id);
                            this.activity_details = false;
                        }
                        Panel::Activity
                    };
                    cx.notify();
                })),
            )
            .when(count > 0, |element| {
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
                        .child(count.to_string()),
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
        rail = rail.child(
            tool_button(
                "tool-select",
                "select",
                "Select layers · V · ⌘-click cycles overlapping layers",
            )
            .atelier_selected(self.tool == Tool::Hand && self.moving_image, cx)
            .on_click(cx.listener(|this, _, window, cx| {
                this.finish_stroke();
                if this.image_crop.take().is_some() {
                    this.visual_revision += 1;
                }
                this.tool = Tool::Hand;
                this.moving_image = true;
                this.selecting_generation_region = false;
                this.panel = Panel::None;
                this.page = Page::Draw;
                this.restore_prompt(window, cx);
                window.focus(&this.focus, cx);
                cx.notify();
            })),
        );
        for (tool, name, tooltip) in [
            (Tool::Pencil, "pencil", "Pencil · B"),
            (Tool::Eraser, "eraser", "Eraser · E"),
            (Tool::Hand, "hand", "Pan · H / middle drag"),
        ] {
            rail = rail.child(
                tool_button(format!("tool-{name}"), name, tooltip)
                    .atelier_selected(
                        self.tool == tool && !(tool == Tool::Hand && self.moving_image),
                        cx,
                    )
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.finish_stroke();
                        if this.image_crop.take().is_some() {
                            this.visual_revision += 1;
                        }
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
                        if this.image_crop.take().is_some() {
                            this.visual_revision += 1;
                        }
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
            .when(self.dock && self.panel == Panel::None, |layout| {
                layout.child(self.dock_panel(cx))
            })
    }
    fn canvas_toolbar(&self, cx: &mut Context<Self>) -> Div {
        row()
            .h(px(46.))
            .px_4()
            .when(
                self.doc.story_context.contains_key(&self.doc.canvas_id),
                |bar| {
                    bar.child(
                        tool_button("story-context", "info", "Story context").on_click(
                            cx.listener(|this, _, _, cx| {
                                this.panel = Panel::StoryContext;
                                cx.notify();
                            }),
                        ),
                    )
                },
            )
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
                tool_button(
                    "artboard-inspector",
                    "grid",
                    "Artboard dimensions and resize",
                )
                .atelier_selected(self.panel == Panel::Canvas, cx)
                .on_click(cx.listener(|this, _, window, cx| {
                    this.width.update(cx, |s, cx| {
                        s.set_value(this.doc.width.to_string(), window, cx)
                    });
                    this.height.update(cx, |s, cx| {
                        s.set_value(this.doc.height.to_string(), window, cx)
                    });
                    this.panel = if this.panel == Panel::Canvas {
                        Panel::None
                    } else {
                        Panel::Canvas
                    };
                    cx.notify();
                })),
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
                tool_button("zoom-to-selection", "search", "Zoom to selected artwork")
                    .disabled(
                        self.doc
                            .selected_layer_bounds(&self.selected_layers)
                            .is_none(),
                    )
                    .on_click(cx.listener(|this, _, _, cx| this.zoom_to_selection(cx))),
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
                tool_button(
                    "dock",
                    "panel",
                    if self.panel == Panel::Activity {
                        "Show layer inspector"
                    } else {
                        "Toggle layer inspector"
                    },
                )
                .atelier_selected(self.dock && self.panel != Panel::Activity, cx)
                .on_click(cx.listener(|this, _, _, cx| {
                    if this.panel == Panel::Activity {
                        this.panel = Panel::None;
                        this.dock_view = DockView::Layers;
                        this.dock = true;
                    } else {
                        this.dock_view = DockView::Layers;
                        this.dock = !this.dock;
                    }
                    cx.notify();
                })),
            )
            .child(
                tool_button("references-view", "reference", "Open References inspector")
                    .atelier_selected(self.dock && self.dock_view == DockView::References, cx)
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.panel = Panel::None;
                        this.dock_view = DockView::References;
                        this.dock = true;
                        cx.notify();
                    })),
            )
    }
    fn canvas(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let menu_app = cx.entity();
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
        let artboard_tool =
            self.panel == Panel::Canvas && self.view.turns == 0 && !self.view.mirror;
        let region = if self.image_crop.is_none() {
            self.generation_region
        } else {
            None
        };
        let crop = self
            .image_crop
            .as_ref()
            .map(|crop| (crop.image.transform, crop.draft, crop.width, crop.height));
        let marquee = self.selection_marquee;
        let transform_guides = self.transform_guides.clone();
        let gizmo = (self.image_crop.is_none() && self.moving_image && self.tool == Tool::Hand)
            .then(|| self.doc.selected_layer_bounds(&self.selected_layers))
            .flatten();
        let individual_bounds: Vec<_> = if gizmo.is_some() && self.selected_layers.len() > 1 {
            self.selected_layers
                .iter()
                .filter_map(|index| self.doc.layer_bounds_for(*index))
                .collect()
        } else {
            Vec::new()
        };

        div()
            .id("canvas-viewport")
            .relative()
            .border_1()
            .border_color(gpui_kit::transparent_black())
            .drag_over::<ExternalPaths>(|style, _, _, _| style.border_color(rgb(WINE)))
            .on_drop(cx.listener(|this, paths: &ExternalPaths, _, cx| {
                this.import_artwork_paths(paths.paths().to_vec(), cx);
            }))
            .flex_1()
            .min_h_0()
            .overflow_hidden()
            .cursor(
                if self.tool == Tool::Hand && self.moving_image && self.image_crop.is_none() {
                    CursorStyle::Arrow
                } else if self.tool == Tool::Hand
                    && !self.selecting_generation_region
                    && self.image_crop.is_none()
                {
                    CursorStyle::OpenHand
                } else {
                    CursorStyle::Crosshair
                },
            )
            .on_mouse_down(MouseButton::Left, cx.listener(Self::pointer_down))
            .on_mouse_down(MouseButton::Middle, cx.listener(Self::pointer_down))
            .on_mouse_down(MouseButton::Right, cx.listener(Self::select_context_target))
            .on_mouse_move(cx.listener(|this, event, _, cx| this.pointer_move(event, cx)))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _, window, cx| {
                    if let Some(crop) = &mut this.image_crop {
                        crop.anchor = None;
                    }
                    let resized_artboard = this.artboard_resize_drag.is_some();
                    let transformed_selection = this.last_image_point.is_some();
                    let painted = this.active_stroke;
                    this.finish_marquee(window, cx);
                    this.finish_stroke();
                    if resized_artboard {
                        this.sync_inputs(window, cx);
                    }
                    if transformed_selection || painted {
                        this.sync_layer_controls(window, cx);
                    }
                    this.finish_drawing_region(cx);
                    this.restore_prompt(window, cx);
                    cx.notify();
                }),
            )
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(|this, _, window, cx| {
                    if let Some(crop) = &mut this.image_crop {
                        crop.anchor = None;
                    }
                    let resized_artboard = this.artboard_resize_drag.is_some();
                    let transformed_selection = this.last_image_point.is_some();
                    let painted = this.active_stroke;
                    this.finish_marquee(window, cx);
                    this.finish_stroke();
                    if resized_artboard {
                        this.sync_inputs(window, cx);
                    }
                    if transformed_selection || painted {
                        this.sync_layer_controls(window, cx);
                    }
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
                        if artboard_tool {
                            for handle in [
                                (0., 0.),
                                (vw / 2., 0.),
                                (vw, 0.),
                                (0., vh / 2.),
                                (vw, vh / 2.),
                                (0., vh),
                                (vw / 2., vh),
                                (vw, vh),
                            ] {
                                let center = point(
                                    paper.origin.x + px(handle.0 * scale),
                                    paper.origin.y + px(handle.1 * scale),
                                );
                                let bounds = Bounds::new(
                                    point(center.x - px(5.), center.y - px(5.)),
                                    size(px(10.), px(10.)),
                                );
                                window.paint_quad(fill(bounds, rgb(0xffffff)));
                                let mut outline = PathBuilder::stroke(px(1.5));
                                outline.move_to(bounds.origin);
                                outline.line_to(point(
                                    bounds.origin.x + bounds.size.width,
                                    bounds.origin.y,
                                ));
                                outline.line_to(point(
                                    bounds.origin.x + bounds.size.width,
                                    bounds.origin.y + bounds.size.height,
                                ));
                                outline.line_to(point(
                                    bounds.origin.x,
                                    bounds.origin.y + bounds.size.height,
                                ));
                                outline.close();
                                if let Ok(outline) = outline.build() {
                                    window.paint_path(outline, rgb(WINE));
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
                        if !transform_guides.is_empty() {
                            let convert = |p: crate::document::Point| {
                                let p = view.to_view(p, w as f32, h as f32);
                                point(
                                    paper.origin.x + px(p.x * scale),
                                    paper.origin.y + px(p.y * scale),
                                )
                            };
                            let mut path = PathBuilder::stroke(px(1.));
                            for &(vertical, value) in &transform_guides {
                                if vertical {
                                    path.move_to(convert(crate::document::Point {
                                        x: value,
                                        y: 0.,
                                    }));
                                    path.line_to(convert(crate::document::Point {
                                        x: value,
                                        y: h as f32,
                                    }));
                                } else {
                                    path.move_to(convert(crate::document::Point {
                                        x: 0.,
                                        y: value,
                                    }));
                                    path.line_to(convert(crate::document::Point {
                                        x: w as f32,
                                        y: value,
                                    }));
                                }
                            }
                            if let Ok(path) = path.build() {
                                window.paint_path(path, rgb(WINE));
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
                        if let Some((a, b)) = marquee {
                            let convert = |p| {
                                let p = view.to_view(p, w as f32, h as f32);
                                point(
                                    paper.origin.x + px(p.x * scale),
                                    paper.origin.y + px(p.y * scale),
                                )
                            };
                            let mut path = PathBuilder::stroke(px(1.));
                            path.move_to(convert(crate::document::Point { x: a.x, y: a.y }));
                            path.line_to(convert(crate::document::Point { x: b.x, y: a.y }));
                            path.line_to(convert(crate::document::Point { x: b.x, y: b.y }));
                            path.line_to(convert(crate::document::Point { x: a.x, y: b.y }));
                            path.close();
                            if let Ok(path) = path.build() {
                                window.paint_path(path, rgb(WINE));
                            }
                        }
                        if let Some((
                            [a, b, c, d, x, y],
                            [left, top, width, height],
                            source_width,
                            source_height,
                        )) = crop
                        {
                            let convert = |sx: u32, sy: u32| {
                                let p = view.to_view(
                                    crate::document::Point {
                                        x: a * sx as f32 + c * sy as f32 + x,
                                        y: b * sx as f32 + d * sy as f32 + y,
                                    },
                                    w as f32,
                                    h as f32,
                                );
                                point(
                                    paper.origin.x + px(p.x * scale),
                                    paper.origin.y + px(p.y * scale),
                                )
                            };
                            let corners = [
                                convert(left, top),
                                convert(left + width, top),
                                convert(left + width, top + height),
                                convert(left, top + height),
                            ];
                            // Dim the source area outside the proposed crop while retaining
                            // the rest of the scene as context for placement.
                            for (x0, y0, x1, y1) in [
                                (0, 0, source_width, top),
                                (0, top + height, source_width, source_height),
                                (0, top, left, top + height),
                                (left + width, top, source_width, top + height),
                            ] {
                                if x0 >= x1 || y0 >= y1 {
                                    continue;
                                }
                                let mut shade = PathBuilder::fill();
                                shade.move_to(convert(x0, y0));
                                shade.line_to(convert(x1, y0));
                                shade.line_to(convert(x1, y1));
                                shade.line_to(convert(x0, y1));
                                shade.close();
                                if let Ok(shade) = shade.build() {
                                    window.paint_path(shade, rgba(0x1b172780));
                                }
                            }
                            let mut outline = PathBuilder::stroke(px(2.));
                            outline.move_to(corners[0]);
                            for corner in corners.iter().skip(1) {
                                outline.line_to(*corner);
                            }
                            outline.close();
                            if let Ok(outline) = outline.build() {
                                window.paint_path(outline, rgb(WINE));
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
                            for &(left, top, right, bottom) in &individual_bounds {
                                let mut outline = PathBuilder::stroke(px(1.));
                                outline.move_to(convert(left, top));
                                outline.line_to(convert(right, top));
                                outline.line_to(convert(right, bottom));
                                outline.line_to(convert(left, bottom));
                                outline.close();
                                if let Ok(outline) = outline.build() {
                                    window.paint_path(outline, rgba(0x790e2680));
                                }
                            }
                            path.move_to(convert(x, y));
                            path.line_to(convert(r, y));
                            path.line_to(convert(r, b));
                            path.line_to(convert(x, b));
                            path.close();
                            if let Ok(path) = path.build() {
                                window.paint_path(path, rgb(WINE));
                            }
                            let rotate_anchor = convert((x + r) / 2., y);
                            let rotate_handle = convert((x + r) / 2., y - 28. / scale.max(0.001));
                            let mut stem = PathBuilder::stroke(px(1.5));
                            stem.move_to(rotate_anchor);
                            stem.line_to(rotate_handle);
                            if let Ok(stem) = stem.build() {
                                window.paint_path(stem, rgb(WINE));
                            }
                            let mut ring = PathBuilder::stroke(px(1.5));
                            for i in 0..=24 {
                                let angle = i as f32 / 24. * std::f32::consts::TAU;
                                let point = point(
                                    rotate_handle.x + px(6. * angle.cos()),
                                    rotate_handle.y + px(6. * angle.sin()),
                                );
                                if i == 0 {
                                    ring.move_to(point);
                                } else {
                                    ring.line_to(point);
                                }
                            }
                            if let Ok(ring) = ring.build() {
                                window.paint_path(ring, rgb(WINE));
                            }
                            for (hx, hy) in [
                                (x, y),
                                ((x + r) / 2., y),
                                (r, y),
                                (x, (y + b) / 2.),
                                (r, (y + b) / 2.),
                                (x, b),
                                ((x + r) / 2., b),
                                (r, b),
                            ] {
                                let handle = convert(hx, hy);
                                window.paint_quad(fill(
                                    Bounds::new(
                                        point(handle.x - px(4.), handle.y - px(4.)),
                                        size(px(8.), px(8.)),
                                    ),
                                    rgb(WINE),
                                ));
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
            .when_some(self.image_crop.as_ref(), |canvas, crop| {
                canvas.child(
                    div()
                        .absolute()
                        .top(px(12.))
                        .left(px(12.))
                        .p_2()
                        .rounded_md()
                        .bg(rgb(0xffffff))
                        .border_1()
                        .border_color(rgb(BORDER))
                        .child(
                            row()
                                .flex_wrap()
                                .child(label(format!(
                                    "Drag the image area to keep · {} × {} px",
                                    crop.draft[2], crop.draft[3]
                                )))
                                .child(
                                    Button::new("crop-reset")
                                        .ghost()
                                        .small()
                                        .label("Reset")
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            this.reset_image_crop_draft(cx)
                                        })),
                                )
                                .child(
                                    Button::new("crop-cancel")
                                        .ghost()
                                        .small()
                                        .label("Cancel")
                                        .on_click(
                                            cx.listener(|this, _, _, cx| {
                                                this.cancel_image_crop(cx)
                                            }),
                                        ),
                                )
                                .child(
                                    Button::new("crop-apply")
                                        .primary()
                                        .small()
                                        .label("Apply")
                                        .on_click(cx.listener(|this, _, window, cx| {
                                            this.apply_image_crop(window, cx)
                                        })),
                                ),
                        )
                        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation()),
                )
            })
            .when(self.show_drawing_composer, |canvas| {
                canvas.child(self.drawing_composer(cx))
            })
            .context_menu(move |menu, _, cx| selection_context_menu(menu, menu_app.clone(), cx))
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
        let close = tool_button("close-inspector", "close", "Close panel").on_click(cx.listener(
            |this, _, window, cx| {
                this.dock = false;
                window.focus(&this.focus, cx);
                cx.notify();
            },
        ));
        if self.dock_view == DockView::Layers {
            column()
                .key_context("SidePanel")
                .w(px(360.))
                .flex_shrink_0()
                .h_full()
                .min_h_0()
                .gap_2()
                .bg(rgb(0xffffff))
                .border_l_1()
                .border_color(rgb(BORDER))
                .p_4()
                .occlude()
                .child(
                    row()
                        .flex_shrink_0()
                        .child(div().flex_1().text_lg().child("Layers"))
                        .child(close),
                )
                .child(self.layer_panel(cx).flex_1().min_h_0())
                .into_any_element()
        } else {
            side_panel_frame(
                "References",
                column().gap_4().child(self.reference_panel(cx)),
                close,
            )
            .into_any_element()
        }
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
        let unit_count = self.selection_units().len();
        let all_units_have_bounds = self
            .selection_units()
            .iter()
            .all(|unit| self.doc.selected_layer_bounds(unit).is_some());
        let mixed_opacity = self
            .selected_layers
            .first()
            .and_then(|index| self.doc.layers.get(*index))
            .is_some_and(|first| {
                self.selected_layers.iter().any(|index| {
                    self.doc
                        .layers
                        .get(*index)
                        .is_some_and(|layer| (layer.opacity - first.opacity).abs() > 0.0001)
                })
            });
        let move_range = self.selected_move_range();
        let can_ungroup = self.selected_group_name().is_some();
        let mut layers = column().gap_2().child(
            row()
                .flex_wrap()
                .child(
                    div()
                        .font_weight(FontWeight::MEDIUM)
                        .child(format!("Artwork · {}", self.doc.layers.len())),
                )
                .child(div().flex_1())
                .when(self.selected_layers.len() >= 2 && !can_ungroup, |header| {
                    header.child(
                        Button::new("group-layers")
                            .ghost()
                            .small()
                            .label("Group")
                            .disabled(self.selected_layers.len() < 2)
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.group_selected_layers();
                                this.sync_layer_controls(window, cx);
                                cx.notify();
                            })),
                    )
                })
                .when(can_ungroup, |header| {
                    header.child(
                        Button::new("ungroup-layers")
                            .ghost()
                            .small()
                            .label("Ungroup")
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.ungroup_selected_layers();
                                this.sync_layer_controls(window, cx);
                                cx.notify();
                            })),
                    )
                })
                .child(
                    tool_button("import-artwork", "image", "Import images as artwork layers")
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.import_artwork_dialog(window, cx)
                        })),
                )
                .child(
                    tool_button("add-layer", "plus", "Add layer").on_click(cx.listener(
                        |this, _, window, cx| {
                            this.add_layer();
                            this.sync_inputs(window, cx);
                            cx.notify();
                        },
                    )),
                ),
        );
        if let Some(group) = self.editing_group_name().map(str::to_owned) {
            let group_for_done = group.clone();
            layers = layers.child(
                row()
                    .child(label(format!("Editing {group}")))
                    .child(div().flex_1())
                    .child(
                        Button::new("exit-group-edit")
                            .ghost()
                            .small()
                            .label("Done")
                            .tooltip("Select the whole group · Esc")
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.select_layer_group(&group_for_done, false, window, cx);
                            })),
                    ),
            );
        }
        if self.selected_group_and_layer().is_some() {
            layers = layers.child(
                Button::new("add-selection-to-group")
                    .ghost()
                    .small()
                    .label("Add selected layer to group")
                    .tooltip("Add the adjacent layer without changing stacking")
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.add_selected_layer_to_group(window, cx)
                    })),
            );
        }
        if self.editing_group_name().is_some() && self.selected_layers.len() == 1 {
            layers = layers.child(
                Button::new("remove-layer-from-group")
                    .ghost()
                    .small()
                    .label("Remove layer from group")
                    .tooltip("Remove an edge layer without changing stacking")
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.remove_selected_layer_from_group(window, cx)
                    })),
            );
        }
        let mut list = column()
            .id("layer-list")
            .gap_1()
            .max_h(px(232.))
            .overflow_y_scroll();
        let mut seen_groups = std::collections::HashSet::new();
        for (i, layer) in self.doc.layers.iter().enumerate().rev() {
            if let Some(group) = &layer.group {
                let begins_segment = self
                    .doc
                    .layers
                    .get(i + 1)
                    .is_none_or(|next| next.group.as_ref() != Some(group));
                let key = (self.epoch, self.doc.canvas_id, group.clone());
                let collapsed = self.collapsed_layer_groups.contains(&key);
                if begins_segment {
                    let continued = !seen_groups.insert(group.clone());
                    let members = self
                        .doc
                        .layers
                        .iter()
                        .filter(|layer| layer.group.as_ref() == Some(group))
                        .count();
                    let all_selected = self
                        .doc
                        .layers
                        .iter()
                        .enumerate()
                        .filter(|(_, layer)| layer.group.as_ref() == Some(group))
                        .all(|(index, _)| self.selected_layers.contains(&index));
                    let selected_group = group.clone();
                    let context_group = group.clone();
                    let group_app = cx.entity();
                    let visibility_group = group.clone();
                    let lock_group = group.clone();
                    let properties = self.doc.layer_groups.get(group);
                    let group_visible = properties.is_none_or(|props| props.visible);
                    let group_locked = properties.is_some_and(|props| props.locked);
                    let collapse_key = key.clone();
                    let positions: Vec<_> = self
                        .doc
                        .layers
                        .iter()
                        .enumerate()
                        .filter_map(|(index, layer)| {
                            (layer.group.as_ref() == Some(group)).then_some(index)
                        })
                        .collect();
                    let group_range = positions
                        .first()
                        .zip(positions.last())
                        .filter(|(start, end)| **end - **start + 1 == positions.len())
                        .map(|(start, end)| (*start, *end + 1));
                    let (drag_start, drag_end) =
                        if all_selected && self.selected_layers.len() > positions.len() {
                            move_range.unwrap_or((0, 0))
                        } else {
                            group_range.unwrap_or((0, 0))
                        };
                    let drag = DraggedLayerRows {
                        epoch: self.epoch,
                        artboard: self.doc.canvas_id,
                        revision: self.revision,
                        start: drag_start,
                        end: drag_end,
                        label: group.clone(),
                    };
                    list = list.child(
                        row()
                            .id(format!("group-row-{i}"))
                            .gap_1()
                            .rounded_md()
                            .border_t_1()
                            .border_color(gpui_kit::transparent_black())
                            .drag_over::<DraggedLayerRows>(|style, _, _, _| {
                                style.border_color(rgb(WINE))
                            })
                            .on_drag(drag, |drag, _, _, cx| cx.new(|_| drag.clone()))
                            .on_drop(cx.listener(
                                move |this, drag: &DraggedLayerRows, window, cx| {
                                    this.drop_layer_rows(drag, i, window, cx);
                                },
                            ))
                            .when(all_selected, |r| r.bg(rgb(0xf1e6eb)))
                            .child(
                                tool_button(
                                    format!("group-expand-{i}"),
                                    if collapsed { "plus" } else { "minus" },
                                    if collapsed {
                                        "Expand group"
                                    } else {
                                        "Collapse group"
                                    },
                                )
                                .on_click(cx.listener(
                                    move |this, _, _, cx| {
                                        if !this.collapsed_layer_groups.remove(&collapse_key) {
                                            this.collapsed_layer_groups
                                                .insert(collapse_key.clone());
                                        }
                                        cx.notify();
                                    },
                                )),
                            )
                            .child(
                                div().flex_1().min_w_0().overflow_hidden().child(
                                    Button::new(format!("group-select-{i}"))
                                        .ghost()
                                        .small()
                                        .w_full()
                                        .justify_start()
                                        .icon(icon("layers"))
                                        .label(if continued {
                                            format!("{group} · continued")
                                        } else {
                                            group.clone()
                                        })
                                        .tooltip("Select group")
                                        .atelier_selected(all_selected, cx)
                                        .on_click(cx.listener(
                                            move |this, event: &ClickEvent, window, cx| {
                                                let modifiers = event.modifiers();
                                                this.select_layer_group(
                                                    &selected_group,
                                                    modifiers.shift
                                                        || modifiers.platform
                                                        || modifiers.control,
                                                    window,
                                                    cx,
                                                );
                                            },
                                        )),
                                ),
                            )
                            .child(label(format!("{members}")))
                            .child(
                                tool_button(
                                    format!("group-visible-{i}"),
                                    if group_visible { "eye" } else { "eye-off" },
                                    if group_visible {
                                        "Hide group"
                                    } else {
                                        "Show group"
                                    },
                                )
                                .on_click(cx.listener(
                                    move |this, _, _, cx| {
                                        this.toggle_group_visibility(&visibility_group, cx)
                                    },
                                )),
                            )
                            .child(
                                tool_button(
                                    format!("group-lock-{i}"),
                                    if group_locked { "lock" } else { "unlock" },
                                    if group_locked {
                                        "Unlock group"
                                    } else {
                                        "Lock group"
                                    },
                                )
                                .atelier_selected(group_locked, cx)
                                .on_click(cx.listener(
                                    move |this, _, _, cx| this.toggle_group_lock(&lock_group, cx),
                                )),
                            )
                            .context_menu(move |menu, window, cx| {
                                group_app.update(cx, |this, cx| {
                                    let all_selected = this
                                        .doc
                                        .layers
                                        .iter()
                                        .enumerate()
                                        .filter(|(_, layer)| {
                                            layer.group.as_deref() == Some(context_group.as_str())
                                        })
                                        .all(|(index, _)| this.selected_layers.contains(&index));
                                    if !all_selected {
                                        this.select_layer_group(&context_group, false, window, cx);
                                    }
                                });
                                selection_context_menu(menu, group_app.clone(), cx)
                            }),
                    );
                }
                if collapsed {
                    continue;
                }
            }
            let thumbnail = layer.image.as_ref().and_then(|image| {
                self.doc
                    .references
                    .iter()
                    .position(|reference| reference.image == image.reference.image)
                    .and_then(|index| self.references.get(index).cloned())
                    .or_else(|| {
                        self.library
                            .iter()
                            .find(|(reference, _)| reference.image == image.reference.image)
                            .map(|(_, preview)| preview.clone())
                    })
            });
            let is_selected = self.selected_layers.contains(&i);
            let is_paint_target = self.doc.paint_layer == Some(i);
            let preview = if let Some(thumbnail) = thumbnail {
                img(thumbnail)
                    .w(px(28.))
                    .h(px(28.))
                    .object_fit(ObjectFit::Contain)
                    .into_any_element()
            } else {
                div()
                    .w(px(28.))
                    .h(px(28.))
                    .rounded_md()
                    .bg(rgb(0xf7f3f5))
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(icon(if layer.image.is_some() {
                        "image"
                    } else {
                        "brush"
                    }))
                    .into_any_element()
            };
            let selection = Button::new(format!("layer-{i}"))
                .ghost()
                .small()
                .w_full()
                .justify_start()
                .accessibility_label(format!(
                    "{}{}{}",
                    layer.name,
                    if is_paint_target {
                        ", paint target"
                    } else {
                        ""
                    },
                    if layer.candidate {
                        ", pending review"
                    } else {
                        ""
                    }
                ))
                .tooltip(layer.name.clone())
                .atelier_selected(is_selected, cx)
                .child(div().w(px(8.)).child(if is_selected { "•" } else { "" }))
                .child(preview)
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .child(layer.name.clone()),
                )
                .when(is_paint_target, |button| button.child(label("Paint")))
                .when(layer.candidate, |button| {
                    button.child(label("Review").text_color(rgb(AMBER)))
                })
                .on_click(cx.listener(move |this, event: &ClickEvent, window, cx| {
                    let modifiers = event.modifiers();
                    if modifiers.shift {
                        this.select_layer_range(i, window, cx);
                    } else if modifiers.platform || modifiers.control {
                        this.toggle_layer_selection(i, window, cx);
                    } else {
                        this.select_layer(i, window, cx);
                    }
                }));
            let layer_row = row()
                .gap_1()
                .rounded_md()
                .when(layer.group.is_some(), |r| r.pl_3())
                .when(is_selected, |r| r.bg(rgb(0xf1e6eb)))
                .child(div().flex_1().min_w_0().child(selection))
                .child(
                    tool_button(
                        format!("layer-visible-{i}"),
                        if layer.visible { "eye" } else { "eye-off" },
                        if layer.visible {
                            "Hide layer"
                        } else {
                            "Show layer"
                        },
                    )
                    .accessibility_label(format!(
                        "{} {}",
                        if layer.visible { "Hide" } else { "Show" },
                        layer.name
                    ))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.checkpoint();
                        this.doc.layers[i].visible = !this.doc.layers[i].visible;
                        this.changed();
                        cx.notify();
                    })),
                )
                .child(
                    tool_button(
                        format!("layer-lock-{i}"),
                        if layer.locked { "lock" } else { "unlock" },
                        if layer.locked {
                            "Unlock layer"
                        } else {
                            "Lock layer"
                        },
                    )
                    .accessibility_label(format!(
                        "{} {}",
                        if layer.locked { "Unlock" } else { "Lock" },
                        layer.name
                    ))
                    .atelier_selected(layer.locked, cx)
                    .on_click(cx.listener(move |this, _, _, cx| this.toggle_layer_lock(i, cx))),
                );
            let row_app = cx.entity();
            let (drag_start, drag_end) = if self.selected_layers.contains(&i) {
                move_range.unwrap_or((0, 0))
            } else {
                (i, i + 1)
            };
            let drag = DraggedLayerRows {
                epoch: self.epoch,
                artboard: self.doc.canvas_id,
                revision: self.revision,
                start: drag_start,
                end: drag_end,
                label: layer.name.clone(),
            };
            list = list.child(
                layer_row
                    .id(format!("layer-row-{i}"))
                    .border_t_1()
                    .border_color(gpui_kit::transparent_black())
                    .drag_over::<DraggedLayerRows>(|style, _, _, _| style.border_color(rgb(WINE)))
                    .on_drag(drag, |drag, _, _, cx| cx.new(|_| drag.clone()))
                    .on_drop(
                        cx.listener(move |this, drag: &DraggedLayerRows, window, cx| {
                            this.drop_layer_rows(drag, i, window, cx);
                        }),
                    )
                    .context_menu(move |menu, window, cx| {
                        row_app.update(cx, |this, cx| {
                            if !this.selected_layers.contains(&i) {
                                this.select_layer(i, window, cx);
                            }
                        });
                        selection_context_menu(menu, row_app.clone(), cx)
                    }),
            );
        }
        layers = layers.child(list.flex_shrink_0());
        let mut inspector = column().gap_2();
        if self.selected_layers.is_empty() {
            inspector = inspector
                .child(separator())
                .child(label("Select a layer to view its properties."));
        }
        if selected.candidate && self.selected_layers.as_slice() == [self.doc.active_layer] {
            inspector = inspector.child(
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
        if !self.selected_layers.is_empty() {
            let group = self.selected_group_name();
            let includes_group = self.selection_units().iter().any(|unit| unit.len() > 1);
            let heading = if let Some(name) = group {
                name.to_owned()
            } else if unit_count == 1 {
                selected.name.clone()
            } else {
                format!("{unit_count} items selected")
            };
            let item_type = if group.is_some() {
                "Group"
            } else if unit_count > 1 {
                ""
            } else if selected.image.is_some() {
                "Image"
            } else {
                "Paint layer"
            };
            let mut properties = column().gap_2().pt_2()
                .child(separator())
                .child(row().child(div().flex_1().text_base().font_weight(FontWeight::SEMIBOLD).child(heading)).child(label(item_type)))
                .when(self.selection_is_locked(), |properties| properties.child(label("Locked · unlock selected items to edit")))
                .when(unit_count == 1 && group.is_none() && !selected.visible, |properties| properties.child(label("Hidden layer")))
                .when_some(group, |properties, _| properties
                    .child(field("Group name", &self.group_name))
                    .child(property_field("Group opacity (%)", &self.group_opacity, self.selection_is_locked()))
                    .when(!self.doc.group_is_contiguous(group.unwrap()), |p| p.child(label("Regroup adjacent layers to edit group opacity."))))
                .when(unit_count == 1 && group.is_none(), |properties| properties
                    .child(field("Layer name", &self.layer_name))
                    .child(if self.doc.paint_layer == Some(self.doc.active_layer) {
                        label("Paint target").into_any_element()
                    } else {
                        Button::new("set-layer-paint-target").ghost().small().label("Set as paint target")
                            .disabled(selected.candidate)
                            .tooltip("Choose where the brush and eraser paint")
                            .on_click(cx.listener(|this, _, _, cx| this.set_paint_layer(this.doc.active_layer, cx)))
                            .into_any_element()
                    }))
                .when(self.doc.selected_layer_bounds(&self.selected_layers).is_some(), |properties| {
                    properties
                        .child(label("Position and size"))
                        .child(row()
                            .child(property_field("X (px)", &self.layer_x, self.selection_is_locked()).flex_1())
                            .child(property_field("Y (px)", &self.layer_y, self.selection_is_locked()).flex_1()))
                        .child(row()
                            .child(property_field("Width (px)", &self.layer_width, self.selection_is_locked()).flex_1())
                            .child(property_field("Height (px)", &self.layer_height, self.selection_is_locked()).flex_1())
                            .child(Button::new("layer-aspect-lock").ghost().small()
                                .icon(icon(if self.layer_aspect_locked { "lock" } else { "unlock" }))
                                .tooltip("Lock width and height proportions")
                                .atelier_selected(self.layer_aspect_locked, cx)
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.layer_aspect_locked = !this.layer_aspect_locked;
                                    cx.notify();
                                }))))
                        .child(row()
                            .child(property_field("Rotate by (°)", &self.layer_rotation, self.selection_is_locked()).flex_1())
                            .child(Button::new("rotate-selection").ghost().small().label("Rotate")
                                .tooltip("Rotate selected artwork around its center")
                                .disabled(self.selection_is_locked())
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.rotate_selection_by_input(window, cx)
                                }))))
                })
                .when(self.doc.selected_layer_bounds(&self.selected_layers).is_none(), |properties| {
                    properties.child(label("Draw on this layer to enable position and size controls."))
                })
                .when(group.is_none() && !includes_group, |properties| {
                    properties.child(property_field("Layer opacity (%)", &self.layer_opacity, self.selection_is_locked()))
                })
                .when(group.is_none() && includes_group, |properties| properties.child(label("Group opacity remains separate from layer opacity.")))
                .when(
                    mixed_opacity && group.is_none() && !includes_group,
                    |properties| {
                        properties.child(label(
                            "Mixed layer opacity · enter a value to change all selected layers",
                        ))
                    },
                )
                .child(Button::new("toggle-arrange").ghost().small()
                    .label(if self.show_layer_arrange { "Hide arrange controls" } else { "Arrange…" })
                    .disabled(self.doc.selected_layer_bounds(&self.selected_layers).is_none())
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.show_layer_arrange = !this.show_layer_arrange;
                        cx.notify();
                    })))
                .when(self.show_layer_arrange && self.doc.selected_layer_bounds(&self.selected_layers).is_some(), |section| {
                    section
                .child(
                    row()
                        .child(label(if self.selected_layers.len() > 1 {
                            format!("{} layers selected", self.selected_layers.len())
                        } else {
                            "Transform".into()
                        }))
                        .child(div().flex_1())
                        .child(
                            tool_button(
                                "layer-snap",
                                "grid",
                                "Snap to artboard, layers, and 16 px grid",
                            )
                            .atelier_selected(self.snap_enabled, cx)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.set_snap_enabled(!this.snap_enabled);
                                cx.notify();
                            })),
                        ),
                )
                .child(label("Position selection as a whole on artboard"))
                .child({
                    let mut controls = row();
                    for (id, icon_name, tooltip, horizontal, mode) in [
                        (
                            "position-left",
                            "align-left",
                            "Move selection to artboard left",
                            true,
                            0,
                        ),
                        (
                            "position-center",
                            "align-center",
                            "Center selection horizontally on artboard",
                            true,
                            1,
                        ),
                        (
                            "position-right",
                            "align-right",
                            "Move selection to artboard right",
                            true,
                            2,
                        ),
                        (
                            "position-top",
                            "align-top",
                            "Move selection to artboard top",
                            false,
                            0,
                        ),
                        (
                            "position-middle",
                            "align-middle",
                            "Center selection vertically on artboard",
                            false,
                            1,
                        ),
                        (
                            "position-bottom",
                            "align-bottom",
                            "Move selection to artboard bottom",
                            false,
                            2,
                        ),
                    ] {
                        controls = controls.child(
                            tool_button(id, icon_name, tooltip)
                                .disabled(
                                    self.selected_layers.is_empty() || self.selection_is_locked(),
                                )
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    this.position_selection_on_artboard(
                                        horizontal, mode, window, cx,
                                    );
                                })),
                        );
                    }
                    controls
                })
                .when(unit_count > 1, |section| section.child(label("Align each item to artboard · items may overlap"))
                .child({
                    let mut controls = row();
                    for (id, icon_name, tooltip, horizontal, mode) in [
                        (
                            "each-left",
                            "align-left",
                            "Align each item to artboard left",
                            true,
                            0,
                        ),
                        (
                            "each-center",
                            "align-center",
                            "Center each item on artboard horizontally; items may overlap",
                            true,
                            1,
                        ),
                        (
                            "each-right",
                            "align-right",
                            "Align each item to artboard right",
                            true,
                            2,
                        ),
                        (
                            "each-top",
                            "align-top",
                            "Align each item to artboard top",
                            false,
                            0,
                        ),
                        (
                            "each-middle",
                            "align-middle",
                            "Center each item on artboard vertically; items may overlap",
                            false,
                            1,
                        ),
                        (
                            "each-bottom",
                            "align-bottom",
                            "Align each item to artboard bottom",
                            false,
                            2,
                        ),
                    ] {
                        controls = controls.child(
                            tool_button(id, icon_name, tooltip)
                                .disabled(
                                    self.selected_layers.is_empty() || self.selection_is_locked(),
                                )
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    if mode == 1 {
                                        this.align_each_confirmation = Some((horizontal, this.epoch, this.doc.canvas_id, this.revision, this.selected_layers.clone()));
                                        cx.notify();
                                    } else {
                                        this.align_each_to_artboard(horizontal, mode, window, cx);
                                    }
                                })),
                        );
                    }
                    controls
                })
                .when(self.align_each_confirmation.as_ref().is_some_and(|(_, epoch, canvas, revision, selection)| *epoch == self.epoch && *canvas == self.doc.canvas_id && *revision == self.revision && *selection == self.selected_layers), |section| section
                    .child(label("Centering each item may overlap them. Continue?"))
                    .child(row()
                        .child(Button::new("cancel-align-each").small().label("Cancel").on_click(cx.listener(|this, _, _, cx| { this.align_each_confirmation = None; cx.notify(); })))
                        .child(Button::new("confirm-align-each").small().label("Center each item").on_click(cx.listener(|this, _, window, cx| {
                            if let Some((horizontal, epoch, canvas, revision, selection)) = this.align_each_confirmation.take() {
                                if epoch == this.epoch && canvas == this.doc.canvas_id && revision == this.revision && selection == this.selected_layers {
                                    this.align_each_to_artboard(horizontal, 1, window, cx);
                                }
                            }
                            cx.notify();
                        }))))
                )
                )
                .when(unit_count > 1, |section| section
                .when(!all_units_have_bounds, |section| section.child(label("Each item needs artwork before alignment or spacing.")))
                .child(label("Align items to each other"))
                .child(
                    row()
                        .child(
                            tool_button("align-left", "align-left", "Align left edges")
                                .disabled(unit_count < 2 || self.selection_is_locked() || !all_units_have_bounds)
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.align_selection(true, 0, window, cx)
                                })),
                        )
                        .child(
                            tool_button(
                                "align-hcenter",
                                "align-center",
                                "Align horizontal centers",
                            )
                            .disabled(unit_count < 2 || self.selection_is_locked() || !all_units_have_bounds)
                            .on_click(cx.listener(
                                |this, _, window, cx| this.align_selection(true, 1, window, cx),
                            )),
                        )
                        .child(
                            tool_button("align-right", "align-right", "Align right edges")
                                .disabled(unit_count < 2 || self.selection_is_locked() || !all_units_have_bounds)
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.align_selection(true, 2, window, cx)
                                })),
                        )
                        .child(
                            tool_button("align-top", "align-top", "Align top edges")
                                .disabled(unit_count < 2 || self.selection_is_locked() || !all_units_have_bounds)
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.align_selection(false, 0, window, cx)
                                })),
                        )
                        .child(
                            tool_button("align-vcenter", "align-middle", "Align vertical centers")
                                .disabled(unit_count < 2 || self.selection_is_locked() || !all_units_have_bounds)
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.align_selection(false, 1, window, cx)
                                })),
                        )
                        .child(
                            tool_button("align-bottom", "align-bottom", "Align bottom edges")
                                .disabled(unit_count < 2 || self.selection_is_locked() || !all_units_have_bounds)
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.align_selection(false, 2, window, cx)
                                })),
                        )
                        .when(unit_count >= 3, |row| row.child(
                            tool_button("distribute-x", "distribute-x", "Distribute horizontally")
                                .disabled(unit_count < 3 || self.selection_is_locked() || !all_units_have_bounds)
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.distribute_selection(true, window, cx)
                                })),
                        ))
                        .when(unit_count >= 3, |row| row.child(
                            tool_button("distribute-y", "distribute-y", "Distribute vertically")
                                .disabled(unit_count < 3 || self.selection_is_locked() || !all_units_have_bounds)
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.distribute_selection(false, window, cx)
                                })),
                        )),
                )
                .child(
                    row()
                        .child(field("Gap (px)", &self.layer_gap).flex_1())
                        .child(
                            Button::new("gap-horizontal")
                                .ghost()
                                .small()
                                .label("Horizontal")
                                .tooltip(
                                    "Set exact horizontal gaps; keep the leftmost layer in place",
                                )
                                .disabled(unit_count < 2 || self.selection_is_locked() || !all_units_have_bounds)
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.set_selection_gap(true, window, cx)
                                })),
                        )
                        .child(
                            Button::new("gap-vertical")
                                .ghost()
                                .small()
                                .label("Vertical")
                                .tooltip("Set exact vertical gaps; keep the topmost layer in place")
                                .disabled(unit_count < 2 || self.selection_is_locked() || !all_units_have_bounds)
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.set_selection_gap(false, window, cx)
                                })),
                        ),
                )
                )
                })
                ;
            if let Some(reference) = (self.selected_layers.len() == 1
                && self.selected_group_name().is_none())
            .then(|| selected.image.as_ref().map(|image| image.reference.clone()))
            .flatten()
            {
                properties = properties.child(
                    Button::new("crop-layer-image")
                        .ghost()
                        .small()
                        .label("Crop image")
                        .disabled(self.doc.layer_is_locked(selected) || self.image_crop.is_some())
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.start_image_crop(cx);
                            window.focus(&this.focus, cx);
                        })),
                );
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
            inspector = inspector.child(properties);
        }
        layers.child(
            div()
                .id("layer-inspector-scroll")
                .flex_1()
                .min_h_0()
                .overflow_y_scroll()
                .child(inspector),
        )
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
        if matches!(self.page, Page::Projects | Page::Library) {
            return row()
                .h(px(28.))
                .px_4()
                .border_t_1()
                .border_color(rgb(BORDER))
                .text_xs()
                .text_color(rgb(MUTED))
                .child(if self.page == Page::Projects {
                    format!("{} projects", self.projects.len())
                } else {
                    format!("{} shared images", self.library.len())
                })
                .child(div().flex_1())
                .child("Stored locally");
        }
        if self.page == Page::Artboards {
            return div();
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
            .when(matches!(self.tool, Tool::Pencil | Tool::Eraser), |r| {
                let name = self
                    .doc
                    .paint_layer
                    .and_then(|index| self.doc.layers.get(index))
                    .map_or("Automatic", |layer| layer.name.as_str());
                r.child(
                    div()
                        .max_w(px(180.))
                        .overflow_hidden()
                        .truncate()
                        .child(format!("Paint: {name}")),
                )
            })
            .child(self.render_backend)
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .overflow_hidden()
                    .truncate()
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
                row()
                    .child(div().flex_1().child(Input::new(&self.search)))
                    .child(
                        Button::new("library-favorites")
                            .ghost()
                            .small()
                            .icon(icon("heart"))
                            .label("Favorites")
                            .atelier_selected(self.favorite_filter, cx)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.favorite_filter = !this.favorite_filter;
                                cx.notify();
                            })),
                    ),
            )
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
                    .w(px(if picker { 154. } else { 218. }))
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
            .when_some(
                self.activity.latest(&crate::activity::Scope::Library, None),
                |c, report| {
                    c.child(row().child(self.report_hint(
                        report.id,
                        "Library needs attention · View",
                        cx,
                    )))
                },
            )
    }

    fn reference_picker(&self, cx: &mut Context<Self>) -> AnyElement {
        let limit = crate::providers::reference_limit(&self.settings).map(|count| format!("{}: {count} image input(s) including the source · references are checked before queueing",self.settings.provider.name()))
            .unwrap_or_else(|error|format!("Choose an editing model: {error}"));
        let content = column().h_full()
            .child(label(limit))
            .child(label("Project references and the shared library · selecting a reference never adds artwork"))
            .child(Input::new(&self.search))
            .child(Button::new("picker-import").small().label("Import into library and select…")
                .on_click(cx.listener(|this,_,window,cx| this.pick(Picker::Import,window,cx))))
            .child(self.reference_cards(true,cx));
        self.side_panel("References", content, cx)
            .into_any_element()
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
        let new_layer = drawing && self.drawing_create_layer;
        let selection = drawing && !new_layer && self.region_bounds().is_some();
        let separate = selection && self.selection_mode == crate::workflow::SelectionMode::Separate;
        let has_image_options = !drawing
            || separate
            || new_layer
            || self.settings.provider.native_architecture().is_some();
        let composer_app = cx.entity();
        let show_details = self.show_prompt_details;
        let caption = if drawing {
            if new_layer {
                "Create an image; review it as a new layer on this artboard"
            } else if separate {
                "Create from the selected area; the result opens on a new artboard"
            } else if selection {
                "Edit the selected area; review the result as a layer"
            } else {
                "Edit the full artboard; review the result as a layer"
            }
        } else if mode == PromptComposerMode::Image {
            "Image variation · new artboard"
        } else {
            "New composition · new artboard"
        };

        let context = row()
            .child(label(caption).flex_1())
            .when_some(self.generation_report(), |row, report| {
                row.child(self.report_hint(report.id, "Generation failed · View", cx))
            })
            .when(drawing && !separate && !new_layer, |r| {
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
                    .flex_wrap()
                    .gap_1()
                    .child(
                        Button::new("drawing-full")
                            .ghost()
                            .small()
                            .label("Edit artboard")
                            .atelier_selected(
                                !new_layer && !selection && !self.selecting_generation_region,
                                cx,
                            )
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
                                "Edit selected area"
                            } else {
                                "Edit an area"
                            })
                            .atelier_selected(
                                !new_layer && (self.selecting_generation_region || selection),
                                cx,
                            )
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.toggle_box_select(cx);
                                this.restore_prompt(window, cx);
                            })),
                    )
                    .child(
                        Button::new("drawing-new-layer")
                            .ghost()
                            .small()
                            .label("Create image layer")
                            .atelier_selected(new_layer, cx)
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.use_full_canvas(cx);
                                this.drawing_create_layer = true;
                                this.restore_prompt(window, cx);
                                cx.notify();
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
                .when(!drawing || separate || new_layer, |r| {
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
                    tool_button("composer-options", "more", "More options").dropdown_menu(
                        move |menu, _, _| {
                            let image_app = composer_app.clone();
                            let provider_app = composer_app.clone();
                            menu.when(has_image_options, |menu| {
                                menu.item(
                                    PopupMenuItem::new("Image options")
                                        .checked(show_details)
                                        .on_click(move |_, _, cx| {
                                            image_app.update(cx, |this, cx| {
                                                this.show_prompt_details =
                                                    !this.show_prompt_details;
                                                cx.notify();
                                            });
                                        }),
                                )
                            })
                            .item(
                                PopupMenuItem::new("Provider settings").on_click(
                                    move |_, _, cx| {
                                        provider_app.update(cx, |this, cx| {
                                            this.panel = Panel::Settings;
                                            cx.notify();
                                        });
                                    },
                                ),
                            )
                        },
                    ),
                )
                .child(
                    Button::new("composer-submit")
                        .primary()
                        .small()
                        .label(if drawing && !separate && !new_layer {
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
                            i + if (drawing && !new_layer) || mode == PromptComposerMode::Image {
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
        if (self.show_prompt_details && has_image_options) || separate {
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
                );
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
        let validation = self.prompt_validation.as_ref().filter(|validation| {
            validation.epoch == self.epoch && validation.target == self.prompt_target
        });
        prompt_composer_frame(
            &self.prompt,
            controls,
            validation.map(|validation| validation.message.as_str()),
        )
    }
    /// One queue entry, oldest first, with an abort action for pending work.
    pub(crate) fn queue_row(&self, item: &crate::queue::QueueItem, cx: &mut Context<Self>) -> Div {
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
        if matches!(
            item.state,
            JobState::Queued | JobState::Running { .. } | JobState::Aborting
        ) {
            entry = entry.child(
                Button::new(format!("queue-abort-{id}"))
                    .ghost()
                    .small()
                    .label("Abort")
                    .disabled(item.state == JobState::Aborting)
                    .accessibility_label(format!("Abort request {id}"))
                    .tooltip(if matches!(item.state, JobState::Running { .. })
                        && item.request.settings.provider.native_architecture().is_some() {
                        "Stop waiting and discard the result; native GPU work may finish on the server"
                    } else {
                        "Abort this request"
                    })
                    .on_click(cx.listener(move |this, _, _, cx| this.queue_abort(id, cx))),
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
        div()
            .w_full()
            .py_3()
            .border_b_1()
            .border_color(rgb(BORDER))
            .child(entry)
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
    pub(crate) fn side_panel(
        &self,
        title: &'static str,
        content: Div,
        cx: &mut Context<Self>,
    ) -> Div {
        side_panel_frame(
            title,
            content,
            tool_button("close-side-panel", "close", "Close panel").on_click(cx.listener(
                |this, _, window, cx| {
                    this.panel = Panel::None;
                    window.focus(&this.focus, cx);
                    cx.notify();
                },
            )),
        )
    }

    fn settings_footer(&self, cx: &mut Context<Self>) -> Div {
        div()
            .flex_shrink_0()
            .border_t_1()
            .border_color(rgb(BORDER))
            .mt_4()
            .pt_4()
            .when_some(
                self.activity.for_task(
                    &crate::activity::Scope::App,
                    &crate::activity::Task::Settings,
                ),
                |p, report| {
                    p.child(row().mb_2().child(self.report_hint(
                        report.id,
                        "Settings not saved · View",
                        cx,
                    )))
                },
            )
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
            )
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
        self.side_panel("Brushes", content, cx).into_any_element()
    }
    fn artboard_settings(&self, cx: &mut Context<Self>) -> AnyElement {
        let content = column()
            .child(label(format!("Artboard {}", self.doc.canvas_id + 1)))
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
                Button::new("aspect-lock")
                    .ghost()
                    .small()
                    .icon(icon("lock"))
                    .label(if self.aspect_locked {
                        format!(
                            "Aspect locked · {:.3}",
                            self.doc.width as f32 / self.doc.height as f32
                        )
                    } else {
                        "Lock current aspect ratio".into()
                    })
                    .atelier_selected(self.aspect_locked, cx)
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.aspect_locked = !this.aspect_locked;
                        cx.notify();
                    })),
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
            .child(label(if self.scale_art {
                "Anchor is used when changing bounds; scaling keeps the top-left origin."
            } else {
                "Keep content anchored while changing canvas bounds"
            }))
            .child(column().gap_1().children((0..3).map(|y| {
                row().gap_1().children((0..3).map(|x| {
                    let anchor = (x as u8, y as u8);
                    Button::new(format!("resize-anchor-{x}-{y}"))
                        .ghost()
                        .small()
                        .w(px(44.))
                        .label(if (x, y) == (1, 1) { "•" } else { "○" })
                        .disabled(self.scale_art)
                        .accessibility_label(match (x, y) {
                            (0, 0) => "Anchor top left",
                            (1, 0) => "Anchor top center",
                            (2, 0) => "Anchor top right",
                            (0, 1) => "Anchor center left",
                            (1, 1) => "Anchor center",
                            (2, 1) => "Anchor center right",
                            (0, 2) => "Anchor bottom left",
                            (1, 2) => "Anchor bottom center",
                            _ => "Anchor bottom right",
                        })
                        .atelier_selected(self.resize_anchor == anchor, cx)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.resize_anchor = anchor;
                            cx.notify();
                        }))
                }))
            })))
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
                                this.sync_inputs(window, cx);
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
            .child(label("Export size follows the artboard dimensions."))
            .child(
                Button::new("fit-artwork")
                    .ghost()
                    .small()
                    .label("Fit artboard to artwork")
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.fit_artboard_to_artwork(window, cx);
                        cx.notify();
                    })),
            );
        self.side_panel("Artboard", content, cx).into_any_element()
    }

    fn export_settings(&self, cx: &mut Context<Self>) -> AnyElement {
        let content = column()
            .child(field("Project title", &self.title))
            .child(label(format!(
                "{} × {} px",
                self.doc.width, self.doc.height
            )))
            .child(separator())
            .child("File export")
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
                Button::new("save-project-title")
                    .ghost()
                    .small()
                    .label("Apply project title")
                    .on_click(cx.listener(|this, _, _, cx| {
                        let title = this.title.read(cx).value().trim().to_owned();
                        this.checkpoint();
                        this.doc.title = if title.is_empty() {
                            "Untitled project".into()
                        } else {
                            title
                        };
                        this.changed();
                        cx.notify();
                    })),
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
        self.side_panel("Export", content, cx).into_any_element()
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
            .child(self.provider_choices(true))
            .child(label(
                "Choose the service used for image creation and editing.",
            ));
        if provider.native_architecture().is_some() {
            content = content
                .child(label(if provider.supports_edit() {
                    "Generation and reference editing"
                } else {
                    "Generation only"
                }))
                .child(label(server_status_label(self.server_status.as_ref())))
                .child(label(
                    "Size and sampling are in Image options; LoRA adapters are set below.",
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
                .child(
                    Button::new("advanced-provider-models")
                        .ghost()
                        .small()
                        .label(if self.show_advanced_provider {
                            "Hide model details"
                        } else {
                            "Model details…"
                        })
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.show_advanced_provider = !this.show_advanced_provider;
                            cx.notify();
                        })),
                )
                .when(self.show_advanced_provider, |content| {
                    content
                        .child(field("Generation model", &self.model))
                        .child(field("Editing model", &self.edit_model))
                        .child(label(
                            "Leave editing model blank for models that only create images.",
                        ))
                });
            content = content.child(separator())
                .child(label("Connections and credentials are managed in Clipforge. Model choices are saved here."));
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
            .child(
                Button::new("settings-reload-providers")
                    .ghost()
                    .small()
                    .label("Reload providers")
                    .on_click(cx.listener(|this, _, window, cx| this.reload_clipforge(window, cx))),
            )
            .child(separator())
            .child(
                div()
                    .text_sm()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child("Generation"),
            )
            .child(number_field(
                "Request time limit (minutes)",
                &self.request_timeout,
            ))
            .child(label(
                "Applies to new requests. Time waiting in Atelier's queue does not count.",
            ))
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
        self.side_panel("Settings", content, cx)
            .child(self.settings_footer(cx))
            .into_any_element()
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
        self.side_panel("Practice", content, cx).into_any_element()
    }
}
