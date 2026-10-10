//! Visual forms compile to the same typed operations as slash commands.
use super::{actions::ArtAction, components::IconButton, dialog::Dialog, icons::IconName};
use crate::state::AppState;
use dioxus::prelude::*;
use std::path::PathBuf;
use storyfella_studio_core::registry::{
    ops::{self, Operation},
    ArtifactKind, GenerationDefaults, Ref,
};

fn operation(
    action: ArtAction,
    kind: ArtifactKind,
    name: &str,
    description: &str,
) -> Result<Operation, String> {
    let name = name.trim();
    let description = description.trim();
    let target = || Ref::new(action.target().expect("Targeted art action").to_string());
    match action {
        ArtAction::Create(_) => {
            if name.is_empty() {
                return Err("Enter a name for this artifact.".into());
            }
            if description.is_empty() {
                return Err("Describe the artifact you want to create.".into());
            }
            Ok(Operation::Create {
                kind,
                name: Some(name.into()),
                description: description.into(),
                size: None,
                steps: None,
                loras: vec![],
            })
        }
        ArtAction::Variant(_) => {
            if description.is_empty() {
                return Err("Describe what should be different in this variant.".into());
            }
            Ok(Operation::Variant {
                target: target(),
                name: (!name.is_empty()).then(|| name.into()),
                description: description.into(),
                axis: None,
                size: None,
                steps: None,
                loras: vec![],
            })
        }
        ArtAction::Modify(_) => {
            if description.is_empty() {
                return Err("Describe what to change and what to keep.".into());
            }
            Ok(Operation::Modify {
                target: target(),
                description: description.into(),
                size: None,
                steps: None,
                loras: vec![],
            })
        }
        ArtAction::Regenerate(_) => Ok(Operation::Regenerate {
            target: target(),
            prompt: (!description.is_empty()).then(|| description.into()),
            size: None,
            steps: None,
            loras: vec![],
        }),
        ArtAction::AttachAdapter(_) => {
            if name.is_empty() {
                return Err("Enter the adapter's registered name or file name.".into());
            }
            Ok(Operation::AttachLora {
                target: target(),
                lora: name.into(),
                multiplier: None,
            })
        }
    }
}

#[component]
pub fn ArtActionDialog(
    action: ArtAction,
    project: Option<PathBuf>,
    app_state: Signal<AppState>,
    defaults: Signal<GenerationDefaults>,
    disabled: bool,
    onrun: EventHandler<Operation>,
    onclose: EventHandler<()>,
) -> Element {
    let mut kind = use_signal(|| match action {
        ArtAction::Create(kind) => kind,
        _ => ArtifactKind::Character,
    });
    let mut name = use_signal(String::new);
    let mut description = use_signal(String::new);
    let mut error = use_signal(|| None::<String>);
    let (name_id, name_mount) = super::text_input::use_text_value(name(), None);
    let (description_id, description_mount) =
        super::text_input::use_text_value(description(), None);
    let target_name = action.target().and_then(|id| {
        app_state
            .read()
            .registry
            .artifact(id)
            .map(|a| a.name.clone())
    });
    let create = matches!(action, ArtAction::Create(_));
    let adapter = matches!(action, ArtAction::AttachAdapter(_));
    let first_image = matches!(action, ArtAction::Regenerate(id) if app_state.read().registry.artifact(id)
        .is_some_and(|artifact| app_state.read().display_image(artifact).is_none()));
    let title = if first_image {
        "Generate image"
    } else {
        action.title()
    };
    let show_name = create || adapter || matches!(action, ArtAction::Variant(_));
    let label = match action {
        ArtAction::Create(_) if kind().is_visual() => "Create and render",
        ArtAction::Create(_) => "Create entry",
        ArtAction::Variant(_) => "Create and render variant",
        ArtAction::Regenerate(_) if first_image => "Generate image",
        ArtAction::Regenerate(_) => "Generate new revision",
        ArtAction::Modify(_) => "Generate edited image",
        ArtAction::AttachAdapter(_) => "Attach adapter",
    };
    let submit = move |event: FormEvent| {
        event.prevent_default();
        if disabled {
            return;
        }
        if project.is_none() || app_state.read().project_path != project {
            error.set(Some(
                "Open the intended project before submitting this action.".into(),
            ));
            return;
        }
        match operation(action, kind(), &name(), &description()).and_then(|op| {
            ops::compile(&app_state.read().registry, &op, &defaults.read())
                .map_err(|e| e.to_string())?;
            Ok(op)
        }) {
            Ok(op) => {
                onrun.call(op);
                onclose.call(());
            }
            Err(message) => error.set(Some(message)),
        }
    };
    rsx! {
        Dialog { title, on_close: onclose,
            form { class: "studio-task-form", onsubmit: submit,
                header { class: "flex items-center gap-3",
                    h2 { class: "flex-1 text-xl font-semibold", "{title}" }
                    IconButton { label: "Close", icon: IconName::Close, onclick: move |_| onclose.call(()) }
                }
                if let Some(target) = target_name { p { class: "text-sm text-zinc-400", "For {target}" } }
                if create {
                    label { class: "studio-field", span { "Kind" }
                        select { class: "setting-control", aria_label: "Artifact kind", value: kind().as_str(),
                            onchange: move |e| { if let Some(next) = ArtifactKind::ALL.iter().find(|k| k.as_str() == e.value()) { kind.set(*next); } },
                            for candidate in ArtifactKind::ALL { option { value: candidate.as_str(), selected: kind() == candidate, "{candidate}" } }
                        }
                    }
                }
                if show_name {
                    label { class: "studio-field", r#for: name_id.clone(),
                        span { if adapter { "Adapter name or file" } else if create { "Name" } else { "Variant name (optional)" } }
                        input { id: name_id, onmounted: name_mount, class: "setting-control", autofocus: true,
                            placeholder: if adapter { "portrait-style.safetensors" } else { "Mara · harbor captain" },
                            oninput: move |e| name.set(e.value()) }
                    }
                }
                if !adapter {
                    label { class: "studio-field", r#for: description_id.clone(),
                        span { if matches!(action, ArtAction::Modify(_)) { "What should change?" } else if matches!(action, ArtAction::Regenerate(_)) { "Prompt override (optional)" } else { "Description" } }
                        textarea { id: description_id, onmounted: description_mount, class: "studio-textarea", rows: "5", autofocus: !show_name,
                            placeholder: if matches!(action, ArtAction::Modify(_) | ArtAction::Variant(_)) { "Change the coat to blue; keep the face, pose and background." } else { "Describe the subject, appearance and mood." },
                            oninput: move |e| description.set(e.value()) }
                    }
                }
                if let Some(message) = error() { p { role: "alert", class: "studio-inline-error", "{message}" } }
                footer { class: "flex justify-end gap-2",
                    button { r#type: "button", class: "studio-button", onclick: move |_| onclose.call(()), "Cancel" }
                    button { r#type: "submit", class: "studio-button studio-button-primary", disabled, title: "Uses project settings; existing revisions are preserved", "{label}" }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use storyfella_studio_core::registry::{ArtifactRegistry, CreateArtifact};
    #[test]
    fn forms_and_commands_produce_equivalent_intent_without_losing_punctuation() {
        let op = operation(
            ArtAction::Create(ArtifactKind::Story),
            ArtifactKind::Story,
            "Harbor story",
            "A captain's \"last\" voyage.",
        )
        .unwrap();
        let registry = ArtifactRegistry::new();
        let typed = crate::commands::parse_command(
            "/create story \"A captain's \\\"last\\\" voyage.\" --name \"Harbor story\"",
            &registry,
            &GenerationDefaults {
                sizes: Default::default(),
                steps: 25,
                cfg_scale: clipforge_inference::sd::DEFAULT_CFG_SCALE,
                sample_method: clipforge_inference::sd::DEFAULT_SAMPLE_METHOD.into(),
                loras: Default::default(),
            },
        )
        .unwrap();
        assert_eq!(op, typed);
    }
    #[test]
    fn form_targets_resolve_spaced_names_by_identity() {
        let mut registry = ArtifactRegistry::new();
        let id = registry
            .create_artifact(
                ArtifactKind::Character,
                CreateArtifact {
                    name: "Captain Mara".into(),
                    description: "Captain".into(),
                    ..Default::default()
                },
            )
            .unwrap();
        let op = operation(ArtAction::Regenerate(id), ArtifactKind::Character, "", "").unwrap();
        let Operation::Regenerate { target, prompt, .. } = op else {
            panic!("Wrong operation")
        };
        assert_eq!(registry.resolve(&target).unwrap(), id);
        assert!(prompt.is_none());
        assert!(operation(ArtAction::Modify(id), ArtifactKind::Character, "", " ").is_err());
    }
}
