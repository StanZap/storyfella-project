//! Preserve native typing while synchronizing explicit changes from app state.
use dioxus::prelude::*;

/// Use the returned id and mount handler instead of a controlled `value`.
/// A delayed render must not overwrite text that the user has already typed.
pub fn use_text_value(value: String, id: Option<&str>) -> (String, EventHandler<MountedEvent>) {
    let requested_id = id.map(str::to_owned);
    let id = use_hook(|| requested_id.unwrap_or_else(|| format!("text-{}", uuid::Uuid::new_v4())));
    let effect_id = serde_json::to_string(&id).expect("a string is valid JSON");
    let mut previous = use_signal(|| value.clone());
    use_effect(use_reactive(&value, move |next| {
        let old = previous.peek().clone();
        previous.set(next.clone());
        document::eval(&format!(
            "const el = document.getElementById({effect_id}); if (el && el.value === {}) el.value = {};",
            serde_json::to_string(&old).unwrap(), serde_json::to_string(&next).unwrap()
        ));
    }));
    let mount_id = serde_json::to_string(&id).expect("a string is valid JSON");
    let mounted = EventHandler::new(move |_| {
        document::eval(&format!(
            "const el = document.getElementById({mount_id}); if (el) el.value = {};",
            serde_json::to_string(&value).unwrap()
        ));
    });
    (id, mounted)
}
