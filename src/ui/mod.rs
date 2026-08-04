mod components;
mod editor;
mod icons;
mod projects;
mod settings;

use std::path::{Path, PathBuf};

use dioxus::prelude::*;

use crate::{
    app::AppConfig,
    runtime::{CreativeRuntime, HealthStatus},
    state::AppState,
};

use components::{RailButton, StatusDot};
use editor::Studio;
use icons::{Icon, IconName};
use projects::Projects;
use settings::Settings;

#[derive(Clone, Copy, Debug, PartialEq)]
enum AppScreen {
    Projects,
    Studio,
    Settings,
}

#[component]
pub fn Workspace(config: AppConfig, app_state: Signal<AppState>) -> Element {
    let mut screen = use_signal(|| AppScreen::Studio);
    let runtime_config = config.clone();
    let runtime = use_hook(move || CreativeRuntime::new(&runtime_config));
    let generated_asset_directory = config.asset_dir.join("generated");
    dioxus::desktop::use_asset_handler("generated", move |request, responder| {
        serve_generated_asset(&generated_asset_directory, request, responder);
    });
    let project_name = app_state.read().project.name.clone();
    let dirty = app_state.read().has_unsaved_changes;

    rsx! {
        main { class: "grid h-screen min-h-[640px] min-w-[900px] grid-cols-[64px_minmax(0,1fr)] grid-rows-[58px_minmax(0,1fr)] overflow-hidden bg-[#0b0b0e] font-sans text-zinc-100 selection:bg-violet-500/30",
            aside { class: "row-span-2 flex flex-col items-center border-r border-white/[0.055] bg-[#09090b] py-3",
                button { class: "mb-6 grid size-10 place-items-center text-violet-300", aria_label: "Smart Visual Sequencer", onclick: move |_| screen.set(AppScreen::Studio),
                    Icon { name: IconName::Sparkles, class: "size-[21px]" }
                }
                nav { class: "flex flex-1 flex-col items-center gap-2",
                    RailButton { label: "Projects", icon: IconName::Home, active: screen() == AppScreen::Projects, onclick: move |_| screen.set(AppScreen::Projects) }
                    RailButton { label: "Create", icon: IconName::Canvas, active: screen() == AppScreen::Studio, onclick: move |_| screen.set(AppScreen::Studio) }
                }
                RailButton { label: "Settings", icon: IconName::Settings, active: screen() == AppScreen::Settings, onclick: move |_| screen.set(AppScreen::Settings) }
            }

            header { class: "flex min-w-0 items-center justify-between border-b border-white/[0.055] bg-[#0b0b0e]/90 px-5 backdrop-blur-xl",
                div { class: "flex min-w-0 items-center gap-3",
                    if screen() == AppScreen::Studio {
                        button { class: "truncate text-[12px] font-medium text-zinc-300 transition hover:text-white", "{project_name}" }
                        if dirty { span { class: "size-1 rounded-full bg-zinc-600", title: "Unsaved changes" } }
                    } else {
                        p { class: "text-[12px] font-medium text-zinc-400", if screen() == AppScreen::Projects { "Projects" } else { "Settings" } }
                    }
                }
                div { class: "flex items-center gap-4",
                    div { class: "flex items-center gap-2 text-[10px] text-zinc-600", StatusDot { status: HealthStatus::Idle } span { "Local · starts on demand" } }
                    if screen() == AppScreen::Studio {
                        button { class: "flex h-8 items-center gap-2 rounded-lg bg-white/[0.055] px-3 text-[10px] font-medium text-zinc-400 transition hover:bg-white/[0.08] hover:text-white",
                            Icon { name: IconName::Play, class: "size-3" }
                            "Preview"
                        }
                        button { class: "h-8 rounded-lg bg-zinc-100 px-3 text-[10px] font-semibold text-zinc-950 transition hover:bg-white", "Export" }
                    }
                }
            }

            div { class: "min-h-0 min-w-0",
                match screen() {
                    AppScreen::Projects => rsx! { Projects { app_state, on_open: move |_| screen.set(AppScreen::Studio) } },
                    AppScreen::Studio => rsx! { Studio { app_state, runtime: runtime.clone() } },
                    AppScreen::Settings => rsx! { Settings { config: config.clone(), runtime: runtime.clone() } },
                }
            }
        }
    }
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
