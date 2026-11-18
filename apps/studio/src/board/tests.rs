use storyfella_studio_core::{
    catalog::CatalogEdit,
    registry::{ArtifactKind, RevisionOrigin},
    story::StoryDocument,
};

use super::*;

/// A two-chapter story naming Mara in both and the harbour in one, with
/// Mara drawn, a variation of her not yet drawn, and the harbour undrawn.
fn harbour() -> (tempfile::TempDir, AppState, Uuid, Uuid, Uuid) {
    let directory = tempfile::tempdir().unwrap();
    let entry = directory.path().join("story.sf");
    std::fs::write(&entry, "include \"one.sf\"\ninclude \"two.sf\"\n").unwrap();
    std::fs::write(
        directory.path().join("one.sf"),
        "scene start\n  location: harbor\n  mara: \"You came back.\"\n  goto ending\n",
    )
    .unwrap();
    std::fs::write(
        directory.path().join("two.sf"),
        "scene ending\n  mara: \"Stay.\"\n  end\n",
    )
    .unwrap();
    let mut state = AppState::default();
    state.story = StoryDocument::open(&entry).unwrap();
    state.reconcile_story();
    let create = |state: &mut AppState, kind, name: &str, variant_of| {
        if let Some(found) = state.registry.artifacts.iter().find(|a| a.name == name) {
            assert_eq!(found.kind, kind, "the story named {name} as another kind");
            return found.id;
        }
        state
            .edit_catalog(CatalogEdit::Create {
                kind,
                name: name.into(),
                description: String::new(),
                variant_of,
            })
            .unwrap()
    };
    let mara = create(&mut state, ArtifactKind::Character, "mara", None);
    let still = create(
        &mut state,
        ArtifactKind::Character,
        "mara_still",
        Some(mara),
    );
    let harbor = create(&mut state, ArtifactKind::Environment, "harbor", None);
    let image = directory.path().join("mara.png");
    std::fs::write(&image, b"png").unwrap();
    let revision = state
        .registry
        .start_revision(mara, "sketch".into(), None, None, RevisionOrigin::Canon)
        .unwrap();
    state
        .registry
        .finish_revision(mara, revision, image.to_string_lossy().into())
        .unwrap();
    (directory, state, mara, still, harbor)
}

#[test]
fn cards_stack_variations_and_count_uses_across_chapters() {
    let (_directory, state, mara, _, harbor) = harbour();
    let reading = Reading::of(&state);
    let cards = cards(&state, &reading, Filter::All);
    let names: Vec<_> = cards.iter().map(|card| card.name.as_str()).collect();
    assert_eq!(names, ["mara", "harbor"], "characters before places");
    assert_eq!(cards[0].id, mara);
    assert_eq!(cards[0].variations, 1);
    assert_eq!(cards[0].uses, 2);
    assert!(cards[0].image.is_some());
    assert_eq!(cards[0].detail(), "1 variation · 2 uses");
    assert_eq!(cards[1].id, harbor);
    assert_eq!(cards[1].detail(), "1 use");

    let uses = reading.uses(&state.registry, mara);
    let places: Vec<_> = uses.iter().map(|at| (at.file.as_str(), at.line)).collect();
    assert_eq!(places, [("one.sf", 3), ("two.sf", 2)]);
}

#[test]
fn filters_pick_a_kind_or_what_still_needs_art() {
    let (_directory, state, _, still, harbor) = harbour();
    let reading = Reading::of(&state);
    let ids = |filter| -> Vec<Uuid> {
        cards(&state, &reading, filter)
            .into_iter()
            .map(|card| card.id)
            .collect()
    };
    assert_eq!(ids(Filter::Kind(ArtifactKind::Environment)), [harbor]);
    assert!(ids(Filter::Kind(ArtifactKind::Object)).is_empty());
    // A variation needs its own art, so it gets its own card here.
    assert_eq!(ids(Filter::NeedsArt), [still, harbor]);
    assert_eq!(needing_art(&state), 2);
}

#[test]
fn a_family_is_the_base_then_its_variations() {
    let (_directory, state, mara, still, harbor) = harbour();
    let names = |id| -> Vec<Uuid> {
        family(&state.registry, id)
            .into_iter()
            .map(|(id, _)| id)
            .collect()
    };
    assert_eq!(names(still), [mara, still]);
    assert_eq!(names(mara), [mara, still]);
    assert_eq!(names(harbor), [harbor]);
}
