//! The generation queue.
//!
//! Every request is captured as a [`QueuedRequest`] when it is added, so the
//! provider, model, size, steps, adapters and prompt it will run with cannot be
//! changed afterwards. The queue is plain data; the app dispatches one item at a
//! time through the existing `worker::generate`, so two requests never share the
//! single GPU. This module owns ordering, pause and the states the UI renders.
use crate::{document::Reference, edit::MaskDraft, providers::Settings};
use std::sync::Arc;

/// A request captured at the moment it was queued.
#[derive(Clone)]
pub struct QueuedRequest {
    /// The provider, model, endpoint, size, steps and adapters to run with.
    pub settings: Settings,
    pub prompt: String,
    pub source: Option<Arc<Reference>>,
    /// Extra ordered references; the source is Image 1.
    pub references: Vec<Arc<Reference>>,
    pub mask: Option<MaskDraft>,
    pub editing: bool,
    /// A drawing selection to receive the generated image, in canvas pixels.
    pub destination: crate::workflow::Destination,
    /// Full-source output cropped locally for selection blend.
    pub blend: Option<crate::workflow::BlendCrop>,
}

impl QueuedRequest {
    /// The output size: a masked edit keeps the mask's source dimensions.
    pub fn size(&self) -> (u32, u32) {
        self.mask
            .as_ref()
            .map(|mask| (mask.width, mask.height))
            .unwrap_or((self.settings.width, self.settings.height))
    }

    /// A short description of what will run, for the queue list.
    pub fn summary(&self) -> String {
        let kind = if self.editing { "Edit" } else { "Create" };
        let (width, height) = self.size();
        let mut summary = format!("{kind} · {width}×{height}");
        if let Some(steps) = self.settings.steps {
            summary.push_str(&format!(" · {steps} steps"));
        }
        match self.settings.loras.len() {
            0 => {}
            1 => summary.push_str(" · 1 LoRA"),
            count => summary.push_str(&format!(" · {count} LoRAs")),
        }
        summary
    }
}

/// Where a queued request is in its life cycle.
#[derive(Clone, Debug, PartialEq)]
pub enum JobState {
    Queued,
    Running { progress: String },
    Done,
    Failed(String),
    Cancelled,
}

impl JobState {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Queued => "Queued",
            Self::Running { .. } => "Running",
            Self::Done => "Done",
            Self::Failed(_) => "Failed",
            Self::Cancelled => "Cancelled",
        }
    }

    /// Finished items stay visible as history until they are cleared.
    pub fn is_finished(&self) -> bool {
        matches!(self, Self::Done | Self::Failed(_) | Self::Cancelled)
    }
}

pub struct QueueItem {
    pub id: u64,
    pub request: QueuedRequest,
    pub state: JobState,
    /// The generated reference, when a completed request was added to a project.
    pub result_id: Option<String>,
    pub result: Option<Arc<Reference>>,
    /// The project epoch when the request was queued; a result only changes the
    /// project it was queued for.
    pub epoch: u64,
}

#[derive(Default)]
pub struct Queue {
    items: Vec<QueueItem>,
    paused: bool,
    next_id: u64,
}

impl Queue {
    pub fn enqueue(&mut self, request: QueuedRequest, epoch: u64) -> u64 {
        self.next_id += 1;
        let id = self.next_id;
        self.items.push(QueueItem {
            id,
            request,
            state: JobState::Queued,
            result_id: None,
            result: None,
            epoch,
        });
        id
    }

    /// Only queued items can be removed; a running or finished item is history.
    pub fn remove(&mut self, id: u64) {
        if self
            .get(id)
            .is_some_and(|item| item.state == JobState::Queued)
        {
            self.items.retain(|item| item.id != id);
        }
    }

    /// Drops finished items, keeping queued and running work.
    pub fn clear_finished(&mut self) {
        self.items.retain(|item| !item.state.is_finished());
    }

    pub fn is_paused(&self) -> bool {
        self.paused
    }

    /// Flips pause and returns the new state.
    pub fn toggle_paused(&mut self) -> bool {
        self.paused = !self.paused;
        self.paused
    }

    /// The next request to run, in the order it was added.
    pub fn next_queued(&self) -> Option<u64> {
        self.items
            .iter()
            .find(|item| item.state == JobState::Queued)
            .map(|item| item.id)
    }

    pub fn get(&self, id: u64) -> Option<&QueueItem> {
        self.items.iter().find(|item| item.id == id)
    }

    pub fn get_mut(&mut self, id: u64) -> Option<&mut QueueItem> {
        self.items.iter_mut().find(|item| item.id == id)
    }

    pub fn items(&self) -> &[QueueItem] {
        &self.items
    }

    pub fn pending(&self) -> usize {
        self.items
            .iter()
            .filter(|item| item.state == JobState::Queued)
            .count()
    }

    pub fn finished(&self) -> usize {
        self.items
            .iter()
            .filter(|item| item.state.is_finished())
            .count()
    }

    /// Requests that still need attention: queued or running.
    pub fn outstanding(&self) -> usize {
        self.items
            .iter()
            .filter(|item| !item.state.is_finished())
            .count()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::providers::LoraSetting;
    use crate::providers::Provider;

    fn request(prompt: &str) -> QueuedRequest {
        let settings = Settings {
            provider: Provider::QwenNative,
            width: 1024,
            height: 1536,
            steps: Some(25),
            ..Settings::default()
        };
        QueuedRequest {
            settings,
            prompt: prompt.into(),
            source: None,
            references: vec![],
            mask: None,
            editing: false,
            destination: crate::workflow::Destination::NewArtboard,
            blend: None,
        }
    }

    #[test]
    fn requests_run_in_the_order_they_were_added() {
        let mut queue = Queue::default();
        let first = queue.enqueue(request("a"), 0);
        let second = queue.enqueue(request("b"), 0);
        assert_eq!(queue.next_queued(), Some(first));
        queue.get_mut(first).unwrap().state = JobState::Done;
        assert_eq!(queue.next_queued(), Some(second));
        assert_eq!(queue.pending(), 1);
        assert_eq!(queue.finished(), 1);
    }

    #[test]
    fn pause_stops_dispatch_but_keeps_the_order() {
        let mut queue = Queue::default();
        queue.enqueue(request("a"), 0);
        assert!(!queue.is_paused());
        assert!(queue.toggle_paused());
        assert!(queue.is_paused());
        // Pausing does not drop queued work; only dispatch is held back.
        assert_eq!(queue.pending(), 1);
        assert!(!queue.toggle_paused());
        assert!(queue.next_queued().is_some());
    }

    #[test]
    fn clearing_finished_history_keeps_queued_and_running_work() {
        let mut queue = Queue::default();
        let running = queue.enqueue(request("running"), 0);
        let queued = queue.enqueue(request("queued"), 0);
        let done = queue.enqueue(request("done"), 0);
        queue.get_mut(running).unwrap().state = JobState::Running {
            progress: String::new(),
        };
        queue.get_mut(done).unwrap().state = JobState::Failed("boom".into());
        queue.clear_finished();
        assert_eq!(queue.items().len(), 2);
        assert!(queue.get(queued).is_some());
        assert!(queue.get(running).is_some());
        assert!(queue.get(done).is_none());
    }

    #[test]
    fn only_queued_requests_can_be_removed() {
        let mut queue = Queue::default();
        let id = queue.enqueue(request("a"), 0);
        queue.get_mut(id).unwrap().state = JobState::Done;
        queue.remove(id);
        assert!(queue.get(id).is_some(), "a finished request is history");

        let id = queue.enqueue(request("b"), 0);
        queue.remove(id);
        assert!(queue.get(id).is_none(), "a queued request can be removed");
    }

    #[test]
    fn a_masked_edit_keeps_the_masks_dimensions() {
        let mut request = request("mask");
        request.mask = Some(MaskDraft {
            width: 768,
            height: 512,
            strokes: vec![],
        });
        assert_eq!(request.size(), (768, 512));
    }

    #[test]
    fn the_summary_reports_size_steps_and_adapters() {
        let mut request = request("a character");
        request.settings.loras = vec![LoraSetting {
            name: "characters-mia.safetensors".into(),
            multiplier: 1.0,
        }];
        assert_eq!(request.summary(), "Create · 1024×1536 · 25 steps · 1 LoRA");
        request.editing = true;
        assert!(request.summary().starts_with("Edit ·"));
    }
}
