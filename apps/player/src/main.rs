use clap::Parser;
use dioxus::prelude::*;
use std::path::PathBuf;
use storyfella_playback_ui::{PlaybackModel, PlaybackPanel};
use storyfella_runtime::AssetPolicy;
use storyfella_studio_core::playback::PlaybackSession;

#[derive(Clone, Parser)]
#[command(about = "Play a Storyfella .sf/.db project pair or compiled JSON plan")]
struct Options {
    input: Option<PathBuf>,
    /// Resource database for a compiled JSON plan with pinned artwork.
    #[arg(long)]
    project: Option<PathBuf>,
    /// Play despite missing or unreadable artwork, using black placeholders.
    #[arg(long)]
    allow_missing_assets: bool,
}

fn main() {
    let options = Options::parse();
    let window = dioxus::desktop::WindowBuilder::new()
        .with_title("Storyfella Player")
        .with_inner_size(dioxus::desktop::LogicalSize::new(1280., 720.));
    dioxus::LaunchBuilder::new()
        .with_cfg(dioxus::desktop::Config::new().with_window(window))
        .with_context(options)
        .launch(Player);
}

#[component]
fn Player() -> Element {
    let options = use_context::<Options>();
    let model = use_signal(|| {
        let mut model = PlaybackModel::default();
        let policy = if options.allow_missing_assets {
            AssetPolicy::AllowMissing
        } else {
            AssetPolicy::RequireCompleted
        };
        if let Some(input) = &options.input {
            model.install(PlaybackSession::open_with_policy(
                input,
                options.project.as_deref(),
                policy,
            ));
        } else if options.project.is_some() {
            model.error = Some("--project requires a compiled JSON input".into());
        }
        model
    });
    rsx! {
        style { "html,body,#main {{height:100%;margin:0;background:#121619}}" }
        PlaybackPanel { model, on_open: move |_| {
            spawn(open_story(model, options.allow_missing_assets));
        } }
    }
}

async fn open_story(mut model: Signal<PlaybackModel>, allow_missing_assets: bool) {
    let window = dioxus::desktop::window().window.clone();
    let Some(input) = rfd::AsyncFileDialog::new()
        .set_parent(window.as_ref())
        .add_filter("Storyfella story", &["sf", "db", "json"])
        .pick_file()
        .await
    else {
        return;
    };
    let policy = if allow_missing_assets {
        AssetPolicy::AllowMissing
    } else {
        AssetPolicy::RequireCompleted
    };
    let project = if input.path().extension().is_some_and(|e| e == "json") {
        match PlaybackSession::plan_requires_project_with_policy(input.path(), policy) {
            Ok(false) => None,
            Ok(true) => {
                let Some(file) = rfd::AsyncFileDialog::new()
                    .set_parent(window.as_ref())
                    .set_title("Choose the plan's artwork project")
                    .add_filter("Project database", &["db"])
                    .pick_file()
                    .await
                else {
                    return;
                };
                Some(file.path().to_path_buf())
            }
            Err(error) => {
                model.write().install(Err(error));
                return;
            }
        }
    } else {
        None
    };
    model.write().install(PlaybackSession::open_with_policy(
        input.path(),
        project.as_deref(),
        policy,
    ));
}
