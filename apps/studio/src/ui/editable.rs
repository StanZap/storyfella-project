//! Display-first fields with explicit confirmation. Callers own domain changes
//! and may retain drafts; both field types share state, actions and shortcuts.
use dioxus::prelude::*;
use serde::Deserialize;

use super::{components::IconButton, icons::IconName, inputs::TextInput};

#[component]
pub fn EditableText(
    label: String,
    value: String,
    onconfirm: Callback<String, Result<(), String>>,
    #[props(default)] disabled: bool,
    #[props(default)] placeholder: String,
) -> Element {
    rsx! { EditableField { label, value, onconfirm, disabled, placeholder } }
}

#[component]
pub fn EditableMultiline(
    label: String,
    value: String,
    onconfirm: Callback<String, Result<(), String>>,
    #[props(default)] disabled: bool,
    #[props(default)] placeholder: String,
    #[props(default = 4)] rows: u32,
    #[props(default)] draft: Option<String>,
    #[props(default)] oninput: Option<EventHandler<String>>,
    #[props(default)] oncancel: Option<EventHandler<()>>,
) -> Element {
    rsx! {
        EditableField {
            label, value, onconfirm, disabled, placeholder, rows,
            draft, oninput, oncancel, multiline: true,
        }
    }
}

#[derive(Deserialize)]
#[serde(tag = "action", content = "value", rename_all = "lowercase")]
enum EditEvent {
    Confirm(String),
    Cancel,
}

fn return_focus(id: &str) {
    document::eval(&format!(
        "requestAnimationFrame(() => document.getElementById('{id}-edit')?.focus());"
    ));
}

#[component]
fn EditableField(
    label: String,
    value: String,
    onconfirm: Callback<String, Result<(), String>>,
    #[props(default)] disabled: bool,
    #[props(default)] placeholder: String,
    #[props(default)] multiline: bool,
    #[props(default = 4)] rows: u32,
    #[props(default)] draft: Option<String>,
    #[props(default)] oninput: Option<EventHandler<String>>,
    #[props(default)] oncancel: Option<EventHandler<()>>,
) -> Element {
    let id = use_hook(|| format!("editable-{}", uuid::Uuid::new_v4()));
    let field_id = format!("{id}-input");
    let mut editing = use_signal(|| draft.is_some());
    let mut requested = use_signal(|| draft.unwrap_or_else(|| value.clone()));
    let mut baseline = use_signal(|| value.clone());
    let mut error = use_signal(|| None::<String>);
    let current = value.clone();
    let cancel_id = id.clone();
    let cancel = use_callback(move |_| {
        if disabled || !*editing.peek() {
            return;
        }
        requested.set(current.clone());
        error.set(None);
        editing.set(false);
        if let Some(handler) = oncancel {
            handler.call(());
        }
        return_focus(&cancel_id);
    });
    let latest = value.clone();
    let confirm_id = id.clone();
    let confirm = use_callback(move |text: String| {
        if disabled || !*editing.peek() {
            return;
        }
        if text != latest && latest != *baseline.peek() {
            error.set(Some("This value changed while you were editing. Discard this draft to edit the current value.".into()));
            return;
        }
        // Keyboard confirmation supplies the actual native value, including
        // keystrokes that have not reached the ordinary input callback yet.
        requested.set(text.clone());
        if let Some(handler) = oninput {
            handler.call(text.clone());
        }
        let result = if text == latest {
            Ok(())
        } else {
            onconfirm.call(text)
        };
        match result {
            Ok(()) => {
                error.set(None);
                editing.set(false);
                return_focus(&confirm_id);
            }
            Err(message) => error.set(Some(message)),
        }
    });
    let bridge_id = id.clone();
    let edit_value = value.clone();
    rsx! {
        div {
            id: id.clone(), class: "studio-editable", role: "group",
            aria_labelledby: "{id}-label",
            onmounted: move |_| {
                let script = format!("const rootId = '{bridge_id}';\n{}", include_str!("../../ui-js/editable.js"));
                let mut bridge = document::eval(&script);
                spawn(async move {
                    while let Ok(event) = bridge.recv::<EditEvent>().await {
                        match event {
                            EditEvent::Confirm(value) => confirm.call(value),
                            EditEvent::Cancel => cancel.call(()),
                        }
                    }
                });
            },
            div { class: "studio-editable-heading",
                span { id: "{id}-label", "{label}" }
                if editing() {
                    IconButton {
                        label: format!("Confirm {label}"), icon: IconName::Check, disabled,
                        tooltip: "Confirm change (Enter)",
                        onclick: move |_| confirm.call(requested()),
                    }
                    IconButton {
                        label: format!("Discard changes to {label}"), icon: IconName::Close, disabled,
                        tooltip: "Discard change (Esc)", onclick: move |_| cancel.call(()),
                    }
                } else {
                    IconButton {
                        id: format!("{id}-edit"),
                        label: format!("Edit {label}"), icon: IconName::Edit, disabled,
                        onclick: move |_| {
                            requested.set(edit_value.clone());
                            baseline.set(edit_value.clone());
                            error.set(None);
                            editing.set(true);
                        },
                    }
                }
            }
            if editing() {
                if multiline {
                    MultilineEditor {
                        id: field_id, rows, disabled, label: label.clone(),
                        invalid: error().is_some(), placeholder, value: requested(),
                        oninput: move |text: String| {
                            requested.set(text.clone());
                            if let Some(handler) = oninput { handler.call(text); }
                        },
                    }
                } else {
                    TextInput {
                        id: field_id, label: label.clone(), value: requested(), disabled,
                        placeholder, autofocus: true,
                        oninput: move |text: String| requested.set(text),
                    }
                }
                p { class: "studio-editable-hint",
                    if multiline { "Enter to confirm · Shift+Enter for a new line · Esc to discard" }
                    else { "Enter to confirm · Esc to discard" }
                }
                if let Some(message) = error() {
                    p { role: "alert", class: "studio-inline-error", "{message}" }
                }
            } else {
                p {
                    class: "studio-editable-value",
                    if value.is_empty() { span { class: "text-zinc-500", if placeholder.is_empty() { "Not set" } else { "{placeholder}" } } }
                    else { "{value}" }
                }
            }
        }
    }
}

#[component]
fn MultilineEditor(
    id: String,
    label: String,
    value: String,
    placeholder: String,
    rows: u32,
    disabled: bool,
    invalid: bool,
    oninput: EventHandler<String>,
) -> Element {
    let (id, mount) = super::text_input::use_text_value(value, Some(&id));
    let focus_id = id.clone();
    rsx! {
        textarea {
            id, rows, class: "studio-textarea", disabled, placeholder,
            aria_label: label, aria_invalid: invalid,
            onmounted: move |event| {
                mount.call(event);
                document::eval(&format!("document.getElementById('{focus_id}')?.focus();"));
            },
            oninput: move |event| oninput.call(event.value()),
        }
    }
}
