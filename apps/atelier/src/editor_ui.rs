use crate::{
    app::{DrawingApp, Page, Picker},
    document::Tool,
    ui::{column, label, row, tool_button, AtelierSelection, BORDER},
};
use gpui_kit::{
    component::{button::*, Disableable, Sizable},
    prelude::FluentBuilder,
    *,
};
impl DrawingApp {
    pub(crate) fn image_workspace(&self, cx: &mut Context<Self>) -> Div {
        let mut workspace = column().flex_1().min_w_0().h_full().gap_0();
        let mut toolbar = row()
            .flex_wrap()
            .py_2()
            .px_4()
            .bg(rgb(0xffffff))
            .border_b_1()
            .border_color(rgb(BORDER))
            .child(
                Button::new("image-back")
                    .ghost()
                    .small()
                    .label("Library")
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.page = Page::Library;
                        cx.notify();
                    })),
            )
            .child(
                tool_button("image-fit", "fit", "Fit image")
                    .disabled(self.comparing)
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.image_fit();
                        cx.notify();
                    })),
            )
            .child(
                Button::new("image-100")
                    .ghost()
                    .small()
                    .label("100%")
                    .disabled(self.comparing)
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.image_auto_fit = false;
                        this.image_zoom = 1.;
                        this.image_pan = crate::document::Point { x: 0., y: 0. };
                        cx.notify();
                    })),
            )
            .child(label(if self.comparing {
                "Before / after · fitted".into()
            } else {
                format!("{:.0}%", self.image_zoom * 100.)
            }));
        if let Some((reference, _, width, height)) = &self.opened_image {
            toolbar = toolbar
                .child(label(format!(
                    "{} · {} × {} px",
                    reference.name, width, height
                )))
                .child(div().flex_1())
                .child(
                    Button::new("image-export-original")
                        .small()
                        .label("Export original")
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.pick(Picker::Original, window, cx)
                        })),
                )
                .child(
                    Button::new("image-add-layer")
                        .small()
                        .label("Add to canvas")
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.add_opened_image_layer(window, cx)
                        })),
                )
                .child(Button::new("image-approve").small().label("Export for Studio")
                    .tooltip("Export an approved artwork package, then use Import Atelier artwork in Studio")
                    .on_click(cx.listener(|this, _, window, cx| this.pick(Picker::Artwork, window, cx))))
                .child(
                    Button::new("image-edit")
                        .primary()
                        .small()
                        .label("Edit image")
                        .disabled(
                            !self.settings.provider.supports_edit() || self.generation.is_some(),
                        )
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.edit_opened_image(window, cx);
                            cx.notify();
                        })),
                );
        }
        workspace = workspace.child(toolbar);
        if let Some(error) = &self.mask_save_error {
            workspace = workspace.child(
                row()
                    .px_4()
                    .py_2()
                    .flex_wrap()
                    .child(label(error.clone()))
                    .child(
                        Button::new("mask-retry-save")
                            .small()
                            .label("Retry mask save")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.save_mask();
                                cx.notify();
                            })),
                    ),
            );
        }
        if self.opened_image.is_some() {
            workspace = workspace.child(row().px_4().py_2().flex_wrap().bg(rgb(0xffffff))
                .child(Button::new("mask-pan").ghost().small().label("Pan").atelier_selected(self.mask_tool == Tool::Hand, cx)
                    .on_click(cx.listener(|this, _, _, cx| { this.finish_mask(); this.mask_tool = Tool::Hand; cx.notify(); })))
                .child(Button::new("mask-paint").ghost().small().label("Paint mask").atelier_selected(self.mask_tool == Tool::Pencil, cx).disabled(self.comparing || self.generation.is_some() || self.opening_image.is_some())
                    .on_click(cx.listener(|this, _, _, cx| { this.finish_mask(); this.mask_tool = Tool::Pencil; cx.notify(); })))
                .child(Button::new("mask-erase").ghost().small().label("Erase mask").atelier_selected(self.mask_tool == Tool::Eraser, cx).disabled(self.comparing || self.generation.is_some() || self.opening_image.is_some())
                    .on_click(cx.listener(|this, _, _, cx| { this.finish_mask(); this.mask_tool = Tool::Eraser; cx.notify(); })))
                .child(tool_button("mask-undo", "undo", "Undo mask stroke").disabled(self.mask.as_ref().is_none_or(|m| m.strokes.is_empty())).on_click(cx.listener(|this, _, _, cx| { this.undo_mask(false); cx.notify(); })))
                .child(tool_button("mask-redo", "redo", "Redo mask stroke").disabled(self.mask_redo.is_empty()).on_click(cx.listener(|this, _, _, cx| { this.undo_mask(true); cx.notify(); })))
                .children([16.,64.,160.].into_iter().enumerate().map(|(i,size)| Button::new(format!("mask-size-{i}")).ghost().small().label(format!("{size:.0} px")).atelier_selected(self.mask_size == size,cx)
                    .on_click(cx.listener(move |this, _, _, cx| { this.mask_size = size; cx.notify(); }))))
                .child(Button::new("mask-use").ghost().small().label("Use mask").atelier_selected(self.use_mask,cx)
                    .on_click(cx.listener(|this, _, _, cx| { this.use_mask = !this.use_mask; cx.notify(); })))
                .when(self.opened_image.as_ref().is_some_and(|(r,_,_,_)| r.parent.is_some()), |p| p.child(Button::new("image-compare").ghost().small().label(if self.comparison_requested.is_some() { "Opening comparison…" } else { "Before / after" }).atelier_selected(self.comparing,cx)
                    .disabled(self.comparison_requested.is_some()).on_click(cx.listener(|this, _, _, cx| { this.toggle_comparison(); cx.notify(); })))))
                .child(label(if !self.use_mask && self.mask.as_ref().is_some_and(|mask| !mask.strokes.is_empty()) {
                    "Saved mask draft is visible. Enable Use mask to apply it with FLUX Fill."
                } else {
                    "Paint the area to change. Masks are saved with the image. FLUX Fill supports masked edits."
                }));
        }
        let mut stage = div()
            .id("image-workspace")
            .flex_1()
            .min_h_0()
            .relative()
            .overflow_hidden()
            .bg(rgb(0xe9e6eb))
            .cursor(if self.mask_tool == Tool::Hand || self.comparing {
                CursorStyle::OpenHand
            } else {
                CursorStyle::Crosshair
            })
            .on_scroll_wheel(cx.listener(|this, event: &ScrollWheelEvent, _, cx| {
                if this.comparing {
                    return;
                }
                this.image_auto_fit = false;
                let delta = event.delta.pixel_delta(px(20.));
                this.image_zoom =
                    (this.image_zoom * (f32::from(delta.y) * 0.003).exp()).clamp(0.02, 8.);
                cx.notify();
            }))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, event: &MouseDownEvent, window, cx| {
                    window.focus(&this.focus, cx);
                    this.mask_down(event.position);
                    cx.notify();
                }),
            )
            .on_mouse_move(cx.listener(|this, event: &MouseMoveEvent, _, cx| {
                if this.mask_active {
                    if event.dragging() {
                        this.mask_move(event.position);
                    } else {
                        this.finish_mask();
                    }
                    cx.notify();
                    return;
                }
                if let Some(last) = this.image_drag {
                    if event.dragging() {
                        this.image_pan.x += f32::from(event.position.x - last.x);
                        this.image_pan.y += f32::from(event.position.y - last.y);
                        this.image_drag = Some(event.position);
                    } else {
                        this.image_drag = None;
                    }
                    cx.notify();
                }
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                    this.image_drag = None;
                    this.finish_mask();
                    cx.notify();
                }),
            )
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                    this.image_drag = None;
                    this.finish_mask();
                    cx.notify();
                }),
            );
        if self.opening_image.is_some() {
            stage = stage.child(
                div()
                    .size_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .child("Opening original…"),
            );
        } else if let Some((_, image, width, height)) = &self.opened_image {
            let image = image.clone();
            let width = *width as f32;
            let height = *height as f32;
            let zoom = self.image_zoom;
            let auto_fit = self.image_auto_fit;
            let pan = self.image_pan;
            let bounds_cell = self.image_bounds.clone();
            let mask = self.mask_preview.clone();
            let comparison = self.comparison.clone().filter(|_| self.comparing);
            stage = stage.child(
                canvas(
                    move |bounds, _, _| {
                        bounds_cell.set(bounds);
                    },
                    move |bounds, _, window, _| {
                        let zoom = if auto_fit {
                            ((f32::from(bounds.size.width) - 48.) / width)
                                .min((f32::from(bounds.size.height) - 48.) / height)
                                .clamp(0.02, 8.)
                        } else {
                            zoom
                        };
                        if let Some((_, before, before_width, before_height)) = &comparison {
                            for (side, image, iw, ih) in [
                                (0, before, *before_width as f32, *before_height as f32),
                                (1, &image, width, height),
                            ] {
                                let half = Bounds::new(
                                    point(
                                        bounds.origin.x + bounds.size.width * side as f32 / 2.,
                                        bounds.origin.y,
                                    ),
                                    size(bounds.size.width / 2., bounds.size.height),
                                );
                                let fit = ((f32::from(half.size.width) - 24.) / iw)
                                    .min((f32::from(half.size.height) - 24.) / ih)
                                    .max(0.02);
                                let rect = Bounds::new(
                                    point(
                                        half.origin.x + (half.size.width - px(iw * fit)) / 2.,
                                        half.origin.y + (half.size.height - px(ih * fit)) / 2.,
                                    ),
                                    size(px(iw * fit), px(ih * fit)),
                                );
                                let _ = window.paint_image(
                                    half,
                                    rect,
                                    Corners::all(px(0.)),
                                    image.clone(),
                                    0,
                                    false,
                                );
                            }
                            return;
                        }
                        let fitted = Bounds::new(
                            point(
                                bounds.origin.x
                                    + (bounds.size.width - px(width * zoom)) / 2.
                                    + px(pan.x),
                                bounds.origin.y
                                    + (bounds.size.height - px(height * zoom)) / 2.
                                    + px(pan.y),
                            ),
                            size(px(width * zoom), px(height * zoom)),
                        );
                        let _ = window.paint_image(
                            bounds,
                            fitted,
                            Corners::all(px(0.)),
                            image.clone(),
                            0,
                            false,
                        );
                        if let Some(mask) = &mask {
                            let _ = window.paint_image(
                                bounds,
                                fitted,
                                Corners::all(px(0.)),
                                mask.clone(),
                                0,
                                false,
                            );
                        }
                    },
                )
                .size_full(),
            );
        } else {
            stage = stage.child(
                div()
                    .size_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .child("Choose an image in the library to open it."),
            );
        }
        if self.comparing {
            workspace = workspace.child(
                row()
                    .px_4()
                    .py_2()
                    .child(div().flex_1().child("Before · source version"))
                    .child(div().flex_1().child("After · selected version")),
            );
        }
        workspace = workspace.child(stage);
        if let Some((reference, _, _, _)) = &self.opened_image {
            let id = crate::library::id(reference);
            let family = crate::library::family(
                reference,
                &self
                    .library
                    .iter()
                    .map(|(r, _)| r.clone())
                    .collect::<Vec<_>>(),
            );
            let mut versions = row()
                .id("image-versions")
                .h(px(86.))
                .px_4()
                .overflow_x_scroll()
                .bg(rgb(0xffffff))
                .child(label("Versions · click to inspect or branch"));
            let mut relatives: Vec<_> = self
                .library
                .iter()
                .enumerate()
                .filter(|(_, (r, _))| family.contains(&crate::library::id(r)))
                .collect();
            relatives.sort_by_key(|(_, (candidate, _))| {
                (candidate.parent.is_some(), crate::library::id(candidate))
            });
            for (version, (i, (candidate, _))) in relatives.into_iter().enumerate() {
                let candidate = candidate.clone();
                versions = versions.child(
                    Button::new(format!("version-{i}"))
                        .ghost()
                        .label(if candidate.parent.is_none() {
                            format!("Original · {}", candidate.name)
                        } else {
                            format!("Version {} · {}", version, candidate.name)
                        })
                        .tooltip(if candidate.prompt.is_empty() {
                            "Original image".into()
                        } else {
                            candidate.prompt.clone()
                        })
                        .atelier_selected(crate::library::id(&candidate) == id, cx)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.open_image(candidate.clone(), cx)
                        })),
                );
            }
            workspace = workspace.child(versions);
        }
        row()
            .items_start()
            .gap_0()
            .flex_1()
            .min_w_0()
            .h_full()
            .child(workspace)
            .child(self.generate_panel(cx))
    }
}
