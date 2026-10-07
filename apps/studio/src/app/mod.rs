pub mod sample;

use dioxus::prelude::*;

use storyfella_studio_core::config::AppConfig;

use crate::{driver::PlanLaunch, state::AppState, ui::Workspace};

const TAILWIND_CSS: Asset = asset!("/assets/tailwind.css");

#[component]
pub fn App() -> Element {
    // The launch context (set by main.rs from `--plan`/`--mode` args);
    // defaults to an empty launch when the app is started normally.
    let plan_launch = use_context::<PlanLaunch>();
    let config_launch = plan_launch.clone();
    let config_result = use_signal(move || {
        AppConfig::load().map(|mut config| {
            config_launch.configure_paths(&mut config);
            config
        })
    });
    let app_state = use_signal(AppState::default);

    rsx! {
        document::Title { "Storyfella Studio" }
        document::Stylesheet { href: TAILWIND_CSS }
        match &*config_result.read() {
            Ok(config) => rsx! { Workspace { config: config.clone(), app_state, plan_launch } },
            Err(error) => rsx! {
                main { class: "min-h-screen bg-slate-950 p-12 font-sans text-red-200",
                    h1 { class: "mb-3 text-2xl font-semibold text-red-100", "Configuration error" }
                    p { class: "max-w-2xl text-sm leading-6 text-red-200/80", "{error}" }
                }
            },
        }
    }
}
