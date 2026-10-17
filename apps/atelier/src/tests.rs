use crate::{
    document::{self, Brush, Document, History, Layer, Point, Stroke, Tool},
    providers::{self, Provider, Settings},
    viewport::Viewport,
    worker::{self, Event, Storage},
};
use std::{
    io::{Read, Write},
    sync::{atomic::AtomicBool, Arc},
    time::Duration,
};
#[test]
fn provider_profiles_and_legacy_settings_round_trip() {
    let mut settings: Settings = serde_json::from_value(
        serde_json::json!({"provider":"Comfy","endpoint":"http://localhost:8188","model":"legacy"}),
    )
    .unwrap();
    settings.select_provider(Provider::Alibaba);
    settings.model = "qwen-image-2.1-pro".into();
    settings.edit_model = "qwen-image-2.1-pro".into();
    settings.endpoint =
        "https://workspace.ap-southeast-1.maas.aliyuncs.com/compatible-mode/v1".into();
    settings.select_provider(Provider::Fal);
    let mut settings: Settings =
        serde_json::from_slice(&serde_json::to_vec(&settings).unwrap()).unwrap();
    settings.select_provider(Provider::Alibaba);
    assert_eq!(settings.model, "qwen-image-2.1-pro");
    assert_eq!(settings.edit_model, "qwen-image-2.1-pro");
    assert!(settings.endpoint.contains("workspace"));
    settings.select_provider(Provider::Comfy);
    assert_eq!(settings.endpoint, "http://localhost:8188");
    assert_eq!(settings.model, "legacy");
    assert!(!serde_json::to_string(&settings)
        .unwrap()
        .contains("api_key"));
}
fn doc() -> Document {
    Document {
        width: 128,
        height: 96,
        ..Document::default()
    }
}
#[test]
fn source_bound_prompt_drafts_survive_project_and_recovery_writes() {
    let directory = temporary("prompt-drafts");
    let mut project = doc();
    project
        .prompt_drafts
        .insert("new".into(), "New image draft".into());
    let source = colored_reference("Source", [30, 90, 50, 255], 32, 32);
    let id = crate::library::id(&source);
    project
        .prompt_drafts
        .insert(id.clone(), "Unsent edit draft".into());
    project.active_prompt_target = Some(id.clone());
    project.references.push(Arc::new(source));
    let mut history = History::new();
    history.checkpoint(&project);
    project.width = 192;
    project
        .prompt_drafts
        .insert(id.clone(), "Latest edit draft".into());
    assert!(history.undo(&mut project));
    assert_eq!(project.width, 128);
    assert_eq!(project.prompt_drafts[&id], "Latest edit draft");
    assert!(history.redo(&mut project));
    assert_eq!(project.width, 192);
    assert_eq!(project.prompt_drafts[&id], "Latest edit draft");
    project
        .prompt_drafts
        .insert(id.clone(), "Unsent edit draft".into());
    for name in ["project.atelier", "recovery.atelier"] {
        let path = directory.join(name);
        document::save(&path, &project).unwrap();
        let restored = document::load(&path).unwrap();
        assert_eq!(restored.prompt_drafts["new"], "New image draft");
        assert_eq!(restored.prompt_drafts[&id], "Unsent edit draft");
        assert_eq!(restored.active_prompt_target.as_deref(), Some(id.as_str()));
    }
    let invalid_path = directory.join("cannot-save.atelier");
    std::fs::create_dir(&invalid_path).unwrap();
    assert!(document::save(&invalid_path, &project).is_err());
    assert_eq!(project.prompt_drafts[&id], "Unsent edit draft");
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn native_profiles_resolve_paths_capabilities_and_reject_wrong_routes() {
    let directory = temporary("native-providers");
    let providers = clipforge_inference::Providers::from_toml(
        r#"
[providers.qwen]
kind = "sdcpp"
architecture = "qwen-image-2.1"
capabilities = ["t2i", "edit"]
base_url = "http://127.0.0.1:7861"
model_dir = "weights"
executable = "runtime/sd-server"
residency = "lowvram"
[providers.krea]
kind = "sdcpp"
architecture = "krea2-turbo"
capabilities = ["t2i"]
base_url = "http://127.0.0.1:7862"
model_dir = "weights"
executable = "runtime/sd-server"
"#,
        &directory,
    )
    .unwrap();
    let mut settings = Settings {
        provider: Provider::QwenNative,
        ..Default::default()
    };
    for editing in [false, true] {
        let deployment = crate::native_images::deployment(&settings, &providers, editing).unwrap();
        assert_eq!(deployment.architecture, "qwen-image-2.1");
        assert_eq!(deployment.model_dir, directory.join("weights"));
        assert_eq!(deployment.executable, directory.join("runtime/sd-server"));
        assert_eq!(
            deployment.residency,
            clipforge_inference::sd::Residency::LowVram
        );
    }
    settings.provider = Provider::KreaNative;
    assert!(!settings.provider.supports_edit());
    assert!(settings.provider.key_env().is_empty());
    assert_eq!(
        crate::native_images::deployment(&settings, &providers, false)
            .unwrap()
            .architecture,
        "krea2-turbo"
    );
    assert!(crate::native_images::deployment(&settings, &providers, true).is_err());
    std::fs::remove_dir_all(directory).unwrap();
}
fn stroke(tool: Tool, y: f32) -> Stroke {
    Stroke {
        brush: Brush::Ink,
        pressures: vec![].into(),
        tool,
        size: 12.,
        opacity: 1.,
        points: vec![Point { x: 10., y }, Point { x: 110., y }].into(),
    }
}
fn temporary(name: &str) -> std::path::PathBuf {
    let path = std::env::temp_dir().join(format!("atelier-tests-{}-{name}", std::process::id()));
    // Start from an empty directory: the process id can be reused across runs and
    // TMPDIR persists, so a fixture left by an earlier run must not leak in.
    let _ = std::fs::remove_dir_all(&path);
    std::fs::create_dir_all(&path).unwrap();
    path
}

fn colored_reference(name: &str, color: [u8; 4], width: u32, height: u32) -> document::Reference {
    let image = image::RgbaImage::from_pixel(width, height, image::Rgba(color));
    let mut bytes = std::io::Cursor::new(Vec::new());
    image::DynamicImage::ImageRgba8(image)
        .write_to(&mut bytes, image::ImageFormat::Png)
        .unwrap();
    document::reference_from_bytes(name.into(), &bytes.into_inner(), "Test original".into())
        .unwrap()
}

#[test]
fn originals_keep_resolution_bytes_and_alpha_across_library_and_projects() {
    use base64::Engine;
    let directory = temporary("shared-originals");
    let library = directory.join("library");
    let original = colored_reference("Original", [12, 34, 56, 128], 2400, 1200);
    let id = crate::library::id(&original);
    crate::library::store(&library, &original).unwrap();
    crate::library::store(&library, &original).unwrap();
    let loaded = crate::library::load(&library).unwrap();
    assert_eq!(loaded.len(), 1);
    assert_eq!(loaded[0].image, original.image);
    assert_eq!(
        document::decode_reference(&loaded[0]).unwrap().width(),
        2400
    );
    assert_eq!(
        document::decode_reference(&loaded[0])
            .unwrap()
            .to_rgba8()
            .get_pixel(0, 0)
            .0,
        [12, 34, 56, 128]
    );
    let raw = base64::engine::general_purpose::STANDARD
        .decode(&original.image)
        .unwrap();
    assert_eq!(
        std::fs::read(library.join(format!("{id}.image"))).unwrap(),
        raw
    );
    let metadata: serde_json::Value =
        serde_json::from_slice(&std::fs::read(library.join(format!("{id}.json"))).unwrap())
            .unwrap();
    assert!(metadata.get("image").is_none());
    for title in ["Project A", "Project B"] {
        let mut project = doc();
        project.title = title.into();
        project.references.push(Arc::new(loaded[0].clone()));
        project.reference = Some(0);
        document::save(&directory.join(format!("{title}.atelier")), &project).unwrap();
    }
    let mut renamed = original.clone();
    renamed.name = "Library rename".into();
    renamed.favorite = true;
    renamed.tags = "house, reference".into();
    crate::library::update(&library, &renamed, crate::library::Update::Details).unwrap();
    crate::library::update(&library, &renamed, crate::library::Update::Favorite).unwrap();
    assert_eq!(
        crate::library::load(&library).unwrap()[0].name,
        "Library rename"
    );
    let project = document::load(&directory.join("Project B.atelier")).unwrap();
    assert_eq!(project.references[0].name, "Original");
    assert_eq!(crate::library::id(&project.references[0]), id);
    assert_eq!(project.references[0].image, original.image);
    std::fs::remove_file(library.join(format!("{id}.image"))).unwrap();
    assert!(crate::library::load(&library).is_err());
    assert!(document::load(&directory.join("Project A.atelier")).is_ok());
}

#[test]
fn project_images_are_not_shared_until_promoted() {
    let directory = temporary("shared-scope");
    let library = directory.join("library");
    let variant = colored_reference("Variant", [10, 20, 30, 255], 64, 64);
    let id = crate::library::id(&variant);

    // A project-only image has no shared record, and renaming it is a harmless
    // no-op in the library: its name, tags and favorite stay project-local.
    crate::library::update(&library, &variant, crate::library::Update::Details).unwrap();
    assert!(!library.join(format!("{id}.json")).exists());
    assert!(crate::library::load(&library).unwrap().is_empty());

    // Promoting it (what "Add to reference library" does) makes it shared.
    crate::library::store(&library, &variant).unwrap();
    let shared = crate::library::load(&library).unwrap();
    assert_eq!(shared.len(), 1);
    assert_eq!(crate::library::id(&shared[0]), id);
}

#[test]
fn the_shared_library_loads_without_collecting_project_images() {
    let directory = temporary("library-load");
    let library = directory.join("library");
    let shared = colored_reference("Shared", [1, 2, 3, 255], 48, 48);
    crate::library::store(&library, &shared).unwrap();
    let workers = worker::start();

    // A plain load reports only what is actually in the shared collection.
    workers
        .storage
        .send(Storage::Library(library.clone()))
        .unwrap();
    match workers.events.recv_timeout(Duration::from_secs(5)).unwrap() {
        Event::Library(Ok(items)) => assert_eq!(items.len(), 1),
        _ => panic!("The library should load"),
    }

    // Adding (a promote) stores the original and reloads the collection.
    let added = colored_reference("Added", [4, 5, 6, 255], 48, 48);
    workers
        .storage
        .send(Storage::LibraryAdd(library, vec![Arc::new(added)]))
        .unwrap();
    match workers.events.recv_timeout(Duration::from_secs(5)).unwrap() {
        Event::Library(Ok(items)) => assert_eq!(items.len(), 2),
        _ => panic!("The added original should reload"),
    }
}

#[test]
fn region_rect_clamps_and_rejects_tiny_selections() {
    let wide = crate::app::region_rect(
        Point { x: -10., y: 5.5 },
        Point { x: 120.4, y: 90.2 },
        128,
        96,
    );
    assert_eq!(wide, Some((0, 5, 121, 86)));
    // A reversed drag produces the same rectangle.
    let reversed = crate::app::region_rect(
        Point { x: 120.4, y: 90.2 },
        Point { x: -10., y: 5.5 },
        128,
        96,
    );
    assert_eq!(reversed, wide);
    // Below 16 px on a side the selection is ignored.
    assert_eq!(
        crate::app::region_rect(Point { x: 10., y: 10. }, Point { x: 24., y: 24. }, 128, 96),
        None
    );
    // A selection outside the canvas clamps away.
    assert_eq!(
        crate::app::region_rect(
            Point { x: 200., y: 200. },
            Point { x: 300., y: 300. },
            128,
            96,
        ),
        None
    );
}

#[test]
fn edit_families_include_branches_and_ancestors_without_unrelated_images() {
    let original = colored_reference("Original", [255, 0, 0, 255], 128, 96);
    let mut first = colored_reference("First edit", [0, 255, 0, 255], 128, 96);
    first.parent = Some(crate::library::id(&original));
    let mut second = colored_reference("Second edit", [0, 0, 255, 255], 128, 96);
    second.parent = Some(crate::library::id(&first));
    let mut branch = colored_reference("Other branch", [255, 255, 0, 255], 128, 96);
    branch.parent = Some(crate::library::id(&original));
    let unrelated = colored_reference("Other original", [255, 0, 255, 255], 128, 96);
    let all = [original, first, second, branch, unrelated].map(Arc::new);
    let family = crate::library::family(&all[2], &all);
    assert_eq!(family.len(), 4);
    for image in &all[..4] {
        assert!(family.contains(&crate::library::id(image)));
    }
    assert!(!family.contains(&crate::library::id(&all[4])));
}

#[test]
fn image_layers_paint_erase_transform_and_survive_undo_and_save() {
    let mut project = doc();
    project.layers[0].image = Some(document::ImageLayer {
        reference: Arc::new(colored_reference("Blue", [0, 80, 255, 255], 128, 96)),
        transform: [1., 0., 0., 1., 0., 0.],
    });
    let original = document::raster(&project, 128, true).unwrap();
    assert_eq!(original.get_pixel(60, 48).0, [0, 80, 255, 255]);
    let mut history = History::new();
    history.checkpoint(&project);
    project.layers[0].strokes.push(stroke(Tool::Eraser, 48.));
    let erased = document::raster(&project, 128, true).unwrap();
    assert_eq!(erased.get_pixel(60, 48)[3], 0);
    assert_eq!(erased.get_pixel(60, 5).0, [0, 80, 255, 255]);
    assert!(history.undo(&mut project));
    assert_eq!(document::raster(&project, 128, true).unwrap(), original);
    project.rotate();
    project.flip();
    let transformed = document::raster(&project, 128, true).unwrap();
    assert_eq!(transformed.dimensions(), (96, 128));
    assert_eq!(transformed.get_pixel(60, 48).0, [0, 80, 255, 255]);
    project.resize(192, 256, true).unwrap();
    assert_eq!(
        document::raster(&project, 256, true)
            .unwrap()
            .get_pixel(120, 96)
            .0,
        [0, 80, 255, 255]
    );
    let path = temporary("image-layer").join("project.atelier");
    document::save(&path, &project).unwrap();
    assert_eq!(
        document::raster(&document::load(&path).unwrap(), 256, true).unwrap(),
        document::raster(&project, 256, true).unwrap()
    );
    project.layers[0].image.as_mut().unwrap().transform[0] = f32::NAN;
    assert!(project.validate().is_err());
}

#[test]
fn image_layer_placement_moves_painted_marks_and_rejects_invalid_changes() {
    let mut project = doc();
    project.layers[0].image = Some(document::ImageLayer {
        reference: Arc::new(colored_reference("Blue", [0, 80, 255, 255], 128, 96)),
        transform: [1., 0., 0., 1., 0., 0.],
    });
    project.layers[0].strokes.push(stroke(Tool::Eraser, 48.));
    project.reposition_image_layer(16., 8., 0.5).unwrap();
    let moved = document::raster(&project, 128, true).unwrap();
    assert_eq!(moved.get_pixel(78, 56)[3], 0);
    assert_eq!(moved.get_pixel(78, 36).0, [0, 80, 255, 255]);
    let before = serde_json::to_vec(&project).unwrap();
    assert!(project.reposition_image_layer(f32::NAN, 0., 1.).is_err());
    assert!(project.reposition_image_layer(0., 0., 0.000001).is_err());
    assert_eq!(serde_json::to_vec(&project).unwrap(), before);
    project.validate().unwrap();
}

#[test]
fn masks_use_white_for_editable_pixels_black_for_protected_and_erase_correctly() {
    let mut mask = crate::edit::MaskDraft {
        width: 128,
        height: 96,
        strokes: vec![stroke(Tool::Pencil, 48.)],
    };
    let image = image::load_from_memory(&mask.png().unwrap())
        .unwrap()
        .to_rgba8();
    assert_eq!(image.dimensions(), (128, 96));
    assert_eq!(image.get_pixel(60, 48).0, [255, 255, 255, 255]);
    assert_eq!(image.get_pixel(60, 5).0, [0, 0, 0, 255]);
    mask.strokes.push(Stroke {
        points: vec![Point { x: 60., y: 48. }].into(),
        size: 20.,
        tool: Tool::Eraser,
        ..stroke(Tool::Eraser, 48.)
    });
    let image = image::load_from_memory(&mask.png().unwrap())
        .unwrap()
        .to_rgba8();
    assert_eq!(image.get_pixel(60, 48).0, [0, 0, 0, 255]);
    assert_eq!(image.get_pixel(20, 48).0, [255, 255, 255, 255]);
    mask.strokes.clear();
    assert!(mask.png().is_err());
    mask.strokes.push(stroke(Tool::Pencil, 48.));
    Arc::make_mut(&mut mask.strokes[0].points)[0].x = f32::NAN;
    assert!(mask.validate().is_err());
}

#[test]
fn a_hosted_provider_reports_no_local_server() {
    let settings = Settings::default();
    assert_eq!(settings.provider, Provider::GptImage);
    assert_eq!(
        crate::native_images::server_status(&settings),
        crate::native_images::ServerStatus::NotLocal
    );
}

#[test]
fn a_box_selection_masks_only_its_rectangle() {
    let mask = crate::edit::MaskDraft {
        width: 128,
        height: 96,
        strokes: vec![Stroke {
            brush: Brush::Ink,
            pressures: vec![].into(),
            tool: Tool::Rect,
            size: 12.,
            opacity: 1.,
            points: vec![Point { x: 20., y: 20. }, Point { x: 80., y: 60. }].into(),
        }],
    };
    let image = image::load_from_memory(&mask.png().unwrap())
        .unwrap()
        .to_rgba8();
    assert_eq!(image.dimensions(), (128, 96));
    assert_eq!(image.get_pixel(40, 40).0, [255, 255, 255, 255]);
    assert_eq!(image.get_pixel(10, 10).0, [0, 0, 0, 255]);
    assert_eq!(image.get_pixel(90, 80).0, [0, 0, 0, 255]);
    // Compositing changes only the box; the protected corners are preserved.
    let original = colored_reference("Original", [10, 40, 90, 255], 128, 96);
    let generated = colored_reference("Changed", [240, 20, 30, 255], 128, 96);
    let pixels = document::decode_reference(&mask.composite(&original, &generated).unwrap())
        .unwrap()
        .to_rgba8();
    assert_eq!(pixels.get_pixel(40, 40).0, [240, 20, 30, 255]);
    assert_eq!(pixels.get_pixel(10, 10).0, [10, 40, 90, 255]);
    assert!(mask.validate().is_ok());
}
#[test]
fn local_sampling_settings_round_trip() {
    let settings = Settings {
        steps: Some(30),
        width: 1536,
        height: 1024,
        loras: vec![providers::LoraSetting {
            name: "characters-mia.safetensors".into(),
            multiplier: 0.8,
        }],
        ..Settings::default()
    };
    let restored: Settings =
        serde_json::from_slice(&serde_json::to_vec(&settings).unwrap()).unwrap();
    assert_eq!(restored.steps, Some(30));
    assert_eq!((restored.width, restored.height), (1536, 1024));
    assert_eq!(restored.loras.len(), 1);
    assert_eq!(restored.loras[0].name, "characters-mia.safetensors");
    assert_eq!(restored.loras[0].multiplier, 0.8);
    // Legacy settings without the new keys load with defaults.
    let legacy: Settings =
        serde_json::from_value(serde_json::json!({"provider":"QwenNative"})).unwrap();
    assert_eq!(legacy.steps, None);
    assert!(legacy.loras.is_empty());
}

#[test]
fn mask_drafts_reload_with_their_image_and_invalid_drafts_are_reported() {
    let directory = temporary("mask-drafts");
    let reference = Arc::new(colored_reference("Image", [1, 2, 3, 255], 128, 96));
    let id = crate::library::id(&reference);
    let workers = worker::start();
    let draft = crate::edit::MaskDraft {
        width: 128,
        height: 96,
        strokes: vec![stroke(Tool::Pencil, 48.)],
    };
    workers
        .storage
        .send(Storage::MaskDraft(directory.clone(), id.clone(), 1, draft))
        .unwrap();
    assert!(matches!(
        workers.events.recv_timeout(Duration::from_secs(5)).unwrap(),
        Event::MaskSaved(_, 1, Ok(()))
    ));
    workers
        .storage
        .send(Storage::OpenImage(reference.clone(), directory.clone()))
        .unwrap();
    match workers.events.recv_timeout(Duration::from_secs(5)).unwrap() {
        Event::OpenedImage(loaded, Ok((image, mask))) => {
            assert_eq!(loaded, id);
            assert_eq!(image.dimensions(), (128, 96));
            assert_eq!(mask.strokes.len(), 1);
        }
        _ => panic!("Image and mask should reload"),
    }
    std::fs::write(directory.join(format!("{id}.mask")), b"broken").unwrap();
    workers
        .storage
        .send(Storage::OpenImage(reference, directory))
        .unwrap();
    assert!(matches!(
        workers.events.recv_timeout(Duration::from_secs(5)).unwrap(),
        Event::OpenedImage(_, Err(_))
    ));
}

#[test]
fn masked_edits_preserve_protected_pixels_even_if_the_provider_changes_them() {
    let original = colored_reference("Original", [10, 40, 90, 128], 128, 96);
    let generated = colored_reference("Changed", [240, 20, 30, 255], 128, 96);
    let mask = crate::edit::MaskDraft {
        width: 128,
        height: 96,
        strokes: vec![stroke(Tool::Pencil, 48.)],
    };
    let composited = mask.composite(&original, &generated).unwrap();
    let pixels = document::decode_reference(&composited).unwrap().to_rgba8();
    assert_eq!(pixels.get_pixel(60, 5).0, [10, 40, 90, 128]);
    assert_eq!(pixels.get_pixel(60, 48).0, [240, 20, 30, 255]);
    let wrong_size = colored_reference("Wrong", [240, 20, 30, 255], 256, 192);
    assert!(mask.composite(&original, &wrong_size).is_err());
}

#[test]
#[ignore = "Creates deterministic fixtures for the live Atelier UX walkthrough"]
fn prepare_workflow_fixture() {
    let directory =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.svs-e2e/atelier-workflow");
    std::fs::create_dir_all(&directory).unwrap();
    let mut image = image::RgbaImage::from_fn(2048, 1024, |_, y| {
        image::Rgba(if y < 700 {
            [125, 190, 230, 255]
        } else {
            [65, 125, 60, 255]
        })
    });
    for y in 350..800 {
        for x in 650..1400 {
            image.put_pixel(x, y, image::Rgba([168, 95, 42, 255]));
        }
    }
    for y in 535..800 {
        for x in 930..1100 {
            image.put_pixel(x, y, image::Rgba([65, 40, 30, 255]));
        }
    }
    for y in 440..570 {
        for x in 730..870 {
            image.put_pixel(x, y, image::Rgba([230, 225, 170, 255]));
        }
    }
    let mut bytes = std::io::Cursor::new(Vec::new());
    image::DynamicImage::ImageRgba8(image.clone())
        .write_to(&mut bytes, image::ImageFormat::Png)
        .unwrap();
    let original = document::reference_from_bytes(
        "Cabin original".into(),
        &bytes.into_inner(),
        "Deterministic local UX fixture · no API call".into(),
    )
    .unwrap();
    for y in 0..350 {
        for x in 0..2048 {
            image.put_pixel(x, y, image::Rgba([235, 160, 120, 255]));
        }
    }
    let mut bytes = std::io::Cursor::new(Vec::new());
    image::DynamicImage::ImageRgba8(image)
        .write_to(&mut bytes, image::ImageFormat::Png)
        .unwrap();
    let mut edited = document::reference_from_bytes(
        "Cabin sunset".into(),
        &bytes.into_inner(),
        "Deterministic edited fixture · no API call".into(),
    )
    .unwrap();
    edited.parent = Some(crate::library::id(&original));
    edited.prompt = "Change the sky to sunset".into();
    for reference in [&original, &edited] {
        crate::library::store(&directory.join("library"), reference).unwrap();
    }
    let palette = colored_reference("Blue palette", [30, 90, 220, 255], 256, 256);
    crate::library::store(&directory.join("library"), &palette).unwrap();
    let mut project = Document {
        title: "Atelier workflow check".into(),
        width: 1024,
        height: 768,
        ..Document::default()
    };
    project.references.push(Arc::new(original));
    project.reference = Some(0);
    document::save(&directory.join("recovery.atelier"), &project).unwrap();
    println!("Live UX data: {}", directory.display());
}

#[test]
fn viewport_inverts_every_rotation_and_mirror() {
    for turns in 0..4 {
        for mirror in [false, true] {
            let view = Viewport {
                turns,
                mirror,
                ..Viewport::default()
            };
            for p in [
                Point { x: 0., y: 0. },
                Point { x: 17.5, y: 83.2 },
                Point { x: 127., y: 95. },
            ] {
                let actual = view.to_document(view.to_view(p, 128., 96.), 128., 96.);
                assert!((actual.x - p.x).abs() < 0.0001 && (actual.y - p.y).abs() < 0.0001);
            }
        }
    }
}
#[test]
fn eraser_removes_only_active_layer_and_reveals_lower_layer() {
    let mut doc = doc();
    doc.layers[0].strokes.push(stroke(Tool::Pencil, 48.));
    doc.layers.push(Layer {
        name: "Top".into(),
        visible: true,
        opacity: 1.,
        strokes: vec![stroke(Tool::Pencil, 48.), stroke(Tool::Eraser, 48.)],
        image: None,
    });
    let visible = document::raster(&doc, 128, true).unwrap();
    assert!(visible.get_pixel(60, 48)[3] > 240);
    doc.layers[0].visible = false;
    let erased = document::raster(&doc, 128, true).unwrap();
    assert_eq!(erased.get_pixel(60, 48)[3], 0);
}
#[test]
fn opacity_is_straight_alpha_in_exports() {
    let mut doc = doc();
    let mut s = stroke(Tool::Pencil, 48.);
    s.opacity = 0.5;
    doc.layers[0].strokes.push(s);
    let image = document::raster(&doc, 128, true).unwrap();
    let pixel = image.get_pixel(60, 48);
    assert!((120..=130).contains(&pixel[3]));
    assert!((38..=44).contains(&pixel[0]));
}
#[test]
fn click_makes_a_mark_and_canvas_bounds_clip_strokes() {
    let mut doc = doc();
    doc.layers[0].strokes.push(Stroke {
        brush: Brush::Ink,
        pressures: vec![].into(),
        tool: Tool::Pencil,
        size: 10.,
        opacity: 1.,
        points: vec![Point { x: 64., y: 48. }].into(),
    });
    let image = document::raster(&doc, 128, true).unwrap();
    assert_eq!(image.get_pixel(64, 48)[3], 255);
    assert_eq!(image.get_pixel(0, 0)[3], 0);
}
#[test]
fn history_restores_deleted_layers_and_references() {
    let mut doc = doc();
    let mut history = History::new();
    history.checkpoint(&doc);
    doc.layers[0].strokes.push(stroke(Tool::Pencil, 48.));
    history.checkpoint(&doc);
    doc.rotate();
    assert_eq!(doc.width, 96);
    assert!(history.undo(&mut doc));
    assert_eq!(doc.width, 128);
    assert_eq!(doc.layers[0].strokes.len(), 1);
    assert!(history.undo(&mut doc));
    assert!(doc.layers[0].strokes.is_empty());
    assert!(history.redo(&mut doc));
    assert_eq!(doc.layers[0].strokes.len(), 1);
    history.checkpoint(&doc);
    doc.flip();
    assert!(!history.can_redo());
}
#[test]
fn document_transform_preserves_strokes_and_resize_modes() {
    let mut doc = doc();
    doc.layers[0].strokes.push(stroke(Tool::Pencil, 48.));
    let original = doc.layers[0].strokes[0].points.clone();
    for _ in 0..4 {
        doc.rotate();
    }
    assert_eq!(doc.layers[0].strokes[0].points, original);
    doc.flip();
    doc.flip();
    assert_eq!(doc.layers[0].strokes[0].points, original);
    doc.resize(256, 192, false).unwrap();
    assert_eq!(doc.layers[0].strokes[0].points, original);
    doc.resize(128, 96, true).unwrap();
    assert_eq!(doc.layers[0].strokes[0].points[0], Point { x: 5., y: 24. });
    assert_eq!(doc.layers[0].strokes[0].size, 6.);
}
#[test]
fn save_round_trip_embeds_reference_and_export_formats() {
    let directory = temporary("roundtrip");
    let mut doc = doc();
    doc.layers[0].strokes.push(stroke(Tool::Pencil, 48.));
    let source = image::DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(
        32,
        32,
        image::Rgba([120, 80, 40, 255]),
    ));
    let mut bytes = std::io::Cursor::new(vec![]);
    source
        .write_to(&mut bytes, image::ImageFormat::Png)
        .unwrap();
    doc.references.push(Arc::new(
        document::reference_from_bytes("Test".into(), &bytes.into_inner(), "Imported".into())
            .unwrap(),
    ));
    doc.reference = Some(0);
    doc.note = "Look at negative space".into();
    let path = directory.join("study.atelier");
    document::save(&path, &doc).unwrap();
    let loaded = document::load(&path).unwrap();
    assert_eq!(loaded.note, doc.note);
    assert_eq!(
        document::decode_reference(&loaded.references[0])
            .unwrap()
            .width(),
        32
    );
    let png = directory.join("study.png");
    let jpg = directory.join("study.jpg");
    document::export(&png, &loaded, true, 90).unwrap();
    document::export(&jpg, &loaded, true, 90).unwrap();
    let png = image::open(png).unwrap().to_rgba8();
    assert_eq!(png.dimensions(), (128, 96));
    assert_eq!(png.get_pixel(0, 0)[3], 0);
    let jpg = image::open(jpg).unwrap().to_rgb8();
    assert_eq!(jpg.dimensions(), (128, 96));
    assert!(jpg.get_pixel(0, 0)[0] > 245);
    std::fs::remove_dir_all(directory).unwrap();
}
#[test]
fn validation_rejects_corrupt_and_unsafe_projects() {
    let mut doc = doc();
    doc.width = u32::MAX;
    assert!(doc.validate().is_err());
    doc.width = 128;
    doc.active_layer = 20;
    assert!(doc.validate().is_err());
    doc.active_layer = 0;
    let mut s = stroke(Tool::Pencil, 0.);
    Arc::make_mut(&mut s.points)[0].x = f32::NAN;
    doc.layers[0].strokes.push(s);
    assert!(doc.validate().is_err());
    assert!(document::validate_size(8192, 8192).is_err());
    assert!(document::validate_size(32, 64).is_err());
    assert!(document::export(std::path::Path::new("bad.gif"), &doc, false, 90).is_err());
}
#[test]
fn queued_saves_keep_latest_recovery() {
    let directory = temporary("ordering");
    let path = directory.join("recovery.atelier");
    let workers = worker::start();
    for revision in 1..=5 {
        let mut doc = doc();
        doc.title = format!("Revision {revision}");
        workers
            .storage
            .send(Storage::Save {
                doc,
                path: path.clone(),
                revision,
                epoch: 1,
                automatic: true,
            })
            .unwrap();
    }
    for _ in 0..5 {
        match workers.events.recv_timeout(Duration::from_secs(5)).unwrap() {
            Event::Saved { result, .. } => result.unwrap(),
            _ => panic!("unexpected event"),
        }
    }
    assert_eq!(document::load(&path).unwrap().title, "Revision 5");
    std::fs::remove_dir_all(directory).unwrap();
}
#[test]
fn settings_save_reports_failures_without_claiming_success() {
    let directory = temporary("settings-save-errors");
    let path = directory.join("settings.json");
    std::fs::write(&path, "previous settings").unwrap();
    let error = worker::apply_settings(&Settings::default(), &directory, "").unwrap_err();
    assert!(format!("{error:#}").contains("Settings file was not saved"));
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "previous settings");
    std::fs::remove_dir_all(directory).unwrap();
}
#[test]
fn shared_provider_config_supplies_hosted_image_credentials() {
    let registry = clipforge_inference::Providers::from_toml(
        r#"
            [defaults]
            t2i = "replicate-image"
            edit = "replicate-image"

            [providers.replicate-image]
            kind = "replicate"
            capabilities = ["t2i", "edit"]
            base_url = "https://api.replicate.com/v1"
            model = "alibaba/qwen-image-3"
            api_key = "replicate-from-shared-config"
        "#,
        std::path::Path::new("."),
    )
    .unwrap();

    assert_eq!(
        providers::shared_config_key(Provider::Replicate, &registry).as_deref(),
        Some("replicate-from-shared-config")
    );
    assert_eq!(
        providers::shared_stored_api_key(Provider::Replicate, &registry).as_deref(),
        Some("replicate-from-shared-config")
    );

    let environment_registry = clipforge_inference::Providers::from_toml(
        r#"
            [defaults]
            t2i = "replicate-image"

            [providers.replicate-image]
            kind = "replicate"
            capabilities = ["t2i"]
            base_url = "https://api.replicate.com/v1"
            model = "alibaba/qwen-image-3"
            api_key_env = "ATELIER_TEST_PROVIDER_KEY"
        "#,
        std::path::Path::new("."),
    )
    .unwrap();
    assert!(providers::shared_stored_api_key(Provider::Replicate, &environment_registry).is_none());
}
fn workflow() -> serde_json::Value {
    serde_json::json!({"6":{"class_type":"CLIPTextEncode","inputs":{"text":"old"}},"5":{"class_type":"EmptyLatentImage","inputs":{"width":512,"height":512,"batch_size":4}},"3":{"class_type":"KSampler","inputs":{"seed":1}},"9":{"class_type":"SaveImage","inputs":{}}})
}
#[test]
fn comfy_maps_only_configured_nodes_and_rejects_editor_workflow() {
    let settings = Settings {
        workflow: workflow(),
        ..Settings::default()
    };
    let mapped =
        providers::prepare_workflow(&settings, "A \"quoted\" subject", (1024, 1536), 123).unwrap();
    assert_eq!(mapped["6"]["inputs"]["text"], "A \"quoted\" subject");
    assert_eq!(mapped["5"]["inputs"]["height"], 1536);
    assert_eq!(mapped["5"]["inputs"]["batch_size"], 1);
    assert_eq!(mapped["3"]["inputs"]["seed"], 123);
    assert_eq!(settings.workflow["6"]["inputs"]["text"], "old");
    let bad = Settings {
        workflow: serde_json::json!({"nodes":[]}),
        ..settings
    };
    assert!(providers::prepare_workflow(&bad, "prompt", (1024, 1024), 1).is_err());
}
#[test]
fn comfy_generation_roundtrip_with_local_mock_server() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let mut image = std::io::Cursor::new(vec![]);
    image::DynamicImage::new_rgb8(32, 32)
        .write_to(&mut image, image::ImageFormat::Png)
        .unwrap();
    let png = image.into_inner();
    let server = std::thread::spawn(move || {
        for expected in ["POST /prompt", "GET /history/job-1", "GET /view?"] {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut request = vec![];
            let mut buffer = [0; 2048];
            loop {
                let n = stream.read(&mut buffer).unwrap();
                request.extend_from_slice(&buffer[..n]);
                if request.windows(4).any(|w| w == b"\r\n\r\n") {
                    break;
                }
            }
            let text = String::from_utf8_lossy(&request);
            assert!(text.starts_with(expected), "{text}");
            let body = if expected.starts_with("POST") {
                br#"{"prompt_id":"job-1"}"#.to_vec()
            } else if expected.contains("history") {
                br#"{"job-1":{"status":{"completed":true,"status_str":"success"},"outputs":{"9":{"images":[{"filename":"output.png","subfolder":"","type":"output"}]}}}}"#.to_vec()
            } else {
                png.clone()
            };
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            )
            .unwrap();
            stream.write_all(&body).unwrap();
        }
    });
    let reference = providers::generate(providers::Request {
        native: Arc::new(Default::default()),
        settings: Settings {
            provider: Provider::Comfy,
            endpoint: format!("http://{address}"),
            workflow: workflow(),
            ..Settings::default()
        },
        prompt: "A ceramic pitcher".into(),
        source: None,
        references: vec![],
        mask: None,
        progress: None,
        size: (1024, 1024),
        cancel: Arc::new(AtomicBool::new(false)),
    })
    .unwrap();
    assert_eq!(document::decode_reference(&reference).unwrap().width(), 32);
    assert!(reference.provenance.contains("A ceramic pitcher"));
    server.join().unwrap();
}
#[test]
fn generation_cancellation_prevents_network_submission() {
    let result = providers::generate(providers::Request {
        native: Arc::new(Default::default()),
        settings: Settings::default(),
        prompt: "Study".into(),
        source: None,
        references: vec![],
        mask: None,
        progress: None,
        size: (1024, 1024),
        cancel: Arc::new(AtomicBool::new(true)),
    });
    assert!(result.unwrap_err().to_string().contains("cancelled"));
}

#[test]
fn switching_saves_current_project_before_opening_and_reports_failure() {
    let directory = temporary("switch");
    let mut previous = doc();
    previous.title = "Keep this work".into();
    let archive = directory.join("previous.atelier");
    let previous_path = directory.join("current.atelier");
    let broken = directory.join("broken.atelier");
    std::fs::write(&broken, b"bad json").unwrap();
    let workers = worker::start();
    workers
        .storage
        .send(Storage::Switch {
            epoch: 7,
            previous: previous.clone(),
            archive: archive.clone(),
            previous_path: Some(previous_path.clone()),
            next: Some(broken),
        })
        .unwrap();
    match workers.events.recv_timeout(Duration::from_secs(5)).unwrap() {
        Event::Switched(epoch, result) => {
            assert_eq!(epoch, 7);
            assert!(result.is_err());
        }
        _ => panic!("Unexpected event"),
    }
    assert_eq!(document::load(&archive).unwrap().title, "Keep this work");
    assert_eq!(
        document::load(&previous_path).unwrap().title,
        "Keep this work"
    );
    let blocked = directory.join("blocked");
    std::fs::write(&blocked, b"not a directory").unwrap();
    workers
        .storage
        .send(Storage::Switch {
            epoch: 7,
            previous,
            archive: blocked.join("project.atelier"),
            previous_path: None,
            next: None,
        })
        .unwrap();
    match workers.events.recv_timeout(Duration::from_secs(5)).unwrap() {
        Event::Switched(_, result) => assert!(result.is_err()),
        _ => panic!("Unexpected event"),
    }
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn mask_save_failure_reports_owned_draft_and_can_be_retried() {
    let directory = temporary("mask-save-failure");
    let blocked = directory.join("blocked");
    std::fs::write(&blocked, b"not a directory").unwrap();
    let reference = colored_reference("Draft", [30, 60, 90, 255], 128, 96);
    let id = crate::library::id(&reference);
    let draft = crate::edit::MaskDraft {
        width: 128,
        height: 96,
        strokes: vec![stroke(Tool::Pencil, 48.)],
    };
    let workers = worker::start();
    for (path, succeeds) in [(blocked, false), (directory.join("library"), true)] {
        workers
            .storage
            .send(Storage::MaskDraft(
                path.clone(),
                id.clone(),
                42,
                draft.clone(),
            ))
            .unwrap();
        match workers.events.recv_timeout(Duration::from_secs(5)).unwrap() {
            Event::MaskSaved(saved_id, revision, result) => {
                assert_eq!(saved_id, id);
                assert_eq!(revision, 42);
                assert_eq!(result.is_ok(), succeeds);
            }
            _ => panic!("Unexpected event"),
        }
        if succeeds {
            let saved: crate::edit::MaskDraft =
                serde_json::from_slice(&std::fs::read(path.join(format!("{id}.mask"))).unwrap())
                    .unwrap();
            assert_eq!(saved.png().unwrap(), draft.png().unwrap());
        }
    }
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
#[cfg_attr(target_os = "macos", ignore = "requires a real Metal device")]
fn metal_canvas_artwork_package_contains_image_paint_and_erase() {
    let directory = temporary("canvas-artwork");
    let path = directory.join("approved.sfart");
    let mut painting = doc();
    painting.title = "Approved painting".into();
    painting.layers[0].image = Some(document::ImageLayer {
        reference: Arc::new(colored_reference("Base", [20, 100, 200, 255], 128, 96)),
        transform: [1., 0., 0., 1., 0., 0.],
    });
    painting.layers[0].strokes.push(stroke(Tool::Pencil, 30.));
    painting.layers[0].strokes.push(stroke(Tool::Eraser, 70.));
    let workers = worker::start();
    workers
        .storage
        .send(Storage::CanvasArtwork(painting, path.clone(), true))
        .unwrap();
    match workers
        .events
        .recv_timeout(Duration::from_secs(15))
        .unwrap()
    {
        Event::Exported(result) => assert_eq!(result.unwrap(), path),
        _ => panic!("Unexpected event"),
    }
    let artwork = storyfella_studio_core::artwork::ApprovedArtwork::load(&path).unwrap();
    assert_eq!(artwork.name, "Approved painting");
    use base64::Engine;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(artwork.image)
        .unwrap();
    let pixels = image::load_from_memory(&bytes).unwrap().to_rgba8();
    assert_eq!(pixels.dimensions(), (128, 96));
    assert_eq!(pixels.get_pixel(60, 10).0, [20, 100, 200, 255]);
    assert!(pixels.get_pixel(60, 30)[2] < 150, "Paint must be included");
    assert_eq!(pixels.get_pixel(60, 70)[3], 0, "Erasing must be included");
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn graphite_is_textured_deterministic_and_builds_with_repeated_strokes() {
    let mut doc = doc();
    let mut pencil = stroke(Tool::Pencil, 48.);
    pencil.brush = Brush::Graphite;
    doc.layers[0].strokes.push(pencil.clone());
    let once = document::raster(&doc, 128, true).unwrap();
    assert_eq!(once, document::raster(&doc, 128, true).unwrap());
    let values: std::collections::HashSet<_> =
        (20..100).map(|x| once.get_pixel(x, 48)[3]).collect();
    assert!(
        values.len() > 20,
        "Pencil must show grain rather than a solid stroke"
    );
    doc.layers[0].strokes.push(pencil);
    let twice = document::raster(&doc, 128, true).unwrap();
    assert!(twice.get_pixel(60, 48)[3] > once.get_pixel(60, 48)[3]);
}
#[test]
fn pressure_changes_width_and_pigment_and_survives_project_round_trip() {
    let mut doc = doc();
    let mut line = stroke(Tool::Pencil, 48.);
    line.size = 20.;
    line.pressures = vec![0.1, 1.].into();
    doc.layers[0].strokes.push(line);
    let raster = document::raster(&doc, 128, true).unwrap();
    let width = |x| (0..96).filter(|y| raster.get_pixel(x, *y)[3] > 20).count();
    assert!(width(95) > width(25) * 2);
    assert!(raster.get_pixel(95, 48)[3] > raster.get_pixel(25, 48)[3] * 2);
    let saved = serde_json::to_vec(&doc).unwrap();
    let mut restored: Document = serde_json::from_slice(&saved).unwrap();
    restored.validate().unwrap();
    assert_eq!(raster, document::raster(&restored, 128, true).unwrap());
    restored.rotate();
    restored.flip();
    restored.resize(128, 96, true).unwrap();
    assert_eq!(*restored.layers[0].strokes[0].pressures, vec![0.1, 1.]);
}
#[test]
fn legacy_strokes_remain_solid_and_invalid_pressure_is_rejected() {
    let old =
        r#"{"tool":"Pencil","size":12,"opacity":1,"points":[{"x":10,"y":48},{"x":110,"y":48}]}"#;
    let legacy: Stroke = serde_json::from_str(old).unwrap();
    assert_eq!(legacy.brush, Brush::Ink);
    assert!(legacy.pressures.is_empty());
    let mut doc = doc();
    doc.layers[0].strokes.push(legacy);
    assert_eq!(
        document::raster(&doc, 128, true).unwrap().get_pixel(60, 48)[3],
        255
    );
    doc.layers[0].strokes[0].pressures = vec![0.5].into();
    assert!(doc.validate().is_err());
    doc.layers[0].strokes[0].pressures = vec![0.5, f32::NAN].into();
    assert!(doc.validate().is_err());
    doc.layers[0].strokes[0].pressures = vec![0.5, 1.1].into();
    assert!(doc.validate().is_err());
}
#[test]
fn presets_produce_distinct_marks_and_pressure_eraser_reveals_paper() {
    let mut results = vec![];
    for brush in Brush::ALL {
        let mut doc = doc();
        let mut line = stroke(Tool::Pencil, 48.);
        line.brush = brush;
        doc.layers[0].strokes.push(line);
        results.push(document::raster(&doc, 128, true).unwrap());
    }
    for i in 0..results.len() {
        for j in i + 1..results.len() {
            assert_ne!(results[i], results[j]);
        }
    }
    let mut doc = doc();
    doc.layers[0].strokes.push(stroke(Tool::Pencil, 48.));
    let mut eraser = stroke(Tool::Eraser, 48.);
    eraser.pressures = vec![0.1, 1.].into();
    doc.layers[0].strokes.push(eraser);
    let result = document::raster(&doc, 128, true).unwrap();
    assert_eq!(result.get_pixel(100, 48)[3], 0);
    assert!(result.get_pixel(20, 52)[3] > 200);
}

#[test]
fn history_and_worker_snapshots_share_completed_stroke_samples() {
    let mut doc = doc();
    doc.layers[0].strokes.push(stroke(Tool::Pencil, 48.));
    let snapshot = doc.clone();
    assert!(Arc::ptr_eq(
        &doc.layers[0].strokes[0].points,
        &snapshot.layers[0].strokes[0].points
    ));
    Arc::make_mut(&mut doc.layers[0].strokes[0].points).push(Point { x: 115., y: 48. });
    assert_eq!(snapshot.layers[0].strokes[0].points.len(), 2);
    assert_eq!(doc.layers[0].strokes[0].points.len(), 3);
}
