//! Shared, quiet desktop presentation for deterministic story playback.
mod controls;
mod icons;
mod panel;

pub use panel::PlaybackPanel;
use storyfella_runtime::{Command, SavedState};
use storyfella_studio_core::playback::PlaybackSession;

#[derive(Clone, Debug, Default)]
pub struct PlaybackModel {
    pub session: Option<PlaybackSession>,
    pub error: Option<String>,
    pub bookmark: Option<SavedState>,
}
impl PlaybackModel {
    pub fn install(&mut self, result: Result<PlaybackSession, String>) {
        match result {
            Ok(session) => {
                self.session = Some(session);
                self.error = None;
                self.bookmark = None;
            }
            Err(error) => {
                self.session = None;
                self.error = Some(error);
                self.bookmark = None;
            }
        }
    }
    pub fn dispatch(&mut self, command: Command) {
        self.error = self
            .session
            .as_mut()
            .and_then(|s| s.dispatch(command).err());
    }
    pub fn bookmark(&mut self) {
        self.bookmark = self.session.as_ref().map(|s| s.runtime.save());
    }
    pub fn restore(&mut self, saved: SavedState) {
        self.error = self.session.as_mut().and_then(|s| s.restore(saved).err());
    }
    pub fn restore_bookmark(&mut self) {
        if let Some(saved) = self.bookmark.clone() {
            self.restore(saved);
        }
    }
}
