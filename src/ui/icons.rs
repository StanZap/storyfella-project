use dioxus::prelude::*;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum IconName {
    Add,
    Arrow,
    Check,
    Close,
    Copy,
    Generate,
    Layers,
    List,
    Pause,
    Play,
    Redo,
    Refresh,
    Save,
    Sparkles,
    Undo,
}

#[component]
pub fn Icon(name: IconName, #[props(default = "size-5".to_owned())] class: String) -> Element {
    let path = match name {
        IconName::Add => "M12 5v14M5 12h14",
        IconName::Arrow => "m9 18 6-6-6-6",
        IconName::Check => "m4 12.5 5 5L20 6.5",
        IconName::Close => "M6 6l12 12M18 6 6 18",
        IconName::Copy => "M8 8h12v12H8V8ZM4 16V4h12",
        IconName::Generate => "M12 3v3m0 12v3M3 12h3m12 0h3M5.64 5.64l2.12 2.12m8.48 8.48 2.12 2.12m0-12.72-2.12 2.12M7.76 16.24l-2.12 2.12M12 8.5a3.5 3.5 0 1 0 0 7 3.5 3.5 0 0 0 0-7Z",
        IconName::Layers => "m12 3 9 5-9 5-9-5 9-5Zm-9 10 9 5 9-5M3 17l9 5 9-5",
        IconName::List => "M4 6h16M4 12h16M4 18h10",
        IconName::Pause => "M7 5v14M17 5v14",
        IconName::Play => "M6 5.5v13l11-6.5-11-6.5Z",
        IconName::Redo => "m15 14 5-5-5-5M20 9H10a6 6 0 0 0 0 12h3",
        IconName::Save => "M5 3h11l5 5v11a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2Zm4 0v6h6V3M6.5 21v-6h11v6",
        IconName::Refresh => "M21 12a9 9 0 1 1-2.64-6.36M21 3v6h-6",
        IconName::Sparkles => "m12 3 1.2 3.8L17 8l-3.8 1.2L12 13l-1.2-3.8L7 8l3.8-1.2L12 3Zm6 10 .8 2.2L21 16l-2.2.8L18 19l-.8-2.2L15 16l2.2-.8L18 13ZM6 14l.9 2.1L9 17l-2.1.9L6 20l-.9-2.1L3 17l2.1-.9L6 14Z",
        IconName::Undo => "M9 14 4 9l5-5M4 9h10a6 6 0 0 1 0 12h-3",
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
