use crate::{
    controls::{
        close_panel, control_keys, focus_stage, load_state, save_state, ExtraPanel, PlaybackOptions,
    },
    icons::{Icon, Symbol},
    PlaybackModel,
};
use dioxus::prelude::*;
use storyfella_compiler::InstructionKind;
use storyfella_language::{AssetKind, Source, Span};
use storyfella_runtime::{Command, Phase};

#[component]
pub fn PlaybackPanel(
    mut model: Signal<PlaybackModel>,
    #[props(default)] on_open: Option<EventHandler<()>>,
    #[props(default)] on_apply: Option<EventHandler<()>>,
    #[props(default)] on_source: Option<EventHandler<Span>>,
    #[props(default)] draft_changed: bool,
) -> Element {
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
    let position = session.zip(span).and_then(|(session, span)| {
        Source::new(&session.runtime.plan().source).position(span.start)
    });
    let title = session
        .and_then(|s| s.runtime.plan().title.as_ref())
        .map(|t| t.value.as_str())
        .unwrap_or("Storyfella Player");
    let paused = session.is_some_and(|s| s.runtime.state().paused);
    let phase = session.map(|s| s.runtime.state().phase);
    let visuals = session.map(|s| s.active_visuals()).unwrap_or_default();
    let location = visuals
        .iter()
        .find(|v| v.binding.kind == AssetKind::Location);
    // With no location, keep the available artwork intact rather than treating
    // a portrait or prop as a landscape. Every active asset remains inspectable.
    let backdrop = location.copied().or_else(|| visuals.first().copied());
    let speaker = match current.map(|i| &i.kind) {
        Some(InstructionKind::Speech { speaker, .. }) => speaker.binding.as_ref(),
        _ => None,
    };
    let portrait = speaker
        .and_then(|binding| visuals.iter().find(|v| &v.binding == binding).copied())
        .filter(|v| backdrop.is_none_or(|b| b.binding != v.binding));
    rsx! {
        style { {include_str!("playback.css")} }
        div { class: "sf-play", tabindex: "0", aria_label: "Story playback", "data-owns-keys": "true",
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
                        button { class: "sf-text-button", onclick: move |_| on_source.call(span), "Edit source" }
                    }
                    button { class: "sf-icon-button sf-options-trigger", aria_label: "Playback options", aria_expanded: visible.is_some(),
                        title: "Playback options", onclick: move |_| {
                            if panel().is_some() { close_panel(panel); } else { panel.set(Some(ExtraPanel::Options)); }
                        }, Icon { symbol: Symbol::More }
                    }
                }
            }
            div { class: "sf-stage", tabindex: "0", aria_label: "Story stage",
                if let Some(backdrop) = backdrop {
                    img { class: if location.is_some() { "sf-backdrop" } else { "sf-backdrop sf-backdrop-contained" },
                        src: "{backdrop.image_url}", alt: "{backdrop.binding.name}" }
                } else {
                    div { class: "sf-missing-backdrop", aria_label: "Artwork unavailable", "Artwork unavailable" }
                }
                if let Some(session) = session {
                    div { class: "sf-scene-label",
                        "{session.runtime.plan().scenes[session.runtime.state().scene].id.0}"
                        if let Some(beat) = &session.runtime.state().beat { " · {beat.name}" }
                    }
                }
                section { class: if phase == Some(Phase::Choice) { "sf-reading sf-reading-choice" } else { "sf-reading" }, aria_label: "Current story",
                    if let Some(error) = &snapshot.error { p { class: "sf-error", role: "alert", "{error}" } }
                    if let Some(instruction) = current {
                        match phase {
                            Some(Phase::Complete) => rsx! { h2 { "Story complete" } },
                            Some(Phase::Budget) => rsx! {
                                h2 { "A moment to pause" }
                                p { class: "sf-secondary", "Automatic transitions reached their limit. Continue another batch, or choose a scene in Options." }
                            },
                            _ => match &instruction.kind {
                                InstructionKind::Speech { speaker, thought, text } => rsx! {
                                    div { class: "sf-dialogue",
                                        if let Some(portrait) = portrait {
                                            img { class: "sf-portrait", src: "{portrait.image_url}", alt: "{portrait.binding.name}" }
                                        } else {
                                            div { class: "sf-portrait sf-missing-portrait", aria_label: "Portrait unavailable", "?" }
                                        }
                                        div { class: "sf-dialogue-text",
                                            p { class: "sf-speaker", "{speaker.name.value}" if *thought { " · thought" } }
                                            p { class: if *thought { "sf-story-text sf-thought" } else { "sf-story-text" }, "{text.value}" }
                                        }
                                    }
                                },
                                InstructionKind::Narration { text } => rsx! { p { class: "sf-story-text", "{text.value}" } },
                                InstructionKind::Choice { prompt, options } => rsx! {
                                    h2 { class: "sf-choice-prompt", "{prompt.value}" }
                                    div { class: "sf-choices", onkeydown: control_keys,
                                        for (index, option) in options.iter().enumerate() {
                                            button { class: "sf-choice", disabled: paused, onclick: move |_| {
                                                model.write().dispatch(Command::SelectChoice { index });
                                                focus_stage();
                                            },
                                                span { "{option.label.value}" } Icon { symbol: Symbol::Arrow }
                                            }
                                        }
                                    }
                                },
                                _ => rsx! {},
                            },
                        }
                        div { class: "sf-story-actions", onkeydown: control_keys,
                            if paused { span { class: "sf-secondary", role: "status", "Paused" } }
                            if matches!(phase, Some(Phase::Text | Phase::Budget)) {
                                button { class: "sf-continue", disabled: paused, onclick: move |_| model.write().dispatch(Command::Advance), "Continue" Icon { symbol: Symbol::Arrow } }
                            }
                            if phase == Some(Phase::Complete) {
                                button { class: "sf-continue", onclick: move |_| { model.write().dispatch(Command::Restart); focus_stage(); }, "Read again" Icon { symbol: Symbol::Arrow } }
                            } else {
                                button { class: "sf-icon-button", aria_label: if paused { "Resume" } else { "Pause" }, title: if paused { "Resume (Space)" } else { "Pause (Space)" },
                                    onclick: move |_| model.write().dispatch(if paused { Command::Resume } else { Command::Pause }),
                                    Icon { symbol: if paused { Symbol::Play } else { Symbol::Pause } }
                                }
                            }
                        }
                    } else {
                        h2 { if on_apply.is_some() { "See your story" } else { "A story awaits" } }
                        if snapshot.error.is_none() {
                            p { class: "sf-secondary", if on_apply.is_some() { "Apply your draft to start a preview." } else { "Open a Storyfella project to begin." } }
                        }
                        if let Some(on_open) = on_open {
                            button { class: "sf-continue", onclick: move |_| on_open.call(()), "Open story" Icon { symbol: Symbol::Arrow } }
                        }
                    }
                }
            }
            if let Some(visible) = visible {
                div { class: "sf-dismiss", aria_hidden: "true", onclick: move |_| close_panel(panel) }
                aside { class: if visible == ExtraPanel::Options { "sf-playback-panel" } else { "sf-playback-panel sf-panel-wide" }, "data-playback-panel": "true",
                    aria_label: match visible { ExtraPanel::Options => "Playback options", ExtraPanel::Artwork => "Artwork", ExtraPanel::Source => "Source snapshot" },
                    onkeydown: move |event| { event.stop_propagation(); if event.key() == Key::Escape { event.prevent_default(); close_panel(panel); } },
                    header { class: "sf-panel-header",
                        if visible != ExtraPanel::Options { button { class: "sf-text-button", onclick: move |_| panel.set(Some(ExtraPanel::Options)), "Back" } }
                        strong { match visible { ExtraPanel::Options => "Options", ExtraPanel::Artwork => "Artwork", ExtraPanel::Source => "Source snapshot" } }
                        button { class: "sf-icon-button sf-panel-close", aria_label: "Close playback options", onclick: move |_| close_panel(panel), Icon { symbol: Symbol::Close } }
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
                                        figcaption { "{visual.binding.name}" span { match visual.binding.kind { AssetKind::Character => "Character", AssetKind::Location => "Location", AssetKind::Prop => "Prop" } } }
                                    }
                                }
                            },
                            ExtraPanel::Source => rsx! {
                                if let Some(session) = session {
                                    p { class: "sf-secondary", "Source snapshot"
                                        if let Some(position) = position { " · line {position.line + 1}:{position.character + 1}" }
                                    }
                                    pre {
                                        for (line, text) in session.runtime.plan().source.lines().enumerate() {
                                            span { class: if position.is_some_and(|p| p.line as usize == line) { "sf-executing" } else { "" }, "{line + 1}: {text}\n" }
                                        }
                                    }
                                }
                            },
                        }
                    }
                }
            }
        }
    }
}
