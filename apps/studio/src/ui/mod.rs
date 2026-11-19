//! The Studio window: Home, the story workspace with its Write, Play and
//! Board views, the command palette and guide card, and the Settings window.
//!
//! [`Studio`] owns the app's state on the UI thread. The plan driver and the
//! live MCP bridge reach it through its [`Mailbox`]; the views reach it
//! directly. Every change to the story goes through [`Studio::changed`], which
//! autosaves, rechecks the compile status and redraws.
mod assistant;
mod board;
mod close;
mod details;
mod guide;
mod home;
mod palette;
mod play;
mod settings;
mod startup;
mod workspace;
mod write;

#[cfg(test)]
mod tests;

use std::{
    panic::{catch_unwind, AssertUnwindSafe},
    path::PathBuf,
    time::Duration,
};

use design_system::{CommandPalette, CommandPaletteEvent, Density, Theme, TypeScale, Typography};
use gpui_kit::{prelude::*, *};
use storyfella_language::Position;
use storyfella_playback_panel::PlaybackPanel;
use storyfella_studio_core::{
    config::{AppConfig, InterfaceSize, ThemePreference},
    errors::{CodedError, ErrorCode, ReportedError},
    playback::{stage::Viewer, PlaybackModel, PlaybackSession},
};

use crate::{
    commands::{self, OpActivity, PendingOperations},
    driver::{GuideSession, PlanLaunch},
    host::{Driven, Mailbox, Place},
    state::AppState,
    status::{Check, Problems},
};

pub use settings::Settings;

pub const TITLE: &str = "Storyfella Studio";
/// Linux matches running windows to the installed `dev.storyfella.studio.desktop`
/// entry, and so to its icon, by this id; it is also the macOS bundle identifier.
const APP_ID: &str = "dev.storyfella.studio";
const CONTEXT: &str = "Studio";

actions!(
    studio,
    [
        Quit,
        Commands,
        NewStory,
        OpenStory,
        Save,
        Undo,
        Redo,
        ShowHome,
        ShowWrite,
        ShowPlay,
        ShowBoard,
        OpenSettings
    ]
);

/// How often the window writes the project to disk. Chosen per session.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum AutosaveMode {
    #[default]
    OnChange,
    EveryMinute,
    Off,
}

impl AutosaveMode {
    pub const ALL: [Self; 3] = [Self::OnChange, Self::EveryMinute, Self::Off];
}

/// Studio's configuration changed, from Settings or from the shared file.
pub enum StudioEvent {
    ConfigChanged,
}

pub struct Studio {
    config: AppConfig,
    launch: PlanLaunch,
    theme: Theme,
    state: AppState,
    guide: Option<GuideSession>,
    place: Place,
    activity: OpActivity,
    error: Option<ReportedError>,
    /// The failure whose details are open.
    details: Option<ReportedError>,
    /// Guidance shown in the status bar until the next.
    hint: Option<String>,
    autosave: AutosaveMode,
    /// The Write view's caret, moved by plans.
    caret: Position,
    /// Whether the open story compiles; `None` until first checked.
    problems: Option<Result<Problems, String>>,
    checking: Option<Task<()>>,
    /// The project last written to the recent-projects index.
    remembered: Option<PathBuf>,
    home: home::Home,
    write: write::Write,
    assistant: assistant::Assistant,
    play: play::Play,
    board: board::Board,
    player: Entity<PlaybackPanel>,
    palette: Entity<CommandPalette>,
    palette_open: bool,
    settings: Option<WindowHandle<Settings>>,
    mailbox: Mailbox,
    focus: FocusHandle,
    closing: bool,
    _tasks: Vec<Task<()>>,
}

impl EventEmitter<StudioEvent> for Studio {}

impl Focusable for Studio {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

/// Opens Studio's window, or a recovery window when its settings cannot load.
pub fn run(launch: PlanLaunch) {
    gpui_kit::application()
        .with_assets(gpui_kit::assets::Assets)
        .run(move |cx| {
            init(cx);
            open(launch, cx);
            cx.activate(true);
        });
}

/// A shortcut on the platform's primary modifier.
macro_rules! key {
    ($keys:literal) => {
        if cfg!(target_os = "macos") {
            concat!("cmd-", $keys)
        } else {
            concat!("ctrl-", $keys)
        }
    };
}
pub(super) use key;

fn init(cx: &mut App) {
    gpui_kit::init(cx);
    storyfella_playback_panel::init(cx);
    write::bind_keys(cx);
    play::bind_keys(cx);
    cx.bind_keys([
        KeyBinding::new(key!("q"), Quit, None),
        KeyBinding::new(key!("k"), Commands, Some(CONTEXT)),
        KeyBinding::new(key!("n"), NewStory, Some(CONTEXT)),
        KeyBinding::new(key!("o"), OpenStory, Some(CONTEXT)),
        KeyBinding::new(key!("s"), Save, Some(CONTEXT)),
        KeyBinding::new(key!("z"), Undo, Some(CONTEXT)),
        KeyBinding::new(key!("shift-z"), Redo, Some(CONTEXT)),
        KeyBinding::new(key!("1"), ShowWrite, Some(CONTEXT)),
        KeyBinding::new(key!("2"), ShowPlay, Some(CONTEXT)),
        KeyBinding::new(key!("3"), ShowBoard, Some(CONTEXT)),
        KeyBinding::new(key!("shift-h"), ShowHome, Some(CONTEXT)),
        KeyBinding::new(key!(","), OpenSettings, Some(CONTEXT)),
    ]);
    cx.set_menus([
        Menu::new("Storyfella Studio").items([
            MenuItem::action("Settings…", OpenSettings),
            MenuItem::separator(),
            MenuItem::action("Quit Storyfella", Quit),
        ]),
        Menu::new("File").items([
            MenuItem::action("New Story…", NewStory),
            MenuItem::action("Open Story…", OpenStory),
            MenuItem::action("Save", Save),
        ]),
        Menu::new("Edit").items([
            MenuItem::action("Undo", Undo),
            MenuItem::action("Redo", Redo),
        ]),
        Menu::new("View").items([
            MenuItem::action("Home", ShowHome),
            MenuItem::action("Write", ShowWrite),
            MenuItem::action("Play", ShowPlay),
            MenuItem::action("Board", ShowBoard),
            MenuItem::separator(),
            MenuItem::action("Commands…", Commands),
        ]),
    ]);
}

/// Height of the bar along the top of every Studio window. It is fixed at
/// every interface size because the traffic lights are placed once, when the
/// window opens, and must stay centred in it.
pub(crate) fn title_bar_height() -> Pixels {
    px(40.)
}

/// Where macOS draws the traffic lights: centred in the title bar. The
/// buttons are 14 points tall.
pub(crate) fn traffic_lights() -> Point<Pixels> {
    point(px(16.), (title_bar_height() - px(14.)) / 2.)
}

fn window_options(size: Size<Pixels>, cx: &App) -> WindowOptions {
    WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(Bounds::centered(None, size, cx))),
        window_min_size: Some(gpui_kit::size(px(720.), px(520.))),
        titlebar: Some(TitlebarOptions {
            title: Some(TITLE.into()),
            appears_transparent: true,
            traffic_light_position: Some(traffic_lights()),
        }),
        focus: true,
        app_id: Some(APP_ID.into()),
        ..WindowOptions::default()
    }
}

fn open(launch: PlanLaunch, cx: &mut App) {
    let size = launch
        .window_size
        .map(|(width, height)| gpui_kit::size(px(width as f32), px(height as f32)))
        .unwrap_or(gpui_kit::size(px(1280.), px(800.)));
    let config = AppConfig::load().map(|mut config| {
        launch.configure_paths(&mut config);
        config
    });
    let config = match config {
        Ok(config) => config,
        Err(error) => {
            startup::open(error, launch, window_options(size, cx), cx);
            return;
        }
    };
    let window = cx
        .open_window(window_options(size, cx), |window, cx| {
            let studio = cx.new(|cx| Studio::new(config, launch, window, cx));
            let guard = studio.downgrade();
            window.on_window_should_close(cx, move |window, cx| {
                guard
                    .update(cx, |studio, cx| studio.can_close(window, cx))
                    .unwrap_or(true)
            });
            let focus = studio.read(cx).focus.clone();
            window.focus(&focus, cx);
            studio
        })
        .expect("open the Studio window");
    // Quit asks the same question as closing the window, from whichever
    // window has focus.
    cx.on_action(move |_: &Quit, cx| {
        window
            .update(cx, |studio, window, cx| {
                if studio.can_close(window, cx) {
                    cx.quit();
                }
            })
            .ok();
    });
    cx.on_window_closed(move |cx, closed| {
        if closed == window.window_id() {
            cx.quit();
        }
    })
    .detach();
}

/// The control density for an interface size. Desktop Studio stays compact
/// by default; Large grows hit areas and spacing with the text.
fn density(size: InterfaceSize) -> Density {
    match size {
        InterfaceSize::Small | InterfaceSize::Default => Density::Compact,
        InterfaceSize::Large => Density::Comfortable,
    }
}

/// The window's rem for an interface size. Every `TypeScale` step is in rems,
/// so this scales all of Studio's text together.
pub(crate) fn rem_size(size: InterfaceSize) -> Pixels {
    px(match size {
        InterfaceSize::Small => 15.,
        InterfaceSize::Default => 16.,
        InterfaceSize::Large => 17.,
    })
}

/// The design-system theme for Studio's appearance settings, following the OS
/// for System.
fn theme_for(config: &AppConfig, window: &Window) -> Theme {
    let preference = config.theme;
    let light = match preference {
        ThemePreference::Light => true,
        ThemePreference::Dark => false,
        ThemePreference::System => matches!(
            window.appearance(),
            WindowAppearance::Light | WindowAppearance::VibrantLight
        ),
    };
    let theme = if light { Theme::light() } else { Theme::dark() };
    theme.with_density(density(config.interface_size))
}

impl Studio {
    fn new(
        config: AppConfig,
        launch: PlanLaunch,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let theme = theme_for(&config, window);
        let (mailbox, mut jobs) = Mailbox::new();
        let pump = cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            while let Some(job) = jobs.next().await {
                let ran = this.update(cx, |studio, cx| {
                    job(&mut Bound { studio, cx });
                });
                if ran.is_err() {
                    break;
                }
            }
        });
        let player = cx.new(|cx| PlaybackPanel::new(viewer(&config), theme, cx));
        cx.observe(&player, |studio, _, cx| {
            studio.light_stage(cx);
            cx.notify();
        })
        .detach();
        let palette = cx.new(|cx| CommandPalette::new(Vec::new(), theme, window, cx));
        cx.subscribe_in(&palette, window, |studio, _, event, window, cx| {
            studio.palette_open = false;
            if let CommandPaletteEvent::Run(id) = event {
                studio.run_command(id, window, cx);
            }
            window.focus(&studio.focus, cx);
            cx.notify();
        })
        .detach();
        cx.observe_window_appearance(window, |studio, window, cx| {
            studio.apply_theme(window, cx);
        })
        .detach();
        let mut studio = Self {
            home: home::Home::new(&config, theme, window, cx),
            write: write::Write::new(theme, window, cx),
            assistant: assistant::Assistant::new(theme, window, cx),
            play: play::Play::new(),
            board: board::Board::new(theme, window, cx),
            config,
            launch,
            theme,
            state: AppState::default(),
            guide: None,
            place: Place::Home,
            activity: OpActivity::Idle,
            error: None,
            details: None,
            hint: None,
            autosave: AutosaveMode::default(),
            caret: Position::default(),
            problems: None,
            checking: None,
            remembered: None,
            player,
            palette,
            palette_open: false,
            settings: None,
            mailbox,
            focus: cx.focus_handle(),
            closing: false,
            _tasks: vec![pump, every_minute(cx), watch_shared_config(cx)],
        };
        if studio.launch.manual_save {
            studio.autosave = AutosaveMode::Off;
        }
        #[cfg(unix)]
        crate::live_mcp::start(studio.mailbox.clone());
        studio.start_launch_plan(cx);
        studio.home.rescan(&studio.config, cx);
        studio
    }

    /// The `--plan` run, started once the first frame has settled.
    fn start_launch_plan(&mut self, cx: &mut Context<Self>) {
        let Some((plan, mode, completion)) = self.launch.plan() else {
            return;
        };
        let plan = match plan {
            Ok(plan) => plan,
            Err(error) => {
                eprintln!("plan error: {error}");
                std::process::exit(1);
            }
        };
        let mailbox = self.mailbox.clone();
        self._tasks.push(
            cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
                cx.background_executor()
                    .timer(Duration::from_millis(400))
                    .await;
                this.update(cx, |studio, cx| {
                    crate::driver::run_plan(
                        &mut Bound { studio, cx },
                        mailbox,
                        plan,
                        mode,
                        completion,
                    );
                })
                .ok();
            }),
        );
    }

    pub fn state(&self) -> &AppState {
        &self.state
    }

    pub fn place(&self) -> Place {
        self.place
    }

    pub fn error(&self) -> Option<&ReportedError> {
        self.error.as_ref()
    }

    /// Where Studio is actually showing work: a story view needs an open story.
    fn shown(&self) -> Place {
        match self.place {
            Place::Write | Place::Play | Place::Board if self.state.project_path.is_none() => {
                Place::Home
            }
            place => place,
        }
    }

    /// Everything that follows a change to the story or project: remembers
    /// a newly opened project, autosaves when asked to, rechecks compilation
    /// and redraws.
    fn changed(&mut self, cx: &mut Context<Self>) {
        let opened = self.state.project_path.clone();
        if opened != self.remembered {
            self.remembered = opened.clone();
            self.player.update(cx, |player, cx| {
                player.update_model(|model| *model = PlaybackModel::default(), cx)
            });
            if let Some(path) = &opened {
                if let Err(error) = crate::recents::remember_project(&self.config.project_dir, path)
                {
                    self.fail(ErrorCode::AppStoriesFolder, error, cx);
                }
            }
        }
        self.hint = None;
        if self.autosave == AutosaveMode::OnChange {
            self.autosave_now(cx);
        }
        self.follow_project_in_assistant(cx);
        self.write.story_changed();
        self.board.story_changed();
        self.follow_caret(true, cx);
        self.recheck(cx);
        cx.notify();
    }

    fn autosave_now(&mut self, cx: &mut Context<Self>) {
        let state = &self.state;
        if state.project_path.is_some()
            && state.has_unsaved_changes
            && state.catalog_conflict.is_none()
            && !state.operation_in_progress()
        {
            if let Err(error) = commands::save_project(&mut self.state) {
                self.report(error, cx);
            }
        }
    }

    /// Compiles the open story off the UI thread, a moment after the last
    /// change, so typing never waits for it.
    fn recheck(&mut self, cx: &mut Context<Self>) {
        let Some(sources) = crate::preview::drafts(&self.state).ok() else {
            self.problems = None;
            self.checking = None;
            self.write.set_check(None, cx);
            return;
        };
        let registry = self.state.registry.clone();
        self.checking = Some(
            cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
                cx.background_executor()
                    .timer(Duration::from_millis(300))
                    .await;
                let checked = cx
                    .background_spawn(async move {
                        let (entry, drafts) = sources;
                        storyfella_studio_core::story::load_source_workspace(&entry, &drafts)
                            .and_then(|workspace| Check::of(&workspace, &registry))
                            .map_err(|error| error.to_string())
                    })
                    .await;
                this.update(cx, |studio, cx| {
                    studio.problems = Some(
                        checked
                            .as_ref()
                            .map(|check| check.problems)
                            .map_err(Clone::clone),
                    );
                    studio.write.set_check(checked.ok(), cx);
                    cx.notify();
                })
                .ok();
            }),
        );
    }

    /// Runs begun operations on a worker and merges them when they finish.
    fn start_operations(&mut self, pending: PendingOperations, cx: &mut Context<Self>) {
        self.activity = OpActivity::Running {
            summary: pending.summary().to_owned(),
        };
        let worker = cx
            .background_spawn(async move { catch_unwind(AssertUnwindSafe(|| pending.run())).ok() });
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let finished = worker.await;
            this.update(cx, |studio, cx| {
                studio.activity = OpActivity::Idle;
                if let Err(error) = commands::finish_operations(&mut studio.state, finished) {
                    studio.error = Some(error);
                }
                studio.changed(cx);
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    fn show(&mut self, place: Place, window: &mut Window, cx: &mut Context<Self>) {
        match place {
            Place::Settings => self.open_settings(cx),
            Place::Play => self.open_play(window, cx),
            Place::Board => self.open_board(window, cx),
            place => {
                self.place = place;
                if place == Place::Home {
                    self.home.rescan(&self.config, cx);
                }
                self.light_stage(cx);
            }
        }
        cx.notify();
    }

    fn open_settings(&mut self, cx: &mut Context<Self>) {
        if let Some(handle) = self.settings {
            if handle
                .update(cx, |_, window, _| window.activate_window())
                .is_ok()
            {
                return;
            }
        }
        let studio = cx.entity();
        cx.defer(move |cx| {
            let handle = settings::open(studio.clone(), cx);
            studio.update(cx, |studio, _| studio.settings = handle);
        });
    }

    fn apply_theme(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let theme = theme_for(&self.config, window);
        if theme.is_light == self.theme.is_light && theme.density == self.theme.density {
            return;
        }
        self.theme = theme;
        self.palette
            .update(cx, |palette, cx| palette.set_theme(theme, cx));
        self.home.set_theme(theme, cx);
        self.write.set_theme(theme, cx);
        self.board.set_theme(theme, cx);
        self.assistant.set_theme(theme, cx);
        let viewer = viewer(&self.config);
        self.player
            .update(cx, |player, cx| player.configure(viewer, theme, cx));
        cx.notify();
    }

    /// Adopts configuration that Settings saved or the shared file changed.
    /// Launch paths, such as a plan run's scratch directories, still apply.
    pub fn set_config(
        &mut self,
        mut config: AppConfig,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.launch.configure_paths(&mut config);
        self.config = config;
        self.apply_theme(window, cx);
        let viewer = viewer(&self.config);
        let theme = self.theme;
        self.player
            .update(cx, |player, cx| player.configure(viewer, theme, cx));
        cx.emit(StudioEvent::ConfigChanged);
        cx.notify();
    }

    pub fn config(&self) -> &AppConfig {
        &self.config
    }

    pub fn theme(&self) -> Theme {
        self.theme
    }

    pub fn autosave(&self) -> AutosaveMode {
        self.autosave
    }

    pub fn set_autosave(&mut self, mode: AutosaveMode, cx: &mut Context<Self>) {
        self.autosave = mode;
        cx.notify();
    }

    fn save(&mut self, _: &Save, window: &mut Window, cx: &mut Context<Self>) {
        if self.state.operation_in_progress() {
            return;
        }
        if let Err(message) = self.state.apply_description_drafts() {
            self.hint(message, cx);
            return;
        }
        if self.state.project_path.is_none() {
            if self.state.has_unsaved_changes {
                self.save_as(window, cx, |_, _, _| {});
            }
            return;
        }
        match commands::save_project(&mut self.state) {
            Ok(()) => self.changed(cx),
            Err(error) => self.report(error, cx),
        }
    }

    /// Asks where to save an unbound draft, saves it there, then runs `then`.
    fn save_as(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
        then: impl FnOnce(&mut Self, &mut Window, &mut Context<Self>) + 'static,
    ) {
        let directory = self.config.project_dir.clone();
        if let Err(error) = std::fs::create_dir_all(&directory) {
            self.fail(ErrorCode::AppStoriesFolder, error.to_string(), cx);
            return;
        }
        let answer = cx.prompt_for_new_path(&directory, Some("Untitled.db"));
        cx.spawn_in(window, async move |this, cx| {
            let Ok(Ok(Some(path))) = answer.await else {
                return;
            };
            let path = if path.extension().is_none() {
                path.with_extension("db")
            } else {
                path
            };
            this.update_in(cx, |studio, window, cx| {
                match commands::save_new_project(&path, &mut studio.state) {
                    Ok(()) => {
                        studio.changed(cx);
                        then(studio, window, cx);
                    }
                    Err(error) => studio.report(error, cx),
                }
            })
            .ok();
        })
        .detach();
    }

    fn undo(&mut self, _: &Undo, _: &mut Window, cx: &mut Context<Self>) {
        if self.state.undo() {
            self.changed(cx);
        }
    }

    fn redo(&mut self, _: &Redo, _: &mut Window, cx: &mut Context<Self>) {
        if self.state.redo() {
            self.changed(cx);
        }
    }

    fn open_palette(&mut self, _: &Commands, window: &mut Window, cx: &mut Context<Self>) {
        self.set_palette(!self.palette_open, window, cx);
    }

    fn set_palette(&mut self, open: bool, window: &mut Window, cx: &mut Context<Self>) {
        self.palette_open = open;
        if open {
            let commands = palette::commands(self);
            self.palette.update(cx, |palette, cx| {
                palette.set_commands(commands, cx);
                palette.reset(window, cx);
            });
        } else {
            window.focus(&self.focus, cx);
        }
        cx.notify();
    }

    fn run_command(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(command) = palette::Command::from_id(id) else {
            return;
        };
        match command {
            palette::Command::Show(place) => self.show(place, window, cx),
            palette::Command::NewStory => self.home.start_creating(window, cx),
            palette::Command::OpenStory => self.prompt_open(cx),
            palette::Command::Save => self.save(&Save, window, cx),
            palette::Command::Undo => self.undo(&Undo, window, cx),
            palette::Command::Redo => self.redo(&Redo, window, cx),
            palette::Command::GoToDefinition => self.go_to_definition(cx),
            palette::Command::RenameSymbol => self.start_rename(window, cx),
            palette::Command::FormatFile => self.format_open_file(cx),
            palette::Command::ToggleProblems => self.toggle_problems(cx),
            palette::Command::TogglePlayFullWindow => self.toggle_full_window(window, cx),
            palette::Command::NewAsset => {
                self.show(Place::Board, window, cx);
                self.start_compose(board::Compose::New, window, cx);
            }
            palette::Command::Quit => window.dispatch_action(Box::new(Quit), cx),
        }
    }

    /// Opens a story chosen in the file dialog: a project folder, its
    /// manifest or database, or a standalone `.sf`.
    fn prompt_open(&mut self, cx: &mut Context<Self>) {
        let answer = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: true,
            multiple: false,
            prompt: Some("Open story".into()),
        });
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let Ok(Ok(Some(paths))) = answer.await else {
                return;
            };
            let Some(path) = paths.into_iter().next() else {
                return;
            };
            this.update(cx, |studio, cx| studio.open_project(&path, cx))
                .ok();
        })
        .detach();
    }

    fn open_project(&mut self, path: &std::path::Path, cx: &mut Context<Self>) {
        match commands::open_project(path, &mut self.state) {
            Ok(()) => {
                self.place = Place::Write;
                self.error = None;
                self.changed(cx);
            }
            Err(error) => self.report(error, cx),
        }
    }

    fn create_project(&mut self, name: &str, cx: &mut Context<Self>) -> Result<(), String> {
        let name = name.trim();
        if name.is_empty() {
            return Err("Name the story.".into());
        }
        match commands::create_project(&self.config.project_dir, name, &mut self.state) {
            Ok(_) => {
                self.place = Place::Write;
                self.error = None;
                self.changed(cx);
                Ok(())
            }
            Err(error) => Err(error.details),
        }
    }
}

/// The reader settings Studio's Play view uses: it always previews a draft.
fn viewer(config: &AppConfig) -> Viewer {
    Viewer {
        previewing_draft: true,
        settings: config.preview,
    }
}

/// Saves once a minute when the session asks for it.
fn every_minute(cx: &mut Context<Studio>) -> Task<()> {
    cx.spawn(
        async move |this: WeakEntity<Studio>, cx: &mut AsyncApp| loop {
            cx.background_executor()
                .timer(Duration::from_secs(60))
                .await;
            let alive = this.update(cx, |studio, cx| {
                if studio.autosave == AutosaveMode::EveryMinute {
                    studio.autosave_now(cx);
                    cx.notify();
                }
            });
            if alive.is_err() {
                break;
            }
        },
    )
}

/// Clipforge replaces the shared provider file atomically; Studio picks up
/// its changes while running.
fn watch_shared_config(cx: &mut Context<Studio>) -> Task<()> {
    cx.spawn(async move |this: WeakEntity<Studio>, cx: &mut AsyncApp| {
        let Ok(path) = this.read_with(cx, |studio, _| studio.config.ai_config_path.clone()) else {
            return;
        };
        let mut last = settings::file_stamp(path.as_deref());
        loop {
            cx.background_executor().timer(Duration::from_secs(2)).await;
            let next = settings::file_stamp(path.as_deref());
            if next == last {
                continue;
            }
            last = next;
            let loaded = AppConfig::load();
            let window = this.read_with(cx, |studio, _| studio.focus.clone());
            if window.is_err() {
                break;
            }
            let applied = this.update(cx, |studio, cx| match loaded {
                Ok(config) => studio.adopt_config(config, cx),
                Err(error) => studio.report(error.report(), cx),
            });
            if applied.is_err() {
                break;
            }
        }
    })
}

impl Studio {
    /// Adopts reloaded configuration where no window is at hand; the theme
    /// follows on the next appearance check.
    fn adopt_config(&mut self, mut config: AppConfig, cx: &mut Context<Self>) {
        self.launch.configure_paths(&mut config);
        self.config = config;
        let viewer = viewer(&self.config);
        let theme = self.theme;
        self.player
            .update(cx, |player, cx| player.configure(viewer, theme, cx));
        cx.emit(StudioEvent::ConfigChanged);
        cx.notify();
    }
}

/// [`Studio`] with its context, as the mailbox's jobs see it.
struct Bound<'a, 'b> {
    studio: &'a mut Studio,
    cx: &'a mut Context<'b, Studio>,
}

impl Driven for Bound<'_, '_> {
    fn config(&self) -> &AppConfig {
        &self.studio.config
    }

    fn state(&mut self) -> &mut AppState {
        // Whatever the job changes is followed through once it returns.
        let studio = self.cx.entity().downgrade();
        self.cx.defer(move |cx| {
            studio.update(cx, |studio, cx| studio.changed(cx)).ok();
        });
        &mut self.studio.state
    }

    fn install_playback(&mut self, session: Result<PlaybackSession, String>) {
        self.studio.install_preview(session, self.cx);
    }

    fn playback(&mut self, act: &mut dyn FnMut(&mut PlaybackModel)) {
        self.studio
            .player
            .update(self.cx, |player, cx| player.update_model(act, cx));
    }

    fn guide(&mut self) -> &mut Option<GuideSession> {
        self.cx.notify();
        &mut self.studio.guide
    }

    fn show(&mut self, place: Place) {
        match place {
            Place::Settings => self.studio.open_settings(self.cx),
            place => self.studio.place = place,
        }
        self.cx.notify();
    }

    fn set_palette(&mut self, open: bool) {
        let studio = self.cx.entity().downgrade();
        self.cx.defer(move |cx| {
            let Some(window) = cx.active_window() else {
                return;
            };
            window
                .update(cx, |_, window, cx| {
                    studio
                        .update(cx, |studio, cx| studio.set_palette(open, window, cx))
                        .ok();
                })
                .ok();
        });
    }

    fn set_caret(&mut self, position: Position) {
        self.studio.caret = position;
        self.studio.write.place_caret(position, &self.studio.state);
        self.cx.notify();
    }

    fn start(&mut self, operations: PendingOperations) {
        self.studio.start_operations(operations, self.cx);
    }

    fn activity(&self) -> OpActivity {
        self.studio.activity.clone()
    }

    fn error(&self) -> Option<ReportedError> {
        self.studio.error.clone()
    }

    fn set_error(&mut self, error: Option<ReportedError>) {
        self.studio.error = error;
        self.cx.notify();
    }
}

impl Render for Studio {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let c = self.theme;
        let shown = self.shown();
        window.set_rem_size(rem_size(self.config.interface_size));
        div()
            .id("studio")
            .key_context(CONTEXT)
            .track_focus(&self.focus)
            .size_full()
            .relative()
            .flex()
            .flex_col()
            .bg(c.surface_deep)
            .text_color(c.text)
            .type_scale(TypeScale::Body)
            .on_action(cx.listener(Self::open_palette))
            .on_action(cx.listener(Self::save))
            .on_action(cx.listener(Self::undo))
            .on_action(cx.listener(Self::redo))
            .on_action(cx.listener(|studio, _: &NewStory, window, cx| {
                studio.show(Place::Home, window, cx);
                studio.home.start_creating(window, cx);
            }))
            .on_action(cx.listener(|studio, _: &OpenStory, _, cx| studio.prompt_open(cx)))
            .on_action(
                cx.listener(|studio, _: &ShowHome, window, cx| {
                    studio.show(Place::Home, window, cx)
                }),
            )
            .on_action(cx.listener(|studio, _: &ShowWrite, window, cx| {
                studio.show(Place::Write, window, cx)
            }))
            .on_action(
                cx.listener(|studio, _: &ShowPlay, window, cx| {
                    studio.show(Place::Play, window, cx)
                }),
            )
            .on_action(cx.listener(|studio, _: &ShowBoard, window, cx| {
                studio.show(Place::Board, window, cx)
            }))
            .on_action(
                cx.listener(|studio, _: &play::ToggleFullWindow, window, cx| {
                    studio.toggle_full_window(window, cx)
                }),
            )
            .on_action(cx.listener(|studio, _: &OpenSettings, window, cx| {
                studio.show(Place::Settings, window, cx)
            }))
            .child(match shown {
                Place::Home => self.render_home(window, cx).into_any_element(),
                view => self.render_workspace(view, window, cx).into_any_element(),
            })
            .children(self.render_guide(cx))
            .children(self.palette_open.then(|| {
                div()
                    .absolute()
                    .inset_0()
                    .bg(c.scrim)
                    .flex()
                    .flex_col()
                    .items_center()
                    .pt(px(96.))
                    .child(self.palette.clone())
            }))
            .child(self.render_rename(cx))
            .child(self.render_compose(cx))
            .child(self.render_trouble(cx))
            .child(self.render_details(cx))
    }
}
