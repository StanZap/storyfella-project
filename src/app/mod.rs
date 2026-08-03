mod config;

pub use config::{AppConfig, ConfigError};

use dioxus::prelude::*;

use crate::{state::AppState, ui::Workspace};

const TAILWIND_CSS: Asset = asset!("/assets/tailwind.css");

#[component]
pub fn App() -> Element {
    let config_result = use_signal(AppConfig::load);
    let app_state = use_signal(AppState::default);

    rsx! {
        document::Title { "Smart Visual Sequencer" }
        document::Stylesheet { href: TAILWIND_CSS }
        match &*config_result.read() {
            Ok(config) => rsx! { Workspace { config: config.clone(), app_state } },
            Err(error) => rsx! {
                main { class: "min-h-screen bg-slate-950 p-12 font-sans text-red-200",
                    h1 { class: "mb-3 text-2xl font-semibold text-red-100", "Configuration error" }
                    p { class: "max-w-2xl text-sm leading-6 text-red-200/80", "{error}" }
                }
            },
        }
    }
}
