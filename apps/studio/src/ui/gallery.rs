//! The artifact grid — the canvas at rest.
//!
//! Replaces the kind-grouped sidebar with tiles: the picture is the thing you
//! are looking for, so it is what you navigate by. Variants stack behind
//! their base, and work in flight stays *in the grid* (dimmed, badged) rather
//! than in a toast that disappears — a generation is a tile that is not
//! finished yet, not a notification.
//!
//! Source associations supply origins; the story sequence follows `.sf`.

use dioxus::prelude::*;
use uuid::Uuid;

use storyfella_studio_core::registry::{Artifact, ArtifactKind, ArtifactRegistry, RevisionStatus};

use crate::state::AppState;

use super::{
    generated_asset_url,
    icons::{Icon, IconName},
};

/// Kind order: the creation loop first.
const GROUPS: [ArtifactKind; 6] = [
    ArtifactKind::Character,
    ArtifactKind::Environment,
    ArtifactKind::Object,
    ArtifactKind::Scene,
    ArtifactKind::Story,
    ArtifactKind::Beat,
];

/// What a tile shows, resolved from the registry before rendering so the
/// markup stays declarative.
#[derive(Clone, PartialEq)]
pub struct Tile {
    pub id: Uuid,
    pub name: String,
    pub kind: ArtifactKind,
    /// The active image, if one has finished.
    pub image: Option<String>,
    /// How many variants sit behind this base.
    pub variants: usize,
    /// A generation is running for this artifact.
    pub working: bool,
    /// The authored declaration key, independent of the artwork's display name.
    pub origin: Option<String>,
    pub subtitle: String,
}

/// Builds the tiles for one kind, bases only — variants are counted into
/// their base's stack rather than given tiles of their own.
fn tiles_for(registry: &ArtifactRegistry, kind: ArtifactKind, query: &str) -> Vec<Tile> {
    let mut tiles: Vec<Tile> = registry
        .artifacts
        .iter()
        .filter(|artifact| artifact.kind == kind && artifact.variant_of.is_none())
        .filter(|artifact| matches(artifact, query))
        .map(|artifact| {
            let variants = registry
                .artifacts
                .iter()
                .filter(|other| other.variant_of == Some(artifact.id))
                .count();
            // Both in-flight states, not just `Queued`: `Generating` is the
            // one a revision holds for most of its life, so keying off
            // `Queued` alone would drop the badge exactly while the picture
            // is being made.
            let working = artifact.revisions.iter().any(|revision| {
                matches!(
                    revision.status,
                    RevisionStatus::Queued | RevisionStatus::Generating
                )
            });
            Tile {
                id: artifact.id,
                name: if artifact.name.trim().is_empty() {
                    "Unnamed".to_owned()
                } else {
                    artifact.name.clone()
                },
                kind,
                image: registry
                    .latest_image(artifact.id)
                    .and_then(|revision| revision.asset_path.as_deref())
                    .and_then(generated_asset_url),
                variants,
                working,
                origin: storyfella_studio_core::authoring::declaration_key(registry, artifact.id),
                subtitle: first_line(&artifact.description),
            }
        })
        .collect();
    tiles.sort_by_key(|tile| tile.name.to_ascii_lowercase());
    tiles
}

fn matches(artifact: &Artifact, query: &str) -> bool {
    if query.is_empty() {
        return true;
    }
    artifact.name.to_ascii_lowercase().contains(query)
        || artifact.description.to_ascii_lowercase().contains(query)
}

/// The first clause of a description, for the caption under a tile.
fn first_line(description: &str) -> String {
    let trimmed = description.trim();
    let cut = trimmed
        .find(['.', ',', '\n'])
        .unwrap_or(trimmed.len())
        .min(trimmed.len());
    trimmed[..cut].trim().to_owned()
}

/// The tile ids in render order, so keyboard navigation and the grid agree
/// on what "next" means.
pub fn ordered_ids(registry: &ArtifactRegistry, query: &str) -> Vec<Uuid> {
    let query = query.to_ascii_lowercase();
    GROUPS
        .into_iter()
        .flat_map(|kind| tiles_for(registry, kind, &query))
        .map(|tile| tile.id)
        .collect()
}

#[component]
pub fn Gallery(
    app_state: Signal<AppState>,
    mut filter: Signal<String>,
    mut composer: Signal<String>,
    /// The keyboard cursor, as an index into [`ordered_ids`].
    cursor: Signal<usize>,
    disabled: bool,
) -> Element {
    let (filter_id, filter_mount) = super::text_input::use_text_value(filter(), None);
    let registry = app_state.read().registry.clone();
    let query = filter.read().to_ascii_lowercase();
    let empty = registry.artifacts.is_empty();
    let ids = ordered_ids(&registry, &query);
    let result_count = ids.len();
    let focused = ids
        .get((*cursor.read()).min(result_count.saturating_sub(1)))
        .copied();

    let groups = GROUPS.into_iter().filter_map(|kind| {
        let tiles = tiles_for(&registry, kind, &query);
        if tiles.is_empty() {
            return None;
        }
        Some(rsx! {
            section { key: "{kind}",
                div { class: "mb-3 flex items-center gap-2",
                    p { class: "text-[10px] font-semibold uppercase tracking-[0.16em] text-zinc-600",
                        "{kind}s"
                    }
                    span { class: "text-[10px] text-zinc-700", "{tiles.len()}" }
                    div { class: "h-px flex-1 bg-white/[0.055]" }
                }
                // `data-grid` is what the key bridge measures to learn how
                // many columns `auto-fill` produced at this width.
                div { "data-grid": "true",
                    class: "grid grid-cols-[repeat(auto-fill,minmax(148px,1fr))] gap-4",
                    for tile in tiles {
                        TileCard {
                            key: "{tile.id}",
                            tile: tile.clone(),
                            focused: focused == Some(tile.id),
                            onopen: move |id| { app_state.write().select_artifact(id); },
                        }
                    }
                }
            }
        })
    });

    rsx! {
        div { class: "min-h-0 w-full flex-1 overflow-y-auto",
            div { class: "sticky top-0 z-10 flex items-center gap-3 border-b border-white/[0.05] bg-zinc-950/80 px-6 py-3 backdrop-blur-xl",
                p { class: "text-[10px] font-semibold uppercase tracking-[0.18em] text-zinc-600",
                    "Artifacts · {result_count} shown"
                }
                p { class: "text-[10px] text-zinc-700",
                    "{super::shortcuts::hint_line(super::shortcuts::Scope::Grid)}"
                }
                input {
                    class: "ml-auto h-8 w-56 rounded-lg bg-white/[0.045] px-3 text-[11px] text-zinc-200 outline-none ring-1 ring-inset ring-white/[0.06] transition placeholder:text-zinc-700 focus:ring-violet-400/40",
                    aria_label: "Filter artifacts",
                    placeholder: "Filter…",
                    id: filter_id,
                    onmounted: filter_mount,
                    disabled,
                    oninput: move |event| filter.set(event.value()),
                }
            }

            if !registry.story_scenes.is_empty() {
                div { class: "space-y-2 border-b border-white/[0.05] px-6 py-3",
                    p { class: "text-[10px] text-zinc-500", "Story sequence · from .sf" }
                    for scene in &registry.story_scenes {
                        div { class: "flex flex-wrap items-center gap-2",
                            button {
                                class: "font-mono text-[11px] text-violet-300 hover:text-white",
                                onclick: { let id = scene.artifact_id; move |_| app_state.write().select_artifact(id) },
                                "{scene.name}"
                            }
                            for beat in &scene.beats {
                                button {
                                    class: "rounded bg-white/[0.05] px-2 py-1 font-mono text-[11px] text-zinc-300 hover:text-white",
                                    onclick: { let id = beat.artifact_id; move |_| app_state.write().select_artifact(id) },
                                    "{beat.name}"
                                }
                            }
                        }
                    }
                }
            }
            if empty {
                div { class: "grid place-items-center px-6 py-24 text-center",
                    div { class: "grid size-12 place-items-center rounded-2xl bg-white/[0.045] text-zinc-600 ring-1 ring-inset ring-white/[0.06]",
                        Icon { name: IconName::Layers, class: "size-5" }
                    }
                    p { class: "mt-4 text-[13px] font-semibold text-zinc-200", "Start with an artifact" }
                    p { class: "mt-1 max-w-sm text-[11px] leading-5 text-zinc-500",
                        "Create your first character, environment, or object with the prompt bar below."
                    }
                    button {
                        class: "mt-4 rounded-lg bg-violet-400/15 px-3 py-1.5 text-[11px] font-semibold text-violet-200 transition hover:bg-violet-400/25 disabled:opacity-40",
                        disabled,
                        onclick: move |_| composer.set("/create character ".to_owned()),
                        "New character"
                    }
                }
            } else if focused.is_none() {
                div { role: "status", class: "px-6 py-16 text-center",
                    p { "No artifacts match this filter." }
                    button { class: "mt-3 rounded bg-violet-400/15 px-3 py-2", onclick: move |_| filter.set(String::new()), "Clear filter" }
                }
            } else {
                div { class: "space-y-8 px-6 pb-28 pt-6", {groups} }
            }
        }
    }
}

#[component]
fn TileCard(tile: Tile, focused: bool, onopen: EventHandler<Uuid>) -> Element {
    rsx! {
        button {
            class: "group text-left",
            // Keeps the keyboard cursor on screen as it moves.
            id: if focused { "svs-focused-tile" } else { "" },
            onclick: move |_| onopen.call(tile.id),

            div { class: "relative mb-2 aspect-[2/3]",
                // Variants stack behind the base, so a tile shows at a glance
                // how much exploration sits under it.
                if tile.variants > 2 {
                    div { class: "absolute inset-0 translate-x-2 -translate-y-1.5 rotate-[5deg] rounded-[10px] bg-zinc-800/60 ring-1 ring-inset ring-white/[0.06]" }
                }
                if tile.variants > 1 {
                    div { class: "absolute inset-0 translate-x-1 -translate-y-1 rotate-[2.5deg] rounded-[10px] bg-zinc-800/80 ring-1 ring-inset ring-white/[0.07]" }
                }

                if let Some(url) = tile.image.clone() {
                    img {
                        class: "absolute inset-0 size-full rounded-[10px] object-contain ring-1 ring-inset ring-white/[0.11] transition group-hover:ring-white/20",
                        class: if focused { "ring-2 ring-violet-400" } else { "" },
                        class: if tile.working { "opacity-40" } else { "" },
                        src: "{url}",
                        alt: "{tile.name}",
                    }
                } else {
                    div { class: "absolute inset-0 grid place-items-center rounded-[10px] bg-white/[0.04] text-zinc-700 ring-1 ring-inset ring-white/[0.07]",
                        Icon { name: IconName::Layers, class: "size-5" }
                    }
                }

                // In-flight work stays in the grid rather than in a toast.
                if tile.working {
                    span { class: "absolute right-1.5 top-1.5 z-10 flex items-center gap-1.5 rounded-md bg-violet-400/25 px-1.5 py-0.5 text-[9px] font-semibold text-violet-100 ring-1 ring-inset ring-violet-300/30 backdrop-blur",
                        span { class: "size-2 animate-pulse rounded-full bg-violet-300" }
                        "rendering"
                    }
                } else if tile.variants > 0 {
                    span { class: "absolute right-1.5 top-1.5 z-10 rounded-md bg-black/60 px-1.5 py-0.5 text-[9px] font-semibold text-zinc-300 ring-1 ring-inset ring-white/10 backdrop-blur",
                        "{tile.variants + 1}"
                    }
                }
            }

            p {
                class: "truncate font-mono text-[12px] font-semibold",
                class: if focused { "text-violet-300" } else { "text-zinc-200" },
                "{tile.name}"
            }
            p { class: "mt-0.5 truncate text-[10px] text-zinc-600",
                "{tile.subtitle}"
                if let Some(origin) = tile.origin.clone() {
                    span { class: "font-mono text-zinc-700", " · {origin}" }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use storyfella_studio_core::registry::CreateArtifact;

    fn registry_with(entries: &[(ArtifactKind, &str)]) -> ArtifactRegistry {
        let mut registry = ArtifactRegistry::new();
        for (kind, name) in entries {
            registry
                .create_artifact(
                    *kind,
                    CreateArtifact {
                        name: (*name).into(),
                        description: "a test artifact, with a clause".into(),
                        ..Default::default()
                    },
                )
                .expect("artifact should create");
        }
        registry
    }

    #[test]
    fn variants_are_counted_into_their_base_not_given_tiles() {
        let mut registry = registry_with(&[(ArtifactKind::Character, "mara")]);
        let base = registry.artifacts[0].id;
        for name in ["mara-oilskins", "mara-night"] {
            let id = registry
                .create_artifact(
                    ArtifactKind::Character,
                    CreateArtifact {
                        name: name.into(),
                        description: "a variant".into(),
                        ..Default::default()
                    },
                )
                .expect("variant should create");
            registry.artifact_mut(id).unwrap().variant_of = Some(base);
        }

        let tiles = tiles_for(&registry, ArtifactKind::Character, "");
        assert_eq!(tiles.len(), 1, "variants must not get their own tiles");
        assert_eq!(tiles[0].variants, 2);
    }

    #[test]
    fn filtering_matches_name_or_description() {
        let registry = registry_with(&[
            (ArtifactKind::Character, "mara"),
            (ArtifactKind::Character, "tam"),
        ]);
        assert_eq!(
            tiles_for(&registry, ArtifactKind::Character, "mar").len(),
            1
        );
        // The shared description matches both.
        assert_eq!(
            tiles_for(&registry, ArtifactKind::Character, "clause").len(),
            2
        );
        assert!(tiles_for(&registry, ArtifactKind::Character, "zzz").is_empty());
    }

    #[test]
    fn captions_stop_at_the_first_clause() {
        assert_eq!(first_line("A harbourmaster, weathered"), "A harbourmaster");
        assert_eq!(first_line("One sentence. Two."), "One sentence");
        assert_eq!(first_line("  no punctuation  "), "no punctuation");
        assert_eq!(first_line(""), "");
    }

    #[test]
    fn both_in_flight_states_mark_a_tile_as_working() {
        for status in [RevisionStatus::Queued, RevisionStatus::Generating] {
            let mut registry = registry_with(&[(ArtifactKind::Character, "mara")]);
            let id = registry.artifacts[0].id;
            let revision = registry
                .start_revision(
                    id,
                    "a prompt".into(),
                    None,
                    None,
                    storyfella_studio_core::registry::RevisionOrigin::Canon,
                )
                .expect("revision should start");
            registry
                .artifact_mut(id)
                .unwrap()
                .revisions
                .iter_mut()
                .find(|candidate| candidate.id == revision)
                .unwrap()
                .status = status;

            let tiles = tiles_for(&registry, ArtifactKind::Character, "");
            assert!(tiles[0].working, "{status:?} should read as in flight");
        }
    }

    #[test]
    fn finished_work_stops_reading_as_in_flight() {
        let registry = registry_with(&[(ArtifactKind::Character, "mara")]);
        let tiles = tiles_for(&registry, ArtifactKind::Character, "");
        assert!(!tiles[0].working);
    }

    #[test]
    fn authored_origins_are_distinct_from_manual_artwork() {
        let mut registry = registry_with(&[(ArtifactKind::Scene, "harbor")]);
        storyfella_studio_core::authoring::reconcile(
            &mut registry,
            &storyfella_language::analyze("scene harbor\n  beat arrival\n", &[]),
        );
        let tiles = tiles_for(&registry, ArtifactKind::Scene, "");
        assert_eq!(tiles[0].origin, None);
        assert_eq!(tiles[1].origin.as_deref(), Some("harbor"));
        assert_eq!(
            tiles_for(&registry, ArtifactKind::Beat, "")[0]
                .origin
                .as_deref(),
            Some("harbor.arrival")
        );
    }
}
