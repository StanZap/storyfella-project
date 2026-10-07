use storyfella_compiler::{compile, AssetBinding, InstructionKind, SceneId, StoryPlan};
use storyfella_language::{AssetKind, Span};
use storyfella_runtime::{Command, Event, Phase, Runtime, TextKind, TRANSITION_BUDGET};

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
    assert!(Runtime::start(plan("scene a\n  shot: wide\n"))
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
