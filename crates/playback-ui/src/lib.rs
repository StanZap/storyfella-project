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
                self.error = Some(error);
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

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn failed_draft_keeps_snapshot_progress_and_bookmark() {
        let directory = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../.svs-e2e/tests")
            .join(format!("playback-ui-recovery-{}", std::process::id()));
        let (source, registry) =
            storyfella_studio_core::playback::fixture::create(&directory).unwrap();
        let mut model = PlaybackModel::default();
        model.install(PlaybackSession::compile(&source, &registry));
        model.dispatch(Command::Advance);
        model.bookmark();
        let bookmark = model.bookmark.clone();
        let progress = model.session.as_ref().unwrap().runtime.save();
        model.install(Err("Draft has errors".into()));
        assert_eq!(model.error.as_deref(), Some("Draft has errors"));
        assert_eq!(model.bookmark, bookmark);
        assert_eq!(model.session.as_ref().unwrap().runtime.save(), progress);
        model.install(PlaybackSession::compile(&source, &registry));
        assert!(model.error.is_none());
        assert!(model.bookmark.is_none());
        assert_ne!(model.session.as_ref().unwrap().runtime.save(), progress);
        std::fs::remove_dir_all(directory).unwrap();
    }
}
