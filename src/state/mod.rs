use crate::{
    models::{Project, StoryboardFrame},
    timeline::Timeline,
};
use uuid::Uuid;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct AppState {
    pub project: Project,
    pub has_unsaved_changes: bool,
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
        self.has_unsaved_changes = true;
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
        self.project.storyboard.push(StoryboardFrame {
            id: Uuid::new_v4(),
            prompt: prompt.trim().to_owned(),
            asset_path: None,
        });
        self.project.timeline.push(label, start_seconds, 5.0);
        self.has_unsaved_changes = true;
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
    }

    #[test]
    fn blank_storyboard_beats_are_ignored() {
        let mut state = AppState::default();

        state.add_storyboard_beat("   ");

        assert!(state.project.storyboard.is_empty());
        assert!(!state.has_unsaved_changes);
    }
}
