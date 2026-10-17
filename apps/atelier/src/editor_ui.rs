use crate::{
    app::{DrawingApp, Page, Panel, Picker},
    document::Tool,
    ui::{
        column, icon, label, panel, row, tool_button, AtelierSelection, PromptComposerMode, BORDER,
        WINE,
    },
    worker,
};
use gpui_kit::{
    component::{button::*, input::Textarea, Disableable, Sizable},
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
                    match self.image_return_page {
                        Page::Draw => "Back to drawing",
                        Page::Artboards => "Back to artboards",
                        _ => "Back to library",
                    },
                )
                .on_click(cx.listener(|this, _, window, cx| this.back_from_image(window, cx))),
            )
            .child(
                tool_button("open-edit-composer", "sparkle", "Edit image · ⌘P")
                    .disabled(!self.settings.provider.supports_edit())
                    .atelier_selected(self.show_edit_composer, cx)
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.show_edit_composer = !this.show_edit_composer;
                        if this.show_edit_composer {
                            this.edit_opened_image(window, cx);
                        }
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
        if let Some((_, _, width, height)) = &self.opened_image {
            toolbar = toolbar
                .child(div().flex_1())
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
        if self.show_edit_composer && self.opened_image.is_some() {
            stage = stage.child(self.edit_composer(cx));
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
        workspace
    }

    fn edit_composer(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let prompt = self
            .opened_image
            .as_ref()
            .map(|(reference, _, _, _)| reference.prompt.trim())
            .filter(|prompt| !prompt.is_empty())
            .unwrap_or("Starting from this artboard");
        let summary = if prompt.chars().count() > 72 {
            format!("You · {}…", prompt.chars().take(72).collect::<String>())
        } else {
            format!("You · {prompt}")
        };
        div()
            .id("floating-edit-composer")
            .absolute()
            .bottom(px(18.))
            .left(px(0.))
            .right(px(0.))
            .flex()
            .justify_center()
            .occlude()
            .child(
                panel()
                    .w(px(500.))
                    .max_w_full()
                    .p_3()
                    .shadow_lg()
                    .child(
                        row()
                            .child(icon("pencil").text_color(rgb(WINE)))
                            .child(label(summary).flex_1().truncate())
                            .child(
                                tool_button("edit-settings", "settings", "Image settings")
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.panel = Panel::Settings;
                                        cx.notify();
                                    })),
                            )
                            .child(
                                tool_button("close-edit-composer", "close", "Close prompt")
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.show_edit_composer = false;
                                        cx.notify();
                                    })),
                            ),
                    )
                    .child(Textarea::new(&self.prompt).h(px(84.)))
                    .child(
                        row()
                            .child(label("Variation · original artboard preserved").flex_1())
                            .child(
                                Button::new("create-variation")
                                    .primary()
                                    .icon(icon("sparkle"))
                                    .label("Create variation")
                                    .disabled(self.generation.is_some())
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.enqueue_generation(window, cx);
                                    })),
                            ),
                    )
                    .when_some(self.generation_error.as_ref(), |card, error| {
                        card.child(label(error.clone()).text_color(rgb(WINE)))
                    }),
            )
    }

    pub(crate) fn artboards_workspace(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let mut all = self.doc.references.clone();
        if let Some(source_id) = &self.iteration_source_id {
            if !all
                .iter()
                .any(|reference| crate::library::id(reference) == *source_id)
            {
                if let Some(source) = self
                    .all_references()
                    .into_iter()
                    .find(|reference| crate::library::id(reference) == *source_id)
                {
                    all.push(source);
                }
            }
        }
        let mut references = all;
        let mut depths = std::collections::HashMap::new();
        for reference in &references {
            let mut depth = 0usize;
            let mut current = reference;
            let mut visited = std::collections::HashSet::new();
            while let Some(parent) = current.parent.as_deref() {
                if !visited.insert(crate::library::id(current)) {
                    break;
                }
                let Some(ancestor) = references
                    .iter()
                    .find(|candidate| crate::library::id(candidate) == parent)
                else {
                    break;
                };
                current = ancestor;
                depth += 1;
            }
            depths.insert(crate::library::id(reference), depth);
        }
        references.sort_by_key(|reference| {
            depths
                .get(&crate::library::id(reference))
                .copied()
                .unwrap_or_default()
        });
        let has_canvas_artwork = self
            .doc
            .layers
            .iter()
            .any(|layer| !layer.strokes.is_empty() || layer.image.is_some());
        let pending = self
            .iteration_job_id
            .and_then(|id| self.queue.get(id))
            .filter(|item| item.result_id.is_none());
        let empty_project = !has_canvas_artwork
            && self.doc.canvases.is_empty()
            && references.is_empty()
            && pending.is_none();
        let show_canvas = has_canvas_artwork || self.doc.canvas_created || empty_project;
        let artboard_count = references.len()
            + self.doc.canvases.len()
            + usize::from(show_canvas)
            + usize::from(pending.is_some());
        let focused_layout = artboard_count == 1;
        let show_add_slot =
            !empty_project && pending.is_none() && self.interaction_open_for.is_none();
        let mut content_width = 0.;
        let mut content_height: f32 = 0.;
        let mut content_items = 0;
        let mut cards = row().relative().items_center().gap_6().flex_shrink_0();

        let mut canvases = self
            .doc
            .canvases
            .iter()
            .map(|canvas| {
                (
                    canvas.id,
                    canvas.width,
                    canvas.height,
                    canvas.layers.as_slice(),
                )
            })
            .collect::<Vec<_>>();
        if show_canvas {
            canvases.push((
                self.doc.canvas_id,
                self.doc.width,
                self.doc.height,
                self.doc.layers.as_slice(),
            ));
        }
        canvases.sort_by_key(|(id, _, _, _)| *id);
        let (initial_canvases, later_canvases): (Vec<_>, Vec<_>) =
            canvases.into_iter().partition(|(id, _, _, _)| *id == 0);
        for (id, width, height, layers) in initial_canvases {
            let (card, card_width, card_height) =
                self.canvas_artboard_card(id, width, height, layers, focused_layout, cx);
            cards = cards.child(card);
            if content_items > 0 {
                content_width += 24.;
            }
            content_width += card_width;
            content_height = content_height.max(card_height + 32.);
            content_items += 1;
        }

        for (index, reference) in references.iter().enumerate() {
            let id = crate::library::id(reference);
            let Some(image) = self
                .reference_cache
                .iter()
                .find(|(cached, _)| crate::library::id(cached) == id)
                .map(|(_, image)| image.clone())
                .or_else(|| {
                    self.library
                        .iter()
                        .find(|(cached, _)| crate::library::id(cached) == id)
                        .map(|(_, image)| image.clone())
                })
            else {
                continue;
            };
            let open = reference.clone();
            let details = reference.clone();
            let menu_id = id.clone();
            let image_size = image.size(0);
            let image_width = image_size.width.0 as f32;
            let image_height = image_size.height.0 as f32;
            let max_width = if focused_layout { 520. } else { 310. };
            let max_height = if focused_layout { 420. } else { 360. };
            let scale = (max_width / image_width.max(1.)).min(max_height / image_height.max(1.));
            let card_width = image_width * scale;
            let card_height = image_height * scale;
            let card = column()
                .id(format!("iteration-artboard-{index}"))
                .relative()
                .w(px(card_width))
                .flex_shrink_0()
                .child(
                    div()
                        .id(format!("iteration-image-{index}"))
                        .h(px(card_height))
                        .w_full()
                        .bg(rgb(0xffffff))
                        .border_1()
                        .border_color(rgb(BORDER))
                        .when(self.interaction_open_for.as_deref() == Some(&id), |image| {
                            image.border_color(rgb(WINE))
                        })
                        .rounded_lg()
                        .shadow_lg()
                        .overflow_hidden()
                        .cursor_pointer()
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.open_image(open.clone(), cx);
                            cx.notify();
                        }))
                        .child(img(image).size_full().object_fit(ObjectFit::Contain)),
                )
                .child(
                    row()
                        .pt_3()
                        .items_center()
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .truncate()
                                .child(reference.name.clone()),
                        )
                        .child(
                            tool_button(
                                format!("iteration-menu-{index}"),
                                "more",
                                "Artboard activity",
                            )
                            .on_click(cx.listener(
                                move |this, _, _, cx| {
                                    this.interaction_open_for =
                                        (this.interaction_open_for.as_deref() != Some(&menu_id))
                                            .then(|| menu_id.clone());
                                    cx.notify();
                                },
                            )),
                        ),
                );
            cards = cards.child(card);
            if content_items > 0 {
                content_width += 24.;
            }
            content_width += card_width;
            content_height = content_height.max(card_height + 44.);
            content_items += 1;
            if self.interaction_open_for.as_deref() == Some(&id) {
                let has_prompt = !reference.prompt.trim().is_empty();
                let request = if !has_prompt {
                    "Original artboard".to_owned()
                } else {
                    let prompt = reference.prompt.trim();
                    let short = prompt.chars().take(42).collect::<String>();
                    format!(
                        "Prompt · {}{}",
                        short,
                        if prompt.chars().count() > 42 {
                            "…"
                        } else {
                            ""
                        }
                    )
                };
                cards = cards.child(
                    div()
                        .w(px(230.))
                        .flex_shrink_0()
                        .p_3()
                        .bg(rgb(0xffffff))
                        .border_1()
                        .border_color(rgb(BORDER))
                        .rounded_lg()
                        .shadow_lg()
                        .child(row().gap_2().child(icon("pencil")).child(label(request)))
                        .child(row().gap_2().child(icon("sparkle")).child(label(
                            if has_prompt && reference.parent.is_some() {
                                "Variation created"
                            } else if has_prompt {
                                "Image created"
                            } else {
                                "Original artboard"
                            },
                        )))
                        .child(
                            Button::new(format!("iteration-details-{index}"))
                                .ghost()
                                .small()
                                .icon(icon("info"))
                                .label("View full details")
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.open_image(details.clone(), cx);
                                    cx.notify();
                                })),
                        ),
                );
            }
        }

        for (id, width, height, layers) in later_canvases {
            let (card, card_width, card_height) =
                self.canvas_artboard_card(id, width, height, layers, focused_layout, cx);
            cards = cards.child(card);
            if content_items > 0 {
                content_width += 24.;
            }
            content_width += card_width;
            content_height = content_height.max(card_height + 32.);
            content_items += 1;
        }

        if let Some(item) = pending {
            let state = match &item.state {
                crate::queue::JobState::Running { progress } if !progress.is_empty() => {
                    progress.as_str()
                }
                crate::queue::JobState::Failed(error) => error.as_str(),
                state => state.label(),
            };
            cards = cards.child(
                panel()
                    .w(px(250.))
                    .h(px(318.))
                    .flex_shrink_0()
                    .flex()
                    .flex_col()
                    .items_center()
                    .justify_center()
                    .gap_3()
                    .child(icon("sparkle").text_color(rgb(WINE)))
                    .child(div().child("New artboard"))
                    .child(label(state.to_owned())),
            );
        }

        if show_add_slot {
            let position = (artboard_count <= 2)
                .then(|| (content_width + 16., ((content_height - 210.) / 2.).max(0.)));
            cards = cards.child(self.new_artboard_slot(cx, position));
        }

        let stage = row()
            .id("iteration-artboards")
            .flex_1()
            .min_h_0()
            .items_center()
            .gap_6()
            .px_6()
            .bg(rgb(0xf7f5f7))
            .overflow_x_scroll()
            .when(artboard_count <= 2, |stage| stage.justify_center())
            .child(cards);

        column()
            .flex_1()
            .min_w_0()
            .h_full()
            .gap_0()
            .bg(rgb(0xf7f5f7))
            .child(
                row()
                    .h(px(58.))
                    .px_6()
                    .gap_3()
                    .child(
                        div()
                            .text_lg()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("Artboards"),
                    )
                    .child(
                        label(format!("{artboard_count:02}"))
                            .px_2()
                            .py_1()
                            .bg(rgb(0xf1e6eb))
                            .text_color(rgb(WINE))
                            .rounded_md(),
                    )
                    .child(div().flex_1())
                    .child(
                        tool_button("toggle-artboards-prompt", "sparkle", "Toggle prompt · ⌘P")
                            .atelier_selected(self.show_artboards_composer, cx)
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.toggle_artboards_composer(window, cx);
                            })),
                    ),
            )
            .child(stage)
            .when(self.show_artboards_composer, |workspace| {
                workspace.child(self.artboards_composer(cx))
            })
    }

    fn canvas_artboard_card(
        &self,
        id: u32,
        width: u32,
        height: u32,
        layers: &[crate::document::Layer],
        focused: bool,
        cx: &mut Context<Self>,
    ) -> (AnyElement, f32, f32) {
        let has_artwork = layers
            .iter()
            .any(|layer| !layer.strokes.is_empty() || layer.image.is_some());
        let preview = has_artwork
            .then(|| {
                crate::document::raster_layers(
                    width,
                    height,
                    layers,
                    if focused { 600 } else { 400 },
                    false,
                )
            })
            .and_then(Result::ok)
            .map(worker::render_image);
        let max_width = if focused { 520. } else { 310. };
        let max_height = if focused { 420. } else { 360. };
        let scale = (max_width / width as f32).min(max_height / height as f32);
        let card_width = width as f32 * scale;
        let card_height = height as f32 * scale;
        let mut canvas = div()
            .id(format!("iteration-canvas-preview-{id}"))
            .w(px(card_width))
            .h(px(card_height))
            .bg(rgb(0xffffff))
            .border_1()
            .border_color(rgb(BORDER))
            .rounded_lg()
            .shadow_lg()
            .overflow_hidden()
            .cursor_pointer()
            .on_click(cx.listener(move |this, _, window, cx| {
                this.open_canvas_artboard(id, window, cx);
            }));
        if let Some(preview) = preview {
            canvas = canvas.child(img(preview).size_full().object_fit(ObjectFit::Contain));
        } else {
            canvas = canvas.child(
                column().size_full().items_center().justify_center().child(
                    div()
                        .w(px(48.))
                        .h(px(48.))
                        .rounded_full()
                        .bg(rgb(0xf7f1f3))
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(
                            tool_button(format!("blank-canvas-open-{id}"), "brush", "Open canvas")
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    this.open_canvas_artboard(id, window, cx);
                                    cx.stop_propagation();
                                })),
                        ),
                ),
            );
        }
        let name = if id == 0 {
            "Canvas".to_owned()
        } else {
            format!("Canvas {:02}", id + 1)
        };
        let card = column()
            .id(format!("iteration-canvas-{id}"))
            .w(px(card_width))
            .flex_shrink_0()
            .child(canvas)
            .child(
                row()
                    .w_full()
                    .pt_2()
                    .child(div().flex_1().font_weight(FontWeight::MEDIUM).child(name))
                    .child(
                        label(format!("{width} × {height}"))
                            .px_2()
                            .py_1()
                            .bg(rgb(0xf0eef2))
                            .rounded_md(),
                    ),
            );
        (card.into_any_element(), card_width, card_height)
    }

    fn new_artboard_slot(
        &self,
        cx: &mut Context<Self>,
        position: Option<(f32, f32)>,
    ) -> impl IntoElement {
        div()
            .id("new-artboard-slot")
            .w(px(140.))
            .h(px(210.))
            .flex_shrink_0()
            .when_some(position, |slot, (left, top)| {
                slot.absolute().left(px(left)).top(px(top))
            })
            .rounded_lg()
            .border_1()
            .border_color(rgb(0xe5dfe5))
            .bg(rgb(0xfaf9fa))
            .flex()
            .items_center()
            .justify_center()
            .cursor_pointer()
            .on_click(cx.listener(|this, _, window, cx| {
                this.create_canvas_artboard(window, cx);
            }))
            .child(
                Button::new("new-artboard-plus")
                    .ghost()
                    .small()
                    .icon(icon("plus").text_color(rgb(0xb8afb9)))
                    .accessibility_label("Create a blank canvas")
                    .tooltip("Create a blank canvas")
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.create_canvas_artboard(window, cx);
                        cx.stop_propagation();
                    })),
            )
    }

    fn artboards_composer(&self, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .w_full()
            .px_5()
            .pb_4()
            .flex()
            .justify_center()
            .child(self.prompt_composer(PromptComposerMode::Artboards, cx))
    }
}
