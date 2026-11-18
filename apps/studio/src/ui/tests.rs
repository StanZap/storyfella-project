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
