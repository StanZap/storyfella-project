//! Shared single-line fields. Keep native typing/selection through the existing
//! value bridge; callers own values, validation and domain actions.
use dioxus::prelude::*;

use super::{components::IconButton, icons::IconName, text_input::use_text_value};

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum InputSize {
    #[default]
    Regular,
    Large,
}

impl InputSize {
    fn class(self) -> &'static str {
        match self {
            Self::Regular => "studio-input-regular",
            Self::Large => "studio-input-large",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum InputKind {
    #[default]
    Text,
    Search,
}

/// A native text field with one shared surface. Children are the trailing slot
/// used by `TextInputWithAction`, rather than a second input implementation.
#[component]
pub fn TextInput(
    value: String,
    label: String,
    oninput: EventHandler<String>,
    #[props(default)] id: Option<String>,
    #[props(default)] placeholder: String,
    #[props(default)] size: InputSize,
    #[props(default)] disabled: bool,
    #[props(default)] read_only: bool,
    #[props(default)] autofocus: bool,
    #[props(default)] controls: Option<String>,
    #[props(default)] title: Option<String>,
    #[props(default)] kind: InputKind,
    #[props(default)] onkeydown: Option<EventHandler<KeyboardEvent>>,
    #[props(default = rsx! {})] children: Element,
) -> Element {
    let (id, mounted) = use_text_value(value, id.as_deref());
    let focus_id = serde_json::to_string(&id).expect("a string is valid JSON");
    let disabled_class = if disabled { "is-disabled" } else { "" };
    rsx! {
        div { class: "studio-input {size.class()} {disabled_class}",
            onkeydown: move |event| { if let Some(handler) = onkeydown { handler.call(event); } },
            input {
                id, class: "studio-input-element",
                r#type: match kind { InputKind::Text => "text", InputKind::Search => "search" },
                aria_label: label, aria_controls: controls,
                placeholder, title, disabled, readonly: read_only,
                onmounted: move |event| {
                    mounted.call(event);
                    if autofocus { document::eval(&format!("document.getElementById({focus_id})?.focus();")); }
                },
                oninput: move |event| oninput.call(event.value()),
            }
            {children}
        }
    }
}

/// A text input with one named, non-submitting trailing icon action.
#[component]
pub fn TextInputWithAction(
    value: String,
    label: String,
    oninput: EventHandler<String>,
    action_icon: IconName,
    action_label: String,
    onaction: EventHandler<MouseEvent>,
    #[props(default)] id: Option<String>,
    #[props(default)] placeholder: String,
    #[props(default)] size: InputSize,
    #[props(default)] disabled: bool,
    #[props(default)] read_only: bool,
    #[props(default)] autofocus: bool,
    #[props(default)] controls: Option<String>,
    #[props(default)] title: Option<String>,
    #[props(default)] kind: InputKind,
    #[props(default)] action_disabled: bool,
    #[props(default)] action_tooltip: Option<String>,
    #[props(default)] onkeydown: Option<EventHandler<KeyboardEvent>>,
) -> Element {
    rsx! {
        TextInput {
            value, label, oninput, id, placeholder, size, disabled, read_only,
            autofocus, controls, title, kind, onkeydown,
            IconButton {
                icon: action_icon, label: action_label, tooltip: action_tooltip,
                disabled: disabled || action_disabled, onclick: onaction,
            }
        }
    }
}

/// A search field whose × and Escape emit the same cancellation: clear the
/// caller's value, then notify its owner to dismiss filtering or restore focus.
#[component]
pub fn SearchInput(
    value: String,
    label: String,
    oninput: EventHandler<String>,
    oncancel: EventHandler<()>,
    #[props(default)] id: Option<String>,
    #[props(default = "Search…".to_owned())] placeholder: String,
    #[props(default)] size: InputSize,
    #[props(default)] disabled: bool,
    #[props(default)] autofocus: bool,
    #[props(default)] controls: Option<String>,
    #[props(default)] title: Option<String>,
    #[props(default = "Cancel search".to_owned())] cancel_label: String,
) -> Element {
    let cancel = EventHandler::new(move |_| {
        if disabled {
            return;
        }
        oninput.call(String::new());
        oncancel.call(());
    });
    let cancel_tooltip = format!("{cancel_label} (Esc)");
    rsx! {
        TextInputWithAction {
            value, label, oninput, id, placeholder, size, disabled, autofocus,
            controls, title, kind: InputKind::Search,
            action_icon: IconName::Close, action_label: cancel_label,
            action_tooltip: cancel_tooltip,
            onaction: move |_| cancel.call(()),
            onkeydown: Some(EventHandler::new(move |event: KeyboardEvent| {
                if !disabled && event.key() == Key::Escape {
                    event.prevent_default();
                    event.stop_propagation();
                    cancel.call(());
                }
            })),
        }
    }
}
