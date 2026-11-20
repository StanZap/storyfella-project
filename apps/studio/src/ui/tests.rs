use std::{path::Path, time::Duration};

use gpui::{px, size, Entity, TestAppContext, VisualTestContext};
use storyfella_studio_core::config::AppConfig;

use super::{init, Studio};
use crate::{driver::PlanLaunch, host::Place};

struct Trial {
    studio: Entity<Studio>,
    cx: VisualTestContext,
    _directory: tempfile::TempDir,
}

fn config(directory: &Path) -> AppConfig {
    let file = directory.join("studio.toml");
    std::fs::write(&file, "").unwrap();
    let mut config = AppConfig::load_from(&file).unwrap();
    config.project_dir = directory.join("projects");
    config.asset_dir = directory.join("artwork");
    config.cache_dir = directory.join("cache");
    config
}

impl Trial {
    fn new(cx: &mut TestAppContext) -> Self {
        let directory = tempfile::tempdir().unwrap();
        let config = config(directory.path());
        cx.update(init);
        // The assistant answers on its own thread, so a trial waits for it.
        cx.executor().allow_parking();
        let window = cx.open_window(size(px(1280.), px(800.)), move |window, cx| {
            Studio::new(config, PlanLaunch::default(), window, cx)
        });
        let studio = window.root(cx).unwrap();
        let mut trial = Self {
            studio,
            cx: VisualTestContext::from_window(*window, cx),
            _directory: directory,
        };
        let focus = trial.studio.clone();
        trial.cx.update(|window, cx| {
            let handle = focus.read(cx).focus.clone();
            window.focus(&handle, cx);
        });
        trial.settle();
        trial
    }

    /// Runs what is pending, past the compile check's short debounce.
    fn settle(&mut self) {
        self.cx.run_until_parked();
        let started = std::time::Instant::now();
        while self.read(|studio| studio.assistant.is_running()) {
            assert!(
                started.elapsed() < Duration::from_secs(10),
                "the assistant never answered"
            );
            std::thread::sleep(Duration::from_millis(5));
            self.cx.run_until_parked();
        }
        self.cx.executor().advance_clock(Duration::from_millis(400));
        self.cx.run_until_parked();
        self.cx.update(|window, cx| window.draw(cx).clear(cx));
    }

    fn read<R>(&mut self, read: impl FnOnce(&Studio) -> R) -> R {
        self.cx.update(|_, cx| read(self.studio.read(cx)))
    }

    fn act<R>(
        &mut self,
        act: impl FnOnce(&mut Studio, &mut gpui::Window, &mut gpui::Context<Studio>) -> R,
    ) -> R {
        let studio = self.studio.clone();
        let result = self
            .cx
            .update(|window, cx| studio.update(cx, |studio, cx| act(studio, window, cx)));
        self.settle();
        result
    }

    fn keys(&mut self, keys: &str) {
        let primary = if cfg!(target_os = "macos") {
            "cmd"
        } else {
            "ctrl"
        };
        self.cx.simulate_keystrokes(&format!("{primary}-{keys}"));
        self.settle();
    }
}

#[gpui::test]
fn a_new_story_opens_in_write_and_is_listed_on_home(cx: &mut TestAppContext) {
    let mut trial = Trial::new(cx);
    assert_eq!(trial.read(Studio::shown), Place::Home);
    // A story view needs an open story; until then Home stays.
    trial.keys("1");
    assert_eq!(trial.read(Studio::shown), Place::Home);

    trial
        .act(|studio, _, cx| studio.create_project("Harbor", cx))
        .unwrap();
    assert_eq!(trial.read(Studio::shown), Place::Write);
    assert_eq!(
        trial.read(|studio| studio.state.project_name.clone()),
        "Harbor"
    );
    assert!(
        trial.read(|studio| studio.problems.is_some()),
        "the new story is checked"
    );

    // An empty story has nothing to play, and Play says why.
    trial.keys("2");
    assert_eq!(trial.read(Studio::shown), Place::Play);
    let model = |trial: &mut Trial| {
        trial
            .cx
            .update(|_, cx| trial.studio.read(cx).player.read(cx).model().clone())
    };
    assert!(model(&mut trial).error.is_some());

    trial.act(|studio, _, cx| {
        studio
            .state
            .set_story_source("scene harbor\n  narrate: Dusk.\n  end\n".into());
        studio.changed(cx);
    });
    trial.keys("2");
    let played = model(&mut trial);
    assert!(played.session.is_some(), "{:?}", played.error);
    assert!(
        !trial.read(|studio| studio.state.has_unsaved_changes),
        "autosave keeps up with each change"
    );

    trial.keys("shift-h");
    assert_eq!(trial.read(Studio::shown), Place::Home);
    let listed = trial.read(|studio| {
        studio
            .home
            .recents
            .iter()
            .map(|project| project.name.clone())
            .collect::<Vec<_>>()
    });
    assert_eq!(listed, ["Harbor"]);
}

#[gpui::test]
fn naming_nothing_is_refused_and_the_palette_follows_the_story(cx: &mut TestAppContext) {
    let mut trial = Trial::new(cx);
    let refused = trial.act(|studio, _, cx| studio.create_project("   ", cx));
    assert!(refused.is_err());
    assert!(trial.read(|studio| studio.state.project_path.is_none()));

    let disabled = |trial: &mut Trial, id: &str| {
        trial.read(|studio| {
            super::palette::commands(studio)
                .into_iter()
                .find(|command| command.id.as_ref() == id)
                .unwrap()
                .disabled
        })
    };
    assert!(disabled(&mut trial, "show-write"));
    trial.keys("k");
    assert!(trial.read(|studio| studio.palette_open));

    trial.act(|studio, window, cx| studio.set_palette(false, window, cx));
    trial
        .act(|studio, _, cx| studio.create_project("Lighthouse", cx))
        .unwrap();
    assert!(!disabled(&mut trial, "show-write"));
}

impl Trial {
    fn editor_text(&mut self) -> String {
        self.cx.update(|_, cx| {
            let editor = self.studio.read(cx).write.editor.clone();
            editor.read(cx).text(cx).to_string()
        })
    }
}

#[gpui::test]
fn typing_in_write_edits_the_story_and_outside_edits_reach_the_editor(cx: &mut TestAppContext) {
    let mut trial = Trial::new(cx);
    trial
        .act(|studio, _, cx| studio.create_project("Harbor", cx))
        .unwrap();
    trial.act(|studio, _, cx| {
        studio
            .state
            .set_story_source("scene harbor\n  narrate: Dusk.\n  end\n".into());
        studio.changed(cx);
    });
    // A change from outside the editor — a plan, the MCP bridge — shows up in it.
    assert_eq!(
        trial.editor_text(),
        "scene harbor\n  narrate: Dusk.\n  end\n"
    );

    // Typing goes through the editor into the story.
    trial.act(|studio, window, cx| {
        let editor = studio.write.editor.clone();
        editor.update(cx, |editor, cx| {
            editor.go_to("scene harbor\n  narrate: Dusk".len(), window, cx)
        });
    });
    trial.cx.simulate_input(" falls");
    trial.settle();
    assert_eq!(
        trial.read(|studio| studio.state.story.source().to_owned()),
        "scene harbor\n  narrate: Dusk falls.\n  end\n"
    );
    assert_eq!(trial.read(|studio| studio.caret.line), 1);
    assert_eq!(
        trial.read(|studio| studio.caret.character as usize),
        "  narrate: Dusk falls".len()
    );

    // Undo is the story's, and the editor follows it.
    trial.act(|studio, window, cx| studio.undo(&super::Undo, window, cx));
    assert_eq!(
        trial.editor_text(),
        "scene harbor\n  narrate: Dusk.\n  end\n"
    );
}

#[gpui::test]
fn write_shows_the_problems_of_the_open_file_and_renames_a_scene(cx: &mut TestAppContext) {
    let mut trial = Trial::new(cx);
    trial
        .act(|studio, _, cx| studio.create_project("Harbor", cx))
        .unwrap();
    trial.act(|studio, _, cx| {
        studio
            .state
            .set_story_source("scene harbor\n  goto pier\n".into());
        studio.changed(cx);
    });
    let errors = trial.read(|studio| {
        studio
            .write
            .check_for_tests()
            .map(|check| check.problems.errors)
    });
    assert!(errors.is_some_and(|errors| errors > 0), "{errors:?}");

    trial.act(|studio, _, cx| {
        studio
            .state
            .set_story_source("scene harbor\n  goto harbor\n".into());
        studio.changed(cx);
    });
    trial.act(|studio, window, cx| {
        let editor = studio.write.editor.clone();
        editor.update(cx, |editor, cx| {
            editor.go_to("scene harb".len(), window, cx)
        });
        studio.start_rename(window, cx);
    });
    assert!(trial.read(|studio| studio.write.renaming_for_tests()));
    trial.act(|studio, window, cx| studio.finish_rename("dock".into(), window, cx));
    assert_eq!(
        trial.read(|studio| studio.state.story.source().to_owned()),
        "scene dock\n  goto dock\n"
    );
    assert_eq!(trial.editor_text(), "scene dock\n  goto dock\n");
}

#[gpui::test]
fn a_story_that_cannot_open_is_a_row_whose_details_open_on_request(cx: &mut TestAppContext) {
    let mut trial = Trial::new(cx);
    let projects = trial.read(|studio| studio.config.project_dir.clone());
    let gone = projects.join("gone/story.db");
    std::fs::create_dir_all(&projects).unwrap();
    std::fs::write(
        projects.join("recent-projects.json"),
        serde_json::to_vec(&[&gone]).unwrap(),
    )
    .unwrap();
    trial.act(|studio, window, cx| studio.show(Place::Home, window, cx));
    let listed = trial.read(|studio| studio.home.recents.clone());
    assert_eq!(listed.len(), 1);
    assert!(listed[0]
        .trouble
        .as_ref()
        .is_some_and(|trouble| trouble.missing));
    assert!(
        trial.read(|studio| studio.home.trouble.is_none()),
        "details wait to be asked for"
    );

    // Activating the row, or its warning, opens the details.
    trial.act(|studio, _, cx| studio.show_trouble(listed[0].clone(), cx));
    assert!(trial.read(|studio| studio.home.trouble.is_some()));
    trial.act(|studio, _, cx| studio.forget(&gone, cx));
    assert!(trial.read(|studio| studio.home.trouble.is_none()));
    assert!(trial.read(|studio| studio.home.recents.is_empty()));
}

/// A model that calls one tool, as a stream.
#[cfg(test)]
fn tool_call(
    id: &str,
    name: &str,
    arguments: serde_json::Value,
) -> Vec<agentkit_ai::ChatStreamEvent> {
    vec![
        agentkit_ai::ChatStreamEvent::ToolCallDelta {
            index: 0,
            id: Some(id.into()),
            name: Some(name.into()),
            arguments: Some(arguments.to_string()),
        },
        agentkit_ai::ChatStreamEvent::Done,
    ]
}

/// A model's final words, as a stream.
#[cfg(test)]
fn says(text: &str) -> Vec<agentkit_ai::ChatStreamEvent> {
    vec![
        agentkit_ai::ChatStreamEvent::TextDelta {
            content: text.into(),
        },
        agentkit_ai::ChatStreamEvent::Done,
    ]
}

#[gpui::test]
fn the_assistant_edits_the_story_as_it_works_and_a_change_is_undone_in_review(
    cx: &mut TestAppContext,
) {
    use agentkit_ai::providers::fake::FakeChatModel;
    use design_system::kits::code::HunkState;
    use storyfella_studio_core::workflows::{
        assistant::{Done, FileChange, Outcome, Turn},
        RunStore,
    };

    let mut trial = Trial::new(cx);
    trial
        .act(|studio, _, cx| studio.create_project("Harbor", cx))
        .unwrap();
    let before = "scene harbor\n  beat arrival\n    narrate: Dusk.\n  beat waiting\n    narrate: Gulls.\n  beat departure\n    narrate: Bells.\n    end\n";
    trial.act(|studio, _, cx| {
        studio.state.set_story_source(before.into());
        studio.changed(cx);
    });
    trial.act(|studio, window, cx| {
        studio.assistant.model = Some(std::sync::Arc::new(FakeChatModel::new_streaming([
            tool_call(
                "call_dusk",
                "edit_file",
                serde_json::json!({"path": "story.sf", "old_text": "Dusk.", "new_text": "Night falls."}),
            ),
            tool_call(
                "call_bells",
                "edit_file",
                serde_json::json!({"path": "story.sf", "old_text": "Bells.", "new_text": "Silence."}),
            ),
            says("I made it night and quietened the departure."),
        ])));
        studio.assistant.prompt.update(cx, |prompt, cx| {
            prompt.set_text("Make it night", window, cx)
        });
        studio.ask(window, cx);
    });
    trial.settle();

    let edited = before
        .replace("Dusk.", "Night falls.")
        .replace("Bells.", "Silence.");
    assert_eq!(
        trial.read(|studio| studio.state.story.source().to_owned()),
        edited,
        "the edits are in the story without a review"
    );
    assert_eq!(trial.editor_text(), edited);
    let outcome = trial.read(|studio| match studio.assistant.current().turns.as_slice() {
        [Turn::Asked(asked), Turn::Answered(answer)] => {
            assert_eq!(asked.scope, "Project · 1 file");
            answer.outcome.clone()
        }
        turns => panic!("{turns:?}"),
    });
    assert_eq!(
        outcome,
        Outcome::Done(Done {
            reply: "I made it night and quietened the departure.".into(),
            files: vec![FileChange {
                file: "story.sf".into(),
                added: 2,
                removed: 2,
            }],
            out_of_steps: false,
            continuation: None,
        })
    );

    // Each change is decided on its own: the first kept, the second undone.
    trial.act(|studio, window, cx| studio.start_review(window, cx));
    trial.act(|studio, _, cx| {
        let review = studio.assistant.changes.review.as_ref().unwrap();
        let (file, diff) = review.shown();
        assert_eq!(file, "story.sf");
        diff.update(cx, |diff, cx| {
            assert_eq!(diff.states().len(), 2);
            diff.set_states(&[HunkState::Accepted, HunkState::Rejected], cx)
        });
    });
    trial
        .act(|studio, _, cx| studio.finish_review(None, cx))
        .unwrap();
    assert_eq!(
        trial.read(|studio| studio.state.story.source().to_owned()),
        before.replace("Dusk.", "Night falls.")
    );
    assert!(trial.read(|studio| studio.assistant.changes.review.is_none()));
    assert!(
        trial.read(|studio| studio.assistant.changes.differing.is_empty()),
        "nothing waits for a decision once the review ends"
    );

    // The conversation is the project's, and outlives the window.
    let project = trial.read(|studio| studio.state.project_path.clone().unwrap());
    let saved = RunStore::open(&project.with_extension("workflows.sqlite3"))
        .unwrap()
        .conversations(&project)
        .unwrap();
    assert_eq!(
        saved,
        [trial.read(|studio| studio.assistant.current().clone())]
    );
}

#[gpui::test]
fn a_reply_lands_in_its_own_conversation_while_another_is_shown(cx: &mut TestAppContext) {
    use agentkit_ai::providers::fake::FakeChatModel;
    use storyfella_studio_core::workflows::{
        assistant::{Outcome, Turn, UNTITLED},
        RunStore,
    };

    let mut trial = Trial::new(cx);
    trial
        .act(|studio, _, cx| studio.create_project("Harbor", cx))
        .unwrap();
    trial.act(|studio, window, cx| {
        studio
            .state
            .set_story_source("scene harbor\n  narrate: Dusk.\n  end\n".into());
        studio.changed(cx);
        studio.assistant.model = Some(std::sync::Arc::new(FakeChatModel::new_streaming([
            says("The harbor is at dusk."),
            says("Ivo waits on the pier."),
        ])));
        studio.assistant.prompt.update(cx, |prompt, cx| {
            prompt.set_text("Where are we?", window, cx)
        });
        studio.ask(window, cx);
        // A new conversation is shown while the first one's reply runs.
        studio.new_conversation(window, cx);
    });
    trial.settle();
    let (first, shown) = trial.read(|studio| {
        let listed = studio.assistant.conversations();
        assert_eq!(listed.len(), 2);
        (listed[1].clone(), studio.assistant.current().clone())
    });
    assert_eq!(first.title, "Where are we?");
    assert!(matches!(
        &first.turns[..],
        [Turn::Asked(_), Turn::Answered(answer)]
            if matches!(&answer.outcome, Outcome::Done(done) if done.reply == "The harbor is at dusk.")
    ));
    assert_eq!(shown.title, UNTITLED);
    assert!(shown.turns.is_empty());

    trial.act(|studio, window, cx| {
        studio.assistant.prompt.update(cx, |prompt, cx| {
            prompt.set_text("Who is there?", window, cx)
        });
        studio.ask(window, cx);
    });
    trial.settle();
    let project = trial.read(|studio| studio.state.project_path.clone().unwrap());
    let saved = || {
        RunStore::open(&project.with_extension("workflows.sqlite3"))
            .unwrap()
            .conversations(&project)
            .unwrap()
    };
    assert_eq!(
        saved().iter().map(|c| c.title.as_str()).collect::<Vec<_>>(),
        ["Who is there?", "Where are we?"],
        "both are saved, newest first"
    );

    // Choosing the first shows it; deleting it shows the other and forgets
    // it for good.
    trial.act(|studio, _, cx| studio.select_conversation(first.id.clone(), cx));
    assert_eq!(
        trial.read(|studio| studio.assistant.current().id.clone()),
        first.id
    );
    trial.act(|studio, _, cx| studio.delete_conversation(&first.id, cx));
    assert_eq!(
        trial.read(|studio| studio.assistant.current().title.clone()),
        "Who is there?"
    );
    assert_eq!(saved().len(), 1);
}

#[gpui::test]
fn the_assistant_answers_a_question_in_prose_and_shows_each_step(cx: &mut TestAppContext) {
    use agentkit_ai::{providers::fake::FakeChatModel, ChatStreamEvent};
    use storyfella_studio_core::workflows::assistant::{Done, Outcome, Turn};

    let mut trial = Trial::new(cx);
    trial
        .act(|studio, _, cx| studio.create_project("Harbor", cx))
        .unwrap();
    let source = "scene harbor\n  narrate: Dusk.\n  end\n";
    trial.act(|studio, window, cx| {
        studio.state.set_story_source(source.into());
        studio.changed(cx);
        studio.assistant.model = Some(std::sync::Arc::new(FakeChatModel::new_streaming([vec![
            ChatStreamEvent::ReasoningDelta {
                content: "They are greeting me.".into(),
            },
            ChatStreamEvent::TextDelta {
                content: "Hello! Which scene should change?".into(),
            },
            ChatStreamEvent::Done,
        ]])));
        studio
            .assistant
            .prompt
            .update(cx, |prompt, cx| prompt.set_text("Hello", window, cx));
        studio.ask(window, cx);
    });
    trial.settle();

    let answer = trial.read(|studio| match studio.assistant.current().turns.as_slice() {
        [Turn::Asked(_), Turn::Answered(answer)] => answer.clone(),
        turns => panic!("{turns:?}"),
    });
    assert_eq!(
        answer.outcome,
        Outcome::Done(Done {
            reply: "Hello! Which scene should change?".into(),
            ..Done::default()
        })
    );
    assert!(answer.steps.iter().all(|step| step.millis.is_some()));
    let thinking = answer
        .steps
        .iter()
        .find(|step| step.label == "Thinking")
        .expect("a thinking step");
    assert_eq!(
        thinking.detail.as_deref(),
        Some("Wrote 33 characters after 21 of thinking")
    );
    assert_eq!(
        trial.read(|studio| studio.state.story.source().to_owned()),
        source,
        "a reply changes nothing"
    );
}

#[gpui::test]
fn a_file_the_assistant_created_waits_while_another_project_is_open_and_undo_all_removes_it(
    cx: &mut TestAppContext,
) {
    use agentkit_ai::providers::fake::FakeChatModel;
    use storyfella_studio_core::workflows::RunStore;

    let mut trial = Trial::new(cx);
    trial
        .act(|studio, _, cx| studio.create_project("Harbor", cx))
        .unwrap();
    let source = "scene harbor\n  narrate: Dusk.\n  goto harbor\n";
    trial.act(|studio, window, cx| {
        studio.state.set_story_source(source.into());
        studio.changed(cx);
        studio.assistant.model = Some(std::sync::Arc::new(FakeChatModel::new_streaming([
            tool_call(
                "call_coda",
                "write_file",
                serde_json::json!({"path": "coda.sf", "text": "scene coda\n  narrate: Bells.\n  end\n"}),
            ),
            says("I added a coda."),
        ])));
        studio
            .assistant
            .prompt
            .update(cx, |prompt, cx| prompt.set_text("Add a coda", window, cx));
        studio.ask(window, cx);
    });
    trial.settle();
    let entry = trial.read(|studio| studio.state.story.source().to_owned());
    assert!(entry.contains("include \"coda.sf\""), "{entry}");
    let project = trial.read(|studio| studio.state.project_path.clone().unwrap());
    let coda = trial.read(|studio| studio.state.story.path().unwrap().with_file_name("coda.sf"));
    let undecided = trial.read(|studio| studio.state.assistant_changes.clone());
    assert_eq!(undecided.get("coda.sf"), Some(&None));
    assert_eq!(undecided.get("story.sf"), Some(&Some(source.to_owned())));
    let store = RunStore::open(&project.with_extension("workflows.sqlite3")).unwrap();
    assert_eq!(store.undecided(&project).unwrap(), undecided);

    trial
        .act(|studio, _, _| crate::commands::save_project(&mut studio.state))
        .unwrap();
    assert!(coda.exists());

    // Opened again after another project, it still has the changes waiting.
    trial
        .act(|studio, _, cx| studio.create_project("Lighthouse", cx))
        .unwrap();
    assert!(trial.read(|studio| studio.assistant.changes.differing.is_empty()));
    trial
        .act(|studio, _, cx| {
            let opened = crate::commands::open_project(&project, &mut studio.state);
            studio.changed(cx);
            opened
        })
        .unwrap();
    assert_eq!(
        trial.read(|studio| studio.assistant.changes.differing.clone()),
        vec!["coda.sf".to_owned(), "story.sf".to_owned()]
    );

    trial.act(|studio, _, cx| studio.undo_all_changes(cx));
    assert_eq!(
        trial.read(|studio| studio.state.story.source().to_owned()),
        source
    );
    trial
        .act(|studio, _, _| crate::commands::save_project(&mut studio.state))
        .unwrap();
    assert!(
        !coda.exists(),
        "undoing a created file leaves nothing behind"
    );
    assert!(store.undecided(&project).unwrap().is_empty());
}

#[gpui::test]
fn the_assistant_renames_a_scene_in_every_file_that_names_it(cx: &mut TestAppContext) {
    use agentkit_ai::providers::fake::FakeChatModel;

    let mut trial = Trial::new(cx);
    trial
        .act(|studio, _, cx| studio.create_project("Harbor", cx))
        .unwrap();
    let entry = "include \"market.sf\"\nscene harbor\n  narrate: Dusk.\n  goto market\n";
    let market =
        "scene market\n  choice \"Back?\"\n    option \"Yes\": harbor\n    option \"No\": market\n";
    trial.act(|studio, _, cx| {
        let base = studio.state.workspace_snapshot().unwrap();
        studio
            .state
            .set_sources(
                &base,
                std::collections::BTreeMap::from([
                    ("story.sf".to_owned(), Some(entry.to_owned())),
                    ("market.sf".to_owned(), Some(market.to_owned())),
                ]),
            )
            .unwrap();
        studio.changed(cx);
    });
    assert!(trial.read(|studio| studio.state.workspace_snapshot().unwrap().files.len() == 2));
    trial.act(|studio, window, cx| {
        studio.assistant.model = Some(std::sync::Arc::new(FakeChatModel::new_streaming([
            tool_call(
                "call_rename",
                "rename_scene",
                serde_json::json!({"scene": "harbor", "to": "quay"}),
            ),
            says("I renamed harbor to quay."),
        ])));
        studio.assistant.prompt.update(cx, |prompt, cx| {
            prompt.set_text("Call the harbor the quay", window, cx)
        });
        studio.ask(window, cx);
    });
    trial.settle();
    let now = trial.read(|studio| studio.state.workspace_snapshot().unwrap());
    assert_eq!(
        now.files["story.sf"],
        entry.replace("scene harbor", "scene quay")
    );
    assert_eq!(now.files["market.sf"], market.replace(": harbor", ": quay"));
    assert_eq!(
        trial.read(|studio| studio.assistant.changes.differing.clone()),
        vec!["market.sf".to_owned(), "story.sf".to_owned()]
    );
    trial.act(|studio, _, cx| studio.undo_all_changes(cx));
    let undone = trial.read(|studio| studio.state.workspace_snapshot().unwrap());
    assert_eq!(undone.files["story.sf"], entry);
    assert_eq!(undone.files["market.sf"], market);
}

#[gpui::test]
fn a_request_out_of_steps_offers_to_continue_where_it_stopped(cx: &mut TestAppContext) {
    use agentkit_ai::providers::fake::FakeChatModel;
    use storyfella_studio_core::workflows::assistant::{Done, Outcome, Turn};

    let mut trial = Trial::new(cx);
    trial
        .act(|studio, _, cx| studio.create_project("Harbor", cx))
        .unwrap();
    let source = "scene harbor\n  narrate: Dusk.\n  end\n";
    trial.act(|studio, window, cx| {
        studio.state.set_story_source(source.into());
        studio.changed(cx);
        studio.config.assistant.max_steps = 4;
        studio.assistant.model = Some(std::sync::Arc::new(FakeChatModel::new_streaming([
            tool_call("call_1", "outline", serde_json::json!({})),
            tool_call(
                "call_2",
                "read_file",
                serde_json::json!({"path": "story.sf"}),
            ),
            tool_call(
                "call_3",
                "edit_file",
                serde_json::json!({"path": "story.sf", "old_text": "Dusk.", "new_text": "Night."}),
            ),
            says("I made it night; the ending is left."),
            says("I rewrote the ending too."),
        ])));
        studio.assistant.prompt.update(cx, |prompt, cx| {
            prompt.set_text("Make it night and end it", window, cx)
        });
        studio.ask(window, cx);
    });
    trial.settle();
    let done = |trial: &mut Trial, turn: usize| {
        trial.read(|studio| match &studio.assistant.current().turns[turn] {
            Turn::Answered(answer) => match &answer.outcome {
                Outcome::Done(done) => done.clone(),
                other => panic!("{other:?}"),
            },
            other => panic!("{other:?}"),
        })
    };
    let first: Done = done(&mut trial, 1);
    assert!(first.out_of_steps);
    let continuation = first.continuation.clone().expect("a way to carry on");
    assert_eq!(continuation.instructions, "Make it night and end it");
    assert_eq!(
        continuation.continuing.as_deref(),
        Some("I made it night; the ending is left.")
    );

    trial.act(|studio, _, cx| studio.continue_request(1, cx));
    trial.settle();
    let turns = trial.read(|studio| studio.assistant.current().turns.clone());
    assert_eq!(turns.len(), 4);
    assert!(matches!(&turns[2], Turn::Asked(asked) if asked.text == "Continue"));
    let second = done(&mut trial, 3);
    assert_eq!(second.reply, "I rewrote the ending too.");
    assert!(!second.out_of_steps);
}

#[gpui::test]
fn a_failure_is_indicated_and_explained_only_on_request(cx: &mut TestAppContext) {
    use storyfella_studio_core::errors::ErrorCode;

    let mut trial = Trial::new(cx);
    trial.act(|studio, _, cx| {
        studio.hint("Put the caret on a name.", cx);
        studio.fail(ErrorCode::AppStoryEdit, "disk full", cx);
    });
    assert!(
        trial.read(|studio| studio.hint.is_none()),
        "a failure replaces guidance"
    );
    let error = trial.read(|studio| studio.error.clone()).unwrap();
    assert!(trial.read(|studio| studio.details.is_none()));

    trial.act(|studio, _, cx| studio.show_details(error, cx));
    assert!(trial.read(|studio| studio.details.is_some()));
    trial.act(|studio, _, cx| studio.close_details(cx));
    assert!(trial.read(|studio| studio.details.is_none() && studio.error.is_none()));
}

#[gpui::test]
fn a_failed_reply_is_tried_again_with_the_same_words(cx: &mut TestAppContext) {
    use agentkit_ai::{providers::fake::FakeChatModel, ChatStreamEvent};
    use storyfella_studio_core::workflows::assistant::{Done, Outcome, Place, Turn};

    let mut trial = Trial::new(cx);
    trial
        .act(|studio, _, cx| studio.create_project("Harbor", cx))
        .unwrap();
    trial.act(|studio, window, cx| {
        studio
            .state
            .set_story_source("scene harbor\n  narrate: Dusk.\n  end\n".into());
        studio.changed(cx);
        // A model with nothing to say fails the request.
        studio.assistant.model = Some(std::sync::Arc::new(FakeChatModel::new_streaming([])));
        studio.assistant.scope = crate::assistant::Scope::File;
        studio
            .assistant
            .prompt
            .update(cx, |prompt, cx| prompt.set_text("Hello", window, cx));
        studio.ask(window, cx);
    });
    trial.settle();
    let turns = trial.read(|studio| studio.assistant.current().turns.clone());
    match turns.as_slice() {
        [Turn::Asked(asked), Turn::Answered(answer)] => {
            assert!(matches!(answer.outcome, Outcome::Failed(_)), "{answer:?}");
            assert_eq!(
                asked.place,
                Some(Place {
                    file: "story.sf".into(),
                    offset: 0
                }),
                "the request remembers where its passage began"
            );
        }
        turns => panic!("{turns:?}"),
    }

    trial.act(|studio, window, cx| {
        studio.assistant.model = Some(std::sync::Arc::new(FakeChatModel::new_streaming([vec![
            ChatStreamEvent::TextDelta {
                content: "Hello! Which scene should change?".into(),
            },
            ChatStreamEvent::Done,
        ]])));
        studio.retry(1, window, cx);
    });
    trial.settle();
    let turns = trial.read(|studio| studio.assistant.current().turns.clone());
    match turns.as_slice() {
        [_, _, Turn::Asked(asked), Turn::Answered(answer)] => {
            assert_eq!(asked.text, "Hello");
            assert_eq!(
                answer.outcome,
                Outcome::Done(Done {
                    reply: "Hello! Which scene should change?".into(),
                    ..Done::default()
                })
            );
        }
        turns => panic!("{turns:?}"),
    }
}

#[gpui::test]
fn play_follows_the_caret_beside_the_source_and_lights_the_line_on_stage(cx: &mut TestAppContext) {
    let mut trial = Trial::new(cx);
    trial
        .act(|studio, _, cx| studio.create_project("Harbor", cx))
        .unwrap();
    let source = "scene start\n  narrate: One\n  narrate: Two\n  goto second\n\
                  scene second\n  narrate: Three\n  end\n";
    trial.act(|studio, _, cx| {
        studio.state.set_story_source(source.into());
        studio.changed(cx);
    });
    trial.keys("2");
    assert_eq!(trial.read(Studio::shown), Place::Play);
    let stage = |trial: &mut Trial| {
        trial.cx.update(|_, cx| {
            let studio = trial.studio.read(cx);
            let stage = studio.on_stage(cx).unwrap();
            let text = studio.state.story.source();
            let lit = studio.write.editor.read(cx).spotlight();
            (
                text[stage.span.start..stage.span.end].to_owned(),
                lit.map(|lit| text[lit].to_owned()),
            )
        })
    };
    let (on_stage, lit) = stage(&mut trial);
    assert!(on_stage.contains("One"), "{on_stage}");
    assert_eq!(lit.as_deref(), Some(on_stage.as_str()));

    // Clicking a line, or moving to it, plays up to it.
    let go_to = |trial: &mut Trial, offset: usize| {
        trial.act(|studio, window, cx| {
            let editor = studio.write.editor.clone();
            editor.update(cx, |editor, cx| editor.go_to(offset, window, cx));
        })
    };
    go_to(&mut trial, source.find("Three").unwrap());
    let (on_stage, lit) = stage(&mut trial);
    assert!(on_stage.contains("Three"), "{on_stage}");
    assert_eq!(lit.as_deref(), Some(on_stage.as_str()));
    assert_eq!(
        trial
            .cx
            .update(|_, cx| trial.studio.read(cx).stage_place(cx)),
        Some(format!(
            "{}, line 6",
            trial.read(|s| s.write.file().unwrap().to_owned())
        ))
    );

    // An edit plays the story as it now reads, at the caret.
    go_to(&mut trial, source.find("Two").unwrap() + 3);
    trial.cx.simulate_input(" and a half");
    trial.settle();
    let (on_stage, lit) = stage(&mut trial);
    assert!(on_stage.contains("Two and a half"), "{on_stage}");
    assert_eq!(lit.as_deref(), Some(on_stage.as_str()));

    // Full window is the stage alone, with nothing lit in the hidden source.
    trial.keys("shift-enter");
    assert!(trial.read(|studio| studio.play.full));
    assert_eq!(stage(&mut trial).1, None);
    trial.keys("shift-enter");
    assert!(stage(&mut trial).1.is_some());

    // Write is for writing; the stage's light goes with Play.
    trial.keys("1");
    assert_eq!(stage(&mut trial).1, None);
}

#[gpui::test]
fn the_board_lists_assets_adds_one_and_leads_to_where_the_story_uses_it(cx: &mut TestAppContext) {
    let mut trial = Trial::new(cx);
    trial
        .act(|studio, _, cx| studio.create_project("Harbor", cx))
        .unwrap();
    let source =
        "scene harbor\n  location: dock\n  cast: mara\n  mara: \"You came back.\"\n  end\n";
    trial.act(|studio, _, cx| {
        studio.state.set_story_source(source.into());
        studio.changed(cx);
    });
    trial.keys("3");
    assert_eq!(trial.read(Studio::shown), Place::Board);
    let names = |trial: &mut Trial| -> Vec<String> {
        trial.read(|studio| {
            studio
                .board
                .cards_for_tests()
                .iter()
                .map(|card| card.name.clone())
                .collect()
        })
    };
    assert_eq!(names(&mut trial), ["mara", "dock"]);
    let mara = trial.read(|studio| studio.board.cards_for_tests()[0].id);
    assert_eq!(
        trial.read(|studio| studio.board.shown_for_tests()),
        Some(mara)
    );

    // A new asset joins the board and is shown.
    trial.act(|studio, window, cx| studio.start_compose(super::board::Compose::New, window, cx));
    trial.act(|studio, window, cx| {
        studio.board.compose.expect("the dialog is open");
        studio.board.set_name_for_tests("lantern", window, cx);
        studio.board_finish_for_tests(window, cx);
    });
    assert_eq!(names(&mut trial), ["lantern", "mara", "dock"]);
    let lantern = trial.read(|studio| {
        studio
            .state
            .registry
            .artifacts
            .iter()
            .find(|artifact| artifact.name == "lantern")
            .map(|artifact| artifact.id)
    });
    assert_eq!(trial.read(|studio| studio.board.shown_for_tests()), lantern);
    assert!(trial.read(|studio| studio.board.compose.is_none()));

    // Where the story names an asset leads back to Write, there.
    trial.act(|studio, window, cx| {
        let uses = crate::board::Reading::of(&studio.state).uses(&studio.state.registry, mara);
        assert_eq!(uses.len(), 2, "cast and speech");
        studio.open_file(&uses[1].file, Some(uses[1].offset), cx);
        studio.show(Place::Write, window, cx);
    });
    assert_eq!(trial.read(Studio::shown), Place::Write);
    assert_eq!(trial.read(|studio| studio.caret.line), 3);
}
