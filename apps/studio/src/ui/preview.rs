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
    let running = model.read().session.is_some();
    rsx! {
        div { class: "flex h-full min-h-0 flex-col",
            div { class: "flex items-center gap-3 border-b border-white/10 px-5 py-3 text-sm text-zinc-300",
                button { class: "rounded bg-violet-500/25 px-3 py-2 text-violet-200", onclick: move |_| {
                    start_preview(model, app_state);
                }, "Start / apply current draft" }
                if changed { span { "Source changed. Apply the current draft to start a new snapshot." } }
                else if running { span { "Preview uses a fixed source and artwork snapshot." } }
                else { span { "Check and start the current draft." } }
            }
            div { class: "min-h-0 flex-1",
                PlaybackPanel { model, on_source: if changed { None } else { Some(EventHandler::new(move |span: storyfella_language::Span| {
                    if let Some(position) = storyfella_language::Source::new(app_state.read().story.source()).position(span.start) {
                        caret.navigate(position);
                        screen.set(AppScreen::Writer);
                    }
                })) } }
            }
        }
    }
}
