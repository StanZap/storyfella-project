pub mod sample;

use dioxus::prelude::*;

use storyfella_studio_core::config::{AppConfig, ConfigError};
use storyfella_studio_core::errors::CodedError;

use crate::{driver::PlanLaunch, state::AppState, ui::Workspace};

const TAILWIND_CSS: Asset = asset!("/assets/tailwind.css");

#[component]
pub fn App() -> Element {
    // The launch context (set by main.rs from `--plan`/`--mode` args);
    // defaults to an empty launch when the app is started normally.
    let plan_launch = use_context::<PlanLaunch>();
    let config_launch = plan_launch.clone();
    let mut config_result = use_signal(move || {
        AppConfig::load().map(|mut config| {
            config_launch.configure_paths(&mut config);
            config
        })
    });
    let app_state = use_signal(AppState::default);
    let details = use_signal(|| None::<storyfella_studio_core::errors::ReportedError>);
    use_context_provider(|| crate::ui::components::ErrorDetails(details));
    let retry_launch = plan_launch.clone();
    let mut open_error = use_signal(|| None::<String>);
    // Startup failures have no Workspace/CloseGuard, but native Quit still works.
    let failure_window = dioxus::desktop::use_window();
    dioxus::desktop::use_muda_event_handler(move |event| {
        if event.id.0 == crate::ui::QUIT_MENU_ID && config_result.peek().is_err() {
            failure_window.close();
        }
    });

    rsx! {
        document::Title { "Storyfella Studio" }
        document::Stylesheet { href: TAILWIND_CSS }
        match &*config_result.read() {
            Ok(config) => rsx! { Workspace { config: config.clone(), app_state, plan_launch } },
            Err(error) => {
                let repair_path = match error {
                    ConfigError::Read { path, .. } | ConfigError::Parse { path, .. }
                    | ConfigError::Write { path, .. } | ConfigError::InvalidSize { path, .. }
                    | ConfigError::MalformedSize { path, .. } => path.clone(),
                    ConfigError::Providers(_) | ConfigError::Serialize { .. } => {
                        storyfella_studio_core::paths::user_config_path()
                            .filter(|path| path.exists())
                            .unwrap_or_else(storyfella_studio_core::paths::config_path)
                    }
                    ConfigError::PlatformDirectories => storyfella_studio_core::paths::config_path(),
                };
                rsx! {
                main { class: "min-h-screen bg-slate-950 p-12 font-sans text-red-200",
                    h1 { class: "mb-3 text-2xl font-semibold text-red-100", "Configuration error" }
                    p { role: "alert", class: "max-w-2xl text-sm leading-6 text-red-200/80", "{error}" }
                    div { class: "mt-5 flex gap-4",
                        crate::ui::components::ErrorDetailsButton { error: error.report() }
                        button { onclick: move |_| open_error.set(crate::ui::components::open_path(&repair_path).err()), "Open configuration" }
                        button { onclick: move |_| {
                            let result = AppConfig::load().map(|mut config| { retry_launch.configure_paths(&mut config); config });
                            config_result.set(result);
                        }, "Retry" }
                    }
                    if let Some(message) = open_error() { p { role: "alert", "{message}" } }
                    crate::ui::components::ErrorDetailsHost {}
                }
                }
            },
        }
    }
}
