mod actions;
mod art_actions;
mod artwork;
pub mod assets;
mod atelier;
mod close;
pub(crate) mod components;
mod dialog;
mod editable;
pub mod gallery;
mod guide;
mod icons;
mod inputs;
mod inspector;
mod keys;
mod palette;
mod plans;
pub mod preview;
mod project_files;
mod projects;
mod settings;
pub mod shortcuts;
mod sync;
mod text_input;
pub(crate) mod theme;
pub mod writer;
mod writing_assistant;
mod writing_review;
mod writing_session;

pub const QUIT_MENU_ID: &str = "storyfella-quit";

use std::{
    path::{Path, PathBuf},
    time::Duration,
};

use dioxus::prelude::*;

use storyfella_studio_core::config::AppConfig;
use storyfella_studio_core::errors::ReportedError;
use storyfella_studio_core::registry::backend::CreativeBackend;

use crate::{
    commands::{self, ExecutionUi},
    driver::{self, DriveContext, GuideMode, GuideSession, Plan, PlanLaunch},
    state::AppState,
};

use assets::Assets;
use components::{ErrorDetailsHost, ErrorToast, IconButton};
use guide::GuideCard;
use icons::{Icon, IconName};
use plans::Plans;
use projects::Projects;
use settings::Settings;
use writer::Writer;

/// How often the workspace writes the project to disk.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum AutosaveMode {
    #[default]
    OnChange,
    EveryMinute,
    Off,
}

impl AutosaveMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::OnChange => "on_change",
            Self::EveryMinute => "every_minute",
            Self::Off => "off",
        }
    }

    pub fn from_value(value: &str) -> Self {
        match value {
            "on_change" => Self::OnChange,
            "every_minute" => Self::EveryMinute,
            _ => Self::Off,
        }
    }
}

/// Writes the current registry snapshot to the project database. No-op
/// when no project file is open; clears the dirty flag on success. A
/// failure is logged with its code and surfaced through the shared error
/// report signal.
pub fn persist_state(app_state: Signal<AppState>, mut error_report: Signal<Option<ReportedError>>) {
    if let Err(error) = commands::save_project(app_state) {
        error_report.set(Some(error.log()));
    }
}

/// Explicit Save applies description drafts; autosave leaves drafts editable.
fn save_current_project(
    mut app_state: Signal<AppState>,
    mut error_report: Signal<Option<ReportedError>>,
) {
    if app_state.read().operation_in_progress() {
        return;
    }
    let result = app_state.write().apply_description_drafts();
    if let Err(message) = result {
        error_report.set(Some(ReportedError::plain(
            storyfella_studio_core::errors::ErrorCode::AppCommandInvalid,
            message,
        )));
        return;
    }
    persist_state(app_state, error_report);
}

/// Precise Writer position and the grid's independent selection.
#[derive(Clone, Copy)]
pub struct WriterCaret {
    pub position: Signal<storyfella_language::Position>,
    pub target: Signal<Option<storyfella_language::Position>>,
}

#[derive(Clone, Copy)]
struct SidebarVisibility(Signal<bool>);
#[derive(Clone, Copy)]
struct ProjectFilePreview(Signal<Option<PathBuf>>);
impl WriterCaret {
    pub fn navigate(mut self, position: storyfella_language::Position) {
        self.position.set(position);
        self.target.set(Some(position));
    }
}
#[derive(Clone, Copy)]
pub struct GridCursor(pub Signal<usize>);

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum AppScreen {
    Projects,
    Writer,
    Assets,
    Preview,
    Plans,
    Settings,
}
impl AppScreen {
    fn available(self, project_open: bool) -> bool {
        project_open || matches!(self, Self::Projects | Self::Plans | Self::Settings)
    }

    pub const ALL: [Self; 6] = [
        Self::Writer,
        Self::Assets,
        Self::Preview,
        Self::Projects,
        Self::Plans,
        Self::Settings,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Self::Projects => "Projects",
            Self::Writer => "Story",
            Self::Assets => "Assets",
            Self::Preview => "Preview",
            Self::Plans => "Plans",
            Self::Settings => "Settings",
        }
    }
    fn icon(self) -> IconName {
        match self {
            Self::Projects => IconName::Folder,
            Self::Writer => IconName::Book,
            Self::Assets => IconName::Image,
            Self::Preview => IconName::Play,
            Self::Plans => IconName::Workflow,
            Self::Settings => IconName::Settings,
        }
    }
}

/// Starts a plan run through the shared driver surface: installs the
/// session (the assistant card appears) and executes the steps. A run is
/// refused while another is active.
#[allow(clippy::too_many_arguments)]
fn start_plan(
    config: AppConfig,
    app_state: Signal<AppState>,
    defaults: Signal<storyfella_studio_core::registry::GenerationDefaults>,
    screen: Signal<AppScreen>,
    playback: Signal<storyfella_playback_ui::PlaybackModel>,
    writer_caret: WriterCaret,
    grid_cursor: GridCursor,
    palette_open: Signal<bool>,
    driver_ui: ExecutionUi,
    session: Signal<Option<GuideSession>>,
    plan: Plan,
    mode: GuideMode,
    exit_on_done: bool,
) {
    if app_state.read().operation_in_progress()
        || session
            .read()
            .as_ref()
            .is_some_and(|run| matches!(run.state, driver::GuideState::Running))
    {
        return;
    }
    let ctx = DriveContext {
        config,
        app_state,
        defaults,
        screen,
        playback,
        writer_caret,
        grid_cursor,
        palette_open,
    };
    driver::run_plan(ctx, driver_ui, plan, mode, session, exit_on_done);
}

#[component]
pub fn Workspace(
    config: AppConfig,
    app_state: Signal<AppState>,
    #[props(default = None)] plan_launch: Option<PlanLaunch>,
) -> Element {
    // Land on Projects so a story is opened before working with assets — the
    // asset catalog cannot save until a project file is bound.
    let mut screen = use_signal(|| AppScreen::Projects);
    let mut writing_help = use_signal(|| false);
    use_context_provider(|| writing_help);
    let selected_scene = use_signal(|| None::<String>);
    use_context_provider(|| selected_scene);
    let mut sidebar_visible = use_signal(|| true);
    use_context_provider(|| SidebarVisibility(sidebar_visible));
    use_context_provider(|| screen);
    let action_request = actions::ActionRequest(use_signal(|| None));
    use_context_provider(|| action_request);
    use_context_provider(|| app_state);
    let mut playback = use_signal(storyfella_playback_ui::PlaybackModel::default);
    use_context_provider(|| playback);
    let preview_settings = use_signal(|| config.preview);
    use_context_provider(|| preview_settings);
    let project_path = app_state.read().project_path.clone();
    use_effect(use_reactive((&project_path,), move |_| {
        playback.set(storyfella_playback_ui::PlaybackModel::default());
    }));
    let backend_config = config.clone();
    // The backend lives in a signal so the Settings model picker can swap
    // the writing model; provider edits rebuild its client.
    let backend = use_signal(|| CreativeBackend::new(&backend_config));
    let project_file_preview = use_signal(|| None::<PathBuf>);
    use_context_provider(|| ProjectFilePreview(project_file_preview));
    dioxus::desktop::use_asset_handler("generated", move |request, responder| {
        let state = app_state.peek();
        let selected = project_file_preview.peek();
        let path = registered_asset_path(&state.registry, request.uri().path()).or_else(|| {
            let candidate = selected.as_ref()?.canonicalize().ok()?;
            let root = state.story.path()?.parent()?.canonicalize().ok()?;
            if !candidate.starts_with(root) {
                return None;
            }
            let key = candidate.to_str().and_then(generated_asset_url)?;
            (key == request.uri().path()).then_some(candidate)
        });
        serve_generated_asset(path.as_deref(), responder);
    });

    // Autosave: the mode lives here and is shared with Settings via context.
    let autosave = use_signal(AutosaveMode::default);
    use_context_provider(|| autosave);

    // Shared operation defaults for authoring guides; no image production UI.
    let defaults = use_signal(|| config.generation_defaults.clone());
    let image_residency = use_signal(|| {
        config
            .image
            .as_ref()
            .map(|image| image.residency.as_str().to_owned())
            .unwrap_or_default()
    });
    use_context_provider(|| image_residency);

    // The app-wide error surface: every failure is reported here (coded
    // log + banner + details modal), whatever screen is showing.
    let error_report = use_signal(|| None::<ReportedError>);
    use_context_provider(|| error_report);

    // Whether the command palette is showing. Owned here because the
    // shortcut that toggles it is app-wide.
    let palette_open = use_signal(|| false);
    let artwork_background = use_signal(artwork::PreviewBackground::default);
    use_context_provider(|| artwork_background);
    use_context_provider(|| palette_open);

    // The two cursors — the writer's caret line and the grid's keyboard
    // selection. Owned here so the plan driver can move them: a demo that
    // cannot move a cursor can only narrate, never show.
    let writer_caret = WriterCaret {
        position: use_signal(storyfella_language::Position::default),
        target: use_signal(|| None),
    };
    use_context_provider(|| writer_caret);
    let grid_cursor = GridCursor(use_signal(|| 0usize));
    use_context_provider(|| grid_cursor);

    // The automation driver: its own execution surface (the assistant card
    // and the runner observe it; the global toast stays untouched) and the
    // session signal the card renders.
    let driver_ui = ExecutionUi::for_driver();
    let session = use_signal(|| None::<GuideSession>);

    // CLI launch (`--plan <file> --mode guide|check`): start the plan once
    // the window is up; check mode prints the report and exits.
    let hook_config = config.clone();
    let hook_driver_ui = driver_ui.clone();
    use_hook(move || {
        let Some(launch) = plan_launch.clone() else {
            return;
        };
        let PlanLaunch { path, mode } = launch;
        let Some(path) = path else {
            return;
        };
        let plan = match std::fs::read_to_string(&path)
            .map_err(|error| format!("cannot read {path:?}: {error}"))
            .and_then(|raw| Plan::from_yaml(&raw))
        {
            Ok(plan) => plan,
            Err(error) => {
                eprintln!("plan error: {error}");
                std::process::exit(1);
            }
        };
        let exit_on_done = mode == GuideMode::Check;
        spawn(async move {
            // Let the first frame settle before the run takes over.
            tokio::time::sleep(Duration::from_millis(400)).await;
            start_plan(
                hook_config,
                app_state,
                defaults,
                screen,
                playback,
                writer_caret,
                grid_cursor,
                palette_open,
                hook_driver_ui,
                session,
                plan,
                mode,
                exit_on_done,
            );
        });
    });

    // Save after every change (the default): the effect re-runs on state
    // changes, saves when the project is dirty, and clears the flag.
    use_effect(move || {
        if autosave() == AutosaveMode::OnChange
            && app_state.read().project_path.is_some()
            && app_state.read().has_unsaved_changes
            && app_state.read().catalog_conflict.is_none()
            && !app_state.read().operation_in_progress()
        {
            persist_state(app_state, error_report);
        }
    });

    // Every-minute mode: one background loop that checks the current mode on
    // each tick (spawned once via use_hook, never duplicated).
    let autosave_loop = autosave;
    let error_report_loop = error_report;
    use_hook(move || {
        spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(60));
            interval.tick().await; // the first tick fires immediately; skip it
            loop {
                interval.tick().await;
                if autosave_loop() == AutosaveMode::EveryMinute
                    && app_state.read().project_path.is_some()
                    && app_state.read().has_unsaved_changes
                    && app_state.read().catalog_conflict.is_none()
                    && !app_state.read().operation_in_progress()
                {
                    persist_state(app_state, error_report_loop);
                }
            }
        });
    });

    let mut palette_open = use_context::<Signal<bool>>();

    let dirty = app_state.read().has_unsaved_changes || app_state.read().has_description_drafts();
    let artifact_count = app_state.read().registry.artifacts.len();

    // The one key listener for the app. Screens observe `last_key` rather
    // than installing their own bridge — the JS listener is registered
    // against `window`, so a second one would never receive anything.
    let mut last_key = use_signal(|| None::<keys::KeyEvent>);
    use_context_provider(|| last_key);
    let mut goto = use_signal(|| None::<std::time::Instant>);
    let mut tick = use_signal(|| 0u64);
    keys::use_keys(move |press| {
        // `Ctrl/Cmd+Z` and `+Shift+Z` (claimed by the bridge in `keys.rs`) are
        // the header's undo/redo, so the key and the button cannot disagree.
        match press.key.as_str() {
            "/" => {
                if app_state.read().project_path.is_some() {
                    palette_open.set(true);
                }
                return;
            }
            "save" => {
                save_current_project(app_state, error_report);
                return;
            }
            "palette" => {
                palette_open.set(true);
                return;
            }
            "sidebar" => {
                let visible = *sidebar_visible.peek();
                sidebar_visible.set(!visible);
                return;
            }
            "undo" => {
                app_state.write().undo();
                return;
            }
            "redo" => {
                app_state.write().redo();
                return;
            }
            _ => {}
        }
        // `g` then `s`/`a` switches screens, matching the tab hints. The
        // pending `g` lives here because the pair spans two presses.
        if goto
            .peek()
            .is_some_and(|started| started.elapsed() < Duration::from_secs(1))
        {
            goto.set(None);
            match press.key.as_str() {
                "s" if AppScreen::Writer.available(app_state.read().project_path.is_some()) => {
                    screen.set(AppScreen::Writer);
                    return;
                }
                "a" if AppScreen::Assets.available(app_state.read().project_path.is_some()) => {
                    app_state.write().clear_selection();
                    screen.set(AppScreen::Assets);
                    return;
                }
                _ => {}
            }
        } else if press.key == "g" {
            goto.set(Some(std::time::Instant::now()));
            return;
        }
        let next = *tick.peek() + 1;
        tick.set(next);
        last_key.set(Some((next, press)));
    });

    // The Plans screen's run handler (captures its own config clone — the
    // `move` closure must not steal the component's `config`; clones inside
    // the body keep it `FnMut` for the `EventHandler`).
    let plans_config = config.clone();
    let plans_driver_ui = driver_ui.clone();
    let on_run_plan = {
        move |(plan, mode): (Plan, GuideMode)| {
            start_plan(
                plans_config.clone(),
                app_state,
                defaults,
                screen,
                playback,
                writer_caret,
                grid_cursor,
                palette_open,
                plans_driver_ui.clone(),
                session,
                plan,
                mode,
                false,
            )
        }
    };

    let dispatch = EventHandler::new(move |action| match action {
        palette::Action::Navigate(destination) => {
            if destination.available(app_state.read().project_path.is_some()) {
                if destination == AppScreen::Assets {
                    app_state.write().clear_selection();
                }
                screen.set(destination);
            }
        }
        palette::Action::Task(action) => {
            screen.set(AppScreen::Assets);
            action_request.send(action);
        }
        palette::Action::Save => save_current_project(app_state, error_report),
        palette::Action::Undo => {
            app_state.write().undo();
        }
        palette::Action::Redo => {
            app_state.write().redo();
        }
        palette::Action::Open(id) => {
            screen.set(AppScreen::Assets);
            app_state.write().select_artifact(id);
        }
    });

    let projects_home = screen() == AppScreen::Projects;
    let shell_rows = if projects_home {
        "grid-rows-[minmax(0,1fr)]"
    } else {
        "grid-rows-[auto_minmax(0,1fr)_auto]"
    };
    rsx! {
        main { class: "studio-shell grid h-screen {shell_rows} overflow-hidden bg-[var(--studio-bg)] font-sans text-zinc-100 selection:bg-violet-500/30",
            if !projects_home {
                header { class: "studio-header",
                    IconButton {
                        icon: IconName::Sidebar,
                        label: shortcuts::tooltip(if sidebar_visible() { "Hide sidebar" } else { "Show sidebar" }, "sidebar"),
                        expanded: sidebar_visible(), controls: "studio-workspace-navigation",
                        onclick: move |_| { let visible = sidebar_visible(); sidebar_visible.set(!visible); },
                    }
                    span { class: "text-sm font-medium text-zinc-400", "{screen().label()}" }
                    div { class: "ml-auto flex items-center gap-1",
                        if screen() == AppScreen::Writer {
                            IconButton { label: "Play story", icon: IconName::Play, onclick: move |_| { preview::start_preview(playback, app_state); screen.set(AppScreen::Preview); } }
                            IconButton { label: "Play scene", icon: IconName::Scene, disabled: selected_scene().is_none(), onclick: move |_| { if let Some(scene) = selected_scene() { preview::start_scene(playback, app_state, scene); screen.set(AppScreen::Preview); } } }
                            IconButton { label: "Writing help", icon: IconName::Sparkles, active: writing_help(), onclick: move |_| writing_help.toggle() }
                        }
                        IconButton { label: shortcuts::tooltip("Commands and navigation", "palette"), icon: IconName::Search, onclick: move |_| palette_open.set(true) }
                        span { class: "mx-2 h-5 w-px bg-white/10" }
                        IconButton { label: shortcuts::tooltip("Undo", "undo"), icon: IconName::Undo, disabled: !app_state.read().can_undo(), onclick: move |_| { app_state.write().undo(); } }
                        IconButton { label: shortcuts::tooltip("Redo", "redo"), icon: IconName::Redo, disabled: !app_state.read().can_redo(), onclick: move |_| { app_state.write().redo(); } }
                        IconButton { label: shortcuts::tooltip("Save project", "save"), icon: IconName::Save, disabled: !dirty || app_state.read().project_path.is_none() || app_state.read().operation_in_progress(), onclick: move |_| save_current_project(app_state, error_report) }
                    }
                }
            }
            div { class: "flex min-h-0 min-w-0",
                if !projects_home {
                    nav {
                        id: "studio-workspace-navigation",
                        hidden: !sidebar_visible(),
                        class: if sidebar_visible() { "studio-navigation" } else { "hidden" },
                        aria_label: "Workspace navigation",
                        for destination in [AppScreen::Writer, AppScreen::Assets] {
                            if destination.available(app_state.read().project_path.is_some()) {
                                ScreenTab { destination, active: screen() == destination, onclick: move |_| { if destination == AppScreen::Assets { app_state.write().clear_selection(); } screen.set(destination); } }
                            }
                        }
                        div { class: "min-h-4 flex-1" }
                        for destination in [AppScreen::Projects, AppScreen::Plans, AppScreen::Settings] {
                            ScreenTab { destination, active: screen() == destination, onclick: move |_| screen.set(destination) }
                        }
                    }
                }
                div { class: "flex min-h-0 min-w-0 flex-1 flex-col",
                sync::CatalogSync { app_state }
                div { class: "min-h-0 flex-1",
                // Keep task owners and drafts alive while navigating. Hidden screens
                // are excluded from focus and accessibility by the native attribute.
                div { hidden: screen() != AppScreen::Projects, class: "h-full", Projects { config: config.clone(), app_state, on_open: move |_| screen.set(AppScreen::Assets) } }
                div { hidden: screen() != AppScreen::Writer, class: "h-full", ProjectGate { open: app_state.read().project_path.is_some(), Writer { app_state, backend } } }
                div { hidden: screen() != AppScreen::Assets, class: "h-full", ProjectGate { open: app_state.read().project_path.is_some(), Assets { app_state } } }
                div { hidden: screen() != AppScreen::Preview, class: "h-full", ProjectGate { open: app_state.read().project_path.is_some(), preview::Preview { app_state } } }
                div { hidden: screen() != AppScreen::Plans, class: "h-full", Plans { on_run: on_run_plan, running: app_state.read().operation_in_progress() || session.read().as_ref().is_some_and(|run| matches!(run.state, driver::GuideState::Running)) } }
                div { hidden: screen() != AppScreen::Settings, class: "h-full", Settings { config: config.clone(), backend } }
                }
                }
            }

            // Status bar: where the keyboard hints and counts live, so they
            // stop crowding the surfaces they describe.
            if !projects_home {
                footer { class: "flex items-center gap-3 border-t border-white/[0.055] bg-[var(--studio-panel)] px-3 font-mono text-[10.5px] text-zinc-600",
                    span { class: "rounded bg-violet-400/15 px-2 py-0.5 text-[10px] font-bold tracking-[0.08em] text-violet-300",
                        if app_state.read().operation_in_progress() { "Working…" } else if dirty { "Unsaved changes" } else if app_state.read().project_path.is_some() { "Saved" } else { "No project open" }
                    }
                    span { "{artifact_count} artifacts" }
                    div { class: "ml-auto flex items-center gap-3",
                        span { class: "font-sans text-zinc-400", "{shortcuts::keys(\"palette\").unwrap_or_default()} commands · {shortcuts::keys(\"save\").unwrap_or_default()} save" }
                    }
                }
            }
        }
        ErrorToast { error_report }
        ErrorDetailsHost {}
        close::CloseGuard { config: config.clone(), app_state }
        if palette_open() {
            palette::Palette { app_state, open: palette_open, onpick: dispatch }
        }
        {guide_overlay(session)}
    }
}

/// Keep authoring screens mounted while a project prerequisite is visible.
#[component]
fn ProjectGate(open: bool, children: Element) -> Element {
    let mut screen = use_context::<Signal<AppScreen>>();
    rsx! {
        if !open {
            div { class: "grid h-full place-content-center justify-items-center gap-4",
                Icon { name: IconName::Folder, class: "size-8 text-zinc-400" }
                h2 { class: "text-lg font-semibold", "Open a project to begin" }
                button { class: "studio-button studio-button-primary", onclick: move |_| screen.set(AppScreen::Projects), "Choose project" }
            }
        }
        div { hidden: !open, class: "h-full", {children} }
    }
}

/// Primary and utility destinations in the workspace rail.
#[component]
fn ScreenTab(destination: AppScreen, active: bool, onclick: EventHandler<()>) -> Element {
    rsx! {
        button {
            class: if active { "studio-nav-item is-active" } else { "studio-nav-item" },
            title: destination.label(),
            aria_current: active.then_some("page"),
            onclick: move |_| onclick.call(()),
            Icon { name: destination.icon(), class: "size-5" }
            span { "{destination.label()}" }
        }
    }
}

/// The assistant card, when a plan run is active.
fn guide_overlay(session: Signal<Option<GuideSession>>) -> Option<Element> {
    session.read().as_ref().is_some().then(|| {
        rsx! {
            GuideCard { session }
        }
    })
}

/// An opaque lookup key for a registered revision path. Including the full
/// path avoids collisions between imported images with the same filename.
pub fn generated_asset_url(path: &str) -> Option<String> {
    use std::hash::{Hash, Hasher};
    if path.is_empty() {
        return None;
    }
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    path.hash(&mut hash);
    Some(format!("/generated/{:016x}", hash.finish()))
}

fn registered_asset_path(
    registry: &storyfella_studio_core::registry::ArtifactRegistry,
    url: &str,
) -> Option<PathBuf> {
    registry
        .artifacts
        .iter()
        .flat_map(|a| &a.revisions)
        .filter_map(|r| r.asset_path.as_deref())
        .find(|path| generated_asset_url(path).as_deref() == Some(url))
        .map(PathBuf::from)
}

fn serve_generated_asset(path: Option<&Path>, responder: dioxus::desktop::RequestAsyncResponder) {
    use dioxus::desktop::wry::http::{Response, StatusCode};

    let content_type = match path
        .and_then(|path| path.extension())
        .and_then(|extension| extension.to_str())
    {
        Some(extension)
            if extension.eq_ignore_ascii_case("jpg") || extension.eq_ignore_ascii_case("jpeg") =>
        {
            "image/jpeg"
        }
        Some(extension) if extension.eq_ignore_ascii_case("webp") => "image/webp",
        _ => "image/png",
    };
    let response = match path.map(std::fs::read).transpose() {
        Ok(Some(bytes)) => Response::builder()
            .status(StatusCode::OK)
            .header("Content-Type", content_type)
            .header("Cache-Control", "no-store")
            .body(bytes),
        Ok(None) | Err(_) => Response::builder()
            .status(StatusCode::NOT_FOUND)
            .body(Vec::new()),
    }
    .unwrap_or_else(|_| Response::new(Vec::new()));
    responder.respond(response);
}

#[cfg(test)]
mod asset_tests {
    use super::*;
    use storyfella_studio_core::registry::{
        ArtifactKind, ArtifactRegistry, CreateArtifact, RevisionOrigin,
    };

    #[test]
    fn artwork_urls_resolve_only_registered_paths_and_distinguish_matching_filenames() {
        let mut registry = ArtifactRegistry::default();
        let id = registry
            .create_artifact(
                ArtifactKind::Illustration,
                CreateArtifact {
                    name: "art".into(),
                    ..Default::default()
                },
            )
            .unwrap();
        for path in ["/project/artwork/image.png", "/import/image.png"] {
            let revision = registry
                .start_revision(id, String::new(), None, None, RevisionOrigin::Canon)
                .unwrap();
            registry.finish_revision(id, revision, path.into()).unwrap();
            let url = generated_asset_url(path).unwrap();
            assert_eq!(
                registered_asset_path(&registry, &url),
                Some(PathBuf::from(path))
            );
        }
        assert_ne!(
            generated_asset_url("/project/artwork/image.png"),
            generated_asset_url("/import/image.png")
        );
        assert!(registered_asset_path(&registry, "/generated/../../secrets").is_none());
        assert!(registered_asset_path(
            &registry,
            &generated_asset_url("/unregistered/image.png").unwrap()
        )
        .is_none());
    }
}
