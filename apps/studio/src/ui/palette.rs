//! The command palette — `Ctrl+K`.
//!
//! A view over the same completion engine the command bar uses, so the
//! vocabulary cannot diverge between them: operations come from
//! [`storyfella_studio_core::registry::slash::OPS`], artifacts from the registry. What the
//! palette adds is *navigation* — jumping straight to an artifact — and a
//! browsable list for the operations, which is the half of the discovery
//! problem a placeholder never solved.
//!
//! Common operations open focused forms; advanced operations prefill the
//! composer. Navigation and history actions act immediately.

use dioxus::prelude::*;
use uuid::Uuid;

use storyfella_studio_core::registry::slash::OPS;

use crate::state::AppState;

use super::components::matches_query;
use super::{
    actions::{ArtAction, StudioAction},
    AppScreen,
};

/// What picking an entry does.
#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    Navigate(AppScreen),
    Task(StudioAction),
    Save,
    Undo,
    Redo,
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
    pub enabled: bool,
}
impl Entry {
    pub fn icon(&self) -> super::icons::IconName {
        use super::icons::IconName;
        match self.action {
            Action::Navigate(destination) => destination.icon(),
            Action::Task(StudioAction::ImportArtwork) => IconName::Upload,
            Action::Task(StudioAction::Art(ArtAction::Modify(_))) => IconName::Edit,
            Action::Task(StudioAction::Art(ArtAction::Regenerate(_))) => IconName::Refresh,
            Action::Task(StudioAction::Art(ArtAction::AttachAdapter(_))) => IconName::Sliders,
            Action::Task(_) => IconName::Add,
            Action::Save => IconName::Save,
            Action::Undo => IconName::Undo,
            Action::Redo => IconName::Redo,
            _ => IconName::Layers,
        }
    }
}

/// Builds the palette's entries for `query`.
///
/// Matching is the shared case-insensitive substring rule over the label and
/// the hint ([`matches_query`]): with a vocabulary this small, fuzzy matching
/// would cost more in surprise than it saves in keystrokes.
pub fn entries(state: &AppState, query: &str) -> Vec<Entry> {
    let mut entries: Vec<Entry> = Vec::new();
    let project_open = state.project_path.is_some();
    let available = project_open && !state.operation_in_progress();
    for destination in AppScreen::ALL {
        entries.push(Entry {
            label: format!("Go to {}", destination.label()),
            hint: "Workspace navigation".into(),
            action: Action::Navigate(destination),
            is_op: false,
            enabled: true,
        });
    }
    for (label, hint, action, enabled) in [
        (
            "New artifact",
            "Create with a visual form",
            Action::Task(StudioAction::Art(ArtAction::Create(
                storyfella_studio_core::registry::ArtifactKind::Character,
            ))),
            available,
        ),
        (
            "Import Atelier artwork",
            "Preview and add approved art",
            Action::Task(StudioAction::ImportArtwork),
            available,
        ),
        (
            "Enter a command",
            "Focus the command entry box",
            Action::Task(StudioAction::FocusCommand),
            project_open,
        ),
        (
            "Save project",
            "Save artwork and description drafts",
            Action::Save,
            available,
        ),
        (
            "Undo",
            "Undo the last project action",
            Action::Undo,
            state.can_undo(),
        ),
        (
            "Redo",
            "Redo the last project action",
            Action::Redo,
            state.can_redo(),
        ),
    ] {
        entries.push(Entry {
            label: label.into(),
            hint: if enabled {
                hint.into()
            } else if !project_open {
                "Open a project first".into()
            } else if matches!(action, Action::Undo | Action::Redo) {
                "No action in history".into()
            } else {
                "Unavailable during the current task".into()
            },
            action,
            is_op: false,
            enabled,
        });
    }
    if let Some(artifact) = state.selected_artifact().filter(|a| a.kind.is_visual()) {
        let id = artifact.id;
        for (label, action) in [
            (
                "New variant",
                ArtAction::Variant(artifact.variant_of.unwrap_or(id)),
            ),
            ("Regenerate image", ArtAction::Regenerate(id)),
            ("Edit image", ArtAction::Modify(id)),
            ("Attach adapter", ArtAction::AttachAdapter(id)),
        ] {
            entries.push(Entry {
                label: label.into(),
                hint: if matches!(action, ArtAction::Modify(_))
                    && state.display_image(artifact).is_none()
                {
                    "Generate an image first".into()
                } else {
                    artifact.name.clone()
                },
                action: Action::Task(StudioAction::Art(action)),
                is_op: false,
                enabled: available
                    && (!matches!(action, ArtAction::Modify(_))
                        || state.display_image(artifact).is_some()),
            });
        }
    }

    for op in OPS {
        entries.push(Entry {
            label: op.name.to_owned(),
            hint: op.summary.to_owned(),
            // The trailing space puts the caret where the first argument
            // goes, so the bar's completion opens immediately.
            action: form_action(state, op.name).unwrap_or_else(|| {
                Action::Compose(if op.params.first() == Some(&"c:<ref>") {
                    state
                        .selected_artifact()
                        .map(|a| {
                            format!(
                                "{} {} ",
                                op.name,
                                storyfella_studio_core::registry::slash::quote_argument(
                                    &state.registry.ref_of(a.id)
                                )
                            )
                        })
                        .unwrap_or_else(|| format!("{} ", op.name))
                } else {
                    format!("{} ", op.name)
                })
            }),
            is_op: true,
            enabled: available,
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
            enabled: true,
        });
    }

    entries.retain(|entry| matches_query(query, &[entry.label.as_str(), entry.hint.as_str()]));
    entries
}

fn form_action(state: &AppState, command: &str) -> Option<Action> {
    let target = state.selected_artifact().filter(|a| a.kind.is_visual());
    let action = match command {
        "/create" => ArtAction::Create(storyfella_studio_core::registry::ArtifactKind::Character),
        "/variant" => {
            let a = target?;
            ArtAction::Variant(a.variant_of.unwrap_or(a.id))
        }
        "/regenerate" => ArtAction::Regenerate(target?.id),
        "/modify" => {
            let a = target?;
            state.display_image(a)?;
            ArtAction::Modify(a.id)
        }
        "/attach-lora" => ArtAction::AttachAdapter(target?.id),
        _ => return None,
    };
    Some(Action::Task(StudioAction::Art(action)))
}

/// Inline action results and an unmet prerequisite for an explicit edit action.
pub struct QuickActions {
    pub entries: Vec<Entry>,
    pub prerequisite: Option<&'static str>,
}

pub fn quick_actions(state: &AppState, query: &str) -> QuickActions {
    let mut entries = quick_entries(state, query);
    let query = query.trim().trim_start_matches('/');
    let prerequisite = if !["edit", "modify", "edit image"]
        .iter()
        .any(|alias| query.eq_ignore_ascii_case(alias))
    {
        None
    } else if state.project_path.is_none() {
        Some("Open a project to edit artwork.")
    } else if state.operation_in_progress() {
        Some("Wait for the current task to finish.")
    } else if let Some(artifact) = state.selected_artifact().filter(|a| a.kind.is_visual()) {
        if state.display_image(artifact).is_some() {
            None
        } else {
            entries.insert(
                0,
                Entry {
                    label: "Generate image first".into(),
                    hint: artifact.name.clone(),
                    action: Action::Task(StudioAction::Art(ArtAction::Regenerate(artifact.id))),
                    is_op: false,
                    enabled: true,
                },
            );
            Some("This artwork has no image to edit yet.")
        }
    } else {
        Some("Select artwork to edit its image.")
    };
    QuickActions {
        entries,
        prerequisite,
    }
}

/// Discoverable actions for the inline entry; raw syntax is opt-in.
fn quick_entries(state: &AppState, query: &str) -> Vec<Entry> {
    let query = query.trim().trim_start_matches('/');
    let mut found = entries(state, "");
    found.retain(|e| {
        if !e.enabled || e.is_op || matches!(e.action, Action::Task(StudioAction::FocusCommand)) {
            return false;
        }
        let alias = match e.action {
            Action::Task(StudioAction::Art(ArtAction::Create(_))) => {
                "new create character object environment story scene beat"
            }
            Action::Task(StudioAction::Art(ArtAction::Variant(_))) => "variant",
            Action::Task(StudioAction::Art(ArtAction::Modify(_))) => "edit modify",
            Action::Task(StudioAction::Art(ArtAction::Regenerate(_))) => "regenerate render",
            Action::Task(StudioAction::Art(ArtAction::AttachAdapter(_))) => "attach-lora adapter",
            _ => "",
        };
        query
            .split_whitespace()
            .all(|word| matches_query(word, &[&e.label, alias]))
    });
    if let Some(kind) = storyfella_studio_core::registry::ArtifactKind::ALL
        .into_iter()
        .find(|kind| {
            query
                .split_whitespace()
                .any(|word| word.eq_ignore_ascii_case(kind.as_str()))
        })
    {
        for entry in &mut found {
            if matches!(
                entry.action,
                Action::Task(StudioAction::Art(ArtAction::Create(_)))
            ) {
                entry.label = format!("New {kind}");
                entry.action = Action::Task(StudioAction::Art(ArtAction::Create(kind)));
            }
        }
    }
    found.sort_by_key(|e| match e.action {
        Action::Task(_) => 0,
        Action::Open(_) => 1,
        Action::Save | Action::Undo | Action::Redo => 2,
        _ => 3,
    });
    found
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
    let actions: Vec<Option<Action>> = found
        .iter()
        .map(|entry| entry.enabled.then(|| entry.action.clone()))
        .collect();
    let active_id = format!("palette-option-{index}");
    let scroll_id = active_id.clone();
    use_effect(use_reactive(&scroll_id, move |id| {
        document::eval(&format!(
            "document.getElementById('{id}')?.scrollIntoView({{block:'nearest'}})"
        ));
    }));

    let mut dismiss = move || {
        open.set(false);
        query.set(String::new());
        selected.set(0);
    };

    rsx! {
        super::dialog::Dialog { title: "Commands and navigation", on_close: move |_| dismiss(),
            div {
                class: "w-[min(560px,90vw)] overflow-hidden rounded-2xl bg-[#17171d] shadow-[0_40px_100px_rgba(0,0,0,.7)] ring-1 ring-white/[0.09]",
                // Keep palette interactions inside the shared native dialog.
                onclick: move |event| event.stop_propagation(),

                div { class: "studio-palette-search flex items-center gap-2.5 border-b border-white/[0.06] px-4 py-2",
                    span { class: "font-mono text-[13px] text-violet-300", ">" }
                    input {
                        aria_label: "Search commands, screens and artifacts",
                        role: "combobox",
                        aria_expanded: "true",
                        aria_controls: "palette-results",
                        aria_autocomplete: "list",
                        aria_activedescendant: if count > 0 { active_id } else { String::new() },
                        class: "studio-search-field min-w-0 flex-1 text-sm text-zinc-100",
                        placeholder: "Search actions, screens, or artifacts…",
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
                                    if let Some(Some(action)) = actions.get(index) {
                                        onpick.call(action.clone());
                                        dismiss();
                                    }
                                }
                                _ => {}
                            }
                        },
                    }
                }

                p { role: "status", class: "px-4 py-2 text-xs text-zinc-300", "{count} results" }
                div { id: "palette-results", role: "listbox", aria_label: "Commands and artifacts", class: "max-h-[330px] overflow-y-auto p-1.5",
                    if found.is_empty() {
                        p { class: "px-3 py-4 text-center text-[11px] text-zinc-600",
                            "Nothing matches."
                        }
                    }
                    for (slot , entry) in found.iter().enumerate() {
                        PaletteRow {
                            key: "{slot}",
                            entry: entry.clone(),
                            id: format!("palette-option-{slot}"),
                            active: slot == index,
                            onpick: move |action: Action| {
                                onpick.call(action);
                                dismiss();
                            },
                        }
                    }
                }

                button { class: "w-full border-t border-white/[0.06] px-4 py-2 text-xs text-zinc-300", onclick: move |_| dismiss(), "Close (Escape)" }
            }
        }
    }
}

#[component]
fn PaletteRow(id: String, entry: Entry, active: bool, onpick: EventHandler<Action>) -> Element {
    let action = entry.action.clone();
    rsx! {
        button {
            id,
            role: "option",
            aria_selected: active,
            tabindex: "-1",
            r#type: "button",
            disabled: !entry.enabled,
            class: if active { "flex w-full cursor-pointer items-center gap-2.5 rounded-lg bg-violet-400/15 px-3 py-2 text-left disabled:opacity-40" } else { "flex w-full cursor-pointer items-center gap-2.5 rounded-lg px-3 py-2 text-left hover:bg-white/[0.04] disabled:opacity-40" },
            onclick: move |_| onpick.call(action.clone()),
            span {
                class: if entry.is_op { "flex size-[18px] shrink-0 items-center justify-center rounded-[5px] bg-violet-400/15 text-[9px] font-bold text-violet-300" } else { "flex size-[18px] shrink-0 items-center justify-center rounded-[5px] bg-teal-400/15 text-[9px] font-bold text-teal-300" },
                if entry.is_op { "/" } else {
                    super::icons::Icon { name: entry.icon(), class: "size-4" }
                }
            }
            span { class: "font-mono text-[12px] text-zinc-100", "{entry.label}" }
            if !entry.hint.is_empty() {
                span { class: "ml-auto max-w-[55%] text-right text-xs text-zinc-400", "{entry.hint}" }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use storyfella_studio_core::registry::{ArtifactKind, ArtifactRegistry, CreateArtifact};

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
        assert_eq!(
            entries.iter().filter(|entry| entry.is_op).count(),
            OPS.len()
        );
    }

    #[test]
    fn quick_actions_open_forms_and_resolve_targets_without_arguments() {
        let mut state = state_with(&[("Captain Mara", ArtifactKind::Character)]);
        state.project_path = Some("quick-actions.svs-project.db".into());
        let id = state.registry.artifacts[0].id;
        state.select_artifact(id);
        assert_eq!(
            quick_entries(&state, "new environment")[0].action,
            Action::Task(StudioAction::Art(ArtAction::Create(
                ArtifactKind::Environment
            )))
        );
        assert_eq!(
            quick_entries(&state, "/variant")[0].action,
            Action::Task(StudioAction::Art(ArtAction::Variant(id)))
        );
        assert_eq!(
            quick_entries(&state, "captain mara")[0].action,
            Action::Open(id)
        );
        assert!(
            quick_entries(&state, "edit").is_empty(),
            "Editing needs a completed image"
        );
        assert_eq!(
            entries(&state, "/create")[0].action,
            Action::Task(StudioAction::Art(ArtAction::Create(
                ArtifactKind::Character
            )))
        );
        assert!(quick_entries(&state, "Give her a blue coat and retain her pose").is_empty());
    }

    #[test]
    fn edit_actions_explain_missing_images_and_offer_generation_for_the_same_target() {
        let mut state = state_with(&[("Harbor door", ArtifactKind::Environment)]);
        state.project_path = Some("edit-prerequisites.svs-project.db".into());
        let id = state.registry.artifacts[0].id;
        state.select_artifact(id);
        for query in ["edit", " EDIT ", "/modify", "edit image"] {
            let result = quick_actions(&state, query);
            assert_eq!(
                result.prerequisite,
                Some("This artwork has no image to edit yet.")
            );
            assert_eq!(result.entries[0].label, "Generate image first");
            assert_eq!(
                result.entries[0].action,
                Action::Task(StudioAction::Art(ArtAction::Regenerate(id)))
            );
        }
        let entry = entries(&state, "Edit image").remove(0);
        assert!(!entry.enabled);
        assert_eq!(entry.hint, "Generate an image first");

        let revision = state
            .registry
            .start_revision(
                id,
                "Harbor door".into(),
                None,
                None,
                storyfella_studio_core::registry::RevisionOrigin::Canon,
            )
            .unwrap();
        state
            .registry
            .finish_revision(id, revision, "assets/harbor-door.png".into())
            .unwrap();
        let result = quick_actions(&state, "edit");
        assert!(result.prerequisite.is_none());
        assert_eq!(
            result.entries[0].action,
            Action::Task(StudioAction::Art(ArtAction::Modify(id)))
        );
        assert!(quick_actions(&state, "Edit the door to be blue")
            .prerequisite
            .is_none());

        state.clear_selection();
        let result = quick_actions(&state, "edit");
        assert_eq!(
            result.prerequisite,
            Some("Select artwork to edit its image.")
        );
        assert!(result.entries.is_empty());
        state.project_path = None;
        assert_eq!(
            quick_actions(&state, "edit").prerequisite,
            Some("Open a project to edit artwork.")
        );
    }

    #[test]
    fn navigation_remains_available_without_a_project_but_mutations_do_not() {
        let found = entries(&AppState::default(), "");
        for screen in AppScreen::ALL {
            assert!(found
                .iter()
                .any(|e| e.action == Action::Navigate(screen) && e.enabled));
        }
        assert!(found
            .iter()
            .filter(|e| matches!(e.action, Action::Compose(_) | Action::Task(_)))
            .all(|e| !e.enabled));
    }

    #[test]
    fn contextual_commands_resolve_spaced_names_and_variants_target_the_base() {
        use storyfella_studio_core::registry::slash::{parse_slash, quote_argument};
        let mut state = state_with(&[
            ("Captain Mara", ArtifactKind::Character),
            ("Mara at night", ArtifactKind::Character),
        ]);
        state.project_path = Some("palette-test.svs-project.db".into());
        let base = state.registry.artifacts[0].id;
        let variant = state.registry.artifacts[1].id;
        state.registry.artifact_mut(variant).unwrap().variant_of = Some(base);
        state.select_artifact(variant);
        let found = entries(&state, "");
        assert!(found.iter().any(|e| e.action
            == Action::Task(StudioAction::Art(ArtAction::Variant(base)))
            && e.enabled));
        let command = found
            .iter()
            .find_map(|e| match &e.action {
                Action::Compose(c) if c.starts_with("/describe ") => Some(c),
                _ => None,
            })
            .unwrap();
        let op = parse_slash(&format!(
            "{command}{}",
            quote_argument("Keeps her captain’s coat — blue.")
        ))
        .unwrap();
        let storyfella_studio_core::registry::ops::Operation::Describe {
            target,
            description,
        } = op
        else {
            panic!("Expected describe")
        };
        assert_eq!(state.registry.resolve(&target).unwrap(), variant);
        assert_eq!(description, "Keeps her captain’s coat — blue.");
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
        use storyfella_studio_core::registry::slash::op_spec;
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
