//! The panel drives the shared playback model from keys, buttons and
//! autoplay, and draws whatever state that leaves it in.
use std::time::Duration;

use design_system::Theme;
use gpui::{px, size, Entity, Modifiers, TestAppContext, VisualTestContext};
use gpui_kit::Focusable;
use storyfella_playback_panel::{Advance, PlaybackPanel, TogglePause};
use storyfella_runtime::Phase;
use storyfella_studio_core::{
    config::PreviewSettings,
    playback::{fixture, stage::Viewer, PlaybackSession},
};

fn panel(
    cx: &mut TestAppContext,
) -> (
    Entity<PlaybackPanel>,
    &mut VisualTestContext,
    tempfile::TempDir,
) {
    let directory = tempfile::tempdir().unwrap();
    let (source, registry) = fixture::create(directory.path()).unwrap();
    cx.update(|cx| {
        gpui_kit::init(cx);
        storyfella_playback_panel::init(cx);
    });
    let viewer = Viewer {
        previewing_draft: false,
        settings: PreviewSettings::default(),
    };
    let (panel, cx) = cx.add_window_view(|window, cx| {
        let mut panel = PlaybackPanel::new(viewer, Theme::dark(), cx);
        panel.install(PlaybackSession::compile(&source, &registry), cx);
        window.focus(&panel.focus_handle(cx), cx);
        panel
    });
    cx.simulate_resize(size(px(1100.), px(700.)));
    draw(cx);
    (panel, cx, directory)
}

fn draw(cx: &mut VisualTestContext) {
    for _ in 0..2 {
        cx.update(|window, cx| window.draw(cx).clear(cx));
    }
    cx.run_until_parked();
}

fn instruction(panel: &Entity<PlaybackPanel>, cx: &mut VisualTestContext) -> (usize, usize, Phase) {
    panel.read_with(cx, |panel, _| {
        let state = panel.model().session.as_ref().unwrap().runtime.state();
        (state.scene, state.instruction, state.phase)
    })
}

fn click(cx: &mut VisualTestContext, selector: &'static str) {
    let bounds = cx
        .debug_bounds(selector)
        .unwrap_or_else(|| panic!("{selector} is drawn"));
    cx.simulate_click(bounds.center(), Modifiers::none());
    draw(cx);
}

#[gpui::test]
fn the_right_arrow_advances_until_a_choice(cx: &mut TestAppContext) {
    let (panel, cx, _directory) = panel(cx);
    let start = instruction(&panel, cx);
    cx.dispatch_action(Advance);
    assert_ne!(instruction(&panel, cx), start, "a passage advances");
    for _ in 0..5 {
        cx.dispatch_action(Advance);
    }
    assert_eq!(
        instruction(&panel, cx).2,
        Phase::Choice,
        "advancing stops at a choice"
    );
}

#[gpui::test]
fn space_pauses_and_a_paused_story_does_not_advance(cx: &mut TestAppContext) {
    let (panel, cx, _directory) = panel(cx);
    cx.dispatch_action(TogglePause);
    let paused = instruction(&panel, cx);
    cx.dispatch_action(Advance);
    assert_eq!(instruction(&panel, cx), paused);
    cx.dispatch_action(TogglePause);
    cx.dispatch_action(Advance);
    assert_ne!(instruction(&panel, cx), paused);
}

#[gpui::test]
fn choosing_an_option_follows_it_and_previous_returns(cx: &mut TestAppContext) {
    let (panel, cx, _directory) = panel(cx);
    for _ in 0..3 {
        cx.dispatch_action(Advance);
    }
    draw(cx);
    let choice = instruction(&panel, cx);
    assert_eq!(choice.2, Phase::Choice);
    click(cx, "choice-0");
    let lighthouse = instruction(&panel, cx);
    assert_ne!(
        lighthouse.0, choice.0,
        "the first option leads to the lighthouse"
    );
    click(cx, "previous");
    assert_eq!(instruction(&panel, cx), choice);
}

#[gpui::test]
fn autoplay_reads_on_and_stops_at_a_choice(cx: &mut TestAppContext) {
    let (panel, cx, _directory) = panel(cx);
    let start = instruction(&panel, cx);
    click(cx, "autoplay");
    let delay = Duration::from_secs(u64::from(PreviewSettings::default().autoplay_seconds));
    cx.executor().advance_clock(delay);
    cx.run_until_parked();
    assert_ne!(
        instruction(&panel, cx),
        start,
        "one passage after the delay"
    );
    for _ in 0..4 {
        cx.executor().advance_clock(delay);
        cx.run_until_parked();
    }
    let stopped = instruction(&panel, cx);
    assert_eq!(stopped.2, Phase::Choice);
    cx.executor().advance_clock(delay * 3);
    cx.run_until_parked();
    assert_eq!(instruction(&panel, cx), stopped, "autoplay has stopped");
}
