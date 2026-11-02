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
    let mut theme = use_signal(|| {
        config_result
            .peek()
            .as_ref()
            .map(|config| config.theme)
            .unwrap_or_default()
    });
    use_effect(move || {
        if let Ok(config) = &*config_result.read() {
            theme.set(config.theme);
        }
    });
    use_context_provider(|| theme);
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
        crate::ui::theme::ThemeHost { preference: theme() }
        match &*config_result.read() {
            Ok(config) => rsx! { Workspace { config: config.clone(), app_state, plan_launch } },
            Err(error) => {
                let repair_path: Option<std::path::PathBuf> = match error {
                    ConfigError::Read { path, .. } | ConfigError::Parse { path, .. }
                    | ConfigError::Write { path, .. } | ConfigError::InvalidSize { path, .. }
                    | ConfigError::MalformedSize { path, .. } => Some(path.clone()),
                    ConfigError::Providers(_) | ConfigError::NativeImageUnavailable | ConfigError::Serialize { .. } => {
                        storyfella_studio_core::paths::clipforge_config_path()
                            .filter(|path| path.exists())
                            .or_else(storyfella_studio_core::paths::config_path)
                    }
                    ConfigError::PlatformDirectories | ConfigError::Identity(_) => None,
                };
                rsx! {
                main { class: "studio-startup",
                  div { class: "studio-startup-content",
                    p { class: "studio-startup-brand", "Storyfella Studio" }
                    h1 { class: "studio-startup-title", if matches!(error, ConfigError::Identity(_)) { "Studio couldn't migrate its data" } else { "Studio couldn't open its settings" } }
                    p { class: "studio-startup-message", if matches!(error, ConfigError::Identity(_)) { "Your original data is preserved. Review the details, correct the problem, then try again." } else { "Your stories are safe. Check the configuration file, then try again. Technical details are available below." } }
                    crate::ui::components::RecoveryNotice { title: error.code().message(), message: error.code().hint(), alert: true,
                        if let Some(repair_path) = repair_path {
                            button { class: "studio-button studio-button-primary", onclick: move |_| open_error.set(crate::ui::components::open_path(&repair_path).err()), "Open configuration" }
                        }
                        button { class: "studio-button", onclick: move |_| {
                            let result = AppConfig::load().map(|mut config| { retry_launch.configure_paths(&mut config); config });
                            config_result.set(result);
                        }, "Try again" }
                        crate::ui::components::ErrorDetailsButton { error: error.report() }
                    }
                    if let Some(message) = open_error() { p { role: "alert", class: "mt-4 text-sm text-zinc-400", "{message}" } }
                    crate::ui::components::ErrorDetailsHost {}
                  }
                }
                }
            },
        }
    }
}
