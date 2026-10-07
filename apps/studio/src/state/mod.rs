use std::path::PathBuf;

use storyfella_language::analyze;
use storyfella_studio_core::authoring::{reconcile, AssetCatalog};
use uuid::Uuid;

use storyfella_studio_core::persistence::StoredProject;
use storyfella_studio_core::registry::{Artifact, ArtifactRegistry, RevisionStatus};
use storyfella_studio_core::story::StoryDocument;

/// Cap on the undo stack depth (source and artwork travel together).
const UNDO_DEPTH: usize = 100;

#[derive(Clone, Debug, PartialEq)]
struct AuthoringSnapshot {
    registry: ArtifactRegistry,
    source: String,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct AppState {
    /// Project artwork and source associations. UI mutations go through
    /// these methods; operations and reconciliation own domain changes.
    pub registry: ArtifactRegistry,
    /// The `projects` row name (stable per database file).
    pub project_name: String,
    /// The SQLite database file this project lives in (`None` until the
    /// project is created or opened from the Projects screen).
    pub project_path: Option<PathBuf>,
    pub has_unsaved_changes: bool,
    pub selected_artifact_id: Option<Uuid>,
    /// Exact Writer text, bound `.sf` path and disk snapshot for conflict checks.
    pub story: StoryDocument,
    operation_in_progress: bool,
    saved_associations: Vec<storyfella_studio_core::authoring::SceneAssociation>,
    /// Registry snapshots taken before each applied operation — undo is
    /// state restore, never pipeline re-execution.
    undo_stack: Vec<AuthoringSnapshot>,
    redo_stack: Vec<AuthoringSnapshot>,
}

impl AppState {
    /// Replaces the in-memory state with a snapshot loaded from disk.
    /// The caller is responsible for saving before navigating away.
    pub fn open_project(&mut self, stored: StoredProject, path: PathBuf) {
        self.saved_associations = stored.registry.story_scenes.clone();
        self.registry = stored.registry;
        self.project_name = stored.name;
        self.story = stored.story;
        self.project_path = Some(path);
        self.has_unsaved_changes = false;
        self.selected_artifact_id = None;
        self.undo_stack.clear();
        self.redo_stack.clear();
        self.reconcile_story();
    }

    pub fn selected_artifact(&self) -> Option<&Artifact> {
        let selected_id = self.selected_artifact_id?;
        self.registry.artifact(selected_id)
    }

    pub fn set_story_source(&mut self, source: String) {
        if self.story.source() != source {
            self.snapshot_for_undo();
            self.story.set_source(source);
            self.reconcile_story();
            self.has_unsaved_changes = true;
        }
    }

    pub fn reconcile_story(&mut self) {
        let analysis = analyze(
            self.story.source(),
            &AssetCatalog::from_registry(&self.registry).assets,
        );
        if reconcile(&mut self.registry, &analysis) {
            self.has_unsaved_changes = true;
        }
    }

    /// Create the missing catalog entry immediately; rendering stays a Canvas
    /// action. Recheck shared resolution to reject collisions and wrong kinds.
    pub fn create_pending_asset(
        &mut self,
        name: &str,
        kind: storyfella_language::AssetKind,
    ) -> Option<Uuid> {
        let analysis = analyze(
            self.story.source(),
            &AssetCatalog::from_registry(&self.registry).assets,
        );
        let pending = analysis.references.iter().any(|reference| {
            reference.name.value == name
                && reference.kind == storyfella_language::ReferenceKind::Asset(kind)
                && analysis
                    .diagnostics
                    .iter()
                    .any(|d| d.code == "SF203" && d.span == reference.name.span)
        });
        if !pending {
            return None;
        }
        self.snapshot_for_undo();
        let id = self
            .registry
            .create_artifact(
                storyfella_studio_core::authoring::artifact_kind(kind),
                storyfella_studio_core::registry::CreateArtifact {
                    name: name.into(),
                    ..Default::default()
                },
            )
            .expect("asset kinds accept parentless placeholder creation");
        self.reconcile_story();
        self.has_unsaved_changes = true;
        Some(id)
    }

    pub fn operation_in_progress(&self) -> bool {
        self.operation_in_progress
    }

    pub fn begin_operation(&mut self) {
        self.operation_in_progress = true;
    }

    pub fn finish_operation(&mut self, baseline: &ArtifactRegistry, finished: ArtifactRegistry) {
        // Writer may have added associations while the worker was generating.
        // Apply its artifact delta without replacing the newer source structure.
        self.snapshot_for_undo();
        for artifact in finished.artifacts {
            if baseline.artifact(artifact.id) == Some(&artifact) {
                continue;
            }
            if let Some(current) = self.registry.artifact_mut(artifact.id) {
                *current = artifact;
            } else {
                self.registry.artifacts.push(artifact);
            }
        }
        self.registry.loras = finished.loras;
        self.registry.log.extend(
            finished
                .log
                .into_iter()
                .filter(|entry| !baseline.log.iter().any(|old| old.id == entry.id)),
        );
        self.operation_in_progress = false;
        self.reconcile_story();
        self.has_unsaved_changes = true;
    }

    pub fn cancel_operation(&mut self) {
        self.operation_in_progress = false;
    }

    /// Adopt a successful save/reload's disk snapshot; undo retains text only,
    /// so it never restores an obsolete disk snapshot and invents a conflict.
    pub fn adopt_story_document(&mut self, story: StoryDocument) {
        if self.story.source() != story.source() {
            self.snapshot_for_undo();
            let analysis = analyze(
                story.source(),
                &AssetCatalog::from_registry(&self.registry).assets,
            );
            if !analysis.has_errors() {
                // Reloading a saved declaration after an unsaved rename/removal
                // reattaches its saved UUID, without restoring older revisions.
                let mut known = self.registry.story_scenes.clone();
                for saved in &self.saved_associations {
                    if self.registry.artifact(saved.artifact_id).is_none() {
                        continue;
                    }
                    if let Some(scene) = known.iter_mut().find(|s| s.name == saved.name) {
                        if scene.artifact_id == saved.artifact_id {
                            scene.beats.extend(
                                saved
                                    .beats
                                    .iter()
                                    .filter(|b| {
                                        self.registry.artifact(b.artifact_id).is_some()
                                            && !scene
                                                .beats
                                                .iter()
                                                .any(|current| current.name == b.name)
                                    })
                                    .cloned()
                                    .collect::<Vec<_>>(),
                            );
                        }
                    } else {
                        let mut scene = saved.clone();
                        scene
                            .beats
                            .retain(|b| self.registry.artifact(b.artifact_id).is_some());
                        known.push(scene);
                    }
                }
                self.registry.story_scenes = known;
            }
        }
        self.story = story;
        self.reconcile_story();
    }

    pub fn mark_saved(&mut self) {
        self.saved_associations = self.registry.story_scenes.clone();
        self.has_unsaved_changes = false;
    }

    fn authoring_snapshot(&self) -> AuthoringSnapshot {
        AuthoringSnapshot {
            registry: self.registry.clone(),
            source: self.story.source().to_owned(),
        }
    }

    /// Selection is validated: ids that do not exist are ignored.
    pub fn select_artifact(&mut self, artifact_id: Uuid) {
        if self.registry.artifact(artifact_id).is_some() {
            self.selected_artifact_id = Some(artifact_id);
        }
    }

    /// Drops the selection, which is what returns the canvas to the grid.
    pub fn clear_selection(&mut self) {
        self.selected_artifact_id = None;
    }

    /// Snapshot the registry before an operation applies — the undo basis.
    /// Also clears the redo stack (a new operation forks history).
    pub fn snapshot_for_undo(&mut self) {
        self.undo_stack.push(self.authoring_snapshot());
        if self.undo_stack.len() > UNDO_DEPTH {
            self.undo_stack.remove(0);
        }
        self.redo_stack.clear();
    }

    pub fn can_undo(&self) -> bool {
        !self.operation_in_progress && !self.undo_stack.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.operation_in_progress && !self.redo_stack.is_empty()
    }

    /// Restores the last pre-operation snapshot. The selection is pruned
    /// when the restored registry no longer contains the artifact.
    pub fn undo(&mut self) -> bool {
        if self.operation_in_progress {
            return false;
        }
        let Some(snapshot) = self.undo_stack.pop() else {
            return false;
        };
        self.redo_stack.push(self.authoring_snapshot());
        self.registry = snapshot.registry;
        self.story.set_source(snapshot.source);
        self.prune_selection();
        self.has_unsaved_changes = true;
        true
    }

    pub fn redo(&mut self) -> bool {
        if self.operation_in_progress {
            return false;
        }
        let Some(snapshot) = self.redo_stack.pop() else {
            return false;
        };
        self.undo_stack.push(self.authoring_snapshot());
        self.registry = snapshot.registry;
        self.story.set_source(snapshot.source);
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
    use super::{AppState, StoryDocument};
    use storyfella_studio_core::registry::{ArtifactKind, ArtifactRegistry, CreateArtifact};

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
    fn story_edits_preserve_exact_text_and_mark_changes() {
        let mut state = state_with("mia");
        let registry = state.registry.clone();
        state.set_story_source(String::new());
        assert!(!state.has_unsaved_changes);
        let source = "// 🌊\r\nscene unfinished\r\n   editing\r\n";
        state.set_story_source(source.to_owned());
        assert_eq!(state.story.source(), source);
        assert!(state.story.is_dirty());
        assert!(state.has_unsaved_changes);
        assert_eq!(
            state.registry, registry,
            "partial drafts do not mutate artwork"
        );
    }

    #[test]
    fn structural_edits_and_pending_creation_undo_source_and_registry_together() {
        let mut state = AppState::default();
        let source = "scene harbor\n  beat arrival\n    prop: lantern\n";
        state.set_story_source(source.into());
        let before = state.registry.clone();
        let lantern = state
            .create_pending_asset("lantern", storyfella_language::AssetKind::Prop)
            .unwrap();
        assert!(state
            .create_pending_asset("lantern", storyfella_language::AssetKind::Prop)
            .is_none());
        assert_eq!(
            state.registry.artifact(lantern).unwrap().kind,
            ArtifactKind::Object
        );
        assert!(state.undo());
        assert_eq!(state.registry, before);
        assert_eq!(state.story.source(), source);
        assert!(state.redo());
        let with_asset = state.registry.clone();
        state.set_story_source("scene sea\n  end\n".into());
        assert!(state.undo());
        assert_eq!(state.registry, with_asset);
        assert_eq!(state.story.source(), source);
        assert!(state.redo());
        assert_eq!(state.registry.story_scenes[0].name, "sea");
        assert!(state
            .registry
            .artifact(before.story_scenes[0].artifact_id)
            .is_some());
    }

    #[test]
    fn worker_completion_keeps_new_source_associations_and_has_a_joint_undo() {
        let mut state = state_with("mara");
        state.set_story_source("scene harbor\n".into());
        let baseline = state.registry.clone();
        state.begin_operation();
        let mut finished = baseline.clone();
        finished.artifacts[0].description = "revised canon".into();
        state.set_story_source("scene harbor\n  beat arrival\n".into());
        assert!(!state.undo());
        let before_completion = state.registry.clone();
        state.finish_operation(&baseline, finished);
        assert_eq!(state.registry.story_scenes, before_completion.story_scenes);
        assert_eq!(state.registry.artifacts[0].description, "revised canon");
        assert!(state.undo());
        assert_eq!(state.registry, before_completion);
        assert_eq!(state.story.source(), "scene harbor\n  beat arrival\n");
    }

    #[test]
    fn opening_a_stored_project_replaces_state_and_marks_clean() {
        let mut state = state_with("mia");
        state.project_name = "Draft".into();
        state.snapshot_for_undo();

        let stored = storyfella_studio_core::persistence::StoredProject {
            id: uuid::Uuid::new_v4(),
            name: "Stored Story".to_owned(),
            registry: character_registry("stored"),
            story: StoryDocument::default(),
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
            .start_revision(
                mia,
                "first draft".to_owned(),
                None,
                None,
                storyfella_studio_core::registry::RevisionOrigin::Canon,
            )
            .expect("revision should start");

        assert!(
            !state.activate_revision(mia, queued),
            "a queued revision without an asset cannot be activated"
        );

        let completed = state
            .registry
            .start_revision(
                mia,
                "second draft".to_owned(),
                None,
                None,
                storyfella_studio_core::registry::RevisionOrigin::Canon,
            )
            .expect("revision should start");
        state
            .registry
            .finish_revision(mia, completed, "assets/generated/mia.png".to_owned())
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
            .start_revision(
                mia,
                "first".to_owned(),
                None,
                None,
                storyfella_studio_core::registry::RevisionOrigin::Canon,
            )
            .expect("revision should start");
        state
            .registry
            .finish_revision(mia, first, "assets/generated/first.png".to_owned())
            .expect("revision should finish");
        let second = state
            .registry
            .start_revision(
                mia,
                "second".to_owned(),
                None,
                None,
                storyfella_studio_core::registry::RevisionOrigin::Canon,
            )
            .expect("revision should start");
        state
            .registry
            .finish_revision(mia, second, "assets/generated/second.png".to_owned())
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
