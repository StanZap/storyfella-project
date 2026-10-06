use storyfella_compiler::*;
use storyfella_language::{AssetKind, Severity, Source};

fn binding(name: &str, kind: AssetKind) -> AssetBinding {
    AssetBinding {
        id: format!("asset-{name}"),
        name: name.into(),
        kind,
        revision: "revision-1".into(),
    }
}

#[test]
fn scenes_fall_through_and_last_boundary_ends() {
    let text = "scene first\n  narrate: One\nscene second\n  narrate: Two\nscene empty\n";
    let result = compile(text, &[]);
    assert!(result.ready(), "{:?}", result.diagnostics);
    let plan = result.plan.unwrap();
    assert_eq!(plan.entry, SceneId("first".into()));
    for (scene, next) in plan.scenes[..2].iter().zip(["second", "empty"]) {
        let boundary = scene.instructions.last().unwrap();
        assert_eq!(boundary.origin, Origin::SceneBoundary);
        assert!(matches!(&boundary.kind, InstructionKind::Jump { target, .. } if target.0 == next));
        assert_eq!(boundary.source.start, boundary.source.end);
    }
    assert!(matches!(
        plan.scenes[2].instructions[0].kind,
        InstructionKind::End
    ));
    assert_eq!(plan.scenes[2].instructions[0].origin, Origin::SceneBoundary);
}

#[test]
fn beats_and_prose_preserve_source_order_scopes_and_text() {
    let text = "scene harbor\n  narrate: Before\n  beat arrival\n    Mara: \"Hola 🌊\"\n    Mara think: She waits.\n  beat departure\n    narrate: After\n  prop: lantern\n  end\n";
    let result = compile(
        text,
        &[
            binding("mara", AssetKind::Character),
            binding("lantern", AssetKind::Prop),
        ],
    );
    assert!(result.ready(), "{:?}", result.diagnostics);
    let plan = result.plan.unwrap();
    let instructions = &plan.scenes[0].instructions;
    assert_eq!(instructions.len(), 8);
    assert!(instructions[0].beat.is_none());
    assert!(
        matches!(&instructions[1].kind, InstructionKind::BeginBeat { id } if id.name == "arrival")
    );
    match &instructions[2].kind {
        InstructionKind::Speech {
            speaker,
            text,
            thought,
        } => {
            assert_eq!(text.value, "Hola 🌊");
            assert!(!thought);
            assert_eq!(speaker.binding.as_ref().unwrap().revision, "revision-1");
        }
        _ => panic!(),
    }
    assert!(matches!(
        &instructions[3].kind,
        InstructionKind::Speech { thought: true, .. }
    ));
    assert_eq!(instructions[2].beat.as_ref().unwrap().name, "arrival");
    assert_eq!(instructions[5].beat.as_ref().unwrap().name, "departure");
    assert!(
        instructions[6].beat.is_none(),
        "scene-level statements restore scene scope"
    );
    assert_eq!(
        Source::new(text).slice(instructions[2].source),
        Some("Mara: \"Hola 🌊\"")
    );
    assert_eq!(instructions[7].origin, Origin::Authored);
}

#[test]
fn branches_resolve_forward_targets_and_intentional_cycles() {
    let text = "scene start\n  choice \"Where?\"\n    option \"Home\": home\n    option \"Again\": start\nscene home\n  goto start\nscene unused\n  end\n";
    let result = compile(text, &[]);
    assert!(result.ready());
    let plan = result.plan.unwrap();
    match &plan.scenes[0].instructions[0].kind {
        InstructionKind::Choice { options, .. } => {
            assert_eq!(options[0].target.0, "home");
            assert_eq!(
                Source::new(text).slice(options[0].target_source),
                Some("home")
            );
        }
        _ => panic!(),
    }
    assert_eq!(
        plan.scenes[0].instructions.len(),
        1,
        "choice is terminal; no fallback jump"
    );
    assert!(result
        .diagnostics
        .iter()
        .any(|d| d.code == "SF305" && d.severity == Severity::Warning));
}

#[test]
fn terminal_instructions_reject_following_statements_including_other_beats() {
    for source in [
        "scene a\n  end\n  narrate: unreachable\n",
        "scene a\n  beat one\n    goto a\n  beat two\n    narrate: unreachable\n",
        "scene a\n  choice \"Where?\"\n    option \"A\": a\n    option \"B\": b\n  narrate: unreachable\nscene b\n  end",
    ] {
        let result = compile(source, &[]);
        assert!(result.plan.is_none());
        assert!(result.diagnostics.iter().any(|d| d.code == "SF301"), "{:?}", result.diagnostics);
    }
}

#[test]
fn pending_assets_and_unsupported_directions_are_distinct_from_source_errors() {
    let text = "scene harbor\n  cast: mara\n  shot: wide, dawn\n  mara: Hello\n";
    let result = compile(text, &[]);
    assert!(!result.has_errors());
    assert!(!result.ready());
    let plan = result.plan.unwrap();
    assert_eq!(plan.readiness.pending_assets.len(), 2);
    assert_eq!(plan.readiness.unsupported_directions.len(), 1);
    assert!(
        matches!(&plan.scenes[0].instructions[1].kind, InstructionKind::Direction { text, .. } if text.value == "wide, dawn")
    );
    let bound = compile(text, &[binding("mara", AssetKind::Character)]);
    assert!(bound
        .plan
        .as_ref()
        .unwrap()
        .readiness
        .pending_assets
        .is_empty());
    assert!(
        !bound.ready(),
        "binding an asset does not implement a direction"
    );
    assert!(compile("scene a\n  goto missing", &[]).plan.is_none());
}

#[test]
fn bindings_are_immutable_complete_and_unambiguous() {
    let text = "scene a\n  mara: Hi";
    let a = binding("mara", AssetKind::Character);
    assert!(compile(text, std::slice::from_ref(&a)).ready());
    for bindings in [
        vec![AssetBinding {
            name: "".into(),
            ..a.clone()
        }],
        vec![AssetBinding {
            revision: "".into(),
            ..a.clone()
        }],
        vec![AssetBinding {
            id: "".into(),
            ..a.clone()
        }],
        vec![
            a.clone(),
            AssetBinding {
                name: "Mara".into(),
                id: "different".into(),
                ..a.clone()
            },
        ],
        vec![
            a.clone(),
            AssetBinding {
                name: "other".into(),
                ..a.clone()
            },
        ],
        vec![binding("mara", AssetKind::Prop)],
    ] {
        let result = compile(text, &bindings);
        assert!(result.has_errors());
        assert!(result.plan.is_none(), "{:?}", result.diagnostics);
    }
}

#[test]
fn invalid_drafts_and_ambiguous_option_labels_do_not_emit_plans() {
    for text in [
        "",
        "story \"Draft\"",
        "scene a\n   end",
        "scene a\n  choice \"\"\n    option \"X\": a\n    option \"Y\": a",
        "scene a\n  choice \"Where\"\n    option \"X\": a\n    option \"X\": a",
        "scene a\n  choice \"Where\"\n    option \" \" : a\n    option \"Y\": a",
    ] {
        let result = compile(text, &[]);
        assert!(
            result.has_errors() && result.plan.is_none(),
            "accepted {text}"
        );
    }
}

#[test]
fn plan_is_deterministic_round_trips_and_ids_survive_source_edits() {
    let text = include_str!("../../../examples/harbor.sf");
    let result = compile(text, &[]);
    assert_eq!(result, compile(text, &[]));
    let plan = result.plan.unwrap();
    let json = serde_json::to_string(&plan).unwrap();
    assert_eq!(serde_json::from_str::<StoryPlan>(&json).unwrap(), plan);
    assert_eq!(plan.source, text);
    assert_eq!(plan.version, PLAN_VERSION);
    let edited = compile(&text.replace("You're late.", "Welcome back."), &[])
        .plan
        .unwrap();
    assert_eq!(plan.entry, edited.entry);
    let beats = |p: &StoryPlan| {
        p.scenes
            .iter()
            .flat_map(|s| &s.instructions)
            .filter_map(|i| match &i.kind {
                InstructionKind::BeginBeat { id } => Some(id.clone()),
                _ => None,
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(beats(&plan), beats(&edited));
    let source = Source::new(text);
    for scene in &plan.scenes {
        assert!(source.slice(scene.source).is_some());
        assert!(scene
            .instructions
            .iter()
            .all(|i| source.slice(i.source).is_some()));
    }
}
