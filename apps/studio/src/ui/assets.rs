//! Asset browsing, source references and read-only saved artwork.
use super::{
    actions::{ActionRequest, ArtAction},
    art_actions::ArtActionDialog,
    components::{EmptyVisual, IconButton},
    gallery::Gallery,
    icons::IconName,
    inspector::Inspector,
};
use crate::state::AppState;
use dioxus::prelude::*;
use storyfella_studio_core::{
    authoring::AssetCatalog,
    registry::{Artifact, ArtifactKind},
};

#[component]
pub fn Assets(mut app_state: Signal<AppState>) -> Element {
    let filter = use_signal(String::new);
    let mut pending = use_signal(|| None::<ArtAction>);
    let mut actions = use_context::<ActionRequest>().0;
    let mut cursor = use_context::<super::GridCursor>().0;
    let palette_open = use_context::<Signal<bool>>();
    let screen = use_context::<Signal<super::AppScreen>>();
    let keys = use_context::<Signal<Option<super::keys::KeyEvent>>>();
    let disabled =
        app_state.read().operation_in_progress() || app_state.read().project_path.is_none();
    let selected = app_state.read().selected_artifact().cloned();
    use_effect(move || {
        if let Some(action) = actions() {
            actions.set(None);
            if !disabled {
                pending.set(Some(action));
            }
        }
    });
    let query = filter();
    use_effect(move || {
        let Some((_, press)) = keys() else { return };
        if *screen.peek() != super::AppScreen::Assets || *palette_open.peek() {
            return;
        }
        let state = app_state.peek();
        let viewing = state.selected_artifact_id.is_some();
        let ids = super::gallery::ordered_ids(&state.registry, &query.to_ascii_lowercase());
        let variant = state
            .selected_artifact()
            .filter(|a| a.kind.supports_variants())
            .map(|a| a.variant_of.unwrap_or(a.id));
        drop(state);
        match press.key.as_str() {
            "Escape" if viewing => app_state.write().clear_selection(),
            "Enter" if !viewing => {
                if let Some(id) = ids.get(*cursor.peek()) {
                    app_state.write().select_artifact(*id);
                }
            }
            "n" if !disabled && !viewing => {
                pending.set(Some(ArtAction::Create(ArtifactKind::Character)))
            }
            "v" if !disabled => {
                if let Some(id) = variant {
                    pending.set(Some(ArtAction::Variant(id)));
                }
            }
            key if !viewing => {
                let current = *cursor.peek();
                if let Some(next) = super::keys::move_cursor(key, current, ids.len(), press.columns)
                {
                    cursor.set(next);
                }
            }
            _ => {}
        }
    });
    rsx! {
        section { class: "flex h-full min-h-0 flex-col",
            if let Some(artifact) = selected {
                AssetDetail { key: "{artifact.id}", app_state, artifact, disabled }
            } else {
                Gallery { app_state, filter, cursor, disabled }
            }
        }
        if let Some(action) = pending() { ArtActionDialog { action, app_state, onclose: move |_| pending.set(None) } }
    }
}

#[component]
fn AssetDetail(mut app_state: Signal<AppState>, artifact: Artifact, disabled: bool) -> Element {
    let mut metadata = use_signal(|| true);
    let mut screen = use_context::<Signal<super::AppScreen>>();
    let caret = use_context::<super::WriterCaret>();
    let registry = app_state.read().registry.clone();
    let analysis = storyfella_language::analyze(
        app_state.read().story.source(),
        &AssetCatalog::from_registry(&registry).assets,
    );
    let links: Vec<_> =
        storyfella_studio_core::authoring::source_links(&registry, &analysis, artifact.id)
            .into_iter()
            .filter_map(|link| {
                analysis
                    .document
                    .source
                    .position(link.span.start)
                    .map(|position| (link.label, position))
            })
            .collect();
    let image = AppState::display_image(&artifact).and_then(super::generated_asset_url);
    let base = artifact.variant_of.unwrap_or(artifact.id);
    let family: Vec<_> = registry
        .artifacts
        .iter()
        .filter(|item| item.id == base || item.variant_of == Some(base))
        .map(|item| (item.id, item.name.clone()))
        .collect();
    let mut actions = use_context::<ActionRequest>().0;
    rsx! {
        div { class: "flex min-h-0 flex-1",
            div { class: "flex min-h-0 min-w-0 flex-1 flex-col",
                header { class: "flex flex-wrap items-center gap-2 border-b border-white/10 p-3",
                    IconButton { label: "Back to assets", icon: IconName::Arrow, onclick: move |_| app_state.write().clear_selection() }
                    h2 { class: "min-w-0 flex-1 truncate text-sm font-semibold", "{artifact.name}" }
                    if image.is_some() { super::artwork::PreviewBackgroundPicker {} }
                    super::atelier::OpenInAtelier { target: Some(artifact.id) }
                    IconButton { label: if metadata() { "Hide asset details" } else { "Show asset details" }, icon: IconName::Sliders, expanded: metadata(), onclick: move |_| { let show = !metadata(); metadata.set(show); } }
                }
                if !links.is_empty() {
                    div { class: "flex flex-wrap gap-2 px-4 py-2",
                        for (label, position) in links {
                            button { class: "studio-button", title: "Open this story reference", onclick: move |_| { app_state.write().active_source = None; caret.navigate(position); screen.set(super::AppScreen::Writer); }, "{label}" }
                        }
                    }
                }
                div { class: "relative min-h-0 flex-1 overflow-hidden bg-black/20 p-3",
                    if let Some(url) = image {
                        super::artwork::ArtworkImage {
                            class: "h-full w-full object-contain", src: url, alt: artifact.name.clone(),
                        }
                    }
                    else { div { class: "grid h-full place-content-center", EmptyVisual { icon: IconName::Image, title: "No saved artwork yet", description: "Open in Atelier to create its artwork." } } }
                }
                if family.len() > 1 || artifact.kind.supports_variants() {
                    div { class: "flex shrink-0 flex-wrap gap-2 border-t border-white/10 p-3",
                        for (id, name) in family { button { class: "studio-button", aria_pressed: id == artifact.id, onclick: move |_| app_state.write().select_artifact(id), "{name}" } }
                        if artifact.kind.supports_variants() {
                            IconButton {
                                label: "New variation definition", icon: IconName::Add, disabled,
                                onclick: move |_| actions.set(Some(ArtAction::Variant(base))),
                            }
                        }
                    }
                }
                if !artifact.revisions.is_empty() {
                    details { class: "max-h-60 shrink-0 overflow-auto border-t border-white/10 p-3 text-xs text-zinc-400",
                        summary { "Saved image history" }
                        for revision in artifact.revisions.iter().rev() {
                            div { class: "flex items-center gap-3 py-2",
                                if let Some(url) = revision.asset_path.as_deref().and_then(super::generated_asset_url) {
                                    super::artwork::ArtworkImage {
                                        class: "size-16 object-contain", src: url, alt: "Saved image revision",
                                    }
                                }
                                span { "{revision.prompt}" }
                                if Some(revision.id) == artifact.active_revision_id { span { "Current" } }
                                span { "{revision.status:?}" }
                            }
                        }
                    }
                }
            }
            if metadata() { Inspector { artifact, disabled } }
        }
    }
}
