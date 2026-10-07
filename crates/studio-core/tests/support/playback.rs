use std::path::Path;
use storyfella_studio_core::{
    authoring::reconcile,
    registry::{ArtifactKind, ArtifactRegistry, CreateArtifact, RevisionOrigin},
};

pub fn fixture(directory: &Path) -> (String, ArtifactRegistry) {
    std::fs::create_dir_all(directory).unwrap();
    let mut registry = ArtifactRegistry::new();
    for (name, kind, color) in [
        ("harbor_dock", ArtifactKind::Environment, [38, 92, 130]),
        ("mara", ArtifactKind::Character, [165, 105, 78]),
        ("brass_lantern", ArtifactKind::Object, [210, 171, 60]),
    ] {
        let path = directory.join(format!("{name}.png"));
        let image = image::RgbImage::from_fn(400, 280, |x, y| {
            if (140..260).contains(&x) && (50..230).contains(&y) {
                image::Rgb(color)
            } else {
                image::Rgb([24 + (x / 20) as u8, 22 + (y / 20) as u8, 35])
            }
        });
        image.save(&path).unwrap();
        let id = registry
            .create_artifact(
                kind,
                CreateArtifact {
                    name: name.into(),
                    ..Default::default()
                },
            )
            .unwrap();
        let revision = registry
            .start_revision(
                id,
                "Procedural playback fixture".into(),
                None,
                Some("fixture".into()),
                RevisionOrigin::Canon,
            )
            .unwrap();
        registry
            .finish_revision(id, revision, path.to_string_lossy().into())
            .unwrap();
    }
    let source = "story \"The Lantern Routes\"\nscene harbor\n  location: harbor_dock\n  cast: mara\n  prop: brass_lantern\n  beat arrival\n    mara: Ready for the light? 🌊\n    mara think: The tide is turning.\n    narrate: Rain taps the lantern.\n    choice \"Where next?\"\n      option \"Follow the light\": lighthouse\n      option \"Return to harbor\": harbor\nscene lighthouse\n  prop: brass_lantern\n  beat signal\n    narrate: The lantern shines.\n    end\n".to_string();
    reconcile(&mut registry, &storyfella_language::analyze(&source, &[]));
    (source, registry)
}
