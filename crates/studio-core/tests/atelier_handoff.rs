use std::{fs, path::PathBuf};
use storyfella_studio_core::{
    atelier::{HandoffReply, Mailbox, OpenRequest},
    catalog::{self, CatalogEdit},
    project::ProjectFolder,
    registry::{ArtifactKind, ArtifactRegistry},
};

fn fixture() -> tempfile::TempDir {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../.svs-e2e/tests");
    fs::create_dir_all(&root).unwrap();
    tempfile::tempdir_in(root).unwrap()
}

#[test]
fn handoff_carries_exact_identity_and_relevant_authored_context_without_rewriting_source() {
    let root = fixture();
    let mut registry = ArtifactRegistry::default();
    let id = catalog::apply(
        &mut registry,
        CatalogEdit::Create {
            kind: ArtifactKind::Character,
            name: "mara".into(),
            description: "A captain wearing blue".into(),
            variant_of: None,
        },
    )
    .unwrap();
    let source = "scene harbor\r\n  cast: mara\r\n  beat arrival\r\n    mara: Hello 🌊\r\nscene elsewhere\r\n  beat quiet\r\n    narrate: unrelated text\r\n";
    let project =
        ProjectFolder::create(&root.path().join("story"), "Harbor", &registry, source).unwrap();
    let request = OpenRequest::prepare(&project.database(), source, id).unwrap();
    assert_eq!(request.artifact_id, id);
    assert_eq!(request.project_id, project.id());
    assert_eq!(request.description, "A captain wearing blue");
    assert!(request.context.contains("Hello 🌊"));
    assert!(!request.context.contains("unrelated text"));
    assert_eq!(request.validate().unwrap().id(), project.id());
    assert_eq!(
        fs::read_to_string(project.root().join("story.sf")).unwrap(),
        source
    );
    assert!(OpenRequest::prepare(&project.database(), "unsaved draft", id).is_err());
    let mut invalid = request.clone();
    invalid.project_id = uuid::Uuid::new_v4();
    assert!(invalid.validate().is_err());
    invalid = request.clone();
    invalid.artifact_id = uuid::Uuid::new_v4();
    assert!(invalid.validate().is_err());
    invalid = request.clone();
    invalid.version = 999;
    assert!(invalid.validate().is_err());
    let stored = storyfella_studio_core::persistence::ProjectDb::open_existing(project.database())
        .unwrap()
        .load()
        .unwrap();
    let scene = stored
        .registry
        .story_scenes
        .iter()
        .find(|s| s.name == "harbor")
        .unwrap();
    let request = OpenRequest::prepare(&project.database(), source, scene.artifact_id).unwrap();
    assert!(request.context.contains("Hello 🌊"));
    assert!(!request.context.contains("unrelated text"));
}

#[test]
fn inbox_supports_cold_start_running_receiver_fifo_cancel_and_owned_results() {
    let root = fixture();
    let mailbox = Mailbox::new(root.path());
    assert!(mailbox.take().unwrap().is_none());
    let lease = mailbox.claim().unwrap().unwrap();
    assert!(Mailbox::new(root.path()).claim().unwrap().is_none());
    let first = OpenRequest {
        version: 1,
        id: uuid::Uuid::new_v4(),
        project: "storyfella.toml".into(),
        project_id: uuid::Uuid::new_v4(),
        artifact_id: uuid::Uuid::new_v4(),
        context: "context".into(),
        description: "description".into(),
    };
    let mut second = first.clone();
    second.id = uuid::Uuid::new_v4();
    mailbox.send(&first).unwrap();
    mailbox.send(&second).unwrap();
    assert_eq!(mailbox.take().unwrap().unwrap().id, first.id);
    mailbox.cancel(second.id);
    assert!(mailbox.take().unwrap().is_none());
    mailbox.complete(first.id, Ok(())).unwrap();
    assert_eq!(mailbox.reply(first.id).unwrap(), Some(HandoffReply::Opened));
    assert!(mailbox.reply(first.id).unwrap().is_none());
    mailbox
        .complete(second.id, Err(anyhow::anyhow!("Save the mask first")))
        .unwrap();
    assert_eq!(
        mailbox.reply(second.id).unwrap(),
        Some(HandoffReply::Failed("Save the mask first".into()))
    );
    drop(lease);
    assert!(mailbox.claim().unwrap().is_some());
    fs::write(root.path().join("handoff/invalid.request"), "{broken").unwrap();
    assert!(mailbox.take().is_err());
    assert!(mailbox.take().unwrap().is_none());
}
