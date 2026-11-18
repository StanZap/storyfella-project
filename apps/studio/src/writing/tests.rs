use storyfella_studio_core::story::StoryDocument;

use super::*;
use crate::status::Check;

/// A story whose entry includes two chapters, open in a fresh state.
fn chaptered() -> (tempfile::TempDir, AppState) {
    let directory = tempfile::tempdir().unwrap();
    let entry = directory.path().join("story.sf");
    std::fs::write(&entry, "include \"first.sf\"\ninclude \"last.sf\"\n").unwrap();
    std::fs::write(
        directory.path().join("first.sf"),
        "scene start\n  goto ending\n",
    )
    .unwrap();
    std::fs::write(directory.path().join("last.sf"), "scene ending\n  end\n").unwrap();
    let mut state = AppState::default();
    state.story = StoryDocument::open(&entry).unwrap();
    state.reconcile_story();
    (directory, state)
}

#[test]
fn the_entry_comes_first_and_included_chapters_follow() {
    let (_directory, state) = chaptered();
    let workspace = crate::preview::workspace(&state).unwrap();
    assert_eq!(
        files(&state, Some(&workspace)),
        ["story.sf", "first.sf", "last.sf"]
    );
    assert_eq!(active(&state).as_deref(), Some("story.sf"));
}

#[test]
fn editing_a_chapter_is_a_draft_until_saved_and_one_undo() {
    let (_directory, mut state) = chaptered();
    open(&mut state, "first.sf").unwrap();
    assert_eq!(active(&state).as_deref(), Some("first.sf"));
    set_source(&mut state, "first.sf", "scene start\n  goto start\n".into()).unwrap();
    assert_eq!(
        source(&state, "first.sf"),
        Some("scene start\n  goto start\n")
    );
    assert!(state.has_unsaved_changes);
    assert!(state.undo());
    assert_eq!(
        source(&state, "first.sf"),
        Some("scene start\n  goto ending\n")
    );
}

#[test]
fn a_jump_leads_to_the_scene_in_another_chapter() {
    let (_directory, mut state) = chaptered();
    open(&mut state, "first.sf").unwrap();
    let at = "scene start\n  goto en".len();
    let found = navigation(&state, "first.sf", at);
    assert_eq!(
        found.definition,
        Some(Place {
            file: "last.sf".into(),
            offset: "scene ".len()
        })
    );
    assert_eq!(found.rename.as_deref(), Some("ending"));
    assert!(found
        .references
        .iter()
        .any(|place| place.file == "first.sf"));
}

#[test]
fn an_include_line_leads_to_its_file() {
    let (_directory, state) = chaptered();
    let found = navigation(
        &state,
        "story.sf",
        "include \"first.sf\"\ninclude \"la".len(),
    );
    assert_eq!(
        found.definition,
        Some(Place {
            file: "last.sf".into(),
            offset: 0
        })
    );
}

#[test]
fn completion_offers_scenes_from_every_chapter_for_the_text_being_typed() {
    let (_directory, state) = chaptered();
    // The editor asks with its own newer text, before the edit reaches the state.
    let typed = "scene start\n  goto e\n";
    let offered = completions(&state, "first.sf", typed, typed.len() - 1);
    let labels: Vec<_> = offered.iter().map(|c| c.label.as_str()).collect();
    assert_eq!(labels, ["ending"]);
    assert_eq!(
        offered[0].replace,
        Span::new(typed.len() - 2, typed.len() - 1)
    );
}

#[test]
fn renaming_from_a_chapter_renames_its_declaration_elsewhere() {
    let (_directory, mut state) = chaptered();
    open(&mut state, "first.sf").unwrap();
    rename(
        &mut state,
        "first.sf",
        "scene start\n  goto ".len(),
        "finale",
    )
    .unwrap();
    assert_eq!(
        source(&state, "first.sf"),
        Some("scene start\n  goto finale\n")
    );
    assert_eq!(
        rename(&mut state, "first.sf", 0, "  "),
        Err("Name the symbol.".into())
    );
}

#[test]
fn formatting_evens_out_spacing_and_keeps_the_rest() {
    let (_directory, mut state) = chaptered();
    open(&mut state, "first.sf").unwrap();
    set_source(
        &mut state,
        "first.sf",
        "scene   start\n  goto    ending\n".into(),
    )
    .unwrap();
    format(&mut state, "first.sf").unwrap();
    assert_eq!(
        source(&state, "first.sf"),
        Some("scene start\n  goto ending\n")
    );
}

#[test]
fn a_problem_is_placed_in_the_chapter_that_holds_it() {
    let (_directory, mut state) = chaptered();
    open(&mut state, "first.sf").unwrap();
    set_source(
        &mut state,
        "first.sf",
        "scene start\n  goto nowhere\n".into(),
    )
    .unwrap();
    let workspace = crate::preview::workspace(&state).unwrap();
    let check = Check::of(&workspace, &state.registry).unwrap();
    assert!(check.problems.errors > 0);
    let found = check
        .diagnostics
        .iter()
        .find(|found| found.file == "first.sf")
        .expect("the unknown scene is in first.sf");
    let text = &check.files["first.sf"];
    assert_eq!(
        &text[found.diagnostic.span.start..found.diagnostic.span.end],
        "nowhere"
    );
}

#[test]
fn keywords_text_and_names_are_highlighted() {
    let (_directory, state) = chaptered();
    let text = "scene start\n  narrate: Dusk.\n  goto start\n";
    let found = highlights(&state, text);
    let kind_of = |word: &str| {
        let at = text.find(word).unwrap();
        found
            .iter()
            .find(|(span, _)| span.start == at)
            .map(|(_, kind)| *kind)
    };
    assert_eq!(kind_of("scene"), Some(Highlight::Keyword));
    assert_eq!(kind_of("start"), Some(Highlight::Symbol));
}

#[test]
fn a_run_of_typing_is_one_undo_step_and_a_pause_starts_another() {
    let (_directory, mut state) = chaptered();
    let start = std::time::Instant::now();
    let pause = std::time::Duration::from_millis(1500);
    let typed = |state: &mut AppState, text: &str, at| {
        type_into(state, "story.sf", text.into(), at).unwrap();
    };
    typed(&mut state, "a", start);
    typed(
        &mut state,
        "ab",
        start + std::time::Duration::from_millis(300),
    );
    typed(&mut state, "abc", start + pause);
    // Anything else between keystrokes ends the run too.
    format(&mut state, "story.sf").ok();
    state.set_story_source("abc!".into());
    typed(&mut state, "abc!d", start + pause);
    assert_eq!(source(&state, "story.sf"), Some("abc!d"));
    assert!(state.undo());
    assert_eq!(source(&state, "story.sf"), Some("abc!"));
    assert!(state.undo());
    assert_eq!(source(&state, "story.sf"), Some("abc"));
    assert!(state.undo());
    assert_eq!(source(&state, "story.sf"), Some("ab"));
    assert!(state.undo());
    assert_eq!(
        source(&state, "story.sf"),
        Some("include \"first.sf\"\ninclude \"last.sf\"\n")
    );
}
