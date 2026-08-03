use dioxus::prelude::*;

use crate::{app::AppConfig, state::AppState};

#[component]
pub fn Workspace(config: AppConfig, app_state: Signal<AppState>) -> Element {
    let project_name = app_state.read().project_name.clone();

    rsx! {
        main { class: "workspace",
            header { class: "topbar",
                div {
                    p { class: "eyebrow", "SMART VISUAL SEQUENCER" }
                    h1 { "{project_name}" }
                }
                span { class: "status", "Bootstrap ready" }
            }
            section { class: "canvas",
                div { class: "empty-state",
                    h2 { "Your visual workspace starts here" }
                    p { "The desktop shell, runtime boundary, and API clients are ready. AI pipeline orchestration comes next." }
                }
            }
            aside { class: "inspector",
                h2 { "Runtime" }
                dl {
                    dt { "LM Studio" }
                    dd { "{config.lm_studio.base_url}" }
                    dt { "Python" }
                    dd { "{config.python_runtime.display()}" }
                    dt { "Models" }
                    dd { "{config.model_dir.display()}" }
                }
            }
            footer { class: "timeline",
                strong { "Timeline" }
                span { "No clips yet" }
            }
        }
    }
}
