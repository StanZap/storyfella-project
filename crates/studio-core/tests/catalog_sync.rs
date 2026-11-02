use std::fs;

use storyfella_studio_core::{
    catalog::{
        self,
        sync::{merge, Resolution},
        CatalogEdit,
    },
    persistence::ProjectDb,
    project::ProjectFolder,
    registry::{ArtifactKind, ArtifactRegistry, CreateArtifact, RevisionOrigin},
};

fn add(registry: &mut ArtifactRegistry, name: &str) -> uuid::Uuid {
    registry
        .create_artifact(
            ArtifactKind::Character,
            CreateArtifact {
                name: name.into(),
                description: "Original".into(),
                ..Default::default()
            },
        )
        .unwrap()
}

fn image_revision(registry: &mut ArtifactRegistry, id: uuid::Uuid, path: &str) -> uuid::Uuid {
    let revision = registry
        .start_revision(id, "Saved image".into(), None, None, RevisionOrigin::Canon)
        .unwrap();
    registry.finish_revision(id, revision, path.into()).unwrap();
    revision
}

#[test]
fn independent_fields_creation_rename_and_visual_state_merge_without_changing_identities() {
    let mut base = ArtifactRegistry::default();
    let id = add(&mut base, "mara");
    let mut local = base.clone();
    local.artifact_mut(id).unwrap().name = "captain_mara".into();
    local.artifact_mut(id).unwrap().description = "Captain in blue".into();
    let local_new = add(&mut local, "tam");
    let mut saved = base.clone();
    let revision = image_revision(&mut saved, id, "/saved.png");
    let remote_new = add(&mut saved, "ren");
    let merged = merge(&base, &local, &saved, Resolution::Check).unwrap();
    assert_eq!(merged.artifacts.len(), 3);
    let item = merged.artifact(id).unwrap();
    assert_eq!(item.name, "captain_mara");
    assert_eq!(item.description, "Captain in blue");
    assert_eq!(item.active_revision_id, Some(revision));
    assert!(merged.artifact(local_new).is_some());
    assert!(merged.artifact(remote_new).is_some());
}

#[test]
fn overlapping_details_and_active_images_need_a_choice_and_retain_both_histories() {
    let mut base = ArtifactRegistry::default();
    let id = add(&mut base, "mara");
    let mut local = base.clone();
    let mut saved = base.clone();
    local.artifact_mut(id).unwrap().description = "Local".into();
    saved.artifact_mut(id).unwrap().description = "Saved".into();
    let local_revision = image_revision(&mut local, id, "/local.png");
    let saved_revision = image_revision(&mut saved, id, "/saved.png");
    let conflict = merge(&base, &local, &saved, Resolution::Check).unwrap_err();
    assert!(!conflict.requires_reload);
    assert_eq!(conflict.fields.len(), 2);
    for (resolution, description, active) in [
        (Resolution::Local, "Local", local_revision),
        (Resolution::Saved, "Saved", saved_revision),
    ] {
        let merged = merge(&base, &local, &saved, resolution).unwrap();
        let item = merged.artifact(id).unwrap();
        assert_eq!(item.description, description);
        assert_eq!(item.active_revision_id, Some(active));
        assert_eq!(item.revisions.len(), 2);
    }
}

#[test]
fn duplicate_names_and_deleted_relationship_targets_are_never_silently_coalesced() {
    let base = ArtifactRegistry::default();
    let mut local = base.clone();
    let mut saved = base.clone();
    add(&mut local, "mara");
    add(&mut saved, "MARA");
    for resolution in [Resolution::Check, Resolution::Local, Resolution::Saved] {
        assert!(
            merge(&base, &local, &saved, resolution)
                .unwrap_err()
                .requires_reload
        );
    }
    let mut existing = local.clone();
    existing.artifacts.extend(saved.artifacts.clone());
    assert_eq!(
        merge(&existing, &existing, &existing, Resolution::Check).unwrap(),
        existing
    );
    let mut base = ArtifactRegistry::default();
    let id = add(&mut base, "mara");
    let mut local = base.clone();
    local
        .create_artifact(
            ArtifactKind::Character,
            CreateArtifact {
                name: "mara_coat".into(),
                variant_of: Some(id),
                ..Default::default()
            },
        )
        .unwrap();
    assert!(
        merge(
            &base,
            &local,
            &ArtifactRegistry::default(),
            Resolution::Check
        )
        .unwrap_err()
        .requires_reload
    );
}

#[test]
fn stale_form_merges_independent_changes_conflicts_retain_disk_and_explicit_retry_survives_reopen()
{
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.svs-e2e/tests");
    fs::create_dir_all(&root).unwrap();
    let root = tempfile::tempdir_in(root).unwrap();
    let mut registry = ArtifactRegistry::default();
    let id = add(&mut registry, "mara");
    let folder = ProjectFolder::create(
        &root.path().join("story"),
        "Story",
        &registry,
        "scene harbor\n  end\n",
    )
    .unwrap();
    let studio = ProjectDb::open_existing(folder.database()).unwrap();
    let atelier = ProjectDb::open_existing(folder.database()).unwrap();
    let baseline = atelier.load().unwrap();
    let mut changed = baseline.registry.clone();
    changed.artifact_mut(id).unwrap().name = "captain_mara".into();
    add(&mut changed, "tam");
    studio
        .save_registry_checked(&changed, baseline.registry_version)
        .unwrap();
    let (_, updated) = catalog::save_edit(
        &atelier,
        &baseline,
        CatalogEdit::Describe {
            id,
            name: "mara".into(),
            description: "Local draft".into(),
        },
        catalog::EditResolution::Check,
    )
    .unwrap();
    assert_eq!(updated.registry.artifact(id).unwrap().name, "captain_mara");
    assert_eq!(
        updated.registry.artifact(id).unwrap().description,
        "Local draft"
    );
    assert_eq!(
        updated.registry.artifacts.len(),
        baseline.registry.artifacts.len() + 1
    );
    let err = catalog::save_edit(
        &atelier,
        &baseline,
        CatalogEdit::Describe {
            id,
            name: "mara".into(),
            description: "Other draft".into(),
        },
        catalog::EditResolution::Check,
    )
    .unwrap_err();
    assert_eq!(
        err.downcast_ref::<catalog::EditConflict>()
            .unwrap()
            .saved_version,
        updated.registry_version
    );
    assert_eq!(studio.load().unwrap(), updated);
    assert!(catalog::save_edit(
        &atelier,
        &baseline,
        CatalogEdit::Describe {
            id,
            name: "mara".into(),
            description: "Other draft".into()
        },
        catalog::EditResolution::KeepDraft {
            saved_version: baseline.registry_version
        }
    )
    .is_err());
    assert_eq!(studio.load().unwrap(), updated);
    let (_, saved) = catalog::save_edit(
        &atelier,
        &baseline,
        CatalogEdit::Describe {
            id,
            name: "mara".into(),
            description: "Other draft".into(),
        },
        catalog::EditResolution::KeepDraft {
            saved_version: updated.registry_version,
        },
    )
    .unwrap();
    assert_eq!(
        saved.registry.artifact(id).unwrap().description,
        "Other draft"
    );
    assert_eq!(saved.registry.artifact(id).unwrap().name, "captain_mara");
    assert_eq!(saved.story, baseline.story);
    assert_eq!(
        ProjectDb::open_existing(folder.database())
            .unwrap()
            .load()
            .unwrap(),
        saved
    );
    // Even an unused item may already be referenced in Studio's dirty buffer.
    assert!(catalog::save_edit(
        &atelier,
        &saved,
        CatalogEdit::Describe {
            id,
            name: "renamed".into(),
            description: String::new()
        },
        catalog::EditResolution::Check
    )
    .is_err());
    assert_eq!(studio.load().unwrap(), saved);
    let absent = root.path().join("missing.db");
    assert!(ProjectDb::open_existing(&absent).is_err());
    assert!(!absent.exists());
}

#[test]
fn simultaneous_writers_cannot_replace_each_other_and_loser_can_merge_then_retry() {
    use std::sync::{Arc, Barrier};
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.svs-e2e/tests");
    fs::create_dir_all(&root).unwrap();
    let root = tempfile::tempdir_in(root).unwrap();
    let folder = ProjectFolder::create(
        &root.path().join("story"),
        "Story",
        &ArtifactRegistry::default(),
        "scene harbor\n  end\n",
    )
    .unwrap();
    let barrier = Arc::new(Barrier::new(2));
    let writers = ["mara", "tam"].map(|name| {
        let path = folder.database();
        let barrier = barrier.clone();
        std::thread::spawn(move || {
            let db = ProjectDb::open_existing(path).unwrap();
            let base = db.load().unwrap();
            let mut local = base.registry.clone();
            let id = add(&mut local, name);
            barrier.wait();
            let result = db.save_registry_checked(&local, base.registry_version);
            (base, local, id, result)
        })
    });
    let outcomes = writers.map(|writer| writer.join().unwrap());
    assert_eq!(
        outcomes
            .iter()
            .filter(|(_, _, _, result)| result.is_ok())
            .count(),
        1
    );
    let (base, local, _, error) = outcomes
        .iter()
        .find(|(_, _, _, result)| result.is_err())
        .unwrap();
    assert!(matches!(
        error,
        Err(storyfella_studio_core::persistence::StoreError::Conflict)
    ));
    let db = ProjectDb::open_existing(folder.database()).unwrap();
    let saved = db.load().unwrap();
    let merged = merge(&base.registry, local, &saved.registry, Resolution::Check).unwrap();
    db.save_registry_checked(&merged, saved.registry_version)
        .unwrap();
    let reopened = ProjectDb::open_existing(folder.database())
        .unwrap()
        .load()
        .unwrap();
    for (_, _, id, _) in outcomes {
        assert!(reopened.registry.artifact(id).is_some());
    }
    assert_eq!(reopened.story.source(), "scene harbor\n  end\n");
}
