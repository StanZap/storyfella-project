//! The artifact registry — the domain model behind the operation API.
//!
//! One unified id space for every artifact (story, scene, beat, character,
//! environment, object); `c:<ref>` references resolve here. This is new
//! domain-model work layered onto the existing `Project` model: the registry
//! lives alongside `Project` in `AppState`, and the TOML `ProjectStore`
//! stays untouched (SQLite persistence is deferred — see
//! `docs/artifact-canvas.md` §10).
//!
//! Invariants maintained by the registry API (mirroring the planned SQL
//! schema in §10):
//!
//! - Variants point at a base artifact of the same kind; only
//!   characters, environments, and objects can have variants; the base is
//!   itself never a variant.
//! - Parents are kind-compatible: scene → story, beat → scene, room
//!   (environment) → environment. Stories, characters, and objects have no
//!   parent.
//! - A beat is a composition: `composition` carries the mode and the layer
//!   list (backdrop / baked / dynamic); the backdrop slot is the environment
//!   (or "generate fresh" when no backdrop layer exists).
//! - Masks attach to a specific revision; a revision carries its generation
//!   seed and model so golden runs can be replayed identically.

pub mod backend;
pub mod image_ops;
pub mod ops;
pub mod pipeline;

use std::fmt;

use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

use crate::models::RevisionStatus;

use self::ops::{OpOrigin, OpStatus, Operation, OperationLogEntry};

/// The kinds of artifacts in the unified registry.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ArtifactKind {
    Story,
    Scene,
    Beat,
    Character,
    Environment,
    Object,
}

impl ArtifactKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            ArtifactKind::Story => "story",
            ArtifactKind::Scene => "scene",
            ArtifactKind::Beat => "beat",
            ArtifactKind::Character => "character",
            ArtifactKind::Environment => "environment",
            ArtifactKind::Object => "object",
        }
    }
}

impl fmt::Display for ArtifactKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// The organizational axes variants can be tagged with. Tags only — a
/// variant's identity comes from its `variant_of` link.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum VariantAxis {
    Outfit,
    Age,
    Body,
    Hair,
    Expression,
    TimeOfDay,
    Weather,
    Season,
    Mood,
}

impl VariantAxis {
    pub fn as_str(&self) -> &'static str {
        match self {
            VariantAxis::Outfit => "outfit",
            VariantAxis::Age => "age",
            VariantAxis::Body => "body",
            VariantAxis::Hair => "hair",
            VariantAxis::Expression => "expression",
            VariantAxis::TimeOfDay => "time-of-day",
            VariantAxis::Weather => "weather",
            VariantAxis::Season => "season",
            VariantAxis::Mood => "mood",
        }
    }
}

impl fmt::Display for VariantAxis {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A `c:<ref>` reference into the registry. Resolution happens against a
/// specific registry (see [`ArtifactRegistry::resolve`]). The primary form
/// is a memorable **artifact name** (`c:mia`, case-insensitive, ambiguous
/// names rejected); full UUIDs, `c:` + UUID, and `c:` + the 8-hex short id
/// remain accepted as fallbacks.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Ref(pub String);

impl Ref {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }
}

impl std::str::FromStr for Ref {
    type Err = std::convert::Infallible;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Ok(Self::new(value))
    }
}

impl fmt::Display for Ref {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[derive(Debug, Error)]
pub enum RefError {
    #[error(
        "`{0}` is not a valid c: ref (expected c:<name>, a UUID, c:<uuid>, or c:<8-hex short id>)"
    )]
    Invalid(String),
    #[error("ref `{0}` does not match any artifact")]
    NotFound(String),
    #[error("ref `{value}` is ambiguous; it matches {matches} artifacts")]
    Ambiguous { value: String, matches: usize },
}

/// A single artifact in the unified registry.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Artifact {
    pub id: Uuid,
    pub kind: ArtifactKind,
    pub name: String,
    /// The defining text of the artifact (character sheet, scene summary,
    /// beat narration request, …). For visual artifacts this is the default
    /// generation prompt.
    pub description: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub variant_of: Option<Uuid>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub variant_axis: Option<VariantAxis>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_id: Option<Uuid>,
    /// Beat-only: how this beat's image is composed from layers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub composition: Option<BeatComposition>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_revision_id: Option<Uuid>,
    #[serde(default)]
    pub revisions: Vec<ArtifactRevision>,
    /// Approved or proposed story text (the stand-in for the deferred `.sf`
    /// story document; see `docs/artifact-canvas.md` §8–9).
    #[serde(default)]
    pub drafts: Vec<StoryDraft>,
}

/// A beat's composition spec — how layers combine into one image
/// (`docs/artifact-canvas.md` §4). `background` is the environment variant
/// ref or "generate fresh" (no backdrop layer); layers carry roles and
/// geometry is direct manipulation (anchor), not an operation.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BeatComposition {
    pub mode: CompositionMode,
    #[serde(default)]
    pub layers: Vec<Layer>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CompositionMode {
    Baked,
    Layered,
    Hybrid,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LayerRole {
    Backdrop,
    Baked,
    Dynamic,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Layer {
    pub id: Uuid,
    pub position: u32,
    pub artifact_ref: Uuid,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub variant_ref: Option<Uuid>,
    pub role: LayerRole,
    /// Position/size JSON, set by direct manipulation, never by an op.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub anchor: Option<String>,
}

/// One generation history entry for a visual artifact.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ArtifactRevision {
    pub id: Uuid,
    pub prompt: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub asset_path: Option<String>,
    pub status: RevisionStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seed: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// Masks are a view over this specific revision image.
    #[serde(default)]
    pub masks: Vec<StoredMask>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StoredMask {
    pub id: Uuid,
    pub asset_path: String,
    pub source: MaskSource,
    /// Grounding text or box JSON for auto masks.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub score: Option<f64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MaskSource {
    Auto,
    Painted,
}

/// A proposed/approved piece of story text (see `draft`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StoryDraft {
    pub id: Uuid,
    pub request: String,
    pub text: String,
    pub approved: bool,
}

#[derive(Debug, Error)]
pub enum RegistryError {
    #[error("artifact {0} does not exist")]
    NoArtifact(Uuid),
    #[error("artifact kind {kind} cannot have variants")]
    NotVariantable { kind: ArtifactKind },
    #[error("artifact {0} is itself a variant; variants must derive from a base artifact")]
    VariantOfVariant(Uuid),
    #[error("variant kind {variant} does not match its base kind {base}")]
    VariantKindMismatch {
        variant: ArtifactKind,
        base: ArtifactKind,
    },
    #[error("artifact kind {kind} cannot be a child of a {parent}")]
    BadParent {
        kind: ArtifactKind,
        parent: ArtifactKind,
    },
    #[error("a {kind} cannot have a parent")]
    NoParentAllowed { kind: ArtifactKind },
    #[error("only beats have compositions; {0} is a {1}")]
    NotABeat(Uuid, ArtifactKind),
    #[error("beat {0} is not inside a scene")]
    NotInScene(Uuid),
    #[error("revision {0} does not exist on artifact {1}")]
    NoRevision(Uuid, Uuid),
}

/// The in-memory artifact registry for one project.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ArtifactRegistry {
    /// One id space; creation order is preserved for display.
    pub artifacts: Vec<Artifact>,
    /// Every applied/rejected operation — provenance and the basis for undo
    /// (undo is state restore via [`ArtifactRegistry::snapshot`], never
    /// pipeline re-execution).
    #[serde(default)]
    pub log: Vec<OperationLogEntry>,
}

/// The stopgap project file format until SQLite lands (§10 is deferred).
/// Contains only the registry; the `Project`/storyboard continues to use the
/// TOML `ProjectStore`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ProjectFile {
    pub version: u32,
    pub registry: ArtifactRegistry,
}

impl Default for ProjectFile {
    fn default() -> Self {
        Self {
            version: 1,
            registry: ArtifactRegistry::default(),
        }
    }
}

impl ArtifactRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Resolves a `c:<ref>` against this registry. Tried in order: full
    /// UUID (with or without `c:`), the 8-hex short id, then the memorable
    /// artifact name (case-insensitive exact match).
    pub fn resolve(&self, reference: &Ref) -> Result<Uuid, RefError> {
        let raw = reference.0.trim();
        if raw.is_empty() {
            return Err(RefError::Invalid(reference.0.clone()));
        }
        let candidate = raw.strip_prefix("c:").unwrap_or(raw);

        if let Ok(parsed) = Uuid::parse_str(candidate) {
            return self.exact(parsed);
        }
        // The 8-hex short id is a fallback; a name that merely looks like
        // hex falls through to name matching when no id prefix matches.
        if candidate.len() == 8 && candidate.chars().all(|c| c.is_ascii_hexdigit()) {
            match self.match_short_id(candidate) {
                Ok(id) => return Ok(id),
                Err(error @ RefError::Ambiguous { .. }) => return Err(error),
                Err(RefError::NotFound(_)) | Err(RefError::Invalid(_)) => {}
            }
        }
        self.match_name(reference, candidate)
    }

    fn exact(&self, id: Uuid) -> Result<Uuid, RefError> {
        if self.artifacts.iter().any(|artifact| artifact.id == id) {
            Ok(id)
        } else {
            Err(RefError::NotFound(id.to_string()))
        }
    }

    fn match_short_id(&self, candidate: &str) -> Result<Uuid, RefError> {
        let matches: Vec<Uuid> = self
            .artifacts
            .iter()
            .filter(|artifact| artifact.id.simple().to_string().starts_with(candidate))
            .map(|artifact| artifact.id)
            .collect();
        match matches.as_slice() {
            [] => Err(RefError::NotFound(candidate.to_owned())),
            [only] => Ok(*only),
            _ => Err(RefError::Ambiguous {
                value: candidate.to_owned(),
                matches: matches.len(),
            }),
        }
    }

    fn match_name(&self, reference: &Ref, name: &str) -> Result<Uuid, RefError> {
        let matches: Vec<Uuid> = self
            .artifacts
            .iter()
            .filter(|artifact| artifact.name.eq_ignore_ascii_case(name))
            .map(|artifact| artifact.id)
            .collect();
        match matches.as_slice() {
            [] => Err(RefError::NotFound(reference.0.clone())),
            [only] => Ok(*only),
            _ => Err(RefError::Ambiguous {
                value: reference.0.clone(),
                matches: matches.len(),
            }),
        }
    }

    /// The `c:` ref to print and type: the artifact's memorable name, or the
    /// 8-hex short id for nameless artifacts.
    pub fn ref_of(&self, id: Uuid) -> String {
        self.artifact(id)
            .map(|artifact| {
                if artifact.name.trim().is_empty() {
                    self.short_id(id)
                } else {
                    format!("c:{}", artifact.name)
                }
            })
            .unwrap_or_else(|| format!("c:{id}"))
    }

    /// The `c:` short id (`c:` + first 8 hex digits of the UUID).
    pub fn short_id(&self, id: Uuid) -> String {
        format!("c:{}", &id.simple().to_string()[..8])
    }

    pub fn artifact(&self, id: Uuid) -> Option<&Artifact> {
        self.artifacts.iter().find(|artifact| artifact.id == id)
    }

    pub fn artifact_mut(&mut self, id: Uuid) -> Option<&mut Artifact> {
        self.artifacts.iter_mut().find(|artifact| artifact.id == id)
    }

    pub fn iter(&self) -> impl Iterator<Item = &Artifact> {
        self.artifacts.iter()
    }

    /// The latest completed revision with an asset — the artifact's active
    /// image for editing and reference. A failed revision never becomes the
    /// active image.
    pub fn latest_image(&self, id: Uuid) -> Option<&ArtifactRevision> {
        let artifact = self.artifact(id)?;
        artifact.revisions.iter().rev().find(|revision| {
            revision.status == RevisionStatus::Completed && revision.asset_path.is_some()
        })
    }

    /// The revision currently marked active (which may be queued/failed).
    pub fn active_revision(&self, id: Uuid) -> Option<&ArtifactRevision> {
        let artifact = self.artifact(id)?;
        let active_id = artifact.active_revision_id?;
        artifact
            .revisions
            .iter()
            .find(|revision| revision.id == active_id)
    }

    /// Creates an artifact, enforcing the kind-compatibility invariants.
    pub fn create_artifact(
        &mut self,
        kind: ArtifactKind,
        name: String,
        description: String,
        parent: Option<Uuid>,
        variant_of: Option<Uuid>,
        axis: Option<VariantAxis>,
    ) -> Result<Uuid, RegistryError> {
        if let Some(base_id) = variant_of {
            if !matches!(
                kind,
                ArtifactKind::Character | ArtifactKind::Environment | ArtifactKind::Object
            ) {
                return Err(RegistryError::NotVariantable { kind });
            }
            let base = self
                .artifact(base_id)
                .ok_or(RegistryError::NoArtifact(base_id))?;
            if base.variant_of.is_some() {
                return Err(RegistryError::VariantOfVariant(base_id));
            }
            if base.kind != kind {
                return Err(RegistryError::VariantKindMismatch {
                    variant: kind,
                    base: base.kind,
                });
            }
        }
        if let Some(parent_id) = parent {
            let parent_artifact = self
                .artifact(parent_id)
                .ok_or(RegistryError::NoArtifact(parent_id))?;
            let parent_kind = parent_artifact.kind;
            let compatible = match kind {
                ArtifactKind::Story | ArtifactKind::Character | ArtifactKind::Object => false,
                ArtifactKind::Scene => parent_kind == ArtifactKind::Story,
                ArtifactKind::Beat => parent_kind == ArtifactKind::Scene,
                ArtifactKind::Environment => parent_kind == ArtifactKind::Environment,
            };
            if !compatible {
                return Err(
                    if matches!(
                        kind,
                        ArtifactKind::Story | ArtifactKind::Character | ArtifactKind::Object
                    ) {
                        RegistryError::NoParentAllowed { kind }
                    } else {
                        RegistryError::BadParent {
                            kind,
                            parent: parent_kind,
                        }
                    },
                );
            }
        }

        let id = Uuid::new_v4();
        self.artifacts.push(Artifact {
            id,
            kind,
            name,
            description,
            variant_of,
            variant_axis: axis,
            parent_id: parent,
            composition: None,
            active_revision_id: None,
            revisions: Vec::new(),
            drafts: Vec::new(),
        });
        Ok(id)
    }

    /// Starts a revision: appends a queued entry and marks it active.
    pub fn start_revision(
        &mut self,
        artifact_id: Uuid,
        prompt: String,
        seed: Option<u64>,
        model: Option<String>,
    ) -> Result<Uuid, RegistryError> {
        let artifact = self
            .artifact_mut(artifact_id)
            .ok_or(RegistryError::NoArtifact(artifact_id))?;
        let revision_id = Uuid::new_v4();
        artifact.revisions.push(ArtifactRevision {
            id: revision_id,
            prompt,
            asset_path: None,
            status: RevisionStatus::Queued,
            seed,
            model,
            error: None,
            masks: Vec::new(),
        });
        artifact.active_revision_id = Some(revision_id);
        Ok(revision_id)
    }

    /// Marks a revision completed and promotes it to the active image.
    pub fn finish_revision(
        &mut self,
        artifact_id: Uuid,
        revision_id: Uuid,
        asset_path: String,
        masks: Vec<StoredMask>,
    ) -> Result<(), RegistryError> {
        let artifact = self
            .artifact_mut(artifact_id)
            .ok_or(RegistryError::NoArtifact(artifact_id))?;
        let revision = artifact
            .revisions
            .iter_mut()
            .find(|revision| revision.id == revision_id)
            .ok_or(RegistryError::NoRevision(revision_id, artifact_id))?;
        revision.status = RevisionStatus::Completed;
        revision.asset_path = Some(asset_path);
        revision.error = None;
        revision.masks = masks;
        artifact.active_revision_id = Some(revision_id);
        Ok(())
    }

    /// Marks a revision failed; the artifact keeps its last good image.
    pub fn fail_revision(
        &mut self,
        artifact_id: Uuid,
        revision_id: Uuid,
        error: String,
    ) -> Result<(), RegistryError> {
        let artifact = self
            .artifact_mut(artifact_id)
            .ok_or(RegistryError::NoArtifact(artifact_id))?;
        let revision = artifact
            .revisions
            .iter_mut()
            .find(|revision| revision.id == revision_id)
            .ok_or(RegistryError::NoRevision(revision_id, artifact_id))?;
        revision.status = RevisionStatus::Failed;
        revision.error = Some(error);
        Ok(())
    }

    /// Marks a revision cancelled (e.g. the user rejected a mask checkpoint).
    pub fn cancel_revision(
        &mut self,
        artifact_id: Uuid,
        revision_id: Uuid,
    ) -> Result<(), RegistryError> {
        let artifact = self
            .artifact_mut(artifact_id)
            .ok_or(RegistryError::NoArtifact(artifact_id))?;
        let revision = artifact
            .revisions
            .iter_mut()
            .find(|revision| revision.id == revision_id)
            .ok_or(RegistryError::NoRevision(revision_id, artifact_id))?;
        revision.status = RevisionStatus::Cancelled;
        Ok(())
    }

    /// Attaches a composition to a beat (the beat must live in a scene).
    pub fn set_composition(
        &mut self,
        beat_id: Uuid,
        composition: BeatComposition,
    ) -> Result<(), RegistryError> {
        let artifact = self
            .artifact_mut(beat_id)
            .ok_or(RegistryError::NoArtifact(beat_id))?;
        if artifact.kind != ArtifactKind::Beat {
            return Err(RegistryError::NotABeat(beat_id, artifact.kind));
        }
        if artifact.parent_id.is_none() {
            return Err(RegistryError::NotInScene(beat_id));
        }
        artifact.composition = Some(composition);
        Ok(())
    }

    /// Records a proposed or approved story draft.
    pub fn add_draft(
        &mut self,
        artifact_id: Uuid,
        request: String,
        text: String,
        approved: bool,
    ) -> Result<Uuid, RegistryError> {
        let artifact = self
            .artifact_mut(artifact_id)
            .ok_or(RegistryError::NoArtifact(artifact_id))?;
        let id = Uuid::new_v4();
        artifact.drafts.push(StoryDraft {
            id,
            request,
            text,
            approved,
        });
        Ok(id)
    }

    /// Appends an operation-log entry; returns its id.
    pub fn push_log(
        &mut self,
        op: Operation,
        origin: OpOrigin,
        status: OpStatus,
        artifact_id: Option<Uuid>,
    ) -> Uuid {
        let id = Uuid::new_v4();
        self.log.push(OperationLogEntry {
            id,
            artifact_id,
            op,
            origin,
            status,
        });
        id
    }

    pub fn log(&self) -> &[OperationLogEntry] {
        &self.log
    }

    pub fn log_for(&self, artifact_id: Uuid) -> impl Iterator<Item = &OperationLogEntry> {
        self.log
            .iter()
            .filter(move |entry| entry.artifact_id == Some(artifact_id))
    }

    /// Full state snapshot — the basis for undo (restore, never re-run).
    pub fn snapshot(&self) -> ArtifactRegistry {
        self.clone()
    }

    /// Restores a prior snapshot (see [`Self::snapshot`]).
    pub fn restore(&mut self, snapshot: ArtifactRegistry) {
        *self = snapshot;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn registry_with(artifacts: &[(ArtifactKind, &str)]) -> ArtifactRegistry {
        let mut registry = ArtifactRegistry::new();
        for (kind, description) in artifacts {
            registry
                .create_artifact(
                    *kind,
                    String::new(),
                    (*description).to_owned(),
                    None,
                    None,
                    None,
                )
                .expect("test artifact should create");
        }
        registry
    }

    #[test]
    fn create_artifact_enforces_kind_parent_rules() {
        let mut registry = ArtifactRegistry::new();
        let story = registry
            .create_artifact(
                ArtifactKind::Story,
                "The Lighthouse".into(),
                "A story".into(),
                None,
                None,
                None,
            )
            .unwrap();
        let scene = registry
            .create_artifact(
                ArtifactKind::Scene,
                "Kitchen".into(),
                "The kitchen".into(),
                Some(story),
                None,
                None,
            )
            .unwrap();
        let beat = registry
            .create_artifact(
                ArtifactKind::Beat,
                "Beat 1".into(),
                "Mia at the stove".into(),
                Some(scene),
                None,
                None,
            )
            .unwrap();

        assert!(registry.artifact(beat).is_some());
        // A beat cannot be a scene's sibling.
        assert!(registry
            .create_artifact(
                ArtifactKind::Beat,
                "X".into(),
                "X".into(),
                Some(story),
                None,
                None
            )
            .is_err());
        // A story cannot have a parent at all.
        assert!(registry
            .create_artifact(
                ArtifactKind::Story,
                "Y".into(),
                "Y".into(),
                Some(scene),
                None,
                None
            )
            .is_err());
        // A room (environment) can live inside an environment.
        let house = registry
            .create_artifact(
                ArtifactKind::Environment,
                "House".into(),
                "A house".into(),
                None,
                None,
                None,
            )
            .unwrap();
        let room = registry
            .create_artifact(
                ArtifactKind::Environment,
                "Kitchen".into(),
                "A kitchen".into(),
                Some(house),
                None,
                None,
            )
            .unwrap();
        assert!(registry.artifact(room).is_some());
    }

    #[test]
    fn variants_require_a_base_of_the_same_kind() {
        let mut registry = ArtifactRegistry::new();
        let mia = registry
            .create_artifact(
                ArtifactKind::Character,
                "mia".into(),
                "Mia, the keeper".into(),
                None,
                None,
                None,
            )
            .unwrap();

        let variant = registry
            .create_artifact(
                ArtifactKind::Character,
                "mia".into(),
                "Mia in rain gear".into(),
                None,
                Some(mia),
                Some(VariantAxis::Outfit),
            )
            .unwrap();
        assert!(registry.artifact(variant).is_some());

        // Variants of variants are rejected.
        assert!(registry
            .create_artifact(
                ArtifactKind::Character,
                "mia".into(),
                "double variant".into(),
                None,
                Some(variant),
                None,
            )
            .is_err());
        // Stories cannot have variants.
        let story = registry
            .create_artifact(
                ArtifactKind::Story,
                "S".into(),
                "S".into(),
                None,
                None,
                None,
            )
            .unwrap();
        assert!(registry
            .create_artifact(
                ArtifactKind::Story,
                "V".into(),
                "V".into(),
                None,
                Some(story),
                None
            )
            .is_err());
        // Kind mismatch is rejected.
        assert!(registry
            .create_artifact(
                ArtifactKind::Object,
                "hat".into(),
                "a hat".into(),
                None,
                Some(mia),
                None,
            )
            .is_err());
    }

    #[test]
    fn short_ids_are_the_first_eight_hex_digits() {
        let mut registry = ArtifactRegistry::new();
        let id = registry
            .create_artifact(
                ArtifactKind::Character,
                "mia".into(),
                "Mia".into(),
                None,
                None,
                None,
            )
            .unwrap();
        let short = registry.short_id(id);
        assert_eq!(short.len(), 10);
        assert!(short.starts_with("c:"));
        assert_eq!(&id.simple().to_string()[..8], &short[2..]);
    }

    #[test]
    fn refs_resolve_by_uuid_and_short_id() {
        let registry = registry_with(&[(ArtifactKind::Story, "a story")]);
        let id = registry.artifacts[0].id;

        assert_eq!(registry.resolve(&Ref::new(id.to_string())).unwrap(), id);
        assert_eq!(registry.resolve(&Ref::new(format!("c:{id}"))).unwrap(), id);
        assert_eq!(
            registry.resolve(&Ref::new(registry.short_id(id))).unwrap(),
            id
        );
        assert!(matches!(
            registry.resolve(&Ref::new("c:00000000")).unwrap_err(),
            RefError::NotFound(_)
        ));
    }

    #[test]
    fn refs_resolve_by_memorable_name() {
        let mut registry = ArtifactRegistry::new();
        let mia = registry
            .create_artifact(
                ArtifactKind::Character,
                "mia".into(),
                "Mia, the keeper".into(),
                None,
                None,
                None,
            )
            .unwrap();
        registry
            .create_artifact(
                ArtifactKind::Character,
                "Elias".into(),
                "Elias, the fisherman".into(),
                None,
                None,
                None,
            )
            .unwrap();

        // Exact and case-insensitive matches resolve.
        assert_eq!(registry.resolve(&Ref::new("c:mia")).unwrap(), mia);
        assert_eq!(registry.resolve(&Ref::new("c:MIA")).unwrap(), mia);
        assert_eq!(
            registry.resolve(&Ref::new("c:elias")).unwrap(),
            registry.artifacts[1].id
        );

        // Unknown names and empty refs are rejected.
        assert!(matches!(
            registry.resolve(&Ref::new("c:nobody")).unwrap_err(),
            RefError::NotFound(_)
        ));
        assert!(matches!(
            registry.resolve(&Ref::new("")).unwrap_err(),
            RefError::Invalid(_)
        ));

        // The printable ref is the memorable name.
        assert_eq!(registry.ref_of(mia), "c:mia");
    }

    #[test]
    fn duplicate_names_are_ambiguous() {
        let mut registry = ArtifactRegistry::new();
        registry
            .create_artifact(
                ArtifactKind::Character,
                "mia".into(),
                "one".into(),
                None,
                None,
                None,
            )
            .unwrap();
        registry
            .create_artifact(
                ArtifactKind::Object,
                "Mia".into(),
                "two".into(),
                None,
                None,
                None,
            )
            .unwrap();

        let error = registry.resolve(&Ref::new("c:mia")).unwrap_err();
        assert!(matches!(error, RefError::Ambiguous { matches: 2, .. }));
    }

    #[test]
    fn short_id_prefixes_can_be_ambiguous() {
        // Craft two artifacts whose ids share an 8-hex prefix.
        let mut registry = ArtifactRegistry::new();
        let first = Uuid::parse_str("aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee").unwrap();
        let second = Uuid::parse_str("aaaaaaaa-0000-1111-2222-333333333333").unwrap();
        registry.artifacts.push(Artifact {
            id: first,
            kind: ArtifactKind::Character,
            name: String::new(),
            description: "one".into(),
            variant_of: None,
            variant_axis: None,
            parent_id: None,
            composition: None,
            active_revision_id: None,
            revisions: Vec::new(),
            drafts: Vec::new(),
        });
        registry.artifacts.push(Artifact {
            id: second,
            kind: ArtifactKind::Object,
            name: String::new(),
            description: "two".into(),
            variant_of: None,
            variant_axis: None,
            parent_id: None,
            composition: None,
            active_revision_id: None,
            revisions: Vec::new(),
            drafts: Vec::new(),
        });

        let error = registry.resolve(&Ref::new("c:aaaaaaaa")).unwrap_err();
        assert!(matches!(error, RefError::Ambiguous { .. }));
        // The full UUID still resolves unambiguously.
        assert_eq!(
            registry.resolve(&Ref::new(first.to_string())).unwrap(),
            first
        );
    }

    #[test]
    fn latest_image_skips_failed_and_queued_revisions() {
        let mut registry = registry_with(&[(ArtifactKind::Character, "Mia")]);
        let id = registry.artifacts[0].id;
        let first = registry
            .start_revision(id, "first".into(), Some(1), None)
            .unwrap();
        registry
            .finish_revision(id, first, "first.png".into(), Vec::new())
            .unwrap();
        let second = registry
            .start_revision(id, "second".into(), Some(2), None)
            .unwrap();
        registry
            .fail_revision(id, second, "backend exploded".into())
            .unwrap();

        let image = registry
            .latest_image(id)
            .expect("first revision should remain the image");
        assert_eq!(image.asset_path.as_deref(), Some("first.png"));
        assert_eq!(registry.active_revision(id).map(|r| r.id), Some(second));
    }

    #[test]
    fn undo_restores_a_snapshot_instead_of_reexecuting() {
        let mut registry = registry_with(&[(ArtifactKind::Story, "a story")]);
        let snapshot = registry.snapshot();
        registry
            .create_artifact(
                ArtifactKind::Character,
                "mia".into(),
                "Mia".into(),
                None,
                None,
                None,
            )
            .unwrap();
        assert_eq!(registry.artifacts.len(), 2);

        registry.restore(snapshot);
        assert_eq!(registry.artifacts.len(), 1);
    }
}
