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
                tool_button(
                    "image-back",
                    "back",
                    if self.image_return_page == Page::Draw {
                        "Back to drawing"
                    } else {
                        "Back to library"
                    },
                )
                .on_click(cx.listener(|this, _, _, cx| this.back_from_image(cx))),
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
                    .accessibility_label("View at actual size")
                    .tooltip("View at actual size")
                    .disabled(self.comparing)
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.image_auto_fit = false;
                        this.image_zoom = 1.;
                        this.image_pan = crate::document::Point { x: 0., y: 0. };
                        cx.notify();
                    })),
            );
        if let Some((reference, _, width, height)) = &self.opened_image {
            toolbar = toolbar
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .child(reference.name.clone()),
                )
                .child(
                    tool_button("image-export-original", "download", "Export original").on_click(
                        cx.listener(|this, _, window, cx| this.pick(Picker::Original, window, cx)),
                    ),
                )
                .child(
                    tool_button("image-add-layer", "layers", "Add to canvas").on_click(
                        cx.listener(|this, _, window, cx| this.add_opened_image_layer(window, cx)),
                    ),
                )
                .child(
                    tool_button("image-approve", "package", "Export for Studio").on_click(
                        cx.listener(|this, _, window, cx| this.pick(Picker::Artwork, window, cx)),
                    ),
                )
                .child(
                    tool_button("image-add-library", "reference", "Add to reference library")
                        .disabled(self.image_is_shared())
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.promote_opened_image(cx);
                        })),
                )
                .child(label(format!("{width} × {height}")));
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
            workspace = workspace.child(
                row()
                    .px_4()
                    .py_1()
                    .flex_wrap()
                    .bg(rgb(0xffffff))
                    .child(
                        tool_button("mask-pan", "hand", "Pan image")
                            .atelier_selected(self.mask_tool == Tool::Hand, cx)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.finish_mask();
                                this.mask_tool = Tool::Hand;
                                cx.notify();
                            })),
                    )
                    .child(
                        tool_button("mask-paint", "brush", "Paint mask")
                            .atelier_selected(self.mask_tool == Tool::Pencil, cx)
                            .disabled(
                                self.comparing
                                    || self.generation.is_some()
                                    || self.opening_image.is_some(),
                            )
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.finish_mask();
                                this.mask_tool = Tool::Pencil;
                                cx.notify();
                            })),
                    )
                    .child(
                        tool_button("mask-erase", "eraser", "Erase mask")
                            .atelier_selected(self.mask_tool == Tool::Eraser, cx)
                            .disabled(
                                self.comparing
                                    || self.generation.is_some()
                                    || self.opening_image.is_some(),
                            )
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.finish_mask();
                                this.mask_tool = Tool::Eraser;
                                cx.notify();
                            })),
                    )
                    .child(
                        tool_button("mask-rect", "mask-rect", "Box selection")
                            .atelier_selected(self.mask_tool == Tool::Rect, cx)
                            .disabled(
                                self.comparing
                                    || self.generation.is_some()
                                    || self.opening_image.is_some(),
                            )
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.finish_mask();
                                this.mask_tool = Tool::Rect;
                                cx.notify();
                            })),
                    )
                    .when(
                        self.mask_tool != Tool::Hand
                            || self.mask.as_ref().is_some_and(|m| !m.strokes.is_empty())
                            || self.use_mask,
                        |tools| {
                            tools
                                .child(
                                    tool_button("mask-undo", "undo", "Undo mask stroke")
                                        .disabled(
                                            self.mask.as_ref().is_none_or(|m| m.strokes.is_empty()),
                                        )
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            this.undo_mask(false);
                                            cx.notify();
                                        })),
                                )
                                .child(
                                    tool_button("mask-redo", "redo", "Redo mask stroke")
                                        .disabled(self.mask_redo.is_empty())
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            this.undo_mask(true);
                                            cx.notify();
                                        })),
                                )
                                .children([16., 64., 160.].into_iter().enumerate().map(
                                    |(i, size)| {
                                        Button::new(format!("mask-size-{i}"))
                                            .ghost()
                                            .small()
                                            .label(format!("{size:.0} px"))
                                            .atelier_selected(self.mask_size == size, cx)
                                            .on_click(cx.listener(move |this, _, _, cx| {
                                                this.mask_size = size;
                                                cx.notify();
                                            }))
                                    },
                                ))
                                .child(
                                    tool_button("mask-use", "mask", "Use mask for next edit")
                                        .atelier_selected(self.use_mask, cx)
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            this.use_mask = !this.use_mask;
                                            cx.notify();
                                        })),
                                )
                        },
                    )
                    .child(div().flex_1())
                    .when(
                        self.opened_image
                            .as_ref()
                            .is_some_and(|(r, _, _, _)| r.parent.is_some()),
                        |p| {
                            p.child(
                                tool_button("image-compare", "compare", "Before / after")
                                    .atelier_selected(self.comparing, cx)
                                    .disabled(self.comparison_requested.is_some())
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.toggle_comparison();
                                        cx.notify();
                                    })),
                            )
                        },
                    ),
            );
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
                    .child(div().flex_1().child("Before"))
                    .child(div().flex_1().child("After")),
            );
        }
        workspace = workspace.child(stage);
        if let Some((reference, _, _, _)) = &self.opened_image {
            let id = crate::library::id(reference);
            let all = self.all_references();
            let family = crate::library::family(reference, &all);
            let mut versions = row()
                .id("image-versions")
                .h(px(44.))
                .px_4()
                .overflow_x_scroll()
                .bg(rgb(0xffffff));
            let mut relatives: Vec<_> = all
                .iter()
                .filter(|candidate| family.contains(&crate::library::id(candidate)))
                .cloned()
                .collect();
            relatives.sort_by_key(|candidate| {
                (candidate.parent.is_some(), crate::library::id(candidate))
            });
            for (i, candidate) in relatives.into_iter().enumerate() {
                let parent_name = candidate
                    .parent
                    .as_ref()
                    .and_then(|parent| all.iter().find(|r| crate::library::id(r) == *parent))
                    .map(|r| r.name.clone());
                let description = parent_name
                    .map(|parent| format!("{} · From {parent}", candidate.name))
                    .unwrap_or_else(|| candidate.name.clone());
                versions = versions.child(
                    Button::new(format!("version-{i}"))
                        .ghost()
                        .small()
                        .label(if candidate.parent.is_none() {
                            "Original".into()
                        } else {
                            candidate.name.clone()
                        })
                        .accessibility_label(description.clone())
                        .tooltip(description)
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
