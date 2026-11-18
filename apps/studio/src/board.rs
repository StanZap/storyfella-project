//! The story's assets as the Board shows them: one card per asset with its
//! variations stacked behind it, which ones still need art, and where the
//! story uses each. Nothing here draws; the view and the tests share it.
use std::path::PathBuf;

use storyfella_language::{analyze, Analysis, Source};
use storyfella_studio_core::{
    authoring::{source_links, AssetCatalog},
    catalog::KINDS,
    registry::{Artifact, ArtifactKind, ArtifactRegistry},
};
use uuid::Uuid;

use crate::{state::AppState, writing};

/// Which cards the Board shows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Filter {
    #[default]
    All,
    Kind(ArtifactKind),
    /// Assets with no finished image yet, variations included.
    NeedsArt,
}

impl Filter {
    pub const ALL: [Self; 6] = [
        Self::All,
        Self::Kind(ArtifactKind::Character),
        Self::Kind(ArtifactKind::Environment),
        Self::Kind(ArtifactKind::Object),
        Self::Kind(ArtifactKind::Illustration),
        Self::NeedsArt,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::All => "All",
            Self::Kind(kind) => plural(kind),
            Self::NeedsArt => "Needs art",
        }
    }
}

/// "Characters", "Places": what a kind is called on the Board.
pub fn plural(kind: ArtifactKind) -> &'static str {
    match kind {
        ArtifactKind::Character => "Characters",
        ArtifactKind::Environment => "Places",
        ArtifactKind::Object => "Props",
        ArtifactKind::Illustration => "Illustrations",
        _ => "Story items",
    }
}

/// "Character", "Place": one of a kind.
pub fn singular(kind: ArtifactKind) -> &'static str {
    match kind {
        ArtifactKind::Character => "Character",
        ArtifactKind::Environment => "Place",
        ArtifactKind::Object => "Prop",
        ArtifactKind::Illustration => "Illustration",
        _ => "Story item",
    }
}

/// One asset on the Board.
#[derive(Clone, Debug, PartialEq)]
pub struct Card {
    pub id: Uuid,
    pub name: String,
    pub kind: ArtifactKind,
    /// The finished image to show, if there is one.
    pub image: Option<PathBuf>,
    /// Variations stacked behind this card; none for a variation itself.
    pub variations: usize,
    /// How many times the story names it.
    pub uses: usize,
}

impl Card {
    /// "3 variations · 41 uses", or what the asset still needs.
    pub fn detail(&self) -> String {
        let mut parts = Vec::new();
        match self.variations {
            0 => {}
            1 => parts.push("1 variation".to_owned()),
            count => parts.push(format!("{count} variations")),
        }
        match self.uses {
            0 => parts.push("Not used yet".to_owned()),
            1 => parts.push("1 use".to_owned()),
            count => parts.push(format!("{count} uses")),
        }
        parts.join(" · ")
    }
}

/// A place the story names an asset.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Use {
    pub file: String,
    pub offset: usize,
    /// One-based, for showing.
    pub line: usize,
}

/// Every story file read against the catalog, for finding where assets are
/// used. Drafts count as they stand in the editor; other chapters as saved.
pub struct Reading {
    files: Vec<(String, String, Analysis)>,
}

impl Reading {
    pub fn of(state: &AppState) -> Self {
        let catalog = AssetCatalog::from_registry(&state.registry);
        let workspace = crate::preview::workspace(state).ok();
        let files = writing::files(state, workspace.as_ref())
            .into_iter()
            .filter_map(|file| {
                let text = workspace
                    .as_ref()
                    .and_then(|workspace| workspace.files.get(&file))
                    .map(String::as_str)
                    .or_else(|| writing::source(state, &file))?
                    .to_owned();
                let analysis = analyze(&text, &catalog.assets);
                Some((file, text, analysis))
            })
            .collect();
        Self { files }
    }

    /// Where the story names `id`, file by file in story order.
    pub fn uses(&self, registry: &ArtifactRegistry, id: Uuid) -> Vec<Use> {
        self.files
            .iter()
            .flat_map(|(file, text, analysis)| {
                let source = Source::new(text);
                source_links(registry, analysis, id)
                    .into_iter()
                    .map(move |link| Use {
                        file: file.clone(),
                        offset: link.span.start,
                        line: source
                            .position(link.span.start)
                            .map_or(1, |position| position.line as usize + 1),
                    })
                    .collect::<Vec<_>>()
            })
            .collect()
    }
}

/// The cards `filter` shows, by kind and then by name. Variations stack
/// behind their base, except under Needs art, where each needs its own.
pub fn cards(state: &AppState, reading: &Reading, filter: Filter) -> Vec<Card> {
    let registry = &state.registry;
    let mut cards: Vec<Card> = registry
        .artifacts
        .iter()
        .filter(|artifact| KINDS.contains(&artifact.kind))
        .filter(|artifact| match filter {
            Filter::All => artifact.variant_of.is_none(),
            Filter::Kind(kind) => artifact.variant_of.is_none() && artifact.kind == kind,
            Filter::NeedsArt => AppState::display_image(artifact).is_none(),
        })
        .map(|artifact| Card {
            id: artifact.id,
            name: name(artifact),
            kind: artifact.kind,
            image: AppState::display_image(artifact).map(PathBuf::from),
            variations: if filter == Filter::NeedsArt {
                0
            } else {
                registry
                    .artifacts
                    .iter()
                    .filter(|other| other.variant_of == Some(artifact.id))
                    .count()
            },
            uses: reading.uses(registry, artifact.id).len(),
        })
        .collect();
    cards.sort_by_key(|card| {
        (
            KINDS.iter().position(|kind| *kind == card.kind),
            card.name.to_lowercase(),
        )
    });
    cards
}

/// How many cards Needs art would show.
pub fn needing_art(state: &AppState) -> usize {
    state
        .registry
        .artifacts
        .iter()
        .filter(|artifact| KINDS.contains(&artifact.kind))
        .filter(|artifact| AppState::display_image(artifact).is_none())
        .count()
}

/// An asset's base and its variations, base first.
pub fn family(registry: &ArtifactRegistry, id: Uuid) -> Vec<(Uuid, String)> {
    let Some(base) = registry
        .artifact(id)
        .map(|artifact| artifact.variant_of.unwrap_or(artifact.id))
    else {
        return Vec::new();
    };
    let mut family: Vec<_> = registry
        .artifacts
        .iter()
        .filter(|artifact| artifact.id == base || artifact.variant_of == Some(base))
        .map(|artifact| (artifact.id, name(artifact)))
        .collect();
    family.sort_by_key(|(id, name)| (*id != base, name.to_lowercase()));
    family
}

fn name(artifact: &Artifact) -> String {
    if artifact.name.trim().is_empty() {
        "Unnamed".to_owned()
    } else {
        artifact.name.clone()
    }
}

/// Whether the open story is a project folder Atelier can open.
pub fn shared_with_atelier(state: &AppState) -> bool {
    state.project_path.as_ref().is_some_and(|path| {
        path.file_name().is_some_and(|name| name == "story.db")
            && path.parent().is_some_and(|folder| {
                folder
                    .join(storyfella_studio_core::project::MANIFEST)
                    .is_file()
            })
    })
}

#[cfg(test)]
#[path = "board/tests.rs"]
mod tests;

/// Hands `id` to Atelier, starting it if it is not running, and waits up to
/// thirty seconds for it to say whether it opened. Blocks; run it off the UI
/// thread.
pub fn open_in_atelier(project: &std::path::Path, source: &str, id: Uuid) -> Result<(), String> {
    use storyfella_studio_core::atelier::{self, HandoffReply, Mailbox, OpenRequest};
    let request =
        OpenRequest::prepare(project, source, id).map_err(|error| format!("{error:#}"))?;
    atelier::send_and_launch(&request).map_err(|error| format!("{error:#}"))?;
    let mailbox = Mailbox::new(&atelier::data_dir());
    for _ in 0..300 {
        match mailbox.reply(request.id) {
            Ok(Some(HandoffReply::Opened)) => return Ok(()),
            Ok(Some(HandoffReply::Failed(message))) => return Err(message),
            Err(error) => {
                mailbox.cancel(request.id);
                return Err(format!("{error:#}"));
            }
            Ok(None) => std::thread::sleep(std::time::Duration::from_millis(100)),
        }
    }
    mailbox.cancel(request.id);
    Err("Atelier did not answer. Check that it can start, then try again.".into())
}
