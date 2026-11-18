//! Board: the story's assets as a grid of cards, with the selected one's
//! picture, description, variations and story uses beside it. Art is made in
//! Atelier; the Board names assets, describes them and hands them over.
use std::path::PathBuf;

use design_system::{
    ButtonKind, Dialog, Glyph, SplitState, SplitView, Tabs, TabsStyle, TextArea, TextField,
    TextFieldEvent, Theme, Tile, TileGrid, TileGridState, TreeSelection, TypeScale, Typography,
};
use gpui_kit::{prelude::*, *};
use storyfella_studio_core::{
    catalog::{CatalogEdit, KINDS},
    errors::ErrorCode,
    registry::ArtifactKind,
};
use uuid::Uuid;

use super::Studio;
use crate::{
    board::{self, Card, Filter, Reading},
    commands,
    host::Place,
};

/// What the asset dialog is for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Compose {
    New,
    Variation(Uuid),
    Edit(Uuid),
}

pub(super) struct Board {
    filter: Filter,
    cards: Vec<Card>,
    reading: Option<Reading>,
    /// The story or catalog changed since the cards were built.
    stale: bool,
    selection: TreeSelection,
    grid: TileGridState,
    grid_focus: FocusHandle,
    /// The asset whose details show: a card, or a variation picked there.
    shown: Option<Uuid>,
    details_split: SplitState,
    pub(super) compose: Option<Compose>,
    /// The kind a new asset will be, as an index into [`KINDS`].
    kind: usize,
    name: Entity<TextField>,
    description: Entity<TextArea>,
    opening: bool,
}

impl Board {
    pub(super) fn new(theme: Theme, window: &mut Window, cx: &mut Context<Studio>) -> Self {
        let name = cx.new(|cx| TextField::new("Name, as the story writes it", theme, window, cx));
        cx.subscribe_in(&name, window, |studio, _, event, window, cx| {
            if let TextFieldEvent::Submitted(_) = event {
                studio.finish_compose(window, cx);
            }
        })
        .detach();
        let description = cx.new(|cx| {
            TextArea::new(
                "What it looks like, for whoever draws it",
                theme,
                window,
                cx,
            )
            .rows(4)
        });
        Self {
            filter: Filter::All,
            cards: Vec::new(),
            reading: None,
            stale: true,
            selection: TreeSelection::default(),
            grid: TileGridState::new(),
            grid_focus: cx.focus_handle(),
            shown: None,
            details_split: SplitState::new(px(300.)).limits(px(240.), px(520.)),
            compose: None,
            kind: 0,
            name,
            description,
            opening: false,
        }
    }

    pub(super) fn story_changed(&mut self) {
        self.stale = true;
    }

    pub(super) fn set_theme(&mut self, theme: Theme, cx: &mut Context<Studio>) {
        self.name.update(cx, |field, cx| field.set_theme(theme, cx));
        self.description
            .update(cx, |area, cx| area.set_theme(theme, cx));
    }

    #[cfg(test)]
    pub(super) fn cards_for_tests(&self) -> &[Card] {
        &self.cards
    }

    #[cfg(test)]
    pub(super) fn set_name_for_tests(
        &self,
        name: &str,
        window: &mut Window,
        cx: &mut Context<Studio>,
    ) {
        self.name
            .update(cx, |field, cx| field.set_text(name.to_owned(), window, cx));
    }

    #[cfg(test)]
    pub(super) fn shown_for_tests(&self) -> Option<Uuid> {
        self.shown
    }

    /// Rebuilds the cards from the story when it changed, keeping the
    /// selected asset selected wherever it moved.
    fn sync(&mut self, state: &crate::state::AppState) {
        if !self.stale {
            return;
        }
        self.stale = false;
        let selected = self
            .selection
            .primary()
            .and_then(|index| self.cards.get(index))
            .map(|card| card.id);
        let reading = Reading::of(state);
        self.cards = board::cards(state, &reading, self.filter);
        self.reading = Some(reading);
        self.selection = match selected
            .and_then(|id| self.cards.iter().position(|card| card.id == id))
            .or((!self.cards.is_empty()).then_some(0))
        {
            Some(index) => TreeSelection::single(index),
            None => TreeSelection::default(),
        };
        if self
            .shown
            .is_none_or(|id| state.registry.artifact(id).is_none())
        {
            self.shown = self
                .selection
                .primary()
                .and_then(|index| self.cards.get(index))
                .map(|card| card.id);
        }
    }
}

impl Studio {
    /// Shows the Board with whatever Atelier has saved since it last looked.
    pub(super) fn open_board(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.place = Place::Board;
        if let Err(error) = commands::refresh_catalog(&mut self.state) {
            self.report(error, cx);
        }
        self.board.story_changed();
        window.focus(&self.board.grid_focus, cx);
    }

    fn set_filter(&mut self, filter: Filter, cx: &mut Context<Self>) {
        self.board.filter = filter;
        self.board.selection = TreeSelection::default();
        self.board.shown = None;
        self.board.story_changed();
        cx.notify();
    }

    fn select_card(&mut self, index: usize, modifiers: Modifiers, cx: &mut Context<Self>) {
        let visible: Vec<usize> = (0..self.board.cards.len()).collect();
        self.board.selection.click(index, modifiers, &visible);
        self.board.shown = self
            .board
            .selection
            .primary()
            .and_then(|index| self.board.cards.get(index))
            .map(|card| card.id);
        cx.notify();
    }

    pub(super) fn start_compose(
        &mut self,
        compose: Compose,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let (name, description) = match compose {
            Compose::Edit(id) => self
                .state
                .registry
                .artifact(id)
                .map(|artifact| (artifact.name.clone(), artifact.description.clone()))
                .unwrap_or_default(),
            Compose::Variation(id) => (
                self.state
                    .registry
                    .artifact(id)
                    .map(|artifact| format!("{}_", artifact.name))
                    .unwrap_or_default(),
                String::new(),
            ),
            Compose::New => {
                if let Filter::Kind(kind) = self.board.filter {
                    self.board.kind = KINDS.iter().position(|k| *k == kind).unwrap_or(0);
                }
                (String::new(), String::new())
            }
        };
        self.board.compose = Some(compose);
        self.board.name.update(cx, |field, cx| {
            field.set_text(name, window, cx);
            field.set_error(None, cx);
            field.focus(window, cx);
        });
        self.board
            .description
            .update(cx, |area, cx| area.set_text(description, window, cx));
        cx.notify();
    }

    fn finish_compose(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(compose) = self.board.compose else {
            return;
        };
        let name = self.board.name.read(cx).text(cx).trim().to_owned();
        let description = self.board.description.read(cx).text(cx).to_string();
        let edit = match compose {
            Compose::New => CatalogEdit::Create {
                kind: KINDS[self.board.kind],
                name,
                description,
                variant_of: None,
            },
            Compose::Variation(base) => CatalogEdit::Create {
                kind: self
                    .state
                    .registry
                    .artifact(base)
                    .map_or(ArtifactKind::Character, |artifact| artifact.kind),
                name,
                description,
                variant_of: Some(base),
            },
            Compose::Edit(id) => CatalogEdit::Describe {
                id,
                name,
                description,
            },
        };
        match self.state.edit_catalog(edit) {
            Ok(id) => {
                self.board.compose = None;
                self.board.shown = Some(id);
                self.board.story_changed();
                self.changed(cx);
                window.focus(&self.board.grid_focus, cx);
            }
            Err(message) => self
                .board
                .name
                .update(cx, |field, cx| field.set_error(Some(message.into()), cx)),
        }
    }

    #[cfg(test)]
    pub(super) fn board_finish_for_tests(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.finish_compose(window, cx);
    }

    fn cancel_compose(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.board.compose = None;
        window.focus(&self.board.grid_focus, cx);
        cx.notify();
    }

    /// Saves the story and hands the asset to Atelier.
    fn open_in_atelier(&mut self, id: Uuid, cx: &mut Context<Self>) {
        if let Err(message) = self.state.apply_description_drafts() {
            self.fail(ErrorCode::StoreSave, message, cx);
            return;
        }
        if let Err(error) = commands::save_project(&mut self.state) {
            self.report(error, cx);
            return;
        }
        let Some(project) = self.state.project_path.clone() else {
            return;
        };
        let source = self.state.story.source().to_owned();
        self.board.opening = true;
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let opened = cx
                .background_spawn(async move { board::open_in_atelier(&project, &source, id) })
                .await;
            this.update(cx, |studio, cx| {
                studio.board.opening = false;
                match opened {
                    Ok(()) => studio.hint("Opened in Atelier. Its saved art shows here when you come back to the Board.", cx),
                    Err(message) => studio.fail(ErrorCode::AppCommandInvalid, message, cx),
                }
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    pub(super) fn render_board(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        self.board.sync(&self.state);
        SplitView::new(
            "board-details",
            self.theme,
            &self.board.details_split,
            div()
                .size_full()
                .flex()
                .flex_col()
                .child(self.render_board_bar(cx))
                .child(div().flex_1().min_h_0().child(self.render_grid(cx))),
            self.render_asset(cx),
        )
        .rest(px(360.))
    }

    fn render_board_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let c = self.theme;
        let selected = Filter::ALL
            .iter()
            .position(|filter| *filter == self.board.filter)
            .unwrap_or(0);
        let needing = board::needing_art(&self.state);
        let labels = Filter::ALL.map(|filter| match filter {
            Filter::NeedsArt if needing > 0 => format!("Needs art {needing}"),
            filter => filter.label().to_owned(),
        });
        let studio = cx.entity().downgrade();
        div()
            .h(px(44.))
            .flex_none()
            .flex()
            .items_center()
            .gap(c.metrics.space * 2.)
            .px(px(12.))
            .border_b_1()
            .border_color(c.hairline)
            .child(
                Tabs::new("board-filter", labels, selected, c)
                    .style(TabsStyle::Pill)
                    .on_select(move |index, _, cx| {
                        studio
                            .update(cx, |studio, cx| studio.set_filter(Filter::ALL[index], cx))
                            .ok();
                    }),
            )
            .child(div().flex_1())
            .child(
                c.button("new-asset", "New asset", ButtonKind::Secondary)
                    .disabled(self.state.operation_in_progress())
                    .on_click(cx.listener(|studio, _, window, cx| {
                        studio.start_compose(Compose::New, window, cx)
                    })),
            )
    }

    fn render_grid(&self, cx: &mut Context<Self>) -> AnyElement {
        let c = self.theme;
        if self.board.cards.is_empty() {
            let (title, body) = match self.board.filter {
                Filter::NeedsArt => ("Nothing needs art", "Every asset has a finished image."),
                Filter::All => (
                    "No assets yet",
                    "Name a character, place or prop in the story, or add one here.",
                ),
                Filter::Kind(kind) => (board::plural(kind), "None of these yet."),
            };
            return div()
                .size_full()
                .flex()
                .flex_col()
                .items_center()
                .justify_center()
                .gap(px(6.))
                .text_color(c.text_muted)
                .child(
                    div()
                        .type_scale(TypeScale::Title)
                        .text_color(c.text)
                        .child(title),
                )
                .child(body)
                .into_any_element();
        }
        let cards = self.board.cards.clone();
        let select = cx.entity().downgrade();
        let open = select.clone();
        let ids: Vec<Uuid> = cards.iter().map(|card| card.id).collect();
        div()
            .size_full()
            .p(px(12.))
            .child(
                TileGrid::new(
                    "board-assets",
                    "Assets",
                    c,
                    &self.board.grid,
                    cards.len(),
                    move |index, _, _| tile(&cards[index]),
                )
                .focus(&self.board.grid_focus)
                .selection(&self.board.selection)
                .on_select(move |index, modifiers, _, cx| {
                    select
                        .update(cx, |studio, cx| studio.select_card(index, modifiers, cx))
                        .ok();
                })
                .on_open(move |index, window, cx| {
                    let id = ids[index];
                    open.update(cx, |studio, cx| {
                        studio.start_compose(Compose::Edit(id), window, cx)
                    })
                    .ok();
                }),
            )
            .into_any_element()
    }

    /// The shown asset: its picture, what it is, its variations and where
    /// the story uses it.
    fn render_asset(&self, cx: &mut Context<Self>) -> AnyElement {
        let c = self.theme;
        let artifact = self
            .board
            .shown
            .and_then(|id| self.state.registry.artifact(id));
        let Some(artifact) = artifact else {
            return div()
                .size_full()
                .bg(c.surface)
                .flex()
                .items_center()
                .justify_center()
                .text_color(c.text_muted)
                .child("Select an asset to see it here.")
                .into_any_element();
        };
        let id = artifact.id;
        let image = crate::state::AppState::display_image(artifact).map(PathBuf::from);
        let family = board::family(&self.state.registry, id);
        let uses = self
            .board
            .reading
            .as_ref()
            .map(|reading| reading.uses(&self.state.registry, id))
            .unwrap_or_default();
        let base = artifact.variant_of.unwrap_or(id);
        let shared = board::shared_with_atelier(&self.state);
        let busy = self.state.operation_in_progress();
        let label = |text: &str| {
            div()
                .pt(px(8.))
                .type_scale(TypeScale::Caption)
                .text_color(c.text_muted)
                .child(text.to_owned())
        };
        let picture = div()
            .w_full()
            .h(px(220.))
            .flex_none()
            .rounded(c.radius_control)
            .bg(c.surface_deep)
            .flex()
            .items_center()
            .justify_center()
            .overflow_hidden();
        let picture = match image {
            Some(path) => picture.child(img(path).size_full().object_fit(ObjectFit::Contain)),
            None => picture
                .flex_col()
                .gap(px(6.))
                .text_color(c.text_muted)
                .child(Glyph::Image.render(c.text_muted, px(24.)))
                .child("No image yet"),
        };
        div()
            .id("board-asset")
            .size_full()
            .overflow_y_scroll()
            .bg(c.surface)
            .p(px(12.))
            .flex()
            .flex_col()
            .gap(px(6.))
            .child(picture)
            .child(
                div()
                    .pt(px(6.))
                    .type_scale(TypeScale::Title)
                    .child(artifact.name.clone()),
            )
            .child(
                div()
                    .type_scale(TypeScale::Small)
                    .text_color(c.text_muted)
                    .child(board::singular(artifact.kind)),
            )
            .child(div().text_color(c.text_muted).child(
                if artifact.description.trim().is_empty() {
                    "No description yet.".to_owned()
                } else {
                    artifact.description.clone()
                },
            ))
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap(c.metrics.space)
                    .child(
                        c.button("edit-asset", "Edit…", ButtonKind::Secondary)
                            .disabled(busy)
                            .on_click(cx.listener(move |studio, _, window, cx| {
                                studio.start_compose(Compose::Edit(id), window, cx)
                            })),
                    )
                    .when(artifact.kind.supports_variants(), |row| {
                        row.child(
                            c.button("new-variation", "New variation", ButtonKind::Secondary)
                                .disabled(busy)
                                .on_click(cx.listener(move |studio, _, window, cx| {
                                    studio.start_compose(Compose::Variation(base), window, cx)
                                })),
                        )
                    }),
            )
            .when(family.len() > 1, |column| {
                column
                    .child(label("Variations"))
                    .children(family.into_iter().map(|(member, name)| {
                        let kind = if member == id {
                            ButtonKind::Primary
                        } else {
                            ButtonKind::Quiet
                        };
                        c.button(
                            SharedString::from(format!("variation-{member}")),
                            name,
                            kind,
                        )
                        .on_click(cx.listener(move |studio, _, _, cx| {
                            studio.board.shown = Some(member);
                            cx.notify();
                        }))
                    }))
            })
            .child(label("Used in"))
            .when(uses.is_empty(), |column| {
                column.child(
                    div()
                        .type_scale(TypeScale::Small)
                        .text_color(c.text_muted)
                        .child("The story does not name it yet."),
                )
            })
            .children(uses.into_iter().enumerate().map(|(index, at)| {
                c.button(
                    SharedString::from(format!("use-{index}")),
                    format!("{}, line {}", at.file, at.line),
                    ButtonKind::Quiet,
                )
                .on_click(cx.listener(move |studio, _, window, cx| {
                    studio.open_file(&at.file, Some(at.offset), cx);
                    studio.show(Place::Write, window, cx);
                }))
            }))
            .child(div().flex_1().min_h(px(12.)))
            .child(
                c.button(
                    "open-in-atelier",
                    if self.board.opening {
                        "Opening Atelier…"
                    } else {
                        "Open in Atelier"
                    },
                    ButtonKind::Primary,
                )
                .disabled(!shared || busy || self.board.opening)
                .on_click(cx.listener(move |studio, _, _, cx| studio.open_in_atelier(id, cx))),
            )
            .when(!shared, |column| {
                column.child(
                    div()
                        .type_scale(TypeScale::Small)
                        .text_color(c.text_muted)
                        .child("Atelier opens stories kept in a project folder."),
                )
            })
            .into_any_element()
    }

    /// The new-asset, new-variation and edit dialog, over the whole window.
    pub(super) fn render_compose(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let c = self.theme;
        let compose = self.board.compose;
        let title = match compose {
            Some(Compose::Variation(_)) => "New variation",
            Some(Compose::Edit(_)) => "Edit asset",
            _ => "New asset",
        };
        let studio = cx.entity().downgrade();
        let (cancel, dismiss, pick) = (studio.clone(), studio.clone(), studio.clone());
        let mut dialog = Dialog::new("compose-asset", title, c)
            .open(compose.is_some())
            .width(px(420.));
        if compose == Some(Compose::New) {
            dialog = dialog.child(
                Tabs::new("asset-kind", KINDS.map(board::singular), self.board.kind, c)
                    .style(TabsStyle::Pill)
                    .on_select(move |index, _, cx| {
                        pick.update(cx, |studio, cx| {
                            studio.board.kind = index;
                            cx.notify();
                        })
                        .ok();
                    }),
            );
        }
        dialog
            .child(self.board.name.clone())
            .child(self.board.description.clone())
            .action(
                c.button("cancel-compose", "Cancel", ButtonKind::Secondary)
                    .on_click(move |_, window, cx| {
                        cancel
                            .update(cx, |studio, cx| studio.cancel_compose(window, cx))
                            .ok();
                    }),
            )
            .action(
                c.button(
                    "apply-compose",
                    if matches!(compose, Some(Compose::Edit(_))) {
                        "Save"
                    } else {
                        "Add"
                    },
                    ButtonKind::Primary,
                )
                .on_click(move |_, window, cx| {
                    studio
                        .update(cx, |studio, cx| studio.finish_compose(window, cx))
                        .ok();
                }),
            )
            .on_dismiss(move |window, cx| {
                dismiss
                    .update(cx, |studio, cx| studio.cancel_compose(window, cx))
                    .ok();
            })
    }
}

fn tile(card: &Card) -> Tile {
    let tile = Tile::new(card.name.clone()).detail(card.detail());
    let tile = match &card.image {
        Some(path) => tile.image(path.clone()),
        None => tile.glyph(Glyph::Image),
    };
    if card.variations > 0 {
        tile.stacked(card.variations)
            .badge((card.variations + 1).to_string())
    } else {
        tile
    }
}
