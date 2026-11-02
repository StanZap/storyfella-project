//! A display-only matte shared by saved-artwork previews. Never flatten alpha
//! or write this preference to an asset or compiled playback resource.
use dioxus::prelude::*;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum PreviewBackground {
    #[default]
    Light,
    Dark,
    Checkerboard,
}

impl PreviewBackground {
    const ALL: [Self; 3] = [Self::Light, Self::Dark, Self::Checkerboard];

    fn label(self) -> &'static str {
        match self {
            Self::Light => "Light",
            Self::Dark => "Dark",
            Self::Checkerboard => "Checkerboard",
        }
    }

    fn class(self) -> &'static str {
        match self {
            Self::Light => "studio-artwork-light",
            Self::Dark => "studio-artwork-dark",
            Self::Checkerboard => "studio-artwork-checkerboard",
        }
    }
}

#[component]
pub fn ArtworkImage(src: String, alt: String, class: String) -> Element {
    let background = use_context::<Signal<PreviewBackground>>();
    rsx! { img { src, alt, class: "studio-artwork {background().class()} {class}" } }
}

#[component]
pub fn PreviewBackgroundPicker() -> Element {
    let mut background = use_context::<Signal<PreviewBackground>>();
    rsx! {
        label { class: "flex items-center gap-2 text-xs text-zinc-400",
            "Background"
            select {
                class: "setting-control w-auto", aria_label: "Artwork preview background",
                title: "Preview background only; artwork transparency is preserved",
                oninput: move |event| {
                    if let Some(next) = PreviewBackground::ALL.into_iter()
                        .find(|item| item.label() == event.value()) {
                        background.set(next);
                    }
                },
                for item in PreviewBackground::ALL {
                    option { value: item.label(), selected: background() == item, "{item.label()}" }
                }
            }
        }
    }
}
