use std::{fs, path::PathBuf};

use catalog::EditResolution as Resolution;
use storyfella_studio_core::{
    authoring::AssetCatalog,
    catalog::{self, CatalogEdit},
    persistence::{ProjectDb, StoreError},
    project::ProjectFolder,
    registry::{ArtifactKind, ArtifactRegistry},
};

fn fixture() -> tempfile::TempDir {
    let directory = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../.svs-e2e/tests");
    fs::create_dir_all(&directory).unwrap();
    tempfile::tempdir_in(directory).unwrap()
}

#[test]
fn declarations_create_pending_catalog_entries_without_images_and_invalid_drafts_preserve_them() {
    let mut registry = ArtifactRegistry::default();
    let source = "scene harbor\n  cast: mara, tam\n  location: dock\n  prop: lamp\n  beat arrival\n    mara: Hello\n    speaker_only: A voice\n";
    assert!(catalog::reconcile_declarations(&mut registry, source, true));
    assert_eq!(registry.artifacts.len(), 4);
    assert!(registry.artifacts.iter().all(|a| a.revisions.is_empty()));
    let saved = registry.clone();
    assert!(!catalog::reconcile_declarations(
        &mut registry,
        source,
        true
    ));
    assert!(!catalog::reconcile_declarations(
        &mut registry,
        "scene harbor\n  cast: incomplete,",
        true
    ));
    assert_eq!(registry, saved);
    assert!(!catalog::reconcile_declarations(
        &mut registry,
        "scene harbor\n  cast: another\n  prop: mara\n",
        true
    ));
    assert_eq!(registry, saved);
    assert!(!catalog::reconcile_declarations(
        &mut registry,
        "scene harbor\n  end\n",
        false
    ));
    assert_eq!(
        registry, saved,
        "removing source usage never removes artwork identities"
    );
    assert!(!catalog::reconcile_declarations(
        &mut registry,
        "scene harbor\n  prop: partial",
        true
    ));
    assert!(catalog::reconcile_declarations(
        &mut registry,
        "scene harbor\n  prop: partial\n",
        true
    ));
}

#[test]
fn staged_artwork_creates_pending_character_entries() {
    let mut registry = ArtifactRegistry::default();
    let source = "scene harbor\n  stage: lead, mara, left, 125, 4\n  narrate: Arrival\n  swap: lead, mara_smile\n  narrate: Hello\n";
    assert!(catalog::reconcile_declarations(
        &mut registry,
        source,
        false
    ));
    assert_eq!(registry.artifacts.len(), 2);
    assert!(registry
        .artifacts
        .iter()
        .all(|item| item.kind == ArtifactKind::Character && item.revisions.is_empty()));
    assert!(registry.artifacts.iter().any(|item| item.name == "mara"));
    assert!(registry
        .artifacts
        .iter()
        .any(|item| item.name == "mara_smile"));
}

#[test]
fn both_editors_share_identity_across_creation_variations_rename_artwork_and_relocation() {
    let fixture = fixture();
    let source = "scene harbor\n  cast: mara\n  beat arrival\n    mara: Hello\n";
    let mut registry = ArtifactRegistry::default();
    catalog::reconcile_declarations(&mut registry, source, false);
    let mara = registry.artifacts[0].id;
    let folder =
        ProjectFolder::create(&fixture.path().join("project"), "Harbor", &registry, source)
            .unwrap();
    let atelier = ProjectDb::open(folder.database()).unwrap();
    let snapshot = atelier.load().unwrap();
    assert!(
        catalog::save_edit(
            &atelier,
            &snapshot,
            CatalogEdit::Describe {
                id: mara,
                name: "captain_mara".into(),
                description: "Captain".into()
            },
            Resolution::Check,
        )
        .is_err(),
        "Atelier must not invalidate source references owned by Studio"
    );
    assert_eq!(atelier.load().unwrap(), snapshot);
    let (coat, snapshot) = catalog::save_edit(
        &atelier,
        &snapshot,
        CatalogEdit::Create {
            kind: ArtifactKind::Character,
            name: "mara_coat".into(),
            description: "Blue coat".into(),
            variant_of: Some(mara),
        },
        Resolution::Check,
    )
    .unwrap();
    let (illustration, snapshot) = catalog::save_edit(
        &atelier,
        &snapshot,
        CatalogEdit::Create {
            kind: ArtifactKind::Illustration,
            name: "arrival".into(),
            description: "Full illustration".into(),
            variant_of: None,
        },
        Resolution::Check,
    )
    .unwrap();
    let (tam, snapshot) = catalog::save_edit(
        &atelier,
        &snapshot,
        CatalogEdit::Create {
            kind: ArtifactKind::Character,
            name: "tam".into(),
            description: "An independent character".into(),
            variant_of: None,
        },
        Resolution::Check,
    )
    .unwrap();
    let image = fixture.path().join("image.png");
    image::RgbaImage::from_pixel(2, 2, image::Rgba([10, 30, 60, 100]))
        .save(&image)
        .unwrap();
    let (_, snapshot) = catalog::save_edit(
        &atelier,
        &snapshot,
        CatalogEdit::ImportImage {
            id: coat,
            path: image,
        },
        Resolution::Check,
    )
    .unwrap();
    let revision = snapshot.registry.artifact(coat).unwrap().active_revision_id;
    let (_, snapshot) = catalog::save_edit(
        &atelier,
        &snapshot,
        CatalogEdit::Describe {
            id: coat,
            name: "mara_coat".into(),
            description: "Updated requirement".into(),
        },
        Resolution::Check,
    )
    .unwrap();
    assert_eq!(
        snapshot.registry.artifact(coat).unwrap().active_revision_id,
        revision
    );
    assert_eq!(
        snapshot.registry.artifact(coat).unwrap().variant_of,
        Some(mara)
    );
    assert_eq!(
        snapshot.story.source(),
        source,
        "Atelier never inserts or rewrites authored source"
    );
    let catalog = AssetCatalog::from_registry(&snapshot.registry);
    let references = storyfella_language::analyze(
        "scene harbor\n  cast: tam\n  beat first\n    tam: Hello\n",
        &catalog.assets,
    );
    assert!(!references.has_errors());
    assert!(references
        .references
        .iter()
        .any(|r| r.asset_id.as_deref() == Some(&tam.to_string())));
    drop(atelier);
    fs::rename(folder.root(), fixture.path().join("moved")).unwrap();
    let folder = ProjectFolder::open(&fixture.path().join("moved")).unwrap();
    let studio = ProjectDb::open(folder.database()).unwrap().load().unwrap();
    assert_eq!(studio.id, folder.id());
    assert_eq!(
        studio.registry.artifact(illustration).unwrap().kind,
        ArtifactKind::Illustration
    );
    assert_eq!(studio.registry.artifact(coat).unwrap().id, coat);
    let image = studio.registry.artifact(coat).unwrap().revisions[0]
        .asset_path
        .as_ref()
        .unwrap();
    assert_eq!(
        image::open(image).unwrap().to_rgba8().get_pixel(0, 0)[3],
        100
    );
}

#[test]
fn stale_catalog_writes_and_invalid_edits_preserve_other_editors_work() {
    let fixture = fixture();
    let folder = ProjectFolder::create(
        &fixture.path().join("project"),
        "Story",
        &ArtifactRegistry::default(),
        "scene first\n  end\n",
    )
    .unwrap();
    let studio = ProjectDb::open(folder.database()).unwrap();
    let atelier = ProjectDb::open(folder.database()).unwrap();
    let stale = studio.load().unwrap();
    let (id, saved) = catalog::save_edit(
        &atelier,
        &stale,
        CatalogEdit::Create {
            kind: ArtifactKind::Character,
            name: "mara".into(),
            description: "Captain".into(),
            variant_of: None,
        },
        Resolution::Check,
    )
    .unwrap();
    assert!(matches!(
        studio.save_registry_checked(&stale.registry, stale.registry_version),
        Err(StoreError::Conflict)
    ));
    assert_eq!(studio.load().unwrap(), saved);
    for edit in [
        CatalogEdit::Create {
            kind: ArtifactKind::Character,
            name: "MARA".into(),
            description: String::new(),
            variant_of: None,
        },
        CatalogEdit::Create {
            kind: ArtifactKind::Character,
            name: "bad name".into(),
            description: String::new(),
            variant_of: None,
        },
        CatalogEdit::Create {
            kind: ArtifactKind::Object,
            name: "coat".into(),
            description: String::new(),
            variant_of: Some(id),
        },
        CatalogEdit::ImportImage {
            id,
            path: fixture.path().join("missing.png"),
        },
    ] {
        assert!(catalog::save_edit(&atelier, &saved, edit, Resolution::Check).is_err());
        assert_eq!(studio.load().unwrap(), saved);
    }
}
