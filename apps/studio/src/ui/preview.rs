use super::{AppScreen, WriterCaret};
use crate::state::AppState;
use dioxus::prelude::*;
use storyfella_playback_ui::{PlaybackModel, PlaybackPanel};
use storyfella_studio_core::playback::PlaybackSession;

pub fn start_preview(mut model: Signal<PlaybackModel>, app_state: Signal<AppState>) {
    let state = app_state.read();
    let result = PlaybackSession::compile(state.story.source(), &state.registry);
    drop(state);
    model.write().install(result);
}

#[component]
pub fn Preview(app_state: Signal<AppState>) -> Element {
    let model = use_context::<Signal<PlaybackModel>>();
    let mut screen = use_context::<Signal<AppScreen>>();
    let caret = use_context::<WriterCaret>();
    let changed = model
        .read()
        .session
        .as_ref()
        .is_some_and(|s| s.runtime.plan().source != app_state.read().story.source());
    rsx! {
        PlaybackPanel { model, draft_changed: changed,
            on_apply: move |_| start_preview(model, app_state),
            on_source: if changed { None } else { Some(EventHandler::new(move |span: storyfella_language::Span| {
                    if let Some(position) = storyfella_language::Source::new(app_state.read().story.source()).position(span.start) {
                        caret.navigate(position);
                        screen.set(AppScreen::Writer);
                    }
                })) }
        }
    }
}
