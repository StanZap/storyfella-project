use std::{
    fs,
    path::{Path, PathBuf},
};

use storyfella_studio_core::{
    persistence::ProjectDb,
    project::{ProjectFolder, MANIFEST},
    registry::{ArtifactKind, ArtifactRegistry, CreateArtifact, RevisionOrigin},
};
use uuid::Uuid;

fn fixture() -> tempfile::TempDir {
    let directory = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../.svs-e2e/tests");
    fs::create_dir_all(&directory).unwrap();
    tempfile::tempdir_in(directory).unwrap()
}

#[test]
fn adoption_and_relocation_preserve_source_identities_associations_and_every_revision() {
    let fixture = fixture();
    let old = fixture.path().join("old");
    fs::create_dir(&old).unwrap();
    let db = ProjectDb::open(old.join("existing.db")).unwrap();
    db.rename_project("Harbor").unwrap();
    let mut registry = ArtifactRegistry::default();
    let base = registry
        .create_artifact(
            ArtifactKind::Character,
            CreateArtifact {
                name: "mara".into(),
                ..Default::default()
            },
        )
        .unwrap();
    let variant = registry
        .create_artifact(
            ArtifactKind::Character,
            CreateArtifact {
                name: "mara-coat".into(),
                variant_of: Some(base),
                ..Default::default()
            },
        )
        .unwrap();
    for (index, id) in [base, base, variant].into_iter().enumerate() {
        let path = old.join(format!("image-{index}.png"));
        image::RgbaImage::from_pixel(2, 2, image::Rgba([index as u8, 22, 33, 100]))
            .save(&path)
            .unwrap();
        let revision = registry
            .start_revision(
                id,
                "portrait".into(),
                Some(42),
                Some("fixture".into()),
                RevisionOrigin::Canon,
            )
            .unwrap();
        registry
            .finish_revision(id, revision, path.to_str().unwrap().into())
            .unwrap();
    }
    let source = "// café 🌊\r\nscene harbor\r\n  beat arrival\r\n    mara: Hello\r\n";
    storyfella_studio_core::authoring::reconcile(
        &mut registry,
        &storyfella_language::analyze(source, &[]),
    );
    db.save_registry(&registry).unwrap();
    let mut stored = db.load().unwrap();
    stored.story.set_source(source.into());
    stored.story.save().unwrap();
    let folder = ProjectFolder::import(&fixture.path().join("portable"), &stored).unwrap();
    assert_eq!(folder.id(), stored.id);
    assert!(ProjectFolder::create(folder.root(), "Overwrite", &registry, "lost").is_err());
    let raw = rusqlite::Connection::open(folder.database()).unwrap();
    let paths: Vec<String> = raw
        .prepare("SELECT asset_path FROM revisions")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    assert_eq!(paths.len(), 3);
    assert!(paths
        .iter()
        .all(|path| path.starts_with("artwork/") && !Path::new(path).is_absolute()));
    drop(raw);
    drop(db);
    fs::remove_dir_all(&old).unwrap();
    let moved = fixture.path().join("moved");
    fs::rename(folder.root(), &moved).unwrap();
    let folder = ProjectFolder::open(&moved.join(MANIFEST)).unwrap();
    let db = ProjectDb::open(folder.database()).unwrap();
    let reopened = db.load().unwrap();
    assert_eq!(reopened.id, stored.id);
    assert_eq!(reopened.story.source(), source);
    assert_eq!(reopened.registry.story_scenes, registry.story_scenes);
    assert_eq!(
        reopened.registry.artifact(variant).unwrap().variant_of,
        Some(base)
    );
    for artifact in &reopened.registry.artifacts {
        for revision in &artifact.revisions {
            let image = image::open(revision.asset_path.as_ref().unwrap()).unwrap();
            assert_eq!(image.to_rgba8().get_pixel(0, 0)[3], 100);
        }
    }
    db.save_registry(&reopened.registry).unwrap();
    assert_eq!(db.load().unwrap(), reopened);
}

#[test]
fn invalid_manifests_and_nonportable_resources_never_create_replacements() {
    let fixture = fixture();
    let folder = ProjectFolder::create(
        &fixture.path().join("story"),
        "Story",
        &ArtifactRegistry::default(),
        "scene first\n  end\n",
    )
    .unwrap();
    let manifest = fs::read_to_string(folder.manifest()).unwrap();
    fs::write(
        folder.manifest(),
        manifest.replace("version = 1", "version = 999"),
    )
    .unwrap();
    assert!(ProjectFolder::open(folder.root()).is_err());
    fs::write(
        folder.manifest(),
        manifest.replace(&folder.id().to_string(), &Uuid::new_v4().to_string()),
    )
    .unwrap();
    assert!(ProjectFolder::open(folder.root()).is_err());
    fs::write(folder.manifest(), &manifest).unwrap();
    let source = folder.root().join("story.sf");
    fs::remove_file(&source).unwrap();
    assert!(ProjectFolder::open(folder.root()).is_err());
    assert!(!source.exists());
}

#[test]
fn missing_artwork_blocks_adoption_without_changing_the_original() {
    let fixture = fixture();
    let mut registry = ArtifactRegistry::default();
    let id = registry
        .create_artifact(ArtifactKind::Object, CreateArtifact::default())
        .unwrap();
    let revision = registry
        .start_revision(id, "cup".into(), None, None, RevisionOrigin::Canon)
        .unwrap();
    registry
        .finish_revision(
            id,
            revision,
            fixture.path().join("missing.png").to_string_lossy().into(),
        )
        .unwrap();
    let target = fixture.path().join("copy");
    assert!(ProjectFolder::create(&target, "Story", &registry, "source").is_err());
    assert!(!target.join(MANIFEST).exists());
}

#[test]
fn immutable_images_and_invalid_stored_paths_are_rejected() {
    let fixture = fixture();
    let image = fixture.path().join("source.png");
    fs::write(&image, b"original encoded bytes").unwrap();
    let mut registry = ArtifactRegistry::default();
    let id = registry
        .create_artifact(ArtifactKind::Object, CreateArtifact::default())
        .unwrap();
    let revision = registry
        .start_revision(id, "cup".into(), None, None, RevisionOrigin::Canon)
        .unwrap();
    registry
        .finish_revision(id, revision, image.to_str().unwrap().into())
        .unwrap();
    let folder =
        ProjectFolder::create(&fixture.path().join("project"), "Story", &registry, "").unwrap();
    let db = ProjectDb::open(folder.database()).unwrap();
    let saved = db.load().unwrap();
    fs::write(&image, b"different bytes").unwrap();
    assert!(db.save_registry(&registry).is_err());
    assert_eq!(db.load().unwrap(), saved);
    let raw = rusqlite::Connection::open(folder.database()).unwrap();
    for invalid in ["../source.png", "/outside.png"] {
        raw.execute("UPDATE revisions SET asset_path = ?1", [invalid])
            .unwrap();
        assert!(db.load().is_err());
    }
}

#[cfg(unix)]
#[test]
fn symlinked_resources_cannot_escape_the_project() {
    let fixture = fixture();
    let folder = ProjectFolder::create(
        &fixture.path().join("project"),
        "Story",
        &ArtifactRegistry::default(),
        "",
    )
    .unwrap();
    let outside = fixture.path().join("outside.atelier");
    fs::write(&outside, "retained external file").unwrap();
    std::os::unix::fs::symlink(&outside, folder.root().join("atelier/main.atelier")).unwrap();
    assert!(folder.atelier_document().is_err());
    assert_eq!(
        fs::read_to_string(outside).unwrap(),
        "retained external file"
    );
}
