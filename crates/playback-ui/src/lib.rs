use dioxus::prelude::*;
use storyfella_compiler::{InstructionKind, SceneId};
use storyfella_language::{Source, Span};
use storyfella_runtime::{Command, Phase, SavedState};
use storyfella_studio_core::playback::PlaybackSession;

#[derive(Clone, Debug, Default)]
pub struct PlaybackModel {
    pub session: Option<PlaybackSession>,
    pub error: Option<String>,
    pub bookmark: Option<SavedState>,
}
impl PlaybackModel {
    pub fn install(&mut self, result: Result<PlaybackSession, String>) {
        match result {
            Ok(session) => {
                self.session = Some(session);
                self.error = None;
                self.bookmark = None;
            }
            Err(error) => {
                self.session = None;
                self.error = Some(error);
                self.bookmark = None;
            }
        }
    }
    pub fn dispatch(&mut self, command: Command) {
        self.error = self
            .session
            .as_mut()
            .and_then(|s| s.dispatch(command).err());
    }
    pub fn bookmark(&mut self) {
        self.bookmark = self.session.as_ref().map(|s| s.runtime.save());
    }
    pub fn restore(&mut self, saved: SavedState) {
        self.error = self.session.as_mut().and_then(|s| s.restore(saved).err());
    }
    pub fn restore_bookmark(&mut self) {
        if let Some(saved) = self.bookmark.clone() {
            self.restore(saved);
        }
    }
}

const CSS: &str = r#"
.sf-play {color:#e8e6f2;background:#13131a;padding:24px;height:100%;overflow:auto;font:15px system-ui;box-sizing:border-box}
.sf-play * {box-sizing:border-box}.sf-play header,.sf-play nav {display:flex;align-items:center;gap:10px;flex-wrap:wrap;margin-bottom:18px}
.sf-play h2 {font-size:22px;margin:0}.sf-play .muted {color:#a6a2b6;font-size:13px}
.sf-play button,.sf-play select {border:1px solid #454058;border-radius:8px;padding:9px 14px;background:#24212e;color:#ece7ff;font:inherit;cursor:pointer}
.sf-play button:focus-visible,.sf-play select:focus-visible {outline:2px solid #b09aff;outline-offset:3px}
.sf-play button:disabled {opacity:.4;cursor:default}.sf-play .primary {background:#6350b4}.sf-play .visuals {display:flex;gap:18px;flex-wrap:wrap}
.sf-play figure {margin:0;flex:1;min-width:160px;max-width:560px}.sf-play img {width:100%;height:280px;object-fit:contain;background:#1b1924;border-radius:12px}
.sf-play figcaption {color:#a6a2b6;font-size:12px;padding:8px 0}.sf-play article {margin:20px 0;padding:24px;background:#1e1b29;border-radius:14px;max-width:900px}
.sf-play .story-text {font-size:22px;white-space:pre-wrap;line-height:1.6}.sf-play .choices {display:grid;gap:10px;margin-top:18px}.sf-play .error {color:#ffb4bc;white-space:pre-wrap;padding:15px;border:1px solid #704554;border-radius:10px}
.sf-play pre {white-space:pre-wrap;font:13px ui-monospace;line-height:1.7}.sf-play .executing {background:#47385c;color:white;display:block}
"#;

fn control_keys(event: KeyboardEvent) {
    if event.key() == Key::Enter || event.key() == Key::Character(" ".into()) {
        event.stop_propagation();
    }
}

#[component]
pub fn PlaybackPanel(
    mut model: Signal<PlaybackModel>,
    #[props(default)] on_source: Option<EventHandler<Span>>,
) -> Element {
    let mut show_source = use_signal(|| false);
    let snapshot = model.read();
    let error = snapshot.error.clone();
    let Some(session) = snapshot.session.as_ref() else {
        return rsx! { style { "{CSS}" } div { class: "sf-play", h2 { "Playback" } if let Some(error) = error { p { class: "error", "{error}" } } else { p { "Open a ready story to begin." } } } };
    };
    let runtime = &session.runtime;
    let state = runtime.state();
    let instruction = runtime.current();
    let scene = &runtime.plan().scenes[state.scene].id.0;
    let beat = state.beat.as_ref().map(|b| b.name.as_str()).unwrap_or("—");
    let source_span = instruction.source;
    let position = Source::new(&runtime.plan().source)
        .position(source_span.start)
        .unwrap();
    let paused = state.paused;
    let phase = state.phase;
    let source_lines: Vec<_> = runtime.plan().source.lines().collect();
    rsx! {
        style { "{CSS}" }
        div { class: "sf-play", tabindex: "0", aria_label: "Story playback", "data-owns-keys": "true",
            onkeydown: move |event| {
                let command = match event.key() {
                    Key::Character(key) if key == " " => Some(if paused { Command::Resume } else { Command::Pause }),
                    Key::ArrowRight if matches!(phase, Phase::Text | Phase::Budget) && !paused => Some(Command::Advance),
                    _ => None,
                };
                if let Some(command) = command { event.prevent_default(); model.write().dispatch(command); }
            },
            header {
                h2 { "{scene}" }
                span { class: "muted", "Beat: {beat} · line {position.line + 1}:{position.character + 1}" }
                if paused { strong { "Paused" } }
            }
            if let Some(error) = error { p { class: "error", "{error}" } }
            div { class: "visuals",
                for visual in session.active_visuals() {
                    figure {
                        img { src: "{visual.image_url}", alt: "{visual.binding.name}" }
                        figcaption { title: "Pinned revision {visual.binding.revision}", "{visual.binding.name} · pinned artwork" }
                    }
                }
            }
            article {
                match phase {
                    Phase::Complete => rsx! { h2 { "Story complete" } },
                    Phase::Budget => rsx! { p { "Automatic transition limit reached. Advance another batch, jump to a scene, or end the story." } },
                    _ => match &instruction.kind {
                        InstructionKind::Speech { speaker, thought, text } => rsx! { p { class: "muted", "{speaker.name.value}" if *thought { " · thought" } } p { class: "story-text", "{text.value}" } },
                        InstructionKind::Narration { text } => rsx! { p { class: "muted", "Narration" } p { class: "story-text", "{text.value}" } },
                        InstructionKind::Choice { prompt, options } => rsx! {
                            p { class: "story-text", "{prompt.value}" }
                            div { class: "choices", onkeydown: control_keys, for (index, option) in options.iter().enumerate() {
                                button { disabled: paused, onclick: move |_| model.write().dispatch(Command::SelectChoice { index }), "{option.label.value}" }
                            } }
                        },
                        _ => rsx! {},
                    },
                }
            }
            nav { onkeydown: control_keys,
                button { class: "primary", disabled: paused || !matches!(phase, Phase::Text | Phase::Budget), onclick: move |_| model.write().dispatch(Command::Advance), "Advance →" }
                button { onclick: move |_| model.write().dispatch(if paused { Command::Resume } else { Command::Pause }), if paused { "Resume" } else { "Pause" } }
                button { onclick: move |_| model.write().dispatch(Command::Restart), "Restart" }
                button { onclick: move |_| model.write().dispatch(Command::End), "End" }
                select { aria_label: "Jump to scene", value: "", disabled: paused, onkeydown: move |event| event.stop_propagation(), onchange: move |event| { if !event.value().is_empty() { model.write().dispatch(Command::Jump { scene: SceneId(event.value()) }); } },
                    option { value: "", "Jump to scene…" }
                    for scene in &runtime.plan().scenes { option { value: "{scene.id.0}", "{scene.id.0}" } }
                }
                button { onclick: move |_| model.write().bookmark(), "Bookmark" }
                button { disabled: snapshot.bookmark.is_none(), onclick: move |_| model.write().restore_bookmark(), "Restore bookmark" }
            }
            nav { onkeydown: control_keys,
                button { onclick: move |_| {
                    let saved = model.read().session.as_ref().unwrap().runtime.save();
                    if let Some(path) = rfd::FileDialog::new().set_file_name("story-state.json").save_file() {
                        model.write().error = serde_json::to_vec_pretty(&saved).map_err(|e| e.to_string()).and_then(|bytes| std::fs::write(path, bytes).map_err(|e| e.to_string())).err();
                    }
                }, "Save state…" }
                button { onclick: move |_| {
                    if let Some(path) = rfd::FileDialog::new().add_filter("Saved state", &["json"]).pick_file() {
                        let result = std::fs::read(path).map_err(|e| e.to_string()).and_then(|bytes| serde_json::from_slice(&bytes).map_err(|e| e.to_string()));
                        match result { Ok(saved) => model.write().restore(saved), Err(error) => model.write().error = Some(error) }
                    }
                }, "Load state…" }
                button { onclick: move |_| show_source.toggle(), "Source snapshot" }
                if let Some(on_source) = on_source { button { onclick: move |_| on_source.call(source_span), "Edit executing line" } }
            }
            p { class: "muted", "Manual playback · Right arrow advances · Space pauses/resumes · Tab and Enter select controls" }
            if show_source() {
                pre { for (line, text) in source_lines.iter().enumerate() {
                    span { class: if line == position.line as usize { "executing" } else { "" }, "{line + 1}: {text}\n" }
                } }
            }
        }
    }
}
