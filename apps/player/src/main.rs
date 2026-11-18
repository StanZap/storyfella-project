use std::path::PathBuf;

use clap::Parser;
use design_system::Theme;
use gpui_kit::{
    actions, div, prelude::*, px, size, AsyncApp, Context, Entity, Focusable, KeyBinding, Menu,
    MenuItem, PathPromptOptions, TitlebarOptions, WeakEntity, Window, WindowBounds, WindowOptions,
};
use storyfella_playback_panel::{PlaybackEvent, PlaybackPanel};
use storyfella_runtime::AssetPolicy;
use storyfella_studio_core::{
    config::PreviewSettings,
    playback::{stage::Viewer, PlaybackSession},
};

#[derive(Clone, Parser)]
#[command(about = "Play a Storyfella project, .sf/.db pair or compiled JSON plan")]
struct Options {
    input: Option<PathBuf>,
    /// Resource project or database for a compiled JSON plan with pinned artwork.
    #[arg(long)]
    project: Option<PathBuf>,
    /// Play despite missing or unreadable artwork, using black placeholders.
    #[arg(long)]
    allow_missing_assets: bool,
}

impl Options {
    fn policy(&self) -> AssetPolicy {
        if self.allow_missing_assets {
            AssetPolicy::AllowMissing
        } else {
            AssetPolicy::RequireCompleted
        }
    }
}

actions!(player, [Quit, OpenStory]);

fn main() {
    let options = Options::parse();
    gpui_kit::application()
        .with_assets(gpui_kit::assets::Assets)
        .run(move |cx| {
            gpui_kit::init(cx);
            storyfella_playback_panel::init(cx);
            cx.bind_keys([
                KeyBinding::new("cmd-q", Quit, None),
                KeyBinding::new("cmd-o", OpenStory, None),
                KeyBinding::new("ctrl-q", Quit, None),
                KeyBinding::new("ctrl-o", OpenStory, None),
            ]);
            cx.on_action(|_: &Quit, cx| cx.quit());
            cx.set_menus([
                Menu::new("Storyfella Player").items([MenuItem::action("Quit Player", Quit)]),
                Menu::new("File").items([MenuItem::action("Open story…", OpenStory)]),
            ]);
            cx.on_window_closed(|cx, _| {
                if cx.windows().is_empty() {
                    cx.quit();
                }
            })
            .detach();
            gpui_kit::open_window(
                WindowOptions {
                    titlebar: Some(TitlebarOptions {
                        title: Some("Storyfella Player".into()),
                        ..Default::default()
                    }),
                    window_bounds: Some(WindowBounds::centered(size(px(1280.), px(720.)), cx)),
                    window_min_size: Some(size(px(640.), px(480.))),
                    ..Default::default()
                },
                cx,
                |window, cx| {
                    let player = cx.new(|cx| Player::new(options, cx));
                    let focus = player.read(cx).panel.focus_handle(cx);
                    window.focus(&focus, cx);
                    player
                },
            )
            .expect("Open the Player window");
            cx.activate(true);
        });
}

struct Player {
    panel: Entity<PlaybackPanel>,
    options: Options,
}

impl Player {
    fn new(options: Options, cx: &mut Context<Self>) -> Self {
        let settings = PreviewSettings::load().map_err(|error| error.to_string());
        let viewer = Viewer {
            previewing_draft: false,
            settings: settings.as_ref().copied().unwrap_or_default(),
        };
        let panel = cx.new(|cx| PlaybackPanel::new(viewer, Theme::dark(), cx).can_open(true));
        cx.subscribe(&panel, |this, _, event, cx| match event {
            PlaybackEvent::OpenRequested => this.open_story(cx),
        })
        .detach();
        panel.update(cx, |panel, cx| {
            if let Some(input) = &options.input {
                panel.install(
                    PlaybackSession::open_with_policy(
                        input,
                        options.project.as_deref(),
                        options.policy(),
                    ),
                    cx,
                );
            } else if options.project.is_some() {
                panel.report("--project requires a compiled JSON input", cx);
            }
            if let Err(error) = settings {
                if panel.model().error.is_none() {
                    panel.report(error, cx);
                }
            }
        });
        Self { panel, options }
    }

    fn open(&mut self, _: &OpenStory, _: &mut Window, cx: &mut Context<Self>) {
        self.open_story(cx);
    }

    /// Asks for a story, then, for a compiled plan with pinned artwork, for
    /// the project that holds that artwork.
    fn open_story(&mut self, cx: &mut Context<Self>) {
        let policy = self.options.policy();
        let panel = self.panel.downgrade();
        let story = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: true,
            multiple: false,
            prompt: Some("Open story".into()),
        });
        cx.spawn(async move |_: WeakEntity<Self>, cx: &mut AsyncApp| {
            let Some(input) = first_path(story.await) else {
                return;
            };
            let project = if input.extension().is_some_and(|e| e == "json") {
                match PlaybackSession::plan_requires_project_with_policy(&input, policy) {
                    Ok(false) => None,
                    Ok(true) => {
                        let prompt = cx.update(|cx| {
                            cx.prompt_for_paths(PathPromptOptions {
                                files: true,
                                directories: true,
                                multiple: false,
                                prompt: Some("Choose the plan's artwork project".into()),
                            })
                        });
                        let Some(project) = first_path(prompt.await) else {
                            return;
                        };
                        Some(project)
                    }
                    Err(error) => {
                        panel
                            .update(cx, |panel, cx| panel.install(Err(error), cx))
                            .ok();
                        return;
                    }
                }
            } else {
                None
            };
            // Opening decodes every pinned image, so it stays off the UI thread.
            let session = cx
                .background_spawn(async move {
                    PlaybackSession::open_with_policy(&input, project.as_deref(), policy)
                })
                .await;
            panel
                .update(cx, |panel, cx| panel.install(session, cx))
                .ok();
        })
        .detach();
    }
}

fn first_path(
    answer: Result<anyhow::Result<Option<Vec<PathBuf>>>, impl std::fmt::Debug>,
) -> Option<PathBuf> {
    answer.ok()?.ok()??.into_iter().next()
}

impl Render for Player {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .on_action(cx.listener(Self::open))
            .child(self.panel.clone())
    }
}
