use super::*;
use std::time::Duration;

use agentkit_mcp::rmcp::model::{ClientInfo, RawContent};
use image::{Rgba, RgbaImage};
use storyfella_runtime::AssetPolicy;
use storyfella_studio_core::{
    playback::PlaybackSession,
    registry::{ArtifactKind, ArtifactRegistry, CreateArtifact},
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

fn args(folder: &ProjectFolder, data_dir: PathBuf) -> Args {
    Args {
        folder: folder.root().into(),
        max_submissions: None,
        data_dir: Some(data_dir),
        allow_generate: true,
        allow_publish: true,
        atelier_channel: None,
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
        "Light the variant from the published artwork" => {
            let original =
                document::decode_reference(request.source.as_deref().unwrap())?.to_rgba8();
            assert_eq!(original.get_pixel(512, 512).0, FIRST);
            REVISED
        }
        "Use the saved painting" => {
            let original =
                document::decode_reference(request.source.as_deref().unwrap())?.to_rgba8();
            assert_eq!(original.get_pixel(40, 30).0, FIRST);
            assert_eq!(original.get_pixel(100, 30)[3], 0);
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

fn late_result_after_cancel(request: Request) -> Result<Reference> {
    request.progress.as_ref().unwrap()("Fixture provider started".into());
    let start = std::time::Instant::now();
    while !request.cancel.load(Ordering::Relaxed) {
        ensure!(
            start.elapsed() < Duration::from_secs(5),
            "Fixture did not receive cancellation"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
    fixture_provider(request)
}

#[test]
fn generation_from_saved_canvas_sends_handmade_artwork_and_keeps_it_editable() {
    use crate::{
        document::Point,
        shapes::{self, ShapeKind},
    };
    let scratch = fixture();
    let (folder, asset) = setup(&scratch.path().join("story"));
    let desktop = scratch.path().join("desktop");
    fs::create_dir(&desktop).unwrap();
    let owner = Owner::desktop(folder.clone(), &desktop, None).unwrap();
    let path = document::project_document(&folder.manifest()).unwrap();
    let mut doc = document::load(&path).unwrap();
    doc.create_canvas().unwrap();
    let source_artboard = doc.canvas_id;
    doc.width = 1024;
    doc.height = 1024;
    let mark = shapes::stroke(
        ShapeKind::Rectangle,
        Point { x: 20., y: 20. },
        Point { x: 80., y: 60. },
        FIRST,
        [0; 4],
        1.,
    )
    .unwrap();
    let shape_id = mark.shape.as_ref().unwrap().id;
    doc.add_mark(0, mark).unwrap();
    shared_save::save(&path, &doc).unwrap();
    owner.release_desktop().unwrap();
    drop(owner);

    let mut tools = Tools::open(args(&folder, scratch.path().join("data"))).unwrap();
    tools.settings.provider = providers::Provider::GptImage;
    tools.settings.edit_model = "gpt-image-1".into();
    tools.provider = fixture_provider;
    assert_eq!(
        tools.project(Project { offset: 0 }).unwrap()["artboards"][0]["artboard"],
        source_artboard
    );
    let response = tools
        .generate(Generate {
            session: tools.session,
            asset,
            expected_version: tools.snapshot(asset).unwrap().version,
            prompt: "Use the saved painting".into(),
            revise: false,
            source_artboard: Some(source_artboard),
            reference_revision: None,
            size: Some([1024, 1024]),
        })
        .unwrap();
    let id = serde_json::from_value::<Uuid>(response["job"].clone()).unwrap();
    let start = std::time::Instant::now();
    loop {
        let result = tools
            .poll(Poll {
                session: tools.session,
                job: id,
            })
            .unwrap();
        if result["status"] == "done" {
            break;
        }
        assert!(start.elapsed() < Duration::from_secs(10), "{result}");
        std::thread::sleep(Duration::from_millis(10));
    }
    let mut reopened = document::load(&path).unwrap();
    assert_ne!(linked_canvas(&reopened, asset), Some(source_artboard));
    assert!(reopened.layers.iter().any(|layer| layer.candidate));
    reopened.select_canvas(source_artboard).unwrap();
    assert_eq!(reopened.shape_location(shape_id), Some((0, 0)));
    assert_eq!(
        reopened
            .generation_pixels(None)
            .unwrap()
            .get_pixel(40, 30)
            .0,
        FIRST
    );
}

#[test]
fn cancellation_suppresses_late_attachment_and_unknown_submission_stays_unknown() {
    let scratch = fixture();
    let (folder, asset) = setup(&scratch.path().join("story"));
    let data = scratch.path().join("data");
    let open = || Tools::open(args(&folder, data.clone())).unwrap();
    let mut tools = open();
    tools.settings.provider = providers::Provider::GptImage;
    tools.settings.model = "gpt-image-1".into();
    let submit = |tools: &mut Tools| {
        let result = tools
            .generate(Generate {
                session: tools.session,
                asset,
                expected_version: tools.snapshot(asset).unwrap().version,
                prompt: "Draw the vault in cool light".into(),
                revise: false,
                source_artboard: None,
                reference_revision: None,
                size: None,
            })
            .unwrap();
        serde_json::from_value::<Uuid>(result["job"].clone()).unwrap()
    };
    tools.provider =
        |_| anyhow::bail!("Fixture lost connection before receiving a provider receipt");
    let unknown = submit(&mut tools);
    let wait = |tools: &mut Tools| {
        let start = std::time::Instant::now();
        while tools.queue.outstanding() != 0 {
            assert!(start.elapsed() < Duration::from_secs(15));
            tools.refresh_queue();
            std::thread::sleep(Duration::from_millis(5));
        }
    };
    wait(&mut tools);
    drop(tools);

    let mut tools = open();
    let status = tools
        .poll(Poll {
            session: tools.session,
            job: unknown,
        })
        .unwrap();
    assert_eq!(status["status"], "unknown");
    assert_eq!(tools.submissions().unwrap().len(), 1);
    tools.settings.provider = providers::Provider::GptImage;
    tools.settings.model = "gpt-image-1".into();
    tools.provider = late_result_after_cancel;
    let job = submit(&mut tools);
    let start = std::time::Instant::now();
    loop {
        let status = tools
            .poll(Poll {
                session: tools.session,
                job,
            })
            .unwrap();
        if status["progress"] == "Fixture provider started" {
            break;
        }
        assert!(start.elapsed() < Duration::from_secs(5));
        std::thread::sleep(Duration::from_millis(5));
    }
    let status = tools
        .cancel(Poll {
            session: tools.session,
            job,
        })
        .unwrap();
    assert_eq!(status["status"], "cancelling");
    assert_eq!(status["remote_cancellation"], "unconfirmed");
    wait(&mut tools);
    drop(tools);

    let mut tools = open();
    tools.provider = |_| panic!("Polling or recovery must never resubmit to a provider");
    let session = tools.session;
    let status = tools.poll(Poll { session, job }).unwrap();
    assert_eq!(status["status"], "retained");
    assert_eq!(status["attachment_suppressed"], true);
    assert!(candidate(&tools.snapshot(asset).unwrap().doc)
        .unwrap()
        .is_none());
    assert_eq!(
        pixels(&tools.thumbnail(Poll { session, job }).unwrap())
            .get_pixel(40, 30)
            .0,
        FIRST
    );
    let recovered = tools
        .recover(Recover {
            session,
            job,
            expected_version: status["version"].as_str().unwrap().into(),
        })
        .unwrap();
    assert_eq!(recovered["status"], "done");
    assert_eq!(tools.submissions().unwrap().len(), 2);
}

async fn exercise(root: &Path) {
    let (folder, illustration) = setup(&root.join("story"));
    let data_dir = root.join("atelier-data");
    let mut state = Tools::open(args(&folder, data_dir.clone())).unwrap();
    state.provider = fixture_provider;
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
    let tools = client.peer().list_all_tools().await.unwrap();
    assert_eq!(tools.len(), 9);
    assert!(tools.iter().any(|tool| tool.name == CANCEL));
    let call = |name: &'static str, arguments: Value| {
        let client = &client;
        async move {
            let params = CallToolRequestParams::new(name)
                .with_arguments(arguments.as_object().unwrap().clone());
            client.peer().call_tool(params).await.unwrap()
        }
    };
    let project = fields(&call(PROJECT, json!({})).await);
    let inspected = fields(&call(INSPECT, json!({"asset": illustration})).await);
    let session = &project["session"];
    let generate = async |version: &Value, prompt: &str, revise| {
        let response = fields(&call(GENERATE, json!({
            "session": session, "asset": illustration, "expected_version": version, "prompt": prompt, "revise": revise,
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
    assert!(Tools::open(args(&folder, data_dir.clone())).is_err());

    let first = generate(&inspected["version"], "Draw the vault in cool light", false).await;
    let first_response = call(
        PREVIEW,
        json!({"session": session, "asset": illustration, "expected_version": first}),
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
        json!({"session": session, "asset": illustration, "expected_version": first})
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
        json!({"session": session, "asset": illustration, "expected_version": second}),
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

#[test]
fn retained_variant_recovers_after_restart_and_approval_publishes_only_its_artboard() {
    use storyfella_studio_core::catalog::{self, CatalogEdit};

    let scratch = fixture();
    let root = scratch.path().join("story");
    let mut registry = ArtifactRegistry::default();
    let mut create = |name: &str, variant_of| {
        catalog::apply(
            &mut registry,
            CatalogEdit::Create {
                name: name.into(),
                kind: ArtifactKind::Character,
                description: String::new(),
                variant_of,
            },
        )
        .unwrap()
    };
    let base = create("keeper", None);
    let variant = create("keeper_wary", Some(base));
    let other = create("courier", None);
    let folder =
        ProjectFolder::create(&root, "Signal fixture", &registry, "scene signal\n  end\n").unwrap();
    let data = scratch.path().join("data");
    let settings = Settings {
        provider: providers::Provider::GptImage,
        model: "gpt-image-1".into(),
        edit_model: "gpt-image-1".into(),
        ..Settings::default()
    };
    worker::apply_settings(&settings, &data.join("settings.json")).unwrap();
    let open = || {
        let mut args = args(&folder, data.clone());
        args.max_submissions = Some(2);
        Tools::open(args).unwrap()
    };
    let mut tools = open();
    tools.provider = fixture_provider;
    let submit = |tools: &mut Tools, asset, prompt: &str, reference_revision| {
        tools
            .generate(Generate {
                session: tools.session,
                asset,
                expected_version: tools.snapshot(asset).unwrap().version,
                prompt: prompt.into(),
                revise: false,
                source_artboard: None,
                reference_revision,
                size: None,
            })
            .map(|value| serde_json::from_value::<Uuid>(value["job"].clone()).unwrap())
    };
    let wait = |tools: &Tools, id| {
        let start = std::time::Instant::now();
        while tools.result(id).unwrap().is_none() {
            assert!(start.elapsed() < Duration::from_secs(15));
            std::thread::sleep(Duration::from_millis(15));
        }
    };
    let approve = |tools: &mut Tools, asset| {
        let preview = tools
            .preview(Checked {
                session: tools.session,
                asset,
                expected_version: tools.snapshot(asset).unwrap().version,
            })
            .unwrap();
        let review = serde_json::from_value::<Uuid>(fields(&preview)["review"].clone()).unwrap();
        record_approval(Approve {
            folder: root.clone(),
            review,
        })
        .unwrap();
        review
    };

    let base_job = submit(&mut tools, base, "Draw the vault in cool light", None).unwrap();
    wait(&tools, base_job);
    let start = std::time::Instant::now();
    loop {
        let status = tools
            .poll(Poll {
                session: tools.session,
                job: base_job,
            })
            .unwrap();
        if status["status"] == "done" {
            break;
        }
        assert_eq!(status["status"], "running");
        assert!(start.elapsed() < Duration::from_secs(15));
        std::thread::sleep(Duration::from_millis(15));
    }
    let review = approve(&mut tools, base);
    let published = tools
        .publish(Publish {
            session: tools.session,
            review,
            agent_approval: false,
        })
        .unwrap();
    let base_revision =
        serde_json::from_value::<Uuid>(fields(&published)["revision"].clone()).unwrap();
    assert!(submit(
        &mut tools,
        variant,
        "Invalid reference",
        Some(Uuid::new_v4())
    )
    .is_err());
    let job = submit(
        &mut tools,
        variant,
        "Light the variant from the published artwork",
        Some(base_revision),
    )
    .unwrap();
    wait(&tools, job);
    let old_session = tools.session;
    let path = tools.path.clone();
    drop(tools);

    // Another owner changed the drawing while the completed result awaited
    // attachment. The unrelated artboard already contains accepted paint.
    let mut doc = document::load(&path).unwrap();
    doc.ensure_story_artboard(other, String::new(), String::new())
        .unwrap();
    doc.attach_result(
        Arc::new(
            fixture_provider(Request {
                settings: settings.clone(),
                native: Arc::default(),
                prompt: "Draw the vault in cool light".into(),
                source: None,
                references: vec![],
                mask: None,
                progress: None,
                size: (doc.width, doc.height),
                cancel: Arc::default(),
            })
            .unwrap(),
        ),
        Destination::Candidate {
            artboard: doc.canvas_id,
            rect: (0, 0, doc.width, doc.height),
        },
    )
    .unwrap();
    doc.review_candidate_layer(Review::Accept);
    document::save(&path, &doc).unwrap();

    let mut tools = open();
    tools.provider = |_| panic!("Recovery must not submit to a provider");
    assert!(tools
        .poll(Poll {
            session: old_session,
            job
        })
        .is_err());
    let session = tools.session;
    let status = tools.poll(Poll { session, job }).unwrap();
    assert_eq!(status["status"], "retained");
    let limit = submit(&mut tools, variant, "Another variant", Some(base_revision)).unwrap_err();
    assert!(limit.to_string().contains("submission limit"));
    let thumbnail = tools.thumbnail(Poll { session, job }).unwrap();
    assert_eq!(pixels(&thumbnail).get_pixel(40, 30).0, REVISED);
    assert_eq!(
        fields(&thumbnail)["reference_revision"],
        base_revision.to_string()
    );
    assert!(tools
        .recover(Recover {
            session,
            job,
            expected_version: "stale".into()
        })
        .is_err());
    let recovery = tools
        .recover(Recover {
            session,
            job,
            expected_version: status["version"].as_str().unwrap().into(),
        })
        .unwrap();
    assert_eq!(recovery["status"], "done");
    assert_eq!(
        tools.poll(Poll { session, job }).unwrap()["version"],
        recovery["version"]
    );

    let variant_review = approve(&mut tools, variant);
    let other_review = approve(&mut tools, other);
    let publication = tools
        .publish(Publish {
            session,
            review: variant_review,
            agent_approval: false,
        })
        .unwrap();
    assert_eq!(fields(&publication)["asset"], variant.to_string());
    let stored = folder.load().unwrap();
    assert_eq!(stored.registry.artifact(base).unwrap().revisions.len(), 1);
    assert!(stored.registry.active_revision(other).is_none());
    fs::remove_file(approval_path(&folder, other_review)).unwrap();
    let decision = || Publish {
        session,
        review: other_review,
        agent_approval: true,
    };
    let publication = tools.publish(decision()).unwrap();
    assert_eq!(fields(&publication)["asset"], other.to_string());
    assert_eq!(tools.submissions().unwrap().len(), 2);
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
        &desktop.join(crate::project_tabs::RECOVERY_DIRECTORY),
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
    assert!(Tools::open(args(&folder, root.join("mcp"))).is_err());
    drop(owner);
    assert!(Owner::desktop(folder.clone(), &alternate, None).is_err());
    assert!(Tools::open(args(&folder, root.join("mcp"))).is_err());
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
    let mcp = Tools::open(args(&folder, root.join("mcp"))).unwrap();
    assert_eq!(mcp.snapshot(illustration).unwrap().doc.title, "Dirty draft");
    let detached = crate::project_tabs::recover(&desktop)
        .unwrap()
        .active
        .unwrap();
    assert!(detached.path.is_none());
    assert_eq!(detached.doc.title, "Dirty draft");
}
