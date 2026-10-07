use dioxus::prelude::*;

#[derive(Clone, Copy, PartialEq)]
pub(crate) enum Symbol {
    Arrow,
    Pause,
    Play,
    More,
    Close,
}

#[component]
pub(crate) fn Icon(symbol: Symbol) -> Element {
    let path = match symbol {
        Symbol::Arrow => "M4 12h16m-6-6 6 6-6 6",
        Symbol::Pause => "M8 5v14M16 5v14",
        Symbol::Play => "m7 5 11 7-11 7V5Z",
        Symbol::More => "M5 12h.01M12 12h.01M19 12h.01",
        Symbol::Close => "m6 6 12 12M18 6 6 18",
    };
    rsx! {
        svg { view_box: "0 0 24 24", fill: "none", stroke: "currentColor",
            stroke_width: "1.5", stroke_linecap: "round", stroke_linejoin: "round",
            "aria-hidden": "true", path { d: path }
        }
    }
}
