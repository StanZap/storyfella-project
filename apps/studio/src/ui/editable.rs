//! Display-first fields with explicit confirmation. Callers own domain changes
//! and may retain drafts; both field types share state, actions and shortcuts.
use dioxus::prelude::*;
use serde::Deserialize;

use super::{
    components::IconButton,
    icons::IconName,
    inputs::{TextInput, TextInputWithAction},
};

#[component]
pub fn EditableText(
    label: String,
    value: String,
    onconfirm: Callback<String, Result<(), String>>,
    #[props(default)] disabled: bool,
    #[props(default)] placeholder: String,
    #[props(default)] edit_requested: bool,
    #[props(default)] on_edit_started: Option<EventHandler<()>>,
) -> Element {
    rsx! { EditableField { label, value, onconfirm, disabled, placeholder, edit_requested, on_edit_started } }
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
    #[props(default)] edit_requested: bool,
    #[props(default)] on_edit_started: Option<EventHandler<()>>,
) -> Element {
    let id = use_hook(|| format!("editable-{}", uuid::Uuid::new_v4()));
    let field_id = format!("{id}-input");
    let mut editing = use_signal(|| draft.is_some());
    let mut requested = use_signal(|| draft.unwrap_or_else(|| value.clone()));
    let mut baseline = use_signal(|| value.clone());
    let mut error = use_signal(|| None::<String>);
    let requested_value = value.clone();
    let requested_field_id = field_id.clone();
    use_effect(use_reactive((&edit_requested,), move |(requested_edit,)| {
        if requested_edit && !disabled {
            if !*editing.peek() {
                requested.set(requested_value.clone());
                baseline.set(requested_value.clone());
                error.set(None);
                editing.set(true);
            } else {
                let field = serde_json::to_string(&requested_field_id).expect("serialize field id");
                document::eval(&format!(
                    "requestAnimationFrame(() => document.getElementById({field})?.focus());"
                ));
            }
            if let Some(handler) = on_edit_started {
                handler.call(());
            }
        }
    }));
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
    let start_editing = use_callback(move |_| {
        requested.set(edit_value.clone());
        baseline.set(edit_value.clone());
        error.set(None);
        editing.set(true);
    });
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
                        tooltip: format!("Confirm {label} (Enter)"),
                        onclick: move |_| confirm.call(requested()),
                    }
                    IconButton {
                        label: format!("Discard changes to {label}"), icon: IconName::Close, disabled,
                        tooltip: format!("Discard changes to {label} (Esc)"), onclick: move |_| cancel.call(()),
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
            } else if multiline {
                div { class: "studio-editable-display",
                    p { class: "studio-editable-value",
                        if value.is_empty() { span { class: "text-zinc-500", if placeholder.is_empty() { "Not set" } else { "{placeholder}" } } }
                        else { "{value}" }
                    }
                    IconButton { id: format!("{id}-edit"), label: format!("Edit {label}"), icon: IconName::Edit, disabled, onclick: move |_| start_editing.call(()) }
                }
            } else {
                TextInputWithAction {
                    value, label: label.clone(), disabled, read_only: true,
                    placeholder: if placeholder.is_empty() { "Not set".to_owned() } else { placeholder },
                    action_id: format!("{id}-edit"), action_label: format!("Edit {label}"), action_icon: IconName::Edit,
                    oninput: move |text| requested.set(text),
                    onaction: move |_| start_editing.call(()),
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};

    static STARTED: AtomicBool = AtomicBool::new(false);

    #[component]
    fn RequestedEditProbe() -> Element {
        rsx! { EditableText {
            label: "Reference name", value: "harbor_dock", edit_requested: true,
            onconfirm: move |_: String| Ok(()),
            on_edit_started: move |_| STARTED.store(true, Ordering::SeqCst),
        } }
    }

    #[tokio::test]
    async fn requested_edit_opens_reference_name_editor() {
        STARTED.store(false, Ordering::SeqCst);
        let mut dom = VirtualDom::new(RequestedEditProbe);
        dom.rebuild_in_place();
        tokio::time::timeout(std::time::Duration::from_secs(1), dom.wait_for_work())
            .await
            .unwrap();
        let rendered = format!("{:?}", dom.render_immediate_to_vec());
        assert!(STARTED.load(Ordering::SeqCst));
        assert!(rendered.contains("Confirm Reference name"));
    }
}
