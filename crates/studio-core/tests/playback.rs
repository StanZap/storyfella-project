#[path = "support/playback.rs"]
mod support;
use storyfella_runtime::{Command, Phase};
use storyfella_studio_core::{playback::PlaybackSession, registry::RevisionOrigin};

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
fn both_hosts_follow_identical_events_source_positions_and_pinned_images() {
    let dir = directory();
    let (source, mut registry) = support::fixture(&dir);
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
        assert_eq!(studio.visuals[0].image_url, player.visuals[0].image_url);
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
                    .map(|v| (v.binding.clone(), v.image_url.clone()))
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
    let (source, registry) = support::fixture(&dir);
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
        .all(|v| v.image_url.starts_with("data:image/png;base64,")));
    assert!(
        PlaybackSession::compile(&source.replace("    mara: ", "    shot: "), &registry)
            .unwrap_err()
            .contains("unsupported")
    );
    assert!(PlaybackSession::open(&dir.join("absent.sf"), None).is_err());
    assert!(!dir.join("absent.db").exists());
    std::fs::remove_dir_all(dir).unwrap();
}
