//! A model-free chapter through the same storage and playback paths the apps use.
use super::*;
use crate::{
    document::{Brush, Point, Reference, Stroke, Tool},
    edit::MaskDraft,
    workflow::{image_layer, Destination},
};
use base64::{engine::general_purpose::STANDARD, Engine};
use image::{Rgba, RgbaImage};
use std::{fs, sync::Arc};
use storyfella_runtime::{Command, Event, Phase};
use storyfella_studio_core::{
    catalog::CatalogEdit,
    persistence::{ProjectDb, StoreError},
    playback::PlaybackSession,
    registry::{ArtifactKind, ArtifactRegistry},
    story::AdditionalSources,
};

const CHAPTER: &str = r#"flag trusted false
scene pier
  location: harbor
  stage: captain, mara, left, 100, 2
  stage: witness, tam, center, 90, 1
  stage: mechanic, io, right, 85, 0
  mara: The signal is late.
  choice "Hand over the code?"
    option "Trust Mara": share
    option "Keep it": gate
scene share
  set trusted true
  goto gate
scene gate
  if trusted goto welcome else warning
scene welcome
  location: signal_room
  stage: captain, mara, left, 100, 2
  swap: captain, mara_smile
  mara: We can light the channel.
  end
scene warning
  location: signal_room
  stage: captain, mara, left, 100, 2
  swap: captain, mara_wary
  mara: The channel stays dark.
  end
"#;

fn render_pixels(doc: &Document) -> Result<RgbaImage> {
    document::raster(doc, doc.width.max(doc.height), true)
}

fn reference(name: &str, color: [u8; 4]) -> Reference {
    let mut png = Cursor::new(Vec::new());
    RgbaImage::from_pixel(128, 96, Rgba(color))
        .write_to(&mut png, image::ImageFormat::Png)
        .unwrap();
    document::reference_from_bytes(name.into(), &png.into_inner(), "Offline fixture".into())
        .unwrap()
}

fn image_pixels(session: &PlaybackSession, name: &str) -> RgbaImage {
    let image = session
        .visuals
        .iter()
        .find(|visual| visual.binding.name == name)
        .unwrap();
    let encoded = image
        .image_url
        .strip_prefix("data:image/png;base64,")
        .unwrap();
    image::load_from_memory(&STANDARD.decode(encoded).unwrap())
        .unwrap()
        .to_rgba8()
}

#[test]
fn branching_chapter_survives_partial_publication_and_stale_authoring() {
    let scratch = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.svs-e2e/tests");
    fs::create_dir_all(&scratch).unwrap();
    let fixture = tempfile::tempdir_in(scratch).unwrap();
    let folder = ProjectFolder::create(
        &fixture.path().join("signal"),
        "The Last Signal",
        &ArtifactRegistry::default(),
        "scene draft\n  end\n",
    )
    .unwrap();
    let db = ProjectDb::open_existing(folder.database()).unwrap();
    let mut author = db.load().unwrap();
    let source = author.story.path().unwrap().to_owned();
    let mut sources = AdditionalSources::new(&source).unwrap();
    let chapter = sources
        .create(&source.with_file_name("chapter.sf"))
        .unwrap();
    sources.set_source(&chapter, CHAPTER.into()).unwrap();
    author.story.set_source("include \"chapter.sf\"\n".into());
    let path = document::project_document(&folder.manifest()).unwrap();
    let mut doc = document::load(&path).unwrap();
    let mut mara = None;
    let mut wary = None;
    for (name, kind, is_variant, color) in [
        ("mara", ArtifactKind::Character, false, [10, 40, 90, 255]),
        ("tam", ArtifactKind::Character, false, [90, 150, 50, 255]),
        ("io", ArtifactKind::Character, false, [100, 50, 180, 255]),
        (
            "harbor",
            ArtifactKind::Environment,
            false,
            [25, 80, 120, 255],
        ),
        (
            "signal_room",
            ArtifactKind::Environment,
            false,
            [190, 160, 100, 255],
        ),
        (
            "mara_smile",
            ArtifactKind::Character,
            true,
            [70, 140, 180, 255],
        ),
        (
            "mara_wary",
            ArtifactKind::Character,
            true,
            [10, 40, 90, 255],
        ),
    ] {
        let id = catalog::apply(
            &mut author.registry,
            CatalogEdit::Create {
                kind,
                name: name.into(),
                description: String::new(),
                variant_of: if is_variant { mara } else { None },
            },
        )
        .unwrap();
        if name == "mara" {
            mara = Some(id);
        } else if name == "mara_wary" {
            wary = Some(id);
        }
        doc.ensure_story_artboard(id, String::new(), String::new())
            .unwrap();
        doc.width = 128;
        doc.height = 96;
        doc.layers[0] = image_layer(
            Arc::new(reference(name, color)),
            128,
            96,
            (0, 0, 128, 96),
            false,
        )
        .unwrap();
    }
    sources.save().unwrap();
    db.save_authoring_checked(&author.registry, author.registry_version, &mut author.story)
        .unwrap();
    let mut stale = db.load().unwrap();
    assert!(PlaybackSession::open(folder.root(), None).is_err());

    let mask = MaskDraft {
        width: 128,
        height: 96,
        strokes: vec![Stroke {
            color: crate::document::default_ink(),
            shape: None,
            brush: Brush::Ink,
            pressures: vec![].into(),
            tool: Tool::Rect,
            size: 12.,
            opacity: 1.,
            points: vec![Point { x: 20., y: 20. }, Point { x: 80., y: 60. }].into(),
        }],
    };
    let before = doc.generation_source(None).unwrap();
    let candidate = Arc::new(
        mask.composite(&before, &reference("Edit", [240, 20, 30, 255]))
            .unwrap(),
    );
    let destination = |rect| Destination::Candidate {
        artboard: doc.canvas_id,
        rect,
    };
    let invalid = destination((120, 90, 8, 8));
    let valid = destination((0, 0, 128, 96));
    assert!(doc.attach_result(candidate.clone(), invalid).is_err());
    doc.attach_result(candidate, valid).unwrap();
    let pending = save_with_renderer(&path, &doc, render_pixels).unwrap();
    assert_eq!(pending.len(), 7);
    let pinned = PlaybackSession::open(folder.root(), None).unwrap();
    assert_eq!(pinned.runtime.staged().len(), 3);
    assert_eq!(
        image_pixels(&pinned, "mara_wary").get_pixel(40, 40).0,
        [10, 40, 90, 255]
    );

    let mut doc = document::load(&path).unwrap();
    assert!(doc.layers[doc.active_layer].candidate);
    doc.layers[doc.active_layer].candidate = false;
    // Prevent the final atomic document write after publication. The first
    // document write has already persisted the accepted edit.
    let blocked = path.with_file_name(format!(".main.atelier.{}.tmp", std::process::id()));
    let error = save_with_renderer(&path, &doc, |canvas| {
        if !blocked.exists() {
            fs::create_dir(&blocked)?;
        }
        render_pixels(canvas)
    })
    .unwrap_err();
    assert!(matches!(
        error.downcast_ref::<SaveStage>(),
        Some(SaveStage::RevisionLinks(_))
    ));
    fs::remove_dir(blocked).unwrap();
    let committed = db.load().unwrap();
    let wary = wary.unwrap();
    let revision = committed
        .registry
        .artifact(wary)
        .unwrap()
        .active_revision_id
        .unwrap();
    let publications = committed_publications(&error);
    assert!(publications
        .iter()
        .any(|p| p.asset_id == wary && p.revision_id == revision));
    let canvas = doc.canvas_id;
    let mut relinked = doc.clone();
    let newer_baseline = Some(uuid::Uuid::new_v4());
    relinked.artwork_bases.insert(canvas, newer_baseline);
    apply_publications(&mut relinked, publications);
    assert_eq!(relinked.artwork_bases[&canvas], newer_baseline);
    apply_publications(&mut doc, publications);
    assert_eq!(doc.artwork_bases[&canvas], Some(revision));
    let recovered = document::load(&path).unwrap();
    assert!(!recovered.layers[recovered.active_layer].candidate);
    assert_ne!(
        recovered.artwork_bases[&recovered.canvas_id],
        Some(revision)
    );
    save_with_renderer(&path, &recovered, render_pixels).unwrap();
    assert_eq!(
        db.load().unwrap(),
        committed,
        "retry keeps the same registry and revisions"
    );
    assert_eq!(
        document::load(&path).unwrap().artwork_bases[&recovered.canvas_id],
        Some(revision)
    );

    stale
        .story
        .set_source("// stale authoring edit\ninclude \"chapter.sf\"\n".into());
    assert!(matches!(
        db.save_authoring_checked(&stale.registry, stale.registry_version, &mut stale.story),
        Err(StoreError::Conflict)
    ));
    assert!(stale.story.is_dirty());
    assert_eq!(
        fs::read_to_string(source).unwrap(),
        "include \"chapter.sf\"\n"
    );
    assert_eq!(db.load().unwrap(), committed);

    for (index, trusted, variant, dialogue) in [
        (0, true, "mara_smile", "We can light the channel."),
        (1, false, "mara_wary", "The channel stays dark."),
    ] {
        let mut session = PlaybackSession::open(folder.root(), None).unwrap();
        session.dispatch(Command::Advance).unwrap();
        assert_eq!(session.runtime.state().phase, Phase::Choice);
        session.dispatch(Command::SelectChoice { index }).unwrap();
        assert_eq!(session.runtime.state().flags["trusted"], trusted);
        assert_eq!(session.runtime.staged()[0].asset.name.value, variant);
        assert!(session
            .active_visuals()
            .iter()
            .any(|visual| visual.binding.name == "signal_room"));
        assert!(session
            .last_events
            .iter()
            .any(|event| matches!(event, Event::Text { text, .. } if text.value == dialogue)));
        let instruction = session.runtime.current().source;
        let map = session.runtime.plan().source_map.as_ref().unwrap();
        assert_eq!(map.original_span(instruction).unwrap().0, "chapter.sf");
        let saved = session.runtime.save();
        session.dispatch(Command::Advance).unwrap();
        assert_eq!(session.runtime.state().phase, Phase::Complete);
        session.restore(saved).unwrap();
        assert_eq!(session.runtime.state().flags["trusted"], trusted);
        assert_eq!(session.runtime.current().source, instruction);
        if !trusted {
            let pixels = image_pixels(&session, variant);
            assert_eq!(pixels.get_pixel(40, 40).0, [240, 20, 30, 255]);
            assert_eq!(pixels.get_pixel(10, 10).0, [10, 40, 90, 255]);
        }
    }
    assert_eq!(
        image_pixels(&pinned, "mara_wary").get_pixel(40, 40).0,
        [10, 40, 90, 255]
    );
}
