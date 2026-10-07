use clap::Parser;
use dioxus::prelude::*;
use std::path::PathBuf;
use storyfella_playback_ui::{PlaybackModel, PlaybackPanel};
use storyfella_studio_core::playback::PlaybackSession;

#[derive(Clone, Parser)]
#[command(about = "Play a Storyfella .sf/.db project pair or compiled JSON plan")]
struct Options {
    input: Option<PathBuf>,
    /// Resource database for a compiled JSON plan with pinned artwork.
    #[arg(long)]
    project: Option<PathBuf>,
}

fn main() {
    let options = Options::parse();
    let window = dioxus::desktop::WindowBuilder::new()
        .with_title("Storyfella Player")
        .with_inner_size(dioxus::desktop::LogicalSize::new(1100., 850.));
    dioxus::LaunchBuilder::new()
        .with_cfg(dioxus::desktop::Config::new().with_window(window))
        .with_context(options)
        .launch(Player);
}

#[component]
fn Player() -> Element {
    let options = use_context::<Options>();
    let mut model = use_signal(|| {
        let mut model = PlaybackModel::default();
        if let Some(input) = &options.input {
            model.install(PlaybackSession::open(input, options.project.as_deref()));
        } else if options.project.is_some() {
            model.error = Some("--project requires a compiled JSON input".into());
        }
        model
    });
    rsx! {
        style { "html,body,#main {{height:100%;margin:0;background:#13131a}}" }
        div { style: "height:100%;display:flex;flex-direction:column",
            header { style: "padding:12px 24px;color:#ece7ff;font:15px system-ui;display:flex;gap:15px;align-items:center",
                strong { "Storyfella Player" }
                button { onclick: move |_| {
                    let Some(input) = rfd::FileDialog::new().add_filter("Storyfella story", &["sf", "db", "json"]).pick_file() else { return; };
                    let project = if input.extension().is_some_and(|e| e == "json") {
                        match PlaybackSession::plan_requires_project(&input) {
                            Ok(false) => None,
                            Ok(true) => {
                                let Some(path) = rfd::FileDialog::new().set_title("Choose the plan's artwork project").add_filter("Project database", &["db"]).pick_file() else { return; };
                                Some(path)
                            },
                            Err(error) => { model.write().install(Err(error)); return; }
                        }
                    } else { None };
                    model.write().install(PlaybackSession::open(&input, project.as_deref()));
                }, "Open story…" }
            }
            div { style: "flex:1;min-height:0", PlaybackPanel { model } }
        }
    }
}
