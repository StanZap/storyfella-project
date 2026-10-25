//! Open documents retain their history, drafts and dirty state independently.
use crate::{
    app::{DrawingApp, Page, Panel},
    document::{Document, History},
    viewport::Viewport,
    worker::{self, Storage},
};
use gpui_kit::*;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Clone, Serialize, Deserialize)]
pub struct RecoveryTab {
    pub epoch: u64,
    pub path: Option<PathBuf>,
    pub dirty: bool,
    #[serde(default)]
    pub active: bool,
}

pub struct RecoveredProjects {
    pub tabs: Vec<ProjectTab>,
    pub active: Option<ProjectTab>,
    pub order: Vec<u64>,
}

pub fn recover(directory: &std::path::Path) -> anyhow::Result<RecoveredProjects> {
    let entries: Vec<RecoveryTab> = serde_json::from_slice(&std::fs::read(
        directory.join("open-projects/session.json"),
    )?)?;
    anyhow::ensure!(entries.len() <= 32, "Invalid open-project session");
    anyhow::ensure!(
        entries.iter().filter(|entry| entry.active).count() <= 1,
        "Multiple active projects in recovery"
    );
    let active_epoch = entries
        .iter()
        .find(|entry| entry.active)
        .or(entries.last())
        .map(|entry| entry.epoch);
    let mut order: Vec<_> = entries.iter().map(|entry| entry.epoch).collect();
    // Older sessions stored the active project last and displayed creation order.
    if !entries.iter().any(|entry| entry.active) {
        order.sort_unstable();
    }
    let mut ids = std::collections::HashSet::new();
    let mut tabs = Vec::new();
    for entry in entries {
        anyhow::ensure!(
            ids.insert(entry.epoch),
            "Duplicate project recovery identity"
        );
        tabs.push(ProjectTab {
            doc: crate::document::load(
                &directory
                    .join("open-projects")
                    .join(format!("{}.atelier", entry.epoch)),
            )?,
            epoch: entry.epoch,
            path: entry.path,
            dirty: entry.dirty,
            revision: 1,
            history: History::new(),
            view: Viewport::default(),
            page: Page::Artboards,
        });
    }
    let active = active_epoch.and_then(|epoch| {
        tabs.iter()
            .position(|tab| tab.epoch == epoch)
            .map(|index| tabs.remove(index))
    });
    Ok(RecoveredProjects {
        tabs,
        active,
        order,
    })
}

/// Move a tab to the target's position without changing the active document.
pub fn reorder(order: &mut Vec<u64>, epoch: u64, target: u64) -> bool {
    let Some(from) = order.iter().position(|id| *id == epoch) else {
        return false;
    };
    let Some(to) = order.iter().position(|id| *id == target) else {
        return false;
    };
    if from == to {
        return false;
    }
    order.remove(from);
    order.insert(to, epoch);
    true
}

#[derive(Clone)]
pub struct DraggedProject {
    pub epoch: u64,
    pub title: String,
}

impl Render for DraggedProject {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        crate::ui::panel()
            .px_3()
            .py_2()
            .text_sm()
            .child(self.title.clone())
    }
}

pub fn save_session(
    directory: &std::path::Path,
    projects: Vec<(RecoveryTab, Document)>,
) -> anyhow::Result<()> {
    let mut entries = Vec::new();
    for (entry, doc) in projects {
        crate::document::save(&directory.join(format!("{}.atelier", entry.epoch)), &doc)?;
        entries.push(entry);
    }
    crate::document::atomic_write(
        &directory.join("session.json"),
        &serde_json::to_vec(&entries)?,
    )
}

pub struct ProjectTab {
    pub epoch: u64,
    pub doc: Document,
    pub history: History,
    pub path: Option<PathBuf>,
    pub dirty: bool,
    pub revision: u64,
    pub view: Viewport,
    pub page: Page,
}

impl DrawingApp {
    pub fn ensure_project_tab(&mut self) {
        if self.project_tab_order.is_empty() {
            self.project_tab_order.push(self.epoch);
        }
    }

    pub fn park_project(&mut self) -> ProjectTab {
        ProjectTab {
            epoch: self.epoch,
            doc: self.doc.clone(),
            history: std::mem::replace(&mut self.history, History::new()),
            path: self.path.clone(),
            dirty: self.dirty,
            revision: self.revision,
            view: self.view,
            page: if self.page == Page::Draw {
                Page::Draw
            } else {
                Page::Artboards
            },
        }
    }

    pub fn activate_project_tab(
        &mut self,
        epoch: u64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.project_tab_order.contains(&epoch) {
            return;
        }
        if epoch == self.epoch {
            if self.page == Page::Library || self.page == Page::Projects {
                self.page = Page::Artboards;
                self.editing_image = false;
                self.restore_prompt(window, cx);
            }
            cx.notify();
            return;
        }
        if !self.prepare_project_switch() {
            return;
        }
        self.capture_prompt(cx);
        let Some(index) = self.project_tabs.iter().position(|tab| tab.epoch == epoch) else {
            return;
        };
        let current = self.park_project();
        let selected = std::mem::replace(&mut self.project_tabs[index], current);
        self.install_project_tab(selected, window, cx);
        self.persist_open_projects();
        cx.notify();
    }

    fn install_project_tab(
        &mut self,
        selected: ProjectTab,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.doc = selected.doc;
        self.history = selected.history;
        self.path = selected.path;
        self.epoch = selected.epoch;
        self.dirty = selected.dirty;
        self.revision = selected.revision;
        self.view = selected.view;
        self.page = selected.page;
        self.panel = Panel::None;
        self.generation_region = None;
        self.selecting_generation_region = false;
        self.restore_drawing_context();
        self.refresh_project_view(window, cx);
        self.restore_prompt(window, cx);
    }

    pub fn reorder_project_tab(&mut self, epoch: u64, target: u64, cx: &mut Context<Self>) {
        if reorder(&mut self.project_tab_order, epoch, target) {
            self.persist_open_projects();
            cx.notify();
        }
    }

    pub fn move_project_tab(&mut self, offset: isize, cx: &mut Context<Self>) {
        let Some(index) = self
            .project_tab_order
            .iter()
            .position(|id| *id == self.epoch)
        else {
            return;
        };
        if let Some(target) = index
            .checked_add_signed(offset)
            .and_then(|index| self.project_tab_order.get(index))
            .copied()
        {
            self.reorder_project_tab(self.epoch, target, cx);
        }
    }

    pub fn close_project_tab(&mut self, epoch: u64, cx: &mut Context<Self>) {
        if self.closing
            || !self.project_tab_order.contains(&epoch)
            || self.closing_project.is_some()
            || self.manual_pending
        {
            return;
        }
        if self
            .queue
            .items()
            .iter()
            .any(|item| item.epoch == epoch && !item.state.is_finished())
        {
            self.message(
                "Wait for this project's image requests to finish, or cancel them in the queue",
                false,
            );
            self.panel = Panel::Activity;
            cx.notify();
            return;
        }
        if !self.prepare_project_switch() {
            return;
        }
        self.capture_prompt(cx);
        self.finish_stroke();
        let (doc, path, revision) = if epoch == self.epoch {
            (self.doc.clone(), self.path.clone(), self.revision)
        } else if let Some(tab) = self.project_tabs.iter().find(|tab| tab.epoch == epoch) {
            (tab.doc.clone(), tab.path.clone(), tab.revision)
        } else {
            return;
        };
        let path = path.unwrap_or_else(|| {
            self.data_dir
                .join(format!("project-closed-{epoch}.atelier"))
        });
        let failure_path = path.clone();
        match self.workers.storage.send(Storage::CloseProject {
            doc,
            path,
            epoch,
            revision,
        }) {
            Ok(()) => {
                self.closing_project = Some(epoch);
                self.message("Saving project before closing…", false);
            }
            Err(_) => {
                self.report_error(
                    crate::activity::Scope::Project(epoch),
                    crate::activity::Task::Save(failure_path),
                    "Project stays open: storage worker stopped".into(),
                );
            }
        }
        cx.notify();
    }

    pub fn finish_project_close(
        &mut self,
        epoch: u64,
        revision: u64,
        result: anyhow::Result<PathBuf>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.closing_project != Some(epoch) {
            return;
        }
        self.closing_project = None;
        self.capture_prompt(cx);
        self.finish_stroke();
        let path = match result {
            Ok(path) if self.mask_save_error.is_none() => path,
            Ok(_) => {
                self.message("Project stays open: save the current mask first", false);
                return;
            }
            Err(error) => {
                let path = if epoch == self.epoch {
                    self.path.clone()
                } else {
                    self.project_tabs
                        .iter()
                        .find(|tab| tab.epoch == epoch)
                        .and_then(|tab| tab.path.clone())
                }
                .unwrap_or_else(|| {
                    self.data_dir
                        .join(format!("project-closed-{epoch}.atelier"))
                });
                self.report_error(
                    crate::activity::Scope::Project(epoch),
                    crate::activity::Task::Save(path),
                    format!("Project stays open: save failed: {error:#}"),
                );
                return;
            }
        };
        self.activity.resolve(
            &crate::activity::Scope::Project(epoch),
            &crate::activity::Task::Save(path.clone()),
        );
        let unchanged = if epoch == self.epoch {
            self.path = Some(path);
            self.revision == revision
        } else if let Some(tab) = self.project_tabs.iter_mut().find(|tab| tab.epoch == epoch) {
            tab.path = Some(path);
            tab.revision == revision
        } else {
            return;
        };
        if !unchanged {
            self.message(
                "Saved a snapshot; newer changes keep the project open",
                false,
            );
            self.persist_open_projects();
            return;
        }
        let index = self
            .project_tab_order
            .iter()
            .position(|id| *id == epoch)
            .expect("Closing tab is open");
        self.project_tab_order.remove(index);
        let workspace_page = self.page;
        if epoch == self.epoch {
            if let Some(next) = self
                .project_tab_order
                .get(index.min(self.project_tab_order.len().saturating_sub(1)))
                .copied()
            {
                let index = self
                    .project_tabs
                    .iter()
                    .position(|tab| tab.epoch == next)
                    .expect("Open tab has a document");
                let selected = self.project_tabs.remove(index);
                self.install_project_tab(selected, window, cx);
                if matches!(workspace_page, Page::Projects | Page::Library) {
                    self.page = workspace_page;
                }
            } else {
                self.next_project_epoch += 1;
                self.install_project_tab(
                    ProjectTab {
                        epoch: self.next_project_epoch,
                        doc: Document::default(),
                        history: History::new(),
                        path: None,
                        dirty: false,
                        revision: 1,
                        view: Viewport::default(),
                        page: Page::Projects,
                    },
                    window,
                    cx,
                );
            }
        } else {
            self.project_tabs.retain(|tab| tab.epoch != epoch);
        }
        self.persist_open_projects();
        self.refresh_projects();
        self.message("Project saved and closed", false);
        cx.notify();
    }

    pub fn refresh_project_view(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.finish_stroke();
        self.visual_revision += 1;
        self.opened_image = None;
        self.opening_image = None;
        self.comparison = None;
        self.comparing = false;
        self.mask = None;
        self.references = self
            .doc
            .references
            .iter()
            .filter_map(|r| crate::document::decode_reference(r).ok())
            .map(|i| worker::render_image(crate::document::thumbnail(i, 800).to_rgba8()))
            .collect();
        self.reference_cache = self
            .doc
            .references
            .iter()
            .cloned()
            .zip(self.references.iter().cloned())
            .collect();
        self.sync_inputs(window, cx);
        self.reset_recovery_clock();
    }

    /// Save every tab on shutdown. Separate recovery files preserve unsaved
    /// changes without claiming their explicit project files have been saved.
    pub fn persist_open_projects(&self) {
        let projects = self
            .project_tab_order
            .iter()
            .filter_map(|epoch| {
                if *epoch == self.epoch {
                    Some((
                        RecoveryTab {
                            epoch: *epoch,
                            path: self.path.clone(),
                            dirty: self.dirty,
                            active: true,
                        },
                        self.doc.clone(),
                    ))
                } else {
                    self.project_tabs
                        .iter()
                        .find(|tab| tab.epoch == *epoch)
                        .map(|tab| {
                            (
                                RecoveryTab {
                                    epoch: *epoch,
                                    path: tab.path.clone(),
                                    dirty: tab.dirty,
                                    active: false,
                                },
                                tab.doc.clone(),
                            )
                        })
                }
            })
            .collect();
        let _ = self.workers.storage.send(Storage::WorkspaceRecovery(
            self.data_dir.join("open-projects"),
            projects,
        ));
    }
}
