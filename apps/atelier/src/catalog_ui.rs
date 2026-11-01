//! Shared asset definitions and explicit artboard links, separate from image
//! production. Catalog persistence runs on the existing storage worker.
use std::path::PathBuf;

use gpui_kit::{
    component::{button::*, input::Textarea, Disableable, Sizable},
    *,
};
use storyfella_studio_core::{
    catalog::{self, CatalogEdit},
    persistence::{ProjectDb, StoredProject},
    project::ProjectFolder,
};
use uuid::Uuid;

use crate::{
    app::{DrawingApp, Panel},
    ui::{column, field, label, row, side_panel_frame, tool_button, AtelierSelection},
    worker::Storage,
};

pub struct CatalogState {
    pub path: PathBuf,
    pub snapshot: StoredProject,
    pub selected: Option<Uuid>,
}

impl DrawingApp {
    pub(crate) fn shared_catalog_path(&self) -> Option<PathBuf> {
        let path = self.path.as_ref()?;
        let parent = path.parent()?;
        let root = parent.parent()?;
        (parent.file_name().is_some_and(|name| name == "atelier")
            && root
                .join(storyfella_studio_core::project::MANIFEST)
                .is_file())
        .then(|| root.join("story.db"))
    }

    pub(crate) fn open_catalog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.catalog_busy {
            return;
        }
        let result = (|| -> anyhow::Result<CatalogState> {
            let path = self
                .shared_catalog_path()
                .ok_or_else(|| anyhow::anyhow!("Open a shared Storyfella project first"))?;
            let folder = ProjectFolder::open(
                path.parent()
                    .ok_or_else(|| anyhow::anyhow!("Project folder is missing"))?,
            )?;
            let snapshot = ProjectDb::open(folder.database())?.load()?;
            Ok(CatalogState {
                path,
                snapshot,
                selected: None,
            })
        })();
        match result {
            Ok(mut catalog) => {
                let same_project = self
                    .catalog
                    .as_ref()
                    .is_some_and(|old| old.path == catalog.path);
                if same_project {
                    catalog.selected = self.catalog.as_ref().and_then(|old| old.selected);
                }
                self.catalog = Some(catalog);
                self.catalog_error = None;
                if !same_project {
                    self.new_catalog_item(window, cx);
                }
                self.panel = Panel::Catalog;
            }
            Err(error) => self.message(format!("Could not open assets: {error:#}"), true),
        }
        cx.notify();
    }

    fn new_catalog_item(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(catalog) = &mut self.catalog {
            catalog.selected = None;
        }
        self.catalog_base = None;
        self.catalog_name
            .update(cx, |s, cx| s.set_value("", window, cx));
        self.catalog_description
            .update(cx, |s, cx| s.set_value("", window, cx));
        cx.notify();
    }

    fn select_catalog_item(&mut self, id: Uuid, window: &mut Window, cx: &mut Context<Self>) {
        let Some(catalog) = &mut self.catalog else {
            return;
        };
        let Some(item) = catalog.snapshot.registry.artifact(id) else {
            return;
        };
        self.catalog_kind = item.kind;
        self.catalog_base = item.variant_of;
        self.catalog_name
            .update(cx, |s, cx| s.set_value(item.name.clone(), window, cx));
        self.catalog_description.update(cx, |s, cx| {
            s.set_value(item.description.clone(), window, cx)
        });
        catalog.selected = Some(id);
        self.catalog_error = None;
        cx.notify();
    }

    fn submit_catalog_edit(&mut self, edit: CatalogEdit, cx: &mut Context<Self>) {
        if self.catalog_busy {
            return;
        }
        let Some(catalog) = &self.catalog else { return };
        if self.shared_catalog_path().as_ref() != Some(&catalog.path) {
            self.catalog_error = Some("Open Assets for the current project before saving.".into());
            cx.notify();
            return;
        }
        self.catalog_busy = self
            .workers
            .storage
            .send(Storage::CatalogEdit {
                epoch: self.epoch,
                path: catalog.path.clone(),
                snapshot: catalog.snapshot.clone(),
                edit,
            })
            .is_ok();
        self.catalog_error = if self.catalog_busy {
            None
        } else {
            Some("Storage worker stopped".into())
        };
        cx.notify();
    }

    fn import_catalog_image(&mut self, id: Uuid, window: &mut Window, cx: &mut Context<Self>) {
        let epoch = self.epoch;
        let receiver = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some("Attach saved image to this asset".into()),
        });
        let entity = cx.weak_entity();
        window
            .spawn(cx, async move |cx| {
                let result = receiver.await;
                let _ = entity.update_in(cx, |this, _, cx| {
                    if epoch != this.epoch {
                        return;
                    }
                    match result {
                        Ok(Ok(Some(paths))) if !paths.is_empty() => this.submit_catalog_edit(
                            CatalogEdit::ImportImage {
                                id,
                                path: paths[0].clone(),
                            },
                            cx,
                        ),
                        Ok(Ok(_)) => {}
                        _ => this.catalog_error = Some("File dialog could not be opened".into()),
                    }
                    cx.notify();
                });
            })
            .detach();
    }

    pub(crate) fn catalog_panel(&self, cx: &mut Context<Self>) -> AnyElement {
        let mut content = column().gap_3();
        let Some(catalog) = &self.catalog else {
            return content.into_any_element();
        };
        let current = self.shared_catalog_path().as_ref() == Some(&catalog.path);
        let selected = catalog.selected;
        let disabled = self.catalog_busy || !current;
        content = content.child(
            row()
                .child(
                    Button::new("catalog-new")
                        .ghost()
                        .small()
                        .label("New item")
                        .disabled(disabled)
                        .on_click(
                            cx.listener(|this, _, window, cx| this.new_catalog_item(window, cx)),
                        ),
                )
                .child(
                    tool_button("catalog-refresh", "rotate", "Refresh asset catalog")
                        .disabled(self.catalog_busy)
                        .on_click(cx.listener(|this, _, window, cx| this.open_catalog(window, cx))),
                ),
        );
        if !current {
            content = content.child(label("Refresh Assets for the current project."));
        }
        for item in catalog
            .snapshot
            .registry
            .artifacts
            .iter()
            .filter(|item| catalog::KINDS.contains(&item.kind))
        {
            let id = item.id;
            let base = item
                .variant_of
                .and_then(|id| catalog.snapshot.registry.artifact(id));
            let title = match base {
                Some(base) => format!("{} / {}", base.name, item.name),
                None => item.name.clone(),
            };
            let availability = if item
                .revisions
                .iter()
                .any(|r| Some(r.id) == item.active_revision_id && r.asset_path.is_some())
            {
                "Saved artwork"
            } else {
                "No artwork"
            };
            content = content.child(
                Button::new(format!("catalog-{id}"))
                    .ghost()
                    .small()
                    .label(format!("{title} · {availability}"))
                    .disabled(disabled)
                    .atelier_selected(selected == Some(id), cx)
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.select_catalog_item(id, window, cx)
                    })),
            );
        }
        if selected.is_none() {
            content = content.child(row().flex_wrap().children(catalog::KINDS.into_iter().map(
                |kind| {
                    Button::new(format!("catalog-kind-{kind}"))
                        .ghost()
                        .small()
                        .label(kind.to_string())
                        .disabled(disabled)
                        .atelier_selected(self.catalog_kind == kind, cx)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.catalog_kind = kind;
                            this.catalog_base = None;
                            cx.notify();
                        }))
                },
            )));
        }
        content = content
            .child(field("Reference name", &self.catalog_name))
            .child(label("Description"))
            .child(Textarea::new(&self.catalog_description).h(px(90.)));
        if let Some(base) = self
            .catalog_base
            .and_then(|id| catalog.snapshot.registry.artifact(id))
        {
            content = content.child(label(format!("Variation of {}", base.name)));
        }
        content = content.child(
            Button::new("catalog-submit")
                .small()
                .label(if selected.is_some() {
                    "Save details"
                } else {
                    "Create item"
                })
                .disabled(disabled)
                .on_click(cx.listener(|this, _, _, cx| {
                    let name = this.catalog_name.read(cx).value().to_string();
                    let description = this.catalog_description.read(cx).value().to_string();
                    let selected = this.catalog.as_ref().and_then(|catalog| catalog.selected);
                    let edit = match selected {
                        Some(id) => CatalogEdit::Describe {
                            id,
                            name,
                            description,
                        },
                        None => CatalogEdit::Create {
                            kind: this.catalog_kind,
                            name,
                            description,
                            variant_of: this.catalog_base,
                        },
                    };
                    this.submit_catalog_edit(edit, cx);
                })),
        );
        if let Some(id) = selected {
            let variantable = catalog
                .snapshot
                .registry
                .artifact(id)
                .is_some_and(|a| a.variant_of.is_none() && a.kind.supports_variants());
            content = content
                .child(
                    Button::new("catalog-variant")
                        .ghost()
                        .small()
                        .label("New variation")
                        .disabled(disabled || !variantable)
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.new_catalog_item(window, cx);
                            this.catalog_base = Some(id);
                            cx.notify();
                        })),
                )
                .child(
                    Button::new("catalog-attach")
                        .ghost()
                        .small()
                        .label("Attach saved image…")
                        .disabled(disabled)
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.import_catalog_image(id, window, cx)
                        })),
                )
                .child(
                    Button::new("catalog-link")
                        .ghost()
                        .small()
                        .label("Link current artboard")
                        .disabled(disabled)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            let canvas_id = this.doc.canvas_id;
                            this.history.checkpoint(&this.doc);
                            this.doc
                                .asset_links
                                .retain(|link| link.canvas_id != canvas_id);
                            this.doc.asset_links.push(catalog::AssetLink {
                                asset_id: id,
                                canvas_id,
                            });
                            this.changed();
                            this.message(
                                "Artboard linked to asset · Save retains this link",
                                false,
                            );
                            cx.notify();
                        })),
                );
        }
        if let Some(error) = &self.catalog_error {
            content = content.child(label(error.clone()).text_color(rgb(0x9a3412)));
        }
        if self.catalog_busy {
            content = content.child(label("Saving catalog…"));
        }
        side_panel_frame(
            "Assets",
            content,
            tool_button("catalog-close", "close", "Close asset catalog").on_click(cx.listener(
                |this, _, _, cx| {
                    this.panel = Panel::None;
                    cx.notify();
                },
            )),
        )
        .into_any_element()
    }
}
