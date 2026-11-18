use storyfella_runtime::{AssetPolicy, Command, Event, Phase};
use storyfella_studio_core::{playback::PlaybackSession, registry::RevisionOrigin};

const PNG_SIGNATURE: &[u8] = b"\x89PNG\r\n\x1a\n";

fn directory() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../.svs-e2e/tests")
        .join(format!(
            "playback-{}-{}",
            std::process::id(),
            uuid::Uuid::new_v4()
        ))
}

#[test]
fn fixture_creation_reports_a_blocked_directory_without_overwriting_it() {
    let dir = directory();
    std::fs::create_dir_all(&dir).unwrap();
    let blocked = dir.join("blocked");
    std::fs::write(&blocked, b"reserved file").unwrap();
    assert!(storyfella_studio_core::playback::fixture::create(&blocked).is_err());
    assert_eq!(std::fs::read(&blocked).unwrap(), b"reserved file");
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn both_hosts_follow_identical_events_source_positions_and_pinned_images() {
    let dir = directory();
    let (source, mut registry) = storyfella_studio_core::playback::fixture::create(&dir).unwrap();
    let studio = PlaybackSession::compile(&source, &registry).unwrap();
    let plan = studio.runtime.plan().clone();
    // A later active revision must not replace the compiled binding.
    let id = registry.artifacts[0].id;
    let new = registry
        .start_revision(id, "later".into(), None, None, RevisionOrigin::Canon)
        .unwrap();
    registry
        .finish_revision(id, new, dir.join("mara.png").to_string_lossy().into())
        .unwrap();
    for route in 0..2 {
        let mut studio = studio.clone();
        let mut player = PlaybackSession::from_plan(plan.clone(), &registry).unwrap();
        assert_eq!(studio.last_events, player.last_events);
        assert_eq!(studio.visuals[0].png, player.visuals[0].png);
        for command in [
            Command::Pause,
            Command::Resume,
            Command::Advance,
            Command::Advance,
            Command::Advance,
            Command::SelectChoice { index: route },
        ] {
            studio.dispatch(command.clone()).unwrap();
            player.dispatch(command).unwrap();
            assert_eq!(studio.last_events, player.last_events);
            assert_eq!(
                studio.runtime.current().source,
                player.runtime.current().source
            );
            assert_eq!(studio.runtime.state(), player.runtime.state());
            let visuals = |session: &PlaybackSession| {
                session
                    .active_visuals()
                    .iter()
                    .map(|v| (v.binding.clone(), v.png.clone()))
                    .collect::<Vec<_>>()
            };
            assert_eq!(visuals(&studio), visuals(&player));
        }
        let saved = studio.runtime.save();
        studio.dispatch(Command::End).unwrap();
        studio.restore(saved.clone()).unwrap();
        player.restore(saved).unwrap();
        assert_eq!(studio.runtime.state(), player.runtime.state());
        assert_eq!(studio.runtime.state().phase, Phase::Text);
        studio.dispatch(Command::Restart).unwrap();
        player.dispatch(Command::Restart).unwrap();
        assert_eq!(studio.last_events, player.last_events);
    }
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn missing_wrong_and_unreadable_pinned_resources_block_before_execution() {
    let dir = directory();
    let (source, registry) = storyfella_studio_core::playback::fixture::create(&dir).unwrap();
    let session = PlaybackSession::compile(&source, &registry).unwrap();
    let plan = session.runtime.plan().clone();
    let mut missing = registry.clone();
    missing.artifacts[0].revisions.clear();
    assert!(PlaybackSession::from_plan(plan.clone(), &missing)
        .unwrap_err()
        .contains("pinned revision is missing"));
    let mut wrong = registry.clone();
    wrong.artifacts[0].name = "renamed".into();
    assert!(PlaybackSession::from_plan(plan.clone(), &wrong).is_err());
    let path = registry.artifacts[0].revisions[0]
        .asset_path
        .as_ref()
        .unwrap();
    std::fs::write(path, b"corrupt image").unwrap();
    assert!(PlaybackSession::from_plan(plan.clone(), &registry)
        .unwrap_err()
        .contains("cannot decode"));
    std::fs::remove_file(path).unwrap();
    assert!(PlaybackSession::from_plan(plan, &registry)
        .unwrap_err()
        .contains("cannot read"));
    // The existing session retains its decoded snapshot.
    assert!(session
        .active_visuals()
        .iter()
        .all(|v| v.png.starts_with(PNG_SIGNATURE)));
    assert!(
        PlaybackSession::compile(&source.replace("    mara: ", "    shot: "), &registry)
            .unwrap_err()
            .contains("unsupported")
    );
    assert!(PlaybackSession::open(&dir.join("absent.sf"), None).is_err());
    assert!(!dir.join("absent.db").exists());
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn allow_missing_assets_keeps_text_playback_available() {
    let source = "scene harbor\n  mara: Welcome to Bellwater\n  end\n";
    let session = PlaybackSession::compile_with_policy(
        source,
        &storyfella_studio_core::registry::ArtifactRegistry::new(),
        AssetPolicy::AllowMissing,
    )
    .unwrap();
    assert!(session.visuals.is_empty());
    assert!(session.last_events.iter().any(|event| matches!(
        event,
        Event::Text {
            speaker: None,
            text,
            ..
        } if text.value == "Welcome to Bellwater"
    )));
}

#[test]
fn workspace_playback_branches_across_files_and_restores() {
    use std::collections::BTreeMap;
    use storyfella_compiler::SourceWorkspace;
    let workspace = SourceWorkspace {
        entry: "story.sf".into(),
        files: BTreeMap::from([
            (
                "story.sf".into(),
                "include \"start.sf\"\ninclude \"end.sf\"\n".into(),
            ),
            ("start.sf".into(), "scene start\n  goto finish\n".into()),
            (
                "end.sf".into(),
                "scene finish\n  narrate: The end\n  end\n".into(),
            ),
        ]),
    };
    let registry = storyfella_studio_core::registry::ArtifactRegistry::new();
    let mut session =
        PlaybackSession::compile_workspace(&workspace, &registry, AssetPolicy::Preview).unwrap();
    let source = session.runtime.current().source;
    let plan = session.runtime.plan();
    let serialized = serde_json::to_string(plan).unwrap();
    let imported: storyfella_compiler::StoryPlan = serde_json::from_str(&serialized).unwrap();
    let mapping = imported.source_map.as_ref().unwrap();
    assert_eq!(mapping.original_span(source).unwrap().0, "end.sf");
    assert_eq!(imported, *plan);
    let mut damaged = imported;
    damaged.source_map.as_mut().unwrap().segments[0]
        .original
        .start += 1;
    assert!(storyfella_runtime::validate_plan(&damaged).is_err());
    let saved = session.runtime.save();
    session.dispatch(Command::Advance).unwrap();
    assert_eq!(session.runtime.state().phase, Phase::Complete);
    session.restore(saved).unwrap();
    assert_eq!(session.runtime.state().phase, Phase::Text);
}

#[test]
fn standalone_player_opens_multi_file_project_from_disk() {
    let dir = directory();
    std::fs::create_dir_all(&dir).unwrap();
    let entry = dir.join("story.sf");
    std::fs::write(&entry, "include \"first.sf\"\ninclude \"last.sf\"\n").unwrap();
    std::fs::write(dir.join("first.sf"), "scene start\n  goto finish\n").unwrap();
    std::fs::write(
        dir.join("last.sf"),
        "scene finish\n  narrate: Done\n  end\n",
    )
    .unwrap();
    storyfella_studio_core::persistence::ProjectDb::open(dir.join("story.db")).unwrap();
    let mut session = PlaybackSession::open(&entry, None).unwrap();
    assert_eq!(session.runtime.state().phase, Phase::Text);
    session.dispatch(Command::Advance).unwrap();
    assert_eq!(session.runtime.state().phase, Phase::Complete);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn preview_reports_unresolved_and_unreadable_active_artwork() {
    let dir = directory();
    let (source, registry) = storyfella_studio_core::playback::fixture::create(&dir).unwrap();
    let location = registry
        .artifacts
        .iter()
        .find(|a| a.name == "harbor_dock")
        .unwrap();
    std::fs::remove_file(location.revisions[0].asset_path.as_ref().unwrap()).unwrap();
    let mut registry = registry;
    let character = registry
        .artifacts
        .iter_mut()
        .find(|a| a.name == "mara")
        .unwrap();
    character.revisions.clear();
    let source = source.replace("  beat arrival", "  shot: wide, dawn\n  beat arrival");
    let session =
        PlaybackSession::compile_with_policy(&source, &registry, AssetPolicy::Preview).unwrap();
    let missing = session
        .missing_uses()
        .iter()
        .map(|asset| asset.name.value.as_str())
        .collect::<Vec<_>>();
    assert_eq!(missing, ["harbor_dock", "mara"]);
    assert!(session
        .active_visuals()
        .iter()
        .any(|v| v.binding.name == "brass_lantern"));
    assert_eq!(
        session
            .runtime
            .plan()
            .readiness
            .unsupported_directions
            .len(),
        1
    );
    assert!(
        PlaybackSession::compile_with_policy(&source, &registry, AssetPolicy::AllowMissing)
            .is_err()
    );
    assert!(PlaybackSession::compile_with_policy(
        "scene harbor\nscene harbor\n",
        &registry,
        AssetPolicy::Preview
    )
    .is_err());
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn relocated_shared_project_opens_from_folder_or_manifest_with_chapters_and_artwork() {
    use storyfella_studio_core::project::ProjectFolder;
    let dir = directory();
    let (_, registry) =
        storyfella_studio_core::playback::fixture::create(&dir.join("inputs")).unwrap();
    let original = dir.join("original");
    let entry = "story \"Travel\"\ninclude \"chapter.sf\"\n";
    ProjectFolder::create(&original, "Travel", &registry, entry).unwrap();
    std::fs::write(original.join("assets/chapter.sf"), "scene harbor\n  location: harbor_dock\n  stage: lead, mara, left, 100, 1, 25, 0\n  mara: Hello\n").unwrap();
    let moved = dir.join("moved");
    std::fs::rename(&original, &moved).unwrap();
    let folder = PlaybackSession::open(&moved, None).unwrap();
    let manifest = PlaybackSession::open(&moved.join("storyfella.toml"), None).unwrap();
    let source = PlaybackSession::open(&moved.join("assets/story.sf"), None).unwrap();
    assert_eq!(folder.runtime.plan(), manifest.runtime.plan());
    assert_eq!(folder.runtime.plan(), source.runtime.plan());
    assert_eq!(folder.runtime.staged()[0].position, Some((25, 0)));
    assert_eq!(folder.visuals.len(), 2);
    assert!(folder
        .visuals
        .iter()
        .all(|image| image.png.starts_with(PNG_SIGNATURE)));
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn playing_to_a_line_puts_that_line_on_stage() {
    let source = "scene start\n  narrate: One\n  narrate: Two\n  goto second\n\
                  scene second\n  narrate: Three\n  end\n";
    let registry = storyfella_studio_core::registry::ArtifactRegistry::new();
    let mut session =
        PlaybackSession::compile_with_policy(source, &registry, AssetPolicy::Preview).unwrap();
    let on_stage = |session: &PlaybackSession| {
        let (file, span) = session.on_stage("story.sf").unwrap();
        assert_eq!(file, "story.sf");
        source[span.start..span.end].to_owned()
    };
    assert!(on_stage(&session).contains("One"));

    let two = source.find("Two").unwrap();
    assert!(session.play_to("story.sf", two, "story.sf").unwrap());
    assert!(on_stage(&session).contains("Two"));

    // The scene header and everything up to its first line start the scene.
    let header = source.find("scene second").unwrap();
    assert!(session.play_to("story.sf", header, "story.sf").unwrap());
    assert!(on_stage(&session).contains("Three"));
    let three = source.find("Three").unwrap();
    assert!(session.play_to("story.sf", three + 2, "story.sf").unwrap());
    assert!(on_stage(&session).contains("Three"));

    // Playing there is a fresh history, not one that grows with each jump.
    session.play_to("story.sf", two, "story.sf").unwrap();
    let before = session.runtime.save().commands.len();
    for _ in 0..50 {
        session.play_to("story.sf", two, "story.sf").unwrap();
    }
    assert_eq!(session.runtime.save().commands.len(), before);

    assert!(!session.play_to("other.sf", two, "story.sf").unwrap());
    session.play_to("story.sf", three, "story.sf").unwrap();
    session.dispatch(Command::Advance).unwrap();
    assert_eq!(session.runtime.state().phase, Phase::Complete);
    assert!(session.on_stage("story.sf").is_none());
}

#[test]
fn playing_to_a_line_finds_it_in_an_included_file() {
    use std::collections::BTreeMap;
    use storyfella_compiler::SourceWorkspace;
    let end = "scene finish\n  narrate: Almost\n  narrate: The end\n  end\n";
    let workspace = SourceWorkspace {
        entry: "story.sf".into(),
        files: BTreeMap::from([
            (
                "story.sf".into(),
                "include \"start.sf\"\ninclude \"end.sf\"\n".into(),
            ),
            (
                "start.sf".into(),
                "scene start\n  narrate: Begin\n  goto finish\n".into(),
            ),
            ("end.sf".into(), end.into()),
        ]),
    };
    let registry = storyfella_studio_core::registry::ArtifactRegistry::new();
    let mut session =
        PlaybackSession::compile_workspace(&workspace, &registry, AssetPolicy::Preview).unwrap();
    assert_eq!(session.on_stage("story.sf").unwrap().0, "start.sf");
    let line = end.find("The end").unwrap();
    assert!(session.play_to("end.sf", line, "story.sf").unwrap());
    let (file, span) = session.on_stage("story.sf").unwrap();
    assert_eq!(file, "end.sf");
    assert!(end[span.start..span.end].contains("The end"));
    assert!(!session.play_to("story.sf", 0, "story.sf").unwrap());
}
