//! Shared, quiet desktop presentation for deterministic story playback.
mod controls;
mod icons;
mod panel;

pub use panel::PlaybackPanel;
use storyfella_runtime::{Command, SavedState};
use storyfella_studio_core::playback::PlaybackSession;

const MAX_BACK_HISTORY: usize = 128;

#[derive(Clone, Debug, Default)]
pub struct PlaybackModel {
    pub session: Option<PlaybackSession>,
    pub error: Option<String>,
    pub bookmark: Option<SavedState>,
    history: Vec<SavedState>,
}
impl PlaybackModel {
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
    use dioxus::prelude::*;

    #[test]
    fn back_history_tracks_successful_steps_and_resets_after_restore() {
        let mut model = PlaybackModel::default();
        model.install(PlaybackSession::compile_with_policy(
            "scene start\n  narrate: First\n  narrate: Second\n  end\n",
            &storyfella_studio_core::registry::ArtifactRegistry::new(),
            storyfella_runtime::AssetPolicy::Preview,
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

    #[component]
    fn MissingPreviewProbe() -> Element {
        let model = use_signal(|| {
            let mut model = PlaybackModel::default();
            model.install(PlaybackSession::compile_with_policy(
                "scene harbor\n  location: dock\n  cast: mara\n  prop: lantern\n  shot: wide\n  mara: Hello\n  end\n",
                &storyfella_studio_core::registry::ArtifactRegistry::new(),
                storyfella_runtime::AssetPolicy::Preview,
            ));
            model
        });
        rsx! { PlaybackPanel { model, on_apply: move |_| {} } }
    }

    #[component]
    fn StagedPreviewProbe() -> Element {
        let model = use_signal(|| {
            let mut model = PlaybackModel::default();
            model.install(PlaybackSession::compile_with_policy(
                "scene harbor\n  stage: lead, mara, left, 125, 4\n  stage: friend, tam, right, 100, 2, 73, 12\n  narrate: Hello\n",
                &storyfella_studio_core::registry::ArtifactRegistry::new(),
                storyfella_runtime::AssetPolicy::Preview,
            ));
            model
        });
        rsx! { PlaybackPanel { model, on_apply: move |_| {} } }
    }

    #[test]
    fn preview_renders_missing_staged_characters_at_authored_positions() {
        let mut dom = VirtualDom::new(StagedPreviewProbe);
        let rendered = format!("{:?}", dom.rebuild_to_vec());
        for value in [
            "mara · missing artwork",
            "tam · missing artwork",
            "left:20%;bottom:0%;height:90%;z-index:4",
            "left:73%;bottom:12%;height:72%;z-index:2",
        ] {
            assert!(rendered.contains(value), "missing staged output: {value}");
        }
    }

    #[test]
    fn preview_renders_names_for_missing_active_artwork() {
        let mut dom = VirtualDom::new(MissingPreviewProbe);
        let rendered = format!("{:?}", dom.rebuild_to_vec());
        for name in ["dock", "mara", "lantern"] {
            assert!(rendered.contains(name), "missing placeholder for {name}");
        }
    }
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
