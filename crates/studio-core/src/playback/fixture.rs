//! Repeatable, model-free artwork shared by playback tests and desktop demos.
use crate::{
    authoring::reconcile,
    registry::{ArtifactKind, ArtifactRegistry, CreateArtifact, RevisionOrigin},
};
use std::path::Path;

pub const SOURCE: &str = "story \"The Lantern Routes\"\nscene harbor\n  location: harbor_dock\n  cast: mara\n  prop: brass_lantern\n  beat arrival\n    mara: Ready for the light? 🌊\n    mara think: The tide is turning.\n    narrate: Rain taps the lantern.\n    choice \"Where next?\"\n      option \"Follow the light\": lighthouse\n      option \"Return to harbor\": harbor\nscene lighthouse\n  prop: brass_lantern\n  beat signal\n    narrate: The lantern shines.\n    end\n";

pub fn create(directory: &Path) -> anyhow::Result<(String, ArtifactRegistry)> {
    std::fs::create_dir_all(directory)?;
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
        image.save(&path)?;
        let id = registry.create_artifact(
            kind,
            CreateArtifact {
                name: name.into(),
                ..Default::default()
            },
        )?;
        let revision = registry.start_revision(
            id,
            "Procedural playback fixture".into(),
            None,
            Some("fixture".into()),
            RevisionOrigin::Canon,
        )?;
        registry.finish_revision(id, revision, path.to_string_lossy().into())?;
    }
    reconcile(&mut registry, &storyfella_language::analyze(SOURCE, &[]));
    Ok((SOURCE.into(), registry))
}
