use dioxus::prelude::*;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum IconName {
    Add,
    Arrow,
    Canvas,
    Generate,
    Grid,
    Home,
    Layers,
    More,
    Play,
    Refresh,
    Settings,
    Sparkles,
    Timeline,
}

#[component]
pub fn Icon(name: IconName, #[props(default = "size-5".to_owned())] class: String) -> Element {
    let path = match name {
        IconName::Add => "M12 5v14M5 12h14",
        IconName::Arrow => "m9 18 6-6-6-6",
        IconName::Canvas => "M4 5.5A1.5 1.5 0 0 1 5.5 4h13A1.5 1.5 0 0 1 20 5.5v13a1.5 1.5 0 0 1-1.5 1.5h-13A1.5 1.5 0 0 1 4 18.5v-13ZM4 16l4.5-4.5 3 3 2-2L20 19M15.5 9a1.5 1.5 0 1 0 0-3 1.5 1.5 0 0 0 0 3Z",
        IconName::Generate => "M12 3v3m0 12v3M3 12h3m12 0h3M5.64 5.64l2.12 2.12m8.48 8.48 2.12 2.12m0-12.72-2.12 2.12M7.76 16.24l-2.12 2.12M12 8.5a3.5 3.5 0 1 0 0 7 3.5 3.5 0 0 0 0-7Z",
        IconName::Grid => "M4 4h6v6H4V4Zm10 0h6v6h-6V4ZM4 14h6v6H4v-6Zm10 0h6v6h-6v-6Z",
        IconName::Home => "m3 11 9-8 9 8v9a1 1 0 0 1-1 1h-5v-7H9v7H4a1 1 0 0 1-1-1v-9Z",
        IconName::Layers => "m12 3 9 5-9 5-9-5 9-5Zm-9 10 9 5 9-5M3 17l9 5 9-5",
        IconName::More => "M5 12h.01M12 12h.01M19 12h.01",
        IconName::Play => "m9 7 8 5-8 5V7Z",
        IconName::Refresh => "M21 12a9 9 0 1 1-2.64-6.36M21 3v6h-6",
        IconName::Settings => "M12 15.5a3.5 3.5 0 1 0 0-7 3.5 3.5 0 0 0 0 7ZM19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06-2.83 2.83-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21h-4v-.09a1.65 1.65 0 0 0-1.08-1.5 1.65 1.65 0 0 0-1.82.33l-.06.06-2.83-2.83.06-.06A1.65 1.65 0 0 0 4.6 15a1.65 1.65 0 0 0-1.51-1H3v-4h.09A1.65 1.65 0 0 0 4.6 9a1.65 1.65 0 0 0-.33-1.82l-.06-.06 2.83-2.83.06.06A1.65 1.65 0 0 0 9 4.6a1.65 1.65 0 0 0 1-1.51V3h4v.09A1.65 1.65 0 0 0 15 4.6a1.65 1.65 0 0 0 1.82-.33l.06-.06 2.83 2.83-.06.06A1.65 1.65 0 0 0 19.4 9c.12.61.72 1 1.34 1H21v4h-.09c-.62 0-1.39.39-1.51 1Z",
        IconName::Sparkles => "m12 3 1.2 3.8L17 8l-3.8 1.2L12 13l-1.2-3.8L7 8l3.8-1.2L12 3Zm6 10 .8 2.2L21 16l-2.2.8L18 19l-.8-2.2L15 16l2.2-.8L18 13ZM6 14l.9 2.1L9 17l-2.1.9L6 20l-.9-2.1L3 17l2.1-.9L6 14Z",
        IconName::Timeline => "M4 7h7M4 12h16M13 17h7M11 5v4M13 15v4",
    };

    rsx! {
        svg {
            class: "{class}",
            view_box: "0 0 24 24",
            fill: "none",
            stroke: "currentColor",
            stroke_width: "1.7",
            stroke_linecap: "round",
            stroke_linejoin: "round",
            path { d: path }
        }
    }
}
