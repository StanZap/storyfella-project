use storyfella_compiler::{compile, AssetBinding, InstructionKind, SceneId, StoryPlan};
use storyfella_language::{AssetKind, Span};
use storyfella_runtime::{
    AssetPolicy, Command, Event, Phase, Runtime, TextKind, TRANSITION_BUDGET,
};

fn plan(source: &str) -> StoryPlan {
    compile(
        source,
        &[AssetBinding {
            id: "mara".into(),
            name: "mara".into(),
            kind: AssetKind::Character,
            revision: "portrait-v1".into(),
        }],
    )
    .plan
    .unwrap()
}
const STORY: &str = "scene empty\nscene harbor\n  cast: mara\n  beat arrival\n    mara: Hello 🌊\n    mara think: A thought\n    narrate: Rain\n    choice \"Go?\"\n      option \"Light\": lighthouse\n      option \"Stay\": shore\nscene lighthouse\n  narrate: The light\n  end\nscene shore\n  narrate: The shore\n";

#[test]
fn version_one_save_without_flags_or_workspace_map_keeps_its_identity() {
    use sha2::{Digest, Sha256};
    let original = r#"{"version":1,"source":"scene start\n  end\n","title":null,"entry":"start","scenes":[{"id":"start","source":{"start":0,"end":18},"instructions":[{"source":{"start":14,"end":17},"origin":"Authored","beat":null,"kind":"End"}]}],"readiness":{"pending_assets":[],"unsupported_directions":[]}}"#;
    let plan: StoryPlan = serde_json::from_str(original).unwrap();
    assert_eq!(serde_json::to_string(&plan).unwrap(), original);
    let saved = serde_json::from_value(serde_json::json!({
        "version": 1,
        "plan_identity": Sha256::digest(original).iter().map(|byte| format!("{byte:02x}")).collect::<String>(),
        "commands": [],
        "checkpoint": {
            "scene": 0, "instruction": 0, "beat": null, "assets": [],
            "phase": "Complete", "paused": false
        }
    })).unwrap();
    assert_eq!(
        Runtime::restore(plan, saved).unwrap().state().phase,
        Phase::Complete
    );
}

#[test]
fn conditional_flow_replays_flag_assignments_and_restart_resets_them() {
    let source = "flag key false\nscene entry\n  choice \"Take key?\"\n    option \"Take\": pickup\n    option \"Leave\": door\nscene pickup\n  set key true\n  goto door\nscene door\n  if not key goto locked else inside\nscene locked\n  narrate: Locked\n  end\nscene inside\n  narrate: Open\n  end\n";
    let compiled = plan(source);
    for (index, expected, value) in [(0, "Open", true), (1, "Locked", false)] {
        let (mut runtime, _) = Runtime::start(compiled.clone()).unwrap();
        let events = runtime.dispatch(Command::SelectChoice { index }).unwrap();
        assert!(events
            .iter()
            .any(|event| matches!(event, Event::Text { text, .. } if text.value == expected)));
        assert_eq!(runtime.state().flags["key"], value);
        assert_eq!(
            events.iter().any(
                |event| matches!(event, Event::FlagSet { name, value: true, .. } if name == "key")
            ),
            value
        );
        let save = runtime.save();
        let mut restored = Runtime::restore(compiled.clone(), save.clone()).unwrap();
        assert_eq!(restored.state(), runtime.state());
        restored.dispatch(Command::Restart).unwrap();
        assert!(!restored.state().flags["key"]);
        assert_eq!(restored.state().phase, Phase::Choice);
        let mut corrupted = save;
        corrupted.checkpoint.flags.insert("key".into(), !value);
        assert!(Runtime::restore(compiled.clone(), corrupted).is_err());
    }
}

#[test]
fn deterministic_traces_cover_every_text_kind_beats_fallthrough_and_both_routes() {
    for route in 0..2 {
        let trace = || {
            let (mut runtime, mut events) = Runtime::start(plan(STORY)).unwrap();
            for command in [
                Command::Advance,
                Command::Advance,
                Command::Advance,
                Command::SelectChoice { index: route },
                Command::Advance,
            ] {
                events.extend(runtime.dispatch(command).unwrap());
            }
            assert_eq!(runtime.state().phase, Phase::Complete);
            events
        };
        let events = trace();
        assert_eq!(events, trace());
        assert!(matches!(&events[0], Event::SceneEntered { scene, .. } if scene.0 == "empty"));
        assert!(events
            .iter()
            .any(|e| matches!(e, Event::BeatEntered { beat, .. } if beat.name == "arrival")));
        for kind in [TextKind::Dialogue, TextKind::Thought, TextKind::Narration] {
            assert!(events
                .iter()
                .any(|e| matches!(e, Event::Text { kind: k, .. } if *k == kind)));
        }
        let text = events
            .iter()
            .filter_map(|e| {
                if let Event::Text { text, .. } = e {
                    Some(text.value.as_str())
                } else {
                    None
                }
            })
            .next_back()
            .unwrap();
        assert_eq!(text, if route == 0 { "The light" } else { "The shore" });
        assert!(events.iter().any(
            |e| matches!(e, Event::Assets { bindings, .. } if bindings[0].revision == "portrait-v1")
        ));
    }
}

#[test]
fn pause_invalid_choices_jump_end_restart_and_verified_restore() {
    let p = plan(STORY);
    let (mut runtime, _) = Runtime::start(p.clone()).unwrap();
    assert!(runtime
        .dispatch(Command::SelectChoice { index: 0 })
        .is_err());
    runtime.dispatch(Command::Pause).unwrap();
    let saved = runtime.save();
    assert!(runtime.dispatch(Command::Advance).is_err());
    assert_eq!(saved, runtime.save());
    let mut restored = Runtime::restore(p.clone(), saved).unwrap();
    for command in [
        Command::Resume,
        Command::Advance,
        Command::Advance,
        Command::Advance,
    ] {
        assert_eq!(
            runtime.dispatch(command.clone()).unwrap(),
            restored.dispatch(command).unwrap()
        );
    }
    let waiting = runtime.save();
    assert!(runtime
        .dispatch(Command::SelectChoice { index: 2 })
        .is_err());
    assert!(runtime.dispatch(Command::Advance).is_err());
    assert_eq!(waiting, runtime.save());
    runtime
        .dispatch(Command::Jump {
            scene: SceneId("shore".into()),
        })
        .unwrap();
    let before = runtime.save();
    assert!(runtime
        .dispatch(Command::Jump {
            scene: SceneId("missing".into())
        })
        .is_err());
    assert_eq!(runtime.save(), before);
    runtime.dispatch(Command::End).unwrap();
    assert_eq!(runtime.state().phase, Phase::Complete);
    assert!(runtime.dispatch(Command::Advance).is_err());
    let mut restored = Runtime::restore(p.clone(), runtime.save()).unwrap();
    assert_eq!(
        runtime.dispatch(Command::Restart).unwrap(),
        restored.dispatch(Command::Restart).unwrap()
    );
    assert_eq!(runtime.state().phase, Phase::Text);
    assert!(runtime.save().commands.is_empty());
    assert_eq!(
        Runtime::restore(p, runtime.save()).unwrap().state(),
        runtime.state()
    );
}

#[test]
fn intentional_jump_cycle_is_bounded_and_recoverable() {
    let (mut runtime, events) =
        Runtime::start(plan("scene loop\n  goto loop\nscene exit\n  end\n")).unwrap();
    assert!(events.len() <= TRANSITION_BUDGET + 2);
    assert!(matches!(
        events.last(),
        Some(Event::TransitionBudgetExceeded { .. })
    ));
    assert_eq!(runtime.state().phase, Phase::Budget);
    assert!(matches!(
        runtime.dispatch(Command::Advance).unwrap().last(),
        Some(Event::TransitionBudgetExceeded { .. })
    ));
    assert!(Runtime::restore(runtime.plan().clone(), runtime.save()).is_ok());
    runtime
        .dispatch(Command::Jump {
            scene: SceneId("exit".into()),
        })
        .unwrap();
    assert_eq!(runtime.state().phase, Phase::Complete);
}

#[test]
fn missing_assets_can_run_with_blackbox_policy() {
    let source = "scene harbor\n  cast: mara\n  mara: Welcome to Bellwater\n  end\n";
    let plan = storyfella_compiler::compile(source, &[]).plan.unwrap();
    assert!(Runtime::start(plan.clone()).is_err());
    let (mut runtime, events) =
        Runtime::start_with_policy(plan, AssetPolicy::AllowMissing).unwrap();
    assert!(events.iter().any(|event| matches!(
        event,
        Event::Text {
            kind: TextKind::Dialogue,
            speaker: None,
            ..
        }
    )));
    runtime.dispatch(Command::Advance).unwrap();
    assert_eq!(runtime.state().phase, Phase::Complete);
}

#[test]
fn preview_skips_directions_and_tracks_missing_uses_across_restore_and_scenes() {
    let source = "scene harbor\n  location: dock\n  cast: mara, tam\n  shot: wide, dawn\n  mara: Hello\n  tam: Welcome\n  choice \"Where?\"\n    option \"Leave\": road\n    option \"Stay\": harbor\nscene road\n  prop: lantern\n  narrate: Onward\n  end\n";
    let plan = compile(source, &[]).plan.unwrap();
    assert!(Runtime::start_with_policy(plan.clone(), AssetPolicy::AllowMissing).is_err());
    assert!(Runtime::start(plan.clone()).is_err());
    let (mut runtime, _) = Runtime::start_with_policy(plan.clone(), AssetPolicy::Preview).unwrap();
    let names = |runtime: &Runtime| {
        runtime
            .active_uses()
            .iter()
            .map(|a| a.name.value.clone())
            .collect::<Vec<_>>()
    };
    assert_eq!(names(&runtime), ["dock", "mara", "tam"]);
    runtime.dispatch(Command::Advance).unwrap();
    assert_eq!(names(&runtime), ["dock", "mara", "tam"]);
    let saved = runtime.save();
    let restored = Runtime::restore_with_policy(plan, saved, AssetPolicy::Preview).unwrap();
    assert_eq!(names(&restored), names(&runtime));
    runtime.dispatch(Command::Advance).unwrap();
    runtime
        .dispatch(Command::SelectChoice { index: 0 })
        .unwrap();
    assert_eq!(names(&runtime), ["lantern"]);
    runtime.dispatch(Command::Restart).unwrap();
    assert_eq!(names(&runtime), ["dock", "mara", "tam"]);
}

#[test]
fn rejects_imported_plan_corruption_unready_plans_and_save_mismatches() {
    let p = plan(STORY);
    let (runtime, _) = Runtime::start(p.clone()).unwrap();
    let mut bad = p.clone();
    bad.version = 99;
    assert!(Runtime::start(bad)
        .unwrap_err()
        .to_string()
        .contains("version"));
    let mut bad = p.clone();
    bad.scenes[0].id.0 = "wrong".into();
    assert!(Runtime::start(bad).is_err());
    let mut bad = p.clone();
    bad.scenes[1].instructions[0].source = Span::new(usize::MAX, 0);
    assert!(Runtime::start(bad).is_err());
    let mut bad = p.clone();
    bad.scenes[0].instructions[0].kind = InstructionKind::Jump {
        target: SceneId("missing".into()),
        target_source: Span::new(0, 0),
    };
    assert!(Runtime::start(bad).is_err());
    let mut bad = p.clone();
    if let InstructionKind::Speech { speaker, .. } = &mut bad.scenes[1].instructions[2].kind {
        speaker.binding.as_mut().unwrap().revision = "different".into();
    } else {
        panic!("speech fixture");
    }
    assert!(Runtime::start(bad).is_err());
    assert!(Runtime::start(plan("scene a\n  prop: missing\n"))
        .unwrap_err()
        .to_string()
        .contains("pending"));
    assert!(Runtime::start(plan("scene a\n  shot: wide, dawn\n"))
        .unwrap_err()
        .to_string()
        .contains("unsupported"));
    let mut saved = runtime.save();
    saved.checkpoint.instruction = usize::MAX;
    assert!(Runtime::restore(p.clone(), saved).is_err());
    let mut saved = runtime.save();
    saved.version = 2;
    assert!(Runtime::restore(p.clone(), saved).is_err());
    let different = plan(&STORY.replace("Hello 🌊", "Changed"));
    assert!(Runtime::restore(different, runtime.save()).is_err());
    let mut saved = runtime.save();
    saved.commands.push(Command::SelectChoice { index: 999 });
    assert!(Runtime::restore(p, saved).is_err());
    assert!(serde_json::from_str::<storyfella_runtime::SavedState>("{}").is_err());
}

#[test]
fn supported_visual_directions_persist_reset_and_restore() {
    use storyfella_compiler::VisualDirection;
    let source = "scene first\n  shot: close\n  mood: tense\n  style: monochrome\n  narrate: One\n  shot: wide\n  mood: neutral\n  style: normal\n  narrate: Again\n  goto second\nscene second\n  narrate: Two\n  end\n";
    let story = plan(source);
    let (mut runtime, _) = Runtime::start(story.clone()).unwrap();
    assert_eq!(runtime.presentation().shot, Some(VisualDirection::Close));
    assert_eq!(runtime.presentation().mood, Some(VisualDirection::Tense));
    assert_eq!(
        runtime.presentation().style,
        Some(VisualDirection::Monochrome)
    );
    let saved = runtime.save();
    runtime.dispatch(Command::Advance).unwrap();
    assert_eq!(runtime.presentation().shot, Some(VisualDirection::Wide));
    assert_eq!(runtime.presentation().mood, Some(VisualDirection::Neutral));
    assert_eq!(runtime.presentation().style, Some(VisualDirection::Normal));
    runtime.dispatch(Command::Advance).unwrap();
    assert_eq!(runtime.presentation(), Default::default());
    let restored = Runtime::restore(story, saved).unwrap();
    assert_eq!(restored.presentation().shot, Some(VisualDirection::Close));
    assert_eq!(restored.presentation().mood, Some(VisualDirection::Tense));
    assert_eq!(
        restored.presentation().style,
        Some(VisualDirection::Monochrome)
    );
    runtime.dispatch(Command::Restart).unwrap();
    assert_eq!(runtime.presentation().shot, Some(VisualDirection::Close));
}

#[test]
fn staged_instances_swap_without_moving_and_restore_across_scenes() {
    use storyfella_language::StageGroup;
    let source = "scene first\n  stage: lead, mara, left, 125, 4\n  stage: friend, mara, right, 85, 2\n  narrate: One\n  swap: lead, smile\n  hide: friend\n  narrate: Two\n  goto second\nscene second\n  narrate: Three\n  end\n";
    let bindings = ["mara", "smile"].map(|name| AssetBinding {
        id: name.into(),
        name: name.into(),
        kind: AssetKind::Character,
        revision: "v1".into(),
    });
    let story = compile(source, &bindings).plan.unwrap();
    let (mut runtime, _) = Runtime::start(story.clone()).unwrap();
    assert_eq!(runtime.staged().len(), 2);
    let saved = runtime.save();
    runtime.dispatch(Command::Advance).unwrap();
    assert_eq!(runtime.staged().len(), 1);
    let lead = &runtime.staged()[0];
    assert_eq!(lead.asset.name.value, "smile");
    assert_eq!(
        (lead.group, lead.scale, lead.depth),
        (StageGroup::Left, 125, 4)
    );
    let restored = Runtime::restore(story, saved).unwrap();
    assert_eq!(restored.staged().len(), 2);
    runtime.dispatch(Command::Advance).unwrap();
    assert!(runtime.staged().is_empty());
}

#[test]
fn hiding_last_character_preserves_explicit_staging_until_scene_change() {
    let source = "scene first\n  stage: lead, mara, center, 100, 0\n  hide: lead\n  mara: Hello\n  goto second\nscene second\n  narrate: Next\n";
    let binding = AssetBinding {
        id: "mara".into(),
        name: "mara".into(),
        kind: AssetKind::Character,
        revision: "v1".into(),
    };
    let plan = compile(source, &[binding]).plan.unwrap();
    let (mut runtime, _) = Runtime::start(plan.clone()).unwrap();
    assert!(runtime.has_staging());
    assert!(runtime.staged().is_empty());
    let restored = Runtime::restore(plan, runtime.save()).unwrap();
    assert!(restored.has_staging());
    runtime.dispatch(Command::Advance).unwrap();
    assert!(!runtime.has_staging());
}

#[test]
fn authored_positions_and_automatic_slots_remain_stable() {
    let source = "scene start\n  stage: first, mara, left, 100, 1, 24, 13\n  stage: second, mara, left, 100, 2\n  narrate: One\n  hide: first\n  stage: third, mara, left, 100, 3\n  swap: second, smile\n  narrate: Two\n";
    let bindings = ["mara", "smile"].map(|name| AssetBinding {
        id: name.into(),
        name: name.into(),
        kind: AssetKind::Character,
        revision: "v1".into(),
    });
    let plan = compile(source, &bindings).plan.unwrap();
    let (mut runtime, _) = Runtime::start(plan.clone()).unwrap();
    assert_eq!(runtime.staged()[0].position, Some((24, 13)));
    let second = runtime.staged()[1].slot;
    runtime.dispatch(Command::Advance).unwrap();
    let current = runtime
        .staged()
        .iter()
        .find(|actor| actor.instance == "second")
        .unwrap();
    assert_eq!(current.slot, second);
    assert_eq!(current.asset.name.value, "smile");
    let restored = Runtime::restore(plan, runtime.save()).unwrap();
    assert_eq!(restored.staged(), runtime.staged());
}
