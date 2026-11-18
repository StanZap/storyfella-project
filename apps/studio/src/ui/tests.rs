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

#[gpui::test]
fn the_assistant_suggests_an_edit_that_is_kept_after_review(cx: &mut TestAppContext) {
    use agentkit_ai::{providers::fake::FakeChatModel, ChatStreamEvent};
    use storyfella_studio_core::workflows::{
        assistant::{Decision, Outcome, Turn},
        RunStore,
    };

    let mut trial = Trial::new(cx);
    trial
        .act(|studio, _, cx| studio.create_project("Harbor", cx))
        .unwrap();
    // An empty file has no passage to edit; the prompt stays and says so.
    trial.act(|studio, window, cx| {
        studio.assistant.model = Some(std::sync::Arc::new(FakeChatModel::new_streaming([])));
        studio
            .assistant
            .prompt
            .update(cx, |prompt, cx| prompt.set_text("Begin", window, cx));
        studio.ask(window, cx);
    });
    assert!(trial.read(|studio| studio.assistant.notice.is_some()));
    assert!(trial.read(|studio| studio.assistant.conversation().turns.is_empty()));

    let before = "scene harbor\n  narrate: Dusk.\n  end\n";
    let after = "scene harbor\n  narrate: Night falls.\n  end\n";
    trial.act(|studio, _, cx| {
        studio.state.set_story_source(before.into());
        studio.changed(cx);
    });
    trial.act(|studio, window, cx| {
        studio.assistant.model = Some(std::sync::Arc::new(FakeChatModel::new_streaming([vec![
            ChatStreamEvent::TextDelta {
                content: after.into(),
            },
            ChatStreamEvent::Done,
        ]])));
        studio.assistant.scope = crate::assistant::Scope::File;
        studio.assistant.prompt.update(cx, |prompt, cx| {
            prompt.set_text("Make it night", window, cx)
        });
        studio.ask(window, cx);
    });
    trial.settle();

    let proposed = |trial: &mut Trial| {
        trial.read(
            |studio| match studio.assistant.conversation().turns.as_slice() {
                [Turn::Asked(_), Turn::Answered(answer)] => match &answer.outcome {
                    Outcome::Proposed(proposed) => Some(proposed.decision),
                    other => panic!("{other:?}"),
                },
                turns => panic!("{turns:?}"),
            },
        )
    };
    assert_eq!(proposed(&mut trial), Some(Decision::Pending));
    assert_eq!(
        trial.read(|studio| studio.state.story.source().to_owned()),
        before,
        "nothing is written before the review"
    );

    trial.act(|studio, window, cx| studio.start_review(1, window, cx));
    assert!(trial.read(|studio| studio.assistant.review.is_some()));
    trial
        .act(|studio, _, cx| studio.finish_review(Some(true), cx))
        .unwrap();
    assert_eq!(
        trial.read(|studio| studio.state.story.source().to_owned()),
        after
    );
    assert_eq!(trial.editor_text(), after);
    assert_eq!(
        proposed(&mut trial),
        Some(Decision::Kept { kept: 1, of: 1 })
    );

    // The conversation is the project's, and outlives the window.
    let project = trial.read(|studio| studio.state.project_path.clone().unwrap());
    let saved = RunStore::open(&project.with_extension("workflows.sqlite3"))
        .unwrap()
        .conversation(&project)
        .unwrap();
    assert_eq!(
        saved,
        trial.read(|studio| studio.assistant.conversation().clone())
    );
}

#[gpui::test]
fn the_assistant_answers_a_question_in_prose_and_shows_each_phase(cx: &mut TestAppContext) {
    use agentkit_ai::{providers::fake::FakeChatModel, ChatStreamEvent};
    use storyfella_studio_core::workflows::assistant::{Outcome, Turn};

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
        studio.assistant.scope = crate::assistant::Scope::File;
        studio
            .assistant
            .prompt
            .update(cx, |prompt, cx| prompt.set_text("Hello", window, cx));
        studio.ask(window, cx);
    });
    trial.settle();

    let answer = trial.read(
        |studio| match studio.assistant.conversation().turns.as_slice() {
            [Turn::Asked(_), Turn::Answered(answer)] => answer.clone(),
            turns => panic!("{turns:?}"),
        },
    );
    assert_eq!(
        answer.outcome,
        Outcome::Replied("Hello! Which scene should change?".into())
    );
    assert!(answer.steps.iter().all(|step| step.millis.is_some()));
    let writing = answer
        .steps
        .iter()
        .find(|step| step.label == "Writing")
        .expect("a writing phase");
    assert_eq!(
        writing.detail.as_deref(),
        Some("Wrote 33 characters after 21 of thinking")
    );
    assert_eq!(
        trial.read(|studio| studio.state.story.source().to_owned()),
        source,
        "a reply changes nothing"
    );
}

#[gpui::test]
fn a_project_edit_is_reviewed_file_by_file_and_kept_as_one_undo(cx: &mut TestAppContext) {
    use agentkit_ai::{providers::fake::FakeChatModel, ChatStreamEvent};
    use storyfella_studio_core::workflows::assistant::{Decision, Outcome, Turn};

    let reply = |text: &str| {
        vec![
            ChatStreamEvent::TextDelta {
                content: text.into(),
            },
            ChatStreamEvent::Done,
        ]
    };
    let mut trial = Trial::new(cx);
    trial
        .act(|studio, _, cx| studio.create_project("Harbor", cx))
        .unwrap();
    let entry = "include \"chapter.sf\"\nscene start\n  narrate: Dawn.\n  goto harbor\n";
    let chapter = "scene harbor\n  narrate: Dusk.\n  end\n";
    trial.act(|studio, window, cx| {
        let folder = studio.state.story.path().unwrap().parent().unwrap().to_owned();
        std::fs::write(folder.join("chapter.sf"), chapter).unwrap();
        studio.state.set_story_source(entry.into());
        studio.changed(cx);
        studio.assistant.model = Some(std::sync::Arc::new(FakeChatModel::new_streaming([
            reply(r#"{"files":["chapter.sf","story.sf"],"note":"Night falls in both scenes."}"#),
            reply("```storyfella\nscene harbor\n  narrate: Night.\n  end\n```"),
            reply(
                "```storyfella\ninclude \"chapter.sf\"\nscene start\n  narrate: Midnight.\n  goto harbor\n```",
            ),
        ])));
        studio.assistant.scope = crate::assistant::Scope::Project;
        studio
            .assistant
            .prompt
            .update(cx, |prompt, cx| prompt.set_text("Make it night", window, cx));
        studio.ask(window, cx);
    });
    trial.settle();

    let (scope, proposed) =
        trial.read(
            |studio| match studio.assistant.conversation().turns.as_slice() {
                [Turn::Asked(asked), Turn::Answered(answer)] => match &answer.outcome {
                    Outcome::Proposed(proposed) => (asked.scope.clone(), proposed.clone()),
                    other => panic!("{other:?} after {:?}", answer.steps),
                },
                turns => panic!("{turns:?}"),
            },
        );
    assert_eq!(scope, "Project · 2 files");
    assert_eq!(
        proposed.suggestion.note(),
        Some("Night falls in both scenes.")
    );
    let files: Vec<_> = proposed
        .files
        .iter()
        .map(|change| change.file.as_str())
        .collect();
    assert_eq!(files, ["chapter.sf", "story.sf"]);

    trial.act(|studio, window, cx| studio.start_review(1, window, cx));
    assert_eq!(
        trial.read(|studio| crate::writing::active(&studio.state)),
        Some("chapter.sf".into()),
        "the review opens its first file"
    );
    // Deciding the first file's only change moves on to the next file.
    trial.act(|studio, _, cx| {
        let review = studio.assistant.review.as_ref().unwrap();
        let (_, diff) = review.shown();
        diff.update(cx, |diff, cx| {
            diff.set_states(&[design_system::kits::code::HunkState::Accepted], cx)
        });
    });
    trial.act(|studio, window, cx| studio.decided(window, cx));
    assert_eq!(
        trial.read(|studio| crate::writing::active(&studio.state)),
        Some("story.sf".into())
    );
    trial
        .act(|studio, _, cx| studio.finish_review(Some(true), cx))
        .unwrap();
    let decision = trial.read(|studio| match &studio.assistant.conversation().turns[1] {
        Turn::Answered(answer) => match &answer.outcome {
            Outcome::Proposed(proposed) => proposed.decision,
            other => panic!("{other:?}"),
        },
        other => panic!("{other:?}"),
    });
    assert_eq!(decision, Decision::Kept { kept: 2, of: 2 });
    let texts = |trial: &mut Trial| {
        trial.read(|studio| {
            (
                studio.state.story.source().to_owned(),
                crate::writing::source(&studio.state, "chapter.sf").map(str::to_owned),
            )
        })
    };
    let (story, kept) = texts(&mut trial);
    assert!(story.contains("Midnight"), "{story}");
    assert!(kept.as_deref().unwrap_or("").contains("Night."), "{kept:?}");

    // Both files return with one Undo.
    trial.act(|studio, _, _| assert!(studio.state.undo()));
    let (story, restored) = texts(&mut trial);
    assert_eq!(story, entry);
    assert_eq!(restored.as_deref(), Some(chapter));
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
    use storyfella_studio_core::workflows::assistant::{Outcome, Place, Turn};

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
    let turns = trial.read(|studio| studio.assistant.conversation().turns.clone());
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
    let turns = trial.read(|studio| studio.assistant.conversation().turns.clone());
    match turns.as_slice() {
        [_, _, Turn::Asked(asked), Turn::Answered(answer)] => {
            assert_eq!(asked.text, "Hello");
            assert_eq!(
                answer.outcome,
                Outcome::Replied("Hello! Which scene should change?".into())
            );
        }
        turns => panic!("{turns:?}"),
    }
}
