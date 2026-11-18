//! Shared, quiet desktop presentation for deterministic story playback.
mod controls;
mod icons;
mod panel;

pub use panel::PlaybackPanel;

#[cfg(test)]
mod tests {
    use super::*;
    use dioxus::prelude::*;
    use storyfella_studio_core::playback::{PlaybackModel, PlaybackSession};

    #[component]
    fn MissingPreviewProbe() -> Element {
        let model = use_signal(|| {
            let mut model = PlaybackModel::default();
            model.install(PlaybackSession::compile_with_policy(
                "scene harbor\n  location: dock\n  cast: mara\n  prop: lantern\n  shot: wide\n  mara: Hello\n  end\n",
                &storyfella_studio_core::registry::ArtifactRegistry::new(),
                storyfella_runtime::AssetPolicy::Preview,
            ));
            model
        });
        rsx! { PlaybackPanel { model, on_apply: move |_| {} } }
    }

    #[component]
    fn StagedPreviewProbe() -> Element {
        let model = use_signal(|| {
            let mut model = PlaybackModel::default();
            model.install(PlaybackSession::compile_with_policy(
                "scene harbor\n  stage: lead, mara, left, 125, 4\n  stage: friend, tam, right, 100, 2, 73, 12\n  narrate: Hello\n",
                &storyfella_studio_core::registry::ArtifactRegistry::new(),
                storyfella_runtime::AssetPolicy::Preview,
            ));
            model
        });
        rsx! { PlaybackPanel { model, on_apply: move |_| {} } }
    }

    #[test]
    fn preview_renders_missing_staged_characters_at_authored_positions() {
        let mut dom = VirtualDom::new(StagedPreviewProbe);
        let rendered = format!("{:?}", dom.rebuild_to_vec());
        for value in [
            "mara · missing artwork",
            "tam · missing artwork",
            "left:20%;bottom:0%;height:90%;z-index:4",
            "left:73%;bottom:12%;height:72%;z-index:2",
        ] {
            assert!(rendered.contains(value), "missing staged output: {value}");
        }
    }

    #[test]
    fn preview_renders_names_for_missing_active_artwork() {
        let mut dom = VirtualDom::new(MissingPreviewProbe);
        let rendered = format!("{:?}", dom.rebuild_to_vec());
        for name in ["dock", "mara", "lantern"] {
            assert!(rendered.contains(name), "missing placeholder for {name}");
        }
    }
}
