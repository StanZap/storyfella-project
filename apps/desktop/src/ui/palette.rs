//! The command palette — `Ctrl+K`.
//!
//! A view over the same completion engine the command bar uses, so the
//! vocabulary cannot diverge between them: operations come from
//! [`svs_core::registry::slash::OPS`], artifacts from the registry. What the
//! palette adds is *navigation* — jumping straight to an artifact — and a
//! browsable list for the operations, which is the half of the discovery
//! problem a placeholder never solved.
//!
//! Picking an operation does not run it. Operations need arguments, so the
//! palette prefills the composer and hands the caret over; only navigation
//! acts immediately.

use dioxus::prelude::*;
use uuid::Uuid;

use svs_core::registry::slash::OPS;

use crate::state::AppState;

use super::components::matches_query;

/// What picking an entry does.
#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    /// Prefill the composer with a command that still needs arguments.
    Compose(String),
    /// Open an artifact.
    Open(Uuid),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    pub label: String,
    pub hint: String,
    pub action: Action,
    /// `true` for operations, `false` for artifacts — drives the badge.
    pub is_op: bool,
}

/// Builds the palette's entries for `query`.
///
/// Matching is the shared case-insensitive substring rule over the label and
/// the hint ([`matches_query`]): with a vocabulary this small, fuzzy matching
/// would cost more in surprise than it saves in keystrokes.
pub fn entries(state: &AppState, query: &str) -> Vec<Entry> {
    let mut entries: Vec<Entry> = Vec::new();

    for op in OPS {
        entries.push(Entry {
            label: op.name.to_owned(),
            hint: op.summary.to_owned(),
            // The trailing space puts the caret where the first argument
            // goes, so the bar's completion opens immediately.
            action: Action::Compose(format!("{} ", op.name)),
            is_op: true,
        });
    }

    for artifact in &state.registry.artifacts {
        let name = if artifact.name.trim().is_empty() {
            "Unnamed".to_owned()
        } else {
            artifact.name.clone()
        };
        entries.push(Entry {
            label: format!("open {name}"),
            hint: artifact.kind.as_str().to_owned(),
            action: Action::Open(artifact.id),
            is_op: false,
        });
    }

    entries.retain(|entry| matches_query(query, &[entry.label.as_str(), entry.hint.as_str()]));
    entries
}

#[component]
pub fn Palette(
    app_state: Signal<AppState>,
    /// Set to false to dismiss.
    open: Signal<bool>,
    onpick: EventHandler<Action>,
) -> Element {
    let mut query = use_signal(String::new);
    let mut selected = use_signal(|| 0usize);

    let found = entries(&app_state.read(), &query.read());
    let count = found.len();
    let index = (*selected.read()).min(count.saturating_sub(1));
    let actions: Vec<Action> = found.iter().map(|entry| entry.action.clone()).collect();

    let mut dismiss = move || {
        open.set(false);
        query.set(String::new());
        selected.set(0);
    };

    rsx! {
        div {
            class: "fixed inset-0 z-50 flex items-start justify-center bg-black/55 pt-[13vh]",
            onclick: move |_| dismiss(),
            div {
                class: "w-[min(560px,90vw)] overflow-hidden rounded-2xl bg-[#17171d] shadow-[0_40px_100px_rgba(0,0,0,.7)] ring-1 ring-white/[0.09]",
                // The backdrop closes on click; the panel must not.
                onclick: move |event| event.stop_propagation(),

                div { class: "flex items-center gap-2.5 border-b border-white/[0.06] px-4 py-3",
                    span { class: "font-mono text-[13px] text-violet-300", ">" }
                    input {
                        class: "flex-1 bg-transparent font-mono text-[13px] text-zinc-100 outline-none placeholder:text-zinc-700",
                        placeholder: "Search commands and artifacts…",
                        value: "{query}",
                        autofocus: true,
                        autocomplete: "off",
                        spellcheck: "false",
                        oninput: move |event| {
                            query.set(event.value());
                            selected.set(0);
                        },
                        onkeydown: move |event| {
                            match event.key() {
                                Key::Escape => {
                                    event.prevent_default();
                                    dismiss();
                                }
                                Key::ArrowDown => {
                                    event.prevent_default();
                                    selected.set(index.saturating_add(1).min(count.saturating_sub(1)));
                                }
                                Key::ArrowUp => {
                                    event.prevent_default();
                                    selected.set(index.saturating_sub(1));
                                }
                                Key::Enter => {
                                    event.prevent_default();
                                    if let Some(action) = actions.get(index) {
                                        onpick.call(action.clone());
                                        dismiss();
                                    }
                                }
                                _ => {}
                            }
                        },
                    }
                }

                div { class: "max-h-[330px] overflow-y-auto p-1.5",
                    if found.is_empty() {
                        p { class: "px-3 py-4 text-center text-[11px] text-zinc-600",
                            "Nothing matches."
                        }
                    }
                    for (slot , entry) in found.iter().enumerate() {
                        PaletteRow {
                            key: "{entry.label}",
                            entry: entry.clone(),
                            active: slot == index,
                            onpick: move |action: Action| {
                                onpick.call(action);
                                dismiss();
                            },
                        }
                    }
                }

                p { class: "border-t border-white/[0.06] bg-white/[0.015] px-4 py-2 text-[10.5px] text-zinc-600",
                    "The same vocabulary as the command bar and the "
                    span { class: "font-mono", "svs" }
                    " CLI."
                }
            }
        }
    }
}

#[component]
fn PaletteRow(entry: Entry, active: bool, onpick: EventHandler<Action>) -> Element {
    let action = entry.action.clone();
    rsx! {
        div {
            class: if active { "flex cursor-pointer items-center gap-2.5 rounded-lg bg-violet-400/15 px-3 py-2" } else { "flex cursor-pointer items-center gap-2.5 rounded-lg px-3 py-2 hover:bg-white/[0.04]" },
            onclick: move |_| onpick.call(action.clone()),
            span {
                class: if entry.is_op { "flex size-[18px] shrink-0 items-center justify-center rounded-[5px] bg-violet-400/15 text-[9px] font-bold text-violet-300" } else { "flex size-[18px] shrink-0 items-center justify-center rounded-[5px] bg-teal-400/15 text-[9px] font-bold text-teal-300" },
                if entry.is_op { "/" } else { "c" }
            }
            span { class: "font-mono text-[12px] text-zinc-100", "{entry.label}" }
            if !entry.hint.is_empty() {
                span { class: "ml-auto text-[11px] text-zinc-500", "{entry.hint}" }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use svs_core::registry::{ArtifactKind, ArtifactRegistry, CreateArtifact};

    fn state_with(names: &[(&str, ArtifactKind)]) -> AppState {
        let mut registry = ArtifactRegistry::new();
        for (name, kind) in names {
            registry
                .create_artifact(
                    *kind,
                    CreateArtifact {
                        name: (*name).into(),
                        description: "a test artifact".into(),
                        ..Default::default()
                    },
                )
                .expect("artifact should create");
        }
        let mut state = AppState::default();
        state.registry = registry;
        state
    }

    #[test]
    fn every_operation_is_offered() {
        let entries = entries(&state_with(&[]), "");
        assert_eq!(entries.len(), OPS.len());
        assert!(entries.iter().all(|entry| entry.is_op));
    }

    #[test]
    fn artifacts_are_offered_for_navigation() {
        let state = state_with(&[("mara", ArtifactKind::Character)]);
        let entries = entries(&state, "mara");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].label, "open mara");
        assert!(matches!(entries[0].action, Action::Open(_)));
    }

    #[test]
    fn matching_covers_the_hint_too() {
        // "reroll" appears only in /regenerate's summary.
        let entries = entries(&state_with(&[]), "reroll");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].label, "/regenerate");
    }

    #[test]
    fn operations_prefill_rather_than_run() {
        // They need arguments, so picking one hands the caret to the bar.
        let entries = entries(&state_with(&[]), "/modify");
        assert_eq!(
            entries[0].action,
            Action::Compose("/modify ".to_owned()),
            "the trailing space is what opens the bar's completion"
        );
    }

    #[test]
    fn every_composed_command_is_a_real_operation() {
        use svs_core::registry::slash::op_spec;
        for entry in entries(&state_with(&[]), "") {
            let Action::Compose(command) = &entry.action else {
                continue;
            };
            assert!(
                op_spec(command.trim()).is_some(),
                "palette offers {command:?}, which is not in the vocabulary"
            );
        }
    }

    #[test]
    fn a_query_matching_nothing_returns_nothing() {
        assert!(entries(&state_with(&[]), "zzzznope").is_empty());
    }
}
