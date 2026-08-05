use std::path::PathBuf;

use crate::{
    models::{ImageRevision, Project, RevisionStatus, StoryboardFrame},
    registry::ArtifactRegistry,
    timeline::Timeline,
};
use uuid::Uuid;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct AppState {
    pub project: Project,
    /// The artifact registry — new domain model layered onto `Project`
    /// (`docs/ROADMAP.md` §3). Mutate it through
    /// `registry::ops::execute`, never by editing `artifacts` directly.
    pub registry: ArtifactRegistry,
    /// The SQLite database file this project lives in (`None` until the
    /// project is created or opened from the Projects screen).
    pub project_path: Option<PathBuf>,
    pub has_unsaved_changes: bool,
    pub selected_frame_id: Option<Uuid>,
}

impl AppState {
    pub fn create_project(&mut self, name: impl Into<String>) {
        let name = name.into();
        self.project = Project {
            id: Uuid::new_v4(),
            name: if name.trim().is_empty() {
                "Untitled Story".to_owned()
            } else {
                name.trim().to_owned()
            },
            timeline: Timeline::default(),
            storyboard: Vec::new(),
        };
        self.registry = ArtifactRegistry::default();
        self.project_path = None;
        self.has_unsaved_changes = true;
        self.selected_frame_id = None;
    }

    pub fn add_storyboard_beat(&mut self, prompt: impl Into<String>) {
        let prompt = prompt.into();
        if prompt.trim().is_empty() {
            return;
        }

        let label = format!("Beat {}", self.project.storyboard.len() + 1);
        let start_seconds = self
            .project
            .timeline
            .clips
            .last()
            .map_or(0.0, |clip| clip.start_seconds + clip.duration_seconds);
        let frame_id = Uuid::new_v4();
        self.project.storyboard.push(StoryboardFrame {
            id: frame_id,
            prompt: prompt.trim().to_owned(),
            asset_path: None,
            revisions: Vec::new(),
            active_revision_id: None,
        });
        self.project.timeline.push(label, start_seconds, 5.0);
        self.has_unsaved_changes = true;
        self.selected_frame_id = Some(frame_id);
    }

    pub fn select_frame(&mut self, frame_id: Uuid) {
        if self
            .project
            .storyboard
            .iter()
            .any(|frame| frame.id == frame_id)
        {
            self.selected_frame_id = Some(frame_id);
        }
    }

    pub fn begin_new_beat(&mut self) {
        self.selected_frame_id = None;
    }

    pub fn selected_frame(&self) -> Option<&StoryboardFrame> {
        let selected_id = self.selected_frame_id?;
        self.project
            .storyboard
            .iter()
            .find(|frame| frame.id == selected_id)
    }

    pub fn start_revision(&mut self, frame_id: Uuid, prompt: impl Into<String>) -> Option<Uuid> {
        let frame = self
            .project
            .storyboard
            .iter_mut()
            .find(|frame| frame.id == frame_id)?;
        let revision_id = Uuid::new_v4();
        frame.revisions.push(ImageRevision {
            id: revision_id,
            prompt: prompt.into(),
            asset_path: None,
            status: RevisionStatus::Queued,
            error: None,
        });
        frame.active_revision_id = Some(revision_id);
        self.has_unsaved_changes = true;
        Some(revision_id)
    }

    pub fn update_revision(
        &mut self,
        frame_id: Uuid,
        revision_id: Uuid,
        status: RevisionStatus,
        asset_path: Option<String>,
        error: Option<String>,
    ) {
        let Some(frame) = self
            .project
            .storyboard
            .iter_mut()
            .find(|frame| frame.id == frame_id)
        else {
            return;
        };
        let Some(revision) = frame
            .revisions
            .iter_mut()
            .find(|revision| revision.id == revision_id)
        else {
            return;
        };
        revision.status = status;
        if let Some(path) = asset_path {
            frame.asset_path = Some(path.clone());
            revision.asset_path = Some(path);
        }
        revision.error = error;
        self.has_unsaved_changes = true;
    }

    pub fn activate_revision(&mut self, frame_id: Uuid, revision_id: Uuid) {
        let Some(frame) = self
            .project
            .storyboard
            .iter_mut()
            .find(|frame| frame.id == frame_id)
        else {
            return;
        };
        let Some(revision) = frame.revisions.iter().find(|item| item.id == revision_id) else {
            return;
        };
        let Some(asset_path) = revision.asset_path.clone() else {
            return;
        };
        frame.active_revision_id = Some(revision_id);
        frame.asset_path = Some(asset_path);
        self.has_unsaved_changes = true;
    }

    /// Replaces the in-memory state with a snapshot loaded from disk.
    /// The caller is responsible for saving before navigating away.
    pub fn open_project(&mut self, stored: crate::persistence::StoredProject, path: PathBuf) {
        self.project = stored.project;
        self.registry = stored.registry;
        self.project_path = Some(path);
        self.has_unsaved_changes = false;
        self.selected_frame_id = None;
    }
}

#[cfg(test)]
mod tests {
    use super::AppState;

    #[test]
    fn creating_a_project_resets_its_sequence() {
        let mut state = AppState::default();
        state.add_storyboard_beat("Opening shot");

        state.create_project("A New Story");

        assert_eq!(state.project.name, "A New Story");
        assert!(state.project.storyboard.is_empty());
        assert!(state.project.timeline.clips.is_empty());
        assert!(state.has_unsaved_changes);
        assert!(state.selected_frame_id.is_none());
    }

    #[test]
    fn opening_a_stored_project_replaces_state_and_marks_clean() {
        let mut state = AppState::default();
        state.add_storyboard_beat("Draft beat");
        state.registry.artifacts.push(crate::registry::Artifact {
            id: uuid::Uuid::new_v4(),
            kind: crate::registry::ArtifactKind::Story,
            name: "stored".to_owned(),
            description: "From disk".to_owned(),
            variant_of: None,
            variant_axis: None,
            parent_id: None,
            composition: None,
            active_revision_id: None,
            revisions: Vec::new(),
            drafts: Vec::new(),
        });
        state.has_unsaved_changes = true;

        let stored = crate::persistence::StoredProject {
            id: uuid::Uuid::new_v4(),
            name: "Stored Story".to_owned(),
            project: crate::models::Project {
                id: uuid::Uuid::new_v4(),
                name: "Stored Story".to_owned(),
                timeline: crate::timeline::Timeline::default(),
                storyboard: Vec::new(),
            },
            registry: crate::registry::ArtifactRegistry::default(),
        };
        state.open_project(stored, std::path::PathBuf::from("/tmp/stored.db"));

        assert_eq!(state.project.name, "Stored Story");
        assert!(state.project.storyboard.is_empty());
        assert!(state.registry.artifacts.is_empty());
        assert!(!state.has_unsaved_changes);
        assert_eq!(
            state.project_path.as_deref(),
            Some(std::path::Path::new("/tmp/stored.db"))
        );
    }

    #[test]
    fn a_storyboard_beat_creates_a_matching_timeline_clip() {
        let mut state = AppState::default();

        state.add_storyboard_beat("  A lighthouse at dusk  ");
        state.add_storyboard_beat("The lantern wakes");

        assert_eq!(state.project.storyboard.len(), 2);
        assert_eq!(state.project.storyboard[0].prompt, "A lighthouse at dusk");
        assert_eq!(state.project.timeline.clips.len(), 2);
        assert_eq!(state.project.timeline.clips[1].start_seconds, 5.0);
        assert_eq!(
            state.selected_frame_id,
            Some(state.project.storyboard[1].id)
        );
    }

    #[test]
    fn blank_storyboard_beats_are_ignored() {
        let mut state = AppState::default();

        state.add_storyboard_beat("   ");

        assert!(state.project.storyboard.is_empty());
        assert!(!state.has_unsaved_changes);
    }

    #[test]
    fn revision_completion_updates_the_frame_asset() {
        let mut state = AppState::default();
        state.add_storyboard_beat("A lighthouse at dusk");
        let frame_id = state
            .selected_frame_id
            .expect("new beat should be selected");
        let revision_id = state
            .start_revision(frame_id, "A lighthouse at dusk")
            .expect("selected frame should exist");

        state.update_revision(
            frame_id,
            revision_id,
            crate::models::RevisionStatus::Completed,
            Some("assets/generated/frame.png".to_owned()),
            None,
        );

        let frame = state
            .selected_frame()
            .expect("frame should remain selected");
        assert_eq!(
            frame.asset_path.as_deref(),
            Some("assets/generated/frame.png")
        );
        assert_eq!(frame.revisions.len(), 1);
    }

    #[test]
    fn creating_a_project_resets_the_registry() {
        let mut state = AppState::default();
        state
            .registry
            .create_artifact(
                crate::registry::ArtifactKind::Character,
                "mia".into(),
                "Mia".into(),
                None,
                None,
                None,
            )
            .expect("artifact should create");

        state.create_project("A New Story");

        assert!(state.registry.artifacts.is_empty());
        assert!(state.registry.log.is_empty());
    }

    #[test]
    fn a_completed_revision_can_be_restored() {
        let mut state = AppState::default();
        state.add_storyboard_beat("A lighthouse at dusk");
        let frame_id = state
            .selected_frame_id
            .expect("new beat should be selected");
        let first = state
            .start_revision(frame_id, "first")
            .expect("frame should exist");
        state.update_revision(
            frame_id,
            first,
            crate::models::RevisionStatus::Completed,
            Some("first.png".to_owned()),
            None,
        );
        let second = state
            .start_revision(frame_id, "second")
            .expect("frame should exist");
        state.update_revision(
            frame_id,
            second,
            crate::models::RevisionStatus::Completed,
            Some("second.png".to_owned()),
            None,
        );

        state.activate_revision(frame_id, first);

        let frame = state.selected_frame().expect("frame should be selected");
        assert_eq!(frame.asset_path.as_deref(), Some("first.png"));
        assert_eq!(frame.active_revision_id, Some(first));
    }
}
