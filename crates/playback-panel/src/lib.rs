//! The GPUI playback panel: a story stage with a reading card, playback
//! controls and an options menu, over the shared `PlaybackModel`.
//!
//! Call [`init`] once to bind the playback keys.
mod images;
pub mod stage;
mod still;

use std::{path::PathBuf, time::Duration};

use design_system::{ButtonKind, Glyph, Menu, MenuItem, Theme, TypeScale, Typography};
use gpui_kit::{
    actions, div, prelude::*, px, relative, rgb, rgba, AnyElement, App, AsyncApp, Context,
    EventEmitter, FocusHandle, Focusable, KeyBinding, PathPromptOptions, SharedString, Task,
    WeakEntity, Window,
};
use storyfella_compiler::SceneId;
use storyfella_language::{AssetKind, Source};
use storyfella_runtime::{Command, Phase, SavedState};
use storyfella_studio_core::playback::{
    stage::{Frame, Passage, Viewer},
    PlaybackModel, PlaybackSession,
};

pub use images::{StageImage, StageImages};
use stage::{TextScale, GROUND, INK, MUTED};
pub use still::Still;

actions!(playback, [Advance, TogglePause, CloseOverlay]);

const CONTEXT: &str = "Playback";

pub fn init(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("right", Advance, Some(CONTEXT)),
        KeyBinding::new("space", TogglePause, Some(CONTEXT)),
        KeyBinding::new("escape", CloseOverlay, Some(CONTEXT)),
    ]);
}

pub enum PlaybackEvent {
    /// The reader asked to open a different story.
    OpenRequested,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Overlay {
    Options,
    Artwork,
    Source,
    Error,
}

impl Overlay {
    fn title(self) -> &'static str {
        match self {
            Self::Options => "Options",
            Self::Artwork => "Artwork",
            Self::Source => "Source snapshot",
            Self::Error => "Playback error details",
        }
    }
}

pub struct PlaybackPanel {
    model: PlaybackModel,
    images: StageImages,
    viewer: Viewer,
    theme: Theme,
    focus: FocusHandle,
    overlay: Option<Overlay>,
    autoplay: Option<Task<()>>,
    /// Shows "Open story" actions; the host answers `OpenRequested`.
    can_open: bool,
}

impl EventEmitter<PlaybackEvent> for PlaybackPanel {}

impl Focusable for PlaybackPanel {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl PlaybackPanel {
    pub fn new(viewer: Viewer, theme: Theme, cx: &mut Context<Self>) -> Self {
        Self {
            model: PlaybackModel::default(),
            images: StageImages::default(),
            viewer,
            theme,
            focus: cx.focus_handle(),
            overlay: None,
            autoplay: None,
            can_open: false,
        }
    }

    pub fn can_open(mut self, can_open: bool) -> Self {
        self.can_open = can_open;
        self
    }

    pub fn model(&self) -> &PlaybackModel {
        &self.model
    }

    /// Runs `act` on the model, for a host that drives playback itself.
    /// Installing a new session goes through [`install`](Self::install),
    /// which decodes its artwork.
    pub fn update_model<R>(
        &mut self,
        act: impl FnOnce(&mut PlaybackModel) -> R,
        cx: &mut Context<Self>,
    ) -> R {
        let result = act(&mut self.model);
        cx.notify();
        result
    }

    /// Applies changed reader settings or a new theme.
    pub fn configure(&mut self, viewer: Viewer, theme: Theme, cx: &mut Context<Self>) {
        self.viewer = viewer;
        self.theme = theme;
        cx.notify();
    }

    /// Installs a freshly compiled session, or reports why it failed while
    /// keeping the session already playing.
    pub fn install(&mut self, result: Result<PlaybackSession, String>, cx: &mut Context<Self>) {
        let replaced = result.is_ok();
        self.model.install(result);
        if replaced {
            self.images = self
                .model
                .session
                .as_ref()
                .map(StageImages::decode)
                .unwrap_or_default();
            self.autoplay = None;
        }
        cx.notify();
    }

    pub fn report(&mut self, error: impl Into<String>, cx: &mut Context<Self>) {
        self.model.error = Some(error.into());
        cx.notify();
    }

    fn dispatch(&mut self, command: Command, cx: &mut Context<Self>) {
        self.model.dispatch(command);
        cx.notify();
    }

    fn phase(&self) -> Option<Phase> {
        self.model.session.as_ref().map(|s| s.runtime.state().phase)
    }

    fn paused(&self) -> bool {
        self.model
            .session
            .as_ref()
            .is_some_and(|s| s.runtime.state().paused)
    }

    fn advance(&mut self, _: &Advance, _: &mut Window, cx: &mut Context<Self>) {
        if self.overlay.is_none()
            && matches!(self.phase(), Some(Phase::Text | Phase::Budget))
            && !self.paused()
        {
            self.dispatch(Command::Advance, cx);
        }
    }

    fn toggle_pause(&mut self, _: &TogglePause, _: &mut Window, cx: &mut Context<Self>) {
        if self.overlay.is_none() && self.phase().is_some_and(|p| p != Phase::Complete) {
            let command = if self.paused() {
                Command::Resume
            } else {
                Command::Pause
            };
            self.dispatch(command, cx);
        }
    }

    fn close_overlay(&mut self, _: &CloseOverlay, window: &mut Window, cx: &mut Context<Self>) {
        self.set_overlay(None, window, cx);
    }

    fn set_overlay(
        &mut self,
        overlay: Option<Overlay>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.overlay = overlay;
        if overlay.is_none() {
            window.focus(&self.focus, cx);
        }
        cx.notify();
    }

    fn toggle_autoplay(&mut self, cx: &mut Context<Self>) {
        if self.autoplay.take().is_none() {
            let delay = Duration::from_secs(u64::from(
                self.viewer.settings.autoplay_seconds.clamp(1, 30),
            ));
            self.autoplay = Some(cx.spawn(
                async move |this: WeakEntity<Self>, cx: &mut AsyncApp| loop {
                    cx.background_executor().timer(delay).await;
                    let playing = this
                        .update(cx, |this, cx| this.autoplay_step(cx))
                        .unwrap_or(false);
                    if !playing {
                        break;
                    }
                },
            ));
        }
        cx.notify();
    }

    /// Advances one passage; autoplay stops at a choice, a pause, an error or
    /// the end.
    fn autoplay_step(&mut self, cx: &mut Context<Self>) -> bool {
        let readable = |model: &PlaybackModel| {
            model.session.as_ref().is_some_and(|s| {
                s.runtime.state().phase == Phase::Text && !s.runtime.state().paused
            })
        };
        if readable(&self.model) {
            self.model.dispatch(Command::Advance);
        }
        let playing = self.model.error.is_none() && readable(&self.model);
        if !playing {
            self.autoplay = None;
        }
        cx.notify();
        playing
    }

    fn save_state(&mut self, cx: &mut Context<Self>) {
        let Some(saved) = self.model.session.as_ref().map(|s| s.runtime.save()) else {
            return;
        };
        let directory = std::env::current_dir().unwrap_or_default();
        let path = cx.prompt_for_new_path(&directory, Some("story-state.json"));
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let Ok(Ok(Some(path))) = path.await else {
                return;
            };
            let result = serde_json::to_vec_pretty(&saved)
                .map_err(|e| e.to_string())
                .and_then(|bytes| std::fs::write(&path, bytes).map_err(|e| e.to_string()));
            this.update(cx, |this, cx| {
                this.model.error = result.err();
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn load_state(&mut self, cx: &mut Context<Self>) {
        let paths = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some("Load saved state".into()),
        });
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let Ok(Ok(Some(paths))) = paths.await else {
                return;
            };
            let Some(path): Option<PathBuf> = paths.into_iter().next() else {
                return;
            };
            let saved = std::fs::read(&path)
                .map_err(|e| e.to_string())
                .and_then(|bytes| {
                    serde_json::from_slice::<SavedState>(&bytes).map_err(|e| e.to_string())
                });
            this.update(cx, |this, cx| {
                match saved {
                    Ok(saved) => this.model.restore(saved),
                    Err(error) => this.model.error = Some(error),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn nav(
        &self,
        id: &'static str,
        glyph: Glyph,
        label: impl Into<SharedString>,
        disabled: bool,
        on_click: impl Fn(&mut Self, &mut Window, &mut Context<Self>) + 'static,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        self.theme
            .icon_action(id, glyph, label, ButtonKind::Quiet)
            .debug_selector(move || id.to_owned())
            .disabled(disabled)
            .on_click(cx.listener(move |this, _, window, cx| on_click(this, window, cx)))
            .into_any_element()
    }

    fn story_actions(&self, cx: &mut Context<Self>) -> AnyElement {
        let phase = self.phase();
        let paused = self.paused();
        let autoplaying = self.autoplay.is_some();
        let seconds = self.viewer.settings.autoplay_seconds;
        div()
            .mt(px(16.))
            .min_h(px(40.))
            .flex()
            .items_center()
            .justify_end()
            .gap(px(12.))
            .child(self.nav(
                "previous",
                Glyph::ChevronLeft,
                "Previous passage",
                !self.model.can_back(),
                |this, _, cx| {
                    this.autoplay = None;
                    this.model.back();
                    cx.notify();
                },
                cx,
            ))
            .child(self.nav(
                "autoplay",
                if autoplaying {
                    Glyph::Pause
                } else {
                    Glyph::Play
                },
                if autoplaying {
                    "Pause automatic playback".to_owned()
                } else {
                    format!("Play automatically (every {seconds} seconds)")
                },
                phase != Some(Phase::Text) || paused,
                |this, _, cx| this.toggle_autoplay(cx),
                cx,
            ))
            .when(matches!(phase, Some(Phase::Text | Phase::Budget)), |this| {
                this.child(self.nav(
                    "next",
                    Glyph::ChevronRight,
                    "Next passage (Right arrow)",
                    paused,
                    |this, _, cx| this.dispatch(Command::Advance, cx),
                    cx,
                ))
            })
            .when(phase == Some(Phase::Complete), |this| {
                this.child(
                    self.theme
                        .button("read-again", "Read again", ButtonKind::Primary)
                        .on_click(
                            cx.listener(|this, _, _, cx| this.dispatch(Command::Restart, cx)),
                        ),
                )
            })
            .when(paused, |this| {
                this.child(self.nav(
                    "resume",
                    Glyph::Play,
                    "Resume playback (Space)",
                    false,
                    |this, _, cx| this.dispatch(Command::Resume, cx),
                    cx,
                ))
            })
            .into_any_element()
    }

    fn choice(&self, index: usize, label: SharedString, cx: &mut Context<Self>) -> AnyElement {
        let scale = self.text_scale();
        stage::choice_row(
            gpui_kit::base::Button::new(("choice", index))
                .accessibility_label(label.clone())
                .debug_selector(move || format!("choice-{index}"))
                .disabled(self.paused())
                .hover(|style| style.bg(rgba(0x263038c9))),
            label,
            scale,
        )
        .on_click(cx.listener(move |this, _, window, cx| {
            this.dispatch(Command::SelectChoice { index }, cx);
            window.focus(&this.focus, cx);
        }))
        .into_any_element()
    }

    fn text_scale(&self) -> TextScale {
        TextScale(self.viewer.settings.text_percent as f32 / 100.)
    }

    fn reading(&self, frame: Option<&Frame>, cx: &mut Context<Self>) -> AnyElement {
        let scale = self.text_scale();
        let error =
            self.model.error.is_some().then(|| {
                div()
                    .mb(px(12.))
                    .px(px(16.))
                    .py(px(12.))
                    .flex()
                    .items_center()
                    .gap(px(12.))
                    .border_l_2()
                    .border_color(rgb(0xdb9590))
                    .bg(rgba(0x301a1ed9))
                    .text_color(rgb(0xffd2cc))
                    .child(div().flex_1().child("Playback problem"))
                    .child(self.theme.text_button("error-details", "Details").on_click(
                        cx.listener(|this, _, window, cx| {
                            this.set_overlay(Some(Overlay::Error), window, cx)
                        }),
                    ))
            });
        let card = stage::reading_card(matches!(
            frame.map(|f| f.passage),
            Some(Passage::Choice { .. })
        ))
        .children(error);
        match (frame, self.model.session.as_ref()) {
            (Some(frame), Some(session)) => {
                let mut choices = Vec::new();
                if let Passage::Choice { options, .. } = frame.passage {
                    for (index, option) in options.iter().enumerate() {
                        choices.push(self.choice(index, option.label.value.clone().into(), cx));
                    }
                }
                let mut choices = choices.into_iter().map(Some).collect::<Vec<_>>();
                card.children(stage::passage(
                    frame.passage,
                    session,
                    &self.images,
                    scale,
                    frame.effects.monochrome,
                    |index, _| choices[index].take().expect("each choice is drawn once"),
                ))
                .child(self.story_actions(cx))
                .into_any_element()
            }
            _ => {
                let previewing = self.viewer.previewing_draft;
                card.child(stage::heading(
                    if previewing {
                        "See your story"
                    } else {
                        "A story awaits"
                    },
                    scale,
                ))
                .when(self.model.error.is_none(), |this| {
                    this.child(stage::secondary(if previewing {
                        "Apply your draft to start a preview."
                    } else {
                        "Open a Storyfella project to begin."
                    }))
                })
                .when(self.can_open, |this| {
                    this.child(
                        div().mt(px(16.)).flex().justify_end().child(
                            self.theme
                                .button("open-story", "Open story", ButtonKind::Primary)
                                .on_click(
                                    cx.listener(|_, _, _, cx| {
                                        cx.emit(PlaybackEvent::OpenRequested)
                                    }),
                                ),
                        ),
                    )
                })
                .into_any_element()
            }
        }
    }

    fn options_menu(&self, cx: &mut Context<Self>) -> AnyElement {
        let this = cx.entity().downgrade();
        let act = move |f: fn(&mut Self, &mut Window, &mut Context<Self>)| {
            let this = this.clone();
            move |window: &mut Window, cx: &mut App| {
                this.update(cx, |this, cx| f(this, window, cx)).ok();
            }
        };
        let mut menu = Menu::new("playback-options", self.theme).width(px(260.));
        if self.can_open {
            menu = menu.item(
                MenuItem::new("Open story…").on_click(act(|this, window, cx| {
                    this.set_overlay(None, window, cx);
                    cx.emit(PlaybackEvent::OpenRequested);
                })),
            );
        }
        if let Some(session) = self.model.session.as_ref() {
            let paused = session.runtime.state().paused;
            menu = menu
                .item(
                    MenuItem::new("Restart story").on_click(act(|this, window, cx| {
                        this.dispatch(Command::Restart, cx);
                        this.set_overlay(None, window, cx);
                    })),
                )
                .item(MenuItem::new("End story").on_click(act(|this, window, cx| {
                    this.dispatch(Command::End, cx);
                    this.set_overlay(None, window, cx);
                })))
                .separator()
                .header("Jump to scene");
            for scene in &session.runtime.plan().scenes {
                let id = scene.id.0.clone();
                let target = cx.entity().downgrade();
                menu = menu.item(MenuItem::new(id.clone()).disabled(paused).on_click(
                    move |window, cx| {
                        let scene = SceneId(id.clone());
                        target
                            .update(cx, |this, cx| {
                                this.dispatch(Command::Jump { scene }, cx);
                                this.set_overlay(None, window, cx);
                            })
                            .ok();
                    },
                ));
            }
            menu = menu
                .separator()
                .item(MenuItem::new("Bookmark").on_click(act(|this, window, cx| {
                    this.model.bookmark();
                    this.set_overlay(None, window, cx);
                })))
                .item(
                    MenuItem::new("Restore bookmark")
                        .disabled(self.model.bookmark.is_none())
                        .on_click(act(|this, window, cx| {
                            this.model.restore_bookmark();
                            this.set_overlay(None, window, cx);
                        })),
                )
                .item(
                    MenuItem::new("Save state…").on_click(act(|this, window, cx| {
                        this.set_overlay(None, window, cx);
                        this.save_state(cx);
                    })),
                )
                .item(
                    MenuItem::new("Load state…").on_click(act(|this, window, cx| {
                        this.set_overlay(None, window, cx);
                        this.load_state(cx);
                    })),
                )
                .separator()
                .item(MenuItem::new("Artwork").on_click(act(|this, window, cx| {
                    this.set_overlay(Some(Overlay::Artwork), window, cx)
                })))
                .item(
                    MenuItem::new("Source snapshot").on_click(act(|this, window, cx| {
                        this.set_overlay(Some(Overlay::Source), window, cx)
                    })),
                );
        }
        menu.header("Right arrow to continue · Space to pause")
            .into_any_element()
    }

    fn detail_overlay(&self, overlay: Overlay, cx: &mut Context<Self>) -> AnyElement {
        let theme = self.theme;
        let body: AnyElement = match overlay {
            Overlay::Options => unreachable!("the options menu is a popover"),
            Overlay::Artwork => self.artwork(),
            Overlay::Source => self.source_snapshot(),
            Overlay::Error => div()
                .code_text()
                .child(self.model.error.clone().unwrap_or_default())
                .into_any_element(),
        };
        theme
            .popover_surface()
            .absolute()
            .top(px(60.))
            .right(px(20.))
            .w(px(560.))
            .max_h(relative(0.8))
            .flex()
            .flex_col()
            .gap(px(8.))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(12.))
                    .child(
                        theme
                            .text_button("overlay-back", "Back")
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.set_overlay(Some(Overlay::Options), window, cx)
                            })),
                    )
                    .child(
                        div()
                            .flex_1()
                            .type_scale(TypeScale::Title)
                            .child(overlay.title()),
                    )
                    .child(
                        theme
                            .icon_action(
                                "overlay-close",
                                Glyph::Close,
                                "Close panel (Escape)",
                                ButtonKind::Quiet,
                            )
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.set_overlay(None, window, cx)
                            })),
                    ),
            )
            .child(
                div()
                    .id("overlay-body")
                    .min_h_0()
                    .overflow_y_scroll()
                    .child(body),
            )
            .into_any_element()
    }

    fn artwork(&self) -> AnyElement {
        let Some(session) = self.model.session.as_ref() else {
            return div().into_any_element();
        };
        let visuals = session.active_visuals();
        if visuals.is_empty() {
            return stage::secondary("This moment has no artwork.").into_any_element();
        }
        div()
            .flex()
            .flex_col()
            .gap(px(20.))
            .children(visuals.into_iter().map(|visual| {
                let kind = match visual.binding.kind {
                    AssetKind::Character => "Character",
                    AssetKind::Location => "Location",
                    AssetKind::Prop => "Prop",
                    AssetKind::Illustration => "Illustration",
                };
                div()
                    .flex()
                    .flex_col()
                    .gap(px(8.))
                    .children(self.images.get(session, visual).map(|image| {
                        gpui_kit::img(image.image.clone())
                            .w_full()
                            .h(px(220.))
                            .bg(rgb(GROUND))
                            .object_fit(gpui_kit::ObjectFit::Contain)
                    }))
                    .child(
                        div()
                            .flex()
                            .justify_between()
                            .child(visual.binding.name.to_string())
                            .child(div().text_color(self.theme.text_muted).child(kind)),
                    )
            }))
            .into_any_element()
    }

    fn source_snapshot(&self) -> AnyElement {
        let Some(session) = self.model.session.as_ref() else {
            return div().into_any_element();
        };
        let plan = session.runtime.plan();
        let span = session.runtime.current().source;
        let (file, text, span) = match plan.source_map.as_ref().and_then(|map| {
            map.original_span(span)
                .map(|(path, span)| (map, path, span))
        }) {
            Some((map, path, span)) => (Some(path), map.workspace.files[path].as_str(), span),
            None => (None, plan.source.as_str(), span),
        };
        let position = Source::new(text).position(span.start);
        let mut caption = "Source snapshot".to_owned();
        if let Some(file) = file {
            caption.push_str(&format!(" · {file}"));
        }
        if let Some(position) = position {
            caption.push_str(&format!(
                " · line {}:{}",
                position.line + 1,
                position.character + 1
            ));
        }
        let executing = position.map(|p| p.line as usize);
        div()
            .flex()
            .flex_col()
            .gap(px(8.))
            .child(div().text_color(self.theme.text_muted).child(caption))
            .child(
                div()
                    .code_text()
                    .children(text.lines().enumerate().map(|(line, text)| {
                        div()
                            .when(executing == Some(line), |this| {
                                this.bg(rgba(0xdfb87518)).text_color(rgb(0xf0d5a8))
                            })
                            .child(format!("{}: {text}", line + 1))
                    })),
            )
            .into_any_element()
    }
}

impl Render for PlaybackPanel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = self.theme;
        let frame = self
            .model
            .session
            .as_ref()
            .map(|session| Frame::new(session, self.viewer));
        let title = self
            .model
            .session
            .as_ref()
            .and_then(|s| s.runtime.plan().title.as_ref())
            .map(|t| t.value.clone())
            .unwrap_or_else(|| "Storyfella Player".into());
        let options_open = self.overlay == Some(Overlay::Options);
        let trigger = theme
            .icon_action(
                "playback-options-trigger",
                Glyph::More,
                "Playback options",
                ButtonKind::Quiet,
            )
            .on_click(cx.listener(move |this, _, window, cx| {
                let next = if this.overlay.is_some() {
                    None
                } else {
                    Some(Overlay::Options)
                };
                this.set_overlay(next, window, cx);
            }));
        let menu = options_open.then(|| {
            div()
                .on_mouse_down_out(cx.listener(|this, _, window, cx| {
                    if this.overlay == Some(Overlay::Options) {
                        this.set_overlay(None, window, cx);
                    }
                }))
                .child(self.options_menu(cx))
                .into_any_element()
        });
        let header = div()
            .flex_none()
            .min_h(px(54.))
            .px(px(28.))
            .flex()
            .items_center()
            .gap(px(20.))
            .border_b_1()
            .border_color(rgba(0xffffff09))
            .bg(rgb(0x15191d))
            .child(
                div()
                    .flex_1()
                    .text_size(px(13.))
                    .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                    .text_color(rgb(0xf0eee9))
                    .child(if self.viewer.previewing_draft {
                        "Preview"
                    } else {
                        "Storyfella"
                    }),
            )
            .when(!self.viewer.previewing_draft, |this| {
                this.child(
                    div()
                        .flex_1()
                        .text_center()
                        .text_size(px(13.))
                        .text_color(rgb(MUTED))
                        .text_ellipsis()
                        .child(title),
                )
            })
            .child(div().flex_1().flex().justify_end().child(theme.popover(
                "playback-options",
                trigger,
                menu,
            )));
        let layers = match (&frame, self.model.session.as_ref()) {
            (Some(frame), Some(session)) => stage::backdrop_layers(frame, session, &self.images),
            _ => Vec::new(),
        };
        let reading = self.reading(frame.as_ref(), cx);
        let stage = stage::surface(layers, reading)
            .id("story-stage")
            .overflow_y_scroll();
        let detail = self
            .overlay
            .filter(|overlay| *overlay != Overlay::Options)
            .map(|overlay| self.detail_overlay(overlay, cx));
        div()
            .key_context(CONTEXT)
            .track_focus(&self.focus)
            .on_action(cx.listener(Self::advance))
            .on_action(cx.listener(Self::toggle_pause))
            .on_action(cx.listener(Self::close_overlay))
            .relative()
            .size_full()
            .flex()
            .flex_col()
            .bg(rgb(GROUND))
            .text_color(rgb(INK))
            .text_size(px(14.))
            .child(header)
            .child(stage)
            .children(detail)
    }
}
