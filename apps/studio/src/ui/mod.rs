pub mod canvas;
mod cmdbar;
mod components;
pub mod gallery;
mod guide;
mod icons;
mod inspector;
mod keys;
mod palette;
mod plans;
mod projects;
mod settings;
pub mod shortcuts;
pub mod tools;
pub mod writer;

use std::{
    path::{Path, PathBuf},
    time::Duration,
};

use dioxus::prelude::*;

use storyfella_studio_core::config::AppConfig;
use storyfella_studio_core::errors::{CodedError, ReportedError};
use storyfella_studio_core::registry::backend::CreativeBackend;
use storyfella_studio_core::runtime::HealthStatus;

use crate::{
    commands::{self, ExecutionUi},
    driver::{self, DriveContext, GuideMode, GuideSession, Plan, PlanLaunch},
    state::AppState,
};

use canvas::Canvas;
use components::{ErrorToast, StatusDot};
use guide::GuideCard;
use icons::{Icon, IconName};
use plans::Plans;
use projects::Projects;
use settings::Settings;
use writer::Writer;

/// How long the app waits for the generation server to stop on quit before
/// leaving it.
const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(5);

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

/// Newtypes so two `Signal<usize>` contexts stay distinguishable.
#[derive(Clone, Copy)]
pub struct WriterCaret(pub Signal<usize>);
#[derive(Clone, Copy)]
pub struct GridCursor(pub Signal<usize>);

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum AppScreen {
    Projects,
    Writer,
    Canvas,
    Plans,
    Settings,
}

/// Starts a plan run through the shared driver surface: installs the
/// session (the assistant card appears) and executes the steps. A run is
/// refused while another is active.
#[allow(clippy::too_many_arguments)]
fn start_plan(
    config: AppConfig,
    app_state: Signal<AppState>,
    backend: Signal<CreativeBackend>,
    defaults: Signal<storyfella_studio_core::registry::GenerationDefaults>,
    screen: Signal<AppScreen>,
    composer: Signal<String>,
    writer_caret: WriterCaret,
    grid_cursor: GridCursor,
    palette_open: Signal<bool>,
    active_tool: canvas::ActiveTool,
    driver_ui: ExecutionUi,
    session: Signal<Option<GuideSession>>,
    plan: Plan,
    mode: GuideMode,
    exit_on_done: bool,
) {
    if session.read().is_some() {
        return;
    }
    let ctx = DriveContext {
        config,
        app_state,
        backend,
        defaults,
        screen,
        composer,
        writer_caret,
        grid_cursor,
        palette_open,
        active_tool,
    };
    driver::run_plan(ctx, driver_ui, plan, mode, session, exit_on_done);
}

#[component]
pub fn Workspace(
    config: AppConfig,
    app_state: Signal<AppState>,
    #[props(default = None)] plan_launch: Option<PlanLaunch>,
) -> Element {
    // Land on Projects so a story is opened before any canvas work — the
    // canvas cannot save until a project file is bound.
    let mut screen = use_signal(|| AppScreen::Projects);
    let backend_config = config.clone();
    // The backend lives in a signal so the Settings model picker can swap
    // the planner model and the canvas picks it up on the next op.
    let backend = use_signal(|| CreativeBackend::new(&backend_config));
    // Bounded stop of the generation server this app started, run at exit —
    // including the check-mode run, which leaves through `std::process::exit`
    // (no destructors) and would otherwise orphan the server. The registered
    // closure holds a runtime clone, so the child is not dropped before it runs.
    let shutdown_backend = backend;
    use_hook(move || {
        let runtime = shutdown_backend.read().runtime();
        crate::shutdown::register(move || {
            if !runtime.stop_generation_runtime_blocking(SHUTDOWN_TIMEOUT) {
                tracing::warn!("generation server did not stop within {SHUTDOWN_TIMEOUT:?}");
            }
        });
    });
    let generated_asset_directory = config.asset_dir.join("generated");
    dioxus::desktop::use_asset_handler("generated", move |request, responder| {
        serve_generated_asset(&generated_asset_directory, request, responder);
    });

    // Autosave: the mode lives here and is shared with Settings via context.
    let autosave = use_signal(AutosaveMode::default);
    use_context_provider(|| autosave);

    // Generation defaults (steps, per-kind sizes, LoRA bindings): shared
    // with the canvas ops and editable in Settings, which persists them.
    let defaults = use_signal(|| config.generation_defaults.clone());
    use_context_provider(|| defaults);

    // Prewarm: start the resident generation server when a project opens, so
    // the first generation is not a multi-minute weight load. Editable in
    // Settings, which persists it to the user config.
    let prewarm = use_signal(|| config.prewarm);
    use_context_provider(|| prewarm);

    // Image residency (`resident`/`lowvram`): a launch-time choice, shared so
    // Settings can change it; a running server restarts into the new mode.
    let image_residency = use_signal(|| config.image.residency.as_str().to_owned());
    use_context_provider(|| image_residency);

    // The app-wide error surface: every failure is reported here (coded
    // log + banner + details modal), whatever screen is showing.
    let error_report = use_signal(|| None::<ReportedError>);
    use_context_provider(|| error_report);

    // The canvas composer text: owned here so the driver can type into it
    // (the visible fidelity of a `compose` step), consumed by the canvas.
    let composer = use_signal(String::new);
    use_context_provider(|| composer);

    // Whether the command palette is showing. Owned here because the
    // shortcut that toggles it is app-wide.
    let palette_open = use_signal(|| false);
    use_context_provider(|| palette_open);

    // The two cursors — the writer's caret line and the grid's keyboard
    // selection. Owned here so the plan driver can move them: a demo that
    // cannot move a cursor can only narrate, never show.
    let writer_caret = WriterCaret(use_signal(|| 0usize));
    use_context_provider(|| writer_caret);
    let grid_cursor = GridCursor(use_signal(|| 0usize));
    use_context_provider(|| grid_cursor);
    // The canvas tool is provided here, beside the other cursors, rather
    // than in the canvas: the driver context is built at this level, and a
    // plan that drives a tool has to reach the same signal the rail does.
    let active_tool = canvas::ActiveTool(use_signal(tools::Tool::default));
    use_context_provider(|| active_tool);

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
                backend,
                defaults,
                screen,
                composer,
                writer_caret,
                grid_cursor,
                palette_open,
                active_tool,
                hook_driver_ui,
                session,
                plan,
                mode,
                exit_on_done,
            );
        });
    });

    // Prewarm on project open: once per opened project, when the toggle is on,
    // start the resident server in the background. A failure is logged, not
    // surfaced — the first generation would report the same error anyway.
    let prewarm_backend = backend;
    let mut prewarmed_for = use_signal(|| None::<PathBuf>);
    use_effect(move || {
        let path = app_state.read().project_path.clone();
        let Some(path) = path else {
            return;
        };
        if !prewarm() || prewarmed_for() == Some(path.clone()) {
            return;
        }
        prewarmed_for.set(Some(path));
        let backend = prewarm_backend.read().clone();
        spawn(async move {
            if let Err(error) = backend.ensure_ready().await {
                let _ = error.report().log();
            }
        });
    });

    // Save after every change (the default): the effect re-runs on state
    // changes, saves when the project is dirty, and clears the flag.
    use_effect(move || {
        if autosave() == AutosaveMode::OnChange
            && app_state.read().project_path.is_some()
            && app_state.read().has_unsaved_changes
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
                {
                    persist_state(app_state, error_report_loop);
                }
            }
        });
    });

    // Cmd/Ctrl+S always saves, whatever the autosave mode.
    let shortcut_app_state = app_state;
    let shortcut_error_report = error_report;
    // These are *global* hotkeys, so registration fails when another
    // instance already holds them — a foreseeable condition, not a bug in
    // this process. Panicking took the whole UI down with it; the in-app
    // Save and palette controls work regardless, so a warning is the honest
    // response.
    if let Err(error) = dioxus::desktop::use_global_shortcut("CmdOrCtrl+S", move |_| {
        persist_state(shortcut_app_state, shortcut_error_report);
    }) {
        tracing::warn!(
            ?error,
            "Ctrl+S is held by another process; use the Save control"
        );
    }

    // The palette is opened from here rather than from the canvas so the
    // shortcut registers once for the app, not once per screen mount.
    let mut palette_open = use_context::<Signal<bool>>();
    if let Err(error) = dioxus::desktop::use_global_shortcut("CmdOrCtrl+K", move |_| {
        let showing = *palette_open.peek();
        palette_open.set(!showing);
    }) {
        tracing::warn!(
            ?error,
            "Ctrl+K is held by another process; use the palette control"
        );
    }

    let project_name = app_state.read().project_name.clone();
    let dirty = app_state.read().has_unsaved_changes;
    let artifact_count = app_state.read().registry.artifacts.len();

    // The one key listener for the app. Screens observe `last_key` rather
    // than installing their own bridge — the JS listener is registered
    // against `window`, so a second one would never receive anything.
    let mut last_key = use_signal(|| None::<keys::KeyEvent>);
    use_context_provider(|| last_key);
    let mut goto = use_signal(|| false);
    let mut tick = use_signal(|| 0u64);
    keys::use_keys(move |press| {
        // `Ctrl/Cmd+Z` and `+Shift+Z` (claimed by the bridge in `keys.rs`) are
        // the header's undo/redo, so the key and the button cannot disagree.
        match press.key.as_str() {
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
        // `g` then `s`/`c` switches screens, matching the tab hints. The
        // pending `g` lives here because the pair spans two presses.
        if *goto.peek() {
            goto.set(false);
            match press.key.as_str() {
                "s" => {
                    screen.set(AppScreen::Writer);
                    return;
                }
                "c" => {
                    screen.set(AppScreen::Canvas);
                    return;
                }
                _ => {}
            }
        } else if press.key == "g" {
            goto.set(true);
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
                backend,
                defaults,
                screen,
                composer,
                writer_caret,
                grid_cursor,
                palette_open,
                active_tool,
                plans_driver_ui.clone(),
                session,
                plan,
                mode,
                false,
            )
        }
    };

    rsx! {
        // The prototype's chrome: a top bar of screen tabs, the work, and a
        // status bar. The old icon rail is gone — with two primary screens
        // and a palette, a permanent 64px column was navigation nobody used.
        main { class: "grid h-screen min-h-[640px] min-w-[900px] grid-rows-[44px_minmax(0,1fr)_26px] overflow-hidden bg-[#0b0b0e] font-sans text-zinc-100 selection:bg-violet-500/30",

            header { class: "flex min-w-0 items-center gap-3.5 border-b border-white/[0.055] bg-[#101014] px-3.5",
                button {
                    class: "text-[11px] font-semibold uppercase tracking-[0.04em] text-zinc-600 transition hover:text-zinc-400",
                    onclick: move |_| screen.set(AppScreen::Projects),
                    "Storyfella"
                }
                p { class: "min-w-0 truncate text-[12px] text-zinc-400",
                    "{project_name}"
                    if dirty {
                        span { class: "ml-1.5 text-violet-400", title: "Unsaved changes", "●" }
                    }
                }

                nav { class: "ml-auto flex items-center gap-0.5",
                    ScreenTab { label: "Story", hint: "g s", active: screen() == AppScreen::Writer, onclick: move |_| screen.set(AppScreen::Writer) }
                    ScreenTab { label: "Canvas", hint: "g c", active: screen() == AppScreen::Canvas, onclick: move |_| screen.set(AppScreen::Canvas) }
                    ScreenTab { label: "Projects", hint: "", active: screen() == AppScreen::Projects, onclick: move |_| screen.set(AppScreen::Projects) }
                    ScreenTab { label: "Plans", hint: "", active: screen() == AppScreen::Plans, onclick: move |_| screen.set(AppScreen::Plans) }
                    ScreenTab { label: "Settings", hint: "", active: screen() == AppScreen::Settings, onclick: move |_| screen.set(AppScreen::Settings) }
                }

                div { class: "flex items-center gap-1",
                    button { class: "grid size-7 place-items-center rounded-lg text-zinc-600 transition hover:bg-white/[0.05] hover:text-zinc-200 disabled:opacity-30", title: shortcuts::tooltip("Undo", "undo"), disabled: !app_state.read().can_undo(), onclick: move |_| { app_state.write().undo(); },
                        Icon { name: IconName::Undo, class: "size-3" }
                    }
                    button { class: "grid size-7 place-items-center rounded-lg text-zinc-600 transition hover:bg-white/[0.05] hover:text-zinc-200 disabled:opacity-30", title: shortcuts::tooltip("Redo", "redo"), disabled: !app_state.read().can_redo(), onclick: move |_| { app_state.write().redo(); },
                        Icon { name: IconName::Redo, class: "size-3" }
                    }
                    button { class: "grid size-7 place-items-center rounded-lg text-zinc-600 transition hover:bg-white/[0.05] hover:text-zinc-200 disabled:opacity-30", title: "Save (Cmd/Ctrl+S)", disabled: !dirty, onclick: move |_| persist_state(app_state, error_report),
                        Icon { name: IconName::Save, class: "size-3" }
                    }
                }
            }

            div { class: "min-h-0 min-w-0",
                match screen() {
                    AppScreen::Projects => rsx! { Projects { config: config.clone(), app_state, on_open: move |_| screen.set(AppScreen::Canvas) } },
                    AppScreen::Writer => rsx! { Writer { app_state } },
                    AppScreen::Canvas => rsx! { Canvas { app_state, backend, defaults } },
                    AppScreen::Plans => rsx! { Plans { on_run: on_run_plan } },
                    AppScreen::Settings => rsx! { Settings { config: config.clone(), backend, defaults } },
                }
            }

            // Status bar: where the keyboard hints and counts live, so they
            // stop crowding the surfaces they describe.
            footer { class: "flex items-center gap-3 border-t border-white/[0.055] bg-[#101014] px-3 font-mono text-[10.5px] text-zinc-600",
                span { class: "rounded bg-violet-400/15 px-2 py-0.5 text-[10px] font-bold tracking-[0.08em] text-violet-300",
                    "NORMAL"
                }
                span { "{artifact_count} artifacts" }
                div { class: "ml-auto flex items-center gap-3",
                    StatusDot { status: HealthStatus::Idle }
                    span { class: "font-sans text-zinc-700", "⌃K palette · ⌃S save" }
                }
            }
        }
        ErrorToast { error_report }
        {guide_overlay(session)}
    }
}

/// One screen tab in the top bar.
#[component]
fn ScreenTab(
    label: &'static str,
    hint: &'static str,
    active: bool,
    onclick: EventHandler<()>,
) -> Element {
    rsx! {
        button {
            class: if active { "flex items-center gap-1.5 rounded-lg bg-violet-400/15 px-2.5 py-1 text-[11px] font-medium text-violet-300" } else { "flex items-center gap-1.5 rounded-lg px-2.5 py-1 text-[11px] font-medium text-zinc-600 transition hover:text-zinc-300" },
            onclick: move |_| onclick.call(()),
            "{label}"
            if !hint.is_empty() {
                span { class: "font-mono text-[9px] opacity-55", "{hint}" }
            }
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

/// Maps a stored asset path to the URL the `generated` asset handler
/// serves (the handler serves `asset_dir/generated/<filename>`).
pub fn generated_asset_url(path: &str) -> Option<String> {
    asset_url("generated", path)
}

fn asset_url(protocol: &str, path: &str) -> Option<String> {
    Path::new(path)
        .file_name()
        .and_then(|filename| filename.to_str())
        .map(|filename| format!("/{protocol}/{filename}"))
}

fn serve_generated_asset(
    directory: &Path,
    request: dioxus::desktop::AssetRequest,
    responder: dioxus::desktop::RequestAsyncResponder,
) {
    use dioxus::desktop::wry::http::{Response, StatusCode};

    let filename = request.uri().path().split('/').nth(2).unwrap_or_default();
    let safe_filename = !filename.is_empty()
        && filename
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '-' | '.'));
    let path = if safe_filename {
        directory.join(filename)
    } else {
        PathBuf::new()
    };
    let response = match std::fs::read(path) {
        Ok(bytes) => Response::builder()
            .status(StatusCode::OK)
            .header("Content-Type", "image/png")
            .header("Cache-Control", "no-store")
            .body(bytes),
        Err(_) => Response::builder()
            .status(StatusCode::NOT_FOUND)
            .body(Vec::new()),
    }
    .unwrap_or_else(|_| Response::new(Vec::new()));
    responder.respond(response);
}
