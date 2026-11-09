use super::*;
use std::time::Duration;

use agentkit_mcp::rmcp::model::{ClientInfo, RawContent};
use image::{Rgba, RgbaImage};
use storyfella_runtime::AssetPolicy;
use storyfella_studio_core::{
    playback::PlaybackSession,
    registry::{ArtifactRegistry, CreateArtifact},
};

const SOURCE: &str =
    "scene vault\n  show illustration seed_vault\n  narrate: The vault opens.\n  end\n";
const FIRST: [u8; 4] = [32, 88, 151, 255];
const REVISED: [u8; 4] = [235, 164, 65, 255];

fn fixture() -> tempfile::TempDir {
    let scratch = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.svs-e2e/tests");
    fs::create_dir_all(&scratch).unwrap();
    tempfile::tempdir_in(scratch).unwrap()
}

fn setup(root: &Path) -> (ProjectFolder, Uuid) {
    let mut registry = ArtifactRegistry::default();
    let illustration = registry
        .create_artifact(
            ArtifactKind::Illustration,
            CreateArtifact {
                name: "seed_vault".into(),
                ..Default::default()
            },
        )
        .unwrap();
    let folder =
        ProjectFolder::create(root, "Vault illustration fixture", &registry, SOURCE).unwrap();
    (folder, illustration)
}

fn args(folder: &ProjectFolder, illustration: Uuid, data_dir: PathBuf) -> Args {
    Args {
        folder: folder.root().into(),
        illustration,
        data_dir: Some(data_dir),
        allow_generate: true,
        allow_publish: true,
    }
}

fn fixture_provider(request: Request) -> Result<Reference> {
    let color = match request.prompt.as_str() {
        "Draw the vault in cool light" => {
            assert!(request.source.is_none());
            FIRST
        }
        "Make the vault glow amber" => {
            let original =
                document::decode_reference(request.source.as_deref().unwrap())?.to_rgba8();
            assert_eq!(original.get_pixel(40, 30).0, FIRST);
            REVISED
        }
        other => panic!("Unexpected fixture prompt {other}"),
    };
    let mut png = Cursor::new(Vec::new());
    RgbaImage::from_fn(128, 80, |x, y| {
        if (12..116).contains(&x) && (10..70).contains(&y) {
            Rgba(color)
        } else {
            Rgba([15, 20, 28, 255])
        }
    })
    .write_to(&mut png, image::ImageFormat::Png)?;
    document::reference_from_bytes(
        "Vault fixture".into(),
        &png.into_inner(),
        "Deterministic provider fixture".into(),
    )
}

async fn exercise(root: &Path) {
    let (folder, illustration) = setup(&root.join("story"));
    let data_dir = root.join("atelier-data");
    let mut state = Tools::open(args(&folder, illustration, data_dir.clone())).unwrap();
    state.dispatch = |request, id, sender, dir| {
        worker::generate_using(request, id, sender, dir, fixture_provider);
    };
    // The fixture replaces only inference. The normal capability checks and
    // worker, durable run, result store and checked publication still run.
    state.settings.provider = providers::Provider::GptImage;
    state.settings.model = "gpt-image-1".into();
    state.settings.edit_model = "gpt-image-1".into();
    state.settings.width = 1024;
    state.settings.height = 1024;
    let (server_io, client_io) = tokio::io::duplex(64 * 1024);
    let (server, client) = tokio::join!(
        Server(Arc::new(Mutex::new(state))).serve(server_io),
        ClientInfo::default().serve(client_io)
    );
    let server = server.unwrap();
    let client = client.unwrap();
    assert_eq!(client.peer().list_all_tools().await.unwrap().len(), 5);
    let call = |name: &'static str, arguments: Value| {
        let client = &client;
        async move {
            let params = CallToolRequestParams::new(name)
                .with_arguments(arguments.as_object().unwrap().clone());
            client.peer().call_tool(params).await.unwrap()
        }
    };
    let inspected = fields(&call(INSPECT, json!({})).await);
    let session = &inspected["session"];
    let generate = async |version: &Value, prompt: &str, revise| {
        let response = fields(&call(GENERATE, json!({
            "session": session, "expected_version": version, "prompt": prompt, "revise": revise,
        })).await);
        let job = &response["job"];
        tokio::time::timeout(Duration::from_secs(20), async {
            loop {
                let progress = fields(&call(POLL, json!({"session": session, "job": job})).await);
                match progress["status"].as_str() {
                    Some("done") => break progress["version"].clone(),
                    Some("running") => tokio::time::sleep(Duration::from_millis(15)).await,
                    _ => panic!("{progress}"),
                }
            }
        })
        .await
        .unwrap()
    };
    assert!(folder.claim_authoring().is_err());
    assert!(Owner::desktop(folder.clone(), root, None).is_err());
    assert!(Tools::open(args(&folder, illustration, data_dir.clone())).is_err());

    let first = generate(&inspected["version"], "Draw the vault in cool light", false).await;
    let first_response = call(
        PREVIEW,
        json!({"session": session, "expected_version": first}),
    )
    .await;
    let first_review = fields(&first_response)["review"].clone();
    assert_eq!(pixels(&first_response).get_pixel(180, 180).0, FIRST);
    assert!(fields(&first_response)["current_artwork_revision"].is_null());
    assert!(
        call(PUBLISH, json!({"session": session, "review": first_review}))
            .await
            .is_error
            .unwrap()
    );
    let first_id = serde_json::from_value(first_review.clone()).unwrap();
    record_approval(Approve {
        folder: folder.root().into(),
        review: first_id,
    })
    .unwrap();

    let second = generate(&first, "Make the vault glow amber", true).await;
    assert!(call(
        PREVIEW,
        json!({"session": session, "expected_version": first})
    )
    .await
    .is_error
    .unwrap());
    assert!(
        call(PUBLISH, json!({"session": session, "review": first_review}))
            .await
            .is_error
            .unwrap()
    );
    let final_response = call(
        PREVIEW,
        json!({"session": session, "expected_version": second}),
    )
    .await;
    let final_pixels = pixels(&final_response);
    assert_eq!(final_pixels.get_pixel(180, 180).0, REVISED);
    assert!(folder
        .load()
        .unwrap()
        .registry
        .artifact(illustration)
        .unwrap()
        .revisions
        .is_empty());
    let review = fields(&final_response)["review"].clone();
    assert!(call(PUBLISH, json!({"session": session, "review": review}))
        .await
        .is_error
        .unwrap());
    let review_id = serde_json::from_value(review.clone()).unwrap();
    record_approval(Approve {
        folder: folder.root().into(),
        review: review_id,
    })
    .unwrap();

    // Simulate an artwork write failure after the accepted document saved.
    let artwork = folder.root().join("artwork");
    fs::remove_dir(&artwork).unwrap();
    fs::write(&artwork, b"blocked").unwrap();
    let blocked = call(PUBLISH, json!({"session": session, "review": review})).await;
    assert!(blocked.is_error.unwrap(), "{blocked:?}");
    assert_eq!(blocked.structured_content.unwrap()["stage"], "artwork");
    fs::remove_file(&artwork).unwrap();
    fs::create_dir(&artwork).unwrap();
    assert!(!approval_path(&folder, review_id).exists());
    let published = fields(&call(PUBLISH, json!({"session": session, "review": review})).await);
    client.cancel().await.unwrap();
    server.waiting().await.unwrap();
    let _owner = folder.claim_authoring().unwrap();

    let stored = folder.load().unwrap();
    let item = stored.registry.artifact(illustration).unwrap();
    assert_eq!(item.revisions.len(), 1);
    assert_eq!(
        published["revision"],
        item.active_revision_id.unwrap().to_string()
    );
    let saved = document::load(&folder.atelier_document().unwrap()).unwrap();
    assert_eq!(
        saved.artwork_bases[&saved.canvas_id],
        item.active_revision_id
    );
    assert!(saved.layers.iter().all(|layer| !layer.candidate));

    // The same compilation and exact-image loader as Studio Preview.
    let preview = PlaybackSession::compile_with_policy(
        stored.story.source(),
        &stored.registry,
        AssetPolicy::Preview,
    )
    .unwrap();
    let visual = preview.active_visuals()[0];
    assert_eq!(visual.binding.id, illustration.to_string());
    assert_eq!(visual.binding.revision, published["revision"]);
    let preview_png = STANDARD
        .decode(
            visual
                .image_url
                .strip_prefix("data:image/png;base64,")
                .unwrap(),
        )
        .unwrap();
    let saved_png = fs::read(item.revisions[0].asset_path.as_ref().unwrap()).unwrap();
    let preview_pixels = image::load_from_memory(&preview_png).unwrap().to_rgba8();
    assert_eq!(
        preview_pixels,
        image::load_from_memory(&saved_png).unwrap().to_rgba8()
    );
    assert_eq!(preview_pixels.get_pixel(360, 360).0, REVISED);
    println!(
        "Project {}\nIllustration {illustration}\nPublished {}",
        folder.root().display(),
        published["revision"]
    );
}

fn fields(result: &CallToolResult) -> Value {
    assert_ne!(result.is_error, Some(true), "{result:?}");
    result.structured_content.clone().unwrap()
}

fn pixels(result: &CallToolResult) -> RgbaImage {
    let image = result
        .content
        .iter()
        .find_map(|block| {
            if let RawContent::Image(image) = &block.raw {
                Some(image)
            } else {
                None
            }
        })
        .expect("Native MCP image block");
    assert_eq!(image.mime_type, "image/png");
    image::load_from_memory(&STANDARD.decode(&image.data).unwrap())
        .unwrap()
        .to_rgba8()
}

#[tokio::test(flavor = "multi_thread")]
async fn revised_candidate_needs_fresh_review_and_reopens_at_the_exact_revision() {
    exercise(fixture().path()).await;
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "Keeps a disposable project for the native Studio Preview check"]
async fn keep_project_for_desktop_preview() {
    let root = fixture().keep();
    exercise(&root).await;
}

#[test]
fn desktop_recovery_reserves_the_drawing_across_data_directories() {
    let scratch = fixture();
    let root = scratch.path();
    let (folder, illustration) = setup(&root.join("story"));
    let desktop = root.join("desktop-a");
    let alternate = root.join("desktop-b");
    fs::create_dir(&desktop).unwrap();
    fs::create_dir(&alternate).unwrap();
    let owner = Owner::desktop(folder.clone(), &desktop, None).unwrap();
    let path = folder.atelier_document().unwrap();
    document::initialize_shared(&folder, "Saved title".into()).unwrap();
    let mut dirty = document::load(&path).unwrap();
    dirty.title = "Dirty draft".into();
    crate::project_tabs::save_session(
        &desktop.join("open-projects"),
        vec![(
            crate::project_tabs::RecoveryTab {
                shared: owner.recovery_identity(),
                epoch: 7,
                path: Some(path.clone()),
                dirty: true,
                active: true,
            },
            dirty,
        )],
    )
    .unwrap();
    assert!(Owner::desktop(folder.clone(), &alternate, None).is_err());
    assert!(Tools::open(args(&folder, illustration, root.join("mcp"))).is_err());
    drop(owner);
    assert!(Owner::desktop(folder.clone(), &alternate, None).is_err());
    assert!(Tools::open(args(&folder, illustration, root.join("mcp"))).is_err());
    let restored = crate::project_tabs::recover(&desktop)
        .unwrap()
        .active
        .unwrap();
    assert!(restored.dirty);
    assert_eq!(restored.doc.title, "Dirty draft");
    assert_eq!(document::load(&path).unwrap().title, "Saved title");
    let owner = restored.owner.unwrap();
    ownership::verify_write(&path, Some(&owner)).unwrap();
    shared_save::save(&path, &restored.doc).unwrap();
    owner.release_desktop().unwrap();
    drop(owner);
    let mcp = Tools::open(args(&folder, illustration, root.join("mcp"))).unwrap();
    assert_eq!(mcp.snapshot().unwrap().doc.title, "Dirty draft");
    let detached = crate::project_tabs::recover(&desktop)
        .unwrap()
        .active
        .unwrap();
    assert!(detached.path.is_none());
    assert_eq!(detached.doc.title, "Dirty draft");
}
