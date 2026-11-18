//! Interactive playback state shared by every visual host: the session, the
//! last error, one bookmark, and a bounded back history.
use super::PlaybackSession;
use storyfella_runtime::{Command, SavedState};

const MAX_BACK_HISTORY: usize = 128;

#[derive(Clone, Debug, Default)]
pub struct PlaybackModel {
    pub session: Option<PlaybackSession>,
    pub error: Option<String>,
    pub bookmark: Option<SavedState>,
    history: Vec<SavedState>,
}

impl PlaybackModel {
    /// A failed compile keeps the running session, its progress and bookmark.
    pub fn install(&mut self, result: Result<PlaybackSession, String>) {
        match result {
            Ok(session) => {
                self.session = Some(session);
                self.error = None;
                self.bookmark = None;
                self.history.clear();
            }
            Err(error) => {
                self.error = Some(error);
            }
        }
    }
    pub fn dispatch(&mut self, command: Command) {
        let previous = if matches!(command, Command::Advance | Command::SelectChoice { .. }) {
            self.session.as_ref().map(|session| session.runtime.save())
        } else {
            None
        };
        let resets_history = matches!(command, Command::Restart | Command::Jump { .. });
        self.error = self
            .session
            .as_mut()
            .and_then(|s| s.dispatch(command).err());
        if self.error.is_none() {
            if resets_history {
                self.history.clear();
            } else if let Some(previous) = previous {
                self.history.push(previous);
                if self.history.len() > MAX_BACK_HISTORY {
                    self.history.remove(0);
                }
            }
        }
    }
    pub fn back(&mut self) {
        if let Some(saved) = self.history.pop() {
            self.error = self
                .session
                .as_mut()
                .and_then(|s| s.restore(saved.clone()).err());
            if self.error.is_some() {
                self.history.push(saved);
            }
        }
    }
    pub fn can_back(&self) -> bool {
        !self.history.is_empty()
    }
    pub fn bookmark(&mut self) {
        self.bookmark = self.session.as_ref().map(|s| s.runtime.save());
    }
    pub fn restore(&mut self, saved: SavedState) {
        self.error = self.session.as_mut().and_then(|s| s.restore(saved).err());
        if self.error.is_none() {
            self.history.clear();
        }
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
    use crate::registry::ArtifactRegistry;
    use storyfella_runtime::AssetPolicy;

    #[test]
    fn back_history_tracks_successful_steps_and_resets_after_restore() {
        let mut model = PlaybackModel::default();
        model.install(PlaybackSession::compile_with_policy(
            "scene start\n  narrate: First\n  narrate: Second\n  end\n",
            &ArtifactRegistry::new(),
            AssetPolicy::Preview,
        ));
        let initial = model.session.as_ref().unwrap().runtime.save();
        model.dispatch(Command::Advance);
        let second = model.session.as_ref().unwrap().runtime.save();
        assert!(model.can_back());
        model.back();
        assert_eq!(model.session.as_ref().unwrap().runtime.save(), initial);
        assert!(!model.can_back());

        model.dispatch(Command::Pause);
        model.dispatch(Command::Advance);
        assert!(model.error.is_some());
        assert!(!model.can_back());
        model.restore(second);
        assert!(!model.can_back());
        assert!(model.error.is_none());
    }

    #[test]
    fn failed_draft_keeps_snapshot_progress_and_bookmark() {
        let directory = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../.svs-e2e/tests")
            .join(format!("playback-model-recovery-{}", std::process::id()));
        let (source, registry) = super::super::fixture::create(&directory).unwrap();
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
