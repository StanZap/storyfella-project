use crate::{
    document::{self, Brush, Document, History, Layer, Point, Stroke, Tool},
    providers::{self, Provider, Settings},
    viewport::Viewport,
    worker::{self, Event, Storage},
};
use image::GenericImageView;
use std::{
    io::{Read, Write},
    sync::{atomic::AtomicBool, Arc},
    time::Duration,
};

#[test]
#[ignore = "requires PNG outputs from the real native acceptance test"]
fn native_acceptance_outputs_survive_atelier_project_reopen() {
    acceptance_outputs_survive_project_reopen(
        "native-acceptance",
        "Native acceptance",
        &["native-create.png", "native-after-abort.png"],
        "native-results.atelier",
    );
}

#[test]
#[ignore = "requires PNG outputs from the real hosted acceptance test"]
fn hosted_acceptance_outputs_survive_atelier_project_reopen() {
    acceptance_outputs_survive_project_reopen(
        "hosted-acceptance",
        "Hosted acceptance",
        &["hosted-create.png", "hosted-edit.png"],
        "hosted-results.atelier",
    );
}

#[test]
#[ignore = "requires PNG outputs from the real native RGBA capability probe"]
fn native_rgba_acceptance_outputs_survive_atelier_project_reopen() {
    acceptance_outputs_survive_project_reopen(
        "image-capabilities",
        "Native RGBA acceptance",
        &[
            "qwen-alpha-create.png",
            "qwen-alpha-edit.png",
            "qwen-three-references.png",
        ],
        "qwen-alpha-results.atelier",
    );
}

#[test]
#[ignore = "requires PNG outputs from both real native mask acceptance scenarios"]
fn native_mask_acceptance_outputs_survive_atelier_project_reopen() {
    acceptance_outputs_survive_project_reopen(
        "native-mask-acceptance",
        "Native mask acceptance",
        &[
            "qwen-mask-black.png",
            "qwen-mask-white.png",
            "qwen-mask-half.png",
            "qwen-mask-shared.png",
        ],
        "qwen-mask-results.atelier",
    );
}

fn acceptance_outputs_survive_project_reopen(
    folder: &str,
    title: &str,
    names: &[&str],
    project_name: &str,
) {
    let directory = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../.svs-e2e")
        .join(folder);
    let mut project = doc();
    project.title = title.into();
    let mut originals = Vec::new();
    for &name in names {
        let bytes = std::fs::read(directory.join(name)).unwrap();
        let reference =
            Arc::new(document::reference_from_bytes(name.into(), &bytes, title.into()).unwrap());
        originals.push(reference.clone());
        project
            .attach_result(reference, crate::workflow::Destination::NewArtboard)
            .unwrap();
    }
    let path = directory.join(project_name);
    document::save(&path, &project).unwrap();
    let restored = document::load(&path).unwrap();
    assert_eq!(restored.title, project.title);
    assert_eq!(restored.canvas_id, project.canvas_id);
    assert_eq!(restored.canvases.len(), project.canvases.len());
    for original in originals {
        let layer = restored
            .layers
            .iter()
            .chain(
                restored
                    .canvases
                    .iter()
                    .flat_map(|canvas| canvas.layers.iter()),
            )
            .filter_map(|layer| layer.image.as_ref())
            .find(|image| image.reference.fingerprint == original.fingerprint)
            .expect("generated artwork must survive in its artboard");
        assert_eq!(layer.reference.image, original.image);
        assert_eq!(layer.reference.provenance, original.provenance);
        assert_eq!(
            document::decode_reference(&layer.reference)
                .unwrap()
                .dimensions(),
            document::decode_reference(&original).unwrap().dimensions()
        );
    }
    eprintln!(
        "All {} outputs survived Atelier save/reopen: {}",
        names.len(),
        path.display()
    );
}

#[test]
fn request_time_limit_migrates_persists_and_stays_global_across_providers() {
    let mut settings: Settings = serde_json::from_str("{}").unwrap();
    assert_eq!(settings.request_timeout_minutes, 30);
    settings.request_timeout_minutes = 90;
    settings.select_provider(Provider::QwenNative);
    assert_eq!(settings.request_timeout(), Duration::from_secs(5400));
    let directory = temporary("request-time-limit");
    let path = directory.join("settings.json");
    worker::apply_settings(&settings, &path).unwrap();
    let restored: Settings = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    assert_eq!(restored.request_timeout_minutes, 90);
    settings.request_timeout_minutes = 0;
    assert_eq!(settings.request_timeout(), Duration::from_secs(60));
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn generated_compositions_and_candidates_keep_distinct_roles_through_save_and_history() {
    use crate::workflow::Destination;
    let mut project = doc();
    let original = Arc::new(colored_reference("Original", [20, 30, 40, 255], 128, 96));
    project
        .attach_result(original.clone(), Destination::NewArtboard)
        .unwrap();
    let original_id = project.canvas_id;
    let mut candidate = colored_reference("Candidate", [220, 0, 0, 255], 32, 24);
    candidate.parent = Some(crate::library::id(&original));
    candidate.prompt = "Change the selected object".into();
    project.prompt_drafts.insert(
        format!("canvas:{original_id}:full"),
        "unsent full edit".into(),
    );
    project.prompt_drafts.insert(
        format!("canvas:{original_id}:Separate:8:8:32:24"),
        "unsent close-up".into(),
    );
    project.references.push(original.clone());
    project
        .artboard_references
        .push(crate::library::id(&original));
    project.references_visible = false;
    project
        .attach_result(
            Arc::new(candidate),
            Destination::Candidate {
                artboard: original_id,
                rect: (8, 8, 32, 24),
            },
        )
        .unwrap();
    assert!(project.layers[1].candidate);
    assert_eq!(
        document::raster(&project, 128, true)
            .unwrap()
            .get_pixel(10, 10)
            .0,
        [220, 0, 0, 255]
    );
    assert_eq!(
        document::raster(&project.artwork(), 128, true)
            .unwrap()
            .get_pixel(10, 10)
            .0,
        [20, 30, 40, 255]
    );
    let mut history = History::new();
    history.checkpoint(&project);
    project.layers[1].candidate = false;
    project.translate_layers(&[1], 3., 4.).unwrap();
    project.layers[1].opacity = 0.4;
    project.layers[1].visible = false;
    assert!(history.undo(&mut project));
    assert!(project.layers[1].candidate);
    assert!(history.redo(&mut project));
    assert_eq!(project.layers[1].opacity, 0.4);
    assert!(!project.layers[1].candidate);
    assert!(!project.layers[1].visible);
    let directory = temporary("candidate-project");
    let path = directory.join("candidate.atelier");
    document::save(&path, &project).unwrap();
    let restored = document::load(&path).unwrap();
    assert_eq!(
        restored.layers[1].image.as_ref().unwrap().reference.parent,
        Some(crate::library::id(&original))
    );
    assert_eq!(restored.prompt_drafts, project.prompt_drafts);
    assert_eq!(restored.artboard_references, project.artboard_references);
    assert!(!restored.references_visible);
    assert_eq!(
        restored.layers[1].image.as_ref().unwrap().transform,
        project.layers[1].image.as_ref().unwrap().transform
    );
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn queued_candidates_attach_to_the_captured_artboard_and_failed_attachments_are_atomic() {
    use crate::workflow::Destination;
    let mut project = doc();
    let original = Arc::new(colored_reference("Original", [20, 30, 40, 255], 128, 96));
    project
        .attach_result(original.clone(), Destination::NewArtboard)
        .unwrap();
    let source = project.canvas_id;
    project.create_canvas().unwrap();
    let active = project.canvas_id;
    project
        .attach_result(
            original.clone(),
            Destination::Candidate {
                artboard: source,
                rect: (8, 8, 32, 24),
            },
        )
        .unwrap();
    assert_eq!(project.canvas_id, active);
    assert_eq!(project.layers.len(), 1);
    assert!(
        project
            .canvases
            .iter()
            .find(|c| c.id == source)
            .unwrap()
            .layers[1]
            .candidate
    );
    let before = serde_json::to_vec(&project).unwrap();
    assert!(project
        .attach_result(
            original,
            Destination::Candidate {
                artboard: source,
                rect: (120, 90, 32, 24)
            }
        )
        .is_err());
    assert_eq!(before, serde_json::to_vec(&project).unwrap());
}

#[test]
fn background_new_compositions_preserve_the_artboard_being_edited() {
    use crate::workflow::Destination;
    let mut project = doc();
    project.layers[0].strokes.push(stroke(Tool::Pencil, 48.));
    let source_id = project.canvas_id;
    let before = serde_json::to_vec(&project.current_canvas()).unwrap();
    project
        .attach_result(
            Arc::new(colored_reference(
                "Separate close-up",
                [120, 80, 40, 255],
                256,
                512,
            )),
            Destination::NewArtboard,
        )
        .unwrap();
    assert_eq!(project.canvas_id, source_id);
    assert_eq!(
        serde_json::to_vec(&project.current_canvas()).unwrap(),
        before
    );
    assert_eq!(project.canvases.len(), 1);
    assert_eq!(
        (project.canvases[0].width, project.canvases[0].height),
        (256, 512)
    );
}

#[test]
fn approved_artboard_metadata_preserves_visible_image_provenance_and_omits_candidates() {
    use crate::workflow::Destination;
    let mut project = doc();
    let mut original = colored_reference("Accepted", [20, 30, 40, 255], 128, 96);
    original.prompt = "Accepted generation instruction".into();
    original.provenance = "Provider · original model · job 123".into();
    project
        .attach_result(Arc::new(original), Destination::NewArtboard)
        .unwrap();
    let mut pending = colored_reference("Candidate", [200, 30, 40, 255], 128, 96);
    pending.provenance = "Unaccepted job 456".into();
    project
        .attach_result(
            Arc::new(pending),
            Destination::Candidate {
                artboard: project.canvas_id,
                rect: (0, 0, 128, 96),
            },
        )
        .unwrap();
    let (prompt, provenance) = project.artwork_metadata();
    assert_eq!(prompt, "Accepted generation instruction");
    assert!(provenance.contains("original model · job 123"));
    assert!(!provenance.contains("Unaccepted job 456"));
    project.layers[0].visible = false;
    assert!(!project.artwork_metadata().1.contains("job 123"));
}

#[test]
fn artboard_source_snapshots_preserve_the_accepted_parent_without_creating_self_cycles() {
    let mut project = doc();
    let mut original = colored_reference("Accepted", [20, 30, 40, 255], 128, 96);
    original.parent = Some("0".repeat(64));
    let original = Arc::new(original);
    project
        .attach_result(original.clone(), crate::workflow::Destination::NewArtboard)
        .unwrap();
    let source = project.generation_source(None).unwrap();
    let original_id = crate::library::id(&original);
    if crate::library::id(&source) == original_id {
        assert_eq!(source.parent, original.parent);
    } else {
        assert_eq!(source.parent, Some(original_id.clone()));
    }
    project.layers[0].strokes.push(stroke(Tool::Pencil, 48.));
    let source = project.generation_source(None).unwrap();
    assert_eq!(source.parent, Some(original_id.clone()));
    let close_up = project.generation_source(Some((16, 16, 32, 24))).unwrap();
    assert_eq!(
        document::decode_reference(&close_up).unwrap().dimensions(),
        (32, 24)
    );
    assert_eq!(close_up.parent, Some(original_id));
    assert!(project.generation_source(Some((120, 90, 32, 24))).is_err());
}

#[test]
fn selection_blend_crops_full_context_and_never_exports_a_pending_result() {
    use crate::workflow::Destination;
    let mut project = doc();
    project
        .attach_result(
            Arc::new(colored_reference("Base", [0, 0, 255, 255], 128, 96)),
            Destination::NewArtboard,
        )
        .unwrap();
    let output = colored_reference("Provider full result", [255, 0, 0, 255], 256, 192);
    let crop = crate::workflow::crop_blend(&output, (128, 96), (16, 24, 32, 24)).unwrap();
    assert_eq!(
        document::decode_reference(&crop).unwrap().dimensions(),
        (32, 24)
    );
    project
        .attach_result(
            Arc::new(crop),
            Destination::Candidate {
                artboard: project.canvas_id,
                rect: (16, 24, 32, 24),
            },
        )
        .unwrap();
    let preview = document::raster(&project, 128, true).unwrap();
    assert_eq!(preview.get_pixel(20, 30).0, [255, 0, 0, 255]);
    assert_eq!(preview.get_pixel(4, 4).0, [0, 0, 255, 255]);
    let directory = temporary("candidate-export");
    let path = directory.join("artwork.png");
    document::export(&path, &project, true, 90).unwrap();
    assert_eq!(
        image::open(&path).unwrap().to_rgba8().get_pixel(20, 30).0,
        [0, 0, 255, 255]
    );
    project.layers[1].candidate = false;
    document::export(&path, &project, true, 90).unwrap();
    assert_eq!(
        image::open(path).unwrap().to_rgba8().get_pixel(20, 30).0,
        [255, 0, 0, 255]
    );
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn legacy_generated_images_upgrade_once_without_turning_imported_references_into_artwork() {
    let mut project = doc();
    project.version = 1;
    project.layers[0].strokes.push(stroke(Tool::Pencil, 48.));
    let mut generated = colored_reference("Generated", [255, 0, 0, 255], 128, 96);
    generated.prompt = "A character".into();
    generated.parent = Some("parent-identity".into());
    project.references = vec![
        Arc::new(generated),
        Arc::new(colored_reference("Imported", [0, 255, 0, 255], 128, 96)),
    ];
    let directory = temporary("artboard-migration");
    let path = directory.join("legacy.atelier");
    document::atomic_write(&path, &serde_json::to_vec(&project).unwrap()).unwrap();
    let upgraded = document::load(&path).unwrap();
    assert_eq!(upgraded.version, 2);
    assert_eq!(upgraded.canvases.len(), 1);
    assert_eq!(upgraded.layers[0].strokes.len(), 1);
    assert_eq!(upgraded.references.len(), 1);
    let image = upgraded.canvases[0].layers[0].image.as_ref().unwrap();
    assert_eq!(image.reference.parent.as_deref(), Some("parent-identity"));
    assert!(!image.reference.fingerprint.is_empty());
    document::save(&path, &upgraded).unwrap();
    assert_eq!(document::load(&path).unwrap().canvases.len(), 1);
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn open_project_recovery_keeps_each_document_path_dirty_state_and_drafts() {
    use crate::project_tabs::{self, RecoveryTab};
    let directory = temporary("open-project-tabs");
    let mut first = doc();
    first.title = "Character".into();
    first
        .prompt_drafts
        .insert("new".into(), "first draft".into());
    let mut second = doc();
    second.title = "Outfit".into();
    second
        .prompt_drafts
        .insert("new".into(), "second draft".into());
    project_tabs::save_session(
        &directory.join("open-projects"),
        vec![
            (
                RecoveryTab {
                    epoch: 3,
                    path: Some(directory.join("first.atelier")),
                    dirty: true,
                    active: false,
                },
                first,
            ),
            (
                RecoveryTab {
                    epoch: 7,
                    path: Some(directory.join("second.atelier")),
                    dirty: false,
                    active: true,
                },
                second,
            ),
        ],
    )
    .unwrap();
    let session = project_tabs::recover(&directory).unwrap();
    let tabs = session.tabs;
    let active = session.active.unwrap();
    assert_eq!(session.order, [3, 7]);
    assert_eq!(active.doc.title, "Outfit");
    assert!(!active.dirty);
    assert_eq!(active.doc.prompt_drafts["new"], "second draft");
    assert_eq!(tabs[0].epoch, 3);
    assert!(tabs[0].dirty);
    assert_eq!(tabs[0].doc.prompt_drafts["new"], "first draft");
    assert_eq!(tabs[0].path, Some(directory.join("first.atelier")));
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn reordered_project_recovery_keeps_active_identity_and_closed_tabs_out() {
    use crate::project_tabs::{self, RecoveryTab};
    let directory = temporary("ordered-tabs");
    let mut order = vec![3, 7, 11];
    assert!(project_tabs::reorder(&mut order, 11, 3));
    assert_eq!(order, [11, 3, 7]);
    assert!(!project_tabs::reorder(&mut order, 99, 3));
    assert!(!project_tabs::reorder(&mut order, 3, 3));
    assert!(project_tabs::reorder(&mut order, 11, 7));
    assert_eq!(order, [3, 7, 11]);
    assert!(project_tabs::reorder(&mut order, 7, 3));
    let snapshot = |order: &[u64]| {
        order
            .iter()
            .map(|epoch| {
                let mut project = doc();
                project.title = format!("Project {epoch}");
                project
                    .prompt_drafts
                    .insert("new".into(), format!("draft {epoch}"));
                (
                    RecoveryTab {
                        epoch: *epoch,
                        path: Some(directory.join(format!("{epoch}.atelier"))),
                        dirty: true,
                        active: *epoch == 3,
                    },
                    project,
                )
            })
            .collect()
    };
    let session_dir = directory.join("open-projects");
    project_tabs::save_session(&session_dir, snapshot(&order)).unwrap();
    let recovered = project_tabs::recover(&directory).unwrap();
    assert_eq!(recovered.order, [7, 3, 11]);
    assert_eq!(recovered.active.unwrap().epoch, 3);
    assert_eq!(recovered.tabs[0].doc.prompt_drafts["new"], "draft 7");
    order.retain(|epoch| *epoch != 11);
    project_tabs::save_session(&session_dir, snapshot(&order)).unwrap();
    assert!(session_dir.join("11.atelier").exists());
    let recovered = project_tabs::recover(&directory).unwrap();
    assert_eq!(recovered.order, [7, 3]);
    assert_eq!(recovered.tabs.len(), 1);
    project_tabs::save_session(&session_dir, vec![]).unwrap();
    let recovered = project_tabs::recover(&directory).unwrap();
    assert!(recovered.active.is_none());
    assert!(recovered.order.is_empty());
    assert!(recovered.tabs.is_empty());
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn legacy_project_session_keeps_its_active_tab_and_upgrades_order() {
    let directory = temporary("legacy-tabs");
    let session = directory.join("open-projects");
    document::save(&session.join("3.atelier"), &doc()).unwrap();
    document::save(&session.join("7.atelier"), &doc()).unwrap();
    document::atomic_write(
        &session.join("session.json"),
        br#"[{"epoch":7,"path":null,"dirty":true},{"epoch":3,"path":null,"dirty":false}]"#,
    )
    .unwrap();
    let recovered = crate::project_tabs::recover(&directory).unwrap();
    assert_eq!(recovered.order, [3, 7]);
    assert_eq!(recovered.active.unwrap().epoch, 3);
    document::atomic_write(&session.join("session.json"), br#"[{"epoch":7,"path":null,"dirty":true,"active":true},{"epoch":3,"path":null,"dirty":false,"active":true}]"#).unwrap();
    assert!(crate::project_tabs::recover(&directory).is_err());
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn closing_project_storage_saves_drafts_and_reports_failures_without_overwriting() {
    let directory = temporary("close-project");
    let workers = worker::start();
    let path = directory.join("character.atelier");
    let mut project = doc();
    project.title = "Character with unsaved work".into();
    project
        .prompt_drafts
        .insert("new".into(), "Keep this unsent prompt".into());
    workers
        .storage
        .send(Storage::CloseProject {
            doc: project.clone(),
            path: path.clone(),
            epoch: 3,
            revision: 8,
        })
        .unwrap();
    match workers.events.recv_timeout(Duration::from_secs(5)).unwrap() {
        Event::ProjectClosed {
            epoch,
            revision,
            result,
        } => {
            assert_eq!((epoch, revision), (3, 8));
            assert_eq!(result.unwrap(), path);
        }
        _ => panic!("Expected close result"),
    }
    let saved = document::load(&path).unwrap();
    assert_eq!(saved.prompt_drafts["new"], "Keep this unsent prompt");
    assert_eq!(saved.title, project.title);
    let invalid_path = directory.join("not-a-file.atelier");
    std::fs::create_dir_all(&invalid_path).unwrap();
    workers
        .storage
        .send(Storage::CloseProject {
            doc: project,
            path: invalid_path.clone(),
            epoch: 7,
            revision: 12,
        })
        .unwrap();
    match workers.events.recv_timeout(Duration::from_secs(5)).unwrap() {
        Event::ProjectClosed {
            epoch,
            revision,
            result,
        } => {
            assert_eq!((epoch, revision), (7, 12));
            assert!(result.is_err());
        }
        _ => panic!("Expected close failure"),
    }
    assert!(invalid_path.is_dir());
    assert_eq!(document::load(&path).unwrap().title, saved.title);
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
#[ignore = "Writes isolated native project-tab fixtures under .svs-e2e"]
fn prepare_project_tabs_fixture() {
    use crate::project_tabs::{self, RecoveryTab};
    let directory = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../.svs-e2e/atelier-tabs-fixture");
    std::fs::create_dir_all(&directory).unwrap();
    let mut projects = Vec::new();
    for (epoch, title, draft) in [
        (3, "Character", "Character draft"),
        (7, "Outfit", "Outfit draft"),
        (11, "Save failure", "Keep me when saving fails"),
    ] {
        let mut project = doc();
        project.title = title.into();
        project.active_prompt_target = Some("new".into());
        project.prompt_drafts.insert("new".into(), draft.into());
        let path = directory.join(format!("project-{epoch}.atelier"));
        if epoch == 11 {
            std::fs::create_dir_all(&path).unwrap();
        } else {
            document::save(&path, &project).unwrap();
        }
        projects.push((
            RecoveryTab {
                epoch,
                path: Some(path),
                dirty: true,
                active: epoch == 7,
            },
            project,
        ));
    }
    project_tabs::save_session(&directory.join("open-projects"), projects).unwrap();
    println!("Project-tab fixture: {}", directory.display());
}

#[test]
fn unsupported_reference_combinations_fail_before_credentials_or_provider_execution() {
    for (provider, model, extra, mask) in [
        (Provider::QwenNative, "", 3, false),
        (Provider::KreaNative, "", 0, true),
        (Provider::KreaNative, "", 0, false),
        (Provider::Comfy, "", 0, false),
        (Provider::Fal, "alibaba/qwen-image-3/edit", 3, false),
        (Provider::Fal, "alibaba/qwen-image-3/edit", 0, true),
    ] {
        let settings = Settings {
            provider,
            edit_model: model.into(),
            ..Settings::default()
        };
        let error = providers::validate_inputs(&settings, true, extra, mask)
            .unwrap_err()
            .to_string();
        assert!(!error.contains("credential"), "{error}");
        assert!(!error.contains("Set up"), "{error}");
    }
}

#[test]
fn native_queue_uses_verified_reference_and_latent_mask_capabilities() {
    let settings = Settings {
        provider: Provider::QwenNative,
        ..Settings::default()
    };
    assert_eq!(providers::reference_limit(&settings).unwrap(), 3);
    providers::validate_capabilities(&settings, true, 2, false).unwrap();
    assert!(providers::validate_capabilities(&settings, true, 3, false).is_err());
    providers::validate_capabilities(&settings, true, 2, true).unwrap();
    assert_eq!(
        providers::image_capabilities(&settings, true)
            .unwrap()
            .unwrap()
            .mask,
        agentkit_ai::images::MaskSupport::OptionalLatent
    );
    assert!(providers::validate_capabilities(&settings, false, 1, false).is_err());
}

#[test]
#[ignore = "Writes deterministic native-UI fixtures under .svs-e2e"]
fn prepare_creation_workflow_fixture() {
    use crate::workflow::Destination;
    let directory = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../.svs-e2e/atelier-creation-fixture");
    std::fs::create_dir_all(&directory).unwrap();
    let mut project = doc();
    project.title = "Candidate review fixture".into();
    let original = Arc::new(colored_reference(
        "Accepted blue artwork",
        [30, 80, 180, 255],
        512,
        384,
    ));
    project
        .attach_result(original.clone(), Destination::NewArtboard)
        .unwrap();
    project.references.push(original.clone());
    let mut candidate = colored_reference("Red selection candidate", [210, 40, 50, 255], 128, 96);
    candidate.parent = Some(crate::library::id(&original));
    candidate.prompt = "Local review fixture; no provider used".into();
    project
        .attach_result(
            Arc::new(candidate),
            Destination::Candidate {
                artboard: project.canvas_id,
                rect: (64, 64, 128, 96),
            },
        )
        .unwrap();
    project.prompt_drafts.insert(
        format!("canvas:{}:full", project.canvas_id),
        "Full artboard draft".into(),
    );
    let reference = colored_reference("Green study reference", [40, 170, 80, 255], 512, 384);
    let decoded = document::decode_reference(&reference).unwrap();
    decoded.save(directory.join("study-reference.png")).unwrap();
    crate::library::store(&directory.join("library"), &reference).unwrap();
    project
        .artboard_references
        .push(crate::library::id(&reference));
    project.references.push(Arc::new(reference));
    project.reference = Some(project.references.len() - 1);
    document::save(&directory.join("review.atelier"), &project).unwrap();
}
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
    let providers = agentkit_ai::deployments::Providers::from_toml(
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
[providers.moss]
kind = "sdcpp"
architecture = "moss-image-sft"
capabilities = ["t2i"]
base_url = "http://127.0.0.1:7863"
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
            agentkit_ai::native::sd::Residency::LowVram
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

    settings.provider = Provider::MossNative;
    assert!(Provider::ALL.contains(&Provider::MossNative));
    assert!(!settings.provider.supports_edit());
    let moss = crate::native_images::deployment(&settings, &providers, false).unwrap();
    assert_eq!(moss.architecture, "moss-image-sft");
    assert_eq!(moss.model_dir, directory.join("weights"));
    assert!(moss.profile().tokenizer.is_some());
    assert!(crate::native_images::deployment(&settings, &providers, true).is_err());
    let reopened: Settings =
        serde_json::from_str(&serde_json::to_string(&settings).unwrap()).unwrap();
    assert_eq!(reopened.provider, Provider::MossNative);

    // Clipforge publishes the effective shared endpoint; Atelier consumes it
    // directly, with the same paths for status, preflight and generation.
    let mut qwen = providers
        .image_deployment_for("qwen", agentkit_ai::deployments::Capability::T2i)
        .unwrap();
    qwen.port = 7862;
    let path = directory.join("clipforge.toml");
    providers
        .write_image_deployments(&path, &[("qwen", qwen.clone())])
        .unwrap();
    let shared = agentkit_ai::deployments::Providers::load(&path).unwrap();
    settings.provider = Provider::QwenNative;
    for editing in [false, true] {
        assert_eq!(
            crate::native_images::deployment(&settings, &shared, editing).unwrap(),
            qwen
        );
    }
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
        crop: None,
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
    project.resize(192, 256, true, (1, 1)).unwrap();
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
        crop: None,
    });
    project.layers[0].strokes.push(stroke(Tool::Eraser, 48.));
    project
        .transform_layers_to_bounds(&[0], (48., 32., 64., 48.))
        .unwrap();
    let moved = document::raster(&project, 128, true).unwrap();
    assert_eq!(moved.get_pixel(78, 56)[3], 0);
    assert_eq!(moved.get_pixel(78, 36).0, [0, 80, 255, 255]);
    let before = serde_json::to_vec(&project).unwrap();
    assert!(project
        .transform_layers_to_bounds(&[0], (f32::NAN, 0., 64., 48.))
        .is_err());
    assert!(project
        .transform_layers_to_bounds(&[0], (0., 0., 0.000001, 0.000001))
        .is_err());
    assert_eq!(serde_json::to_vec(&project).unwrap(), before);
    project.validate().unwrap();
}

#[test]
fn rotating_images_and_paint_is_atomic_when_any_selection_is_invalid() {
    let mut project = doc();
    project.layers[0].image = Some(document::ImageLayer {
        reference: Arc::new(colored_reference("Blue", [0, 80, 255, 255], 128, 96)),
        transform: [1., 0., 0., 1., 0., 0.],
        crop: None,
    });
    project.layers[0].strokes.push(stroke(Tool::Eraser, 48.));
    let before = serde_json::to_vec(&project).unwrap();
    assert!(project
        .rotate_layers(&[0, 99], Point { x: 64., y: 48. }, 1.)
        .is_err());
    assert_eq!(serde_json::to_vec(&project).unwrap(), before);
    project
        .rotate_layers(&[0], Point { x: 64., y: 48. }, std::f32::consts::FRAC_PI_2)
        .unwrap();
    let transformed = &project.layers[0].image.as_ref().unwrap().transform;
    assert!((transformed[0]).abs() < 0.0001);
    assert!((transformed[1] - 1.).abs() < 0.0001);
    assert_ne!(serde_json::to_vec(&project).unwrap(), before);
    project.validate().unwrap();
}

#[test]
fn undone_artboard_ids_stay_reserved_for_pending_work() {
    let mut project = doc();
    let mut history = History::new();
    history.checkpoint(&project);
    project.create_canvas().unwrap();
    let source = project.canvas_id;
    assert!(history.undo(&mut project));
    assert!(history.redo(&mut project));
    assert_eq!(project.canvas_id, source);
    assert!(history.undo(&mut project));

    let mut project: Document =
        serde_json::from_slice(&serde_json::to_vec(&project).unwrap()).unwrap();
    project.create_canvas().unwrap();
    assert!(project.canvas_id > source);
    let reference = colored_reference("Pending", [255, 0, 0, 255], 32, 16);
    let rect = (0, 0, 128, 96);
    assert!(project
        .attach_imported_images(source, rect, vec![reference.clone()])
        .is_err());
    assert!(project
        .attach_result(
            Arc::new(reference),
            crate::workflow::Destination::NewLayer {
                artboard: source,
                rect
            },
        )
        .is_err());
    assert_eq!(project.layers.len(), 1);
}

#[test]
fn imported_artwork_preserves_paint_target_and_rejects_missing_artboard_atomically() {
    let mut project = doc();
    let images = vec![
        colored_reference("First", [255, 0, 0, 255], 32, 16),
        colored_reference("Second", [0, 0, 255, 255], 16, 32),
    ];
    let before = serde_json::to_vec(&project).unwrap();
    assert!(project
        .attach_imported_images(999, (0, 0, 128, 96), images.clone())
        .is_err());
    assert_eq!(serde_json::to_vec(&project).unwrap(), before);
    let active = project.active_layer;
    let paint = project.paint_layer;
    let indices = project
        .attach_imported_images(project.canvas_id, (0, 0, 128, 96), images)
        .unwrap();
    assert_eq!(indices, vec![1, 2]);
    assert_eq!(project.active_layer, active);
    assert_eq!(project.paint_layer, paint);
    assert!(indices
        .iter()
        .all(|index| project.layers[*index].image.is_some()));
    assert!(project.layer_bounds_for(1).unwrap().2 < project.layer_bounds_for(2).unwrap().0);
}

#[test]
fn group_opacity_applies_to_composited_artwork_once() {
    let mut project = doc();
    project.version = 3;
    project.layers[0].group = Some("Group".into());
    project.layers[0].image = Some(document::ImageLayer {
        reference: Arc::new(colored_reference("Red", [255, 0, 0, 255], 16, 16)),
        transform: [1., 0., 0., 1., 0., 0.],
        crop: None,
    });
    project.layers.push(project.layers[0].clone());
    project.layer_groups.insert(
        "Group".into(),
        document::LayerGroup {
            opacity: 0.5,
            ..Default::default()
        },
    );
    let pixel = document::raster(&project, 128, true)
        .unwrap()
        .get_pixel(4, 4)
        .0;
    assert!((126..=129).contains(&pixel[3]), "group alpha: {pixel:?}");
    project.layer_groups.get_mut("Group").unwrap().visible = false;
    assert_eq!(
        document::raster(&project, 128, true)
            .unwrap()
            .get_pixel(4, 4)[3],
        0
    );
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
        candidate: false,
        name: "Top".into(),
        visible: true,
        locked: false,
        opacity: 1.,
        group: None,
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
    doc.resize(256, 192, false, (0, 0)).unwrap();
    assert_eq!(doc.layers[0].strokes[0].points, original);
    doc.resize(128, 96, true, (1, 1)).unwrap();
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
    let error = worker::apply_settings(&Settings::default(), &directory).unwrap_err();
    assert!(format!("{error:#}").contains("Settings file was not saved"));
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "previous settings");
    std::fs::remove_dir_all(directory).unwrap();
}
#[test]
fn shared_provider_config_supplies_hosted_image_credentials() {
    let registry = agentkit_ai::deployments::Providers::from_toml(
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

    let environment_registry = agentkit_ai::deployments::Providers::from_toml(
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
fn comfy_workflow_roundtrip_stores_output_and_reopens_in_atelier() {
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
    let directory = temporary("creative-workflow");
    let (reference, thumbnail) = crate::creative_workflow::generate(
        providers::Request {
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
        },
        &directory,
    )
    .unwrap();
    assert_eq!(document::decode_reference(&reference).unwrap().width(), 32);
    assert_eq!(thumbnail.width(), 32);
    assert!(reference.provenance.contains("A ceramic pitcher"));
    assert_eq!(reference.prompt, "A ceramic pitcher");
    let saved = std::fs::read_dir(directory.join("generated"))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .find(|path| {
            path.extension()
                .is_some_and(|extension| extension == "json")
        })
        .unwrap();
    let run_id = saved
        .file_stem()
        .unwrap()
        .to_str()
        .unwrap()
        .strip_prefix("reference-")
        .unwrap();
    use storyfella_studio_core::workflows::{
        framework::{WorkflowRunEvents, WorkflowRunStatus, WorkflowRunStore},
        RunStore,
    };
    let store = RunStore::open(&directory.join("workflows.sqlite3")).unwrap();
    let state = store.load(run_id).unwrap().unwrap();
    assert_eq!(state.status, WorkflowRunStatus::Completed);
    assert!(store.events(run_id).unwrap().len() > 2);
    assert_eq!(
        store.context(run_id).unwrap().unwrap()["prompt"],
        "A ceramic pitcher"
    );
    let mut project = doc();
    project
        .attach_result(
            Arc::new(reference.clone()),
            crate::workflow::Destination::NewArtboard,
        )
        .unwrap();
    let path = directory.join("result.atelier");
    document::save(&path, &project).unwrap();
    let reopened = document::load(&path).unwrap();
    assert!(reopened
        .layers
        .iter()
        .chain(
            reopened
                .canvases
                .iter()
                .flat_map(|canvas| canvas.layers.iter())
        )
        .filter_map(|layer| layer.image.as_ref())
        .any(|image| image.reference.image == reference.image));
    server.join().unwrap();
}
#[test]
#[ignore = "Runs a local rate-limit UI fixture server under .svs-e2e until stop-server exists"]
fn serve_activity_ui_fixture() {
    use crate::project_tabs::{self, RecoveryTab};
    let directory = std::env::var_os("ATELIER_UI_FIXTURE_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../.svs-e2e/atelier-activity-ui")
        });
    std::fs::create_dir_all(&directory).unwrap();
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    let settings = Settings {
        provider: Provider::Comfy,
        endpoint: endpoint.clone(),
        workflow: workflow(),
        ..Settings::default()
    };
    document::atomic_write(
        &directory.join("settings.json"),
        &serde_json::to_vec(&settings).unwrap(),
    )
    .unwrap();
    let mut projects = vec![];
    for (epoch, title, prompt) in [
        (3, "Character", "Make the jacket a deeper blue."),
        (7, "Outfit", "A separate outfit study."),
    ] {
        let mut project = doc();
        project.title = title.into();
        project.active_prompt_target = Some("new".into());
        project.prompt_drafts.insert("new".into(), prompt.into());
        let path = directory.join(format!("project-{epoch}.atelier"));
        if epoch == 3 {
            std::fs::create_dir_all(&path).unwrap();
        } else {
            document::save(&path, &project).unwrap();
        }
        projects.push((
            RecoveryTab {
                epoch,
                path: Some(path),
                dirty: true,
                active: epoch == 3,
            },
            project,
        ));
    }
    project_tabs::save_session(&directory.join("open-projects"), projects).unwrap();
    println!(
        "Activity fixture ready: {} at {endpoint}",
        directory.display()
    );
    let mut image = std::io::Cursor::new(Vec::new());
    image::DynamicImage::new_rgb8(32, 32)
        .write_to(&mut image, image::ImageFormat::Png)
        .unwrap();
    let png = image.into_inner();
    let started = std::time::Instant::now();
    while started.elapsed() < Duration::from_secs(30 * 60)
        && !directory.join("stop-server").exists()
    {
        let (mut stream, _) = match listener.accept() {
            Ok(connection) => connection,
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(20));
                continue;
            }
            Err(error) => panic!("Fixture accept: {error}"),
        };
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let mut request = Vec::new();
        let mut buffer = [0; 4096];
        let header_end = loop {
            let n = stream.read(&mut buffer).unwrap();
            if n == 0 {
                break None;
            }
            request.extend_from_slice(&buffer[..n]);
            assert!(request.len() < 2 * 1024 * 1024);
            if let Some(position) = request.windows(4).position(|bytes| bytes == b"\r\n\r\n") {
                break Some(position + 4);
            }
        };
        let Some(header_end) = header_end else {
            continue;
        };
        let headers = String::from_utf8_lossy(&request[..header_end]).to_string();
        let size = headers
            .lines()
            .find_map(|line| {
                line.to_lowercase()
                    .strip_prefix("content-length:")
                    .and_then(|value| value.trim().parse::<usize>().ok())
            })
            .unwrap_or(0);
        while request.len() < header_end + size {
            let n = stream.read(&mut buffer).unwrap();
            if n == 0 {
                break;
            }
            request.extend_from_slice(&buffer[..n]);
            assert!(request.len() < 2 * 1024 * 1024);
        }
        let (status, body) = if headers.starts_with("POST /prompt ") {
            let request: serde_json::Value =
                serde_json::from_slice(&request[header_end..]).unwrap();
            let mut log = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(directory.join("requests.jsonl"))
                .unwrap();
            writeln!(log, "{request}").unwrap();
            if directory.join("allow-success").exists() {
                ("200 OK", br#"{"prompt_id":"activity-result"}"#.to_vec())
            } else {
                let message = format!("Request was throttled. Your rate limit is reduced while account credit is low. {}", "Additional provider diagnostic context. ".repeat(30));
                (
                    "429 Too Many Requests",
                    serde_json::to_vec(&serde_json::json!({"error":{"message":message}})).unwrap(),
                )
            }
        } else if headers.starts_with("GET /history/") && directory.join("hold-jobs").exists() {
            ("200 OK", b"{}".to_vec())
        } else if headers.starts_with("GET /history/") {
            ("200 OK", br#"{"activity-result":{"status":{"completed":true,"status_str":"success"},"outputs":{"9":{"images":[{"filename":"output.png","subfolder":"","type":"output"}]}}}}"#.to_vec())
        } else if headers.starts_with("GET /view?") {
            ("200 OK", png.clone())
        } else {
            ("200 OK", b"{}".to_vec())
        };
        write!(
            stream,
            "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        )
        .unwrap();
        stream.write_all(&body).unwrap();
    }
}

#[test]
fn generation_cancellation_prevents_network_submission() {
    let directory = temporary("workflow-cancel");
    let result = crate::creative_workflow::generate(
        providers::Request {
            native: Arc::new(Default::default()),
            settings: Settings::default(),
            prompt: "Study".into(),
            source: None,
            references: vec![],
            mask: None,
            progress: None,
            size: (1024, 1024),
            cancel: Arc::new(AtomicBool::new(true)),
        },
        &directory,
    );
    assert!(result.unwrap_err().to_string().contains("cancelled"));
    assert!(!directory.join("generated").exists());
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
        crop: None,
    });
    painting.layers[0].strokes.push(stroke(Tool::Pencil, 30.));
    painting.layers[0].strokes.push(stroke(Tool::Eraser, 70.));
    let workers = worker::start();
    workers
        .storage
        .send(Storage::CanvasArtwork(
            painting,
            path.clone(),
            true,
            crate::activity::Scope::Project(1),
        ))
        .unwrap();
    match workers
        .events
        .recv_timeout(Duration::from_secs(15))
        .unwrap()
    {
        Event::Exported(scope, result) => {
            assert_eq!(scope, crate::activity::Scope::Project(1));
            assert_eq!(result.unwrap(), path);
        }
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
    restored.resize(128, 96, true, (1, 1)).unwrap();
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
#[test]
fn shared_story_folder_relocates_with_its_editable_document_and_artwork() {
    use storyfella_studio_core::{
        persistence::ProjectDb, project::ProjectFolder, registry::ArtifactRegistry,
    };
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../.svs-e2e/tests")
        .join(format!("shared-folder-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    let folder = ProjectFolder::create(
        &root.join("original"),
        "Harbor",
        &ArtifactRegistry::default(),
        "scene harbor\n  end\n",
    )
    .unwrap();
    let path = crate::document::project_document(&folder.manifest()).unwrap();
    let mut doc = crate::document::load(&path).unwrap();
    assert_eq!(doc.title, "Harbor");
    doc.note = "Retain my visual working document".into();
    let mut bytes = std::io::Cursor::new(Vec::new());
    image::DynamicImage::new_rgba8(2, 2)
        .write_to(&mut bytes, image::ImageFormat::Png)
        .unwrap();
    let reference =
        crate::document::reference_from_bytes("cup".into(), &bytes.into_inner(), "fixture".into())
            .unwrap();
    doc.references.push(std::sync::Arc::new(reference));
    crate::document::save(&path, &doc).unwrap();
    std::fs::rename(folder.root(), root.join("relocated")).unwrap();
    let moved = ProjectFolder::open(&root.join("relocated")).unwrap();
    let path = crate::document::project_document(&moved.manifest()).unwrap();
    let reopened = crate::document::load(&path).unwrap();
    assert_eq!(reopened.note, doc.note);
    assert_eq!(reopened.references[0].image, doc.references[0].image);
    assert_eq!(
        ProjectDb::open(moved.database())
            .unwrap()
            .load()
            .unwrap()
            .story
            .source(),
        "scene harbor\n  end\n"
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn shared_catalog_worker_refreshes_items_and_document_links_survive_save_rename_and_move() {
    use storyfella_studio_core::{
        catalog::{AssetLink, CatalogEdit},
        persistence::ProjectDb,
        project::ProjectFolder,
        registry::{ArtifactKind, ArtifactRegistry},
    };
    let root = temporary("shared-catalog");
    std::fs::create_dir_all(&root).unwrap();
    let source = "scene harbor\n  end\n";
    let folder = ProjectFolder::create(
        &root.join("project"),
        "Harbor",
        &ArtifactRegistry::default(),
        source,
    )
    .unwrap();
    let path = crate::document::project_document(&folder.manifest()).unwrap();
    let snapshot = ProjectDb::open(folder.database()).unwrap().load().unwrap();
    let workers = worker::start();
    workers
        .storage
        .send(Storage::CatalogEdit {
            epoch: 7,
            path: folder.database(),
            snapshot,
            resolution: storyfella_studio_core::catalog::EditResolution::Check,
            edit: CatalogEdit::Create {
                kind: ArtifactKind::Character,
                name: "mara".into(),
                description: "Captain".into(),
                variant_of: None,
            },
        })
        .unwrap();
    let (id, snapshot) = match workers.events.recv_timeout(Duration::from_secs(5)).unwrap() {
        Event::CatalogSaved {
            epoch: 7,
            result: Ok(saved),
            ..
        } => saved,
        _ => panic!("Catalog save should report its captured project"),
    };
    let mut document = crate::document::load(&path).unwrap();
    document.asset_links.push(AssetLink {
        asset_id: id,
        canvas_id: document.canvas_id,
    });
    crate::document::save(&path, &document).unwrap();
    workers
        .storage
        .send(Storage::CatalogEdit {
            epoch: 7,
            path: folder.database(),
            snapshot,
            resolution: storyfella_studio_core::catalog::EditResolution::Check,
            edit: CatalogEdit::Describe {
                id,
                name: "mara".into(),
                description: "Updated story requirement".into(),
            },
        })
        .unwrap();
    match workers.events.recv_timeout(Duration::from_secs(5)).unwrap() {
        Event::CatalogSaved {
            result: Ok((same, stored)),
            ..
        } => {
            assert_eq!(same, id);
            assert_eq!(stored.story.source(), source);
        }
        _ => panic!("Catalog edit should preserve identity"),
    }
    let db = ProjectDb::open_existing(folder.database()).unwrap();
    let mut renamed = db.load().unwrap();
    renamed.registry.artifact_mut(id).unwrap().name = "captain_mara".into();
    db.save_registry_checked(&renamed.registry, renamed.registry_version)
        .unwrap();
    workers
        .storage
        .send(Storage::RefreshCatalog {
            epoch: 7,
            path: folder.database(),
        })
        .unwrap();
    match workers.events.recv_timeout(Duration::from_secs(5)).unwrap() {
        Event::CatalogRefreshed {
            epoch: 7,
            path,
            result: Ok(stored),
        } => {
            assert_eq!(path, folder.database());
            assert_eq!(stored.registry.artifact(id).unwrap().name, "captain_mara");
        }
        _ => panic!("Refresh must retain its captured project identity"),
    }
    drop(db);
    std::fs::rename(folder.root(), root.join("moved")).unwrap();
    let moved = ProjectFolder::open(&root.join("moved")).unwrap();
    let document = crate::document::load(&moved.atelier_document().unwrap()).unwrap();
    assert_eq!(document.asset_links[0].asset_id, id);
    let stored = ProjectDb::open(moved.database()).unwrap().load().unwrap();
    assert_eq!(stored.registry.artifact(id).unwrap().name, "captain_mara");
    assert_eq!(
        stored.registry.artifact(id).unwrap().description,
        "Updated story requirement"
    );
    let mut invalid = document.clone();
    invalid.asset_links.push(invalid.asset_links[0].clone());
    assert!(invalid.validate().is_err());
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn studio_handoff_reuses_artboard_preserves_prompt_and_drawing_and_embeds_existing_artwork() {
    use storyfella_studio_core::{
        atelier::OpenRequest,
        catalog::{self, CatalogEdit},
        project::ProjectFolder,
        registry::{ArtifactKind, ArtifactRegistry},
    };
    let root = temporary("studio-handoff");
    std::fs::create_dir_all(&root).unwrap();
    let image = root.join("source.png");
    image::RgbaImage::from_pixel(64, 64, image::Rgba([12, 34, 56, 255]))
        .save(&image)
        .unwrap();
    let mut registry = ArtifactRegistry::default();
    let id = catalog::apply(
        &mut registry,
        CatalogEdit::Create {
            kind: ArtifactKind::Character,
            name: "mara".into(),
            description: "Captain in blue".into(),
            variant_of: None,
        },
    )
    .unwrap();
    catalog::apply(&mut registry, CatalogEdit::ImportImage { id, path: image }).unwrap();
    let source = "scene harbor\n  cast: mara\n  beat first\n    mara: Hello\n";
    let folder = ProjectFolder::create(&root.join("story"), "Harbor", &registry, source).unwrap();
    let mut request = OpenRequest::prepare(&folder.database(), source, id).unwrap();
    let workers = worker::start();
    workers
        .storage
        .send(Storage::StoryContext(request.clone()))
        .unwrap();
    let target = match workers.events.recv_timeout(Duration::from_secs(5)).unwrap() {
        Event::StoryContext(received, Ok(target)) => {
            assert_eq!(received.id, request.id);
            target
        }
        _ => panic!("Handoff should resolve on the storage worker"),
    };
    let mut doc = crate::document::load(&target.path).unwrap();
    let canvas = target.apply(&mut doc, &request).unwrap();
    assert_eq!(doc.asset_links[0].asset_id, id);
    assert!(doc.layers[0].image.is_some());
    assert!(!doc.layers[0].candidate);
    let prompt = format!("canvas:{canvas}:full");
    assert_eq!(doc.prompt_drafts[&prompt], "Captain in blue");
    doc.layers[0].name = "My edited layer".into();
    doc.prompt_drafts
        .insert(prompt.clone(), "My unsent prompt".into());
    request.description = "Updated description".into();
    request.context = "Updated context".into();
    assert_eq!(target.apply(&mut doc, &request).unwrap(), canvas);
    assert_eq!(doc.layers[0].name, "My edited layer");
    assert_eq!(doc.prompt_drafts[&prompt], "My unsent prompt");
    assert_eq!(doc.asset_links.len(), 1);
    assert_eq!(doc.story_context[&canvas], "Updated context");
    crate::document::save(&target.path, &doc).unwrap();
    let reopened = crate::document::load(&target.path).unwrap();
    assert_eq!(reopened.asset_links, doc.asset_links);
    assert_eq!(reopened.story_context, doc.story_context);
    assert_eq!(reopened.prompt_drafts[&prompt], "My unsent prompt");
    assert!(crate::story_context::prepare_target(&request)
        .unwrap()
        .artwork
        .is_none());
    assert_eq!(
        std::fs::read_to_string(folder.root().join("assets/story.sf")).unwrap(),
        source
    );
    let before = serde_json::to_value(&doc).unwrap();
    request.context = "a".repeat(131_073);
    assert!(target.apply(&mut doc, &request).is_err());
    assert_eq!(serde_json::to_value(&doc).unwrap(), before);
    std::fs::remove_dir_all(root).unwrap();
}
