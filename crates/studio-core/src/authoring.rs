//! Source-to-artwork associations and the host catalog, independent of the GUI.
use serde::{Deserialize, Serialize};
use storyfella_compiler::AssetBinding;
use storyfella_language::{Analysis, Asset, AssetKind, SymbolKind};
use uuid::Uuid;

use crate::registry::{
    ops::unique_name, ArtifactKind, ArtifactRegistry, CreateArtifact, RevisionStatus,
};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BeatAssociation {
    pub name: String,
    pub artifact_id: Uuid,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SceneAssociation {
    pub name: String,
    pub artifact_id: Uuid,
    pub beats: Vec<BeatAssociation>,
}

/// One kind mapping for analysis and exact compiler revision bindings.
pub struct AssetCatalog {
    pub assets: Vec<Asset>,
    pub bindings: Vec<AssetBinding>,
}

pub fn artifact_kind(kind: AssetKind) -> ArtifactKind {
    match kind {
        AssetKind::Character => ArtifactKind::Character,
        AssetKind::Location => ArtifactKind::Environment,
        AssetKind::Prop => ArtifactKind::Object,
    }
}

impl AssetCatalog {
    pub fn from_registry(registry: &ArtifactRegistry) -> Self {
        let mut assets = Vec::new();
        let mut bindings = Vec::new();
        for artifact in &registry.artifacts {
            let kind = match artifact.kind {
                ArtifactKind::Character => AssetKind::Character,
                ArtifactKind::Environment => AssetKind::Location,
                ArtifactKind::Object => AssetKind::Prop,
                _ => continue,
            };
            assets.push(Asset {
                id: artifact.id.to_string(),
                name: artifact.name.clone(),
                kind,
            });
            // A queued/failed active revision stays pending. Do not silently
            // bind the latest completed image in place of the active revision.
            if let Some(revision) = artifact.revisions.iter().find(|revision| {
                Some(revision.id) == artifact.active_revision_id
                    && revision.status == RevisionStatus::Completed
                    && revision.asset_path.is_some()
            }) {
                bindings.push(AssetBinding {
                    id: artifact.id.to_string(),
                    name: artifact.name.clone(),
                    kind,
                    revision: revision.id.to_string(),
                });
            }
        }
        Self { assets, bindings }
    }
}

/// Replace the association only on an error-free snapshot, including empty
/// source. Removed declarations detach artwork; never delete revisions. Identity
/// is exactly scene name / (scene, beat name), not position or nearby dialogue.
pub fn reconcile(registry: &mut ArtifactRegistry, analysis: &Analysis) -> bool {
    if analysis.has_errors() {
        return false;
    }
    let previous = registry.story_scenes.clone();
    let mut scenes = Vec::new();
    for symbol in analysis
        .symbols
        .iter()
        .filter(|s| s.kind == SymbolKind::Scene)
    {
        let old = previous.iter().find(|s| s.name == symbol.name.value);
        let scene_id = old
            .map(|s| s.artifact_id)
            .unwrap_or_else(|| create(registry, ArtifactKind::Scene, &symbol.name.value, None));
        let mut beats = Vec::new();
        for beat in analysis.symbols.iter().filter(|s| {
            s.kind == SymbolKind::Beat && s.scene.as_deref() == Some(symbol.name.value.as_str())
        }) {
            let id = old
                .and_then(|s| s.beats.iter().find(|b| b.name == beat.name.value))
                .map(|b| b.artifact_id)
                .unwrap_or_else(|| {
                    create(
                        registry,
                        ArtifactKind::Beat,
                        &format!("{}.{}", symbol.name.value, beat.name.value),
                        Some(scene_id),
                    )
                });
            beats.push(BeatAssociation {
                name: beat.name.value.clone(),
                artifact_id: id,
            });
        }
        scenes.push(SceneAssociation {
            name: symbol.name.value.clone(),
            artifact_id: scene_id,
            beats,
        });
    }
    let changed = previous != scenes;
    registry.story_scenes = scenes;
    changed
}

/// Stored associations are untrusted. Reject missing/wrong-kind UUIDs and
/// duplicate keys rather than rebuilding their artwork during project open.
pub fn validate_associations(registry: &ArtifactRegistry) -> Result<(), String> {
    let mut ids = std::collections::HashSet::new();
    let mut scenes = std::collections::HashSet::new();
    for scene in &registry.story_scenes {
        if !scenes.insert(&scene.name) || scene.name.is_empty() {
            return Err(format!("duplicate or empty scene key '{}'", scene.name));
        }
        let mut beats = std::collections::HashSet::new();
        for (id, kind, parent) in std::iter::once((scene.artifact_id, ArtifactKind::Scene, None))
            .chain(scene.beats.iter().map(|beat| {
                (
                    beat.artifact_id,
                    ArtifactKind::Beat,
                    Some(scene.artifact_id),
                )
            }))
        {
            if !ids.insert(id)
                || !registry
                    .artifact(id)
                    .is_some_and(|a| a.kind == kind && a.parent_id == parent)
            {
                return Err(format!("invalid {kind} association for {id}"));
            }
        }
        for beat in &scene.beats {
            if beat.name.is_empty() || !beats.insert(&beat.name) {
                return Err(format!(
                    "duplicate or empty beat key '{}.{}'",
                    scene.name, beat.name
                ));
            }
        }
    }
    Ok(())
}

fn create(
    registry: &mut ArtifactRegistry,
    kind: ArtifactKind,
    name: &str,
    parent: Option<Uuid>,
) -> Uuid {
    registry
        .create_artifact(
            kind,
            CreateArtifact {
                name: unique_name(registry, name),
                parent,
                ..Default::default()
            },
        )
        .expect("reconciliation only creates scenes and beats with valid scene parents")
}

pub fn declaration_key(registry: &ArtifactRegistry, id: Uuid) -> Option<String> {
    for scene in &registry.story_scenes {
        if scene.artifact_id == id {
            return Some(scene.name.clone());
        }
        if let Some(beat) = scene.beats.iter().find(|b| b.artifact_id == id) {
            return Some(format!("{}.{}", scene.name, beat.name));
        }
    }
    None
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourceLink {
    pub label: String,
    pub span: storyfella_language::Span,
}

/// Current source origins and appearances, derived from shared symbols and
/// resolved references. Invalid drafts can still navigate recovered references.
pub fn source_links(registry: &ArtifactRegistry, analysis: &Analysis, id: Uuid) -> Vec<SourceLink> {
    let mut links = Vec::new();
    if let Some(key) = declaration_key(registry, id) {
        for symbol in &analysis.symbols {
            let name = symbol.scene.as_ref().map_or_else(
                || symbol.name.value.clone(),
                |scene| format!("{scene}.{}", symbol.name.value),
            );
            if name == key {
                links.push(SourceLink {
                    label: name,
                    span: symbol.name.span,
                });
            }
        }
    }
    for reference in &analysis.references {
        if reference.asset_id.as_deref() == Some(id.to_string().as_str()) {
            let label = reference.beat.as_ref().map_or_else(
                || reference.scene.clone(),
                |beat| format!("{}.{beat}", reference.scene),
            );
            links.push(SourceLink {
                label,
                span: reference.name.span,
            });
        }
    }
    links.sort_by_key(|link| link.span.start);
    links.dedup_by_key(|link| link.span);
    links
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::RevisionOrigin;
    use storyfella_language::analyze;

    const SOURCE: &str = "scene harbor\n  beat arrival\n    mara: Hello\n  beat light\n    prop: lantern\nscene sea\n  end\n";

    fn sync(registry: &mut ArtifactRegistry, source: &str) -> bool {
        reconcile(
            registry,
            &analyze(source, &AssetCatalog::from_registry(registry).assets),
        )
    }

    #[test]
    fn text_edits_reorder_and_invalid_drafts_preserve_artwork() {
        let mut registry = ArtifactRegistry::default();
        assert!(sync(&mut registry, SOURCE));
        let scene = registry.story_scenes[0].clone();
        let beat = scene.beats[0].artifact_id;
        let revision = registry
            .start_revision(beat, "fixture".into(), None, None, RevisionOrigin::Canon)
            .unwrap();
        registry
            .finish_revision(beat, revision, "fixture.png".into())
            .unwrap();
        let artwork = registry.artifact(beat).unwrap().clone();
        assert!(!sync(&mut registry, &SOURCE.replace("Hello", "Welcome 🌊")));
        let invalid = registry.clone();
        for source in [
            "scene harbor\n  beat",
            "scene harbor\nscene harbor\n",
            "scene harbor\n  goto missing\n",
        ] {
            assert!(!sync(&mut registry, source));
            assert_eq!(registry, invalid);
        }
        assert!(sync(
            &mut registry,
            "scene sea\nscene harbor\n  beat light\n  beat arrival\n"
        ));
        assert_eq!(registry.story_scenes[1].artifact_id, scene.artifact_id);
        assert_eq!(registry.story_scenes[1].beats[1].artifact_id, beat);
        assert_eq!(registry.artifact(beat), Some(&artwork));
        assert!(sync(&mut registry, "scene sea\n"));
        assert_eq!(registry.artifact(beat), Some(&artwork));
    }

    #[test]
    fn names_and_moves_never_adopt_manual_or_detached_artwork() {
        let mut registry = ArtifactRegistry::default();
        let manual = create(&mut registry, ArtifactKind::Scene, "harbor", None);
        sync(&mut registry, SOURCE);
        let harbor = registry.story_scenes[0].clone();
        assert_ne!(harbor.artifact_id, manual);
        assert_eq!(registry.artifact(manual).unwrap().name, "harbor");
        sync(&mut registry, "scene sea\n  beat arrival\nscene port\n");
        assert_ne!(
            registry.story_scenes[0].beats[0].artifact_id,
            harbor.beats[0].artifact_id
        );
        assert_ne!(registry.story_scenes[1].artifact_id, harbor.artifact_id);
        assert!(registry.artifact(harbor.artifact_id).is_some());
    }

    #[test]
    fn catalog_keeps_kind_errors_and_requires_an_exact_completed_active_revision() {
        let mut registry = ArtifactRegistry::default();
        let id = registry
            .create_artifact(
                ArtifactKind::Object,
                CreateArtifact {
                    name: "lantern".into(),
                    ..Default::default()
                },
            )
            .unwrap();
        let first = registry
            .start_revision(id, "fixture".into(), None, None, RevisionOrigin::Canon)
            .unwrap();
        registry
            .finish_revision(id, first, "fixture.png".into())
            .unwrap();
        let catalog = AssetCatalog::from_registry(&registry);
        assert_eq!(catalog.bindings[0].revision, first.to_string());
        assert!(analyze("scene harbor\n  lantern: hello\n", &catalog.assets).has_errors());
        registry
            .start_revision(id, "queued".into(), None, None, RevisionOrigin::Canon)
            .unwrap();
        assert!(AssetCatalog::from_registry(&registry).bindings.is_empty());
    }
}
