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
}

pub fn recover(directory: &std::path::Path) -> anyhow::Result<(Vec<ProjectTab>, ProjectTab)> {
    let entries: Vec<RecoveryTab> = serde_json::from_slice(&std::fs::read(
        directory.join("open-projects/session.json"),
    )?)?;
    anyhow::ensure!(
        !entries.is_empty() && entries.len() <= 32,
        "Invalid open-project session"
    );
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
    let active = tabs.pop().expect("Session is not empty");
    Ok((tabs, active))
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
        if epoch == self.epoch {
            if self.page == Page::Library || self.page == Page::Projects {
                self.page = Page::Artboards;
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
        self.persist_open_projects();
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
        let mut projects: Vec<_> = self
            .project_tabs
            .iter()
            .map(|tab| {
                (
                    RecoveryTab {
                        epoch: tab.epoch,
                        path: tab.path.clone(),
                        dirty: tab.dirty,
                    },
                    tab.doc.clone(),
                )
            })
            .collect();
        projects.push((
            RecoveryTab {
                epoch: self.epoch,
                path: self.path.clone(),
                dirty: self.dirty,
            },
            self.doc.clone(),
        ));
        let _ = self.workers.storage.send(Storage::WorkspaceRecovery(
            self.data_dir.join("open-projects"),
            projects,
        ));
    }
}
