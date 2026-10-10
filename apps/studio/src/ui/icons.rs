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
    Book,
    Image,
    Folder,
    Settings,
    Search,
    Workflow,
    Sliders,
    Upload,
    Edit,
    Lock,
    Definition,
    References,
    Format,
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
        IconName::Book => "M12 5v16M12 5C8 2 4 3 3 4v15c3-2 6-1 9 2 3-3 6-4 9-2V4c-3-2-6-1-9 1Z",
        IconName::Image => "M4 3h16a1 1 0 0 1 1 1v16a1 1 0 0 1-1 1H4a1 1 0 0 1-1-1V4a1 1 0 0 1 1-1Zm-1 14 5-5 4 4 3-3 6 6M7 7h.01",
        IconName::Folder => "M3 7V5a2 2 0 0 1 2-2h5l2 3h7a2 2 0 0 1 2 2v11a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V7Z",
        IconName::Settings => "M9 4 10 2h4l1 2 3 2 2 1v4l-2 1-2 3v3l-3 2-2-1-3-1-2-2H3v-4l2-2 1-3V5l3-1ZM12 8a4 4 0 1 0 0 8 4 4 0 0 0 0-8Z",
        IconName::Search => "M21 21l-5-5M10 3a7 7 0 1 0 0 14 7 7 0 0 0 0-14Z",
        IconName::Workflow => "M4 3h6v6H4V3Zm10 12h6v6h-6v-6ZM7 9v9h7M10 6h7v9",
        IconName::Sliders => "M4 7h7m4 0h5M4 17h2m4 0h10M11 4v6M6 14v6",
        IconName::Upload => "M12 16V3m-5 5 5-5 5 5M4 16v5h16v-5",
        IconName::Edit => "m4 16-1 5 5-1L20 8l-4-4L4 16Zm10-10 4 4",
        IconName::Lock => "M6 10h12v11H6V10Zm3 0V6a3 3 0 0 1 6 0v4M12 14v3",
        IconName::Definition => "M14 3h7v7M21 3 10 14M10 3H3v18h18v-7",
        IconName::References => "M8 12h8M9 7H7a5 5 0 0 0 0 10h2M15 7h2a5 5 0 0 1 0 10h-2",
        IconName::Format => "M8 5h13M8 10h9M8 15h13M8 20h9M3 5h1M3 10h1M3 15h1M3 20h1",
    };

    rsx! {
        svg {
            "aria-hidden": "true",
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
