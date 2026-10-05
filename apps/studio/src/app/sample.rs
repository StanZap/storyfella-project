//! A populated project, for looking at the app with real shape in it.
//!
//! An empty registry hides every layout question worth asking: how a grid of
//! twenty tiles wraps, what a variant stack looks like next to a bare one,
//! whether the writer's scope panel is legible when half its references
//! resolve and half do not. This builds that state without a backend — no
//! images are generated, so tiles show placeholders and the arrangement is
//! what you are judging.
//!
//! The story deliberately references artifacts that **do not exist**
//! (`keeper_elias`, `lighthouse_tower`, `wreck_hull`), so the writer's
//! pending affordances have something to point at.

use std::path::Path;

use storyfella_studio_core::registry::{
    ArtifactKind, ArtifactRegistry, CreateArtifact, RevisionOrigin, RevisionStatus, VariantAxis,
};

/// `(name, description)` for one artifact.
type Entry = (&'static str, &'static str);

const CHARACTERS: &[Entry] = &[
    ("mara", "A harbourmaster in her forties, weathered face, close-cropped grey hair, heavy navy oilskin coat"),
    ("tam", "A courier on a rooftop at sunrise, cropped jacket, city behind"),
    ("isak", "A lamplighter in a long wool coat, carrying a brass pole and a lit lantern"),
    ("nell", "A dockhand in her twenties, rope over one shoulder, sunburnt forearms"),
    ("the_purser", "A ledger-keeper with wire spectacles and ink-stained cuffs"),
];

const ENVIRONMENTS: &[Entry] = &[
    (
        "harbor_dock",
        "A stone quay at dawn, low tide, fishing boats leaning on the mud",
    ),
    (
        "lamp_street",
        "A narrow cobbled street at blue hour, cast-iron gas lamps, wet stone",
    ),
    ("ellery_road", "A washed-out coast road under heavy cloud"),
    (
        "the_counting_house",
        "A cramped office of ledgers and oil light above the quay",
    ),
];

const OBJECTS: &[Entry] = &[
    (
        "brass_lantern",
        "A dented brass lantern with a cracked glass pane",
    ),
    ("rope_coil", "A coil of tarred hemp rope on a bollard"),
    (
        "tide_ledger",
        "A water-stained ledger of tide times and cargo weights",
    ),
];

/// Variants: `(base, name, description, axis)`.
const VARIANTS: &[(&str, &str, &str, VariantAxis)] = &[
    (
        "mara",
        "mara_oilskins",
        "in heavy yellow oilskins, hood up",
        VariantAxis::Outfit,
    ),
    (
        "mara",
        "mara_night",
        "lit from below by a lantern, night",
        VariantAxis::TimeOfDay,
    ),
    (
        "mara",
        "mara_young",
        "twenty years younger, dark hair",
        VariantAxis::Age,
    ),
    (
        "tam",
        "tam_storm",
        "soaked through, collar up",
        VariantAxis::Weather,
    ),
    (
        "harbor_dock",
        "harbor_dock_storm",
        "under a running squall, boats pitching",
        VariantAxis::Weather,
    ),
];

/// The story text, referencing both artifacts that exist and three that do
/// not — the pending state is the point.
pub const STORY: &str = r#"# Three of the names below have no artifact yet.
# They show as pending on the right, ready to create.

story "Keeper of the Low Light"

scene harbor
  location: harbor_dock
  cast: mara, tam

  beat arrival
    shot: wide, dawn
    mara: "You're late. Tide's already turning."
    tam think: She counts the minutes like coins.
    tam: "The road was out past Ellery."

  beat handoff
    shot: close on mara
    prop: brass_lantern
    mara: "Take it up before dark. Don't stop to look at the wreck."

scene counting_house
  location: the_counting_house
  cast: the_purser, nell

  beat the_ledger
    shot: over the shoulder
    prop: tide_ledger
    the_purser: "Two tons short, and the weather isn't to blame."

scene lighthouse
  location: lighthouse_tower
  cast: tam, keeper_elias

  beat climb
    shot: low angle, spiral stair
    tam think: Ninety-two steps. She said ninety.
    keeper_elias: "You brought the wrong light, girl."

  beat the_wreck
    prop: wreck_hull
    keeper_elias: "It's been down there longer than you've been alive."
"#;

/// The picture checked into `docs/poc/` that stands in for a generated one.
///
/// A single real image across twelve artifacts is deliberate: the grid has to
/// be judged with *both* states present, and a project mid-way through its
/// work is mostly placeholders anyway.
///
/// Resolved against the manifest directory rather than the working
/// directory, so a dev build finds it from anywhere. The image lives in the
/// repository's `docs/poc/`, and the manifest directory is this crate
/// (`apps/studio`), so the `../..` hops up to the workspace root. They are
/// ~2.4MB and are **not** embedded: a shipped binary simply will not find
/// them, and [`install_image`] falls back to leaving the artifact imageless.
const STAND_IN_IMAGES: &[&str] = &[concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../docs/poc/image-generation/latest/lighthouse-seed-42.png"
)];

/// Copies a stand-in into the generated-asset directory under a fresh name
/// and returns that filename, which is what a revision stores.
fn install_image(asset_dir: &Path, source: &str) -> Option<String> {
    let generated = asset_dir.join("generated");
    std::fs::create_dir_all(&generated).ok()?;
    let filename = format!("{}.png", uuid::Uuid::new_v4());
    std::fs::copy(source, generated.join(&filename)).ok()?;
    Some(filename)
}

/// A registry with characters, environments, objects and variants, plus the
/// story that references them.
///
/// `asset_dir` receives copies of the stand-in images; when they cannot be
/// read (running from another directory) the artifacts simply stay
/// imageless, which is still a usable sample.
pub fn project(asset_dir: &Path) -> (ArtifactRegistry, String) {
    let mut registry = ArtifactRegistry::new();

    let add = |registry: &mut ArtifactRegistry, kind, (name, description): Entry| {
        registry
            .create_artifact(
                kind,
                CreateArtifact {
                    name: name.to_owned(),
                    description: description.to_owned(),
                    ..Default::default()
                },
            )
            .expect("sample artifacts are well formed");
    };

    for entry in CHARACTERS {
        add(&mut registry, ArtifactKind::Character, *entry);
    }
    for entry in ENVIRONMENTS {
        add(&mut registry, ArtifactKind::Environment, *entry);
    }
    for entry in OBJECTS {
        add(&mut registry, ArtifactKind::Object, *entry);
    }

    for (base, name, description, axis) in VARIANTS {
        let base_id = registry
            .artifacts
            .iter()
            .find(|artifact| artifact.name == *base)
            .map(|artifact| artifact.id)
            .expect("a variant's base is declared above it");
        let kind = registry.artifact(base_id).expect("base exists").kind;
        registry
            .create_artifact(
                kind,
                CreateArtifact {
                    name: (*name).to_owned(),
                    description: (*description).to_owned(),
                    variant_of: Some(base_id),
                    axis: Some(*axis),
                    ..Default::default()
                },
            )
            .expect("sample variants are well formed");
    }

    // Give the first few artifacts a history so the detail pane has
    // something to show: a canon render, then an experiment on top of it.
    let with_images: Vec<uuid::Uuid> = registry
        .artifacts
        .iter()
        .filter(|artifact| artifact.variant_of.is_none())
        .take(STAND_IN_IMAGES.len())
        .map(|artifact| artifact.id)
        .collect();

    for (slot, id) in with_images.iter().enumerate() {
        let description = registry
            .artifact(*id)
            .map(|artifact| artifact.description.clone())
            .unwrap_or_default();

        // Canon: renders the description.
        let canon = registry
            .start_revision(
                *id,
                description,
                Some(4471),
                Some(storyfella_studio_core::registry::pipeline::DEFAULT_MODEL.to_owned()),
                RevisionOrigin::Canon,
            )
            .expect("sample revision starts");
        match install_image(asset_dir, STAND_IN_IMAGES[slot]) {
            Some(filename) => {
                registry
                    .finish_revision(*id, canon, filename)
                    .expect("sample revision finishes");
            }
            None => {
                registry
                    .fail_revision(*id, canon, "stand-in image not found".to_owned())
                    .ok();
            }
        }

        // Experiment: a delta against the description, which is what the
        // canon/experiment badges and Promote exist to distinguish.
        if slot == 0 {
            let experiment = registry
                .start_revision(
                    *id,
                    "give her cropped bleached hair".to_owned(),
                    Some(9120),
                    Some(storyfella_studio_core::registry::pipeline::DEFAULT_MODEL.to_owned()),
                    RevisionOrigin::Experiment,
                )
                .expect("sample revision starts");
            if let Some(filename) = install_image(asset_dir, STAND_IN_IMAGES[0]) {
                registry
                    .finish_revision(*id, experiment, filename)
                    .expect("sample revision finishes");
            }
        }
    }

    // One artifact left mid-render, so the grid's `rendering` badge and the
    // dimmed tile are visible without a backend running.
    if let Some(id) = registry
        .artifacts
        .iter()
        .find(|artifact| artifact.name == "isak")
        .map(|artifact| artifact.id)
    {
        let queued = registry
            .start_revision(
                id,
                "a lamplighter".to_owned(),
                None,
                None,
                RevisionOrigin::Canon,
            )
            .expect("sample revision starts");
        if let Some(revision) = registry
            .artifact_mut(id)
            .and_then(|artifact| artifact.revisions.iter_mut().find(|r| r.id == queued))
        {
            revision.status = RevisionStatus::Generating;
        }
    }

    // Stored generation defaults, so the inspector shows both states: a
    // pinned seed and an explicit step count on one artifact, defaults
    // everywhere else.
    if let Some(id) = registry
        .artifacts
        .iter()
        .find(|artifact| artifact.name == "mara")
        .map(|artifact| artifact.id)
    {
        registry.set_default_steps(id, Some(12)).ok();
        registry.set_default_seed(id, Some(4471)).ok();
    }

    (registry, STORY.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_sample_project_builds() {
        let (registry, story) = project(Path::new("/nonexistent"));
        assert_eq!(
            registry.artifacts.len(),
            CHARACTERS.len() + ENVIRONMENTS.len() + OBJECTS.len() + VARIANTS.len()
        );
        assert!(!story.is_empty());
    }

    #[test]
    fn variants_point_at_their_base() {
        let (registry, _) = project(Path::new("/nonexistent"));
        for (base, name, _, _) in VARIANTS {
            let variant = registry
                .artifacts
                .iter()
                .find(|artifact| artifact.name == *name)
                .expect("variant exists");
            let base_id = registry
                .artifacts
                .iter()
                .find(|artifact| artifact.name == *base)
                .map(|artifact| artifact.id);
            assert_eq!(variant.variant_of, base_id);
            assert!(variant.variant_axis.is_some());
        }
    }

    #[test]
    fn the_sample_exercises_every_state_the_canvas_renders() {
        // The sample exists to make parity judgeable, so it has to contain
        // each state at once: a finished render, an experiment beside a
        // canon revision, work in flight, and stored generation defaults.
        let dir = std::env::temp_dir().join(format!("svs-sample-{}", uuid::Uuid::new_v4()));
        let (registry, _) = project(&dir);

        let rendered = registry
            .artifacts
            .iter()
            .filter(|artifact| registry.latest_image(artifact.id).is_some())
            .count();
        assert!(rendered > 0, "no artifact has a picture to show");

        assert!(
            registry.artifacts.iter().any(|artifact| artifact
                .revisions
                .iter()
                .any(|revision| revision.origin == RevisionOrigin::Experiment)),
            "nothing to promote"
        );
        assert!(
            registry.artifacts.iter().any(|artifact| artifact
                .revisions
                .iter()
                .any(|revision| revision.status == RevisionStatus::Generating)),
            "no in-flight tile, so the rendering badge cannot be seen"
        );
        assert!(
            registry
                .artifacts
                .iter()
                .any(|artifact| artifact.default_seed.is_some()),
            "no pinned seed, so the inspector shows only one seed state"
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn the_story_leaves_exactly_three_references_pending() {
        // The pending affordances are the reason this project exists; if a
        // later edit resolves them all, the sample stops demonstrating them.
        let (registry, story) = project(Path::new("/nonexistent"));
        let known: Vec<String> = registry
            .artifacts
            .iter()
            .map(|artifact| artifact.name.clone())
            .collect();

        let mut pending: Vec<String> = story
            .lines()
            .flat_map(crate::ui::writer::names_in)
            .filter(|name| !known.contains(name))
            .collect();
        pending.sort();
        pending.dedup();
        assert_eq!(pending, ["keeper_elias", "lighthouse_tower", "wreck_hull"]);
    }
}
