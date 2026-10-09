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
    std::fs::create_dir_all(&path).unwrap();
    path
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
fn settings_save_stores_provider_key_separately_from_settings() {
    let directory = temporary("settings-and-key");
    let path = directory.join("settings.json");
    let mut settings = Settings::default();
    settings.select_provider(Provider::Replicate);
    settings.model = "custom-generation-model".into();
    let mut stored = None;
    worker::apply_settings(
        &settings,
        &path,
        Some("  test-api-key  "),
        |provider, key| {
            stored = Some((provider, key.to_owned()));
            Ok(())
        },
    )
    .unwrap();
    assert_eq!(stored, Some((Provider::Replicate, "test-api-key".into())));
    let bytes = std::fs::read(&path).unwrap();
    let restored: Settings = serde_json::from_slice(&bytes).unwrap();
    assert!(restored == settings);
    assert!(!String::from_utf8(bytes).unwrap().contains("test-api-key"));
    // Saving without a replacement key keeps the existing credential untouched.
    worker::apply_settings(&settings, &path, Some("  "), |_, _| {
        panic!("A blank key must not replace a saved credential")
    })
    .unwrap();
    std::fs::remove_dir_all(directory).unwrap();
}
#[test]
fn settings_save_reports_failures_without_claiming_success() {
    let directory = temporary("settings-save-errors");
    let path = directory.join("settings.json");
    std::fs::write(&path, "previous settings").unwrap();
    let error = worker::apply_settings(&Settings::default(), &path, Some("test-key"), |_, _| {
        anyhow::bail!("Keychain unavailable")
    })
    .unwrap_err();
    assert!(format!("{error:#}").contains("Keychain unavailable"));
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "previous settings");
    let error =
        worker::apply_settings(&Settings::default(), &directory, None, |_, _| Ok(())).unwrap_err();
    assert!(error.to_string().contains("Settings file was not saved"));
    std::fs::remove_dir_all(directory).unwrap();
}
#[test]
fn saved_and_unlocked_keys_do_not_reprompt_during_the_session() {
    let cache = providers::CredentialCache::default();
    cache.remember(Provider::Fal, "saved-fal-key".into());
    assert_eq!(
        cache
            .resolve(Provider::Fal, None, || panic!(
                "Saved key must avoid Keychain"
            ))
            .unwrap(),
        "saved-fal-key"
    );
    let mut reads = 0;
    for _ in 0..3 {
        assert_eq!(
            cache
                .resolve(Provider::Replicate, None, || {
                    reads += 1;
                    Ok(Some("unlocked-replicate-key".into()))
                })
                .unwrap(),
            "unlocked-replicate-key"
        );
    }
    assert_eq!(reads, 1);
    assert_eq!(
        cache
            .resolve(Provider::Fal, Some(" env-fal-key ".into()), || {
                panic!("Environment key must avoid Keychain")
            })
            .unwrap(),
        "env-fal-key"
    );
    cache.remember(Provider::Fal, "replacement-fal-key".into());
    assert_eq!(
        cache.resolve(Provider::Fal, None, || Ok(None)).unwrap(),
        "replacement-fal-key"
    );
}
#[test]
fn credential_read_errors_are_preserved_and_can_be_retried() {
    let cache = providers::CredentialCache::default();
    let error = cache
        .resolve(Provider::Fal, None, || {
            anyhow::bail!("Keychain access denied")
        })
        .unwrap_err();
    assert!(error.to_string().contains("Keychain access denied"));
    assert!(!error.to_string().contains("Add your"));
    let missing = cache
        .resolve(Provider::Replicate, None, || Ok(None))
        .unwrap_err();
    assert!(missing.to_string().contains("Add your Replicate key"));
    assert_eq!(
        cache
            .resolve(Provider::Fal, None, || Ok(Some("retry-key".into())))
            .unwrap(),
        "retry-key"
    );
    let cache = providers::CredentialCache::default();
    let key = cache
        .resolve(Provider::Fal, None, || {
            cache.remember(Provider::Fal, "newly-saved-key".into());
            Ok(Some("older-keychain-key".into()))
        })
        .unwrap();
    assert_eq!(key, "newly-saved-key");
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
        settings: Settings {
            provider: Provider::Comfy,
            endpoint: format!("http://{address}"),
            workflow: workflow(),
            ..Settings::default()
        },
        prompt: "A ceramic pitcher".into(),
        source: None,
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
        settings: Settings::default(),
        prompt: "Study".into(),
        source: None,
        progress: None,
        size: (1024, 1024),
        cancel: Arc::new(AtomicBool::new(true)),
    });
    assert!(result.unwrap_err().to_string().contains("cancelled"));
}

#[test]
fn switching_archives_current_study_before_opening_and_reports_failure() {
    let directory = temporary("switch");
    let mut previous = doc();
    previous.title = "Keep this work".into();
    let archive = directory.join("previous.atelier");
    let broken = directory.join("broken.atelier");
    std::fs::write(&broken, b"bad json").unwrap();
    let workers = worker::start();
    workers
        .storage
        .send(Storage::Switch {
            epoch: 7,
            previous: previous.clone(),
            archive: archive.clone(),
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
    let blocked = directory.join("blocked");
    std::fs::write(&blocked, b"not a directory").unwrap();
    workers
        .storage
        .send(Storage::Switch {
            epoch: 7,
            previous,
            archive: blocked.join("project.atelier"),
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
