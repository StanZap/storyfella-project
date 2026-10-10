use crate::state::AppState;
use dioxus::prelude::*;
use std::{path::PathBuf, sync::Arc};
use storyfella_studio_core::{artwork::ApprovedArtwork, registry::ArtifactKind};

#[component]
pub fn ArtworkImport(
    mut app_state: Signal<AppState>,
    asset_directory: PathBuf,
    disabled: bool,
    trigger: Signal<u64>,
) -> Element {
    let mut pending = use_signal(|| None::<Arc<ApprovedArtwork>>);
    let mut preview = use_signal(String::new);
    let mut name = use_signal(String::new);
    let mut kind = use_signal(|| ArtifactKind::Character);
    let mut loading = use_signal(|| false);
    let mut importing = use_signal(|| false);
    let mut error = use_signal(|| None::<String>);
    let mut review_project = use_signal(|| None::<PathBuf>);
    let mut pick = move || {
        // This action runs inside a trigger effect. Reading task state must
        // not subscribe that effect and reopen the picker when loading ends.
        if disabled || *loading.peek() || *importing.peek() {
            return;
        }
        let Some(path) = rfd::FileDialog::new()
            .set_title("Import approved Atelier artwork")
            .add_filter("Atelier artwork", &["sfart"])
            .pick_file()
        else {
            return;
        };
        loading.set(true);
        review_project.set(app_state.peek().project_path.clone());
        error.set(None);
        spawn(async move {
            let result = tokio::task::spawn_blocking(move || {
                let artwork = ApprovedArtwork::load(&path)?;
                let preview = artwork.preview_data_url()?;
                Ok::<_, anyhow::Error>((artwork, preview))
            })
            .await;
            loading.set(false);
            match result {
                Ok(Ok((artwork, thumbnail))) => {
                    preview.set(thumbnail);
                    name.set(artwork.name.clone());
                    pending.set(Some(Arc::new(artwork)));
                }
                Ok(Err(failure)) => error.set(Some(format!(
                    "Could not load artwork: {failure:#}. Export it again from Atelier and retry."
                ))),
                Err(failure) => error.set(Some(format!(
                    "Artwork worker failed: {failure}. Try importing again."
                ))),
            }
        });
    };
    let trigger_value = trigger();
    use_effect(use_reactive(&trigger_value, move |value| {
        if value > 0 {
            pick();
        }
    }));
    let (name_id, name_mount) = super::text_input::use_text_value(name(), None);
    let apply = move |_| {
        if importing() {
            return;
        }
        let Some(artwork) = pending() else {
            return;
        };
        let mut artwork = (*artwork).clone();
        artwork.name = name().trim().to_owned();
        if artwork.name.is_empty() {
            error.set(Some("Enter a name for this artwork.".into()));
            return;
        }
        let project = review_project();
        if app_state.read().project_path != project {
            error.set(Some(
                "The project changed. Cancel and import into the intended project.".into(),
            ));
            return;
        }
        let directory = asset_directory.clone();
        let selected_kind = kind();
        importing.set(true);
        error.set(None);
        spawn(async move {
            let image = artwork.clone();
            let result = tokio::task::spawn_blocking(move || image.write_asset(&directory)).await;
            importing.set(false);
            match result {
                Ok(Ok(path)) => {
                    let result = if app_state.read().project_path == project {
                        app_state.write().import_artwork(
                            &artwork,
                            selected_kind,
                            path.display().to_string(),
                        )
                    } else {
                        Err(
                            "The project changed. Import cancelled; retry in the intended project."
                                .into(),
                        )
                    };
                    match result {
                        Ok(_) => pending.set(None),
                        Err(message) => {
                            let _ = std::fs::remove_file(path);
                            error.set(Some(message));
                        }
                    }
                }
                Ok(Err(failure)) => error.set(Some(format!(
                    "Could not store artwork: {failure:#}. Check free disk space and retry."
                ))),
                Err(failure) => error.set(Some(format!(
                    "Artwork worker failed: {failure}. Retry the import."
                ))),
            }
        });
    };
    rsx! {
        if pending().is_none() {
            if loading() { p { role: "status", class: "px-4 py-2 text-xs", "Loading artwork…" } }
            if let Some(message) = error() { p { role: "alert", class: "studio-inline-error", "{message}" } }
        }
        if let Some(artwork) = pending() {
            super::dialog::Dialog { title: "Import approved artwork", on_close: move |_| { if !importing() { pending.set(None); error.set(None); } },
                div { class: "w-[420px] max-w-full space-y-4 p-6",
                    h2 { class: "text-lg font-semibold text-zinc-100", "Import approved artwork" }
                    img { src: preview(), alt: "Approved artwork preview", class: "h-40 w-full rounded-lg bg-zinc-900 object-contain" }
                    label { class: "block text-sm text-zinc-300", "Name",
                        input { class: "mt-1 w-full rounded-lg bg-zinc-900 p-2 text-zinc-100 ring-1 ring-zinc-700", autofocus: true, disabled: importing(), id: name_id, onmounted: name_mount, oninput: move |event| name.set(event.value()) }
                    }
                    label { class: "block text-sm text-zinc-300", "Use as",
                        select { class: "mt-1 w-full rounded-lg bg-zinc-900 p-2 text-zinc-100", disabled: importing(), value: kind().as_str(),
                            onchange: move |event| kind.set(match event.value().as_str() { "environment" => ArtifactKind::Environment, "object" => ArtifactKind::Object, _ => ArtifactKind::Character }),
                            option { value: "character", "Character" } option { value: "environment", "Environment" } option { value: "object", "Object" }
                        }
                    }
                    details { summary { class: "text-xs text-zinc-400", "Provenance" }
                        p { class: "mt-2 max-h-28 overflow-y-auto whitespace-pre-wrap break-words text-xs text-zinc-400", "{artwork.provenance}" }
                    }
                    if let Some(message) = error() { p { role: "alert", class: "text-sm text-rose-300", "{message}" } }
                    div { class: "flex justify-end gap-2",
                        button { class: "rounded-lg px-3 py-2 text-sm text-zinc-300", disabled: importing(), onclick: move |_| { pending.set(None); error.set(None); }, "Cancel" }
                        button { class: "rounded-lg bg-violet-500 px-3 py-2 text-sm font-semibold text-white disabled:opacity-40", disabled: importing() || name().trim().is_empty(), onclick: apply,
                            if importing() { "Importing…" } else { "Add to project" }
                        }
                    }
                }
            }
        }
    }
}
