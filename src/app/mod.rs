mod config;

pub use config::{AppConfig, ConfigError};

use dioxus::prelude::*;

use crate::{state::AppState, ui::Workspace};

const MAIN_CSS: Asset = asset!("/assets/main.css");

#[component]
pub fn App() -> Element {
    let config_result = use_signal(AppConfig::load);
    let app_state = use_signal(AppState::default);

    rsx! {
        document::Title { "Smart Visual Sequencer" }
        document::Stylesheet { href: MAIN_CSS }
        match &*config_result.read() {
            Ok(config) => rsx! { Workspace { config: config.clone(), app_state } },
            Err(error) => rsx! {
                main { class: "error-screen",
                    h1 { "Configuration error" }
                    p { "{error}" }
                }
            },
        }
    }
}
