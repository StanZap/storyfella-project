use crate::{
    controls::{
        close_panel, control_keys, focus_stage, load_state, save_state, ExtraPanel, PlaybackOptions,
    },
    icons::{Icon, Symbol},
};
use dioxus::prelude::*;
use storyfella_language::{AssetKind, Source, Span};
use storyfella_runtime::{Command, Phase};
use storyfella_studio_core::playback::{
    stage::{Effects, Frame, Passage, Portrait, Viewer},
    PlaybackModel,
};

fn stage_class(effects: Effects) -> String {
    format!(
        "sf-stage{}{}{}",
        if effects.close_shot {
            " sf-shot-close"
        } else {
            ""
        },
        if effects.tense { " sf-mood-tense" } else { "" },
        if effects.monochrome {
            " sf-style-monochrome"
        } else {
            ""
        },
    )
}

#[component]
fn PlaybackNav(
    label: String,
    title: String,
    symbol: Symbol,
    #[props(default)] disabled: bool,
    onclick: EventHandler<MouseEvent>,
) -> Element {
    rsx! {
        span { class: "sf-action-tooltip", title: title.clone(),
            button { class: "sf-nav", aria_label: label, title, disabled, onclick, Icon { symbol } }
        }
    }
}

#[component]
pub fn PlaybackPanel(
    mut model: Signal<PlaybackModel>,
    #[props(default)] on_open: Option<EventHandler<()>>,
    #[props(default)] on_apply: Option<EventHandler<()>>,
    #[props(default)] on_source: Option<EventHandler<Span>>,
    #[props(default)] draft_changed: bool,
    #[props(default)] settings: storyfella_studio_core::config::PreviewSettings,
) -> Element {
    let mut autoplay = use_signal(|| false);
    let mut live_settings = use_signal(|| settings);
    use_effect(use_reactive((&settings,), move |(settings,)| {
        live_settings.set(settings)
    }));
    use_resource(move || async move {
        while autoplay() {
            let delay = live_settings().autoplay_seconds.clamp(1, 30) * 1000;
            let mut timer =
                document::eval(&format!("setTimeout(() => dioxus.send(true), {delay})"));
            if timer.recv::<bool>().await.is_err() {
                autoplay.set(false);
                break;
            }
            if autoplay() {
                let mut state = model.write();
                let playable = state.session.as_ref().is_some_and(|s| {
                    s.runtime.state().phase == Phase::Text && !s.runtime.state().paused
                });
                if playable {
                    state.dispatch(Command::Advance);
                }
                if !playable
                    || state.error.is_some()
                    || state.session.as_ref().is_none_or(|s| {
                        s.runtime.state().phase != Phase::Text || s.runtime.state().paused
                    })
                {
                    autoplay.set(false);
                }
            }
        }
    });
    let mut panel = use_signal(|| None::<ExtraPanel>);
    let visible = panel();
    use_effect(use_reactive((&visible,), move |(visible,)| {
        if visible.is_some() {
            let _ = document::eval("document.querySelector('.sf-panel-close')?.focus()");
        }
    }));
    let snapshot = model.read();
    let session = snapshot.session.as_ref();
    let current = session.map(|s| s.runtime.current());
    let span = current.map(|i| i.source);
    use_effect(use_reactive((&span,), move |_| {
        // A new passage starts at its beginning, even after a long reading.
        let _ = document::eval(
            "const stage = document.querySelector('.sf-stage'); if (stage) stage.scrollTop = 0;",
        );
    }));
    let source_snapshot = session.zip(span).map(|(session, span)| {
        let plan = session.runtime.plan();
        if let Some((map, path, original)) = plan.source_map.as_ref().and_then(|map| {
            map.original_span(span)
                .map(|(path, original)| (map, path, original))
        }) {
            (Some(path), map.workspace.files[path].as_str(), original)
        } else {
            (None, plan.source.as_str(), span)
        }
    });
    let position =
        source_snapshot.and_then(|(_, text, span)| Source::new(text).position(span.start));
    let title = session
        .and_then(|s| s.runtime.plan().title.as_ref())
        .map(|t| t.value.as_str())
        .unwrap_or("Storyfella Player");
    let paused = session.is_some_and(|s| s.runtime.state().paused);
    let phase = session.map(|s| s.runtime.state().phase);
    let viewer = Viewer {
        previewing_draft: on_apply.is_some(),
        settings,
    };
    let frame = session.map(|s| Frame::new(s, viewer));
    let stage_class = stage_class(frame.as_ref().map(|f| f.effects).unwrap_or_default());
    let visuals = session.map(|s| s.active_visuals()).unwrap_or_default();
    rsx! {
        style { {include_str!("playback.css")} }
        div { class: "sf-play", style: "--sf-text-scale: {settings.text_percent as f32 / 100.0};", tabindex: "0", aria_label: "Story playback", "data-owns-keys": "true",
            onkeydown: move |event| {
                if event.key() == Key::Escape && panel().is_some() {
                    event.prevent_default(); close_panel(panel); return;
                }
                if panel().is_some() { return; }
                let command = match event.key() {
                    Key::Character(key) if key == " " && phase.is_some_and(|p| p != Phase::Complete) => Some(if paused { Command::Resume } else { Command::Pause }),
                    Key::ArrowRight if matches!(phase, Some(Phase::Text | Phase::Budget)) && !paused => Some(Command::Advance),
                    _ => None,
                };
                if let Some(command) = command { event.prevent_default(); model.write().dispatch(command); }
            },
            header { class: "sf-header", onkeydown: control_keys,
                span { class: "sf-brand", if on_apply.is_some() { "Preview" } else { "Storyfella" } }
                if on_apply.is_none() { span { class: "sf-title", "{title}" } }
                div { class: "sf-header-actions",
                    if draft_changed { span { class: "sf-draft-changed", title: "Apply draft to preview your latest source.", "Draft changed" } }
                    if let Some(on_apply) = on_apply {
                        button { class: "sf-text-button", title: "Start at the beginning with current source and artwork. Replaces the preview and its bookmark only if compilation succeeds.", onclick: move |_| { panel.set(None); on_apply.call(()); }, if model.read().session.is_some() { "Restart with draft" } else { "Start preview" } }
                    }
                    if let (Some(on_source), Some(span)) = (on_source, span) {
                        button { class: "sf-text-button", title: "Edit source", onclick: move |_| on_source.call(span), "Edit source" }
                    }
                    button { class: "sf-icon-button sf-options-trigger", aria_label: "Playback options", title: "Playback options", aria_expanded: visible.is_some(),
                        onclick: move |_| {
                            if panel().is_some() { close_panel(panel); } else { panel.set(Some(ExtraPanel::Options)); }
                        }, Icon { symbol: Symbol::More }
                    }
                }
            }
            div { class: "{stage_class}", tabindex: "0", aria_label: "Story stage",
                if let Some(asset) = frame.as_ref().and_then(|f| f.missing_backdrop) {
                    div { class: "sf-placeholder-location", aria_label: if asset.kind == AssetKind::Location { format!("Location {}: missing artwork", asset.name.value) } else { format!("Illustration {}: missing artwork", asset.name.value) } }
                }
                if let Some(backdrop) = frame.as_ref().and_then(|f| f.backdrop) {
                    div { class: "sf-backdrop-frame",
                        img { class: if backdrop.contained { "sf-backdrop sf-backdrop-contained" } else { "sf-backdrop" },
                            src: "{backdrop.visual.image_url}", alt: "{backdrop.visual.binding.name}" }
                    }
                } else if frame.as_ref().map_or(settings.show_diagnostics, |f| f.backdrop_unavailable) {
                    div { class: "sf-missing-backdrop", aria_label: "Artwork unavailable", "Artwork unavailable" }
                }
                if let Some(frame) = &frame {
                    div { class: "sf-actors", aria_label: "Characters on stage",
                        for actor in frame.actors.iter() {
                            { let style = format!("left:{}%;bottom:{}%;height:{}%;z-index:{};", actor.left, actor.bottom, actor.height, actor.depth);
                              rsx! {
                                  if let Some(visual) = actor.visual {
                                      img { key: "{actor.instance}", class: "sf-actor", style: "{style}", src: "{visual.image_url}", alt: "{actor.name}" }
                                  } else {
                                      div { key: "{actor.instance}", class: "sf-actor sf-staged-missing", style: "{style}", "{actor.name} · missing artwork" }
                                  }
                              }
                            }
                        }
                    }
                    if !frame.placeholders.is_empty() {
                        div { class: "sf-placeholders", aria_label: "Missing artwork; positions are approximate",
                            for placeholder in frame.placeholders.iter() {
                                div { class: if placeholder.kind == AssetKind::Character { "sf-placeholder sf-placeholder-character" } else { "sf-placeholder sf-placeholder-prop" }, style: "left: {placeholder.left}%;",
                                    span { if placeholder.kind == AssetKind::Character { "Character" } else { "Prop" } }
                                    strong { "{placeholder.name}" }
                                }
                            }
                        }
                    }
                    div { class: "sf-scene-label",
                        "{frame.label.scene}"
                        if let Some(beat) = frame.label.beat { " · {beat}" }
                        if frame.label.missing_artwork { div { class: "sf-preview-note", "Missing artwork · positions are approximate" } }
                        if frame.label.ignored_directions { div { class: "sf-preview-note", "Unsupported visual directions are ignored in preview" } }
                    }
                }
                section { class: if phase == Some(Phase::Choice) { "sf-reading sf-reading-choice" } else { "sf-reading" }, aria_label: "Current story",
                    if snapshot.error.is_some() { div { class: "sf-error", role: "alert", "Playback problem" button { class: "sf-text-button", title: "Show playback error details", onclick: move |_| panel.set(Some(ExtraPanel::Error)), "Details" } } }
                    if let Some(frame) = &frame {
                        match frame.passage {
                            Passage::Complete => rsx! { h2 { "Story complete" } },
                            Passage::Budget => rsx! {
                                h2 { "A moment to pause" }
                                p { class: "sf-secondary", "Automatic transitions reached their limit. Continue another batch, or choose a scene in Options." }
                            },
                            Passage::Speech { speaker, thought, text, portrait } => rsx! {
                                div { class: "sf-dialogue",
                                    match portrait {
                                        Portrait::Image(visual) => rsx! { img { class: "sf-portrait", src: "{visual.image_url}", alt: "{visual.binding.name}" } },
                                        Portrait::Missing => rsx! { div { class: "sf-portrait sf-missing-portrait", aria_label: "Portrait unavailable", "?" } },
                                        Portrait::None => rsx! {},
                                    }
                                    div { class: "sf-dialogue-text",
                                        p { class: "sf-speaker", "{speaker}" if thought { " · thought" } }
                                        p { class: if thought { "sf-story-text sf-thought" } else { "sf-story-text" }, "{text}" }
                                    }
                                }
                            },
                            Passage::Narration(text) => rsx! { p { class: "sf-story-text", "{text}" } },
                            Passage::Choice { prompt, options } => rsx! {
                                h2 { class: "sf-choice-prompt", "{prompt}" }
                                div { class: "sf-choices", onkeydown: control_keys,
                                    for (index, option) in options.iter().enumerate() {
                                        button { class: "sf-choice", title: "{option.label.value}", disabled: paused, onclick: move |_| {
                                            model.write().dispatch(Command::SelectChoice { index });
                                            focus_stage();
                                        },
                                            span { "{option.label.value}" } Icon { symbol: Symbol::Arrow }
                                        }
                                    }
                                }
                            },
                            Passage::Silent => rsx! {},
                        }
                        div { class: "sf-story-actions", onkeydown: control_keys,
                            PlaybackNav { label: "Previous passage", title: "Previous passage", symbol: Symbol::Previous, disabled: !snapshot.can_back(), onclick: move |_| { autoplay.set(false); model.write().back(); } }
                            PlaybackNav { label: if autoplay() { "Pause automatic playback" } else { "Play automatically" }, title: if autoplay() { "Pause automatic playback".to_owned() } else { format!("Play automatically (every {} seconds)", settings.autoplay_seconds) }, symbol: if autoplay() { Symbol::Pause } else { Symbol::Play }, disabled: phase != Some(Phase::Text) || paused, onclick: move |_| autoplay.toggle() }
                            if matches!(phase, Some(Phase::Text | Phase::Budget)) {
                                PlaybackNav { label: "Next passage", title: "Next passage (Right arrow)", symbol: Symbol::Next, disabled: paused, onclick: move |_| model.write().dispatch(Command::Advance) }
                            }
                            if phase == Some(Phase::Complete) { button { class: "sf-continue", title: "Read again", onclick: move |_| { model.write().dispatch(Command::Restart); focus_stage(); }, "Read again" Icon { symbol: Symbol::Arrow } } }
                            if paused { PlaybackNav { label: "Resume playback", title: "Resume playback (Space)", symbol: Symbol::Play, onclick: move |_| model.write().dispatch(Command::Resume) } }
                        }
                    } else {
                        h2 { if on_apply.is_some() { "See your story" } else { "A story awaits" } }
                        if snapshot.error.is_none() {
                            p { class: "sf-secondary", if on_apply.is_some() { "Apply your draft to start a preview." } else { "Open a Storyfella project to begin." } }
                        }
                        if let Some(on_open) = on_open {
                            button { class: "sf-continue", title: "Open story", onclick: move |_| on_open.call(()), "Open story" Icon { symbol: Symbol::Arrow } }
                        }
                    }
                }
            }
            if let Some(visible) = visible {
                div { class: "sf-dismiss", aria_hidden: "true", onclick: move |_| close_panel(panel) }
                aside { class: if visible == ExtraPanel::Options { "sf-playback-panel" } else { "sf-playback-panel sf-panel-wide" }, "data-playback-panel": "true",
                    aria_label: match visible { ExtraPanel::Options => "Playback options", ExtraPanel::Artwork => "Artwork", ExtraPanel::Source => "Source snapshot", ExtraPanel::Error => "Playback error details" },
                    onkeydown: move |event| { event.stop_propagation(); if event.key() == Key::Escape { event.prevent_default(); close_panel(panel); } },
                    header { class: "sf-panel-header",
                        if visible != ExtraPanel::Options { button { class: "sf-text-button", title: "Back to playback options", onclick: move |_| panel.set(Some(ExtraPanel::Options)), "Back" } }
                        strong { match visible { ExtraPanel::Options => "Options", ExtraPanel::Artwork => "Artwork", ExtraPanel::Source => "Source snapshot", ExtraPanel::Error => "Playback error details" } }
                        button { class: "sf-icon-button sf-panel-close", aria_label: "Close panel", title: "Close panel (Escape)", onclick: move |_| close_panel(panel), Icon { symbol: Symbol::Close } }
                    }
                    div { class: "sf-panel-content",
                        match visible {
                            // Dialog tasks belong to the persistent stage, so
                            // closing/unmounting Options does not cancel them.
                            ExtraPanel::Options => rsx! { PlaybackOptions { model, panel, on_open,
                                on_save: move |_| save_state(model),
                                on_load: move |_| load_state(model),
                            } },
                            ExtraPanel::Artwork => rsx! {
                                if visuals.is_empty() { p { class: "sf-secondary", "This moment has no artwork." } }
                                for visual in visuals {
                                    figure { class: "sf-artwork",
                                        img { src: "{visual.image_url}", alt: "{visual.binding.name}" }
                                        figcaption { "{visual.binding.name}" span { match visual.binding.kind { AssetKind::Character => "Character", AssetKind::Location => "Location", AssetKind::Prop => "Prop", AssetKind::Illustration => "Illustration" } } }
                                    }
                                }
                            },
                            ExtraPanel::Source => rsx! {
                                if let Some((file, source, _)) = source_snapshot {
                                    p { class: "sf-secondary", "Source snapshot"
                                        if let Some(file) = file { " · {file}" }
                                        if let Some(position) = position { " · line {position.line + 1}:{position.character + 1}" }
                                    }
                                    pre {
                                        for (line, text) in source.lines().enumerate() {
                                            span { class: if position.is_some_and(|p| p.line as usize == line) { "sf-executing" } else { "" }, "{line + 1}: {text}\n" }
                                        }
                                    }
                                }
                            },
                            ExtraPanel::Error => rsx! { if let Some(error) = &snapshot.error { pre { "{error}" } } },
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stage_classes_name_each_effect() {
        let active = Effects {
            close_shot: true,
            tense: true,
            monochrome: true,
        };
        assert_eq!(
            stage_class(active),
            "sf-stage sf-shot-close sf-mood-tense sf-style-monochrome"
        );
        assert_eq!(stage_class(Effects::default()), "sf-stage");
    }
}
