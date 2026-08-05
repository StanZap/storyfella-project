use std::path::PathBuf;

use uuid::Uuid;

use crate::{
    persistence::StoredProject,
    registry::{Artifact, ArtifactRegistry, RevisionStatus},
};

/// Cap on the undo stack depth (each entry is a full registry snapshot).
const UNDO_DEPTH: usize = 100;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct AppState {
    /// The artifact registry — the whole project. Mutate it through
    /// `registry::ops::execute`, never by editing `artifacts` directly.
    pub registry: ArtifactRegistry,
    /// The `projects` row name (stable per database file).
    pub project_name: String,
    /// The SQLite database file this project lives in (`None` until the
    /// project is created or opened from the Projects screen).
    pub project_path: Option<PathBuf>,
    pub has_unsaved_changes: bool,
    pub selected_artifact_id: Option<Uuid>,
    /// Registry snapshots taken before each applied operation — undo is
    /// state restore, never pipeline re-execution.
    undo_stack: Vec<ArtifactRegistry>,
    redo_stack: Vec<ArtifactRegistry>,
}

impl AppState {
    pub fn create_project(&mut self, name: impl Into<String>) {
        let name = name.into();
        self.registry = ArtifactRegistry::default();
        self.project_name = if name.trim().is_empty() {
            "Untitled Story".to_owned()
        } else {
            name.trim().to_owned()
        };
        self.project_path = None;
        self.has_unsaved_changes = true;
        self.selected_artifact_id = None;
        self.undo_stack.clear();
        self.redo_stack.clear();
    }

    /// Replaces the in-memory state with a snapshot loaded from disk.
    /// The caller is responsible for saving before navigating away.
    pub fn open_project(&mut self, stored: StoredProject, path: PathBuf) {
        self.registry = stored.registry;
        self.project_name = stored.name;
        self.project_path = Some(path);
        self.has_unsaved_changes = false;
        self.selected_artifact_id = None;
        self.undo_stack.clear();
        self.redo_stack.clear();
    }

    pub fn selected_artifact(&self) -> Option<&Artifact> {
        let selected_id = self.selected_artifact_id?;
        self.registry.artifact(selected_id)
    }

    /// Selection is validated: ids that do not exist are ignored.
    pub fn select_artifact(&mut self, artifact_id: Uuid) {
        if self.registry.artifact(artifact_id).is_some() {
            self.selected_artifact_id = Some(artifact_id);
        }
    }

    /// Snapshot the registry before an operation applies — the undo basis.
    /// Also clears the redo stack (a new operation forks history).
    pub fn snapshot_for_undo(&mut self) {
        self.undo_stack.push(self.registry.clone());
        if self.undo_stack.len() > UNDO_DEPTH {
            self.undo_stack.remove(0);
        }
        self.redo_stack.clear();
    }

    pub fn can_undo(&self) -> bool {
        !self.undo_stack.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo_stack.is_empty()
    }

    /// Restores the last pre-operation snapshot. The selection is pruned
    /// when the restored registry no longer contains the artifact.
    pub fn undo(&mut self) -> bool {
        let Some(snapshot) = self.undo_stack.pop() else {
            return false;
        };
        self.redo_stack.push(self.registry.clone());
        self.registry = snapshot;
        self.prune_selection();
        self.has_unsaved_changes = true;
        true
    }

    pub fn redo(&mut self) -> bool {
        let Some(snapshot) = self.redo_stack.pop() else {
            return false;
        };
        self.undo_stack.push(self.registry.clone());
        self.registry = snapshot;
        self.prune_selection();
        self.has_unsaved_changes = true;
        true
    }

    /// Marks a revision as the artifact's active image. Only revisions that
    /// produced an asset can be activated; the active revision is what
    /// [`Self::display_image`] shows.
    pub fn activate_revision(&mut self, artifact_id: Uuid, revision_id: Uuid) -> bool {
        let Some(artifact) = self.registry.artifact_mut(artifact_id) else {
            return false;
        };
        let Some(revision) = artifact
            .revisions
            .iter()
            .find(|revision| revision.id == revision_id)
        else {
            return false;
        };
        if revision.asset_path.is_none() {
            return false;
        }
        artifact.active_revision_id = Some(revision_id);
        self.has_unsaved_changes = true;
        true
    }

    /// The image to show for an artifact: the active revision's asset when
    /// it has one, otherwise the latest completed revision.
    pub fn display_image<'a>(&self, artifact: &'a Artifact) -> Option<&'a str> {
        let active = artifact
            .active_revision_id
            .and_then(|id| artifact.revisions.iter().find(|revision| revision.id == id))
            .and_then(|revision| revision.asset_path.as_deref());
        if active.is_some() {
            return active;
        }
        artifact
            .revisions
            .iter()
            .rev()
            .find(|revision| revision.status == RevisionStatus::Completed)
            .and_then(|revision| revision.asset_path.as_deref())
    }

    fn prune_selection(&mut self) {
        if let Some(selected) = self.selected_artifact_id {
            if self.registry.artifact(selected).is_none() {
                self.selected_artifact_id = None;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::AppState;
    use crate::registry::{ArtifactKind, ArtifactRegistry, CreateArtifact};

    fn character_registry(name: &str) -> ArtifactRegistry {
        let mut registry = ArtifactRegistry::default();
        registry
            .create_artifact(
                ArtifactKind::Character,
                CreateArtifact {
                    name: name.into(),
                    description: "A test character".into(),
                    ..Default::default()
                },
            )
            .expect("artifact should create");
        registry
    }

    /// State holding one named character (the common test fixture).
    fn state_with(name: &str) -> AppState {
        AppState {
            registry: character_registry(name),
            ..Default::default()
        }
    }

    #[test]
    fn creating_a_project_resets_registry_name_and_history() {
        let mut state = state_with("mia");
        state.snapshot_for_undo();

        state.create_project("A New Story");

        assert_eq!(state.project_name, "A New Story");
        assert!(state.registry.artifacts.is_empty());
        assert!(state.has_unsaved_changes);
        assert!(state.selected_artifact_id.is_none());
        assert!(!state.can_undo());
        assert!(!state.can_redo());
    }

    #[test]
    fn opening_a_stored_project_replaces_state_and_marks_clean() {
        let mut state = state_with("mia");
        state.create_project("Draft");

        let stored = crate::persistence::StoredProject {
            id: uuid::Uuid::new_v4(),
            name: "Stored Story".to_owned(),
            registry: character_registry("stored"),
        };
        state.open_project(stored, std::path::PathBuf::from("/tmp/stored.db"));

        assert_eq!(state.project_name, "Stored Story");
        assert_eq!(state.registry.artifacts.len(), 1);
        assert_eq!(state.registry.artifacts[0].name, "stored");
        assert!(!state.has_unsaved_changes);
        assert!(!state.can_undo(), "history does not survive open");
        assert_eq!(
            state.project_path.as_deref(),
            Some(std::path::Path::new("/tmp/stored.db"))
        );
    }

    #[test]
    fn selection_ignores_unknown_artifact_ids() {
        let mut state = state_with("mia");
        let mia = state.registry.artifacts[0].id;

        state.select_artifact(mia);
        assert_eq!(state.selected_artifact_id, Some(mia));

        state.select_artifact(uuid::Uuid::new_v4());
        assert_eq!(
            state.selected_artifact_id,
            Some(mia),
            "unknown ids do not clear the selection"
        );
        assert!(state.selected_artifact().is_some());
    }

    #[test]
    fn undo_restores_the_pre_operation_snapshot_and_redoes() {
        let mut state = state_with("mia");
        let mia = state.registry.artifacts[0].id;
        state.select_artifact(mia);
        let before = state.registry.clone();

        state.snapshot_for_undo();
        state.registry = character_registry("mia");
        state.registry.artifacts[0].name = "mia-rain-gear".to_owned();
        state.has_unsaved_changes = true;

        assert!(state.can_undo());
        assert!(state.undo());
        assert_eq!(state.registry, before, "undo restores the snapshot");
        assert!(state.can_redo());

        assert!(state.redo());
        assert_eq!(state.registry.artifacts[0].name, "mia-rain-gear");
        assert!(!state.can_redo());
    }

    #[test]
    fn undo_prunes_a_selection_that_no_longer_exists() {
        // The pre-operation snapshot has no artifacts; the op creates one
        // and selects it. Undo restores the snapshot, and the selection must
        // be pruned because the artifact only exists in the undone state.
        let mut state = AppState::default();
        state.snapshot_for_undo();

        state.registry = character_registry("mia");
        let mia = state.registry.artifacts[0].id;
        state.selected_artifact_id = Some(mia);

        assert!(state.undo());
        assert_eq!(
            state.selected_artifact_id, None,
            "selection is pruned when the restored registry lacks the artifact"
        );
    }

    #[test]
    fn a_new_operation_clears_the_redo_stack() {
        let mut state = state_with("mia");
        state.snapshot_for_undo();
        state.registry.artifacts[0].name = "changed".to_owned();
        assert!(state.undo());
        assert!(state.can_redo());

        state.snapshot_for_undo();
        assert!(!state.can_redo(), "a new operation forks history");
    }

    #[test]
    fn activation_requires_a_revision_with_an_asset() {
        let mut state = state_with("mia");
        let mia = state.registry.artifacts[0].id;
        let queued = state
            .registry
            .start_revision(mia, "first draft".to_owned(), None, None)
            .expect("revision should start");

        assert!(
            !state.activate_revision(mia, queued),
            "a queued revision without an asset cannot be activated"
        );

        let completed = state
            .registry
            .start_revision(mia, "second draft".to_owned(), None, None)
            .expect("revision should start");
        state
            .registry
            .finish_revision(
                mia,
                completed,
                "assets/generated/mia.png".to_owned(),
                Vec::new(),
            )
            .expect("revision should finish");
        assert!(state.activate_revision(mia, completed));
        let artifact = state.registry.artifact(mia).expect("artifact exists");
        assert_eq!(artifact.active_revision_id, Some(completed));
        assert!(state.has_unsaved_changes);
    }

    #[test]
    fn display_image_prefers_the_active_revision() {
        let mut state = state_with("mia");
        let mia = state.registry.artifacts[0].id;
        let first = state
            .registry
            .start_revision(mia, "first".to_owned(), None, None)
            .expect("revision should start");
        state
            .registry
            .finish_revision(
                mia,
                first,
                "assets/generated/first.png".to_owned(),
                Vec::new(),
            )
            .expect("revision should finish");
        let second = state
            .registry
            .start_revision(mia, "second".to_owned(), None, None)
            .expect("revision should start");
        state
            .registry
            .finish_revision(
                mia,
                second,
                "assets/generated/second.png".to_owned(),
                Vec::new(),
            )
            .expect("revision should finish");

        let artifact = state.registry.artifact(mia).expect("artifact exists");
        assert_eq!(
            state.display_image(artifact),
            Some("assets/generated/second.png"),
            "the latest completed revision is the display image"
        );

        assert!(state.activate_revision(mia, first));
        let artifact = state.registry.artifact(mia).expect("artifact exists");
        assert_eq!(
            state.display_image(artifact),
            Some("assets/generated/first.png"),
            "activating an older revision changes the display image"
        );
    }
}
