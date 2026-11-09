use std::path::PathBuf;

use storyfella_language::analyze;
use storyfella_studio_core::authoring::{reconcile, AssetCatalog};
use uuid::Uuid;

use storyfella_studio_core::catalog::sync::{self, Conflict, Resolution};
use storyfella_studio_core::persistence::StoredProject;
use storyfella_studio_core::registry::{Artifact, ArtifactRegistry, RevisionStatus};
use storyfella_studio_core::story::{AdditionalSources, StoryDocument, StoryFileError};

/// Cap on the undo stack depth (source and artwork travel together).
const UNDO_DEPTH: usize = 100;

#[derive(Clone, Debug, PartialEq)]
struct AuthoringSnapshot {
    registry: ArtifactRegistry,
    source: String,
    additional: std::collections::BTreeMap<String, String>,
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
    pub registry_version: i64,
    saved_registry: ArtifactRegistry,
    project_id: Option<Uuid>,
    incoming_catalog: Option<StoredProject>,
    pub catalog_conflict: Option<Conflict>,
    pub has_unsaved_changes: bool,
    pub selected_artifact_id: Option<Uuid>,
    /// Exact Writer text, bound `.sf` path and disk snapshot for conflict checks.
    pub story: StoryDocument,
    pub additional_sources: Option<AdditionalSources>,
    pub active_source: Option<String>,
    pub source_notice: Option<String>,
    description_drafts: std::collections::BTreeMap<Uuid, String>,
    operation_in_progress: bool,
    saved_associations: Vec<storyfella_studio_core::authoring::SceneAssociation>,
    /// Registry snapshots taken before each applied operation — undo is
    /// state restore, never pipeline re-execution.
    undo_stack: Vec<AuthoringSnapshot>,
    redo_stack: Vec<AuthoringSnapshot>,
}

impl AppState {
    pub fn edit_catalog(
        &mut self,
        edit: storyfella_studio_core::catalog::CatalogEdit,
    ) -> Result<Uuid, String> {
        if self.operation_in_progress {
            return Err("Wait for the current operation before editing the catalog.".into());
        }
        let mut registry = self.registry.clone();
        let id = storyfella_studio_core::catalog::apply(&mut registry, edit)
            .map_err(|error| error.to_string())?;
        let mut source = self.story.source().to_owned();
        if let (Some(before), Some(after)) = (self.registry.artifact(id), registry.artifact(id)) {
            if before.name != after.name {
                if storyfella_language::has_includes(self.story.source()) {
                    let name = after.name.clone();
                    self.rename_workspace_asset(id, &name, registry)?;
                    self.selected_artifact_id = Some(id);
                    return Ok(id);
                }
                source = storyfella_studio_core::catalog::rename_references(
                    &self.registry,
                    &source,
                    id,
                    &after.name,
                )
                .map_err(|error| error.to_string())?;
            }
        }
        self.snapshot_for_undo();
        self.registry = registry;
        self.story.set_source(source);
        self.selected_artifact_id = Some(id);
        self.reconcile_story();
        self.has_unsaved_changes = true;
        Ok(id)
    }

    pub fn apply_writing_proposal(
        &mut self,
        proposal: &storyfella_studio_core::workflows::writing::WritingProposal,
    ) -> Result<(), String> {
        if self.operation_in_progress {
            return Err(
                "Wait for the current operation before applying a writing proposal.".into(),
            );
        }
        let project = self
            .project_path
            .as_deref()
            .ok_or("Open the proposal's project first")?;
        let path = self
            .story
            .path()
            .ok_or("Open the proposal's source file first")?;
        let source = proposal
            .apply(project, path, self.story.source())
            .map_err(|error| error.to_string())?;
        let catalog = AssetCatalog::from_registry(&self.registry);
        let analysis = analyze(&source, &catalog.assets);
        if storyfella_compiler::compile_analysis(&analysis, &catalog.bindings)
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.severity == storyfella_language::Severity::Error)
        {
            return Err(
                "The proposed source has errors. Review the diagnostics before applying.".into(),
            );
        }
        self.set_story_source(source);
        Ok(())
    }

    pub fn description_draft(&self, id: Uuid) -> Option<&str> {
        self.description_drafts.get(&id).map(String::as_str)
    }

    pub fn set_description_draft(&mut self, id: Uuid, text: String) {
        if self
            .registry
            .artifact(id)
            .is_some_and(|artifact| artifact.description == text)
        {
            self.description_drafts.remove(&id);
        } else {
            self.description_drafts.insert(id, text);
        }
    }

    pub fn discard_description_draft(&mut self, id: Uuid) {
        self.description_drafts.remove(&id);
    }

    pub fn has_description_drafts(&self) -> bool {
        !self.description_drafts.is_empty()
    }

    pub fn apply_description_drafts(&mut self) -> Result<(), String> {
        let ids: Vec<_> = self.description_drafts.keys().copied().collect();
        self.apply_descriptions(&ids)
    }

    /// Inline confirmation applies only this field, leaving other drafts alone.
    pub fn apply_description_draft(&mut self, id: Uuid) -> Result<(), String> {
        self.apply_descriptions(&[id])
    }

    fn apply_descriptions(&mut self, ids: &[Uuid]) -> Result<(), String> {
        if self.operation_in_progress {
            return Err("Wait for the current operation before editing descriptions.".into());
        }
        if self
            .description_drafts
            .iter()
            .filter(|(id, _)| ids.contains(id))
            .map(|(_, text)| text)
            .any(|text| text.trim().is_empty())
        {
            return Err(
                "Descriptions cannot be empty. Complete or discard the empty draft first.".into(),
            );
        }
        if ids
            .iter()
            .any(|id| self.description_drafts.contains_key(id))
        {
            let mut registry = self.registry.clone();
            for (id, text) in self
                .description_drafts
                .iter()
                .filter(|(id, _)| ids.contains(id))
            {
                registry
                    .set_description(*id, text.clone())
                    .map_err(|error| error.to_string())?;
            }
            self.snapshot_for_undo();
            self.registry = registry;
            self.description_drafts.retain(|id, _| !ids.contains(id));
            self.has_unsaved_changes = true;
        }
        Ok(())
    }
    /// Replaces the in-memory state with a snapshot loaded from disk.
    /// The caller is responsible for saving before navigating away.
    pub fn open_project(&mut self, stored: StoredProject, path: PathBuf) {
        self.project_id = Some(stored.id);
        self.saved_registry = stored.registry.clone();
        self.incoming_catalog = None;
        self.catalog_conflict = None;
        self.registry_version = stored.registry_version;
        self.saved_associations = stored.registry.story_scenes.clone();
        self.registry = stored.registry;
        self.project_name = stored.name;
        self.story = stored.story;
        self.additional_sources = None;
        self.active_source = None;
        self.source_notice = None;
        self.project_path = Some(path);
        self.has_unsaved_changes = false;
        self.selected_artifact_id = None;
        self.description_drafts.clear();
        self.undo_stack.clear();
        self.redo_stack.clear();
        self.reconcile_story();
    }

    pub fn selected_artifact(&self) -> Option<&Artifact> {
        let selected_id = self.selected_artifact_id?;
        self.registry.artifact(selected_id)
    }

    pub fn open_additional_source(&mut self, relative: &str) -> Result<(), StoryFileError> {
        if self.additional_sources.is_none() {
            let entry = self.story.path().ok_or(StoryFileError::Unbound)?;
            self.additional_sources = Some(AdditionalSources::new(entry)?);
        }
        let old_clean = self
            .additional_sources
            .as_ref()
            .and_then(|sources| sources.get(relative))
            .filter(|document| !document.is_dirty())
            .map(|document| document.source().to_owned());
        let document = self
            .additional_sources
            .as_mut()
            .ok_or(StoryFileError::Unbound)?
            .open(relative)?;
        let refreshed = old_clean
            .filter(|old| old != document.source())
            .map(|_| document.source().to_owned());
        if refreshed.is_some() {
            self.undo_stack.clear();
            self.redo_stack.clear();
            self.source_notice = Some(format!("{relative} changed outside Studio and was reloaded. Undo history was cleared to keep the story and assets consistent."));
            self.reconcile_story();
        }
        self.active_source = Some(relative.to_owned());
        Ok(())
    }

    pub fn create_additional_source(
        &mut self,
        path: &std::path::Path,
    ) -> Result<(), StoryFileError> {
        if self.operation_in_progress {
            return Err(StoryFileError::Unbound);
        }
        if self.additional_sources.is_none() {
            self.additional_sources = Some(AdditionalSources::new(
                self.story.path().ok_or(StoryFileError::Unbound)?,
            )?);
        }
        let relative = self
            .additional_sources
            .as_mut()
            .ok_or(StoryFileError::Unbound)?
            .create(path)?;
        let source = self.story.source();
        let newline = if source.contains("\r\n") {
            "\r\n"
        } else {
            "\n"
        };
        let separator = if source.is_empty() || source.ends_with('\n') {
            ""
        } else {
            newline
        };
        let updated = format!("{source}{separator}include \"{relative}\"{newline}");
        self.set_story_source(updated);
        self.active_source = Some(relative);
        Ok(())
    }

    pub fn forget_source_file(&mut self, relative: &str) {
        if let Some(sources) = &mut self.additional_sources {
            sources.forget(relative);
        }
        if self.active_source.as_deref() == Some(relative) {
            self.active_source = None;
        }
        // Earlier snapshots can still contain an include targeting this path.
        if !self.undo_stack.is_empty() || !self.redo_stack.is_empty() {
            self.undo_stack.clear();
            self.redo_stack.clear();
            self.source_notice = Some(
                "Undo history was cleared because a story file was renamed or removed.".into(),
            );
        }
    }

    pub fn set_additional_source(
        &mut self,
        relative: &str,
        source: String,
    ) -> Result<(), StoryFileError> {
        let current = self
            .additional_sources
            .as_ref()
            .and_then(|sources| sources.get(relative))
            .ok_or(StoryFileError::Unbound)?;
        if current.source() == source {
            return Ok(());
        }
        self.snapshot_for_undo();
        let sources = self
            .additional_sources
            .as_mut()
            .ok_or(StoryFileError::Unbound)?;
        sources.set_source(relative, source)?;
        self.has_unsaved_changes = true;
        self.reconcile_multi(true);
        Ok(())
    }

    pub fn set_story_source(&mut self, source: String) {
        if self.story.source() != source {
            self.snapshot_for_undo();
            self.story.set_source(source);
            if storyfella_language::has_includes(self.story.source()) {
                self.reconcile_multi(true);
                self.has_unsaved_changes = true;
                return;
            }
            storyfella_studio_core::catalog::reconcile_declarations(
                &mut self.registry,
                self.story.source(),
                true,
            );
            let analysis = analyze(
                self.story.source(),
                &AssetCatalog::from_registry(&self.registry).assets,
            );
            // Live typing is not an explicit EOF snapshot. Wait for a new
            // declaration's terminating newline before installing its identity.
            let unfinished_header = analysis.symbols.iter().any(|symbol| {
                let line = analysis
                    .document
                    .source
                    .position(symbol.name.span.end)
                    .unwrap()
                    .line as usize;
                let unterminated = analysis.document.source.line_span(line).unwrap().end
                    == self.story.source().len();
                let known = match symbol.kind {
                    storyfella_language::SymbolKind::Scene => self
                        .registry
                        .story_scenes
                        .iter()
                        .any(|s| s.name == symbol.name.value),
                    storyfella_language::SymbolKind::Beat => {
                        self.registry.story_scenes.iter().any(|s| {
                            symbol.scene.as_ref() == Some(&s.name)
                                && s.beats.iter().any(|b| b.name == symbol.name.value)
                        })
                    }
                    storyfella_language::SymbolKind::Flag => true,
                };
                unterminated && !known
            });
            if !unfinished_header {
                reconcile(&mut self.registry, &analysis);
            }
            self.has_unsaved_changes = true;
        }
    }

    pub fn rename_story_symbol(&mut self, offset: usize, name: &str) -> Result<(), String> {
        if storyfella_language::has_includes(self.story.source()) {
            let entry = self
                .story
                .path()
                .and_then(|path| path.file_name())
                .and_then(|name| name.to_str())
                .ok_or("Story filename is not UTF-8")?
                .to_owned();
            return self.rename_workspace_symbol(&entry, offset, name);
        }
        let analysis = analyze(
            self.story.source(),
            &AssetCatalog::from_registry(&self.registry).assets,
        );
        let edits = storyfella_language::rename(&analysis, offset, name)?;
        let source = storyfella_language::apply_edits(self.story.source(), &edits)?;
        let definition = analysis
            .definition_at(offset)
            .ok_or("Select a resolved declaration")?;
        let symbol = analysis
            .symbols
            .iter()
            .find(|s| s.name.span == definition)
            .ok_or("Select a resolved declaration")?;
        self.snapshot_for_undo();
        for scene in &mut self.registry.story_scenes {
            match symbol.kind {
                storyfella_language::SymbolKind::Scene if scene.name == symbol.name.value => {
                    scene.name = name.into()
                }
                storyfella_language::SymbolKind::Beat
                    if symbol.scene.as_ref() == Some(&scene.name) =>
                {
                    if let Some(beat) = scene.beats.iter_mut().find(|b| b.name == symbol.name.value)
                    {
                        beat.name = name.into();
                    }
                }
                _ => {}
            }
        }
        self.story.set_source(source);
        self.reconcile_story();
        self.has_unsaved_changes = true;
        Ok(())
    }

    /// Rename a resolved symbol throughout one immutable project
    /// snapshot, retaining identities and applying all draft edits as one undo.
    pub fn rename_workspace_symbol(
        &mut self,
        file: &str,
        offset: usize,
        name: &str,
    ) -> Result<(), String> {
        let workspace = self.workspace_snapshot()?;
        let expanded = workspace.expand().map_err(|error| error.message)?;
        let global_offset = expanded
            .expanded_offset(file, offset)
            .ok_or("Select a resolved symbol in the story")?;
        let catalog = AssetCatalog::from_registry(&self.registry);
        let analysis = analyze(&expanded.source, &catalog.assets);
        if analysis.has_errors() {
            return Err("Fix story errors before renaming across files.".into());
        }
        let definition = analysis
            .definition_at(global_offset)
            .ok_or("Select a resolved symbol")?;
        let symbol = analysis
            .symbols
            .iter()
            .find(|symbol| symbol.name.span == definition)
            .ok_or("Select a resolved declaration")?
            .clone();
        let edits = storyfella_language::rename(&analysis, global_offset, name)?;
        let mut registry = self.registry.clone();
        for scene in &mut registry.story_scenes {
            match symbol.kind {
                storyfella_language::SymbolKind::Scene if scene.name == symbol.name.value => {
                    scene.name = name.into()
                }
                storyfella_language::SymbolKind::Beat
                    if symbol.scene.as_ref() == Some(&scene.name) =>
                {
                    if let Some(beat) = scene
                        .beats
                        .iter_mut()
                        .find(|beat| beat.name == symbol.name.value)
                    {
                        beat.name = name.into();
                    }
                }
                _ => {}
            }
        }
        self.commit_workspace_edits(workspace, expanded, edits, registry)
    }

    fn workspace_snapshot(&self) -> Result<storyfella_compiler::SourceWorkspace, String> {
        let path = self.story.path().ok_or("Open a story first")?;
        let entry = path
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or("Story filename is not UTF-8")?;
        let mut drafts = self
            .additional_sources
            .as_ref()
            .map_or_else(Default::default, AdditionalSources::overrides);
        drafts.insert(entry.to_owned(), self.story.source().to_owned());
        storyfella_studio_core::story::load_source_workspace(path, &drafts)
            .map_err(|error| error.to_string())
    }

    fn rename_workspace_asset(
        &mut self,
        id: Uuid,
        name: &str,
        registry: ArtifactRegistry,
    ) -> Result<(), String> {
        let workspace = self.workspace_snapshot()?;
        let expanded = workspace.expand().map_err(|error| error.message)?;
        let catalog = AssetCatalog::from_registry(&self.registry);
        let analysis = analyze(&expanded.source, &catalog.assets);
        if analysis.has_errors() {
            return Err("Fix story errors before renaming an asset across files.".into());
        }
        let id = id.to_string();
        let edits = analysis
            .references
            .iter()
            .filter(|reference| reference.asset_id.as_deref() == Some(id.as_str()))
            .map(|reference| storyfella_language::TextEdit {
                replace: reference.name.span,
                new_text: name.into(),
            })
            .collect();
        self.commit_workspace_edits(workspace, expanded, edits, registry)
    }

    fn commit_workspace_edits(
        &mut self,
        mut workspace: storyfella_compiler::SourceWorkspace,
        expanded: storyfella_compiler::ExpandedWorkspace,
        edits: Vec<storyfella_language::TextEdit>,
        registry: ArtifactRegistry,
    ) -> Result<(), String> {
        let entry_path = self.story.path().ok_or("Open a story first")?.to_owned();
        let entry = workspace.entry.clone();
        let mut groups =
            std::collections::BTreeMap::<String, Vec<storyfella_language::TextEdit>>::new();
        for edit in edits {
            let (path, span) = expanded
                .original_span(edit.replace)
                .ok_or("A rename crosses file boundaries")?;
            groups
                .entry(path.to_owned())
                .or_default()
                .push(storyfella_language::TextEdit {
                    replace: span,
                    new_text: edit.new_text,
                });
        }
        let mut before = match &self.additional_sources {
            Some(sources) => sources.clone(),
            None => AdditionalSources::new(&entry_path).map_err(|error| error.to_string())?,
        };
        for (path, edits) in &groups {
            let original = workspace
                .files
                .get(path)
                .ok_or("Source disappeared from the snapshot")?;
            if path != &entry {
                let document = before.open(path).map_err(|error| error.to_string())?;
                if document.source() != original {
                    return Err(format!("{path} changed while preparing the rename"));
                }
                if self
                    .additional_sources
                    .as_ref()
                    .and_then(|sources| sources.get(path))
                    .is_some_and(|old| !old.is_dirty() && old.source() != document.source())
                {
                    return Err(format!(
                        "{path} changed outside Studio. Open it to refresh before renaming."
                    ));
                }
            }
            let updated = storyfella_language::apply_edits(original, edits)?;
            workspace.files.insert(path.clone(), updated);
        }
        let updated = workspace.expand().map_err(|error| error.message)?;
        if analyze(
            &updated.source,
            &AssetCatalog::from_registry(&registry).assets,
        )
        .has_errors()
        {
            return Err("The rename would make the story invalid.".into());
        }
        let mut after = before.clone();
        for path in groups.keys().filter(|path| *path != &entry) {
            after
                .set_source(path, workspace.files[path].clone())
                .map_err(|error| error.to_string())?;
        }
        self.additional_sources = Some(before);
        self.snapshot_for_undo();
        self.additional_sources = Some(after);
        self.story.set_source(workspace.files[&entry].clone());
        self.registry = registry;
        self.reconcile_story();
        self.has_unsaved_changes = true;
        Ok(())
    }

    pub fn reconcile_story(&mut self) {
        if storyfella_language::has_includes(self.story.source()) {
            self.reconcile_multi(false);
            return;
        }
        if storyfella_studio_core::catalog::reconcile_declarations(
            &mut self.registry,
            self.story.source(),
            false,
        ) {
            self.has_unsaved_changes = true;
        }
        let analysis = analyze(
            self.story.source(),
            &AssetCatalog::from_registry(&self.registry).assets,
        );
        if reconcile(&mut self.registry, &analysis) {
            self.has_unsaved_changes = true;
        }
    }

    fn reconcile_multi(&mut self, live: bool) {
        let Some(path) = self.story.path() else {
            return;
        };
        let Some(entry) = path.file_name().and_then(|name| name.to_str()) else {
            return;
        };
        let mut drafts = self
            .additional_sources
            .as_ref()
            .map_or_else(Default::default, AdditionalSources::overrides);
        drafts.insert(entry.to_owned(), self.story.source().to_owned());
        if live {
            let active = self.active_source.as_deref().unwrap_or(entry);
            if drafts
                .get(active)
                .is_some_and(|source| !source.is_empty() && !source.ends_with('\n'))
            {
                return;
            }
        }
        let Ok(workspace) = storyfella_studio_core::story::load_source_workspace(path, &drafts)
        else {
            return;
        };
        let Ok(expanded) = workspace.expand() else {
            return;
        };
        if storyfella_studio_core::catalog::reconcile_declarations(
            &mut self.registry,
            &expanded.source,
            live,
        ) {
            self.has_unsaved_changes = true;
        }
        let analysis = analyze(
            &expanded.source,
            &AssetCatalog::from_registry(&self.registry).assets,
        );
        if reconcile(&mut self.registry, &analysis) {
            self.has_unsaved_changes = true;
        }
    }

    /// Create the missing catalog entry immediately; artwork is made in Atelier.
    /// Recheck shared resolution to reject collisions and wrong kinds.
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
        self.description_drafts.retain(|id, draft| {
            self.registry
                .artifact(*id)
                .is_some_and(|artifact| artifact.description != *draft)
        });
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

    pub fn mark_saved(&mut self, registry_version: i64) {
        self.registry_version = registry_version;
        self.saved_registry = self.registry.clone();
        self.saved_associations = self.registry.story_scenes.clone();
        self.has_unsaved_changes = false;
    }

    /// Refresh catalog fields without replacing source, inspector drafts or an
    /// active generation. Every undo state receives the same external delta.
    pub fn refresh_catalog(
        &mut self,
        stored: StoredProject,
        resolution: Resolution,
    ) -> Result<(), String> {
        if self.project_id != Some(stored.id) {
            return Err(
                "The project identity changed on disk. Open the intended project again.".into(),
            );
        }
        if self.operation_in_progress {
            return Ok(());
        }
        if stored.registry_version < self.registry_version {
            return Ok(());
        }
        let mut with_drafts = self.registry.clone();
        for (id, text) in &self.description_drafts {
            if let Some(item) = with_drafts.artifact_mut(*id) {
                item.description = text.clone();
            }
        }
        // Inspector drafts participate in conflict detection even though normal
        // autosave must leave them unapplied and editable.
        let preview = sync::merge(
            &self.saved_registry,
            &with_drafts,
            &stored.registry,
            resolution,
        );
        match preview {
            Ok(_) => {}
            Err(conflict) => {
                let message = conflict.to_string();
                self.catalog_conflict = Some(conflict);
                self.incoming_catalog = Some(stored);
                return Err(message);
            }
        }
        let mut merged = sync::merge(
            &self.saved_registry,
            &self.registry,
            &stored.registry,
            resolution,
        )
        .map_err(|error| error.to_string())?;
        self.description_drafts.retain(|id, text| {
            let before = self.saved_registry.artifact(*id).map(|a| &a.description);
            let saved = stored.registry.artifact(*id).map(|a| &a.description);
            if saved == Some(text) {
                return false;
            }
            if resolution != Resolution::Check && before != saved {
                if resolution == Resolution::Local {
                    if let Some(item) = merged.artifact_mut(*id) {
                        item.description = text.clone();
                    }
                }
                return false;
            }
            true
        });
        for stack in [&mut self.undo_stack, &mut self.redo_stack] {
            // A structurally incompatible historical snapshot cannot safely be
            // restored. Keep the other undo steps, including their source text.
            stack.retain_mut(|snapshot| {
                match sync::merge(
                    &self.saved_registry,
                    &snapshot.registry,
                    &stored.registry,
                    Resolution::Saved,
                ) {
                    Ok(registry) => {
                        snapshot.registry = registry;
                        true
                    }
                    Err(_) => false,
                }
            });
        }
        self.registry = merged;
        self.project_name = stored.name;
        self.registry_version = stored.registry_version;
        self.saved_associations = stored.registry.story_scenes.clone();
        self.saved_registry = stored.registry;
        // External catalog changes never request a source edit. A clean source
        // may follow its owner on disk; a dirty buffer keeps its conflict guard.
        let source_changed = !self.story.is_dirty() && self.story.source() != stored.story.source();
        if source_changed {
            self.story = stored.story;
        }
        self.catalog_conflict = None;
        self.incoming_catalog = None;
        self.prune_selection();
        if source_changed {
            self.reconcile_story();
        }
        self.has_unsaved_changes = self.story.is_dirty()
            || self
                .additional_sources
                .as_ref()
                .is_some_and(AdditionalSources::is_dirty)
            || self.registry != self.saved_registry;
        Ok(())
    }

    pub fn resolve_catalog_conflict(&mut self, resolution: Resolution) -> Result<(), String> {
        if self.operation_in_progress {
            return Err("Wait for the current operation before resolving catalog changes.".into());
        }
        let stored = self
            .incoming_catalog
            .clone()
            .ok_or("Refresh the catalog first")?;
        self.refresh_catalog(stored, resolution)
    }

    /// Explicit discard for a duplicate identity or broken relationship. The
    /// confirmation names the catalog/undo loss; authored text and drafts stay.
    pub fn use_saved_catalog(&mut self) -> Result<(), String> {
        if self.operation_in_progress {
            return Err("Wait for the current operation before reloading the catalog.".into());
        }
        let stored = self
            .incoming_catalog
            .clone()
            .ok_or("Refresh the catalog first")?;
        self.registry = stored.registry.clone();
        self.saved_registry = stored.registry.clone();
        self.description_drafts
            .retain(|id, _| self.registry.artifact(*id).is_some());
        self.undo_stack.clear();
        self.redo_stack.clear();
        self.refresh_catalog(stored, Resolution::Saved)
    }

    fn authoring_snapshot(&self) -> AuthoringSnapshot {
        AuthoringSnapshot {
            registry: self.registry.clone(),
            source: self.story.source().to_owned(),
            additional: self
                .additional_sources
                .as_ref()
                .map_or_else(Default::default, AdditionalSources::drafts),
        }
    }

    /// Selection is validated: ids that do not exist are ignored.
    pub fn select_artifact(&mut self, artifact_id: Uuid) {
        if self.registry.artifact(artifact_id).is_some() {
            self.selected_artifact_id = Some(artifact_id);
        }
    }

    /// Drops the selection, which is what returns Assets to the grid.
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
        if let Some(sources) = &mut self.additional_sources {
            sources.restore_drafts(&snapshot.additional);
        }
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
        if let Some(sources) = &mut self.additional_sources {
            sources.restore_drafts(&snapshot.additional);
        }
        self.prune_selection();
        self.has_unsaved_changes = true;
        true
    }

    /// The image to show for an artifact: the active revision's asset when
    /// it has one, otherwise the latest completed revision.
    pub fn display_image(artifact: &Artifact) -> Option<&str> {
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

    #[test]
    fn undo_after_clean_external_refresh_does_not_restore_old_chapter_text() {
        let directory = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../.svs-e2e/tests")
            .join(format!("studio-rebase-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&directory).unwrap();
        let entry = directory.join("story.sf");
        let chapter = directory.join("chapter.sf");
        std::fs::write(&entry, "include \"chapter.sf\"\n").unwrap();
        std::fs::write(&chapter, "scene old\n  end\n").unwrap();
        let mut state = AppState {
            story: StoryDocument::open(&entry).unwrap(),
            ..Default::default()
        };
        state.open_additional_source("chapter.sf").unwrap();
        state.active_source = None;
        state.set_story_source("# note\ninclude \"chapter.sf\"\n".into());
        std::fs::write(&chapter, "scene external\n  end\n").unwrap();
        state.open_additional_source("chapter.sf").unwrap();
        assert!(!state.undo());
        let document = state
            .additional_sources
            .as_ref()
            .unwrap()
            .get("chapter.sf")
            .unwrap();
        assert_eq!(document.source(), "scene external\n  end\n");
        assert!(!document.is_dirty());
        assert!(state.source_notice.is_some());
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn rename_scene_across_files_is_one_undo_and_preserves_identity() {
        let directory = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../.svs-e2e/tests")
            .join(format!("studio-rename-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&directory).unwrap();
        let entry = directory.join("story.sf");
        std::fs::write(&entry, "include \"first.sf\"\ninclude \"last.sf\"\n").unwrap();
        std::fs::write(directory.join("first.sf"), "scene start\n  goto ending\n").unwrap();
        std::fs::write(directory.join("last.sf"), "scene ending\n  end\n").unwrap();
        let mut state = AppState {
            story: StoryDocument::open(&entry).unwrap(),
            ..Default::default()
        };
        state.reconcile_story();
        let id = state
            .registry
            .story_scenes
            .iter()
            .find(|scene| scene.name == "ending")
            .unwrap()
            .artifact_id;
        state.open_additional_source("first.sf").unwrap();
        state
            .rename_workspace_symbol("first.sf", "scene start\n  goto ".len(), "finale")
            .unwrap();
        let sources = state.additional_sources.as_ref().unwrap();
        assert!(sources
            .get("first.sf")
            .unwrap()
            .source()
            .contains("goto finale"));
        assert!(sources
            .get("last.sf")
            .unwrap()
            .source()
            .contains("scene finale"));
        assert_eq!(
            state
                .registry
                .story_scenes
                .iter()
                .find(|scene| scene.name == "finale")
                .unwrap()
                .artifact_id,
            id
        );
        assert!(state.undo());
        let sources = state.additional_sources.as_ref().unwrap();
        assert!(sources
            .get("first.sf")
            .unwrap()
            .source()
            .contains("goto ending"));
        assert!(sources
            .get("last.sf")
            .unwrap()
            .source()
            .contains("scene ending"));
        assert!(state.redo());
        assert!(state
            .additional_sources
            .as_ref()
            .unwrap()
            .get("last.sf")
            .unwrap()
            .source()
            .contains("scene finale"));
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn rename_asset_updates_all_files_and_undo_restores_every_reference() {
        let directory = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../.svs-e2e/tests")
            .join(format!("studio-asset-rename-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&directory).unwrap();
        let entry = directory.join("story.sf");
        std::fs::write(&entry, "include \"first.sf\"\ninclude \"last.sf\"\n").unwrap();
        std::fs::write(
            directory.join("first.sf"),
            "scene start\n  cast: mara\n  mara: Hello\n  goto finish\n",
        )
        .unwrap();
        std::fs::write(
            directory.join("last.sf"),
            "scene finish\n  mara: Goodbye\n  end\n",
        )
        .unwrap();
        let mut state = AppState {
            story: StoryDocument::open(&entry).unwrap(),
            ..Default::default()
        };
        state.reconcile_story();
        let id = state
            .registry
            .artifacts
            .iter()
            .find(|item| item.name == "mara")
            .unwrap()
            .id;
        state
            .edit_catalog(storyfella_studio_core::catalog::CatalogEdit::Describe {
                id,
                name: "captain".into(),
                description: "A sailor".into(),
            })
            .unwrap();
        let sources = state.additional_sources.as_ref().unwrap();
        assert!(sources
            .get("first.sf")
            .unwrap()
            .source()
            .contains("cast: captain\n  captain: Hello"));
        assert!(sources
            .get("last.sf")
            .unwrap()
            .source()
            .contains("captain: Goodbye"));
        assert_eq!(state.registry.artifact(id).unwrap().name, "captain");
        assert!(state.undo());
        assert_eq!(state.registry.artifact(id).unwrap().name, "mara");
        assert!(state
            .additional_sources
            .as_ref()
            .unwrap()
            .get("last.sf")
            .unwrap()
            .source()
            .contains("mara: Goodbye"));
        assert!(state.redo());
        assert_eq!(state.registry.artifact(id).unwrap().name, "captain");
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn multi_file_reconciliation_keeps_scene_and_asset_identities_on_invalid_drafts() {
        let directory = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../.svs-e2e/tests")
            .join(format!("studio-catalog-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&directory).unwrap();
        let entry = directory.join("story.sf");
        std::fs::write(&entry, "include \"chapter.sf\"\n").unwrap();
        std::fs::write(directory.join("chapter.sf"), "scene start\n  end\n").unwrap();
        let mut state = AppState {
            story: StoryDocument::open(&entry).unwrap(),
            ..Default::default()
        };
        state.open_additional_source("chapter.sf").unwrap();
        state
            .set_additional_source("chapter.sf", "scene start\n  cast: mara\n  end\n".into())
            .unwrap();
        let scene = state.registry.story_scenes[0].artifact_id;
        let character = state
            .registry
            .artifacts
            .iter()
            .find(|item| item.name == "mara")
            .unwrap()
            .id;
        state
            .set_additional_source(
                "chapter.sf",
                "scene start\n  cast: mara\n  goto missing\n".into(),
            )
            .unwrap();
        assert_eq!(state.registry.story_scenes[0].artifact_id, scene);
        assert_eq!(
            state
                .registry
                .artifacts
                .iter()
                .find(|item| item.name == "mara")
                .unwrap()
                .id,
            character
        );
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn creating_chapter_links_it_without_overwriting_and_preserves_line_endings() {
        let directory = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../.svs-e2e/tests")
            .join(format!("studio-create-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&directory).unwrap();
        let entry = directory.join("story.sf");
        std::fs::write(&entry, "scene first\r\n  end\r\n").unwrap();
        let mut state = AppState {
            story: StoryDocument::open(&entry).unwrap(),
            ..Default::default()
        };
        state
            .create_additional_source(&directory.join("chapter.sf"))
            .unwrap();
        assert_eq!(
            state.story.source(),
            "scene first\r\n  end\r\ninclude \"chapter.sf\"\r\n"
        );
        assert_eq!(state.active_source.as_deref(), Some("chapter.sf"));
        assert!(state.has_unsaved_changes);
        assert!(state
            .create_additional_source(&directory.join("chapter.sf"))
            .is_err());
        std::fs::remove_dir_all(directory).unwrap();
    }

    fn sync_state() -> (AppState, storyfella_studio_core::persistence::StoredProject) {
        let stored = storyfella_studio_core::persistence::StoredProject {
            registry_version: 1,
            id: uuid::Uuid::new_v4(),
            name: "Shared".into(),
            registry: character_registry("mara"),
            story: StoryDocument::default(),
        };
        let mut state = AppState::default();
        state.open_project(stored.clone(), "shared.db".into());
        (state, stored)
    }

    #[test]
    fn catalog_refresh_keeps_dirty_source_and_undo_cannot_remove_external_artwork() {
        use super::Resolution;
        let (mut state, mut stored) = sync_state();
        let id = state.registry.artifacts[0].id;
        state.set_story_source("// unsaved 🌊\r\n".into());
        state
            .edit_catalog(storyfella_studio_core::catalog::CatalogEdit::Describe {
                id,
                name: "mara".into(),
                description: "Local details".into(),
            })
            .unwrap();
        let revision = stored
            .registry
            .start_revision(
                id,
                "Artwork".into(),
                None,
                None,
                storyfella_studio_core::registry::RevisionOrigin::Canon,
            )
            .unwrap();
        stored
            .registry
            .finish_revision(id, revision, "/image.png".into())
            .unwrap();
        let added = stored
            .registry
            .create_artifact(
                ArtifactKind::Object,
                CreateArtifact {
                    name: "lamp".into(),
                    ..Default::default()
                },
            )
            .unwrap();
        stored.registry_version += 1;
        state
            .refresh_catalog(stored.clone(), Resolution::Check)
            .unwrap();
        assert_eq!(state.story.source(), "// unsaved 🌊\r\n");
        assert_eq!(
            state.registry.artifact(id).unwrap().description,
            "Local details"
        );
        assert_eq!(
            state.registry.artifact(id).unwrap().active_revision_id,
            Some(revision)
        );
        assert!(state.has_unsaved_changes);
        while state.undo() {
            assert_eq!(
                state.registry.artifact(id).unwrap().active_revision_id,
                Some(revision)
            );
            assert!(state.registry.artifact(added).is_some());
        }
        while state.redo() {
            assert_eq!(
                state.registry.artifact(id).unwrap().active_revision_id,
                Some(revision)
            );
            assert!(state.registry.artifact(added).is_some());
        }
        assert_eq!(state.story.source(), "// unsaved 🌊\r\n");
    }

    #[test]
    fn unapplied_description_drafts_conflict_before_autosave_and_require_an_explicit_choice() {
        use super::Resolution;
        for resolution in [Resolution::Saved, Resolution::Local] {
            let (mut state, mut stored) = sync_state();
            let id = state.registry.artifacts[0].id;
            state.set_description_draft(id, "My retained draft".into());
            stored.registry.artifact_mut(id).unwrap().description = "Atelier details".into();
            stored.registry_version += 1;
            let original = state.registry.clone();
            assert!(state
                .refresh_catalog(stored.clone(), Resolution::Check)
                .is_err());
            assert_eq!(state.registry, original);
            assert_eq!(state.description_draft(id), Some("My retained draft"));
            assert!(state.catalog_conflict.is_some());
            state.resolve_catalog_conflict(resolution).unwrap();
            assert!(state.catalog_conflict.is_none());
            assert!(!state.has_description_drafts());
            assert_eq!(
                state.registry.artifact(id).unwrap().description,
                if resolution == Resolution::Local {
                    "My retained draft"
                } else {
                    "Atelier details"
                }
            );
            assert_eq!(state.has_unsaved_changes, resolution == Resolution::Local);
        }
    }

    #[test]
    fn refresh_waits_for_generation_rejects_another_project_and_ignores_old_results() {
        use super::Resolution;
        let (mut state, mut stored) = sync_state();
        stored.registry_version += 1;
        stored.registry.artifacts[0].description = "External".into();
        state.begin_operation();
        let before = state.clone();
        assert!(state.resolve_catalog_conflict(Resolution::Saved).is_err());
        assert!(state.use_saved_catalog().is_err());
        assert_eq!(state, before);
        state
            .refresh_catalog(stored.clone(), Resolution::Check)
            .unwrap();
        assert_eq!(state, before);
        state.cancel_operation();
        state
            .refresh_catalog(stored.clone(), Resolution::Check)
            .unwrap();
        let before = state.clone();
        stored.registry_version -= 1;
        stored.registry.artifacts[0].description = "Obsolete".into();
        state
            .refresh_catalog(stored.clone(), Resolution::Check)
            .unwrap();
        assert_eq!(state, before);
        stored.id = uuid::Uuid::new_v4();
        assert!(state.refresh_catalog(stored, Resolution::Check).is_err());
        assert_eq!(state, before);
    }

    #[test]
    fn duplicate_creation_recovery_uses_saved_identity_without_rewriting_source() {
        use super::Resolution;
        let (mut state, mut stored) = sync_state();
        state.set_story_source("// keep my writing\n".into());
        state
            .edit_catalog(storyfella_studio_core::catalog::CatalogEdit::Create {
                kind: ArtifactKind::Object,
                name: "lamp".into(),
                description: String::new(),
                variant_of: None,
            })
            .unwrap();
        let saved_id = stored
            .registry
            .create_artifact(
                ArtifactKind::Object,
                CreateArtifact {
                    name: "lamp".into(),
                    ..Default::default()
                },
            )
            .unwrap();
        stored.registry_version += 1;
        assert!(state.refresh_catalog(stored, Resolution::Check).is_err());
        assert!(state.catalog_conflict.as_ref().unwrap().requires_reload);
        state.use_saved_catalog().unwrap();
        assert_eq!(state.story.source(), "// keep my writing\n");
        assert_eq!(
            state
                .registry
                .artifacts
                .iter()
                .filter(|a| a.name == "lamp")
                .count(),
            1
        );
        assert!(state.registry.artifact(saved_id).is_some());
        assert!(state.has_unsaved_changes);
        assert!(!state.can_undo());
    }
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

    #[test]
    fn writing_acceptance_is_one_undo_and_refuses_concurrent_or_invalid_edits() {
        use storyfella_studio_core::workflows::writing::{
            WritingProposal, WritingRequest, WritingScope,
        };
        let directory = std::env::temp_dir().join(format!("writing-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&directory).unwrap();
        let project = directory.join("story.db");
        let path = directory.join("story.sf");
        let original = "scene harbor\n  beat arrival\n    narrate: The sea is quiet.\n    end\n";
        std::fs::write(&path, original).unwrap();
        let mut state = AppState {
            project_path: Some(project.clone()),
            story: StoryDocument::open(&path).unwrap(),
            ..Default::default()
        };
        state.reconcile_story();
        let request = WritingRequest::capture(
            &project,
            state.story.path().unwrap(),
            original,
            WritingScope::Beat(40),
            &state.registry,
        )
        .unwrap();
        let proposal = WritingProposal {
            run_id: "test".into(),
            replacement: request.original().replace("quiet", "restless"),
            request,
        };
        let registry = state.registry.clone();
        state.apply_writing_proposal(&proposal).unwrap();
        assert!(state.story.source().contains("restless"));
        assert_eq!(state.undo_stack.len(), 1);
        assert!(state.has_unsaved_changes);
        state.undo();
        assert_eq!(state.story.source(), original);
        assert_eq!(state.registry, registry);
        let mut invalid = proposal.clone();
        invalid.replacement = "scene\n".into();
        assert!(state.apply_writing_proposal(&invalid).is_err());
        state.set_story_source(format!("{original}# new edit\n"));
        let before = state.clone();
        assert!(state.apply_writing_proposal(&proposal).is_err());
        assert_eq!(state, before);
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
    fn description_drafts_survive_selection_and_operations_until_applied() {
        let mut state = state_with("mia");
        let id = state.registry.artifacts[0].id;
        state.set_description_draft(id, "A new canonical description 🌊".into());
        state.clear_selection();
        assert_eq!(
            state.description_draft(id),
            Some("A new canonical description 🌊")
        );
        assert_eq!(
            state.registry.artifact(id).unwrap().description,
            "A test character"
        );
        let baseline = state.registry.clone();
        state.finish_operation(&baseline, baseline.clone());
        assert!(state.has_description_drafts());
        state.apply_description_drafts().unwrap();
        assert!(!state.has_description_drafts());
        assert!(state.has_unsaved_changes);
        assert_eq!(
            state.registry.artifact(id).unwrap().description,
            "A new canonical description 🌊"
        );
        state.undo();
        assert_eq!(
            state.registry.artifact(id).unwrap().description,
            "A test character"
        );
    }

    #[test]
    fn confirming_one_description_leaves_other_drafts_and_artwork_untouched() {
        let mut state = state_with("mia");
        let id = state.registry.artifacts[0].id;
        let revision = state
            .registry
            .start_revision(
                id,
                "Existing artwork".into(),
                None,
                None,
                storyfella_studio_core::registry::RevisionOrigin::Canon,
            )
            .unwrap();
        state
            .registry
            .finish_revision(id, revision, "/artwork/mia.png".into())
            .unwrap();
        let image = state.registry.artifact(id).unwrap().revisions.clone();
        let missing = uuid::Uuid::from_u128(u128::MAX);
        state.set_description_draft(missing, "Another unfinished field".into());
        state.set_description_draft(id, "Confirmed description\nSecond line".into());
        state.apply_description_draft(id).unwrap();
        let item = state.registry.artifact(id).unwrap();
        assert_eq!(item.description, "Confirmed description\nSecond line");
        assert_eq!(item.revisions, image);
        assert_eq!(item.active_revision_id, Some(revision));
        assert!(state.description_draft(id).is_none());
        assert_eq!(
            state.description_draft(missing),
            Some("Another unfinished field")
        );
        state.undo();
        assert_eq!(
            state.registry.artifact(id).unwrap().description,
            "A test character"
        );
        assert_eq!(state.registry.artifact(id).unwrap().revisions, image);
        state.set_description_draft(id, " ".into());
        let registry = state.registry.clone();
        assert!(state.apply_description_draft(id).is_err());
        assert_eq!(state.registry, registry);
        assert_eq!(state.description_draft(id), Some(" "));
        state.discard_description_draft(id);
        assert!(state.description_draft(id).is_none());
        assert!(state.description_draft(missing).is_some());
    }

    #[test]
    fn a_missing_artifact_keeps_all_description_drafts_unapplied() {
        let mut state = state_with("mia");
        let id = state.registry.artifacts[0].id;
        let missing = uuid::Uuid::from_u128(u128::MAX);
        state.set_description_draft(id, "Pending description".into());
        state.set_description_draft(missing, "Removed artifact draft".into());
        let registry = state.registry.clone();
        let undo_count = state.undo_stack.len();
        let dirty = state.has_unsaved_changes;
        assert!(state.apply_description_drafts().is_err());
        assert_eq!(state.registry, registry);
        assert_eq!(state.undo_stack.len(), undo_count);
        assert_eq!(state.has_unsaved_changes, dirty);
        assert_eq!(state.description_draft(id), Some("Pending description"));
        assert_eq!(
            state.description_draft(missing),
            Some("Removed artifact draft")
        );
    }

    #[test]
    fn empty_description_cannot_be_applied_or_lost_by_failed_apply() {
        let mut state = state_with("mia");
        let id = state.registry.artifacts[0].id;
        state.set_description_draft(id, " ".into());
        assert!(state.apply_description_drafts().is_err());
        assert_eq!(state.description_draft(id), Some(" "));
        assert_eq!(
            state.registry.artifact(id).unwrap().description,
            "A test character"
        );
        state.discard_description_draft(id);
        assert!(!state.has_description_drafts());
    }

    #[test]
    fn structural_edits_and_declarations_undo_source_and_registry_together() {
        let mut state = AppState::default();
        let source = "scene harbor\n  beat arrival\n    prop: lantern\n";
        state.set_story_source(source.into());
        let before = state.registry.clone();
        let lantern = state
            .registry
            .artifacts
            .iter()
            .find(|a| a.name == "lantern")
            .unwrap()
            .id;
        assert!(state
            .create_pending_asset("lantern", storyfella_language::AssetKind::Prop)
            .is_none());
        assert_eq!(
            state.registry.artifact(lantern).unwrap().kind,
            ArtifactKind::Object
        );
        assert!(state.undo());
        assert!(state.registry.artifacts.is_empty());
        assert_eq!(state.story.source(), "");
        assert!(state.redo());
        assert_eq!(state.registry, before);
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
    fn catalog_rename_keeps_identity_and_changes_only_resolved_source_references_in_one_undo() {
        use storyfella_studio_core::catalog::CatalogEdit;
        let mut state = AppState::default();
        let source = "scene harbor\r\n  cast: mara, tam\r\n  beat arrival\r\n    mara: mara is my old name.\r\n    tam: Hello mara\r\n";
        state.set_story_source(source.into());
        let id = state
            .registry
            .artifacts
            .iter()
            .find(|a| a.name == "mara")
            .unwrap()
            .id;
        let before = state.registry.clone();
        state
            .edit_catalog(CatalogEdit::Describe {
                id,
                name: "captain_mara".into(),
                description: "Captain".into(),
            })
            .unwrap();
        assert_eq!(state.registry.artifact(id).unwrap().name, "captain_mara");
        assert_eq!(state.story.source(), "scene harbor\r\n  cast: captain_mara, tam\r\n  beat arrival\r\n    captain_mara: mara is my old name.\r\n    tam: Hello mara\r\n");
        assert_eq!(state.registry.artifacts.len(), before.artifacts.len());
        assert!(state.undo());
        assert_eq!(state.registry, before);
        assert_eq!(state.story.source(), source);
        assert!(state.redo());
        assert_eq!(state.registry.artifact(id).unwrap().name, "captain_mara");
    }

    #[test]
    fn explicit_symbol_rename_keeps_artwork_identity_and_undoes_both_representations() {
        let mut state = AppState::default();
        state.set_story_source(
            "scene harbor\n  beat arrival\n    narrate: harbor\nscene sea\n  goto harbor\n".into(),
        );
        let before = state.registry.clone();
        state.rename_story_symbol(7, "port").unwrap();
        assert_eq!(
            state.registry.story_scenes[0].artifact_id,
            before.story_scenes[0].artifact_id
        );
        assert_eq!(
            state.registry.story_scenes[0].beats,
            before.story_scenes[0].beats
        );
        assert!(state.story.source().contains("narrate: harbor"));
        assert!(state.story.source().ends_with("goto port\n"));
        assert!(state.undo());
        assert_eq!(state.registry, before);
        assert!(state.story.source().starts_with("scene harbor"));
    }

    #[test]
    fn typing_declaration_prefixes_waits_for_a_complete_line_or_explicit_snapshot() {
        let mut state = AppState::default();
        let source = "scene harbor\n  beat arrival\n";
        for (offset, ch) in source.char_indices() {
            state.set_story_source(source[..offset + ch.len_utf8()].into());
        }
        assert_eq!(state.registry.artifacts.len(), 2);
        assert_eq!(state.registry.story_scenes[0].name, "harbor");
        assert_eq!(state.registry.story_scenes[0].beats[0].name, "arrival");
        state.set_story_source("scene sea".into());
        assert_eq!(state.registry.story_scenes[0].name, "harbor");
        state.reconcile_story();
        assert_eq!(state.registry.story_scenes[0].name, "sea");
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
            registry_version: 0,
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
            AppState::display_image(artifact),
            Some("assets/generated/second.png"),
            "the latest completed revision is the display image"
        );

        // Studio only observes this selection, which is saved by the artwork owner.
        state.registry.artifact_mut(mia).unwrap().active_revision_id = Some(first);
        let artifact = state.registry.artifact(mia).expect("artifact exists");
        assert_eq!(
            AppState::display_image(artifact),
            Some("assets/generated/first.png"),
            "activating an older revision changes the display image"
        );
    }
}
