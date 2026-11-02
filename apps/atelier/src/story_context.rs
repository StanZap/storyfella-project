use crate::{
    app::{DrawingApp, Panel},
    worker::Storage,
};
use anyhow::{anyhow, ensure, Result};
use gpui_kit::*;
use std::{
    path::PathBuf,
    time::{Duration, Instant},
};
use storyfella_studio_core::atelier::{Mailbox, OpenRequest};

pub(crate) struct StoryTarget {
    pub base_revision: Option<uuid::Uuid>,
    pub path: PathBuf,
    pub artwork: Option<(crate::document::Layer, u32, u32)>,
}

impl StoryTarget {
    pub(crate) fn apply(
        &self,
        doc: &mut crate::document::Document,
        request: &OpenRequest,
    ) -> Result<u32> {
        let mut next = doc.clone();
        let fresh = !next
            .asset_links
            .iter()
            .any(|link| link.asset_id == request.artifact_id);
        let canvas = next.ensure_story_artboard(
            request.artifact_id,
            request.context.clone(),
            request.description.clone(),
        )?;
        next.artwork_bases
            .entry(canvas)
            .or_insert(self.base_revision);
        if fresh {
            if let Some((layer, width, height)) = &self.artwork {
                next.width = *width;
                next.height = *height;
                next.layers = vec![layer.clone()];
                next.active_layer = 0;
            }
        }
        next.validate()?;
        *doc = next;
        Ok(canvas)
    }
}

pub(crate) fn prepare_target(request: &OpenRequest) -> Result<StoryTarget> {
    let folder = request.validate()?;
    let path = crate::document::project_document(&folder.manifest())?;
    let doc = crate::document::load(&path)?;
    let stored =
        storyfella_studio_core::persistence::ProjectDb::open_existing(folder.database())?.load()?;
    let artwork = if doc
        .asset_links
        .iter()
        .any(|link| link.asset_id == request.artifact_id)
    {
        None
    } else if let Some(revision) = stored.registry.latest_image(request.artifact_id) {
        if let Some(image_path) = &revision.asset_path {
            let item = stored
                .registry
                .artifact(request.artifact_id)
                .ok_or_else(|| anyhow!("Item no longer exists"))?;
            let reference = std::sync::Arc::new(crate::document::reference_from_bytes(
                item.name.clone(),
                &std::fs::read(image_path)?,
                format!("Storyfella · asset {} · revision {}", item.id, revision.id),
            )?);
            let image = crate::document::decode_reference(&reference)?;
            let (w, h) = (image.width(), image.height());
            Some((
                crate::workflow::image_layer(reference, w, h, (0, 0, w, h), false)?,
                w.max(64),
                h.max(64),
            ))
        } else {
            None
        }
    } else {
        None
    };
    Ok(StoryTarget {
        base_revision: stored
            .registry
            .artifact(request.artifact_id)
            .and_then(|item| item.active_revision_id),
        path,
        artwork,
    })
}

impl DrawingApp {
    pub(crate) fn receive_story_context(
        &mut self,
        request: OpenRequest,
        result: Result<StoryTarget>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.handoff_loading = false;
        let target = match result {
            Ok(target) => target,
            Err(error) => {
                self.finish_handoff(request.id, Err(error));
                return;
            }
        };
        self.finish_mask();
        if self.mask_dirty || self.mask_save_error.is_some() {
            self.save_mask();
            self.finish_handoff(
                request.id,
                Err(anyhow!("Save the current mask, then retry Open in Atelier")),
            );
            return;
        }
        let path = target.path.clone();
        self.handoff_open = Some((request, target));
        self.open_project(path, window, cx);
    }

    fn finish_handoff(&mut self, id: uuid::Uuid, result: Result<()>) {
        let message = result
            .as_ref()
            .err()
            .map(|e| format!("Could not open from Studio: {e:#}"));
        if let Err(error) = Mailbox::new(&self.data_dir).complete(id, result) {
            self.message(format!("Could not report handoff result: {error:#}"), true);
        }
        if let Some(message) = message {
            self.message(message, true);
        }
    }

    pub(crate) fn poll_story_context(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.switching {
            if let Some((request, target)) = self.handoff_open.take() {
                let result = (|| -> Result<()> {
                    ensure!(self.path.as_ref().and_then(|current| current.canonicalize().ok()).zip(target.path.canonicalize().ok()).is_some_and(|(current, target)| current == target), "The project could not be opened. Resolve the current save error and retry from Studio");
                    ensure!(
                        self.mask_save_error.is_none(),
                        "Save the current mask before opening another item"
                    );
                    self.capture_prompt(cx);
                    self.finish_stroke();
                    let previous = self.doc.clone();
                    let canvas = target.apply(&mut self.doc, &request)?;
                    if !previous
                        .asset_links
                        .iter()
                        .any(|link| link.asset_id == request.artifact_id)
                    {
                        self.view = crate::viewport::Viewport::default();
                    }
                    self.history.checkpoint(&previous);
                    self.changed();
                    self.sync_inputs(window, cx);
                    self.open_canvas_artboard(canvas, window, cx);
                    self.panel = Panel::StoryContext;
                    window.activate_window();
                    cx.activate(true);
                    cx.notify();
                    Ok(())
                })();
                self.finish_handoff(request.id, result);
            }
        }
        if self.handoff_loading
            || self.handoff_open.is_some()
            || self.switching
            || self.closing
            || self.handoff_checked.elapsed() < Duration::from_millis(250)
        {
            return;
        }
        self.handoff_checked = Instant::now();
        match Mailbox::new(&self.data_dir).take() {
            Ok(Some(request)) => {
                self.handoff_loading = self
                    .workers
                    .storage
                    .send(Storage::StoryContext(request.clone()))
                    .is_ok();
                if !self.handoff_loading {
                    self.finish_handoff(request.id, Err(anyhow!("Storage worker stopped")));
                }
            }
            Ok(None) => {}
            Err(error) => self.message(format!("Could not read Studio request: {error:#}"), true),
        }
    }
}
